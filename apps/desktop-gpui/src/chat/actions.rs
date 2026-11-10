//! What the user does in Chat: send / stop / regenerate, the model pin, pin /
//! rename / delete, and the two toasts. Backend calls go through `data.rs` on
//! tokio; the entity only changes view state.

use capture_types::ConversationSummary;
use gpui_kit::{Context, PromptLevel, Window};

use super::{core, data, format, Chat, ChatTurn, History, Pane, Rename, TITLE_MAX};
use super::{DELETED_TOAST_TTL, PIN_FAIL_TOAST_TTL};
use crate::bridge::tokio::Tokio;
use crate::ui::clock;
use crate::ui::toast::Toast;

fn title_from(question: &str) -> String {
    let t = question.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > TITLE_MAX {
        format!("{}…", t.chars().take(TITLE_MAX - 1).collect::<String>())
    } else {
        t
    }
}

impl Chat {
    /// A host chip (`openCapturedUrl`): the backend opens the page; a frame with
    /// no openable page, or a failure, says so.
    pub(super) fn open_captured_url(&mut self, frame_id: Option<i64>, cx: &mut Context<Self>) {
        let (Some(core), Some(id)) = (core(cx), frame_id) else {
            return;
        };
        let task = Tokio::spawn(cx, async move { data::open_captured_url(&core, id).await });
        cx.spawn(async move |this, cx| {
            let message = match task.await {
                Ok(Ok(true)) => return,
                Ok(Ok(false)) => "No openable page for this result.".to_string(),
                Ok(Err(e)) => format!("Couldn't open page: {e}"),
                Err(e) => format!("Couldn't open page: {e}"),
            };
            let _ = this.update(cx, |this, cx| {
                this.toasts.update(cx, |t, cx| {
                    t.push(crate::ui::toast::Toast::danger(message), cx)
                });
            });
        })
        .detach();
    }

    /// The picker's star: save the starred list, then re-read the engine facts.
    pub(super) fn toggle_favorite(&mut self, engine: (String, String), cx: &mut Context<Self>) {
        let Some(core) = core(cx) else { return };
        let task = Tokio::spawn(cx, async move {
            let worker = core.clone();
            core.spawn_blocking(move || data::toggle_favorite(&worker, engine))
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(Err(e)) = result {
                    let toast = crate::ui::toast::Toast::danger(format!("Couldn't save: {e}"));
                    this.toasts.update(cx, |t, cx| t.push(toast, cx));
                }
                this.reload(cx);
            });
        })
        .detach();
    }

    /// Send the composer (or re-send `resend` for Regenerate / Retry).
    pub fn send(&mut self, resend: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let question = resend
            .clone()
            .unwrap_or_else(|| self.composer.read(cx).value().to_string())
            .trim()
            .to_string();
        if question.is_empty() || self.streaming || self.available() != Some(true) {
            return;
        }
        let id = self
            .active
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();
        // A trailing local-only error has no backend row (CH-04).
        if self.turns.last().is_some_and(|t| t.local_only) {
            self.turns.pop();
        }
        let first = self.turns.is_empty();
        if first && self.title.is_empty() {
            self.title = title_from(&question);
        }
        if resend.is_none() {
            self.composer
                .update(cx, |s, cx| s.set_value("", window, cx));
        }
        // Settle any prior turn still flagged working (a displaced turn).
        for t in &mut self.turns {
            if t.live() {
                t.view.live_activity = None;
                t.view.phase = "done".into();
            }
        }
        let now = clock::now(cx);
        let now_ms = clock::now_ms(cx);
        let ix = self.turns.len() as i64;
        let mut turn = ChatTurn::new(ix, question.clone(), "thinking");
        turn.started_at_ms = Some(now_ms);
        turn.at_ms = Some(now_ms);
        turn.scope_label = Some(self.scope.label(now.date()));
        self.turns.push(turn);
        self.streaming = true;
        self.pane = Pane::Thread;
        self.scroll.scroll_to_bottom();
        cx.notify();
        let Some(core) = core(cx) else { return };
        let req = data::SendRequest {
            conversation_id: id.clone(),
            question: question.clone(),
            title: first.then(|| self.title.clone()),
            first,
            scope: self.scope.to_wire(now.offset()),
        };
        let pin = self.pin.clone();
        let task = Tokio::spawn(cx, async move {
            if first && pin.is_some() {
                data::set_engine(&core, &req.conversation_id, pin).await;
            }
            data::send(&core, req).await
        });
        cx.spawn(async move |this, cx| {
            let Ok(Err(e)) = task.await else { return };
            let _ = this.update(cx, |this, cx| {
                if this.active.as_deref() != Some(id.as_str()) {
                    return;
                }
                this.streaming = false;
                if let Some(t) = this.turns.iter_mut().find(|t| t.view.turn_index == ix) {
                    t.view.phase = "error".into();
                    t.view.error_message = Some(e);
                    t.local_only = true;
                }
                this.restore_question(question, cx);
                cx.notify();
            });
        })
        .detach();
        if first {
            self.created_at_ms.get_or_insert(now_ms);
        }
    }

    /// The backend always emits a terminal `done` on cancel, which settles the UI.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        let (Some(id), true) = (self.active.clone(), self.streaming) else {
            return;
        };
        let Some(core) = core(cx) else { return };
        Tokio::spawn(cx, async move { data::cancel(&core, &id).await }).detach();
    }

    /// Regenerate / Retry the trailing settled turn in place (CH4): delete its
    /// row, drop it, re-send. A failed delete keeps the old answer.
    pub fn regenerate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(last) = self.turns.last().cloned() else {
            return;
        };
        if self.streaming || self.available() != Some(true) || last.live() {
            return;
        }
        let question = last.view.question.clone();
        if last.local_only {
            self.turns.pop();
            self.send(Some(question), window, cx);
            return;
        }
        let (Some(id), Some(core)) = (self.active.clone(), core(cx)) else {
            return;
        };
        self.streaming = true; // blocks send() while the delete runs
        let ix = last.view.turn_index;
        let task = Tokio::spawn(cx, {
            let id = id.clone();
            async move { data::delete_last_turn(&core, &id, ix).await }
        });
        cx.spawn_in(window, async move |this, cx| {
            let ok = matches!(task.await, Ok(Ok(())));
            let _ = this.update_in(cx, |this, window, cx| {
                this.streaming = false;
                if !ok {
                    log::warn!("chat: regenerate couldn't delete the last turn");
                    cx.notify();
                    return;
                }
                if this.active.as_deref() != Some(id.as_str()) {
                    return;
                }
                this.turns.pop();
                this.send(Some(question), window, cx);
            });
        })
        .detach();
    }

    /// The model picker's pick; `None` clears the pin back to Default.
    pub fn select_model(&mut self, engine: Option<(String, String)>, cx: &mut Context<Self>) {
        self.pin = engine.clone();
        cx.notify();
        // No row before the first turn: send() persists the pin with it.
        let (Some(id), false) = (self.active.clone(), self.turns.is_empty()) else {
            return;
        };
        let Some(core) = core(cx) else { return };
        Tokio::spawn(
            cx,
            async move { data::set_engine(&core, &id, engine).await },
        )
        .detach();
    }

    /// The picker opened: list the pool once (a reopen re-lists).
    pub fn load_models(&mut self, cx: &mut Context<Self>) {
        let Some(core) = core(cx) else { return };
        self.pool_loading = true;
        cx.notify();
        let task = Tokio::spawn(cx, async move { data::list_models(&core).await });
        cx.spawn(async move |this, cx| {
            let (pool, failures) = task.await.unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.pool = pool;
                this.pool_failures = failures;
                this.pool_loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn patch_row(&mut self, id: &str, f: impl Fn(&mut ConversationSummary)) {
        if let History::Rows(rows) = &mut self.data.history {
            rows.iter_mut()
                .filter(|r| r.conversation_id == id)
                .for_each(f);
        }
    }

    pub fn row(&self, id: &str) -> Option<&ConversationSummary> {
        match &self.data.history {
            History::Rows(rows) => rows.iter().find(|r| r.conversation_id == id),
            _ => None,
        }
    }

    pub fn head_pinned(&self) -> bool {
        self.active
            .as_deref()
            .and_then(|id| self.row(id))
            .map(|r| r.pinned)
            .or(self.pinned)
            .unwrap_or(false)
    }

    /// Pin / unpin, optimistically; a failed persist reverts and toasts Retry.
    pub fn toggle_pin(&mut self, id: String, cx: &mut Context<Self>) {
        let now = if self.active.as_deref() == Some(id.as_str()) {
            self.head_pinned()
        } else {
            self.row(&id).is_some_and(|r| r.pinned)
        };
        let pinned = !now;
        self.patch_row(&id, |r| r.pinned = pinned);
        if self.active.as_deref() == Some(id.as_str()) {
            self.pinned = Some(pinned);
        }
        cx.notify();
        let Some(core) = core(cx) else { return };
        let task = Tokio::spawn(cx, {
            let id = id.clone();
            async move { data::set_pinned(&core, &id, pinned).await }
        });
        cx.spawn(async move |this, cx| {
            if matches!(task.await, Ok(Ok(()))) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                this.patch_row(&id, |r| r.pinned = !pinned);
                if this.active.as_deref() == Some(id.as_str()) {
                    this.pinned = Some(!pinned);
                }
                this.pin_failed(id, pinned, cx);
            });
        })
        .detach();
    }

    /// The danger toast with Retry, for [`PIN_FAIL_TOAST_TTL`].
    pub fn pin_failed(&mut self, id: String, pinned: bool, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        let verb = if pinned { "pin" } else { "unpin" };
        let mut toast = Toast::danger(format!("Couldn’t {verb} the chat")).action(
            "Retry",
            true,
            move |_, cx| {
                let id = id.clone();
                let _ = this.update(cx, |this, cx| this.toggle_pin(id, cx));
            },
        );
        toast.ttl = PIN_FAIL_TOAST_TTL;
        self.toasts.update(cx, |t, cx| t.push(toast, cx));
    }

    /// The ok toast after a delete, for [`DELETED_TOAST_TTL`].
    pub fn show_deleted(&mut self, cx: &mut Context<Self>) {
        let mut toast = Toast::ok("Chat deleted");
        toast.ttl = DELETED_TOAST_TTL;
        self.toasts.update(cx, |t, cx| t.push(toast, cx));
    }

    /// Delete after a native confirm; the open chat becomes a fresh one.
    pub fn delete(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .row(&id)
            .map(|r| {
                if r.title.is_empty() {
                    r.preview.clone()
                } else {
                    r.title.clone()
                }
            })
            .filter(|s| !s.is_empty())
            .or_else(|| (self.active.as_deref() == Some(id.as_str())).then(|| self.display_title()))
            .unwrap_or_else(|| "this chat".into());
        let answer = window.prompt(
            PromptLevel::Warning,
            "Delete chat",
            Some(&format!("Delete “{name}”? This can’t be undone.")),
            &["Delete", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let Some(core) = this.update(cx, |_, cx| core(cx)).ok().flatten() else {
                return;
            };
            let task = this
                .update(cx, |_, cx| {
                    Tokio::spawn(cx, {
                        let id = id.clone();
                        async move { data::delete(&core, &id).await }
                    })
                })
                .ok();
            let Some(task) = task else { return };
            if !matches!(task.await, Ok(Ok(()))) {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| {
                if let History::Rows(rows) = &mut this.data.history {
                    rows.retain(|r| r.conversation_id != id);
                }
                if this.active.as_deref() == Some(id.as_str()) {
                    this.new_chat(None, false, window, cx);
                }
                this.show_deleted(cx);
            });
        })
        .detach();
    }

    pub fn display_title(&self) -> String {
        let row = self.active.as_deref().and_then(|id| self.row(id));
        match row.map(|r| r.title.clone()).filter(|t| !t.is_empty()) {
            Some(t) => t,
            None if !self.title.is_empty() => self.title.clone(),
            None => "New chat".into(),
        }
    }

    pub fn start_rename(&mut self, target: Rename, window: &mut Window, cx: &mut Context<Self>) {
        let current = match &target {
            Rename::Header => self.display_title(),
            Rename::Row(id) => self
                .row(id)
                .map(|r| {
                    if r.title.is_empty() {
                        r.preview.clone()
                    } else {
                        r.title.clone()
                    }
                })
                .unwrap_or_default(),
        };
        self.renaming = Some(target);
        self.rename_input.update(cx, |s, cx| {
            s.set_value(current, window, cx);
            s.select_all(window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    pub fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        self.renaming = None;
        cx.notify();
    }

    /// Enter / blur saves; empty or unchanged cancels.
    pub fn commit_rename(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.renaming.take() else {
            return;
        };
        let title = self.rename_input.read(cx).value().trim().to_string();
        let id = match &target {
            Rename::Header => self.active.clone(),
            Rename::Row(id) => Some(id.clone()),
        };
        let current = match &target {
            Rename::Header => self.display_title(),
            Rename::Row(id) => self.row(id).map(|r| r.title.clone()).unwrap_or_default(),
        };
        cx.notify();
        let Some(id) = id.filter(|_| !title.is_empty() && title != current) else {
            return;
        };
        self.patch_row(&id, |r| r.title = title.clone());
        if self.active.as_deref() == Some(id.as_str()) {
            self.title = title.clone();
        }
        let Some(core) = core(cx) else { return };
        Tokio::spawn(cx, async move {
            if let Err(e) = data::set_title(&core, &id, &title).await {
                log::warn!("chat: rename failed: {e}");
            }
        })
        .detach();
    }

    /// A "Try asking" chip: fill the composer and set its scope preset; never sends.
    pub fn pick_suggestion(
        &mut self,
        text: &str,
        preset: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let today = clock::now(cx).date();
        self.scope = super::Scope::preset(preset, self.scope.about_you, today);
        self.composer.update(cx, |s, cx| {
            s.set_value(text.to_string(), window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    /// The steps line for a turn ("3 steps · 2.1s · searched this week").
    pub fn steps(&self, t: &ChatTurn) -> Option<String> {
        let line = format::steps_line(t.view.tool_activities.len(), t.elapsed_ms)?;
        Some(match &t.scope_label {
            Some(s) => format!("{line} · searched {s}"),
            None => line,
        })
    }
}
