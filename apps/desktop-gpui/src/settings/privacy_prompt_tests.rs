//! The view model (cases from `app-privacy-exclusion.test.ts`, plus the prompt's
//! show rules) and the prompt's behaviour through `FixtureBackend`.

use std::sync::Arc;

use capture_types::ExcludedAppEntry;
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Entity, TestAppContext, WindowHandle};

use super::*;
use crate::settings::fixture::{Call, FixtureBackend};
use crate::settings::groups::GroupId;
use crate::settings::tests::{frame, open_at};

fn entry(id: &str, enabled: bool, bundle_id: &str, name: &str) -> ExcludedAppEntry {
    ExcludedAppEntry {
        id: id.into(),
        enabled,
        bundle_id: bundle_id.into(),
        display_name: name.into(),
    }
}

fn pending_figma() -> ExcludedAppEntry {
    entry("rule-pending", true, "", "Figma")
}

const ID: &str = "sensitive_capture_protection_v1_recommended_exclusions";

/// The catalog as app-core lists it.
fn catalog() -> PromptData {
    let app = |b: &str, n: &str| Recommended {
        bundle_id: b.into(),
        display_name: n.into(),
    };
    PromptData {
        prompt_id: ID.into(),
        apps: vec![
            app("com.apple.Passwords", "Apple Passwords"),
            app("com.apple.keychainaccess", "Keychain Access"),
            app("com.1password.1password", "1Password"),
            app("com.bitwarden.desktop", "Bitwarden"),
        ],
        closed: false,
    }
}

// ── isPendingExclusion ──────────────────────────────────────────────────────

#[test]
fn a_name_only_rule_is_pending_until_a_bundle_id_lands() {
    let figma = pending_figma();
    assert!(is_pending_exclusion(&figma));
    assert!(is_pending_exclusion(&ExcludedAppEntry {
        bundle_id: "  ".into(),
        ..figma.clone()
    }));
    assert!(!is_pending_exclusion(&ExcludedAppEntry {
        bundle_id: "com.figma.Desktop".into(),
        ..figma
    }));
    assert!(!is_pending_exclusion(&entry(
        "rule-1",
        true,
        "com.example.installed",
        "Installed"
    )));
}

#[test]
fn a_rule_with_neither_a_bundle_id_nor_a_name_is_not_pending() {
    assert!(!is_pending_exclusion(&ExcludedAppEntry {
        display_name: "  ".into(),
        ..pending_figma()
    }));
}

// ── recommendationActionFor with a not-yet-installed app ────────────────────

#[test]
fn a_second_not_installed_app_is_added_not_the_first_re_enabled() {
    assert_eq!(
        action_for("", "Bitwarden", ExclusionState::Missing, &[pending_figma()]),
        Action::Add {
            bundle_id: String::new(),
            display_name: "Bitwarden".into()
        }
    );
}

#[test]
fn the_struck_pending_rule_with_the_same_name_is_re_enabled() {
    let struck = ExcludedAppEntry {
        enabled: false,
        ..pending_figma()
    };
    assert_eq!(
        action_for("", "figma", ExclusionState::Missing, &[struck]),
        Action::Reenable("rule-pending".into())
    );
}

#[test]
fn nothing_happens_when_that_pending_rule_is_already_live() {
    assert_eq!(
        action_for("", "Figma", ExclusionState::Missing, &[pending_figma()]),
        Action::None
    );
}

#[test]
fn an_installed_app_is_matched_by_bundle_id_ignoring_case() {
    let struck = [entry("app-9", false, "COM.BITWARDEN.DESKTOP", "Bitwarden")];
    let state = exclusion_state(&struck, "com.bitwarden.desktop");
    assert_eq!(state, ExclusionState::Disabled);
    assert_eq!(
        action_for("com.bitwarden.desktop", "Bitwarden", state, &struck),
        Action::Reenable("app-9".into())
    );
    assert_eq!(
        action_for(
            "com.bitwarden.desktop",
            "Bitwarden",
            ExclusionState::Enabled,
            &[]
        ),
        Action::None
    );
}

// ── shouldShowSensitiveRecommendationPrompt ─────────────────────────────────

#[test]
fn it_shows_the_apps_not_yet_enabled_exclusions() {
    let excluded = [
        entry("a", true, "com.apple.passwords", "Passwords"),
        entry("b", false, "com.bitwarden.desktop", "Bitwarden"),
    ];
    let v = view(Some(&catalog()), false, &excluded).expect("shows");
    let names: Vec<&str> = v.pending.iter().map(|a| a.display_name.as_str()).collect();
    assert_eq!(names, ["Keychain Access", "1Password", "Bitwarden"]);
    assert_eq!(v.title(), "3 recommended app exclusions pending");
}

#[test]
fn one_pending_app_reads_singular() {
    let excluded: Vec<_> = catalog().apps[..3]
        .iter()
        .map(|a| entry("x", true, &a.bundle_id, &a.display_name))
        .collect();
    let v = view(Some(&catalog()), false, &excluded).expect("shows");
    assert_eq!(v.title(), "1 recommended app exclusion pending");
}

#[test]
fn it_hides_with_nothing_recommended_or_everything_excluded() {
    assert_eq!(view(None, false, &[]), None, "not loaded");
    let empty = PromptData {
        apps: Vec::new(),
        ..catalog()
    };
    assert_eq!(view(Some(&empty), false, &[]), None, "nothing recommended");
    let all: Vec<_> = catalog()
        .apps
        .iter()
        .map(|a| entry("x", true, &a.bundle_id, &a.display_name))
        .collect();
    assert_eq!(view(Some(&catalog()), false, &all), None, "all excluded");
}

#[test]
fn it_hides_once_dismissed_or_completed() {
    let closed = PromptData {
        closed: true,
        ..catalog()
    };
    assert_eq!(view(Some(&closed), false, &[]), None, "the record says so");
    assert_eq!(
        view(Some(&catalog()), true, &[]),
        None,
        "closed this session"
    );
}

#[test]
fn the_record_closes_it_on_dismissed_or_completed_never_on_shown() {
    use app_core::one_time_prompts::{OneTimePromptRecord, OneTimePromptState};
    let recs =
        app_core::sensitive_capture_recommendations::SensitiveCaptureRecommendationsResponse {
            prompt_id: ID.into(),
            recommended_apps: Vec::new(),
            actionable_recommendation_count: 0,
            should_show_existing_user_prompt: false,
            browser_disclosures: Vec::new(),
        };
    let with = |r: OneTimePromptRecord| {
        let mut s = OneTimePromptState::default();
        s.prompts.insert(ID.into(), r);
        PromptData::from_core(&recs, &s).closed
    };
    let at = Some("2026-11-10T00:00:00Z".to_string());
    assert!(!PromptData::from_core(&recs, &OneTimePromptState::default()).closed);
    assert!(!with(OneTimePromptRecord {
        shown_at: at.clone(),
        ..Default::default()
    }));
    assert!(with(OneTimePromptRecord {
        dismissed_at: at.clone(),
        ..Default::default()
    }));
    assert!(with(OneTimePromptRecord {
        completed_at: at,
        ..Default::default()
    }));
}

// ── behaviour through the view ──────────────────────────────────────────────
// The parity fixture already excludes 1Password and Passwords (both enabled).

fn open(
    cx: &mut TestAppContext,
    fail: bool,
    data: Option<PromptData>,
) -> (WindowHandle<Root>, Entity<Settings>, Arc<FixtureBackend>) {
    let (window, v, b) = open_at(cx, fail, (1280., 800.));
    v.update(cx, |s, cx| {
        s.data.privacy_prompt = data;
        cx.notify();
    });
    cx.run_until_parked();
    frame(cx, window);
    (window, v, b)
}

fn prompt_calls(b: &FixtureBackend) -> Vec<Mark> {
    b.calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::Prompt(m) => Some(m),
            _ => None,
        })
        .collect()
}

fn click(cx: &mut TestAppContext, window: WindowHandle<Root>, id: &'static str) {
    cx.update_window(window.into(), |_, w, cx| w.click(id, cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, window);
}

fn shown(cx: &mut TestAppContext, window: WindowHandle<Root>) -> bool {
    cx.update_window(window.into(), |_, w, _| {
        w.try_find("sensitive-prompt").is_some()
    })
    .unwrap()
}

#[gpui_kit::test]
fn it_shows_on_every_group_and_records_shown_once(cx: &mut TestAppContext) {
    let (window, v, b) = open(cx, false, Some(catalog()));
    assert!(shown(cx, window));
    v.read_with(cx, |s, _| {
        assert_eq!(
            s.privacy_prompt().map(|p| p.title()),
            Some("2 recommended app exclusions pending".into())
        );
    });
    for tab in ["license", "ocr", "privacy"] {
        v.update(cx, |s, cx| s.open_tab(tab, cx));
        cx.run_until_parked();
        frame(cx, window);
        assert!(shown(cx, window), "above the {tab} group too");
    }
    assert_eq!(
        prompt_calls(&b),
        [Mark::Shown],
        "recorded once, not per frame"
    );
}

#[gpui_kit::test]
fn nothing_shows_or_is_recorded_without_a_recommendation(cx: &mut TestAppContext) {
    let (window, _, b) = open(cx, false, None);
    assert!(!shown(cx, window));
    let mut all = catalog();
    all.apps.truncate(0);
    let (window, _, b2) = open(cx, false, Some(all));
    assert!(!shown(cx, window));
    assert!(prompt_calls(&b).is_empty() && prompt_calls(&b2).is_empty());
}

#[gpui_kit::test]
fn review_only_jumps_to_privacy_and_dismiss_closes_it_for_good(cx: &mut TestAppContext) {
    let (window, v, b) = open(cx, false, Some(catalog()));
    click(cx, window, "sensitive-prompt-review");
    v.read_with(cx, |s, _| assert_eq!(s.group, GroupId::Capture));
    assert!(shown(cx, window), "Review is not a dismissal");
    assert_eq!(prompt_calls(&b), [Mark::Shown]);

    click(cx, window, "priv-prompt-dismiss");
    assert!(!shown(cx, window));
    assert_eq!(prompt_calls(&b), [Mark::Shown, Mark::Dismissed]);
    assert!(
        !b.calls().contains(&Call::Privacy),
        "dismissing excludes nothing"
    );
    // A reload that read the record before the dismissal landed can't reopen it.
    v.update(cx, |s, cx| {
        s.data.privacy_prompt = Some(catalog());
        cx.notify();
    });
    frame(cx, window);
    assert!(!shown(cx, window));
}

#[gpui_kit::test]
fn add_exclusions_saves_every_pending_app_then_completes(cx: &mut TestAppContext) {
    let (window, v, b) = open(cx, false, Some(catalog()));
    // Bitwarden is a struck rule: it is re-enabled, not duplicated.
    v.update(cx, |s, cx| {
        for d in [s.saved.as_mut(), s.draft.as_mut()].into_iter().flatten() {
            d.privacy.excluded_apps.push(entry(
                "app-bw",
                false,
                "com.bitwarden.desktop",
                "Bitwarden",
            ));
        }
        cx.notify();
    });
    frame(cx, window);
    click(cx, window, "sensitive-prompt-add");
    assert_eq!(
        b.calls(),
        [
            Call::Prompt(Mark::Shown),
            Call::Privacy,
            Call::Prompt(Mark::Completed)
        ],
        "one privacy save, then the record"
    );
    v.read_with(cx, |s, _| {
        let apps = &s.d().privacy.excluded_apps;
        for a in catalog().apps {
            assert_eq!(
                exclusion_state(apps, &a.bundle_id),
                ExclusionState::Enabled,
                "{} excluded",
                a.bundle_id
            );
        }
        assert_eq!(apps.len(), 4, "Bitwarden kept its row");
        assert!(apps.iter().any(|a| a.id == "app-bw" && a.enabled));
        assert!(!s.ui.prompt.in_flight);
    });
    assert!(!shown(cx, window));
}

#[gpui_kit::test]
fn a_failed_save_leaves_the_prompt_open_and_uncompleted(cx: &mut TestAppContext) {
    let (window, v, b) = open(cx, true, Some(catalog()));
    click(cx, window, "sensitive-prompt-add");
    assert_eq!(
        prompt_calls(&b),
        [Mark::Shown],
        "never completed without the save"
    );
    assert!(shown(cx, window));
    v.read_with(cx, |s, _| {
        assert!(!s.ui.prompt.in_flight);
        assert!(s.privacy_error.is_some());
    });
}

#[gpui_kit::test]
fn escape_leaves_the_prompt_alone(cx: &mut TestAppContext) {
    // Svelte's prompt is inline, not a dialog: Esc and leaving the page neither
    // dismiss nor complete it.
    let (window, _, b) = open(cx, false, Some(catalog()));
    cx.update_window(window.into(), |_, w, cx| w.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, window);
    assert!(shown(cx, window));
    assert_eq!(prompt_calls(&b), [Mark::Shown]);
}
