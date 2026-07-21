//! Meeting detection — Core Audio mic-holds, not calendars (issue #177, ADR 0057).
//!
//! A **Meeting** is an allowlisted conferencing app holding the microphone for
//! at least the per-trigger minimum (default 5 min); **Meeting Ends** fires
//! when the mic stays released through the global release grace (default
//! 2 min, `app_settings` kv — the #182 Settings knob).
//!
//! Three layers, split so everything above the OS boundary unit-tests:
//! - [`capture_system_audio::snapshot_mic_holding_bundle_ids`] — the Core
//!   Audio read (that crate alone enables cidre's `core_audio` feature).
//! - [`MeetingDetector`] (this module) — a pure state machine over
//!   `(holders, now)` snapshots. Allowlist-agnostic: the caller filters
//!   holders first, which is the seam #180 extends (browser processes count as
//!   holders only with URL evidence).
//! - The worker + firing wiring ([`super::meeting_worker`]): cooldown/provider
//!   gate via the shared [`super::firing_decision`], then the Readiness Wait
//!   ([`super::readiness`]), then the shared run path ([`super::run`]).
//!
//! Detector events log under [`MEETING_LOG_PREFIX`] (`meeting-detect:`) for
//! manual drills, mirroring the `system-audio-tap:` style.

use std::collections::{BTreeMap, BTreeSet};

use super::{TriggerCondition, TriggerDefinition};

/// Greppable marker for every detector event in `rust.log`.
pub(crate) const MEETING_LOG_PREFIX: &str = "meeting-detect:";

/// Default per-trigger minimum meeting length ("Advanced Options").
const DEFAULT_MIN_MEETING_MINUTES: u32 = 5;

// ── Conferencing Allowlist (ADR 0057: app-curated, not user-editable) ────────

/// `(bundle id, display name)`. Browsers are deliberately absent — browser
/// meetings (Meet in Chrome) are #180's slice, gated on URL evidence.
pub(crate) const CONFERENCING_ALLOWLIST: &[(&str, &str)] = &[
    ("us.zoom.xos", "Zoom"),
    ("com.microsoft.teams2", "Microsoft Teams"),
    ("com.microsoft.teams", "Microsoft Teams"),
    ("com.tinyspeck.slackmacgap", "Slack"),
    ("com.apple.FaceTime", "FaceTime"),
    ("Cisco-Systems.Spark", "Webex"),
    ("com.hnc.Discord", "Discord"),
];

pub(crate) fn conferencing_display_name(bundle_id: &str) -> Option<&'static str> {
    CONFERENCING_ALLOWLIST
        .iter()
        .find(|(id, _)| *id == bundle_id)
        .map(|(_, name)| *name)
}

/// The allowlist filter between a raw mic-holder snapshot and the detector.
/// #180 extends exactly here: a browser bundle id joins the returned set only
/// while meeting-URL evidence marks its hold.
pub(crate) fn conferencing_holders(holders: &BTreeSet<String>) -> BTreeSet<String> {
    holders
        .iter()
        .filter(|bundle_id| conferencing_display_name(bundle_id).is_some())
        .cloned()
        .collect()
}

// ── The detector state machine (pure) ────────────────────────────────────────

/// Per-app hold state: Holding while the mic is held; Released while inside
/// the grace, still the same meeting if the app rejoins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HoldState {
    Holding { since_ms: i64 },
    Released { since_ms: i64, released_at_ms: i64 },
}

/// A finished mic hold: `[start_ms, end_ms]` is the meeting window handed to
/// the firing (end = the release instant, not release + grace). Whether it was
/// long enough to BE a Meeting is the per-trigger floor's call, not the
/// detector's — see [`trigger_floor_ms`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EndedHold {
    pub bundle_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

impl EndedHold {
    pub fn duration_ms(&self) -> i64 {
        self.end_ms - self.start_ms
    }
}

/// What one observation tick saw — logged for the drills, `Ended` drives
/// firings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MeetingEvent {
    HoldStarted { bundle_id: String, at_ms: i64 },
    /// Mic released; the grace countdown started. Not an end yet.
    HoldReleased { bundle_id: String, at_ms: i64 },
    /// Rejoined within the grace: the SAME meeting continues.
    HoldRejoined { bundle_id: String },
    Ended(EndedHold),
}

/// Pure state machine over `(allowlist-filtered holders, now)` snapshots.
/// One instance tracks every concurrent holder independently.
#[derive(Debug)]
pub(crate) struct MeetingDetector {
    grace_ms: i64,
    holds: BTreeMap<String, HoldState>,
}

impl MeetingDetector {
    pub fn new(grace_ms: i64) -> Self {
        Self {
            grace_ms,
            holds: BTreeMap::new(),
        }
    }

    /// The global grace is a Settings kv read fresh each tick; mid-flight
    /// changes apply to the next release judgement.
    pub fn set_grace_ms(&mut self, grace_ms: i64) {
        self.grace_ms = grace_ms;
    }

    /// Whether any hold is being tracked (drives the poll cadence).
    pub fn is_tracking(&self) -> bool {
        !self.holds.is_empty()
    }

    /// Forget everything (e.g. the last meeting_ends trigger was deleted
    /// mid-hold: with nobody to fire, a half-tracked meeting is noise).
    pub fn reset(&mut self) {
        self.holds.clear();
    }

    /// Feed one snapshot. `holders` must already be allowlist-filtered.
    pub fn observe(&mut self, holders: &BTreeSet<String>, now_ms: i64) -> Vec<MeetingEvent> {
        let mut events = Vec::new();

        for bundle_id in holders {
            match self.holds.get(bundle_id).copied() {
                None => {
                    self.holds
                        .insert(bundle_id.clone(), HoldState::Holding { since_ms: now_ms });
                    events.push(MeetingEvent::HoldStarted {
                        bundle_id: bundle_id.clone(),
                        at_ms: now_ms,
                    });
                }
                Some(HoldState::Holding { .. }) => {}
                Some(HoldState::Released {
                    since_ms,
                    released_at_ms,
                }) => {
                    if now_ms.saturating_sub(released_at_ms) < self.grace_ms {
                        // Drop-and-rejoin within grace: same meeting, original
                        // start (the gap is inside the meeting window).
                        self.holds
                            .insert(bundle_id.clone(), HoldState::Holding { since_ms });
                        events.push(MeetingEvent::HoldRejoined {
                            bundle_id: bundle_id.clone(),
                        });
                    } else {
                        // The grace had expired before this tick saw the new
                        // hold: end the old meeting, start a fresh one.
                        events.push(MeetingEvent::Ended(EndedHold {
                            bundle_id: bundle_id.clone(),
                            start_ms: since_ms,
                            end_ms: released_at_ms,
                        }));
                        self.holds
                            .insert(bundle_id.clone(), HoldState::Holding { since_ms: now_ms });
                        events.push(MeetingEvent::HoldStarted {
                            bundle_id: bundle_id.clone(),
                            at_ms: now_ms,
                        });
                    }
                }
            }
        }

        let mut resolved = Vec::new();
        for (bundle_id, state) in self.holds.iter_mut() {
            if holders.contains(bundle_id) {
                continue;
            }
            match *state {
                HoldState::Holding { since_ms } => {
                    *state = HoldState::Released {
                        since_ms,
                        released_at_ms: now_ms,
                    };
                    events.push(MeetingEvent::HoldReleased {
                        bundle_id: bundle_id.clone(),
                        at_ms: now_ms,
                    });
                }
                HoldState::Released {
                    since_ms,
                    released_at_ms,
                } => {
                    if now_ms.saturating_sub(released_at_ms) >= self.grace_ms {
                        events.push(MeetingEvent::Ended(EndedHold {
                            bundle_id: bundle_id.clone(),
                            start_ms: since_ms,
                            end_ms: released_at_ms,
                        }));
                        resolved.push(bundle_id.clone());
                    }
                }
            }
        }
        for bundle_id in resolved {
            self.holds.remove(&bundle_id);
        }

        events
    }
}

/// The per-trigger Meeting floor in ms — `None` for non-meeting conditions.
/// Sub-floor holds (dictation, a voice memo, a 2-minute call) never fire.
pub(crate) fn trigger_floor_ms(trigger: &TriggerDefinition) -> Option<i64> {
    match trigger.condition {
        TriggerCondition::MeetingEnds { min_meeting_minutes } => Some(
            i64::from(min_meeting_minutes.unwrap_or(DEFAULT_MIN_MEETING_MINUTES)) * 60_000,
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN_MS: i64 = 60_000;
    const GRACE_MS: i64 = 2 * MIN_MS;
    const ZOOM: &str = "us.zoom.xos";

    fn holders(bundle_ids: &[&str]) -> BTreeSet<String> {
        bundle_ids.iter().map(|id| id.to_string()).collect()
    }

    fn ended_events(events: Vec<MeetingEvent>) -> Vec<EndedHold> {
        events
            .into_iter()
            .filter_map(|event| match event {
                MeetingEvent::Ended(ended) => Some(ended),
                _ => None,
            })
            .collect()
    }

    fn meeting_trigger(min_meeting_minutes: Option<u32>) -> TriggerDefinition {
        TriggerDefinition {
            id: "meeting-recap".to_string(),
            name: "Meeting Recap".to_string(),
            condition: TriggerCondition::MeetingEnds { min_meeting_minutes },
            prompt: "Recap the meeting.".to_string(),
            enabled: true,
            cooldown_minutes: None,
            version: 1,
        }
    }

    // ── Allowlist ────────────────────────────────────────────────────────────

    #[test]
    fn allowlist_keeps_conferencing_apps_and_drops_everything_else() {
        let raw = holders(&[
            ZOOM,
            "com.apple.FaceTime",
            // Non-conferencing mic grabbers must never reach the detector.
            "com.apple.VoiceMemos",
            "com.apple.Dictation",
            // Browsers are #180's slice (URL evidence), deliberately not here.
            "com.google.Chrome",
            "com.apple.Safari",
            "org.mozilla.firefox",
        ]);
        assert_eq!(
            conferencing_holders(&raw),
            holders(&[ZOOM, "com.apple.FaceTime"])
        );
    }

    #[test]
    fn allowlist_names_the_expected_apps_and_no_browsers() {
        for expected in [
            "us.zoom.xos",
            "com.microsoft.teams2",
            "com.microsoft.teams",
            "com.tinyspeck.slackmacgap",
            "com.apple.FaceTime",
            "Cisco-Systems.Spark",
            "com.hnc.Discord",
        ] {
            assert!(
                conferencing_display_name(expected).is_some(),
                "{expected} missing from the allowlist"
            );
        }
        for (bundle_id, _) in CONFERENCING_ALLOWLIST {
            let lower = bundle_id.to_ascii_lowercase();
            assert!(
                !lower.contains("chrome")
                    && !lower.contains("safari")
                    && !lower.contains("firefox")
                    && !lower.contains("zen"),
                "browsers must not be in the v1 allowlist (that is #180): {bundle_id}"
            );
        }
    }

    // ── Detector: grace ──────────────────────────────────────────────────────

    #[test]
    fn release_through_grace_ends_the_meeting_at_the_release_instant() {
        let mut detector = MeetingDetector::new(GRACE_MS);
        assert_eq!(
            detector.observe(&holders(&[ZOOM]), 0),
            vec![MeetingEvent::HoldStarted {
                bundle_id: ZOOM.to_string(),
                at_ms: 0
            }]
        );
        // Held for 10 minutes, then released.
        assert!(ended_events(detector.observe(&holders(&[ZOOM]), 5 * MIN_MS)).is_empty());
        assert_eq!(
            detector.observe(&holders(&[]), 10 * MIN_MS),
            vec![MeetingEvent::HoldReleased {
                bundle_id: ZOOM.to_string(),
                at_ms: 10 * MIN_MS
            }]
        );
        // Inside the grace: nothing ends yet.
        assert!(ended_events(detector.observe(&holders(&[]), 11 * MIN_MS)).is_empty());
        // Grace elapses: the meeting window is the HOLD window — end is the
        // release instant, not release + grace.
        assert_eq!(
            ended_events(detector.observe(&holders(&[]), 12 * MIN_MS)),
            vec![EndedHold {
                bundle_id: ZOOM.to_string(),
                start_ms: 0,
                end_ms: 10 * MIN_MS
            }]
        );
        assert!(!detector.is_tracking());
    }

    #[test]
    fn drop_and_rejoin_within_grace_is_one_meeting() {
        let mut detector = MeetingDetector::new(GRACE_MS);
        detector.observe(&holders(&[ZOOM]), 0);
        // Call drops at 10min, rejoins at 11min (inside the 2-min grace).
        detector.observe(&holders(&[]), 10 * MIN_MS);
        let events = detector.observe(&holders(&[ZOOM]), 11 * MIN_MS);
        assert_eq!(
            events,
            vec![MeetingEvent::HoldRejoined {
                bundle_id: ZOOM.to_string()
            }]
        );
        // Final release at 20min → ONE meeting spanning the whole thing.
        detector.observe(&holders(&[]), 20 * MIN_MS);
        assert_eq!(
            ended_events(detector.observe(&holders(&[]), 23 * MIN_MS)),
            vec![EndedHold {
                bundle_id: ZOOM.to_string(),
                start_ms: 0,
                end_ms: 20 * MIN_MS
            }]
        );
    }

    #[test]
    fn rehold_after_grace_expiry_ends_the_old_meeting_and_starts_a_new_one() {
        let mut detector = MeetingDetector::new(GRACE_MS);
        detector.observe(&holders(&[ZOOM]), 0);
        detector.observe(&holders(&[]), 10 * MIN_MS);
        // The next tick only arrives AFTER the grace expired, with Zoom already
        // holding again (back-to-back meetings across a coarse gap).
        let events = detector.observe(&holders(&[ZOOM]), 13 * MIN_MS);
        assert_eq!(
            events,
            vec![
                MeetingEvent::Ended(EndedHold {
                    bundle_id: ZOOM.to_string(),
                    start_ms: 0,
                    end_ms: 10 * MIN_MS
                }),
                MeetingEvent::HoldStarted {
                    bundle_id: ZOOM.to_string(),
                    at_ms: 13 * MIN_MS
                }
            ]
        );
        assert!(detector.is_tracking());
    }

    #[test]
    fn exact_grace_boundary_ends_the_meeting() {
        let mut detector = MeetingDetector::new(GRACE_MS);
        detector.observe(&holders(&[ZOOM]), 0);
        detector.observe(&holders(&[]), 10 * MIN_MS);
        // now - released_at == grace exactly → ended (>= semantics).
        assert_eq!(
            ended_events(detector.observe(&holders(&[]), 10 * MIN_MS + GRACE_MS)).len(),
            1
        );
    }

    // ── Detector: concurrent holders ─────────────────────────────────────────

    #[test]
    fn concurrent_holders_end_independently_with_their_own_windows() {
        let mut detector = MeetingDetector::new(GRACE_MS);
        detector.observe(&holders(&[ZOOM]), 0);
        detector.observe(&holders(&[ZOOM, "com.apple.FaceTime"]), 2 * MIN_MS);
        // Zoom releases at 10min; FaceTime keeps holding.
        detector.observe(&holders(&["com.apple.FaceTime"]), 10 * MIN_MS);
        let ended = ended_events(detector.observe(&holders(&["com.apple.FaceTime"]), 13 * MIN_MS));
        assert_eq!(
            ended,
            vec![EndedHold {
                bundle_id: ZOOM.to_string(),
                start_ms: 0,
                end_ms: 10 * MIN_MS
            }]
        );
        // FaceTime is still a live hold with its own start.
        detector.observe(&holders(&[]), 20 * MIN_MS);
        assert_eq!(
            ended_events(detector.observe(&holders(&[]), 23 * MIN_MS)),
            vec![EndedHold {
                bundle_id: "com.apple.FaceTime".to_string(),
                start_ms: 2 * MIN_MS,
                end_ms: 20 * MIN_MS
            }]
        );
    }

    // ── Floor (per-trigger) ──────────────────────────────────────────────────

    #[test]
    fn sub_floor_holds_never_fire() {
        // A 3-minute mic grab ends as a hold, but the 5-min default floor
        // rejects it — dictation and voice memos land here too (when they are
        // conferencing apps at all; others never pass the allowlist).
        let mut detector = MeetingDetector::new(GRACE_MS);
        detector.observe(&holders(&[ZOOM]), 0);
        detector.observe(&holders(&[]), 3 * MIN_MS);
        let ended = ended_events(detector.observe(&holders(&[]), 3 * MIN_MS + GRACE_MS));
        assert_eq!(ended.len(), 1);
        let floor = trigger_floor_ms(&meeting_trigger(None)).expect("meeting trigger has a floor");
        assert!(ended[0].duration_ms() < floor, "3 min is below the floor");
    }

    #[test]
    fn floor_is_met_at_exactly_the_minimum_and_respects_the_override() {
        let default_floor = trigger_floor_ms(&meeting_trigger(None)).unwrap();
        assert_eq!(default_floor, 5 * MIN_MS);
        // A meeting spanning the floor exactly fires (`<` rejects, so == passes).
        let mut detector = MeetingDetector::new(GRACE_MS);
        detector.observe(&holders(&[ZOOM]), 0);
        detector.observe(&holders(&[]), 5 * MIN_MS);
        let ended = ended_events(detector.observe(&holders(&[]), 5 * MIN_MS + GRACE_MS));
        assert!(ended[0].duration_ms() >= default_floor);

        // The Advanced override wins; Schedule triggers have no floor.
        assert_eq!(
            trigger_floor_ms(&meeting_trigger(Some(15))),
            Some(15 * MIN_MS)
        );
        assert_eq!(trigger_floor_ms(&super::super::tests::sample_daily()), None);
    }

    // ── Sleep gaps (the worker injects an empty observation at the last tick
    //    before the real post-wake observation — meeting_worker::gap_observation_ms)

    #[test]
    fn sub_floor_call_ending_during_long_sleep_does_not_clear_the_floor() {
        let mut detector = MeetingDetector::new(GRACE_MS);
        detector.observe(&holders(&[ZOOM]), 0);
        // Still on the call at the last real tick (3 min in), then lid close;
        // the call ends during sleep; wake an hour later.
        detector.observe(&holders(&[ZOOM]), 3 * MIN_MS);
        // Synthetic empty observation at the last tick: release stamped there.
        assert!(ended_events(detector.observe(&holders(&[]), 3 * MIN_MS)).is_empty());
        // Real post-wake observation: ends at the knowledge boundary, so the
        // 3-min call stays under the 5-min floor instead of absorbing the hour.
        let ended = ended_events(detector.observe(&holders(&[]), 63 * MIN_MS));
        assert_eq!(
            ended,
            vec![EndedHold {
                bundle_id: ZOOM.to_string(),
                start_ms: 0,
                end_ms: 3 * MIN_MS
            }]
        );
        let floor = trigger_floor_ms(&meeting_trigger(None)).unwrap();
        assert!(
            ended[0].duration_ms() < floor,
            "sleep must not inflate a 3-min call past the floor"
        );
    }

    #[test]
    fn ongoing_hold_across_a_short_gap_stays_one_meeting() {
        let mut detector = MeetingDetector::new(GRACE_MS);
        detector.observe(&holders(&[ZOOM]), 0);
        detector.observe(&holders(&[ZOOM]), 10 * MIN_MS);
        // 90s sleep: over the worker's 60s gap threshold, under the 2-min
        // grace. Synthetic empty at the last tick, then the wake observation
        // with the call still live → rejoin, same meeting.
        detector.observe(&holders(&[]), 10 * MIN_MS);
        assert_eq!(
            detector.observe(&holders(&[ZOOM]), 10 * MIN_MS + 90_000),
            vec![MeetingEvent::HoldRejoined {
                bundle_id: ZOOM.to_string()
            }]
        );
        // Final release: ONE meeting with the original start.
        detector.observe(&holders(&[]), 20 * MIN_MS);
        assert_eq!(
            ended_events(detector.observe(&holders(&[]), 23 * MIN_MS)),
            vec![EndedHold {
                bundle_id: ZOOM.to_string(),
                start_ms: 0,
                end_ms: 20 * MIN_MS
            }]
        );
    }

    #[test]
    fn overnight_gap_mid_hold_ends_the_meeting_at_the_pre_sleep_tick() {
        let mut detector = MeetingDetector::new(GRACE_MS);
        detector.observe(&holders(&[ZOOM]), 0);
        detector.observe(&holders(&[ZOOM]), 30 * MIN_MS);
        // Overnight sleep; at wake the app still holds the mic. The synthetic
        // empty observation at the pre-sleep tick ends the meeting there; the
        // ongoing hold re-holds fresh at wake.
        detector.observe(&holders(&[]), 30 * MIN_MS);
        assert_eq!(
            detector.observe(&holders(&[ZOOM]), 8 * 60 * MIN_MS),
            vec![
                MeetingEvent::Ended(EndedHold {
                    bundle_id: ZOOM.to_string(),
                    start_ms: 0,
                    end_ms: 30 * MIN_MS
                }),
                MeetingEvent::HoldStarted {
                    bundle_id: ZOOM.to_string(),
                    at_ms: 8 * 60 * MIN_MS
                }
            ]
        );
    }
}
