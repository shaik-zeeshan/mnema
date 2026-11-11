//! An activity's receipt, inline under its Journal card (`ReceiptViewer.svelte`):
//! the real captured frames over the card's span played as a timelapse — no
//! video, just frame previews swapped on GPUI animation frames at `speed`
//! frames per second (8× default, 16× fast) — with a scrub track and the
//! frame's clock.
//!
//! The cited audio (the spoken turns, 1× relive) is `audio.rs` + `audio_view.rs`.

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

/// The speed picker (`SPEEDS` without 2×): 1× is the audible relive, faster is silent.
pub const SPEEDS: [u32; 3] = [1, 8, 16];
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
    /// The span's spoken turns and the clip playing.
    pub audio: super::audio_view::ReceiptAudio,
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
            audio: Default::default(),
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
        if let Some(a) = self
            .receipt
            .as_mut()
            .map(|r| &mut r.audio)
            .filter(|a| a.transport.playing)
        {
            a.transport.toggle(); // the sound never outlives the receipt
        }
        if self.receipt.as_ref().is_some_and(|r| r.activity_id == id) {
            self.receipt = None;
        } else {
            self.receipt = Some(Receipt::new(id, span));
            self.load_receipt_audio(id, span, cx);
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
    if this
        .receipt
        .as_ref()
        .is_some_and(|r| r.audio.transport.playing)
    {
        this.tick_receipt_audio(window, cx);
        this.fetch_previews(cx);
    }
    let off = clock::now(cx).offset();
    let Some(rc) = this.receipt.as_ref() else {
        return div().into_any_element();
    };
    let foot = |s: String| foot(s, p);
    let shell = shell(p).id("insights-receipt");
    let act = this
        .journal
        .as_ref()
        .and_then(|j| j.activities.iter().find(|a| a.id == rc.activity_id));
    let cited_n = |kind: &str| {
        act.map_or(0, |a| {
            a.evidence.iter().filter(|e| e.subject_type == kind).count()
        })
    };
    let (cited, spoken) = (cited_n("frame"), cited_n("audio_segment"));
    let a = &rc.audio;
    let turns = a.turns().len();
    let empty_frames = match &rc.frames {
        None => {
            return shell
                .child(skeleton::thumb(p))
                .child(foot("Loading footage…".into()))
                .into_any_element()
        }
        Some(Ok(f)) => f.is_empty(),
        Some(Err(_)) => true,
    };
    // `receiptViewState`: frames, else audio, else the error, else expired / none.
    let audio_only =
        empty_frames && (turns > 0 || a.transcribing().is_some() || (spoken > 0 && a.pending()));
    let failed = matches!(rc.frames, Some(Err(_))) || matches!(a.span, Some(Err(_)));
    let policy = this.status.settings.as_ref().map(|s| &s.retention_policy);
    let removed = matches!(
        crate::insights::parts::retention(rc.span.0, rc.span.1, policy, clock::now_ms(cx)),
        Some((true, _))
    );
    let retention_days =
        crate::insights::parts::retention(rc.span.0, rc.span.1, policy, clock::now_ms(cx))
            .map(|r| r.1);
    if empty_frames && !audio_only {
        if failed {
            let id = rc.activity_id;
            let span = rc.span;
            let retry = Button::new("insights-receipt-retry", "Try again")
                .sm()
                .icon(IconName::RotateCcw.path())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.receipt = None;
                    this.toggle_receipt(id, span, cx);
                }));
            return shell
                .child(empty(
                    EmptyLayout::Compact,
                    Tone::Danger,
                    IconName::TriangleAlert.path(),
                    "Couldn't load footage",
                    "Reading this activity's frames failed this time. The summary above isn't affected.",
                    vec![retry.into_any_element()],
                    p,
                ))
                .child(foot("Footage didn't load · summary shown above".into()))
                .into_any_element();
        }
        let (title, body, f) = match retention_days {
            Some(days) => (
                "Footage expired",
                format!("Removed by your {days}-day retention. The card, its summary and its evidence list are kept — only the pixels age out."),
                "0 frames still on disk · summary retained",
            ),
            None => (
                "No screen capture",
                "No screen frames were captured during this activity. The card and its summary are kept.".to_string(),
                "No screen capture · summary retained",
            ),
        };
        return shell
            .child(empty(
                EmptyLayout::Compact,
                Tone::Neutral,
                IconName::Inbox.path(),
                title,
                body,
                Vec::new(),
                p,
            ))
            .child(foot(f.into()))
            .into_any_element();
    }
    let frames: &[(i64, i64)] = match &rc.frames {
        Some(Ok(f)) => f,
        _ => &[],
    };
    let n = frames.len();
    let audible = audio_only || (rc.speed == 1 && turns > 0);
    let clip_playing = a.transport.playing;
    let is_playing = if audible { clip_playing } else { rc.playing };
    let head_ms = if audio_only {
        a.head().or(a.selected_turn().map(|t| t.start))
    } else {
        frames.get(rc.index.min(n.saturating_sub(1))).map(|f| f.1)
    };
    // The scrub track: the frame playhead, or (audio-only) the clip's place in the span.
    let head = if audio_only {
        let w = (rc.span.1 - rc.span.0).max(1) as f32;
        head_ms.map_or(0., |ms| ((ms - rc.span.0) as f32 / w).clamp(0., 1.))
    } else if n > 1 {
        rc.index as f32 / (n - 1) as f32
    } else {
        0.
    };
    let speed_ix = SPEEDS.iter().position(|s| *s == rc.speed).unwrap_or(1);
    let entity = cx.entity().downgrade();
    let controls = div()
        .flex()
        .items_center()
        .gap(space::S3)
        .child(
            Button::new(
                "insights-receipt-play",
                if is_playing { "Pause" } else { "Play" },
            )
            .sm()
            .icon(if is_playing {
                IconName::Pause.path()
            } else {
                IconName::Play.path()
            })
            .disabled(audio_only && a.selected.is_none())
            .on_click(cx.listener(|this, _, _, cx| this.receipt_play(cx))),
        )
        .when(!audio_only, |d| {
            d.child(
                Segmented::new(
                    "insights-receipt-speed",
                    "Playback speed",
                    SPEEDS.map(|s| format!("{s}×")),
                    speed_ix,
                )
                .mono()
                .on_change(move |ix, _, cx| {
                    entity
                        .update(cx, |this, cx| this.receipt_speed(SPEEDS[ix], cx))
                        .ok();
                }),
            )
        })
        .child(div().flex_1())
        .children(head_ms.map(|ms| {
            mono_label(
                crate::timeline::model::clock_label(stats::local(ms, off), true),
                p,
            )
        }));
    // Lands on the frame the receipt is showing; audio-only, on the selected segment.
    let fid = frames.get(rc.index.min(n.saturating_sub(1))).map(|f| f.0);
    let seg = a
        .selected_turn()
        .map(|t| (t.segment, t.start - t.seg_start));
    let open = Button::new("insights-receipt-open", "Open in Timeline")
        .ghost()
        .sm()
        .icon(IconName::ArrowRight.path())
        .icon_after()
        .on_click(cx.listener(move |_, _, _, cx| match (fid, seg) {
            (Some(f), _) => cx.emit(crate::insights::InsightsEvent::OpenFrame(f)),
            (None, Some((id, at))) => {
                cx.defer(move |cx| crate::timeline::open_audio(id, at, None, None, cx))
            }
            _ => {}
        }))
        .into_any_element();
    let right = if turns > 0 {
        super::audio_view::turn_list(this, open, p, cx)
    } else if audio_only {
        super::audio_view::nothing(p)
    } else {
        div().child(open)
    };
    // `.in-scrub i`: each cited frame at its time in the span.
    let w = (rc.span.1 - rc.span.0).max(1) as f32;
    let ticks: Vec<Div> = act
        .map(|a| {
            a.evidence
                .iter()
                .filter(|e| e.subject_type == "frame")
                .filter_map(|e| {
                    let ms = frames
                        .iter()
                        .find(|f| f.0 == e.subject_id)
                        .map(|f| f.1)
                        .or(e.captured_at_ms)?;
                    let x = ((ms - rc.span.0) as f32 / w).clamp(0., 1.);
                    Some(
                        div()
                            .absolute()
                            .top(px(4.))
                            .left(relative(x))
                            .w(px(2.))
                            .h(px(10.))
                            .rounded(px(1.))
                            .bg(p.cat_meetings),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let left = if audio_only {
        div()
            .flex()
            .flex_col()
            .gap(space::S3)
            .child(super::audio_view::audio_box(this, p, cx))
            .when(a.selected.is_some(), |d| {
                d.child(track(head, p)).child(controls)
            })
    } else {
        let (fid, _) = frames[rc.index.min(n - 1)];
        div()
            .flex()
            .flex_col()
            .gap(space::S3)
            .child(screen(rc.previews.get(&fid), &this.images, p))
            .when(n > 1 || turns > 0, |d| {
                d.child(track(head, p).children(ticks)).child(controls)
            })
            .children(a.media_error.clone().map(|e| {
                crate::ui::notice::inline(
                    Tone::Danger,
                    None,
                    vec![div().child(e).into_any_element()],
                    p,
                )
            }))
    };
    let footer = if audio_only {
        let left = if removed {
            "0 screen frames — screen frames have expired"
        } else {
            "0 screen frames — captured as audio"
        };
        let right = if a.transcribing().is_some() {
            "transcript on its way".to_string()
        } else {
            let r = super::audio::roster(a.turns());
            if r.is_empty() {
                format!(
                    "{spoken} spoken segment{} cited",
                    if spoken == 1 { "" } else { "s" }
                )
            } else {
                r
            }
        };
        format!("{left} · {right}")
    } else {
        let times: Vec<i64> = frames.iter().map(|f| f.1).collect();
        footage(n, segments(&times), cited, spoken)
    };
    shell
        .role(Role::Group)
        .child(
            div()
                .flex()
                .gap(space::S5)
                .child(div().flex_1().min_w_0().child(left))
                .child(div().flex_1().min_w_0().child(right)),
        )
        .child(foot(footer))
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
pub fn footage(frames: usize, segs: usize, cited: usize, spoken: usize) -> String {
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
    let parts: Vec<String> = [(cited, "frame"), (spoken, "spoken segment")]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, w)| count(n, w))
        .collect();
    if parts.is_empty() {
        strip
    } else {
        format!("{strip} · {} cited", parts.join(" + "))
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
