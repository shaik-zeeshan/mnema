//! The **Hybrid Search** query-embedding seam (issue #124): embeds the search
//! query string into a **Semantic Search Vector** so the app-infra search path
//! can fuse a `vec0` KNN with the FTS5 **Text Search** ranking by RRF.
//!
//! app-infra takes no embedding-runtime dependency (the same boundary that keeps
//! `ai-runtime` out of it for User Context), so the *query* is embedded here, in
//! the desktop layer, and passed into `search_capture` as a pre-computed
//! `query_embedding`. When no **Semantic Search Model** is installed — the
//! default/Anthropic-only case — this returns `None` and search stays
//! keyword-only with no regression, exactly the **default-on but model-gated**
//! shape of local transcription/OCR. Mnema never auto-downloads a model.
//!
//! The loaded model is cached in [`SemanticQueryEmbedderState`] so repeated
//! searches (every keystroke) reuse one embedder rather than reloading the model;
//! a Settings model switch reloads it because the cache remembers which
//! provider/model it holds. The embed runs on a blocking thread because the candle
//! forward is synchronous model work (Metal GPU on macOS / candle-CPU elsewhere)
//! that must stay off the tokio reactor (ADR 0037).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use semantic_search::EmbedKind;

use crate::semantic_search_worker::{
    effective_semantic_search_settings, load_embedder, load_embedder_private,
    resolve_selected_descriptor, selected_model_available, single_copy_only, warm_shared_embedder,
    LoadedEmbedder, BACKGROUND_EMBED_SLOT,
};
use crate::AppCore;

/// The cached query embedder, shared as Tauri managed state across `search_capture`
/// calls. `None` until the first search runs with an installed model. A model
/// switch from Settings reloads it (the cache remembers its provider/model id).
///
/// `load_lock` serializes the take→load→embed→restore region so two searches
/// racing a *cold* load (or a model-switch reload) do not each pay the heavy
/// `load_embedder` cost: the second waiter awaits the in-flight load behind the
/// async lock and then reuses the cached embedder. It is a `tokio::sync::Mutex`
/// (not `std::Mutex`) precisely because it is held across the `.await` of the
/// blocking embed task; the `std::Mutex` around `cached` is only ever taken in
/// short, await-free critical sections (the invariant: no `std::Mutex` across an
/// await point).
#[derive(Default)]
pub struct SemanticQueryEmbedderState {
    cached: Arc<Mutex<IdleSlot<Arc<LoadedEmbedder>>>>,
    load_lock: tokio::sync::Mutex<()>,
}

/// How long the query embedder stays loaded after the last search. For a model
/// under 1 GB it is a PRIVATE second copy (see `load_embedder_private`), so holding
/// it costs the full weights again — ~450 MB of GPU memory for the default model
/// (measured 2026-11-11: one search pinned it for the process lifetime). Short
/// enough to return that soon after a search session, long enough that typing
/// and follow-up questions never reload; the next search pays one model load.
const QUERY_EMBEDDER_IDLE_RELEASE: Duration = Duration::from_secs(180);

/// A cached value plus a counter of how many times it was parked, so a timer
/// started at one park can tell whether a later search has used it since.
struct IdleSlot<T> {
    value: Option<T>,
    parks: u64,
}

impl<T> Default for IdleSlot<T> {
    fn default() -> Self {
        Self { value: None, parks: 0 }
    }
}

impl<T> IdleSlot<T> {
    /// Store `value` and return the park number its idle timer must present.
    fn park(&mut self, value: T) -> u64 {
        self.value = Some(value);
        self.parks += 1;
        self.parks
    }

    /// Take the value if nothing was parked after `park`. A search in flight has
    /// already taken it, so this finds `None` and that search re-arms on restore.
    fn release_if_idle(&mut self, park: u64) -> Option<T> {
        if self.parks == park {
            self.value.take()
        } else {
            None
        }
    }
}

impl SemanticQueryEmbedderState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Whether the cached query embedder can still be handed to this search: the same
/// provider/model AND still inside the shared warm cap
/// ([`MAX_EMBEDDER_WARM_LIFETIME`](crate::semantic_search_worker::MAX_EMBEDDER_WARM_LIFETIME)).
///
/// The staleness half is not an optimization: a search every few minutes keeps
/// resetting [`QUERY_EMBEDDER_IDLE_RELEASE`] and this holder has no per-pass
/// recycle, so a stale `Arc` kept here pins the instance — and, on macOS, candle's never-swept Metal buffer
/// pool — for the whole process lifetime, which makes both workers' recycle free
/// nothing and leaves a second full model resident once they reload.
/// `warm_for` is a `Duration` (`loaded_at.elapsed()`), not the `Instant`, so the
/// boundary is testable without winding a real clock back — `Instant::checked_sub`
/// returns `None` on a host whose uptime is under the cap.
fn cached_embedder_is_reusable(
    provider: &str,
    model_id: &str,
    warm_for: std::time::Duration,
    descriptor: &semantic_search::SemanticSearchModelDescriptor,
) -> bool {
    provider == descriptor.provider
        && model_id == descriptor.model_id
        && warm_for < crate::semantic_search_worker::MAX_EMBEDDER_WARM_LIFETIME
}

/// Embed the search `query` into a **Semantic Search Vector** for **Hybrid
/// Search**, or `None` when **Semantic Search** is inert: no model installed,
/// the selection is disabled/unknown, an empty query, or a model load/embed
/// failure. A `None` never surfaces an error to the caller — search simply runs
/// keyword-only, the same "no usable runtime → feature unavailable" shape as
/// local transcription.
///
/// `query` is the operator-stripped residual body (see
/// `::app_infra::semantic_search_residual_query`), not the raw query — so
/// `app:`/`before:`/`source:` operators and quoted phrases never pollute the
/// meaning vector, and it matches what FTS ranks on. An all-operators query has an
/// empty residual, which the trim/empty-guard below resolves to `None`
/// (keyword-only).
///
/// The model load and the embed both run on a blocking thread because the candle
/// forward is synchronous model work (Metal GPU on macOS / candle-CPU elsewhere)
/// that must stay off the tokio reactor (and off the capture hot path — this is
/// user-initiated search, not capture).
pub async fn embed_search_query(
    core: &AppCore,
    state: &SemanticQueryEmbedderState,
    query: &str,
) -> Option<Vec<f32>> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return None;
    }

    let settings = effective_semantic_search_settings(core);
    let app_data_dir = core.paths().config.clone();

    // Model-gating: silent no-op when no model is installed (no error, no
    // auto-download). Routes through the resolver so a Custom-picked model is
    // recognized once installed; a corrupt marker is treated as unavailable.
    if !selected_model_available(&app_data_dir, &settings) {
        return None;
    }

    let descriptor = resolve_selected_descriptor(&settings)?;

    // Serialize the load→embed→restore region with the async load lock so two
    // searches racing a cold load (or model-switch reload) do not each pay the
    // heavy `load_embedder` cost: a concurrent search awaits the in-flight load
    // here and reuses the embedder the winner caches. Held across the blocking
    // embed `.await` below, so it must be the async lock — never a `std::Mutex`
    // across an await point. The `std::Mutex` around `cached` is still taken only
    // in short, await-free sections (take below, restore after the await).
    let _load_guard = state.load_lock.lock().await;

    // Borrow the indexer's copy when it is already loaded and the background GPU
    // slot is free right now, so a search does not load a second copy (~450 MB of
    // GPU memory for the default model). Holding the slot for this one embed keeps
    // the device to one user at a time — sharing was slow only while a backfill
    // batch ran on it (`load_embedder_private`'s table). A busy slot never waits:
    // it falls through to the private copy below. Background embeds wait at most
    // this one query (~30 ms). A single-copy model already shares below.
    if !single_copy_only(&descriptor) {
        let shared_descriptor = descriptor.clone();
        let query = trimmed.to_string();
        let borrowed = core
            .spawn_blocking(move || {
                borrow_when_free(
                    &BACKGROUND_EMBED_SLOT,
                    || warm_shared_embedder(&shared_descriptor),
                    |shared| {
                        shared
                            .embedder
                            .embed_text(&query, EmbedKind::Query)
                            .map_err(|error| error.to_string())
                    },
                )
            })
            .await;
        if let Ok(Some(vector)) = borrowed {
            return usable(vector);
        }
    }

    // Take the cached embedder out of the std mutex for the blocking task (so we
    // never hold a std Mutex across an await), reloading if the selection changed,
    // then put it back. The async load lock above keeps the brief None window
    // between take and restore from racing a concurrent cold load.
    let cached = {
        let mut guard = state.cached.lock().unwrap_or_else(|poison| poison.into_inner());
        match guard.value.take() {
            Some(loaded)
                if cached_embedder_is_reusable(
                    &loaded.provider,
                    &loaded.model_id,
                    loaded.loaded_at.elapsed(),
                    &descriptor,
                ) =>
            {
                Some(loaded)
            }
            // A different model (Settings switch), an instance warm past the shared
            // cap, or nothing cached: drop the `Arc` here (so the shared instance can
            // actually be freed) and reload below.
            _ => None,
        }
    };

    let query = trimmed.to_string();
    let result = core.spawn_blocking(move || {
        let loaded = match cached {
            Some(loaded) => loaded,
            // A PRIVATE instance, not the shared background one: the slot was busy
            // (or nothing was warm), and sharing the sweep's candle device
            // would queue its kernels behind the sweep's on one device lock
            // (measured p50 92ms -> 310ms, p95 233ms -> 995ms). See
            // `load_embedder_private`. A model over 1 GB is the exception: it is
            // never loaded twice, so the search shares the indexer's instance.
            None => match if single_copy_only(&descriptor) {
                load_embedder(&app_data_dir, &descriptor)
            } else {
                load_embedder_private(&app_data_dir, &descriptor)
            } {
                Ok(loaded) => loaded,
                Err(error) => {
                    crate::debug_log::log_warn(format!(
                        "semantic query embed failed to load model '{}/{}': {error}",
                        descriptor.provider, descriptor.model_id
                    ));
                    return None;
                }
            },
        };
        // The query embed runs at the blocking thread's DEFAULT QoS. The retired
        // backfill per-thread background-QoS downclock is gone under candle (ADR
        // 0037), so there is no QoS to deliberately leave un-backgrounded here; this
        // is simply user-initiated, latency-sensitive search work. `embed_text` is
        // `&self`-immutable, so the embedder needs no `mut`.
        let vector = loaded
            .embedder
            .embed_text(&query, EmbedKind::Query)
            .map_err(|error| error.to_string());
        Some((loaded, vector))
    })
    .await;

    let (loaded, vector) = match result {
        Ok(Some((loaded, vector))) => (loaded, vector),
        Ok(None) => return None,
        Err(error) => {
            crate::debug_log::log_warn(format!(
                "semantic query embed task panicked/cancelled: {error}"
            ));
            return None;
        }
    };

    // Restore the embedder for the next search — unless it is already warm past the
    // shared cap, in which case drop the `Arc` here instead of parking it — and arm
    // the idle release: with no further search inside
    // `QUERY_EMBEDDER_IDLE_RELEASE` the private copy is dropped.
    //
    // ponytail: one sleeping timer per search rather than one re-armed timer; each
    // is a few hundred bytes for three minutes and all but the last are no-ops.
    if !loaded.is_stale() {
        let park = state
            .cached
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .park(loaded);
        let cached = state.cached.clone();
        core.spawn(async move {
            tokio::time::sleep(QUERY_EMBEDDER_IDLE_RELEASE).await;
            let released = cached
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .release_if_idle(park);
            if released.is_some() {
                crate::debug_log::log(format!(
                    "semantic query embedder released after {}s without a search \
                     (model weights returned to the OS; reloads on next search)",
                    QUERY_EMBEDDER_IDLE_RELEASE.as_secs()
                ));
            }
        });
    }

    usable(vector)
}

/// Mirror the write-path guard (`store_vector` rejects non-finite components): a
/// NaN/inf component would yield non-deterministic KNN ordering, so drop the
/// vector and fall back to the keyword-only path the function already takes when
/// there is no usable semantic vector.
fn usable(vector: Result<Vec<f32>, String>) -> Option<Vec<f32>> {
    match vector {
        Ok(vector) if vector.iter().any(|component| !component.is_finite()) => {
            crate::debug_log::log_warn(
                "semantic query embed produced a non-finite component; \
                 falling back to keyword-only search"
                    .to_string(),
            );
            None
        }
        Ok(vector) => Some(vector),
        Err(error) => {
            crate::debug_log::log_warn(format!(
                "semantic query embed failed: {error}"
            ));
            None
        }
    }
}

/// Run `embed` on `peek`'s instance while holding `slot`, or `None` without
/// waiting when the slot is busy or `peek` finds nothing warm. The borrowed value
/// is dropped when `embed` returns, so the owner's idle drop and warm-cap recycle
/// still free it.
fn borrow_when_free<T, R>(
    slot: &tokio::sync::Semaphore,
    peek: impl FnOnce() -> Option<T>,
    embed: impl FnOnce(T) -> R,
) -> Option<R> {
    let _permit = slot.try_acquire().ok()?;
    peek().map(embed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic_search_worker::MAX_EMBEDDER_WARM_LIFETIME;
    use capture_types::default_semantic_search_settings;

    fn nomic_descriptor() -> semantic_search::SemanticSearchModelDescriptor {
        resolve_selected_descriptor(&default_semantic_search_settings())
            .expect("the default English tier resolves")
    }

    #[test]
    fn the_query_cache_releases_an_embedder_warm_past_the_shared_cap() {
        let descriptor = nomic_descriptor();

        // A freshly loaded instance is reused across keystrokes (the whole point of
        // the cache).
        assert!(cached_embedder_is_reusable(
            &descriptor.provider,
            &descriptor.model_id,
            Duration::ZERO,
            &descriptor
        ));

        // A Settings model switch is still a miss.
        assert!(!cached_embedder_is_reusable(
            "some-other-provider",
            &descriptor.model_id,
            Duration::ZERO,
            &descriptor
        ));

        // ...and so is an instance warm past the shared cap. Steady searching keeps
        // resetting the idle release and there is no per-pass recycle, so keeping
        // its `Arc` past the cap pins
        // the process-wide embedder (and candle's never-swept Metal buffer pool) for
        // the process lifetime: both workers' recycle then frees nothing, and their
        // reload leaves a SECOND full model resident.
        assert!(
            !cached_embedder_is_reusable(
                &descriptor.provider,
                &descriptor.model_id,
                MAX_EMBEDDER_WARM_LIFETIME,
                &descriptor
            ),
            "a query embedder warm past MAX_EMBEDDER_WARM_LIFETIME must be dropped, \
             not pinned forever"
        );
    }

    /// `gte-modernbert-base` shares nomic's 768-dim width, so a dimension check
    /// could never substitute for the `model_id` clause: reusing the cached nomic
    /// embedder for a gte selection would embed the query with the wrong model and
    /// still produce a "valid-looking" 768-wide vector. Only the id distinguishes
    /// them.
    #[test]
    fn a_same_width_model_switch_is_still_a_cache_miss() {
        let descriptor = nomic_descriptor();
        assert!(!cached_embedder_is_reusable(
            &descriptor.provider,
            "gte-modernbert-base",
            Duration::ZERO,
            &descriptor
        ));
    }

    #[test]
    fn the_idle_release_only_fires_when_no_search_came_after_its_park() {
        let mut slot = IdleSlot::default();

        // One search, then silence: its timer releases the model.
        let first = slot.park("model");
        assert_eq!(slot.release_if_idle(first), Some("model"));

        // A second search re-parks before the first timer fires: the first timer
        // must leave the model alone, and only the latest one releases it.
        let first = slot.park("model");
        let second = slot.park("model");
        assert_eq!(slot.release_if_idle(first), None);
        assert_eq!(slot.release_if_idle(second), Some("model"));

        // A search in flight has taken the model out; the timer finds nothing.
        let parked = slot.park("model");
        let in_flight = slot.value.take();
        assert_eq!(slot.release_if_idle(parked), None);
        assert_eq!(in_flight, Some("model"));
    }

    #[test]
    fn a_search_borrows_the_warm_copy_only_while_the_background_slot_is_free() {
        let slot = tokio::sync::Semaphore::new(1);
        let indexer_copy = Arc::new("model");
        let peeks = std::cell::Cell::new(0);
        let peek = || {
            peeks.set(peeks.get() + 1);
            Some(Arc::clone(&indexer_copy))
        };

        // Slot free + copy warm: the query embeds on it (no load), holding the
        // slot so no background batch shares the device mid-embed.
        let used = borrow_when_free(&slot, peek, |model| {
            assert_eq!(slot.available_permits(), 0);
            *model
        });
        assert_eq!(used, Some("model"));
        // Neither the borrow nor the permit outlives the embed, so the owner's
        // idle drop still frees the model.
        assert_eq!(Arc::strong_count(&indexer_copy), 1);
        assert_eq!(slot.available_permits(), 1);

        // A background batch holds the slot: no wait, no peek, private path.
        let batch = slot.try_acquire().unwrap();
        assert_eq!(borrow_when_free(&slot, peek, |model| *model), None);
        assert_eq!(peeks.get(), 1);
        drop(batch);

        // Nothing warm: private path, and the slot is handed straight back.
        assert_eq!(
            borrow_when_free(&slot, || None::<Arc<&str>>, |model| *model),
            None
        );
        assert_eq!(slot.available_permits(), 1);
    }

    #[test]
    fn query_embed_is_keyword_only_without_an_installed_model() {
        let dir = tempfile::tempdir().expect("tempdir");
        let core = crate::test_support::core_in(Default::default(), dir.path());
        let state = SemanticQueryEmbedderState::new();
        assert_eq!(core.block_on(embed_search_query(&core, &state, "standup notes")), None);
        assert_eq!(core.block_on(embed_search_query(&core, &state, "   ")), None);
    }
}
