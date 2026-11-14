//! `audio-transport.svelte.ts` + `DrawerTransport` + `WaveformScrubber`: the
//! player behind a small trait (AVAudioPlayer in the app, a fake in tests), the
//! seek paths and bounded-sample playback, then the transport bar with the
//! speaker-coloured waveform scrubber.
//!
//! Every path that seeks or starts playback ends sample mode first; that rule is
//! why it all lives here.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{
    canvas, div, fill, point, px, size, AnyElement, Bounds, Context, MouseButton, Pixels, Role,
    SharedString, Task,
};

use super::view_model::{player_time, Bar, Marks, StripSpeaker};
use super::{AudioDrawer, Seek};
use crate::theme::{self, radius, size as sz, space, text, Palette, FONT_MONO};
use crate::ui::{alpha, progress, Button};

/// "Play 8s of each": how long each bounded sample runs before the next.
pub const SAMPLE_SECONDS: f64 = 8.;
/// Waveform heights: full (expanded) / compact (peek); the bare track.
const WAVE_H: Pixels = px(30.);
const WAVE_H_COMPACT: Pixels = px(20.);
const TRACK_H: Pixels = px(4.);
const THUMB: Pixels = px(10.);
const HEAD_DOT: Pixels = px(7.);

/// How often a playing clip polls its player (~30 Hz). A poll repaints only when
/// what shows moved, never on every display frame.
pub const PLAY_POLL: Duration = Duration::from_millis(33);
/// Playhead steps across a track whose width the view doesn't measure (~1 px).
pub const TRACK_STEPS: f64 = 600.;

/// Run `step` every [`PLAY_POLL`] until it returns `false` (playback stopped).
/// Keep the task: dropping it (or the view) stops the poll.
pub fn poll_playback<V: 'static>(
    cx: &mut Context<V>,
    step: fn(&mut V, &mut Context<V>) -> bool,
) -> Task<()> {
    cx.spawn(async move |this, cx| loop {
        cx.background_executor().timer(PLAY_POLL).await;
        if !matches!(this.update(cx, step), Ok(true)) {
            break;
        }
    })
}

/// What the drawer needs from a player.
pub trait Player {
    fn play(&mut self) -> bool;
    fn pause(&mut self);
    fn seek(&mut self, seconds: f64);
    fn position(&self) -> f64;
    fn duration(&self) -> f64;
    fn playing(&self) -> bool;
}

#[derive(Default)]
pub struct Transport {
    player: Option<Box<dyn Player>>,
    pub current: f64,
    pub duration: f64,
    pub playing: bool,
    has_seeked: bool,
    /// The scrubber's drag value (seconds): timeupdates must not fight it.
    pub scrubbing: Option<f64>,
    stop_at: Option<f64>,
    queue: Vec<i64>,
}

impl Transport {
    pub fn set_player(&mut self, player: Box<dyn Player>) {
        self.duration = player.duration().max(0.);
        self.player = Some(player);
    }

    pub fn playable(&self) -> bool {
        self.player.is_some()
    }

    pub fn current_ms(&self) -> i64 {
        (self.current * 1000.).round() as i64
    }

    /// What a transport bar shows: playing, the clock's whole second, and the
    /// playhead in `steps` across its track. A poll repaints when this moves.
    pub fn shown(&self, steps: f64) -> (bool, i64, i64) {
        let head = if self.duration > 0. {
            (self.current / self.duration * steps) as i64
        } else {
            0
        };
        (self.playing, self.current as i64, head)
    }

    /// The user has engaged with playback (vs. a drawer open at 0:00).
    pub fn engaged(&self) -> bool {
        self.playing || self.current > 0. || self.has_seeked
    }

    fn end_samples(&mut self) {
        self.stop_at = None;
        self.queue.clear();
    }

    /// Space. `false` when playback could not start (the media error).
    pub fn toggle(&mut self) -> bool {
        self.end_samples();
        let Some(p) = self.player.as_mut() else {
            return true;
        };
        if p.playing() {
            p.pause();
            self.playing = false;
            return true;
        }
        if self.duration > 0. && self.current >= self.duration {
            p.seek(0.);
        }
        self.playing = p.play();
        self.playing
    }

    fn cap(&self) -> f64 {
        if self.duration > 0. {
            self.duration
        } else {
            f64::INFINITY
        }
    }

    fn set(&mut self, seconds: f64) {
        if let Some(p) = self.player.as_mut() {
            p.seek(seconds);
            self.current = seconds;
            self.has_seeked = true;
        }
    }

    pub fn seek(&mut self, to: Seek) {
        self.end_samples();
        let target = match to {
            Seek::By(delta) => self.current + delta,
            Seek::ToMs(ms) => ms as f64 / 1000.,
            Seek::To(s) => s,
        };
        if target.is_finite() {
            self.set(target.clamp(0., self.cap()));
        }
    }

    /// Bounded previews of each head in turn (this voice, then the merge candidate).
    pub fn play_samples(&mut self, heads: &[i64]) {
        let Some((&first, rest)) = heads.split_first() else {
            return;
        };
        if self.player.is_none() {
            return;
        }
        self.queue = rest.to_vec();
        self.set(first as f64 / 1000.);
        self.stop_at = Some(first as f64 / 1000. + SAMPLE_SECONDS);
        self.playing = self.player.as_mut().is_some_and(|p| p.play());
    }

    /// One poll (a display frame while playing): position, the sample bounds, the end.
    pub fn tick(&mut self) {
        let Some(p) = self.player.as_mut() else {
            return;
        };
        let at = p.position();
        if let Some(stop) = self.stop_at.filter(|s| at >= *s) {
            let _ = stop;
            if self.queue.is_empty() {
                self.stop_at = None;
                p.pause();
            } else {
                let next = self.queue.remove(0) as f64 / 1000.;
                p.seek(next);
                self.stop_at = Some(next + SAMPLE_SECONDS);
            }
        }
        let was = self.playing;
        self.playing = p.playing();
        self.current = if was && !self.playing && self.stop_at.is_none() && at <= 0. {
            // AVAudioPlayer rewinds to 0 when it finishes: hold the end.
            self.duration
        } else {
            p.position()
        };
    }
}

/// The transport bar: play, the clock, the waveform scrubber, the duration and
/// (peek, reader showing) the floating "open reader" door.
pub fn render(d: &AudioDrawer, p: &Palette, cx: &mut Context<AudioDrawer>) -> AnyElement {
    let t = &d.transport;
    let shown = t.scrubbing.unwrap_or(t.current);
    let play = Button::icon_only(
        "drawer-play",
        if t.playing {
            IconName::Pause
        } else {
            IconName::Play
        }
        .path(),
        if t.playing { "Pause" } else { "Play" },
    )
    .pressed(t.playing)
    .disabled(!t.playable())
    .on_click(cx.listener(|this, _, _, cx| this.toggle(cx)));
    let middle = if d.media_loading {
        div()
            .flex_1()
            .flex()
            .items_center()
            .gap(space::S2)
            .child(progress::spinner(sz::SPIN_SM))
            .child("loading audio segment…")
            .into_any_element()
    } else {
        scrubber(d, shown, p, cx)
    };
    div()
        .flex()
        .items_center()
        .gap(space::S3)
        .px(px(14.))
        .py(px(10.))
        .border_t_1()
        .border_color(p.hairline)
        .font_family(FONT_MONO)
        .text_size(text::SM)
        .font_weight(theme::W_MEDIUM)
        .text_color(p.text_muted)
        .child(play)
        .child(player_time(shown))
        .child(middle)
        .child(player_time(t.duration))
        .into_any_element()
}

fn bar_color(bar: &Bar, marks: &Marks, p: &Palette) -> gpui_kit::Hsla {
    bar.cluster_id
        .and_then(|c| marks.get(&c))
        .map_or(p.text_faint, |m| m.hue.color(p))
}

/// The scrubber: real amplitude bars coloured by speaker over a seek area, a
/// playhead, and the hover tip naming whoever held the floor there.
fn scrubber(d: &AudioDrawer, shown: f64, p: &Palette, cx: &mut Context<AudioDrawer>) -> AnyElement {
    let t = &d.transport;
    let duration = t.duration;
    let progress = if duration > 0. {
        (shown / duration).clamp(0., 1.) as f32
    } else {
        0.
    };
    let bars = d.bars.clone();
    let marks = d.marks.clone();
    let sel = d.selected_cluster;
    let now_ms = (shown * 1000.) as i64;
    let compact = !d.expanded;
    let has_bars = !bars.is_empty();
    let bounds: Rc<Cell<Bounds<Pixels>>> = d.wave_bounds.clone();
    let b2 = bounds.clone();
    let pal = *p;
    let paint = canvas(
        move |b, _, _| b2.set(b),
        move |b, _, window, _| {
            if bars.is_empty() {
                let mid = b.origin.y + b.size.height / 2. - TRACK_H / 2.;
                let track = Bounds::new(point(b.origin.x, mid), size(b.size.width, TRACK_H));
                window.paint_quad(fill(track, pal.surface_hover).corner_radii(px(2.)));
                let done = Bounds::new(track.origin, size(b.size.width * progress, TRACK_H));
                window.paint_quad(fill(done, pal.record_glyph_start).corner_radii(px(2.)));
                return;
            }
            let n = bars.len() as f32;
            let gap = px(1.);
            let w = ((b.size.width - gap * (n - 1.)) / n).max(px(1.));
            for (i, bar) in bars.iter().enumerate() {
                let h = b.size.height * (bar.height_pct / 100.);
                let x = b.origin.x + (w + gap) * i as f32;
                let y = b.origin.y + (b.size.height - h) / 2.;
                let dim = sel.is_some_and(|s| bar.cluster_id != Some(s));
                let o = if dim {
                    0.1
                } else if bar.at_ms <= now_ms {
                    1.
                } else if bar.cluster_id.is_none() {
                    0.22
                } else if sel.is_some() {
                    0.6
                } else {
                    0.42
                };
                let c = alpha(bar_color(bar, &marks, &pal), o);
                window
                    .paint_quad(fill(Bounds::new(point(x, y), size(w, h)), c).corner_radii(px(1.)));
            }
        },
    )
    .absolute()
    .inset_0();
    let seek_to = move |x: Pixels, b: Bounds<Pixels>| {
        let f = ((x - b.origin.x) / b.size.width).clamp(0., 1.) as f64;
        f * duration
    };
    let head_x = d.wave_bounds.get().size.width * progress;
    let hover = d.wave_hover.and_then(|x| {
        let b = bounds.get();
        if !has_bars || b.size.width <= px(0.) {
            return None;
        }
        let f = (x / b.size.width).clamp(0., 1.);
        let bar = d
            .bars
            .get(((f * d.bars.len() as f32) as usize).min(d.bars.len() - 1))?;
        let who: Option<&StripSpeaker> = d
            .strip
            .iter()
            .find(|s| Some(s.cluster_id) == bar.cluster_id);
        Some((
            x,
            who.map(|s| s.name.clone()),
            player_time(f as f64 * duration),
        ))
    });
    let halo = crate::ui::spread(alpha(p.record_glyph_start, 0.18), px(4.));
    div()
        .id("drawer-wave")
        .group("drawer-wave")
        .role(Role::Slider)
        .aria_label(SharedString::from(format!(
            "Seek within segment, {} of {}",
            player_time(shown),
            player_time(duration)
        )))
        .relative()
        .flex_1()
        .min_w_0()
        .h(if !has_bars {
            px(18.)
        } else if compact {
            WAVE_H_COMPACT
        } else {
            WAVE_H
        })
        .rounded(radius::XS)
        .when(duration > 0., |el| el.cursor_pointer())
        .when(duration <= 0., |el| el.opacity(theme::DISABLED_OPACITY))
        .child(paint)
        .when(has_bars, |el| {
            el.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(head_x)
                    .w(px(1.))
                    .bg(p.accent_strong)
                    .shadow(vec![crate::ui::spread(p.accent_glow, px(1.))])
                    .when(!compact, |h| {
                        h.child(
                            div()
                                .absolute()
                                .top(px(-2.))
                                .left(px(-3.))
                                .size(HEAD_DOT)
                                .rounded(radius::PILL)
                                .bg(p.accent_strong),
                        )
                    }),
            )
        })
        .when(!has_bars && duration > 0., |el| {
            el.child(
                div()
                    .absolute()
                    .top(px(4.))
                    .left(head_x - THUMB / 2.)
                    .size(THUMB)
                    .rounded(radius::PILL)
                    .border_2()
                    .border_color(p.surface_raised)
                    .bg(p.status_running_fg)
                    .group_hover("drawer-wave", move |s| s.shadow(vec![halo.clone()])),
            )
        })
        .children(hover.map(|(x, who, at)| {
            div()
                .absolute()
                .bottom(px(38.))
                .left(x - px(40.))
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(8.))
                .py(px(5.))
                .rounded(radius::SM)
                .border_1()
                .border_color(p.overlay_border)
                .bg(p.overlay_bg_strong)
                .shadow(crate::ui::popover_shadow(p))
                .whitespace_nowrap()
                .font_family(theme::FONT_UI)
                .text_color(p.text_strong)
                .child(who.unwrap_or_else(|| "no speech".into()))
                .child(
                    div()
                        .font_family(FONT_MONO)
                        .text_color(p.text_subtle)
                        .child(at),
                )
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, e: &gpui_kit::MouseDownEvent, _, cx| {
                if duration > 0. {
                    this.transport.scrubbing = Some(seek_to(e.position.x, this.wave_bounds.get()));
                    cx.notify();
                }
            }),
        )
        .on_mouse_move(
            cx.listener(move |this, e: &gpui_kit::MouseMoveEvent, _, cx| {
                let b = this.wave_bounds.get();
                this.wave_hover = b.contains(&e.position).then(|| e.position.x - b.origin.x);
                if this.transport.scrubbing.is_some() && e.dragging() {
                    this.transport.scrubbing = Some(seek_to(e.position.x, b));
                }
                cx.notify();
            }),
        )
        .on_hover(cx.listener(|this, on: &bool, _, cx| {
            if !on {
                this.wave_hover = None;
                cx.notify();
            }
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                if let Some(s) = this.transport.scrubbing.take() {
                    this.transport.seek(Seek::To(s));
                    cx.notify();
                }
            }),
        )
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                if let Some(s) = this.transport.scrubbing.take() {
                    this.transport.seek(Seek::To(s));
                    cx.notify();
                }
            }),
        )
        .into_any_element()
}
