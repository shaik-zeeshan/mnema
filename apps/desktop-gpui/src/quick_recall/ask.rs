//! The ask slot (`AskPane.svelte`): one turn per question over `ask_ai_start`.
//! An edit cancels the turn in flight and asks again after the 750 ms pause; a
//! Tab-forced ask waits for ↵. Rendering only: the backend owns the `TurnView`
//! and streams `ask_ai_update` ops, applied exactly-next (`chat::model::receive`).

use app_core::ask_ai::AskAiUpdateEvent;
use gpui_kit::{Context, Task};

use super::model::{cites, Cite, Ranked};
use super::rules::{self, AskNotice};
use super::tokens::{self, Forced};
use super::{AskView, QuickRecall, ASK_PAUSE, COPIED_ANSWER};
use crate::chat::model::{receive, ChatTurn, Receive};

const TITLE_MAX: usize = 80;

#[derive(Default)]
pub struct AskState {
    pub turn: Option<ChatTurn>,
    pub stopped_early: bool,
    /// The live thread (the update guard; None once stopped or cancelled).
    pub live_id: Option<String>,
    /// The persisted thread "Continue in Chat" opens (survives a Stop).
    pub thread_id: Option<String>,
    /// The question the current turn answers.
    pub asked: String,
    pub seen: bool,
    pub handoff: bool,
    /// 0 = the answer, n = evidence row n.
    pub sel: usize,
    /// Arrowed into the list: ←/→ walk citations instead of the caret.
    pub nav: bool,
    /// The lit citation (1-based, 0 = none).
    pub cite: usize,
    pub copied: bool,
    /// The answer's natural height (measured as it lays out): past the clip
    /// it points to Chat.
    pub prose_h: std::rc::Rc<std::cell::Cell<f32>>,
    pause: Option<Task<()>>,
    sync_key: Option<(String, bool, bool, String)>,
}

impl AskState {
    pub fn live(&self) -> bool {
        self.live_id.is_some() && self.turn.as_ref().is_some_and(ChatTurn::live)
    }

    /// Something the reset must not wipe: an unseen or running answer, or a hand-off.
    pub fn pending(&self) -> bool {
        self.handoff || (self.live_id.is_some() && (self.live() || !self.seen))
    }

    pub fn sources(&self) -> Vec<Cite> {
        self.turn
            .as_ref()
            .map(|t| cites(&t.view.sources))
            .unwrap_or_default()
    }

    pub fn phase(&self) -> Option<&str> {
        self.turn.as_ref().map(|t| t.view.phase.as_str())
    }
}

impl QuickRecall {
    /// The scoped question (the free text with each pill's phrase put back).
    pub fn question_text(&self) -> String {
        tokens::scoped_question(&self.parse.residual, &self.effective_pills())
    }

    pub fn ask_notice(&self) -> Option<AskNotice> {
        if self.ask_view() != Some(AskView::Ask) {
            return None;
        }
        let a = self.availability.as_ref()?;
        let reason = if a.available {
            None
        } else {
            Some(a.reason.clone().unwrap_or_default())
        };
        rules::ask_notice(
            reason.as_deref(),
            self.settings.as_ref(),
            self.online,
            &self.engine(),
        )
    }

    fn can_ask(&self) -> bool {
        self.ask_view() == Some(AskView::Ask) && self.ask_available() && self.ask_notice().is_none()
    }

    /// The question is typed but not yet the one being answered.
    pub fn ask_waiting(&self) -> bool {
        let q = self.question_text();
        self.can_ask() && !q.trim().is_empty() && !tokens::same_question(&q, &self.ask.asked)
    }

    /// The moments the answer didn't cite, listed under the cited rows.
    pub fn more_moments(&self) -> Vec<Ranked> {
        let sources = self.ask.sources();
        self.visible_results()
            .iter()
            .filter(|r| !sources.iter().any(|s| s.cites(r)))
            .cloned()
            .collect()
    }

    /// The results belong under the gate and the notices.
    pub fn moments_below(&self) -> bool {
        self.ask_view() == Some(AskView::Gate) || self.ask_notice().is_some()
    }

    /// Answer once the question holds still; an edit cancels the turn in flight.
    pub(super) fn ask_sync(&mut self, cx: &mut Context<Self>) {
        let q = self.question_text().trim().to_string();
        let forced = self.forced == Some(Forced::Ask);
        let key = (q.clone(), self.can_ask(), forced, self.ask.asked.clone());
        if self.ask.sync_key.as_ref() == Some(&key) {
            return;
        }
        self.ask.sync_key = Some(key);
        self.ask.pause = None;
        if !self.can_ask() || q.is_empty() || tokens::same_question(&q, &self.ask.asked) {
            return;
        }
        // An edit drops the old thread even mid-answer: it answers a question no longer asked.
        self.ask_cancel(true, cx);
        self.ask.turn = None;
        if forced {
            return;
        }
        self.ask.pause = Some(self.after(ASK_PAUSE, cx, move |qr, cx| qr.ask_start(q, cx)));
    }

    /// ↵ (or the gate, "ask instead"): send the question now.
    pub fn ask_submit(&mut self, cx: &mut Context<Self>) {
        if !self.ask_waiting() {
            return;
        }
        self.ask.pause = None;
        let q = self.question_text().trim().to_string();
        self.ask_start(q, cx);
    }

    /// An explicit ask (the gate, "ask instead"): Ask mode, top of Recent, send.
    pub fn ask_now(&mut self, cx: &mut Context<Self>) {
        self.forced = Some(Forced::Ask);
        self.remember_query(true, cx);
        self.refresh(cx);
        self.ask_submit(cx);
    }

    pub(super) fn ask_start(&mut self, q: String, cx: &mut Context<Self>) {
        self.ask.pause = None;
        self.ask.asked = q.clone();
        self.ask_cancel(false, cx);
        let id = uuid::Uuid::new_v4().to_string();
        self.ask.live_id = Some(id.clone());
        self.ask.thread_id = Some(id.clone());
        self.ask.seen = false;
        self.ask.sel = 0;
        self.ask.nav = false;
        self.ask.cite = 0;
        self.ask.stopped_early = false;
        let mut turn = ChatTurn::new(0, q.clone(), "thinking");
        turn.started_at_ms = Some(crate::ui::clock::now_ms(cx));
        self.ask.turn = Some(turn);
        let title: String = {
            let t = q.split_whitespace().collect::<Vec<_>>().join(" ");
            if t.chars().count() > TITLE_MAX {
                format!("{}…", t.chars().take(TITLE_MAX - 1).collect::<String>())
            } else {
                t
            }
        };
        self.fetch(
            cx,
            move |b| b.ask_start(&id, &q, &title).err().map(|e| (id, e)),
            |qr, failed, _| {
                if let Some((id, e)) = failed {
                    if qr.ask.live_id.as_deref() == Some(id.as_str()) {
                        if let Some(t) = qr.ask.turn.as_mut() {
                            t.view.phase = "error".into();
                            t.view.error_message = Some(e);
                        }
                    }
                }
            },
        );
    }

    /// Only a running turn has anything to stop; a settled one may be Chat's thread now.
    fn ask_cancel(&mut self, discard: bool, cx: &mut Context<Self>) {
        let Some(id) = self.ask.live_id.take() else {
            return;
        };
        let running = self.ask.turn.as_ref().is_some_and(ChatTurn::live);
        if running {
            self.fetch(cx, move |b| b.ask_cancel(&id, discard), |_, _, _| {});
        }
    }

    /// Esc while streaming: the partial stays, still continuable in Chat.
    pub fn ask_stop(&mut self, cx: &mut Context<Self>) {
        if !self.ask.live() {
            return;
        }
        self.ask_cancel(false, cx);
        if let Some(t) = self.ask.turn.as_mut() {
            if t.view.blocks.is_empty() {
                self.ask.thread_id = None;
            }
            t.view.phase = "done".into();
            t.view.live_activity = None;
        }
        self.ask.stopped_early = true;
    }

    /// The emptied bar unmounts the pane, which cancels its turn.
    pub(super) fn ask_reset(&mut self, cx: &mut Context<Self>) {
        self.ask_cancel(false, cx);
        let handoff = self.ask.handoff;
        self.ask = AskState {
            handoff,
            ..Default::default()
        };
    }

    /// `ask_ai_update`: exactly-next applies; a gap re-snapshots.
    pub fn on_ask_update(&mut self, ev: AskAiUpdateEvent, cx: &mut Context<Self>) {
        if self.ask.live_id.as_deref() != Some(ev.conversation_id.as_str()) {
            return;
        }
        let now = crate::ui::clock::now_ms(cx);
        let Some(turn) = self
            .ask
            .turn
            .as_mut()
            .filter(|t| t.view.turn_index == ev.turn_index)
        else {
            return;
        };
        if receive(turn, ev.version, &ev.update, now) == Receive::Gap {
            let id = ev.conversation_id.clone();
            self.fetch(
                cx,
                move |b| b.ask_snapshot(&id),
                move |qr, snap, _| {
                    if let (Some(snap), Some(t)) = (snap, qr.ask.turn.as_mut()) {
                        if qr.ask.live_id.as_deref() == Some(snap.conversation_id.as_str())
                            && snap.view.turn_index == t.view.turn_index
                        {
                            t.adopt(&snap);
                        }
                    }
                },
            );
        }
        self.refresh(cx);
    }

    /// Seen once a settled answer was on screen while the panel had focus.
    pub(super) fn mark_seen(&mut self) {
        if self.focused && matches!(self.ask.phase(), Some("done" | "error")) {
            self.ask.seen = true;
        }
    }

    /// ↵ on a finished answer: Chat takes the thread on.
    pub fn continue_in_chat(&mut self, cx: &mut Context<Self>) {
        if self.ask.phase() != Some("done") {
            return;
        }
        let Some(id) = self.ask.thread_id.clone() else {
            return;
        };
        super::panel::open_in_chat(id, cx);
        self.dismiss(cx);
    }

    pub fn copy_answer(&mut self, cx: &mut Context<Self>) {
        let text = self
            .ask
            .turn
            .as_ref()
            .map(ChatTurn::plain_text)
            .unwrap_or_default();
        if text.is_empty() {
            return;
        }
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
        self.ask.copied = true;
        self.timers.copied = Some(self.after(COPIED_ANSWER, cx, |qr, _| qr.ask.copied = false));
    }

    /// The cited row as an open target; the rest of the evidence is search hits.
    pub fn open_cite(&mut self, i: usize, page: bool, cx: &mut Context<Self>) {
        let Some(s) = self.ask.sources().get(i).cloned() else {
            return;
        };
        if page {
            if let Some(id) = s.frame_id {
                self.open_page(id, cx);
            }
            return;
        }
        let target = super::panel::Target::of_cite(&s);
        self.ask.handoff = true;
        let q = self.ask.asked.clone();
        self.hand_off(target, q, cx);
    }

    /// Evidence rows 1…n for Quick Look: the citations, then the rest.
    pub fn looks(&self) -> Vec<Option<Ranked>> {
        let ranked = &self.results.ranked;
        self.ask
            .sources()
            .iter()
            .map(|s| super::look::source_result(s, ranked))
            .chain(self.more_moments().into_iter().map(Some))
            .collect()
    }

    /// The keys the foot shows while the ask slot is up.
    pub fn ask_foot_keys(&self) -> Vec<(&'static str, &'static str)> {
        let a = &self.ask;
        if self.ask_view() == Some(AskView::Gate) {
            return if a.nav {
                vec![("↵", "open")]
            } else {
                vec![("↵", "ask"), ("↓", "moments")]
            };
        }
        if self.ask_notice().is_some() {
            return vec![("↓", "moments"), ("⇥", "just search")];
        }
        if a.sel > 0 {
            return vec![("↵", "open"), ("space", "look")];
        }
        if self.ask_waiting() && !a.live() {
            return vec![("↵", "ask"), ("esc", "clear")];
        }
        if a.live() {
            return vec![("esc", "stop")];
        }
        if a.phase() == Some("error") {
            return vec![("↵", "retry"), ("esc", "clear")];
        }
        if a.cite > 0 {
            return vec![
                ("←→", "source"),
                ("space", "look"),
                ("↵", "continue in chat"),
            ];
        }
        let nav = if a.nav && !a.sources().is_empty() {
            ("←→", "sources")
        } else {
            ("↓", "evidence")
        };
        vec![
            ("↵", "continue in chat"),
            nav,
            ("⌘C", if a.copied { "copied" } else { "copy" }),
        ]
    }

    /// A snapshot as the live turn (the gap path's adopt; goldens show a stream with it).
    pub fn adopt_snapshot(&mut self, snap: capture_types::TurnSnapshot, cx: &mut Context<Self>) {
        if let Some(t) = self.ask.turn.as_mut() {
            t.adopt(&snap);
        }
        self.refresh(cx);
    }
}
