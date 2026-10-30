//! The privacy filter plan: which listed apps the ScreenCaptureKit filter excludes, what it
//! records as applied, and when the privacy poll can skip re-reading the shareable content.

#[cfg(target_os = "macos")]
use crate::privacy_visibility;
use crate::PrivacyContentFilter;
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrivacyContentFilterStrategy {
    ExcludingApps,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrivacyFilterAvailableApp {
    pub(crate) bundle_id: String,
    pub(crate) pid: i32,
    /// One of its windows shows on the captured display (`privacy_visibility`).
    pub(crate) shows_window: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrivacyContentFilterPlan {
    pub(crate) strategy: PrivacyContentFilterStrategy,
    pub(crate) excluded_bundle_ids: std::collections::BTreeSet<String>,
    /// Every running process of those apps, as `(pid, bundle id)`: what the filter names.
    pub(crate) excluded_processes: std::collections::BTreeSet<(i32, String)>,
    pub(crate) unresolved_bundle_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrivacyFilterAppliedKey {
    display_id: u32,
    strategy: PrivacyContentFilterStrategy,
    excluded_bundle_ids: Vec<String>,
    /// ScreenCaptureKit excludes running processes, so a second copy or a relaunch of an
    /// excluded app changes the key and forces a rebuild.
    excluded_processes: Vec<(i32, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrivacyFilterApplicationState {
    requested_key: capture_metadata::PrivacyFilterKey,
    applied_key: PrivacyFilterAppliedKey,
    unresolved_bundle_ids: Vec<String>,
}

impl PrivacyFilterApplicationState {
    /// `display_id` is the display the filter was built for, not the requested one: a request
    /// for display 0 resolves to the stream's display.
    pub(crate) fn from_plan(
        privacy_filter: &PrivacyContentFilter,
        display_id: u32,
        plan: &PrivacyContentFilterPlan,
    ) -> Self {
        Self {
            requested_key: privacy_filter.key(),
            applied_key: PrivacyFilterAppliedKey {
                display_id,
                strategy: plan.strategy,
                excluded_bundle_ids: plan.excluded_bundle_ids.iter().cloned().collect(),
                excluded_processes: plan.excluded_processes.iter().cloned().collect(),
            },
            unresolved_bundle_ids: plan.unresolved_bundle_ids.clone(),
        }
    }

    pub(crate) fn satisfies_request(
        &self,
        requested_key: &capture_metadata::PrivacyFilterKey,
    ) -> bool {
        self.requested_key == *requested_key
            && self.unresolved_bundle_ids.is_empty()
            && !self.requires_content_refresh()
    }

    pub(crate) fn applied_filter_matches(&self, next: &Self) -> bool {
        self.applied_key == next.applied_key
    }

    fn diagnostic_state_matches(&self, next: &Self) -> bool {
        self.requested_key == next.requested_key
            && self.applied_key == next.applied_key
            && self.unresolved_bundle_ids == next.unresolved_bundle_ids
    }

    /// An applied exclusion is dropped once its app stops showing a window, so a filter that
    /// excludes anything keeps the privacy poll re-checking the shareable content.
    fn requires_content_refresh(&self) -> bool {
        !self.applied_key.excluded_bundle_ids.is_empty()
    }
}

/// The applied state after a filter update. One that failed or timed out may still land later,
/// so it leaves nothing recorded as applied: the next refresh then rebuilds against the in-flight
/// union guard instead of matching the old filter and narrowing the guard to it.
#[cfg(target_os = "macos")]
pub(crate) fn applied_state_after_update(
    next: PrivacyFilterApplicationState,
    landed: bool,
) -> Option<PrivacyFilterApplicationState> {
    landed.then_some(next)
}

/// At most this long between shareable-content reads, however quiet the window list. It bounds
/// how long a window `CGWindowList` misses but ScreenCaptureKit lists can stay unexcluded.
#[cfg(target_os = "macos")]
pub(crate) const SHAREABLE_CONTENT_MAX_SKIP: Duration = Duration::from_secs(5);

/// The window list taken just before the last shareable-content read.
#[cfg(target_os = "macos")]
#[derive(Debug)]
pub(crate) struct ShareableContentRead {
    pub(crate) at: Instant,
    pub(crate) showing: privacy_visibility::ShowingSnapshot,
}

/// Whether the privacy poll can keep the live filter without re-reading the shareable content
/// (a ScreenCaptureKit round trip every second, all day, while a listed app runs hidden). Only
/// when the request is unchanged, the filter excludes nothing — an exclusion is dropped once
/// its app hides, which only a read sees — and the same processes show windows on the same
/// display as before the last read, which was recent. The frame guard keeps checking every
/// frame either way.
#[cfg(target_os = "macos")]
pub(crate) fn shareable_content_read_can_be_skipped(
    state: &PrivacyFilterApplicationState,
    requested_key: &capture_metadata::PrivacyFilterKey,
    last_read: Option<&ShareableContentRead>,
    showing_now: Option<&privacy_visibility::ShowingSnapshot>,
    now: Instant,
) -> bool {
    let (Some(last_read), Some(showing_now)) = (last_read, showing_now) else {
        return false;
    };
    state.requested_key == *requested_key
        && state.applied_key.excluded_bundle_ids.is_empty()
        && last_read.showing == *showing_now
        && now.saturating_duration_since(last_read.at) < SHAREABLE_CONTENT_MAX_SKIP
}

pub(crate) fn should_log_privacy_filter_diagnostics(
    previous: Option<&PrivacyFilterApplicationState>,
    next: &PrivacyFilterApplicationState,
) -> bool {
    previous.map_or(true, |previous| !previous.diagnostic_state_matches(next))
}

pub(crate) fn log_privacy_filter_diagnostics(state: &PrivacyFilterApplicationState) {
    if !state.unresolved_bundle_ids.is_empty() {
        capture_runtime::debug_log!(
            "privacy filter leaves out {} requested bundle id(s) (not running or not showing a window on the captured display)",
            state.unresolved_bundle_ids.len()
        );
    }
}

/// The display a privacy filter is built for. Display 0 means the stream's own display — never
/// "whichever display is listed first now": the list order changes as displays come and go,
/// and a filter (and frame guard) for another display would leave the recorded one unguarded.
/// `None` when that display is gone; the caller fails closed as display-unavailable.
pub(crate) fn resolve_privacy_filter_display(
    requested_display_id: u32,
    stream_display_id: u32,
    available_display_ids: &[u32],
) -> Option<u32> {
    let display_id = if requested_display_id == 0 {
        stream_display_id
    } else {
        requested_display_id
    };
    available_display_ids
        .contains(&display_id)
        .then_some(display_id)
}

pub(crate) fn plan_privacy_content_filter(
    requested_bundle_ids: &std::collections::BTreeSet<String>,
    available_apps: &[PrivacyFilterAvailableApp],
) -> PrivacyContentFilterPlan {
    // Only apps showing a window are excluded: excluding a running app costs WindowServer GPU
    // work even when it shows nothing (`privacy_visibility`). The rest stay unresolved, which
    // keeps the poll re-checking them and the stream handler guarding frames against them.
    let mut available_bundle_ids = std::collections::BTreeSet::new();
    for app in available_apps.iter().filter(|app| app.shows_window) {
        available_bundle_ids.insert(app.bundle_id.clone());
    }

    let excluded_bundle_ids = requested_bundle_ids
        .intersection(&available_bundle_ids)
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let unresolved_bundle_ids = requested_bundle_ids
        .difference(&available_bundle_ids)
        .cloned()
        .collect();
    let excluded_processes = available_apps
        .iter()
        .filter(|app| excluded_bundle_ids.contains(&app.bundle_id))
        .map(|app| (app.pid, app.bundle_id.clone()))
        .collect();

    PrivacyContentFilterPlan {
        strategy: PrivacyContentFilterStrategy::ExcludingApps,
        excluded_bundle_ids,
        excluded_processes,
        unresolved_bundle_ids,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privacy_plan_is_app_only() {
        let requested_bundle_ids = std::collections::BTreeSet::from(["com.secret".to_string()]);
        let available_apps = vec![
            PrivacyFilterAvailableApp {
                bundle_id: "com.secret".to_string(),
                pid: 1,
                shows_window: true,
            },
            PrivacyFilterAvailableApp {
                bundle_id: "com.google.Chrome".to_string(),
                pid: 2,
                shows_window: true,
            },
        ];

        let plan = plan_privacy_content_filter(&requested_bundle_ids, &available_apps);

        assert_eq!(plan.strategy, PrivacyContentFilterStrategy::ExcludingApps);
        assert_eq!(
            plan.excluded_bundle_ids,
            std::collections::BTreeSet::from(["com.secret".to_string()])
        );
    }

    #[test]
    fn unresolved_privacy_filter_targets_do_not_satisfy_cached_request() {
        let requested = PrivacyContentFilter {
            display_id: 0,
            excluded_bundle_ids: vec!["com.secret".to_string()],
        };
        let requested_bundle_ids = std::collections::BTreeSet::from(["com.secret".to_string()]);

        let unresolved_plan = plan_privacy_content_filter(&requested_bundle_ids, &[]);
        let unresolved_state =
            PrivacyFilterApplicationState::from_plan(&requested, 1, &unresolved_plan);

        assert_eq!(
            unresolved_state.unresolved_bundle_ids,
            vec!["com.secret".to_string()]
        );
        assert!(!unresolved_state.satisfies_request(&requested.key()));

        let resolved_plan = plan_privacy_content_filter(
            &requested_bundle_ids,
            &[PrivacyFilterAvailableApp {
                bundle_id: "com.secret".to_string(),
                pid: 1,
                shows_window: true,
            }],
        );
        let resolved_state =
            PrivacyFilterApplicationState::from_plan(&requested, 1, &resolved_plan);

        // An applied exclusion keeps the poll re-checking: it is dropped once the app hides.
        assert!(!resolved_state.satisfies_request(&requested.key()));
        assert!(!unresolved_state.applied_filter_matches(&resolved_state));
    }

    #[test]
    fn running_excluded_app_without_a_showing_window_is_left_out_of_the_filter() {
        let requested = PrivacyContentFilter {
            display_id: 0,
            excluded_bundle_ids: vec!["com.secret".to_string()],
        };
        let requested_bundle_ids = std::collections::BTreeSet::from(["com.secret".to_string()]);
        let hidden_plan = plan_privacy_content_filter(
            &requested_bundle_ids,
            &[PrivacyFilterAvailableApp {
                bundle_id: "com.secret".to_string(),
                pid: 1,
                shows_window: false,
            }],
        );

        assert!(hidden_plan.excluded_bundle_ids.is_empty());
        assert_eq!(
            hidden_plan.unresolved_bundle_ids,
            vec!["com.secret".to_string()]
        );
        let hidden_state = PrivacyFilterApplicationState::from_plan(&requested, 1, &hidden_plan);
        assert!(!hidden_state.satisfies_request(&requested.key()));
    }

    fn secret_state(pids: &[i32]) -> PrivacyFilterApplicationState {
        let requested = PrivacyContentFilter {
            display_id: 0,
            excluded_bundle_ids: vec!["com.secret".to_string()],
        };
        let available_apps = pids
            .iter()
            .map(|&pid| PrivacyFilterAvailableApp {
                bundle_id: "com.secret".to_string(),
                pid,
                shows_window: true,
            })
            .collect::<Vec<_>>();
        let plan = plan_privacy_content_filter(
            &std::collections::BTreeSet::from(["com.secret".to_string()]),
            &available_apps,
        );
        PrivacyFilterApplicationState::from_plan(&requested, 1, &plan)
    }

    #[test]
    fn a_second_or_relaunched_copy_of_an_excluded_app_forces_a_rebuild() {
        let applied = secret_state(&[100]);
        assert_eq!(
            applied.applied_key.excluded_processes,
            vec![(100, "com.secret".to_string())]
        );
        // `open -n`: the same bundle id now runs twice.
        assert!(!applied.applied_filter_matches(&secret_state(&[100, 200])));
        // Quit and relaunched within one poll: same bundle id, new pid.
        assert!(!applied.applied_filter_matches(&secret_state(&[200])));
        assert!(applied.applied_filter_matches(&secret_state(&[100])));
    }

    #[test]
    fn a_failed_filter_update_leaves_the_next_refresh_rebuilding() {
        let live = secret_state(&[100]);
        let attempted = secret_state(&[100, 200]);
        let recorded = applied_state_after_update(attempted.clone(), false);
        // The next poll builds the old filter again: it must not match a recorded state and
        // take the refresh path, which would narrow the in-flight union guard.
        let rebuilt = secret_state(&[100]);
        assert!(!recorded
            .as_ref()
            .is_some_and(|state| state.applied_filter_matches(&rebuilt)));
        assert_eq!(
            applied_state_after_update(attempted.clone(), true),
            Some(attempted)
        );
        assert!(live.applied_filter_matches(&rebuilt));
    }

    #[test]
    fn the_poll_skips_the_shareable_content_read_only_while_nothing_changed() {
        use privacy_visibility::ShowingSnapshot;
        let requested = PrivacyContentFilter {
            display_id: 0,
            excluded_bundle_ids: vec!["com.secret".to_string()],
        };
        let key = requested.key();
        let hidden = PrivacyFilterApplicationState::from_plan(
            &requested,
            1,
            &plan_privacy_content_filter(
                &std::collections::BTreeSet::from(["com.secret".to_string()]),
                &[],
            ),
        );
        let quiet = ShowingSnapshot::for_test(&[7, 8]);
        let now = Instant::now();
        let read = ShareableContentRead {
            at: now,
            showing: quiet.clone(),
        };
        let skip = |state: &PrivacyFilterApplicationState,
                    key: &capture_metadata::PrivacyFilterKey,
                    read: Option<&ShareableContentRead>,
                    showing: Option<&ShowingSnapshot>,
                    at: Instant| {
            shareable_content_read_can_be_skipped(state, key, read, showing, at)
        };
        assert!(skip(&hidden, &key, Some(&read), Some(&quiet), now));
        // A process starts (or stops) showing a window: read.
        let busier = ShowingSnapshot::for_test(&[7, 8, 9]);
        assert!(!skip(&hidden, &key, Some(&read), Some(&busier), now));
        // The window list can't be read, or was never taken: read.
        assert!(!skip(&hidden, &key, Some(&read), None, now));
        assert!(!skip(&hidden, &key, None, Some(&quiet), now));
        // The request changed: read.
        let other = PrivacyContentFilter {
            excluded_bundle_ids: vec!["com.other".to_string()],
            ..requested.clone()
        };
        assert!(!skip(&hidden, &other.key(), Some(&read), Some(&quiet), now));
        // The filter excludes an app: only a read sees it hide.
        assert!(!skip(
            &secret_state(&[100]),
            &key,
            Some(&read),
            Some(&quiet),
            now
        ));
        // The last read is too old.
        assert!(!skip(
            &hidden,
            &key,
            Some(&read),
            Some(&quiet),
            now + SHAREABLE_CONTENT_MAX_SKIP
        ));
    }

    #[test]
    fn privacy_filter_for_display_zero_stays_on_the_stream_display() {
        // The stream records display 7; display 3 is now listed first.
        assert_eq!(resolve_privacy_filter_display(0, 7, &[3, 7]), Some(7));
        assert_eq!(resolve_privacy_filter_display(3, 7, &[3, 7]), Some(3));
        // The stream's display is gone: fail closed instead of guarding another one.
        assert_eq!(resolve_privacy_filter_display(0, 7, &[3]), None);
        assert_eq!(resolve_privacy_filter_display(0, 7, &[]), None);
    }

    #[test]
    fn applied_key_records_the_resolved_display() {
        let requested = PrivacyContentFilter {
            display_id: 0,
            excluded_bundle_ids: Vec::new(),
        };
        let plan = plan_privacy_content_filter(&Default::default(), &[]);
        let on_seven = PrivacyFilterApplicationState::from_plan(&requested, 7, &plan);
        let on_three = PrivacyFilterApplicationState::from_plan(&requested, 3, &plan);
        assert_eq!(on_seven.applied_key.display_id, 7);
        assert!(!on_seven.applied_filter_matches(&on_three));
    }

    #[test]
    fn an_empty_exclusion_list_satisfies_the_request() {
        let requested = PrivacyContentFilter {
            display_id: 0,
            excluded_bundle_ids: Vec::new(),
        };
        let plan = plan_privacy_content_filter(&Default::default(), &[]);
        let state = PrivacyFilterApplicationState::from_plan(&requested, 1, &plan);
        assert!(state.satisfies_request(&requested.key()));
    }

    #[test]
    fn repeated_unresolved_privacy_filter_plan_suppresses_duplicate_diagnostics() {
        let requested = PrivacyContentFilter {
            display_id: 0,
            excluded_bundle_ids: vec!["com.secret".to_string()],
        };
        let requested_bundle_ids = std::collections::BTreeSet::from(["com.secret".to_string()]);
        let plan = plan_privacy_content_filter(&requested_bundle_ids, &[]);
        let previous = PrivacyFilterApplicationState::from_plan(&requested, 1, &plan);
        let next = PrivacyFilterApplicationState::from_plan(&requested, 1, &plan);

        assert!(should_log_privacy_filter_diagnostics(None, &next));
        assert!(!should_log_privacy_filter_diagnostics(
            Some(&previous),
            &next
        ));
    }

    #[test]
    fn privacy_filter_diagnostics_log_when_unresolved_targets_change() {
        let previous_request = PrivacyContentFilter {
            display_id: 0,
            excluded_bundle_ids: vec!["com.secret".to_string()],
        };
        let next_request = PrivacyContentFilter {
            display_id: 0,
            excluded_bundle_ids: vec!["com.other-secret".to_string()],
        };
        let previous_plan = plan_privacy_content_filter(
            &std::collections::BTreeSet::from(["com.secret".to_string()]),
            &[],
        );
        let next_plan = plan_privacy_content_filter(
            &std::collections::BTreeSet::from(["com.other-secret".to_string()]),
            &[],
        );
        let previous =
            PrivacyFilterApplicationState::from_plan(&previous_request, 1, &previous_plan);
        let next = PrivacyFilterApplicationState::from_plan(&next_request, 1, &next_plan);

        assert!(should_log_privacy_filter_diagnostics(
            Some(&previous),
            &next
        ));
    }
}
