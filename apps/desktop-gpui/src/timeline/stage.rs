//! `.timeline__stage`: radius 12, the frame painted `contain`, the stale tint
//! (0.55 opacity, 0.6 saturation: `+page.svelte:7016-7018`), the empty and
//! error states, the stage note, the frame actions, the overlays and the dev
//! metadata panel.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{
    canvas, div, img, px, AnyElement, Bounds, Context, ObjectFit, Pixels, Role, SharedString,
    Window,
};

use super::model::{clock_label, day_label, local};
use super::{frame_actions, match_overlay, ocr_overlay, TimelineView};
use crate::theme::{self, radius, size, space, text, Palette, FONT_MONO};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::notice::{inline, notice};
use crate::ui::{icon, skeleton, Button, Tone};

/// `.timeline__stage--stale`: CSS `opacity: .55; filter: saturate(.6)`. GPUI has
/// no saturate filter, but CSS saturate(s) is a lerp to the Rec.709 luma GPUI's
/// grayscale sprite uses, so a grey layer under a colour layer composes it
/// exactly: colour α = .55·.6, grey α = .55·.4 / (1 − .55·.6).
pub const STALE_OPACITY: f32 = 0.55;
pub const STALE_SATURATION: f32 = 0.6;
const STALE_COLOR_ALPHA: f32 = STALE_OPACITY * STALE_SATURATION;
const STALE_GREY_ALPHA: f32 = STALE_OPACITY * (1. - STALE_SATURATION) / (1. - STALE_COLOR_ALPHA);

/// The painted image rect inside the stage (`renderedImageRect`, contain-fit).
pub fn image_rect(
    stage: gpui_kit::Size<Pixels>,
    w: Option<u32>,
    h: Option<u32>,
) -> Option<Bounds<Pixels>> {
    let (sw, sh) = (f32::from(stage.width), f32::from(stage.height));
    let (iw, ih) = (w? as f32, h? as f32);
    if sw <= 0. || sh <= 0. || iw <= 0. || ih <= 0. {
        return None;
    }
    let s = (sw / iw).min(sh / ih);
    let (w, h) = (iw * s, ih * s);
    Some(Bounds::new(
        gpui_kit::point(px((sw - w) / 2.), px((sh - h) / 2.)),
        gpui_kit::size(px(w), px(h)),
    ))
}

fn retry(v: &TimelineView, cx: &mut Context<TimelineView>) -> AnyElement {
    let busy = v.paging.loading;
    Button::new("timeline-retry", if busy { "Retrying…" } else { "Retry" })
        .sm()
        .icon(IconName::RotateCcw.path())
        .busy(busy)
        .disabled(busy || v.paging.loading_more)
        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx)))
        .into_any_element()
}

fn period(s: &str) -> String {
    format!("{}.", s.trim_end_matches('.'))
}

fn empty_state(v: &TimelineView, p: &Palette, cx: &mut Context<TimelineView>) -> AnyElement {
    if let Some(e) = &v.paging.error {
        return empty(
            EmptyLayout::Center,
            Tone::Danger,
            IconName::TriangleAlert.path(),
            "Couldn’t load your timeline",
            format!("{} Your recordings are safe.", period(e)),
            vec![retry(v, cx)],
            p,
        )
        .into_any_element();
    }
    let rec = crate::assets::TIMELINE_GLYPH;
    if (v.capture.capturing && !v.capture.screen) || !v.audio.is_empty() {
        let lead = if v.capture.capturing {
            "Microphone and system audio are."
        } else {
            "Only audio was recorded."
        };
        return empty(
            EmptyLayout::Center,
            Tone::Off,
            IconName::Monitor.path(),
            "Screen isn’t being recorded",
            format!("{lead} Open a recent segment to listen or read it."),
            Vec::new(),
            p,
        )
        .into_any_element();
    }
    if v.capture.capturing {
        return empty(
            EmptyLayout::Center,
            Tone::Neutral,
            rec,
            "Recording started",
            "Your first frames appear here in a moment.",
            vec![inline(
                Tone::Neutral,
                Some(crate::ui::progress::spinner(size::SPIN_SM).into_any_element()),
                vec!["Waiting for the first frame…".into_any_element()],
                p,
            )
            .into_any_element()],
            p,
        )
        .into_any_element();
    }
    empty(
        EmptyLayout::Center,
        Tone::Neutral,
        rec,
        "Nothing recorded yet",
        "Your timeline fills in as Mnema captures your screen, microphone and system audio — all on this Mac.",
        vec![Button::new("timeline-start", "Start recording")
            .primary()
            .on_click(cx.listener(|_, _, _, cx| cx.emit(super::TimelineEvent::StartRecording)))
            .into_any_element()],
        p,
    )
    .into_any_element()
}

fn status_note(
    v: &TimelineView,
    p: &Palette,
    cx: &mut Context<TimelineView>,
) -> Option<AnyElement> {
    let s = v.status.as_ref()?;
    let fg = if s.error {
        crate::ui::mix(p.danger, p.text, 0.72)
    } else {
        p.text
    };
    Some(
        div()
            .id("stage-status")
            .role(if s.error { Role::Alert } else { Role::Status })
            .absolute()
            .right(px(10.))
            .bottom(px(10.))
            .max_w(px(360.))
            .flex()
            .items_start()
            .gap(px(8.))
            .px(px(9.))
            .py(px(7.))
            .rounded(radius::MD)
            .border_1()
            .border_color(if s.error {
                crate::ui::mix(p.danger, p.overlay_border, 0.4)
            } else {
                p.overlay_border
            })
            .bg(p.overlay_bg_strong)
            .text_color(fg)
            .text_size(if s.error { text::SM } else { text::XS })
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hold_status();
                } else if let Some(s) = this.status.clone() {
                    this.set_status(Some(s.message), s.detail, s.error, cx);
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(theme::W_SEMIBOLD)
                            .child(s.message.clone()),
                    )
                    .children(s.detail.clone().map(|d| {
                        div()
                            .pt(px(3.))
                            .border_t_1()
                            .border_color(crate::ui::alpha(fg, 0.16))
                            .font_family(FONT_MONO)
                            .text_size(if s.error { text::XS } else { px(9.) })
                            .text_color(p.text_subtle)
                            .child(d)
                    })),
            )
            .when(s.error, |d| {
                d.child(
                    Button::icon_only("stage-status-close", IconName::X.path(), "Dismiss message")
                        .ghost()
                        .sm()
                        .tip("Dismiss")
                        .on_click(
                            cx.listener(|this, _, _, cx| this.set_status(None, None, false, cx)),
                        ),
                )
            })
            .into_any_element(),
    )
}

fn dev_overlay(
    v: &TimelineView,
    p: &Palette,
    cx: &mut Context<TimelineView>,
) -> Option<AnyElement> {
    let f = v.paging.active_frame().filter(|_| v.developer)?;
    let o = crate::ui::clock::now(cx).offset();
    let t = local(f.captured_at_ms, o);
    let d = v.engine.display();
    let s = v.engine.stats;
    let mut rows: Vec<(&str, String)> = vec![
        ("id", f.id.to_string()),
        (
            "captured",
            format!("{} {}", day_label(t), clock_label(t, true)),
        ),
        ("session", f.session_id.clone()),
    ];
    if let (Some(w), Some(h)) = (f.width, f.height) {
        rows.push(("dims", format!("{w}×{h}")));
    }
    // Developer options only: the dev panel shows what the Svelte one does.
    rows.extend([
        (
            "app",
            format!(
                "{}{}",
                f.app_name.as_deref().unwrap_or("—"),
                f.app_bundle_id
                    .as_ref()
                    .map(|b| format!(" ({b})"))
                    .unwrap_or_default()
            ),
        ),
        (
            "window",
            f.window_title.clone().unwrap_or_else(|| "—".into()),
        ),
        ("url", f.url.clone().unwrap_or_else(|| "—".into())),
        ("file", f.file_path.display().to_string()),
        (
            "image",
            d.map_or("none", |d| if d.exact { "exact" } else { "scrub" })
                .into(),
        ),
        ("source", d.map_or("none", |d| d.source_kind.name()).into()),
        (
            "preview",
            format!(
                "hit {} miss {} fail {}",
                s.exact_hits, s.exact_misses, s.exact_failures
            ),
        ),
        (
            "scrub",
            format!(
                "batch {} ready {} warm {}",
                s.scrub_batches, s.scrub_ready, s.warm
            ),
        ),
        ("velocity", format!("{:.2}px/ms", v.engine.velocity)),
    ]);
    Some(
        div()
            .absolute()
            .top(px(10.))
            .left(px(10.))
            .max_w(px(360.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(10.))
            .py(px(8.))
            .rounded(radius::XS)
            .border_1()
            .border_color(p.overlay_border)
            .bg(p.overlay_bg)
            .children(rows.into_iter().map(|(k, val)| {
                div()
                    .flex()
                    .gap(px(10.))
                    .child(
                        div()
                            .w(px(64.))
                            .text_size(text::XS)
                            .font_weight(theme::W_SEMIBOLD)
                            .text_color(p.text_subtle)
                            .child(k.to_uppercase()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(FONT_MONO)
                            .text_size(text::XS)
                            .text_color(p.text)
                            .child(SharedString::from(val)),
                    )
            }))
            .into_any_element(),
    )
}

fn ocr_status(v: &TimelineView, p: &Palette) -> Option<AnyElement> {
    let here = v.ocr.visible && v.ocr.frame_id == v.paging.active_frame().map(|f| f.id);
    if !here {
        return None;
    }
    ocr_overlay::status_note(v.ocr.status, v.ocr.error.as_deref(), p)
}

pub fn render(
    v: &TimelineView,
    p: &Palette,
    window: &mut Window,
    cx: &mut Context<TimelineView>,
) -> impl IntoElement {
    let has_frames = !v.paging.frames.is_empty();
    let stale = has_frames && v.paging.error.is_some();
    let size_cell = v.stage_size.clone();
    let measure = canvas(
        move |b, window, _| {
            if size_cell.get() != b.size {
                size_cell.set(b.size);
                window.refresh();
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full();

    let body: AnyElement = if v.paging.loading && !has_frames {
        skeleton::thumb(p)
            .w(gpui_kit::relative(0.9))
            .h(gpui_kit::relative(0.9))
            .into_any_element()
    } else if !has_frames {
        empty_state(v, p, cx)
    } else if let Some(shown) = v.painted.clone() {
        let frame = v.paging.active_frame().cloned();
        let layer = |grey: bool| {
            img(shown.path.clone())
                .image_cache(&v.images)
                .object_fit(ObjectFit::Contain)
                .grayscale(grey)
                .absolute()
                .inset_0()
                .size_full()
        };
        let exact_here = frame
            .as_ref()
            .is_some_and(|f| shown.exact && shown.frame_id == f.id);
        let rect = frame
            .as_ref()
            .and_then(|f| image_rect(v.stage_size.get(), f.width, f.height));
        div()
            .absolute()
            .inset_0()
            .child(if stale {
                div()
                    .absolute()
                    .inset_0()
                    .child(layer(true).opacity(STALE_GREY_ALPHA))
                    .child(layer(false).opacity(STALE_COLOR_ALPHA))
                    .into_any_element()
            } else {
                layer(false).into_any_element()
            })
            .children(
                exact_here
                    .then(|| ocr_overlay::render(v, rect, p, cx))
                    .flatten(),
            )
            .children(
                exact_here
                    .then(|| match_overlay::render(v, rect, p))
                    .flatten(),
            )
            .child(frame_actions::render(v, p, window, cx))
            .into_any_element()
    } else if v.status.as_ref().is_some_and(|s| s.error) {
        inline(
            Tone::Danger,
            Some(icon(IconName::TriangleAlert.path(), size::ICON_SM).into_any_element()),
            vec!["Preview unavailable".into_any_element()],
            p,
        )
        .into_any_element()
    } else {
        skeleton::thumb(p)
            .w(gpui_kit::relative(0.9))
            .h(gpui_kit::relative(0.9))
            .into_any_element()
    };

    let stale_notice = stale.then(|| {
        let e = v.paging.error.clone().unwrap_or_default();
        div()
            .absolute()
            .top(space::S3)
            .left(space::S3)
            .right(px(120.))
            .flex()
            .child(
                notice(
                    Tone::Warn,
                    IconName::TriangleAlert.path(),
                    "Couldn’t refresh the timeline",
                    Some(format!("Showing what was already loaded. {}", period(&e)).into()),
                    vec![retry(v, cx)],
                    p,
                )
                .max_w(px(620.))
                // `color-mix(tone-bg 100%, surface-raised)` = the tone fill.
                .bg(p.warn_bg),
            )
    });

    div()
        .id("timeline-stage")
        .relative()
        .flex()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .rounded(radius::LG)
        .border_1()
        .border_color(p.border)
        // shortcut: the CSS radial highlight (raised 70% at 50% 42%) has no GPUI
        // radial gradient; solid surface-subtle until one exists.
        .bg(p.surface_subtle)
        .overflow_hidden()
        .child(measure)
        .child(body)
        .children(ocr_status(v, p))
        .children(dev_overlay(v, p, cx))
        .children(stale_notice)
        .children(status_note(v, p, cx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_layers_compose_to_the_css_filter() {
        // colour coeff .33, grey coeff .22, background .45, as opacity .55 · saturate .6.
        let c = STALE_COLOR_ALPHA;
        let g = (1. - c) * STALE_GREY_ALPHA;
        let bg = (1. - c) * (1. - STALE_GREY_ALPHA);
        assert!((c - 0.33).abs() < 1e-6 && (g - 0.22).abs() < 1e-6 && (bg - 0.45).abs() < 1e-6);
    }

    #[test]
    fn contain_fit_centres_the_frame() {
        let r = image_rect(gpui_kit::size(px(1000.), px(500.)), Some(1600), Some(1000)).unwrap();
        assert_eq!(
            (f32::from(r.size.width), f32::from(r.size.height)),
            (800., 500.)
        );
        assert_eq!(f32::from(r.origin.x), 100.);
        assert!(image_rect(gpui_kit::size(px(1000.), px(500.)), None, Some(1)).is_none());
    }
}
