//! An audio source's inline receipt (`ReceiptViewer` for a cited clip), under
//! its answer's sources row: the clip's real audio at 1× through the shared
//! player (the Timeline drawer's `Transport` over AVAudioPlayer), the frames
//! captured over its span following the same clock (Insights' receipt strip and
//! screen), and the way into the Timeline. A second click on the chip, or Esc,
//! closes it. A clip retention removed says so.
//!
//! shortcut: no per-turn transcript rows and no silent 8×/16× timelapse
//! (`receipt-audio.ts`); add them to Insights' receipt first, then share here.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, AnyElement, Context, FocusHandle, Role, SharedString, Task, Window};

use super::{Chat, Source};
use crate::insights::journal::receipt::{foot, screen, shell, track, Receipt};
use crate::keymap::CloseOverlay;
use crate::theme::{space, text, Palette, FONT_MONO};
use crate::timeline::audio::transport::{poll_playback, Transport, TRACK_STEPS};
use crate::timeline::audio::view_model::player_time;
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::notice::inline;
use crate::ui::{icon, skeleton, Button, Tone};

#[derive(Clone, Debug, PartialEq)]
pub enum ClipLoad {
    Loading,
    Ready,
    /// `get_audio_segment` found no row: retention (or a delete) removed it.
    Gone,
    Failed(String),
}

pub struct Clip {
    /// The answer whose sources row it opened under.
    pub turn: i64,
    pub segment_id: i64,
    pub mic: bool,
    /// `(started, ended)` ms of the cited span.
    pub span: (i64, i64),
    pub load: ClipLoad,
    pub transport: Transport,
    /// The player couldn't open or start (a missing or unreadable file).
    pub media_error: Option<String>,
    /// The frames over the span and their previews.
    pub strip: Receipt,
    focus: FocusHandle,
    /// Polls the player while it plays; closing the clip drops it.
    poll: Option<Task<()>>,
}

fn ms(iso: &str) -> Option<i64> {
    time::OffsetDateTime::parse(iso, &time::format_description::well_known::Rfc3339)
        .ok()
        .map(|t| (t.unix_timestamp_nanos() / 1_000_000) as i64)
}

/// The strip index showing at `at_ms`: the newest frame captured by then.
pub fn index_at(frames: &[(i64, i64)], at_ms: i64) -> usize {
    frames.partition_point(|f| f.1 <= at_ms).saturating_sub(1)
}

impl Chat {
    /// An audio chip: open its receipt (a second click on the same chip closes it).
    pub(super) fn toggle_clip(
        &mut self,
        turn: i64,
        src: Source,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = src.audio_segment_id else {
            return;
        };
        if self
            .clip
            .as_ref()
            .is_some_and(|c| c.turn == turn && c.segment_id == id)
        {
            return self.close_clip(cx);
        }
        self.clip_gen += 1;
        let gen = self.clip_gen;
        let span = ms(&src.started_at)
            .zip(ms(&src.ended_at))
            .filter(|(s, e)| e > s);
        // No playable span: the raw-Timeline hand-off (`openInTimeline`) instead.
        let Some(span) = span else {
            let aligned = src.aligned_frame_id;
            let at = src.span_start_ms.unwrap_or(0);
            cx.defer(move |cx| crate::timeline::open_audio(id, at, aligned, None, cx));
            return;
        };
        let span = Some(span);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        self.clip = Some(Clip {
            turn,
            segment_id: id,
            mic: src.source_kind.as_deref() == Some("microphone"),
            span: span.unwrap_or_default(),
            load: ClipLoad::Loading,
            transport: Transport::default(),
            media_error: None,
            strip: Receipt::new(id, span.unwrap_or_default()),
            focus,
            poll: None,
        });
        cx.notify();
        let (audio, b) = (self.audio.clone(), self.sources.clone());
        let task = cx.background_spawn(async move {
            let seg = audio.backend.segment(id);
            let player = match &seg {
                Ok(Some(s)) => Some((audio.open_player)(&s.file_path)),
                _ => None,
            };
            let strip = match span {
                Some((s, e)) => b.frames(s, e),
                None => Ok(Vec::new()),
            };
            (seg, player, strip)
        });
        cx.spawn(async move |this, cx| {
            let (seg, player, strip) = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.clip_gen != gen {
                    return; // closed, or another clip opened
                }
                let Some(c) = this.clip.as_mut() else { return };
                c.load = match seg {
                    Ok(Some(_)) => ClipLoad::Ready,
                    Ok(None) => ClipLoad::Gone,
                    Err(e) => ClipLoad::Failed(e),
                };
                match player {
                    Some(Ok(p)) => c.transport.set_player(p),
                    Some(Err(e)) => c.media_error = Some(e),
                    None => {}
                }
                c.strip.frames = Some(strip);
                this.fetch_clip_previews(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn close_clip(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = self.clip.as_mut().filter(|c| c.transport.playing) {
            c.transport.toggle(); // pause: the sound never outlives the receipt
        }
        self.clip = None;
        self.clip_gen += 1;
        cx.notify();
    }

    fn toggle_clip_play(&mut self, cx: &mut Context<Self>) {
        let Some(c) = self.clip.as_mut() else { return };
        if !c.transport.toggle() {
            c.media_error = Some("Playback didn't start.".into());
        }
        cx.notify();
    }

    /// Preview paths for the frames around the strip's playhead.
    fn fetch_clip_previews(&mut self, cx: &mut Context<Self>) {
        let Some(c) = self.clip.as_mut() else { return };
        let gen = self.clip_gen;
        for frame in c.strip.wanted() {
            let b = self.sources.clone();
            let task = cx.background_spawn(async move { b.preview(frame) });
            cx.spawn(async move |this, cx| {
                let path = task.await;
                let _ = this.update(cx, |this, cx| {
                    if let (true, Some(c), Some(path)) =
                        (this.clip_gen == gen, this.clip.as_mut(), path)
                    {
                        c.strip.previews.insert(frame, path);
                        cx.notify();
                    }
                });
            })
            .detach();
        }
    }

    /// While the clip plays, poll its player on a timer (from render, so any
    /// path that starts playback arms it).
    pub(super) fn tick_clip(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = self
            .clip
            .as_mut()
            .filter(|c| c.transport.playing && c.poll.is_none())
        {
            c.poll = Some(poll_playback(cx, Self::poll_clip));
        }
    }

    /// One poll: move the strip to the audio clock; repaint only when the
    /// clock, the playhead or the frame moved.
    fn poll_clip(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(c) = self.clip.as_mut() else {
            return false;
        };
        let before = (c.transport.shown(TRACK_STEPS), c.strip.index);
        c.transport.tick();
        let at = c.span.0 + c.transport.current_ms();
        if let Some(Ok(frames)) = &c.strip.frames {
            c.strip.index = index_at(frames, at);
        }
        let playing = c.transport.playing;
        if !playing {
            c.poll = None;
        }
        if (c.transport.shown(TRACK_STEPS), c.strip.index) != before {
            self.fetch_clip_previews(cx);
            cx.notify();
        }
        playing
    }
}

const GONE: &str = "This clip's audio is no longer stored. Retention or a delete removed it; the answer is unchanged.";

pub fn render(chat: &Chat, turn: i64, p: &Palette, cx: &mut Context<Chat>) -> Option<AnyElement> {
    let c = chat.clip.as_ref().filter(|c| c.turn == turn)?;
    let kind = if c.mic {
        "Microphone audio"
    } else {
        "System audio"
    };
    let foot = |s: String| foot(s, p);
    let base = shell(p)
        .id("chat-clip")
        .role(Role::Group)
        .aria_label(kind)
        .track_focus(&c.focus)
        .on_action(cx.listener(|this, _: &CloseOverlay, _, cx| this.close_clip(cx)));
    let el = match &c.load {
        ClipLoad::Loading => base
            .child(skeleton::thumb(p))
            .child(foot("Loading clip…".into())),
        ClipLoad::Gone => base
            .child(
                div()
                    .id("chat-clip-gone")
                    .test_support()
                    .role(Role::Status)
                    .aria_label("Clip expired")
                    .child(empty(
                        EmptyLayout::Compact,
                        Tone::Neutral,
                        IconName::AudioWaveform.path(),
                        "Clip expired",
                        GONE,
                        Vec::new(),
                        p,
                    )),
            )
            .child(foot(format!("{kind} · removed"))),
        ClipLoad::Failed(_) => base
            .child(empty(
                EmptyLayout::Compact,
                Tone::Danger,
                IconName::TriangleAlert.path(),
                "Couldn't load this clip",
                "Reading the clip failed this time. The answer above isn't affected.",
                Vec::new(),
                p,
            ))
            .child(foot(format!("{kind} · didn't load"))),
        ClipLoad::Ready => ready(base, chat, c, kind, p, cx),
    };
    Some(el.into_any_element())
}

fn ready(
    base: gpui_kit::Stateful<gpui_kit::Div>,
    chat: &Chat,
    c: &Clip,
    kind: &str,
    p: &Palette,
    cx: &mut Context<Chat>,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let t = &c.transport;
    let frames: &[(i64, i64)] = match &c.strip.frames {
        Some(Ok(f)) => f,
        _ => &[],
    };
    let shown = frames.get(c.strip.index.min(frames.len().saturating_sub(1)));
    let head = if t.duration > 0. {
        (t.current / t.duration) as f32
    } else {
        0.
    };
    let play = Button::new("chat-clip-play", if t.playing { "Pause" } else { "Play" })
        .sm()
        .icon(if t.playing {
            IconName::Pause.path()
        } else {
            IconName::Play.path()
        })
        .pressed(t.playing)
        .disabled(!t.playable())
        .on_click(cx.listener(|this, _, _, cx| this.toggle_clip_play(cx)));
    let controls = div()
        .flex()
        .items_center()
        .gap(space::S3)
        .child(play)
        .child(div().flex_1())
        .child(
            div()
                .font_family(FONT_MONO)
                .text_size(text::SM)
                .text_color(p.text_muted)
                .child(SharedString::from(format!(
                    "{} / {}",
                    player_time(t.current),
                    player_time(t.duration)
                ))),
        );
    let open = shown.map(|&(fid, _)| {
        Button::new("chat-clip-open", "Open in Timeline")
            .ghost()
            .sm()
            .icon(IconName::ArrowRight.path())
            .icon_after()
            .on_click(move |_, _, cx| cx.defer(move |cx| crate::window::open_frame(fid, cx)))
    });
    let note = c.media_error.as_ref().map(|_| {
        inline(
            Tone::Danger,
            Some(
                icon(IconName::TriangleAlert.path(), crate::theme::size::ICON_SM)
                    .into_any_element(),
            ),
            vec!["This clip's audio file couldn't be played.".into_any_element()],
            p,
        )
    });
    let strip = match (&c.strip.frames, frames.len()) {
        (Some(Err(_)), _) => "frames didn't load".to_string(),
        (_, 0) => "no screen capture".to_string(),
        (_, 1) => "1 frame".to_string(),
        (_, n) => format!("{n} frames"),
    };
    base.children(shown.map(|(fid, _)| screen(c.strip.previews.get(fid), &chat.images, p)))
        .child(track(head, p))
        .child(controls)
        .children(note)
        .children(open)
        .child(foot(format!("{kind} · {strip}"), p))
}

#[cfg(test)]
mod tests {
    use super::index_at;

    #[test]
    fn the_strip_shows_the_newest_frame_captured_by_the_audio_clock() {
        let f = [(1, 1000), (2, 2000), (3, 3000)];
        assert_eq!(index_at(&f, 500), 0, "before the first frame: the first");
        assert_eq!(index_at(&f, 1999), 0);
        assert_eq!(index_at(&f, 2000), 1);
        assert_eq!(index_at(&f, 9000), 2);
        assert_eq!(index_at(&[], 9000), 0);
    }
}
