//! The one external link: `.answer-prose a` (accent, underlined) or, with
//! `.quiet()`, About's `.notice-item__source` (mono, muted, accent on hover
//! and keyboard focus, the up-right arrow). A Tab stop that opens on click,
//! Enter or Space, through GPUI's `open_url` (where `Host::open_url` ends up
//! too), and only for `http(s)://` and `mailto:` URLs ([`openable`]).

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, App, ClickEvent, ElementId, MouseButton, SharedString, Window};

use super::{focus_handle, focus_visible, icon, outline, Force};
use crate::theme::{self, radius, size, space, text, FONT_MONO};

/// `isOpenableHref` (`lib/markdown.ts`): an absolute `http`/`https` URL with a
/// host, or a `mailto:` with an address. Everything else (`file:`, `javascript:`,
/// a bare path, a custom scheme) is refused.
pub fn openable(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once(':') else {
        return false;
    };
    if scheme.eq_ignore_ascii_case("mailto") {
        return !rest.trim().is_empty();
    }
    (scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http"))
        && rest
            .strip_prefix("//")
            .is_some_and(|host| !host.is_empty() && !host.starts_with('/'))
}

/// Opens `url` when it is [`openable`]; otherwise logs the scheme only (never
/// the URL) and does nothing. Every link, button or prose anchor that hands a
/// URL to the OS goes through here.
pub fn open(url: &str, cx: &mut App) -> bool {
    if openable(url) {
        cx.open_url(url);
        return true;
    }
    let scheme = url
        .split_once(':')
        .map(|(s, _)| s)
        .filter(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        })
        .unwrap_or("(none)");
    log::debug!("link: refused to open a URL with scheme {scheme}");
    false
}

/// TextView's link handler (Chat and Quick Recall answers, release notes):
/// the default's buttons (left, middle, keyboard), but through [`open`], so a
/// Markdown link in model output never hands `file:` or a custom scheme to
/// the OS (`AnswerProse`'s `isOpenableHref` gate).
pub fn prose_click(url: &SharedString, e: &ClickEvent, _: &mut Window, cx: &mut App) {
    let go = match e {
        ClickEvent::Mouse(c) => matches!(c.up.button, MouseButton::Left | MouseButton::Middle),
        ClickEvent::Keyboard(_) => true,
        ClickEvent::Touch(t) => !t.long_press,
    };
    if go {
        open(url, cx);
    }
}

#[derive(IntoElement)]
pub struct Link {
    id: ElementId,
    label: SharedString,
    url: SharedString,
    quiet: bool,
    force: Force,
}

impl Link {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        url: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            url: url.into(),
            quiet: false,
            force: Force::None,
        }
    }

    /// About's source link: mono 10, muted, accent on hover / focus, ↗.
    pub fn quiet(mut self) -> Self {
        self.quiet = true;
        self
    }

    /// Pins hover / focus for goldens.
    pub fn force(mut self, force: Force) -> Self {
        self.force = force;
        self
    }
}

impl RenderOnce for Link {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = theme::tokens(cx);
        let handle = focus_handle(self.id.clone(), window, cx);
        let ring = focus_visible(&handle, self.force, window);
        let hot = ring || self.force == Force::Hover;
        let rest = if self.quiet { p.text_muted } else { p.accent };
        let url = self.url.clone();
        div()
            .id(self.id)
            .test_support()
            .role(gpui_kit::Role::Link)
            .track_focus(&handle)
            .relative()
            .flex()
            .items_center()
            .gap(space::S1)
            .min_w_0()
            .cursor_pointer()
            .text_color(if hot { p.accent } else { rest })
            .hover(move |s| s.text_color(p.accent))
            .when(self.quiet, |d| d.font_family(FONT_MONO).text_size(text::XS))
            // CSS draws the prose link's focus as the browser ring; the quiet
            // one has `outline: none` and only turns accent.
            .when(!self.quiet, |d| d.underline())
            .when(!self.quiet && ring, |d| {
                d.child(outline(p.accent, radius::XS))
            })
            .child(div().min_w_0().child(self.label))
            .when(self.quiet, |d| {
                d.child(
                    div()
                        .flex_none()
                        .opacity(0.7)
                        .child(icon(IconName::ArrowUpRight.path(), size::LINK_ICON)),
                )
            })
            .on_click(move |_, _, cx| {
                open(&url, cx);
            })
    }
}

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;
