//! The one-time Sensitive Capture Protection prompt
//! (`AppPrivacyExclusionPrompt.svelte` above every Settings group, and the
//! notice atop `Privacy.svelte`; logic from `app-privacy-exclusion{,.svelte}.ts`).
//! It shows while a catalog app (password managers, Apple Passwords) is not an
//! enabled exclusion and the prompt was never dismissed or completed. Render
//! marks it shown (once); Add exclusions saves them through Settings' privacy
//! command, then completes it; Dismiss closes it for good; Review only jumps to
//! Capture › Privacy. Pending state comes from the live exclusion list, so it
//! follows every privacy edit without a re-read.

use app_core::one_time_prompts::OneTimePromptState;
use app_core::privacy_redaction_sources as rules;
use app_core::sensitive_capture_recommendations::SensitiveCaptureRecommendationsResponse;
use capture_types::ExcludedAppEntry;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, Context, SharedString};

use super::groups::SectionId;
use super::view::PAGE_MAX_W;
use super::{click, icons, Settings};
use crate::theme::{self, radius, space, text, Palette};
use crate::ui::{notice, Button, Tone};

/// One catalog app (`recommendedApps[]`): the name the app list knows it by.
#[derive(Clone, Debug, PartialEq)]
pub struct Recommended {
    pub bundle_id: String,
    pub display_name: String,
}

/// `get_sensitive_capture_recommendations` + this prompt's one-time record.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PromptData {
    pub prompt_id: String,
    pub apps: Vec<Recommended>,
    /// Dismissed or completed: it never shows again.
    pub closed: bool,
}

impl PromptData {
    pub fn from_core(r: &SensitiveCaptureRecommendationsResponse, s: &OneTimePromptState) -> Self {
        let closed = s
            .prompts
            .get(&r.prompt_id)
            .is_some_and(|p| p.dismissed_at.is_some() || p.completed_at.is_some());
        Self {
            prompt_id: r.prompt_id.clone(),
            apps: r
                .recommended_apps
                .iter()
                .map(|a| Recommended {
                    bundle_id: a.bundle_id.clone(),
                    display_name: a.display_name.clone(),
                })
                .collect(),
            closed,
        }
    }
}

/// The record writes (`*_one_time_prompt`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Shown,
    Dismissed,
    Completed,
}

/// The prompt's session state (the Svelte controller's latches).
#[derive(Default)]
pub struct PromptUi {
    /// `promptMarkedId`: shown is recorded once per prompt.
    pub marked: Option<String>,
    /// Closed here this session (a slower reload's older answer can't reopen it).
    pub closed: bool,
    /// `promptActionInFlight`.
    pub in_flight: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExclusionState {
    Missing,
    Disabled,
    Enabled,
}

fn same_bundle_id(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// A name-only rule (typed before the app was installed). It excludes nothing
/// until its bundle id resolves.
pub fn is_pending_exclusion(e: &ExcludedAppEntry) -> bool {
    e.bundle_id.trim().is_empty() && !e.display_name.trim().is_empty()
}

/// The recommendation's `exclusionState`, from the live exclusion list.
pub fn exclusion_state(excluded: &[ExcludedAppEntry], bundle_id: &str) -> ExclusionState {
    match excluded
        .iter()
        .find(|e| same_bundle_id(&e.bundle_id, bundle_id))
    {
        Some(e) if e.enabled => ExclusionState::Enabled,
        Some(_) => ExclusionState::Disabled,
        None => ExclusionState::Missing,
    }
}

/// What excluding one app does (`recommendationActionFor`).
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    /// `add_privacy_excluded_app`.
    Add {
        bundle_id: String,
        display_name: String,
    },
    /// `set_privacy_excluded_app_enabled(source id, true)`.
    Reenable(String),
}

pub fn action_for(
    bundle_id: &str,
    display_name: &str,
    state: ExclusionState,
    excluded: &[ExcludedAppEntry],
) -> Action {
    // Every blank bundle id equals every other one, so a not-installed app's
    // identity is its name; else a second one re-enables the first pending row.
    let existing = excluded.iter().find(|e| {
        if bundle_id.trim().is_empty() {
            is_pending_exclusion(e) && same_name(&e.display_name, display_name)
        } else {
            same_bundle_id(&e.bundle_id, bundle_id)
        }
    });
    if state == ExclusionState::Enabled || existing.is_some_and(|e| e.enabled) {
        return Action::None;
    }
    match existing {
        Some(e) => Action::Reenable(e.id.clone()),
        None => Action::Add {
            bundle_id: bundle_id.to_string(),
            display_name: display_name.to_string(),
        },
    }
}

/// What the prompt shows.
#[derive(Clone, Debug, PartialEq)]
pub struct PromptView {
    pub prompt_id: String,
    /// `pendingRecommendedApps`: not yet an enabled exclusion.
    pub pending: Vec<Recommended>,
}

impl PromptView {
    pub fn title(&self) -> String {
        let n = self.pending.len();
        let s = if n == 1 { "" } else { "s" };
        format!("{n} recommended app exclusion{s} pending")
    }
}

pub const BODY: &str =
    "Password managers and Apple Passwords can be excluded from future screen capture.";

/// `shouldShowSensitiveRecommendationPrompt`: open, and something left to exclude.
pub fn view(
    data: Option<&PromptData>,
    closed_here: bool,
    excluded: &[ExcludedAppEntry],
) -> Option<PromptView> {
    let data = data.filter(|d| !d.closed && !closed_here && !d.prompt_id.trim().is_empty())?;
    let pending: Vec<Recommended> = data
        .apps
        .iter()
        .filter(|a| exclusion_state(excluded, &a.bundle_id) != ExclusionState::Enabled)
        .cloned()
        .collect();
    (!pending.is_empty()).then(|| PromptView {
        prompt_id: data.prompt_id.clone(),
        pending,
    })
}

impl Settings {
    /// The prompt for this page, if it shows (nothing before the settings load).
    pub fn privacy_prompt(&self) -> Option<PromptView> {
        let d = self.draft.as_ref()?;
        view(
            self.data.privacy_prompt.as_ref(),
            self.ui.prompt.closed,
            &d.privacy.excluded_apps,
        )
    }

    /// Record `shown` the first time this prompt renders (`mark_one_time_prompt_shown`).
    pub(crate) fn mark_prompt_shown(&mut self, v: &PromptView, cx: &mut Context<Self>) {
        if self.ui.prompt.marked.as_deref() == Some(v.prompt_id.as_str()) {
            return;
        }
        self.ui.prompt.marked = Some(v.prompt_id.clone());
        let backend = self.backend.clone();
        let id = v.prompt_id.clone();
        cx.background_spawn(async move {
            if let Err(e) = backend.prompt(&id, Mark::Shown) {
                log::warn!("settings: couldn't record prompt {id} as shown: {e}");
            }
        })
        .detach();
    }

    /// `dismissSensitiveRecommendationPrompt`: closed for good.
    pub fn dismiss_privacy_prompt(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.privacy_prompt() else {
            return;
        };
        if self.ui.prompt.in_flight {
            return;
        }
        self.close_prompt(v.prompt_id, Mark::Dismissed, cx);
    }

    /// `applyAllRecommendedPrivacyApps`: every pending app becomes an enabled
    /// exclusion in one privacy save, then the prompt completes.
    pub fn apply_privacy_prompt(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.privacy_prompt() else {
            return;
        };
        if self.ui.prompt.in_flight {
            return;
        }
        let pending = v.pending.clone();
        let started = self.privacy_command_then(
            move |d| {
                for a in &pending {
                    let state = exclusion_state(&d.privacy.excluded_apps, &a.bundle_id);
                    match action_for(
                        &a.bundle_id,
                        &a.display_name,
                        state,
                        &d.privacy.excluded_apps,
                    ) {
                        Action::None => {}
                        Action::Add {
                            bundle_id,
                            display_name,
                        } => {
                            rules::upsert_privacy_excluded_app(d, &bundle_id, &display_name, false)?
                        }
                        Action::Reenable(id) => {
                            rules::set_privacy_excluded_app_enabled(d, &id, true)?
                        }
                    }
                }
                Ok(())
            },
            move |s, saved, cx| {
                if saved {
                    log::info!(
                        "settings: added {} recommended app exclusions",
                        v.pending.len()
                    );
                    s.close_prompt(v.prompt_id, Mark::Completed, cx);
                } else {
                    s.ui.prompt.in_flight = false;
                }
            },
            cx,
        );
        self.ui.prompt.in_flight = started;
        cx.notify();
    }

    fn close_prompt(&mut self, id: String, mark: Mark, cx: &mut Context<Self>) {
        self.ui.prompt.in_flight = true;
        let backend = self.backend.clone();
        let job = cx.background_spawn(async move { backend.prompt(&id, mark) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |s, cx| {
                s.ui.prompt.in_flight = false;
                match result {
                    Ok(()) => s.ui.prompt.closed = true,
                    Err(e) => {
                        log::warn!("settings: couldn't record the privacy prompt as {mark:?}: {e}");
                        s.ui.note = Some((false, "Sensitive capture protection", e));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

fn add_button(s: &Settings, id: &'static str, cx: &mut Context<Settings>) -> AnyElement {
    Button::new(id, "Add exclusions")
        .sm()
        .disabled(s.privacy_in_flight || s.ui.prompt.in_flight)
        .on_click(click(cx, |s, cx| s.apply_privacy_prompt(cx)))
        .into_any_element()
}

fn dismiss_button(s: &Settings, id: &'static str, cx: &mut Context<Settings>) -> AnyElement {
    Button::new(id, "Dismiss")
        .ghost()
        .sm()
        .disabled(s.ui.prompt.in_flight)
        .on_click(click(cx, |s, cx| s.dismiss_privacy_prompt(cx)))
        .into_any_element()
}

/// The page-wide prompt above the active group (`.sensitive-prompt`).
pub fn banner(s: &Settings, v: &PromptView, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    div()
        .id("sensitive-prompt")
        .test_support()
        .role(gpui_kit::Role::Region)
        .aria_label("Sensitive capture recommendation")
        .max_w(px(PAGE_MAX_W))
        .mb(space::S5)
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap(px(14.))
        .p(space::S3)
        .rounded(radius::MD)
        .border_1()
        .border_color(p.warn_border)
        .bg(p.warn_bg)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(260.))
                .gap(px(3.))
                .child(
                    div()
                        .text_size(text::XS)
                        .font_weight(theme::W_SEMIBOLD)
                        .text_color(p.warn)
                        .child("SENSITIVE CAPTURE PROTECTION"),
                )
                .child(
                    div()
                        .text_size(text::BASE)
                        .font_weight(theme::W_SEMIBOLD)
                        .line_height(gpui_kit::relative(1.25))
                        .text_color(p.text_strong)
                        .child(SharedString::from(v.title())),
                )
                .child(
                    div()
                        .text_size(text::SM)
                        .line_height(gpui_kit::relative(1.4))
                        .text_color(p.text_muted)
                        .child(BODY),
                ),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_end()
                .gap(px(6.))
                .child(add_button(s, "sensitive-prompt-add", cx))
                .child(
                    Button::new("sensitive-prompt-review", "Review")
                        .ghost()
                        .sm()
                        .disabled(s.ui.prompt.in_flight)
                        .on_click(click(cx, |s, cx| s.navigate(SectionId::Privacy, cx))),
                )
                .child(dismiss_button(s, "sensitive-prompt-dismiss", cx)),
        )
        .into_any_element()
}

/// The same prompt atop Capture › Privacy (`.priv-top`).
pub fn section_notice(
    s: &Settings,
    v: &PromptView,
    p: &Palette,
    cx: &mut Context<Settings>,
) -> AnyElement {
    div()
        .py(space::S3)
        .child(notice::notice(
            Tone::Info,
            icons::SHIELD,
            v.title(),
            Some(BODY.into()),
            vec![
                add_button(s, "priv-prompt-add", cx),
                dismiss_button(s, "priv-prompt-dismiss", cx),
            ],
            p,
        ))
        .into_any_element()
}

#[cfg(test)]
#[path = "privacy_prompt_tests.rs"]
mod tests;
