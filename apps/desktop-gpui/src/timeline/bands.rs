//! What rides on the rail: app-episode bands (category colour, 18 px app icon,
//! a major tick at each boundary), the Recall match mark, and the audio lane
//! below (mic / sys bars, 11 px). Geometry: slot `i`'s centre sits at
//! `centre + advance − i·8`, so the active slot is under the fixed caret.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::{
    div, fill, img, linear_color_stop, linear_gradient, point, px, size, AnyElement, Bounds,
    Context, Hsla, Pixels, Role, SharedString, Window,
};

use super::model::{
    app_runs, audio_slots, category_color, category_for_span, AppRun, AudioSource, SLOT_W,
};
use super::TimelineView;
use crate::theme::{self, radius, text, Palette, FONT_MONO};
use crate::ui::{alpha, mix, nid, spread, with_tip};

/// `timelineAppRuns` is O(frames); it's recomputed only when the frame list
/// changes (first id, last id, length), not every scrub frame.
#[derive(Default)]
pub struct RunsCache(RefCell<Option<((i64, i64, usize, i64), Rc<Vec<AppRun>>)>>);

impl RunsCache {
    pub fn get(&self, v: &TimelineView) -> Rc<Vec<AppRun>> {
        let f = &v.paging.frames;
        let key = (
            f.first().map_or(0, |x| x.id),
            f.last().map_or(0, |x| x.id),
            f.len(),
            v.max_gap_ms,
        );
        let mut cell = self.0.borrow_mut();
        if let Some((k, runs)) = cell.as_ref() {
            if *k == key {
                return runs.clone();
            }
        }
        let runs = Rc::new(app_runs(f, v.max_gap_ms));
        *cell = Some((key, runs.clone()));
        runs
    }
}

/// A run's band colour: the episode it overlaps most.
pub fn run_color(v: &TimelineView, r: &AppRun, p: &Palette) -> Hsla {
    let f = &v.paging.frames;
    category_color(
        category_for_span(
            f[r.end].captured_at_ms,
            f[r.start].captured_at_ms,
            &v.episodes,
        ),
        p,
    )
}

/// Screen x of slot `i`'s centre.
pub fn slot_x(centre: f32, advance: f32, i: usize) -> f32 {
    centre + advance - i as f32 * SLOT_W
}

/// Paint the bands (gradient fill, 1 px left edge, 2 px top) inside `b`.
pub fn paint_bands(
    b: Bounds<Pixels>,
    centre: f32,
    advance: f32,
    runs: &[(usize, usize, Hsla)],
    window: &mut Window,
) {
    for &(start, end, c) in runs {
        let right = slot_x(centre, advance, start) + SLOT_W / 2.;
        let left = slot_x(centre, advance, end) - SLOT_W / 2.;
        let x0 = left.max(f32::from(b.left()));
        let x1 = right.min(f32::from(b.right()));
        if x1 <= x0 {
            continue;
        }
        let band = Bounds::new(point(px(x0), b.top()), size(px(x1 - x0), b.size.height));
        window.paint_quad(fill(
            band,
            linear_gradient(
                180.,
                linear_color_stop(alpha(c, 0.16), 0.),
                linear_color_stop(alpha(c, 0.04), 1.),
            ),
        ));
        window.paint_quad(fill(
            Bounds::new(band.origin, size(band.size.width, px(2.))),
            alpha(c, 0.75),
        ));
        if left >= f32::from(b.left()) {
            window.paint_quad(fill(
                Bounds::new(point(px(left), b.top()), size(px(1.), b.size.height)),
                alpha(c, 0.45),
            ));
        }
    }
}

/// `.tl-group__icon`: 18 px tile (app icon or first letter), 5 px in from the
/// band's oldest visible edge, only when more than 30 px of the band shows.
pub fn icons(
    v: &TimelineView,
    runs: &[AppRun],
    window_range: std::ops::Range<usize>,
    centre: f32,
    p: &Palette,
) -> Vec<AnyElement> {
    let mut out = Vec::new();
    for r in runs {
        if r.end < window_range.start || r.start >= window_range.end {
            continue;
        }
        let vis_start = r.start.max(window_range.start);
        let vis_end = r.end.min(window_range.end - 1);
        if ((vis_end - vis_start + 1) as f32 * SLOT_W) <= 30. {
            continue;
        }
        let c = run_color(v, r, p);
        let x = slot_x(centre, v.advance_px, vis_end) - SLOT_W / 2. + 5.;
        let icon = r.bundle_id.as_ref().and_then(|b| v.icons.get(b)).cloned();
        out.push(
            div()
                .absolute()
                .top(px(7.))
                .left(px(x))
                .size(px(18.))
                .flex()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .rounded(px(5.))
                .bg(mix(c, p.surface_raised, 0.32))
                .shadow(vec![spread(alpha(c, 0.5), px(1.))])
                .text_size(text::XS)
                .font_weight(gpui_kit::FontWeight::BOLD)
                .text_color(p.text_strong)
                .child(match icon {
                    Some(path) => img(path).size(px(14.)).into_any_element(),
                    None => SharedString::from(r.fallback.clone()).into_any_element(),
                })
                .into_any_element(),
        );
    }
    out
}

/// The audio lane under the rail. Its track mirrors the rail's, centred on the
/// lane viewport (as the Svelte lane's own `50cqi` margins are).
pub fn audio_lane(
    v: &TimelineView,
    rail_w: f32,
    p: &Palette,
    cx: &mut Context<TimelineView>,
) -> AnyElement {
    const GUTTER: f32 = 28. + 6.;
    let lane_w = (rail_w - GUTTER).max(0.);
    let centre = lane_w / 2.;
    let label = |t: &'static str, c: Hsla| {
        div()
            .flex()
            .flex_1()
            .items_center()
            .justify_end()
            .font_family(FONT_MONO)
            .text_size(px(9.))
            .font_weight(theme::W_MEDIUM)
            .text_color(c)
            .child(t)
    };
    let frames = &v.paging.frames;
    let bars: Vec<AnyElement> = v
        .audio
        .iter()
        .filter_map(|s| audio_slots(frames, s).map(|slots| (s, slots)))
        .map(|(s, (newest, oldest))| {
            let right = slot_x(centre, v.advance_px, newest) + SLOT_W / 2.;
            let width = ((oldest - newest + 1) as f32 * SLOT_W).max(2.);
            let mic = s.source == AudioSource::Microphone;
            let selected = v.selected_audio.map(|a| a.0) == Some(s.id);
            let id = s.id;
            let tip = format!("{} #{}", s.source.label(), s.segment_index);
            let bar = div()
                .id(nid("audio-bar", id as u64))
                .role(Role::Button)
                .aria_label(SharedString::from(format!("Play {}", tip.to_lowercase())))
                .absolute()
                .top(px(if mic { 1. } else { 14. }))
                .left(px(right - width))
                .w(px(width))
                .h(px(11.))
                .rounded(px(3.))
                .bg(if mic { p.source_mic } else { p.source_sysaudio })
                .when(selected, |d| d.shadow(vec![spread(p.accent, px(1.5))]))
                .cursor_pointer()
                .on_hover(cx.listener(move |this, on: &bool, _, _| {
                    if *on {
                        this.bar_hover = Some(id);
                    } else if this.bar_hover == Some(id) {
                        this.bar_hover = None;
                    }
                }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected_audio = Some((id, 0));
                    cx.notify();
                }));
            with_tip(bar, tip).into_any_element()
        })
        .collect();
    let note = if !bars.is_empty() {
        None
    } else if let Some(e) = &v.audio_error {
        Some((format!("audio unavailable · {e}"), p.danger_text))
    } else if frames.is_empty() {
        None
    } else if v.audio_loading {
        Some(("loading audio…".to_string(), p.text_subtle))
    } else {
        Some(("no audio in range".to_string(), p.text_subtle))
    };
    div()
        .flex()
        .flex_none()
        .gap(px(6.))
        .w_full()
        .py(px(4.))
        .child(
            div()
                .flex()
                .flex_col()
                .w(px(28.))
                .pr(px(4.))
                .child(label("MIC", p.source_mic))
                .child(label("SYS", p.source_sysaudio)),
        )
        .child(
            div()
                .relative()
                .flex_1()
                .min_w_0()
                .h(px(26.))
                .overflow_hidden()
                .children(bars)
                .children(note.map(|(t, c)| {
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(text::SM)
                        .text_color(c)
                        .child(t)
                })),
        )
        .into_any_element()
}

/// `.tl-matchmark`: the one dot at the Recall-landed frame.
pub fn match_mark(v: &TimelineView, centre: f32, p: &Palette) -> Option<AnyElement> {
    let l = v.landing.as_ref()?;
    let i = v.paging.frames.iter().position(|f| f.id == l.frame_id)?;
    Some(
        div()
            .absolute()
            .top(px(6.))
            .left(px(slot_x(centre, v.advance_px, i) - 2.))
            .size(px(4.))
            .rounded(radius::PILL)
            .bg(p.accent)
            .shadow(vec![spread(p.accent_glow, px(2.))])
            .into_any_element(),
    )
}
