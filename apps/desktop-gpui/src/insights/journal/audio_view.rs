//! The receipt's audio (`ReceiptViewer.svelte`, ADR 0049): selecting a spoken
//! turn relives it at 1× through the Timeline drawer's `Transport` (the same
//! AVAudioPlayer factory, `timeline::audio::Deps`) while the frame strip
//! follows the audio clock; a clip that ends hands on to the next segment's
//! first turn. 8× / 16× stay the silent frame timelapse. An activity with
//! audio and no frames becomes a bounded audio player.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, Context, Div, Role, SharedString};

use super::audio::{self, Span, TurnView};
use crate::insights::stats;
use crate::insights::Insights;
use crate::theme::{radius, space, text, Palette, FONT_MONO};
use crate::timeline::audio::Seek;
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::{clock, nid, Button, Tone};

/// The receipt's audio state (beside its frame strip).
#[derive(Default)]
pub struct ReceiptAudio {
    /// `None` while the span hydrates.
    pub span: Option<Result<Span, String>>,
    pub selected: Option<(i64, i64)>,
    /// The segment whose audio the transport holds, and its start (ms).
    pub clip: Option<(i64, i64)>,
    pub transport: crate::timeline::audio::transport::Transport,
    /// The clip's file couldn't open or play, or its segment is gone.
    pub media_error: Option<String>,
    gen: u64,
}

impl ReceiptAudio {
    pub fn turns(&self) -> &[TurnView] {
        match &self.span {
            Some(Ok(s)) => &s.turns,
            _ => &[],
        }
    }
    pub fn pending(&self) -> bool {
        self.span.is_none()
    }
    pub fn transcribing(&self) -> Option<(usize, usize)> {
        match &self.span {
            Some(Ok(s)) if s.transcribing.0 < s.transcribing.1 => Some(s.transcribing),
            _ => None,
        }
    }
    pub fn selected_turn(&self) -> Option<&TurnView> {
        self.turns().iter().find(|t| Some(t.key) == self.selected)
    }
    /// The audio clock (wall-clock ms) while a clip is loaded.
    pub fn head(&self) -> Option<i64> {
        self.clip
            .map(|(_, start)| start + self.transport.current_ms())
    }
    /// The lit turn: while a clip plays only its segment's turns can light up.
    pub fn active_turn(&self) -> Option<(i64, i64)> {
        let head = self.head().filter(|_| self.transport.engaged());
        match self.clip {
            Some((seg, _)) if head.is_some() => {
                audio::active_key_at(self.turns().iter().filter(|t| t.segment == seg), head)
            }
            _ => self.selected,
        }
    }
}

/// The activity's cited audio refs: `(segment id, headline)`.
fn cited(this: &Insights, id: i64) -> Vec<(i64, bool)> {
    this.journal
        .iter()
        .flat_map(|j| j.activities.iter())
        .find(|a| a.id == id)
        .map(|a| {
            a.evidence
                .iter()
                .filter(|e| e.subject_type == "audio_segment")
                .map(|e| (e.subject_id, e.is_headline))
                .collect()
        })
        .unwrap_or_default()
}

impl Insights {
    /// `loadAudio`: hydrate the span's turns; audio makes 1× the default.
    pub(super) fn load_receipt_audio(&mut self, id: i64, span: (i64, i64), cx: &mut Context<Self>) {
        let refs = cited(self, id);
        let au = self.audio.backend.clone();
        self.write(
            cx,
            move |b| audio::load_span(b, &*au, span, &refs),
            move |this, r, _| {
                let Some(rc) = this.receipt.as_mut().filter(|rc| rc.activity_id == id) else {
                    return;
                };
                if let Ok(s) = &r {
                    rc.speed = if s.turns.is_empty() { 8 } else { 1 };
                    rc.audio.selected = audio::default_key(&s.turns);
                }
                rc.audio.span = Some(r);
            },
        );
    }

    /// Selection = play: clicking a turn relives it at 1×.
    pub fn select_turn(&mut self, key: (i64, i64), cx: &mut Context<Self>) {
        if let Some(rc) = self.receipt.as_mut() {
            rc.audio.selected = Some(key);
            rc.speed = 1;
        }
        self.play_clip(key, None, true, cx);
    }

    /// `playClip`: the turn's segment at its offset (or `seek_ms` inside it).
    pub fn play_clip(
        &mut self,
        key: (i64, i64),
        seek_ms: Option<i64>,
        autoplay: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(rc) = self.receipt.as_mut() else {
            return;
        };
        let Some(turn) = rc.audio.turns().iter().find(|t| t.key == key).cloned() else {
            return;
        };
        rc.playing = false; // the audio clocks from here
        let at = seek_ms.filter(|ms| *ms >= turn.seg_start && *ms <= turn.end);
        let offset = at.unwrap_or(turn.seg_start) - turn.seg_start;
        let a = &mut rc.audio;
        a.media_error = None;
        if a.clip.map(|c| c.0) == Some(turn.segment) && a.transport.playable() {
            a.transport.seek(Seek::ToMs(offset));
            if autoplay && !a.transport.playing && !a.transport.toggle() {
                a.media_error = Some("Playback didn't start.".into());
            }
            cx.notify();
            return;
        }
        if a.transport.playing {
            a.transport.toggle();
        }
        a.gen += 1;
        let gen = a.gen;
        let deps = self.audio.clone();
        let task = cx.background_spawn(async move {
            match deps.backend.segment(turn.segment) {
                Ok(Some(s)) => (deps.open_player)(&s.file_path).map(Some),
                Ok(None) => Ok(None),
                Err(e) => Err(e),
            }
        });
        cx.spawn(async move |this, cx| {
            let opened = task.await;
            let _ = this.update(cx, |this, cx| {
                let Some(a) = this
                    .receipt
                    .as_mut()
                    .map(|rc| &mut rc.audio)
                    .filter(|a| a.gen == gen)
                else {
                    return;
                };
                match opened {
                    Ok(Some(p)) => {
                        a.transport = Default::default();
                        a.transport.set_player(p);
                        a.clip = Some((turn.segment, turn.seg_start));
                        a.transport.seek(Seek::ToMs(offset));
                        if autoplay && !a.transport.toggle() {
                            a.media_error = Some("Playback didn't start.".into());
                        }
                    }
                    Ok(None) => {
                        a.media_error = Some("This clip's audio is no longer stored.".into())
                    }
                    Err(e) => {
                        log::warn!("insights: receipt audio failed to open");
                        a.media_error = Some(e);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Play / Pause routes to whichever clock owns the receipt.
    pub fn receipt_play(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let Some(rc) = self.receipt.as_mut() else {
            return;
        };
        let audible = rc.speed == 1 && !rc.audio.turns().is_empty() || rc.len() == 0;
        if !audible {
            rc.toggle(now);
            return cx.notify();
        }
        if rc.audio.clip.is_some() && rc.audio.transport.playable() {
            if !rc.audio.transport.toggle() {
                rc.audio.media_error = Some("Playback didn't start.".into());
            }
            return cx.notify();
        }
        if let Some(key) = rc.audio.selected {
            self.play_clip(key, None, true, cx);
        }
    }

    /// A speed pick: 1× plays the selected turn, a silent speed pauses the audio.
    pub fn receipt_speed(&mut self, speed: u32, cx: &mut Context<Self>) {
        let Some(rc) = self.receipt.as_mut() else {
            return;
        };
        rc.speed = speed;
        if speed != 1 {
            if rc.audio.transport.playing {
                rc.audio.transport.toggle();
            }
        } else if let Some(key) = rc.audio.selected {
            return self.play_clip(key, None, true, cx);
        }
        cx.notify();
    }

    /// One poll while a clip plays: poll the player, move the strip to the
    /// audio clock; at the clip's end, the next segment's first turn.
    pub(super) fn tick_receipt_audio(&mut self, cx: &mut Context<Self>) {
        let Some(rc) = self.receipt.as_mut() else {
            return;
        };
        if !rc.audio.transport.playing {
            return;
        }
        rc.audio.transport.tick();
        if let (Some(head), Some(Ok(frames))) = (rc.audio.head(), &rc.frames) {
            rc.index = crate::chat::clip::index_at(frames, head);
        }
        if rc.audio.transport.playing {
            return;
        }
        let ended = rc.audio.clip.map(|c| c.0);
        let next = ended.and_then(|e| audio::next_clip(rc.audio.turns(), e).map(|t| t.key));
        if let Some(key) = next {
            rc.audio.selected = Some(key);
            self.play_clip(key, None, true, cx);
        }
    }
}

/// `in-turns`: one click-first row per spoken turn, then Open in Timeline.
pub fn turn_list(
    this: &Insights,
    open: AnyElement,
    p: &Palette,
    cx: &mut Context<Insights>,
) -> Div {
    let off = clock::now(cx).offset();
    let Some(rc) = this.receipt.as_ref() else {
        return div();
    };
    let a = &rc.audio;
    let active = a.active_turn();
    div().child(
        div()
            .id("insights-receipt-turns")
            .flex()
            .flex_col()
            .gap(px(2.))
            .max_h(px(320.))
            .overflow_y_scroll()
            .children(a.turns().iter().enumerate().map(|(i, t)| {
                let key = t.key;
                let on = active == Some(key);
                div()
                    .id(nid("insights-turn", i as u64))
                    .test_support()
                    .role(Role::Button)
                    .flex()
                    .gap(space::S2)
                    .py(px(6.))
                    .px(px(8.))
                    .rounded(radius::SM)
                    .cursor_pointer()
                    .text_size(text::BASE)
                    .text_color(p.text)
                    .when(on, |d| d.bg(p.surface_active))
                    .when(!on, |d| d.hover(|s| s.bg(p.surface_hover)))
                    .child(
                        div()
                            .w(px(40.))
                            .flex_none()
                            .font_family(FONT_MONO)
                            .text_size(text::XS)
                            .text_color(p.text_subtle)
                            .child(stats::clock(t.start, off)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(text::SM)
                                    .font_weight(crate::theme::W_SEMIBOLD)
                                    .text_color(audio::color(t.color, p))
                                    .child(t.speaker.clone()),
                            )
                            .child(SharedString::from(if t.text.is_empty() {
                                "—".to_string()
                            } else {
                                t.text.clone()
                            })),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.select_turn(key, cx)))
            }))
            .child(div().mt(px(6.)).flex().child(open)),
    )
}

/// The audio-only box (`.in-audio`): play, who, when, or why not.
pub fn audio_box(this: &Insights, p: &Palette, cx: &mut Context<Insights>) -> Div {
    let off = clock::now(cx).offset();
    let Some(rc) = this.receipt.as_ref() else {
        return div();
    };
    let a = &rc.audio;
    let playing = a.transport.playing;
    let sel = a.selected_turn();
    let play = Button::icon_only(
        "insights-receipt-audio-play",
        if playing {
            IconName::Pause.path()
        } else {
            IconName::Play.path()
        },
        if playing {
            "Pause spoken evidence"
        } else {
            "Play spoken evidence"
        },
    )
    .disabled(sel.is_none())
    .on_click(cx.listener(|this, _, _, cx| this.receipt_play(cx)));
    let label = |s: String| {
        div()
            .font_family(FONT_MONO)
            .text_size(text::SM)
            .text_color(p.text_subtle)
            .child(s)
    };
    let body: Vec<AnyElement> = match sel {
        Some(t) => vec![
            div()
                .flex()
                .items_center()
                .gap(px(7.))
                .text_size(text::MD)
                .font_weight(crate::theme::W_SEMIBOLD)
                .text_color(p.text_strong)
                .child(
                    div()
                        .flex_none()
                        .size(px(8.))
                        .rounded_full()
                        .bg(audio::color(t.color, p)),
                )
                // One line, cut with "…" (the turn list has the full name).
                .child(crate::ui::with_tip(
                    div()
                        .id("insights-receipt-audio-speaker")
                        .min_w_0()
                        .truncate()
                        .child(t.speaker.clone()),
                    t.speaker.clone(),
                ))
                .child(label(format!("via {}", t.source)).flex_none())
                .into_any_element(),
            label(format!(
                "spoken segment · {} · captured as audio",
                stats::clock_range(t.start, t.end, off, "–")
            ))
            .into_any_element(),
        ],
        None if a.pending() => vec![label("Loading spoken evidence…".into()).into_any_element()],
        None => match a.transcribing() {
            Some((d, n)) => {
                vec![
                    label(format!("Transcribing this audio… {d} of {n} segments done"))
                        .into_any_element(),
                ]
            }
            None => vec![label("No readable speech in the cited audio".into()).into_any_element()],
        },
    };
    div().child(
        div()
            .id("insights-receipt-audio")
            .flex()
            .flex_col()
            .items_start()
            .justify_center()
            .gap(space::S2)
            .min_h(px(180.))
            .p(space::S4)
            .rounded(radius::MD)
            .border_1()
            .border_color(p.border)
            .bg(p.bg)
            .child(play)
            .children(body)
            .children(a.media_error.clone().map(|e| {
                crate::ui::notice::inline(
                    Tone::Danger,
                    None,
                    vec![div().min_w_0().child(e).into_any_element()],
                    p,
                )
            })),
    )
}

/// "Nothing to play" when the cited audio held no words.
pub fn nothing(p: &Palette) -> Div {
    empty(
        EmptyLayout::Compact,
        Tone::Neutral,
        IconName::Info.path(),
        "Nothing to play",
        "The cited audio was silent, or transcription found no words.",
        Vec::new(),
        p,
    )
}
