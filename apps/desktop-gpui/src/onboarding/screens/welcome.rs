//! 1 / 8 Welcome (`WelcomeScreen.svelte`): one button, one sentence, and the
//! one-shot filmstrip rewind. Thirty captured-screen tiles slide back under a
//! fixed playhead and settle on one, the timecode counting back with them,
//! and that frame gives up its words. Reduced motion lands settled.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::AnimationExt as _;
use gpui_kit::{
    div, linear_color_stop, linear_gradient, px, relative, Animation, AnyElement, Context, Hsla,
    Window,
};

use super::primary;
use crate::onboarding::{parts, Onboarding};
use crate::theme::{self, leading, motion, radius, space, text, Palette, FONT_MONO, FONT_UI};
use crate::ui::baseline::{self, Line};
use crate::ui::Tracking as _;
use crate::ui::{kicker, mix};

const TILES: [[u8; 5]; 8] = [
    [71, 60, 53, 73, 90],
    [55, 37, 66, 74, 90],
    [46, 90, 38, 51, 46],
    [90, 60, 86, 39, 78],
    [55, 76, 59, 81, 44],
    [66, 55, 43, 53, 36],
    [48, 65, 77, 48, 85],
    [41, 34, 37, 90, 79],
];
const FRAMES: usize = 30;
const HIT: usize = 19;
/// 132 px tile + 12 px gap.
const PITCH: f32 = 144.;
const TILE_W: f32 = 132.;
/// Frames rewound.
const TRAVEL: f32 = 9.;
/// The rewind: 2.6 s on the expo curve, once.
pub const REWIND: Duration = Duration::from_millis(2600);
const FILM_H: f32 = 104.;

fn tile(k: usize, settled: bool, p: &Palette) -> AnyElement {
    let hit = k == HIT && settled;
    let widths = TILES[k % TILES.len()];
    div()
        .flex_none()
        .w(px(TILE_W))
        .h(px(88.))
        .flex()
        .flex_col()
        .overflow_hidden()
        .rounded(radius::SM)
        .border_1()
        .border_color(if hit { p.accent_border } else { p.border })
        .bg(if hit {
            mix(p.accent, p.surface, 0.05)
        } else {
            p.surface
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(3.))
                .h(px(10.))
                .px(px(6.))
                .border_b_1()
                .border_color(p.hairline)
                .children((0..3).map(|_| div().size(px(3.)).rounded_full().bg(p.chart_grey_2))),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .px(px(10.))
                .py(px(9.))
                .children(widths.iter().enumerate().map(|(i, w)| {
                    div()
                        .h(px(4.))
                        .w(relative(*w as f32 / 100.))
                        .rounded(px(2.))
                        .bg(if hit && i == 1 {
                            p.accent
                        } else {
                            p.chart_grey_1
                        })
                })),
        )
        .into_any_element()
}

/// An edge fade over the film (`mask-image`: transparent → black over 14 %).
fn fade(left: bool, bg: Hsla) -> impl IntoElement {
    let clear = Hsla { a: 0., ..bg };
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .w(relative(0.14))
        .when(left, |d| d.left_0())
        .when(!left, |d| d.right_0())
        .bg(linear_gradient(
            if left { 90. } else { 270. },
            linear_color_stop(bg, 0.),
            linear_color_stop(clear, 1.),
        ))
}

impl Onboarding {
    pub(in crate::onboarding) fn welcome(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = theme::tokens(cx);
        let film_w = f32::from(window.viewport_size().width);
        let end = film_w / 2. - (HIT as f32 * PITCH + 66.);
        let start = end - TRAVEL * PITCH;
        let interval = (self.interval_s() * 1000.) as i64;
        let anchor = self.welcome_anchor;
        // Frame k back from "now" at offset x: the timecode counts back with the strip.
        let tc = move |x: f32| {
            let back = 8 - ((end - x) / PITCH).round() as i64;
            crate::timeline::model::clock_label(
                anchor - time::Duration::milliseconds(back * interval),
                true,
            )
        };
        // One-shot: the rewind plays from the first render; after it (or with
        // reduced motion) the strip is settled and the hit frame lit.
        let now = cx.background_executor().now();
        if self.welcome_started.is_none() {
            self.welcome_started = Some(now);
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(REWIND).await;
                let _ = this.update(cx, |_, cx| cx.notify());
            })
            .detach();
        }
        let settled = cx.reduce_motion()
            || self
                .welcome_started
                .is_some_and(|t| now.saturating_duration_since(t) >= REWIND);
        let tiles = |settled: bool| {
            div()
                .absolute()
                .top(px(6.))
                .flex()
                .gap(px(PITCH - TILE_W))
                .children((0..FRAMES).map(|k| tile(k, settled, &p)))
        };
        let strip: AnyElement = if settled {
            tiles(true).left(px(end)).into_any_element()
        } else {
            tiles(false)
                .with_animation(
                    "ob-rewind",
                    Animation::new(REWIND).with_easing(motion::expo),
                    move |el, t| el.left(px(start + (end - start) * t)),
                )
                .into_any_element()
        };
        let read = |settled: f32| {
            div()
                .opacity(settled)
                .flex()
                .text_size(text::BASE)
                .text_color(p.text_subtle)
                .child("read off that frame \u{2014} ")
                .child(
                    div()
                        .font_weight(theme::W_MEDIUM)
                        .text_color(p.text_strong)
                        .child("\u{201c}Quarterly review \u{2014} draft agenda\u{201d}"),
                )
        };
        let strong = p.text_strong;
        let timecode = move |x: f32| {
            div()
                .font_family(FONT_MONO)
                .font_weight(theme::W_MEDIUM)
                .text_size(text::MD)
                .text_color(strong)
                .child(tc(x))
        };
        let (tc_el, read_el): (AnyElement, AnyElement) = if settled {
            (
                timecode(end).into_any_element(),
                read(1.).into_any_element(),
            )
        } else {
            (
                div()
                    .with_animation(
                        "ob-tc",
                        Animation::new(REWIND).with_easing(motion::expo),
                        move |el, t| el.child(timecode(start + (end - start) * t)),
                    )
                    .into_any_element(),
                read(0.).into_any_element(),
            )
        };
        let film = div()
            .relative()
            .h(px(FILM_H))
            .overflow_hidden()
            .child(strip)
            .child(fade(true, p.bg))
            .child(fade(false, p.bg))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(film_w / 2. - 0.75))
                    .w(px(1.5))
                    .bg(p.accent),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left(px(film_w / 2. - 4.75))
                    .w(px(9.5))
                    .h(px(6.))
                    .rounded_b(px(5.))
                    .bg(p.accent),
            );
        // `.film__tag` "◂◂ rewinding" (CSS uppercases it); the glyph is Lucide's rewind.
        let tag = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .font_weight(theme::W_MEDIUM)
            .tracking(0.12)
            .text_color(p.text_subtle)
            .child(crate::ui::icon("icons/rewind.svg", text::XS))
            .child("REWINDING");
        let rail = baseline::row(vec![
            (
                Line::new(FONT_MONO, text::XS, leading::BODY),
                tag.into_any_element(),
            ),
            (Line::new(FONT_MONO, text::MD, leading::BODY), tc_el),
            (Line::new(FONT_UI, text::BASE, leading::BODY), read_el),
        ])
        .gap(space::S4)
        .px(space::S8 + self.inset)
        .pt(space::S3);
        let title = div()
            .flex()
            .flex_col()
            .font_weight(theme::W_SEMIBOLD)
            .text_size(parts::HERO)
            .tracking(-0.045)
            .line_height(relative(0.98))
            .text_color(p.text_strong)
            .child("Your memory,")
            .child(div().text_color(p.text_subtle).child("on rewind."));
        let hero = div()
            .flex()
            .flex_col()
            .items_start()
            .gap(space::S5)
            .px(space::S8 + self.inset)
            .child(kicker("Mnema", &p))
            .child(title)
            .child(
                div()
                    .max_w(px(46. * 9.3))
                    .text_size(text::LG)
                    .line_height(relative(1.6))
                    .text_color(p.text_muted)
                    .child("Records your screen so you can scrub back to anything you have seen. Every byte stays on this Mac."),
            )
            .child(
                parts::acts(vec![
                    primary("ob-begin", "Begin setup", cx, |this, _, cx| this.next(cx))
                        .disabled(!self.loaded)
                        .into_any_element(),
                    parts::fine("About a minute. Nothing is recorded until you finish.", &p)
                        .ml(space::S2)
                        .into_any_element(),
                ])
                .mt(space::S2),
            );
        div()
            .id("ob-welcome")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .justify_between()
            .gap(space::S6)
            .pt(px(88.))
            .pb(space::S6)
            .child(hero)
            .child(div().child(film).child(rail))
            .into_any_element()
    }
}
