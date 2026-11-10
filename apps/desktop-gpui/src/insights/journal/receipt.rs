//! An activity's receipt, inline under its Journal card (`ReceiptViewer.svelte`):
//! the real captured frames over the card's span played as a timelapse — no
//! video, just frame previews swapped on GPUI animation frames at `speed`
//! frames per second (8× default, 16× fast) — with a scrub track and the
//! frame's clock.
//!
//! shortcut: cited audio (1×, per-turn relive: `receipt-audio.ts` +
//! `receipt-audio-loader.ts`, ~600 lines of turn hydration and scheduling) is not
//! wired yet, so 1× plays silently; `timeline::audio::transport::Player` /
//! `native::audio_player::AudioPlayer` are the player to build it on.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, img, px, relative, AnyElement, Context, Div, Entity, Role, Window};

use crate::insights::parts::{inset, mono_label};
use crate::insights::stats;
use crate::insights::Insights;
use crate::theme::{radius, space, text, Palette, FONT_MONO};
use crate::timeline::image_cache::FrameImages;
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::segmented::Segmented;
use crate::ui::{clock, skeleton, Button, Tone};

/// `SPEEDS`: 1× is the audible relive (inert until S9); faster is silent.
pub const SPEEDS: [u32; 4] = [1, 2, 8, 16];
/// Preview paths fetched ahead of the playhead.
const LOOKAHEAD: usize = 8;

pub struct Receipt {
    pub activity_id: i64,
    pub span: (i64, i64),
    /// `None` while loading; `Some(Err)` = the listing failed.
    pub frames: Option<Result<Vec<(i64, i64)>, String>>,
    pub index: usize,
    pub playing: bool,
    pub speed: u32,
    accum: f64,
    last: Option<Instant>,
    pub previews: HashMap<i64, PathBuf>,
    requested: HashSet<i64>,
}

impl Receipt {
    pub fn new(activity_id: i64, span: (i64, i64)) -> Self {
        Self {
            activity_id,
            span,
            frames: None,
            index: 0,
            playing: false,
            speed: 8,
            accum: 0.,
            last: None,
            previews: HashMap::new(),
            requested: HashSet::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.frames
            .as_ref()
            .and_then(|f| f.as_ref().ok())
            .map_or(0, Vec::len)
    }

    /// Play from `now`; at the end, start over.
    pub fn toggle(&mut self, now: Instant) {
        if self.playing {
            self.playing = false;
        } else if self.len() > 1 {
            if self.index + 1 >= self.len() {
                self.index = 0;
            }
            self.playing = true;
            self.accum = 0.;
            self.last = Some(now);
        }
    }

    /// One animation frame: advance `speed` frames per second of `now - last`.
    pub fn advance(&mut self, now: Instant) {
        if !self.playing {
            return;
        }
        let dt = self
            .last
            .map_or(0., |l| now.saturating_duration_since(l).as_secs_f64());
        self.last = Some(now);
        self.accum += dt * self.speed as f64;
        let steps = self.accum.floor() as usize;
        self.accum -= steps as f64;
        self.index += steps;
        if self.index + 1 >= self.len() {
            self.index = self.len().saturating_sub(1);
            self.playing = false;
        }
    }

    /// Frame ids around the playhead whose preview path isn't asked for yet.
    pub fn wanted(&mut self) -> Vec<i64> {
        let Some(Ok(frames)) = &self.frames else {
            return Vec::new();
        };
        let ids: Vec<i64> = frames
            .iter()
            .skip(self.index)
            .take(LOOKAHEAD + 1)
            .map(|f| f.0)
            .collect();
        ids.into_iter()
            .filter(|id| self.requested.insert(*id))
            .collect()
    }
}

impl Insights {
    pub fn toggle_receipt(&mut self, id: i64, span: (i64, i64), cx: &mut Context<Self>) {
        if self.receipt.as_ref().is_some_and(|r| r.activity_id == id) {
            self.receipt = None;
        } else {
            self.receipt = Some(Receipt::new(id, span));
            let (s, e) = span;
            self.write(
                cx,
                move |b| b.frames(s, e),
                move |this, r, cx| {
                    if let Some(rc) = this.receipt.as_mut().filter(|rc| rc.activity_id == id) {
                        if let Ok(f) = &r {
                            rc.index = f.len().saturating_sub(1) / 2;
                        }
                        rc.frames = Some(r);
                        this.fetch_previews(cx);
                    }
                },
            );
        }
        cx.notify();
    }

    fn fetch_previews(&mut self, cx: &mut Context<Self>) {
        let Some(rc) = self.receipt.as_mut() else {
            return;
        };
        let id = rc.activity_id;
        for frame in rc.wanted() {
            self.write(
                cx,
                move |b| b.frame_preview(frame),
                move |this, path, _| {
                    if let (Some(rc), Some(path)) =
                        (this.receipt.as_mut().filter(|r| r.activity_id == id), path)
                    {
                        rc.previews.insert(frame, path);
                    }
                },
            );
        }
    }
}

pub fn render(
    this: &mut Insights,
    p: &Palette,
    window: &mut Window,
    cx: &mut Context<Insights>,
) -> AnyElement {
    let now = cx.background_executor().now();
    let playing = this.receipt.as_ref().is_some_and(|r| r.playing);
    if playing {
        if let Some(rc) = this.receipt.as_mut() {
            rc.advance(now);
        }
        this.fetch_previews(cx);
        window.request_animation_frame();
    }
    let off = clock::now(cx).offset();
    let Some(rc) = this.receipt.as_ref() else {
        return div().into_any_element();
    };
    let foot = |s: String| foot(s, p);
    let shell = shell(p).id("insights-receipt");
    let frames = match &rc.frames {
        None => {
            return shell
                .child(skeleton::thumb(p))
                .child(foot("Loading footage…".into()))
                .into_any_element()
        }
        Some(Err(_)) => {
            return shell
                .child(empty(
                    EmptyLayout::Compact,
                    Tone::Danger,
                    IconName::TriangleAlert.path(),
                    "Couldn't load footage",
                    "Reading this activity's frames failed this time. The summary above isn't affected.",
                    Vec::new(),
                    p,
                ))
                .child(foot("Footage didn't load · summary shown above".into()))
                .into_any_element()
        }
        Some(Ok(f)) if f.is_empty() => {
            return shell
                .child(empty(
                    EmptyLayout::Compact,
                    Tone::Neutral,
                    IconName::Inbox.path(),
                    "No screen capture",
                    "No screen frames were captured during this activity. The card and its summary are kept.",
                    Vec::new(),
                    p,
                ))
                .child(foot("No screen capture · summary retained".into()))
                .into_any_element()
        }
        Some(Ok(f)) => f,
    };
    let (fid, fms) = frames[rc.index.min(frames.len() - 1)];
    let screen = screen(rc.previews.get(&fid), &this.images, p);
    let n = frames.len();
    let head = if n > 1 {
        rc.index as f32 / (n - 1) as f32
    } else {
        0.
    };
    let track = track(head, p);
    let speed_ix = SPEEDS.iter().position(|s| *s == rc.speed).unwrap_or(2);
    let entity = cx.entity().downgrade();
    let controls = div()
        .flex()
        .items_center()
        .gap(space::S3)
        .child(
            Button::new(
                "insights-receipt-play",
                if rc.playing { "Pause" } else { "Play" },
            )
            .sm()
            .icon(if rc.playing {
                IconName::Pause.path()
            } else {
                IconName::Play.path()
            })
            .on_click(cx.listener(|this, _, _, cx| {
                let now = cx.background_executor().now();
                if let Some(rc) = this.receipt.as_mut() {
                    rc.toggle(now);
                }
                cx.notify();
            })),
        )
        .child(
            Segmented::new(
                "insights-receipt-speed",
                "Playback speed",
                SPEEDS.map(|s| format!("{s}×")),
                speed_ix,
            )
            .mono()
            .on_change(move |ix, _, cx| {
                entity
                    .update(cx, |this, cx| {
                        if let Some(rc) = this.receipt.as_mut() {
                            rc.speed = SPEEDS[ix];
                        }
                        cx.notify();
                    })
                    .ok();
            }),
        )
        .child(div().flex_1())
        .child(mono_label(
            crate::timeline::model::clock_label(stats::local(fms, off), true),
            p,
        ));
    // Lands on the frame the receipt is showing.
    let open = Button::new("insights-receipt-open", "Open in Timeline")
        .ghost()
        .sm()
        .icon(IconName::ArrowRight.path())
        .icon_after()
        .on_click(
            cx.listener(move |_, _, _, cx| cx.emit(crate::insights::InsightsEvent::OpenFrame(fid))),
        );
    let cited = this
        .journal
        .as_ref()
        .and_then(|j| j.activities.iter().find(|a| a.id == rc.activity_id))
        .map_or(0, |a| {
            a.evidence
                .iter()
                .filter(|e| e.subject_type == "frame")
                .count()
        });
    let times: Vec<i64> = frames.iter().map(|f| f.1).collect();
    shell
        .role(Role::Group)
        .child(
            div()
                .flex()
                .gap(space::S5)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(space::S3)
                        .child(screen)
                        .when(n > 1, |d| d.child(track).child(controls)),
                )
                .child(div().flex_1().min_w_0().child(open)),
        )
        .child(foot(footage(n, segments(&times), cited)))
        .into_any_element()
}

/// The receipt's box (`.in-receipt`); Chat's audio receipt uses it too.
pub fn shell(p: &Palette) -> Div {
    inset(p)
        .mt(space::S3)
        .p(space::S4)
        .flex()
        .flex_col()
        .gap(space::S3)
}

/// The frame on screen: its preview through `images`, a skeleton until it lands.
pub fn screen(path: Option<&PathBuf>, images: &Entity<FrameImages>, p: &Palette) -> Div {
    div()
        .relative()
        .w_full()
        .h(px(200.))
        .rounded(radius::MD)
        .overflow_hidden()
        .bg(p.surface_subtle)
        .child(match path {
            Some(path) => img(path.clone())
                .image_cache(images)
                .size_full()
                .object_fit(gpui_kit::ObjectFit::Contain)
                .into_any_element(),
            None => skeleton::skel(p).size_full().into_any_element(),
        })
}

/// The scrub track with the playhead at `head` (0..=1).
pub fn track(head: f32, p: &Palette) -> Div {
    div()
        .relative()
        .h(px(18.))
        .rounded(radius::XS)
        .bg(p.wash)
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .w(px(2.))
                .left(relative(head.clamp(0., 1.)))
                .bg(p.accent),
        )
}

/// The mono footer under a hairline.
pub fn foot(s: String, p: &Palette) -> Div {
    div()
        .pt(space::S3)
        .border_t_1()
        .border_color(p.hairline)
        .font_family(FONT_MONO)
        .text_size(text::XS)
        .text_color(p.text_subtle)
        .child(s)
}

/// `countCaptureSegments`: a gap over 90 s starts a new segment.
// shortcut: FrameSummaryDto carries no segment id (the Svelte heuristic too).
pub fn segments(sorted_ms: &[i64]) -> usize {
    if sorted_ms.is_empty() {
        return 0;
    }
    1 + sorted_ms
        .windows(2)
        .filter(|w| w[1] - w[0] > 90_000)
        .count()
}

/// `receiptFootage`: "12 frames across 2 capture segments · 3 frames cited".
pub fn footage(frames: usize, segs: usize, cited: usize) -> String {
    let count = |n: usize, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });
    let strip = if segs > 1 {
        format!(
            "{} across {}",
            count(frames, "frame"),
            count(segs, "capture segment")
        )
    } else {
        count(frames, "frame")
    };
    if cited > 0 {
        format!("{strip} · {} cited", count(cited, "frame"))
    } else {
        strip
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn timelapse_steps_speed_frames_per_second_and_stops_at_the_end() {
        let t0 = Instant::now();
        let mut r = Receipt::new(1, (0, 1));
        r.frames = Some(Ok((0..40).map(|i| (i, i * 1000)).collect()));
        r.toggle(t0);
        r.advance(t0 + Duration::from_millis(124));
        assert_eq!(r.index, 0, "8× needs 125 ms per frame");
        r.advance(t0 + Duration::from_millis(1000));
        assert_eq!(r.index, 8);
        r.speed = 16;
        r.advance(t0 + Duration::from_millis(1500));
        assert_eq!(r.index, 16);
        r.advance(t0 + Duration::from_secs(10));
        assert_eq!((r.index, r.playing), (39, false));
        r.toggle(t0);
        assert_eq!(
            (r.index, r.playing),
            (0, true),
            "play at the end starts over"
        );
    }
}
