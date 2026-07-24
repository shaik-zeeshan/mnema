//! Browser meetings — evidence-sticky meeting-URL detection (issue #180, ADR 0057).
//!
//! A browser process holding the mic is only a Meeting when a known meeting URL
//! was sighted at some point during the hold. The probe (the existing
//! browser-URL machinery: Chromium/WebKit AppleScript, Gecko AX) only sees the
//! active tab of the front window — meeting tabs get backgrounded — so evidence
//! is **sticky**: one sighting marks the WHOLE hold as a Meeting, and the
//! meeting window is the whole mic hold (true hold start, not first-sighting
//! time).
//!
//! [`BrowserMeetingTracker`] is the pure worker-side companion to
//! [`MeetingDetector`](super::meeting::MeetingDetector): it tracks true hold
//! starts from the RAW (unfiltered) mic snapshot, names the probe targets for
//! each tick, records sticky evidence, and decides which browsers join the
//! detector's holder set (the `conferencing_holders` seam). Evidence lives
//! until the detector reports the hold **Ended** — surviving release-within-
//! grace so drop-and-rejoin keeps the sticky mark — and never carries into the
//! next hold.

use std::collections::{BTreeMap, BTreeSet};

// ── Meeting URL Allowlist (app-shipped, like the Conferencing Allowlist) ─────

/// `(host, required path prefix, service display name)`. The host matches
/// exactly or as a suffix on a `.`-label boundary (`us05web.zoom.us` matches
/// `zoom.us`; `meet.google.com.evil.com` must NOT match `meet.google.com`).
const MEETING_URL_ALLOWLIST: &[(&str, Option<&str>, &str)] = &[
    ("meet.google.com", None, "Google Meet"),
    ("zoom.us", Some("/j/"), "Zoom"),
    ("zoom.us", Some("/wc"), "Zoom"), // app.zoom.us/wc — the Zoom web client
    ("teams.microsoft.com", None, "Microsoft Teams"),
    ("teams.live.com", None, "Microsoft Teams"),
    ("whereby.com", None, "Whereby"),
    ("discord.com", Some("/channels"), "Discord"),
    ("around.co", None, "Around"),
    ("gather.town", None, "Gather"),
    ("webex.com", None, "Webex"), // company sites are *.webex.com; mic-hold gates the rest
];

/// The service display name for a meeting URL, or `None` when the URL is not a
/// known meeting surface. Host matching is exact-or-label-boundary-suffix —
/// never substring — so `meet.google.com.evil.com` and `evilmeet.google.com`
/// don't pass.
pub(crate) fn meeting_url_service(raw_url: &str) -> Option<&'static str> {
    let parsed = url::Url::parse(raw_url.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    let host = host.trim_end_matches('.');
    let path = parsed.path();
    MEETING_URL_ALLOWLIST
        .iter()
        .find_map(|(pattern_host, path_prefix, service)| {
            let host_matches = host == *pattern_host
                || host
                    .strip_suffix(pattern_host)
                    .is_some_and(|prefix| prefix.ends_with('.'));
            let path_matches = path_prefix.map_or(true, |prefix| path.starts_with(prefix));
            (host_matches && path_matches).then_some(*service)
        })
}

// ── Sticky evidence ──────────────────────────────────────────────────────────

/// The sticky mark on a browser mic hold: the first meeting-URL sighting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MeetingUrlEvidence {
    pub url: String,
    pub service: &'static str,
}

/// Pure per-tick state for browser mic holds. The worker feeds it the raw
/// holder snapshot and any probe sightings; it answers what to probe and which
/// browsers count as conferencing holders for the detector.
#[derive(Debug, Default)]
pub(crate) struct BrowserMeetingTracker {
    /// True mic-hold start per browser bundle id — tracked from the FIRST tick
    /// that saw the hold, so evidence appearing minutes later still yields the
    /// whole-hold meeting window.
    hold_starts: BTreeMap<String, i64>,
    /// Sticky evidence per browser bundle id; cleared only by [`Self::take_ended`].
    evidence: BTreeMap<String, MeetingUrlEvidence>,
}

impl BrowserMeetingTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed the RAW (unfiltered) holder snapshot for one tick. Tracks hold
    /// starts for known browsers and returns this tick's probe targets:
    /// browsers currently holding the mic WITHOUT evidence yet. Evidence is
    /// sticky, so an evidence-backed hold is never probed again (cost control),
    /// and a browser that released pre-evidence stops being tracked — its next
    /// hold starts fresh, so stale sightings can't attach to it.
    pub fn observe_raw_holders(
        &mut self,
        raw_holders: &BTreeSet<String>,
        now_ms: i64,
    ) -> Vec<String> {
        let browser_holders: BTreeSet<&String> = raw_holders
            .iter()
            .filter(|bundle_id| capture_metadata::is_known_browser_bundle(bundle_id))
            .collect();
        for bundle_id in &browser_holders {
            self.hold_starts
                .entry((*bundle_id).clone())
                .or_insert(now_ms);
        }
        // Evidence-backed holds are the detector's to end (their start must
        // survive the released-within-grace window); pre-evidence releases just
        // stop being tracked.
        let Self {
            hold_starts,
            evidence,
        } = self;
        hold_starts.retain(|bundle_id, _| {
            browser_holders.contains(bundle_id) || evidence.contains_key(bundle_id)
        });
        browser_holders
            .into_iter()
            .filter(|bundle_id| !self.evidence.contains_key(*bundle_id))
            .cloned()
            .collect()
    }

    /// A URL sighting from the probe. Records sticky evidence when the URL is a
    /// known meeting surface AND the browser's hold is currently tracked (a
    /// stale probe result from before the hold can't mark it). Returns the
    /// matched service on the first (marking) sighting.
    pub fn record_sighting(&mut self, bundle_id: &str, raw_url: &str) -> Option<&'static str> {
        if !self.hold_starts.contains_key(bundle_id) || self.evidence.contains_key(bundle_id) {
            return None;
        }
        let service = meeting_url_service(raw_url)?;
        self.evidence.insert(
            bundle_id.to_string(),
            MeetingUrlEvidence {
                url: raw_url.trim().to_string(),
                service,
            },
        );
        Some(service)
    }

    /// Browsers that join the detector's holder set this tick: evidence-backed
    /// AND currently holding the mic. (An evidence-backed browser that released
    /// is deliberately absent — the detector runs its grace on the absence.)
    pub fn evidence_backed_holders(&self, raw_holders: &BTreeSet<String>) -> BTreeSet<String> {
        self.evidence
            .keys()
            .filter(|bundle_id| raw_holders.contains(*bundle_id))
            .cloned()
            .collect()
    }

    /// The true mic-hold start — what the detector gets seeded with so the
    /// meeting window is the whole hold, not evidence-onward.
    pub fn hold_start_ms(&self, bundle_id: &str) -> Option<i64> {
        self.hold_starts.get(bundle_id).copied()
    }

    /// The detector reported this hold Ended: clear its state so the NEXT hold
    /// needs fresh evidence. Returns the evidence for the firing context (also
    /// `None` for conferencing-app holds, which this tracker never sees).
    pub fn take_ended(&mut self, bundle_id: &str) -> Option<MeetingUrlEvidence> {
        self.hold_starts.remove(bundle_id);
        self.evidence.remove(bundle_id)
    }

    /// Sleep-gap knowledge boundary (see `meeting_worker::gap_observation_ms`):
    /// pre-evidence hold starts are stale after a sleep — the mic state across
    /// the gap is unknown, so a post-wake hold must not backdate to pre-sleep.
    /// Evidence-backed holds stay: the detector owns their gap semantics.
    pub fn clear_pre_evidence_holds(&mut self) {
        let Self {
            hold_starts,
            evidence,
        } = self;
        hold_starts.retain(|bundle_id, _| evidence.contains_key(bundle_id));
    }

    /// Whether any browser hold is being watched (drives the poll cadence —
    /// probing needs the 5s tick even before the detector tracks anything).
    pub fn is_tracking(&self) -> bool {
        !self.hold_starts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::super::meeting::{
        conferencing_holders, trigger_floor_ms, EndedHold, MeetingDetector, MeetingEvent,
    };
    use super::*;

    const MIN_MS: i64 = 60_000;
    const GRACE_MS: i64 = 2 * MIN_MS;
    const CHROME: &str = "com.google.Chrome";
    const ZOOM_APP: &str = "us.zoom.xos";
    const MEET_URL: &str = "https://meet.google.com/abc-defg-hij";

    // ── URL pattern matching ─────────────────────────────────────────────────

    #[test]
    fn meeting_urls_match_positively() {
        for (url, service) in [
            ("https://meet.google.com/abc-defg-hij", "Google Meet"),
            ("https://MEET.GOOGLE.COM/abc", "Google Meet"),
            ("https://zoom.us/j/123456789", "Zoom"),
            ("https://us05web.zoom.us/j/123?pwd=x", "Zoom"),
            ("https://app.zoom.us/wc/join/123", "Zoom"),
            ("https://teams.microsoft.com/l/meetup-join/x", "Microsoft Teams"),
            ("https://teams.live.com/meet/9312...", "Microsoft Teams"),
            ("https://whereby.com/my-room", "Whereby"),
            ("https://subdomain.whereby.com/room", "Whereby"),
            ("https://discord.com/channels/123/456", "Discord"),
            ("https://around.co/r/room", "Around"),
            ("https://app.gather.town/app/xyz", "Gather"),
            ("https://company.webex.com/meet/alice", "Webex"),
            ("http://meet.google.com/plain-http", "Google Meet"),
            // A port keeps the host intact — still Google's host.
            ("https://meet.google.com:8443/abc", "Google Meet"),
        ] {
            assert_eq!(meeting_url_service(url), Some(service), "{url}");
        }
    }

    #[test]
    fn non_meeting_urls_never_match() {
        for url in [
            "https://google.com",
            "https://www.google.com/search?q=meet",
            "https://docs.google.com/document/d/1",
            // Suffix attacks: label-boundary matching, never substring.
            "https://meet.google.com.evil.com/abc",
            "https://evilmeet.google.com/abc",
            "https://notzoom.us/j/123",
            // Userinfo trick: the WHATWG host here is evil.com, not meet.google.com.
            "https://meet.google.com@evil.com/abc",
            // Right host, missing required path prefix.
            "https://zoom.us/",
            "https://zoom.us/pricing",
            "https://discord.com/",
            "https://discord.com/login",
            // Non-web schemes and garbage.
            "ftp://meet.google.com/x",
            "file:///Users/me/meet.google.com.html",
            "not a url",
            "",
        ] {
            assert_eq!(meeting_url_service(url), None, "{url}");
        }
    }

    // ── Tracker + detector, driven exactly like the worker tick ─────────────

    fn raw(bundle_ids: &[&str]) -> BTreeSet<String> {
        bundle_ids.iter().map(|id| id.to_string()).collect()
    }

    /// One worker-shaped tick: raw snapshot in, optional probe sighting,
    /// detector set = conferencing ∪ evidence-backed browsers (seeded with
    /// their true hold start), Ended events drain tracker state. Returns the
    /// Ended holds paired with the evidence taken for them.
    fn tick(
        detector: &mut MeetingDetector,
        tracker: &mut BrowserMeetingTracker,
        raw_holders: &BTreeSet<String>,
        sighting: Option<(&str, &str)>,
        now_ms: i64,
    ) -> Vec<(EndedHold, Option<MeetingUrlEvidence>)> {
        let _probe_targets = tracker.observe_raw_holders(raw_holders, now_ms);
        if let Some((bundle_id, url)) = sighting {
            tracker.record_sighting(bundle_id, url);
        }
        let mut detector_set = conferencing_holders(raw_holders);
        for bundle_id in tracker.evidence_backed_holders(raw_holders) {
            if let Some(start_ms) = tracker.hold_start_ms(&bundle_id) {
                detector.seed_hold(&bundle_id, start_ms);
            }
            detector_set.insert(bundle_id);
        }
        detector
            .observe(&detector_set, now_ms)
            .into_iter()
            .filter_map(|event| match event {
                MeetingEvent::Ended(ended) => {
                    let evidence = tracker.take_ended(&ended.bundle_id);
                    Some((ended, evidence))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn evidence_at_minute_two_yields_the_whole_hold_window() {
        let mut detector = MeetingDetector::new(GRACE_MS);
        let mut tracker = BrowserMeetingTracker::new();
        // Chrome grabs the mic at minute 0; the meeting tab is backgrounded, so
        // the first two ticks sight nothing.
        assert!(tick(&mut detector, &mut tracker, &raw(&[CHROME]), None, 0).is_empty());
        assert!(tick(&mut detector, &mut tracker, &raw(&[CHROME]), None, MIN_MS).is_empty());
        assert!(!detector.is_tracking(), "no evidence yet — detector must not see the browser");
        // Minute 2: the Meet tab surfaces once. Sticky from here on.
        assert!(tick(
            &mut detector,
            &mut tracker,
            &raw(&[CHROME]),
            Some((CHROME, MEET_URL)),
            2 * MIN_MS
        )
        .is_empty());
        assert!(detector.is_tracking());
        // Tab backgrounded again for the rest of the call; still a meeting.
        assert!(tick(&mut detector, &mut tracker, &raw(&[CHROME]), None, 6 * MIN_MS).is_empty());
        // Release at minute 10, grace runs out at 12.
        assert!(tick(&mut detector, &mut tracker, &raw(&[]), None, 10 * MIN_MS).is_empty());
        let ended = tick(&mut detector, &mut tracker, &raw(&[]), None, 12 * MIN_MS);
        assert_eq!(ended.len(), 1);
        let (hold, evidence) = &ended[0];
        // The meeting window is the WHOLE mic hold — start at minute 0 (true
        // hold start), not minute 2 (first evidence).
        assert_eq!(
            *hold,
            EndedHold {
                bundle_id: CHROME.to_string(),
                start_ms: 0,
                end_ms: 10 * MIN_MS
            }
        );
        let evidence = evidence.as_ref().expect("browser meeting carries evidence");
        assert_eq!(evidence.service, "Google Meet");
        assert_eq!(evidence.url, MEET_URL);
        // ≥ the default 5-minute floor: this hold WOULD fire.
        let floor = 5 * MIN_MS;
        assert!(hold.duration_ms() >= floor);
        assert!(!tracker.is_tracking());
    }

    #[test]
    fn no_evidence_hold_never_reaches_the_detector() {
        let mut detector = MeetingDetector::new(GRACE_MS);
        let mut tracker = BrowserMeetingTracker::new();
        // Web dictation: Chrome holds the mic 10 minutes, no meeting URL ever.
        for minute in [0, 2, 5, 9, 10] {
            assert!(tick(
                &mut detector,
                &mut tracker,
                &raw(&[CHROME]),
                None,
                minute * MIN_MS
            )
            .is_empty());
        }
        assert!(tracker.is_tracking(), "the hold is watched (probing cadence)");
        assert!(!detector.is_tracking(), "but the detector never sees it");
        // Release: nothing ends, nothing ever fires, tracking stops.
        assert!(tick(&mut detector, &mut tracker, &raw(&[]), None, 11 * MIN_MS).is_empty());
        assert!(!tracker.is_tracking());
        assert!(tick(&mut detector, &mut tracker, &raw(&[]), None, 20 * MIN_MS).is_empty());
    }

    #[test]
    fn evidence_survives_drop_and_rejoin_within_grace() {
        let mut detector = MeetingDetector::new(GRACE_MS);
        let mut tracker = BrowserMeetingTracker::new();
        tick(&mut detector, &mut tracker, &raw(&[CHROME]), None, 0);
        tick(
            &mut detector,
            &mut tracker,
            &raw(&[CHROME]),
            Some((CHROME, MEET_URL)),
            2 * MIN_MS,
        );
        // Call drops at minute 5 (network blip), rejoins at minute 6 — inside
        // the 2-minute grace. Evidence must survive the released window.
        tick(&mut detector, &mut tracker, &raw(&[]), None, 5 * MIN_MS);
        tick(&mut detector, &mut tracker, &raw(&[CHROME]), None, 6 * MIN_MS);
        // Final release at minute 12 → ONE meeting spanning the whole hold,
        // still evidence-backed.
        tick(&mut detector, &mut tracker, &raw(&[]), None, 12 * MIN_MS);
        let ended = tick(&mut detector, &mut tracker, &raw(&[]), None, 14 * MIN_MS);
        assert_eq!(ended.len(), 1);
        assert_eq!(
            ended[0].0,
            EndedHold {
                bundle_id: CHROME.to_string(),
                start_ms: 0,
                end_ms: 12 * MIN_MS
            }
        );
        assert_eq!(
            ended[0].1.as_ref().map(|evidence| evidence.service),
            Some("Google Meet")
        );
    }

    #[test]
    fn evidence_clears_after_ended_and_the_next_hold_needs_fresh_evidence() {
        let mut detector = MeetingDetector::new(GRACE_MS);
        let mut tracker = BrowserMeetingTracker::new();
        // Meeting 1: evidence-backed, ends.
        tick(
            &mut detector,
            &mut tracker,
            &raw(&[CHROME]),
            Some((CHROME, MEET_URL)),
            0,
        );
        tick(&mut detector, &mut tracker, &raw(&[]), None, 6 * MIN_MS);
        let ended = tick(&mut detector, &mut tracker, &raw(&[]), None, 8 * MIN_MS);
        assert_eq!(ended.len(), 1);
        assert!(ended[0].1.is_some());
        // Hold 2 (dictation this time): the old evidence must NOT carry over.
        for minute in [20, 25, 30] {
            assert!(tick(
                &mut detector,
                &mut tracker,
                &raw(&[CHROME]),
                None,
                minute * MIN_MS
            )
            .is_empty());
        }
        assert!(
            !detector.is_tracking(),
            "a fresh hold without its own sighting must stay invisible"
        );
        // Once hold 2 gets its own sighting, its window starts at ITS start.
        tick(
            &mut detector,
            &mut tracker,
            &raw(&[CHROME]),
            Some((CHROME, "https://zoom.us/j/999")),
            31 * MIN_MS,
        );
        tick(&mut detector, &mut tracker, &raw(&[]), None, 40 * MIN_MS);
        let ended = tick(&mut detector, &mut tracker, &raw(&[]), None, 42 * MIN_MS);
        assert_eq!(
            ended[0].0,
            EndedHold {
                bundle_id: CHROME.to_string(),
                start_ms: 20 * MIN_MS,
                end_ms: 40 * MIN_MS
            }
        );
        assert_eq!(ended[0].1.as_ref().map(|evidence| evidence.service), Some("Zoom"));
    }

    #[test]
    fn pre_evidence_release_resets_the_hold_start() {
        let mut detector = MeetingDetector::new(GRACE_MS);
        let mut tracker = BrowserMeetingTracker::new();
        // A short pre-meeting mic grab (mic test) at minute 0, released.
        tick(&mut detector, &mut tracker, &raw(&[CHROME]), None, 0);
        tick(&mut detector, &mut tracker, &raw(&[]), None, MIN_MS);
        // The real hold starts at minute 10; evidence at 12.
        tick(&mut detector, &mut tracker, &raw(&[CHROME]), None, 10 * MIN_MS);
        tick(
            &mut detector,
            &mut tracker,
            &raw(&[CHROME]),
            Some((CHROME, MEET_URL)),
            12 * MIN_MS,
        );
        tick(&mut detector, &mut tracker, &raw(&[]), None, 20 * MIN_MS);
        let ended = tick(&mut detector, &mut tracker, &raw(&[]), None, 22 * MIN_MS);
        // Window starts at the CURRENT hold's start, not the earlier grab's.
        assert_eq!(ended[0].0.start_ms, 10 * MIN_MS);
    }

    #[test]
    fn sighting_for_an_untracked_browser_or_non_meeting_url_records_nothing() {
        let mut tracker = BrowserMeetingTracker::new();
        // Not holding the mic → a (stale) sighting can't mark anything.
        assert_eq!(tracker.record_sighting(CHROME, MEET_URL), None);
        tracker.observe_raw_holders(&raw(&[CHROME]), 0);
        // Holding, but the front tab is not a meeting surface.
        assert_eq!(
            tracker.record_sighting(CHROME, "https://docs.google.com/document/d/1"),
            None
        );
        assert!(tracker.evidence_backed_holders(&raw(&[CHROME])).is_empty());
        // A real sighting marks it; a second sighting keeps the FIRST evidence.
        assert_eq!(tracker.record_sighting(CHROME, MEET_URL), Some("Google Meet"));
        assert_eq!(
            tracker.record_sighting(CHROME, "https://zoom.us/j/1"),
            None,
            "evidence is sticky — the first sighting wins"
        );
    }

    #[test]
    fn evidence_backed_hold_stops_being_probed() {
        let mut tracker = BrowserMeetingTracker::new();
        assert_eq!(
            tracker.observe_raw_holders(&raw(&[CHROME]), 0),
            vec![CHROME.to_string()],
            "a fresh browser hold is a probe target"
        );
        tracker.record_sighting(CHROME, MEET_URL);
        assert!(
            tracker.observe_raw_holders(&raw(&[CHROME]), MIN_MS).is_empty(),
            "sticky evidence ends probing for this hold"
        );
        // Non-browser holders are never probe targets.
        assert!(tracker.observe_raw_holders(&raw(&[ZOOM_APP]), 2 * MIN_MS).is_empty());
    }

    #[test]
    fn browser_meeting_and_conferencing_app_run_concurrently_and_independently() {
        let mut detector = MeetingDetector::new(GRACE_MS);
        let mut tracker = BrowserMeetingTracker::new();
        // Zoom app on a call from minute 0; Chrome joins a Meet at minute 2.
        tick(&mut detector, &mut tracker, &raw(&[ZOOM_APP]), None, 0);
        tick(
            &mut detector,
            &mut tracker,
            &raw(&[ZOOM_APP, CHROME]),
            Some((CHROME, MEET_URL)),
            2 * MIN_MS,
        );
        // Chrome hangs up at minute 10; Zoom keeps going.
        tick(&mut detector, &mut tracker, &raw(&[ZOOM_APP]), None, 10 * MIN_MS);
        let ended = tick(&mut detector, &mut tracker, &raw(&[ZOOM_APP]), None, 12 * MIN_MS);
        assert_eq!(ended.len(), 1);
        assert_eq!(
            ended[0].0,
            EndedHold {
                bundle_id: CHROME.to_string(),
                start_ms: 2 * MIN_MS,
                end_ms: 10 * MIN_MS
            }
        );
        assert_eq!(ended[0].1.as_ref().map(|evidence| evidence.service), Some("Google Meet"));
        // Zoom ends later with its own window and NO url evidence.
        tick(&mut detector, &mut tracker, &raw(&[]), None, 20 * MIN_MS);
        let ended = tick(&mut detector, &mut tracker, &raw(&[]), None, 22 * MIN_MS);
        assert_eq!(
            ended[0].0,
            EndedHold {
                bundle_id: ZOOM_APP.to_string(),
                start_ms: 0,
                end_ms: 20 * MIN_MS
            }
        );
        assert_eq!(ended[0].1, None);
    }

    #[test]
    fn sub_floor_evidence_backed_hold_is_ended_but_below_the_floor() {
        let mut detector = MeetingDetector::new(GRACE_MS);
        let mut tracker = BrowserMeetingTracker::new();
        // A 3-minute Meet huddle: detected, ended, but the worker's floor check
        // (trigger_floor_ms) rejects it — same as a short conferencing call.
        tick(
            &mut detector,
            &mut tracker,
            &raw(&[CHROME]),
            Some((CHROME, MEET_URL)),
            0,
        );
        tick(&mut detector, &mut tracker, &raw(&[]), None, 3 * MIN_MS);
        let ended = tick(&mut detector, &mut tracker, &raw(&[]), None, 5 * MIN_MS);
        assert_eq!(ended.len(), 1);
        let trigger = super::super::TriggerDefinition {
            id: "meeting-recap".to_string(),
            name: "Meeting Recap".to_string(),
            condition: super::super::TriggerCondition::MeetingEnds {
                min_meeting_minutes: None,
            },
            prompt: "Recap the meeting.".to_string(),
            enabled: true,
            cooldown_minutes: None,
            version: 1,
        };
        let floor = trigger_floor_ms(&trigger).expect("meeting trigger has a floor");
        assert!(ended[0].0.duration_ms() < floor);
    }

    #[test]
    fn sleep_gap_clears_pre_evidence_starts_but_keeps_evidence_backed_holds() {
        let mut tracker = BrowserMeetingTracker::new();
        tracker.observe_raw_holders(&raw(&[CHROME, "com.apple.Safari"]), 0);
        tracker.record_sighting(CHROME, MEET_URL);
        tracker.clear_pre_evidence_holds();
        // Safari's pre-evidence start is stale after a sleep; Chrome's
        // evidence-backed hold keeps its start (the detector owns its gap
        // semantics via the synthetic empty observation).
        assert_eq!(tracker.hold_start_ms("com.apple.Safari"), None);
        assert_eq!(tracker.hold_start_ms(CHROME), Some(0));
        // Safari re-holding after wake starts fresh at the post-wake tick.
        tracker.observe_raw_holders(&raw(&[CHROME, "com.apple.Safari"]), 60 * MIN_MS);
        assert_eq!(tracker.hold_start_ms("com.apple.Safari"), Some(60 * MIN_MS));
    }
}
