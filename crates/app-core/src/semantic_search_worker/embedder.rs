use super::*;

/// A loaded **Semantic Search Model**, tagged with the provider/model id it was
/// loaded for so a Settings model switch triggers a reload. Shared with the query
/// path (`semantic_search_query.rs`), which loads the same model to embed the
/// query string for **Hybrid Search**.
pub struct LoadedEmbedder {
    pub provider: String,
    pub model_id: String,
    pub embedder: SemanticSearchEmbedder,
    /// When this instance was built — the clock
    /// [`MAX_EMBEDDER_WARM_LIFETIME`] is measured against. Set once at load and
    /// shared by every holder, so all of them recycle the same instance within one
    /// pass of each other rather than each running its own staggered timer.
    pub loaded_at: Instant,
}

impl LoadedEmbedder {
    /// Whether this instance has been warm past [`MAX_EMBEDDER_WARM_LIFETIME`] and
    /// should be dropped so candle's Metal buffer pool goes back to the OS.
    pub fn is_stale(&self) -> bool {
        Self::stale_after(self.loaded_at.elapsed())
    }

    /// [`is_stale`](Self::is_stale) over a bare warm duration, so the cap's boundary
    /// is testable without a real model on disk.
    ///
    /// Takes the elapsed `Duration`, NOT the load `Instant`: a test that wound a real
    /// `Instant` back with `checked_sub(MAX_EMBEDDER_WARM_LIFETIME)` gets `None` on a
    /// host whose uptime is under the cap (`Instant` is boot-relative on macOS), so
    /// the `.expect(...)` panicked on a fresh CI VM for reasons unrelated to the
    /// product. There is no clock to read here.
    pub fn stale_after(warm_for: Duration) -> bool {
        warm_for >= MAX_EMBEDDER_WARM_LIFETIME
    }
}

/// The one process-wide loaded **Semantic Search Model**, held as a [`Weak`] so
/// the four independent holders — this worker, the **Subject Vector** backfill,
/// the query path's cache, and the subject-candidate scorer — share a single set
/// of model weights and, on macOS, a single candle `MetalDevice`.
///
/// Before this each holder called `load_embedder` for itself, so up to four
/// devices could be resident at once, each with its own never-reclaimed Metal
/// buffer pool (see [`MAX_EMBEDDER_WARM_LIFETIME`] for why the pool only frees on
/// device drop). Measured peak with several warm: 2.0 GB of process footprint.
///
/// ponytail: `Weak`, not `Arc`, and no refcount of our own — the cache must not
/// be the thing that keeps the model alive, or every holder's idle-drop would
/// stop freeing anything. The last holder to drop its `Arc` frees the weights and
/// the GPU pool; `upgrade()` returning `None` is exactly "nobody is using it".
pub(super) static SHARED_EMBEDDER: Mutex<Weak<LoadedEmbedder>> = Mutex::new(Weak::new());

/// Load the embedder for `descriptor` from its install directory under
/// `semantic_search_models/{provider}/{model_id}/`.
///
/// The candle backend (built inside `load_from_dir`) reads everything it needs —
/// architecture, dimension, window, pooling, on-disk layout — from the one
/// `descriptor`. There is no ONNX intra-op thread cap to resolve (ADR 0037):
/// candle runs the forward on the Metal GPU on macOS / candle-CPU elsewhere, with
/// no thread-pool spin-wait to clamp.
///
/// Returns the [`SHARED_EMBEDDER`] instance when one is already alive for this
/// exact provider/model id and is not [stale](LoadedEmbedder::is_stale), so a
/// second BACKGROUND caller (the Subject Vector backfill, the subject candidate
/// scorer) reuses the loaded weights and the one candle device instead of building
/// its own. Otherwise it loads a fresh one and publishes it as the shared instance.
///
/// **For holders that take [`BACKGROUND_EMBED_SLOT`] only.** Sharing an instance
/// means sharing its one candle `Device`, and candle 0.10.2's `MetalDevice` funnels
/// every kernel dispatch through a single `commands: Arc<RwLock<Commands>>` — both
/// `command_encoder()` and `wait_until_completed()` take the WRITE lock — so two
/// threads embedding on one instance serialize at the device, not merely at the
/// GPU. That is free between holders the permit already serializes, and a hard
/// latency regression for one that does not. The user-facing query path therefore
/// uses [`load_embedder_private`]; see its docs for the measurement. The exception
/// is a [single-copy](single_copy_only) model, where a search shares this instance
/// and accepts that latency rather than loading the weights twice.
///
/// Callers must keep holding the returned `Arc` for exactly as long as they want
/// the model resident — dropping every `Arc` is what frees the weights and, on
/// macOS, candle's Metal buffer pool.
///
/// The load runs under the shared lock on purpose: a concurrent second caller
/// waits and then reuses the winner's instance rather than racing it into a
/// duplicate multi-hundred-MB load. Every call site is already on a blocking
/// thread, so this never parks the tokio reactor.
pub fn load_embedder(
    app_data_dir: &std::path::Path,
    descriptor: &SemanticSearchModelDescriptor,
) -> Result<Arc<LoadedEmbedder>, String> {
    let mut shared = SHARED_EMBEDDER
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if let Some(existing) = shared.upgrade() {
        if shared_embedder_is_reusable(
            &existing.provider,
            &existing.model_id,
            existing.loaded_at.elapsed(),
            descriptor,
        ) {
            return Ok(existing);
        }
    }

    let loaded = load_embedder_private(app_data_dir, descriptor)?;
    *shared = Arc::downgrade(&loaded);
    Ok(loaded)
}

/// The [`SHARED_EMBEDDER`] instance for `descriptor` if one is already alive and
/// [reusable](shared_embedder_is_reusable); never loads. For the query path, which
/// may embed on it only while holding [`BACKGROUND_EMBED_SLOT`] (see
/// `semantic_search_query::borrow_when_free`).
pub fn warm_shared_embedder(
    descriptor: &SemanticSearchModelDescriptor,
) -> Option<Arc<LoadedEmbedder>> {
    SHARED_EMBEDDER
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .upgrade()
        .filter(|existing| {
            shared_embedder_is_reusable(
                &existing.provider,
                &existing.model_id,
                existing.loaded_at.elapsed(),
                descriptor,
            )
        })
}

/// Models above this download size are never loaded twice (founder ruling
/// 2026-11-11): two copies of a 2 GB model is 4 GB. Every caller, search included,
/// shares the one instance, and a live instance is reused even past the warm cap —
/// a fresh load while another holder still has the old one would be the second
/// copy. The price is a search queueing behind a backfill batch on the one device
/// (see [`load_embedder_private`]) and a recycle that waits for all holders to let go.
pub const SINGLE_COPY_MODEL_BYTES: u64 = 1_000_000_000;

/// Whether `descriptor` is big enough that only one loaded copy may ever exist.
pub fn single_copy_only(descriptor: &SemanticSearchModelDescriptor) -> bool {
    descriptor.approx_download_bytes > SINGLE_COPY_MODEL_BYTES
}

/// Whether the [`SHARED_EMBEDDER`] instance can be handed to a caller asking for
/// `descriptor`: same provider AND same model id AND (not
/// [stale](LoadedEmbedder::stale_after) OR a [single-copy](single_copy_only)
/// model). Mirrors the query cache's
/// `cached_embedder_is_reusable` (semantic_search_query.rs) — `warm_for` is the
/// elapsed `Duration`, not the load `Instant`, for the same testability reason.
pub(super) fn shared_embedder_is_reusable(
    provider: &str,
    model_id: &str,
    warm_for: Duration,
    descriptor: &SemanticSearchModelDescriptor,
) -> bool {
    provider == descriptor.provider
        && model_id == descriptor.model_id
        && (!LoadedEmbedder::stale_after(warm_for) || single_copy_only(descriptor))
}

/// Load a **private** embedder instance — its own weights and, on macOS, its own
/// candle `Device` — bypassing [`SHARED_EMBEDDER`] entirely.
///
/// This is the query path's loader for when the background slot is busy (an idle
/// slot lets a search borrow the warm shared instance via [`warm_shared_embedder`]).
/// A search is the one embed caller that never waits for
/// [`BACKGROUND_EMBED_SLOT`] ("making a search wait behind a backfill batch is the
/// wrong trade"), so it is also the one caller for which embedding on the
/// background sweep's device without the slot turns that into a pessimisation:
/// instead of two command queues the GPU can interleave, the query's kernels queue
/// behind the sweep's on one `Commands` write lock, and its readback
/// `wait_until_completed()` waits on the sweep's outstanding command buffer too.
///
/// Measured on an M4 Air, `nomic-embed-text-v1.5` on Metal, with one thread looping
/// the worker's 16-anchor batch shape while a second times query embeds:
///
/// | query embedder     | p50    | p95    |
/// |--------------------|--------|--------|
/// | idle (no backfill) | 31 ms  | 38 ms  |
/// | own device         | 92 ms  | 233 ms |
/// | shared with sweep  | 310 ms | 995 ms |
///
/// The cost is one extra resident copy of the weights while a search is warm — the
/// query cache drops it three minutes after the last search (and at
/// [`MAX_EMBEDDER_WARM_LIFETIME`] under steady use), and the two background
/// workers still share one instance between them, so the four-devices-resident peak
/// the shared cache was added to fix does not come back.
pub fn load_embedder_private(
    app_data_dir: &std::path::Path,
    descriptor: &SemanticSearchModelDescriptor,
) -> Result<Arc<LoadedEmbedder>, String> {
    let models_dir = semantic_search_models_dir(app_data_dir);
    let install_dir = model_install_dir(&models_dir, &descriptor.provider, &descriptor.model_id)
        .map_err(|error| error.to_string())?;
    let embedder = SemanticSearchEmbedder::load_from_dir(&install_dir, descriptor)
        .map_err(|error| error.to_string())?;
    Ok(Arc::new(LoadedEmbedder {
        provider: descriptor.provider.clone(),
        model_id: descriptor.model_id.clone(),
        embedder,
        loaded_at: Instant::now(),
    }))
}
