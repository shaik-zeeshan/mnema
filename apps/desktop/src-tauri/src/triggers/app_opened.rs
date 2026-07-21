//! The App Opened condition (issue #178): fires when a chosen app becomes
//! frontmost after ≥ the away gap (default 30 min) of NOT being frontmost.
//!
//! Activation events come from the existing NSWorkspace `did_activate_app`
//! observer in `native_capture_metadata.rs` — one extra guard on the SAME
//! notification center fans activations into this module's channel
//! ([`publish_activation`]), so capture metadata/privacy behavior is untouched
//! and observer lifecycle stays in one place.
//!
//! The gap logic is a pure state machine ([`AppOpenedTracker`]): every
//! activation of ANY app is an input. Per app, "last frontmost" ends the
//! instant a DIFFERENT app activates and displaces it — NSWorkspace only fires
//! on transitions, so an app that stays frontmost produces no events and never
//! accrues away time. Consequences, deliberately:
//! - Cmd-tab churn never fires: every frontmost moment restamps the
//!   displacement time, so the away span keeps resetting.
//! - First-ever activation (no recorded last-frontmost — including right after
//!   a Mnema restart, state is in-memory) fires: it IS a fresh session as far
//!   as we can observe. CONTEXT.md is silent on this; the persisted Cooldown
//!   keeps a restart from re-firing within the window.
//! - Sleep counts toward the away gap only if the app was NOT frontmost when
//!   sleep began (timestamps are wall-clock). If it WAS frontmost the whole
//!   time, a wake re-activation is a same-frontmost continuation
//!   ([`Activation::Continuing`]) and never fires — the user never left.
//!
//! Firing mirrors the meeting worker: shared decision
//! ([`super::firing_decision`]) with the event cooldown anchor
//! ([`super::event_cooldown_anchor_ms`]) → claim (`set_last_fired_ms`) → the
//! shared run path ([`super::run`]). No Readiness Wait — nothing to transcribe.

use std::collections::HashMap;
use std::sync::OnceLock;

use super::run::{AppOpenedFiringContext, EventFiringContext};
use super::{FiringDecision, TriggerCondition, TriggerDefinition};
use crate::app_infra::{AppInfraState, BackgroundWorkersState};
use crate::user_context::worker::now_ms;

/// Greppable marker for every app-opened event in `rust.log`.
pub(crate) const APP_OPENED_LOG_PREFIX: &str = "app-opened:";

/// Default per-trigger away gap ("Advanced Options").
const DEFAULT_AWAY_GAP_MINUTES: u32 = 30;

/// The trigger's away gap in ms, `None` for non-app-opened conditions.
pub(crate) fn away_gap_ms(trigger: &TriggerDefinition) -> Option<i64> {
    match trigger.condition {
        TriggerCondition::AppOpened {
            away_gap_minutes, ..
        } => Some(i64::from(away_gap_minutes.unwrap_or(DEFAULT_AWAY_GAP_MINUTES)) * 60_000),
        _ => None,
    }
}

// ── The gap state machine (pure) ─────────────────────────────────────────────

/// What one activation observation means for the activated app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Activation {
    /// The app was already frontmost (duplicate/self re-activation, e.g. a
    /// wake re-delivery): the session continues, never a fire.
    Continuing,
    /// The app just became frontmost. `last_frontmost_end_ms` is the instant
    /// it was last displaced by another app; `None` = first ever observed.
    BecameFrontmost { last_frontmost_end_ms: Option<i64> },
}

/// Per-app last-frontmost tracking over the full activation stream.
/// ponytail: in-memory only and one entry per app ever displaced — bounded by
/// the user's installed apps; persist the map if restart-fires ever matter.
#[derive(Debug, Default)]
pub(crate) struct AppOpenedTracker {
    /// Bundle id of the most recent activation — the app that is (as far as
    /// the event stream knows) frontmost right now.
    prev_frontmost: Option<String>,
    /// Per bundle id: the instant it was last displaced from frontmost.
    last_frontmost_end: HashMap<String, i64>,
}

impl AppOpenedTracker {
    /// Feed one activation (any app). The displaced previous app gets its
    /// last-frontmost-end stamped at `at_ms` — that instant is when it stopped
    /// being frontmost.
    pub(crate) fn observe(&mut self, bundle_id: &str, at_ms: i64) -> Activation {
        let prev = self.prev_frontmost.replace(bundle_id.to_string());
        match prev {
            Some(prev_id) if prev_id == bundle_id => Activation::Continuing,
            Some(prev_id) => {
                self.last_frontmost_end.insert(prev_id, at_ms);
                Activation::BecameFrontmost {
                    last_frontmost_end_ms: self.last_frontmost_end.get(bundle_id).copied(),
                }
            }
            None => Activation::BecameFrontmost {
                last_frontmost_end_ms: self.last_frontmost_end.get(bundle_id).copied(),
            },
        }
    }
}

/// Whether an activation starts a fresh session for a given gap:
/// `None` = not fresh (away < gap — churn). `Some(last_frontmost_end_ms)` =
/// fire, carrying the away window's start (`None` inside = first observed).
pub(crate) fn fresh_session(
    last_frontmost_end_ms: Option<i64>,
    at_ms: i64,
    gap_ms: i64,
) -> Option<Option<i64>> {
    match last_frontmost_end_ms {
        None => Some(None),
        Some(end_ms) => (at_ms.saturating_sub(end_ms) >= gap_ms).then_some(Some(end_ms)),
    }
}

// ── Observer fan-out channel ─────────────────────────────────────────────────

/// One frontmost activation from the NSWorkspace observer.
#[derive(Debug)]
pub(crate) struct AppActivation {
    pub bundle_id: String,
    pub at_ms: i64,
}

static ACTIVATION_TX: OnceLock<tokio::sync::mpsc::UnboundedSender<AppActivation>> =
    OnceLock::new();

/// Called from the `did_activate_app` observer (main thread, must stay cheap):
/// stamp now and hand off to the worker. Activations before the worker spawns
/// (deferred startup) are dropped — the first one after behaves as
/// first-observed, which is the documented fresh-session semantics.
pub(crate) fn publish_activation(bundle_id: Option<String>) {
    let Some(bundle_id) = bundle_id.filter(|id| !id.trim().is_empty()) else {
        return;
    };
    if let Some(tx) = ACTIVATION_TX.get() {
        let _ = tx.send(AppActivation {
            bundle_id,
            at_ms: now_ms(),
        });
    }
}

// ── Worker ───────────────────────────────────────────────────────────────────

/// Spawn the app-opened worker: consumes the activation channel, keeps the
/// tracker, and fires matching triggers. Same shutdown pattern as the sibling
/// trigger workers. macOS-only in effect — nothing publishes elsewhere.
pub fn spawn_app_opened_worker(
    infra: AppInfraState,
    app_handle: tauri::AppHandle,
    background_workers: BackgroundWorkersState,
) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    if ACTIVATION_TX.set(tx).is_err() {
        tauri_plugin_log::log::warn!(
            "{APP_OPENED_LOG_PREFIX} activation channel already installed; not spawning twice"
        );
        return;
    }
    let mut shutdown_rx = background_workers.subscribe();
    crate::native_capture::debug_log::log_info("starting app-opened trigger worker");
    let handle = tauri::async_runtime::spawn(async move {
        let mut tracker = AppOpenedTracker::default();
        loop {
            if *shutdown_rx.borrow() {
                break;
            }
            tokio::select! {
                changed = shutdown_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
                event = rx.recv() => {
                    let Some(event) = event else { break };
                    handle_activation(&mut tracker, event, &infra, &app_handle).await;
                }
            }
        }
        crate::native_capture::debug_log::log_info("stopped app-opened trigger worker");
    });
    background_workers.track(handle);
}

/// One activation through the tracker, then decide + fire every enabled
/// app_opened trigger watching this bundle id.
async fn handle_activation(
    tracker: &mut AppOpenedTracker,
    event: AppActivation,
    infra: &AppInfraState,
    app_handle: &tauri::AppHandle,
) {
    // The tracker eats EVERY activation (displacements are what end a watched
    // app's frontmost span), independent of what triggers exist right now.
    let observed = tracker.observe(&event.bundle_id, event.at_ms);
    let Activation::BecameFrontmost {
        last_frontmost_end_ms,
    } = observed
    else {
        return;
    };

    let watching: Vec<TriggerDefinition> = super::load_triggers(app_handle)
        .into_iter()
        .filter(|trigger| {
            trigger.enabled
                && matches!(
                    &trigger.condition,
                    TriggerCondition::AppOpened { bundle_id, .. } if *bundle_id == event.bundle_id
                )
        })
        .collect();
    if watching.is_empty() {
        return;
    }

    let now = now_ms();
    for trigger in watching {
        let (Some(gap_ms), TriggerCondition::AppOpened { app_name, .. }) =
            (away_gap_ms(&trigger), &trigger.condition)
        else {
            continue;
        };
        let Some(since_ms) = fresh_session(last_frontmost_end_ms, event.at_ms, gap_ms) else {
            tauri_plugin_log::log::debug!(
                "{APP_OPENED_LOG_PREFIX} {} re-activated within trigger '{}' gap; not a fresh session",
                event.bundle_id,
                trigger.id
            );
            continue;
        };
        let ledger_ms = infra
            .trigger_firings()
            .last_firing(&trigger.id)
            .await
            .ok()
            .flatten()
            .map(|firing| firing.fired_at_ms);
        let claim_cursor_ms = infra
            .trigger_state()
            .last_fired_ms(&trigger.id)
            .await
            .ok()
            .flatten();
        let last_firing_ms = super::event_cooldown_anchor_ms(ledger_ms, claim_cursor_ms);
        let provider_ready = crate::ask_ai::ensure_ask_ai_access_ready(app_handle)
            .await
            .is_ok();
        match super::firing_decision(
            Some(event.at_ms),
            last_firing_ms,
            trigger.cooldown_ms(),
            provider_ready,
            now,
        ) {
            FiringDecision::NotDue => continue,
            FiringDecision::CooldownSuppressed => {
                // An activation event is one-shot: suppressed means dropped —
                // exactly what Cooldown is for (a crash-looping app re-opening
                // cannot spam runs).
                tauri_plugin_log::log::info!(
                    "{APP_OPENED_LOG_PREFIX} trigger '{}' cooling down; activation dropped",
                    trigger.id
                );
            }
            FiringDecision::NeedsProvider => {
                tauri_plugin_log::log::info!(
                    "{APP_OPENED_LOG_PREFIX} trigger '{}' needs an AI provider; activation dropped",
                    trigger.id
                );
            }
            FiringDecision::Fire { .. } => {
                if let Err(error) = infra.trigger_state().set_last_fired_ms(&trigger.id, now).await
                {
                    tauri_plugin_log::log::warn!(
                        "{APP_OPENED_LOG_PREFIX} failed to record firing for trigger '{}': {error}; not running",
                        trigger.id
                    );
                    continue;
                }
                tauri_plugin_log::log::info!(
                    "{APP_OPENED_LOG_PREFIX} firing trigger '{}' for {} session at {}",
                    trigger.id,
                    event.bundle_id,
                    event.at_ms
                );
                spawn_app_opened_firing(
                    std::sync::Arc::clone(infra),
                    app_handle.clone(),
                    app_name.clone(),
                    trigger.clone(),
                    since_ms,
                    event.at_ms,
                );
            }
        }
    }
}

/// The run as its own task so a multi-minute AI turn never blocks the
/// activation loop. Deliberately untracked, mirroring the meeting firing: a
/// shutdown mid-run is the documented crash-mid-run semantics (occurrence
/// claimed, run quietly missed).
fn spawn_app_opened_firing(
    infra: AppInfraState,
    app_handle: tauri::AppHandle,
    app_display_name: String,
    trigger: TriggerDefinition,
    last_frontmost_end_ms: Option<i64>,
    occurrence_ms: i64,
) {
    tauri::async_runtime::spawn(async move {
        let offset_minutes = infra
            .user_context()
            .local_offset_minutes()
            .await
            .ok()
            .flatten()
            .map(|minutes| minutes as i32)
            .unwrap_or(0);
        let context = EventFiringContext::AppOpened(AppOpenedFiringContext {
            app_display_name,
            last_frontmost_end_ms,
        });
        super::run::run_trigger_fire(
            &app_handle,
            &infra,
            &trigger,
            occurrence_ms,
            offset_minutes,
            Some(&context),
        )
        .await;
    });
}

#[cfg(test)]
mod tests {
    use super::super::tests::sample_daily;
    use super::*;

    const MIN_MS: i64 = 60_000;
    const GAP: i64 = 30 * MIN_MS;

    fn watched(bundle: &str, gap_minutes: Option<u32>) -> TriggerDefinition {
        TriggerDefinition {
            condition: TriggerCondition::AppOpened {
                bundle_id: bundle.to_string(),
                app_name: "Figma".to_string(),
                away_gap_minutes: gap_minutes,
            },
            ..sample_daily()
        }
    }

    #[test]
    fn away_gap_defaults_to_30_minutes_and_honors_the_override() {
        assert_eq!(away_gap_ms(&watched("com.figma.Desktop", None)), Some(GAP));
        assert_eq!(
            away_gap_ms(&watched("com.figma.Desktop", Some(120))),
            Some(120 * MIN_MS)
        );
        // Non-app-opened conditions have no gap.
        assert_eq!(away_gap_ms(&sample_daily()), None);
    }

    #[test]
    fn first_ever_activation_is_a_fresh_session() {
        let mut tracker = AppOpenedTracker::default();
        // No recorded last-frontmost: fire — it IS a fresh session as far as
        // we can observe (documented choice; cooldown guards restarts).
        assert_eq!(
            tracker.observe("com.figma.Desktop", 1_000_000),
            Activation::BecameFrontmost {
                last_frontmost_end_ms: None
            }
        );
        assert_eq!(fresh_session(None, 1_000_000, GAP), Some(None));
    }

    #[test]
    fn cmd_tab_churn_never_fires() {
        let mut tracker = AppOpenedTracker::default();
        let t0 = 1_000_000;
        tracker.observe("com.figma.Desktop", t0);
        // Rapid A→B→A→B→A over a few seconds: every return to Figma sees a
        // last-frontmost-end only seconds old — never a fresh session.
        for i in 1..=4 {
            let t = t0 + i * 3_000;
            let bundle = if i % 2 == 1 {
                "com.tinyspeck.slackmacgap"
            } else {
                "com.figma.Desktop"
            };
            let observed = tracker.observe(bundle, t);
            if bundle == "com.figma.Desktop" {
                let Activation::BecameFrontmost {
                    last_frontmost_end_ms,
                } = observed
                else {
                    panic!("expected BecameFrontmost");
                };
                assert_eq!(fresh_session(last_frontmost_end_ms, t, GAP), None);
            }
        }
    }

    #[test]
    fn returning_after_at_least_the_gap_fires_with_the_away_span() {
        let mut tracker = AppOpenedTracker::default();
        let t0 = 1_000_000;
        tracker.observe("com.figma.Desktop", t0);
        // Displaced at t1: that instant — not the activation at t0 — is when
        // Figma stopped being frontmost.
        let t1 = t0 + 45 * MIN_MS;
        tracker.observe("com.tinyspeck.slackmacgap", t1);
        // Return exactly at the gap boundary: >= fires.
        let back = t1 + GAP;
        assert_eq!(
            tracker.observe("com.figma.Desktop", back),
            Activation::BecameFrontmost {
                last_frontmost_end_ms: Some(t1)
            }
        );
        assert_eq!(fresh_session(Some(t1), back, GAP), Some(Some(t1)));
        // One millisecond earlier: churn, no fire.
        assert_eq!(fresh_session(Some(t1), back - 1, GAP), None);
    }

    #[test]
    fn continuing_frontmost_never_fires() {
        let mut tracker = AppOpenedTracker::default();
        tracker.observe("com.figma.Desktop", 1_000_000);
        // A duplicate self-activation (e.g. wake re-delivery hours later) is a
        // continuation of the same session — the user never left the app, so
        // sleep with the app frontmost never fires.
        assert_eq!(
            tracker.observe("com.figma.Desktop", 1_000_000 + 8 * 60 * MIN_MS),
            Activation::Continuing
        );
    }

    #[test]
    fn watched_apps_track_independently() {
        let mut tracker = AppOpenedTracker::default();
        let t0 = 1_000_000;
        tracker.observe("com.figma.Desktop", t0);
        let t1 = t0 + MIN_MS;
        tracker.observe("com.apple.dt.Xcode", t1); // Figma displaced at t1
        let t2 = t1 + 2 * GAP;
        tracker.observe("com.apple.finder", t2); // Xcode displaced at t2
        // Figma returns after 2×gap since ITS displacement: fires from t1.
        let t3 = t2 + MIN_MS;
        assert_eq!(
            tracker.observe("com.figma.Desktop", t3),
            Activation::BecameFrontmost {
                last_frontmost_end_ms: Some(t1)
            }
        );
        assert_eq!(fresh_session(Some(t1), t3, GAP), Some(Some(t1)));
        // Xcode returns only two minutes after ITS displacement (at t2, by
        // Finder): churn — Figma's fresh session did not disturb it.
        let t4 = t3 + MIN_MS;
        assert_eq!(
            tracker.observe("com.apple.dt.Xcode", t4),
            Activation::BecameFrontmost {
                last_frontmost_end_ms: Some(t2)
            }
        );
        assert_eq!(fresh_session(Some(t2), t4, GAP), None);
    }

    #[test]
    fn cooldown_on_top_suppresses_a_crash_looping_app() {
        // A crash-looping watched app (with a tiny/zero gap) produces fresh
        // sessions back to back; the FIRST claims (`set_last_fired_ms`) while
        // its run is still in flight (no ledger row yet), so the second
        // activation 3 min later must be suppressed via the claim cursor.
        let claim = 1_000_000_i64;
        let second = claim + 3 * MIN_MS;
        assert_eq!(
            super::super::firing_decision(
                Some(second),
                super::super::event_cooldown_anchor_ms(None, Some(claim)),
                10 * MIN_MS,
                true,
                second,
            ),
            FiringDecision::CooldownSuppressed
        );
        // Past the cooldown the next fresh session fires again.
        let later = claim + 11 * MIN_MS;
        assert_eq!(
            super::super::firing_decision(
                Some(later),
                super::super::event_cooldown_anchor_ms(None, Some(claim)),
                10 * MIN_MS,
                true,
                later,
            ),
            FiringDecision::Fire {
                occurrence_ms: later
            }
        );
    }
}
