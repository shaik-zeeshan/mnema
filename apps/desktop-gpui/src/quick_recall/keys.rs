//! The bar's keymap (`search-keys.ts`, AskPane's `handleKey`, RecentRows and
//! EmptyStates' capture listeners, the layout's Esc-closes-panel). Keys reach
//! [`QuickRecall::key`] through a keystroke interceptor, ahead of the field's
//! own bindings (the WKWebView's window capture listener); `true` = consumed.
//!
//!   typing edits · → at the end accepts the ghost · Tab = search ⇄ ask ·
//!   ↑/↓ move · ↵ opens · ⌘↵ its page · ⌫/← at the start walks into the pills ·
//!   Esc clears, then closes · Ctrl+F or `/` on an empty field: the last pill's popover.

use gpui_kit::{Context, Keystroke};

use super::tokens::{next_forced, Forced};
use super::{AskView, EmptyState, QuickRecall};

fn plain(k: &Keystroke) -> bool {
    let m = &k.modifiers;
    !m.platform && !m.control && !m.alt && !m.shift
}

/// A key that types (or deletes) text: it closes Quick Look and edits the query.
fn types_text(k: &Keystroke) -> bool {
    (k.key_char.is_some() && !k.modifiers.platform && !k.modifiers.control) || k.key == "backspace"
}

impl QuickRecall {
    pub fn key(&mut self, k: &Keystroke, cx: &mut Context<Self>) -> bool {
        let used = self.key_inner(k, cx);
        if used {
            self.refresh(cx);
        }
        used
    }

    fn key_inner(&mut self, k: &Keystroke, cx: &mut Context<Self>) -> bool {
        let key = k.key.as_str();
        let cmd = k.modifiers.platform || k.modifiers.control;
        // The ask slot first (stop, evidence, citations, Continue in Chat).
        if self.look.is_none()
            && !self.filters.popover
            && self.ask_view().is_some()
            && self.ask_key(k, cx)
        {
            return true;
        }
        if self.filters.popover {
            return self.popover_key(k);
        }
        let moments = self.ask_view().is_none() || self.moments_below() || self.look.is_some();
        if moments {
            if let Some(used) = self.look_key(k, cx) {
                return used;
            }
        }
        if self.show_recent() && self.recent_key(k, cx) {
            return true;
        }
        if self.empty_key(k, cx) {
            return true;
        }
        // Ctrl+F, or `/` on an empty field: the last pill's popover (if any).
        if (k.modifiers.control && key == "f")
            || (key == "/" && plain(k) && self.text.is_empty() && !self.pills.is_empty())
        {
            if let Some(i) = self.pills.len().checked_sub(1) {
                self.open_popover(i, cx);
                return true;
            }
        }
        self.field_key(k, cmd, cx)
    }

    /// The field's own keys (`handleSearchKeydown`).
    fn field_key(&mut self, k: &Keystroke, cmd: bool, cx: &mut Context<Self>) -> bool {
        let key = k.key.as_str();
        let at_start = {
            let i = self.input.read(cx);
            i.cursor() == 0 && i.selected_range().is_empty()
        };
        // A walked-into pill owns ⌫ / ← / → / ↵ / Esc; any other key lets it go.
        if let Some(i) = self.filters.pill {
            match key {
                "backspace" | "delete" => self.remove_pill(Some(i)),
                "left" => self.select_pill(Some(i.saturating_sub(1))),
                "right" => self.select_pill((i + 1 < self.pills.len()).then_some(i + 1)),
                "enter" => self.open_popover(i, cx),
                "escape" if plain(k) => self.select_pill(None),
                _ => {
                    self.select_pill(None);
                    return false;
                }
            }
            return true;
        }
        if key == "tab" && plain(k) {
            self.commit_pills(cx);
            self.toggle_mode(cx);
            return true;
        }
        if key == "right" && plain(k) && self.caret_at_end && self.ghost().is_some() {
            return self.accept_ghost(cx);
        }
        if (key == "backspace" || key == "left")
            && !cmd
            && !k.modifiers.alt
            && at_start
            && !self.pills.is_empty()
        {
            self.select_pill(Some(self.pills.len() - 1));
            return true;
        }
        if key == "escape" && plain(k) {
            if !self.text.is_empty() || !self.pills.is_empty() {
                self.clear_query(cx);
            } else {
                self.dismiss(cx);
            }
            return true;
        }
        match key {
            "down" => self.move_selection(1, cx),
            "up" => self.move_selection(-1, cx),
            "enter" => {
                self.commit_pills(cx);
                match self.results.selected {
                    Some(_) if cmd => self.open_selected_page(cx),
                    Some(i) => self.open_result_at(i, cx),
                    None => {}
                }
            }
            "c" if k.modifiers.platform => {
                let sel = self.input.read(cx).selected_range();
                let Some(r) = self
                    .results
                    .selected
                    .and_then(|i| self.visible_results().get(i).cloned())
                else {
                    return false;
                };
                if !sel.is_empty() {
                    return false; // copying typed text
                }
                self.copy_result(r, cx);
            }
            _ => return false,
        }
        true
    }

    /// Tab (or the mode chip): search ⇄ ask.
    pub fn toggle_mode(&mut self, cx: &mut Context<Self>) {
        self.select_pill(None);
        let auto = super::tokens::is_question(&self.parse.residual)
            && super::rules::auto_answer_allowed(self.settings.as_ref(), &self.engine());
        self.forced = next_forced(self.forced, auto);
        if self.forced == Some(Forced::Ask) {
            self.remember_query(true, cx);
        }
        self.refresh(cx);
    }

    /// The pill popover: ↑/↓ move, ↵ applies, ⌫ removes, Esc/Tab close; anything else types on.
    fn popover_key(&mut self, k: &Keystroke) -> bool {
        match k.key.as_str() {
            "down" => self.popover_move(1),
            "up" => self.popover_move(-1),
            "enter" => self.apply_option(None),
            "backspace" | "delete" => self.remove_pill(None),
            "escape" | "tab" => self.filters.popover = false,
            _ => {
                if !k.modifiers.platform && !k.modifiers.control {
                    self.select_pill(None);
                }
                return false;
            }
        }
        true
    }

    /// Quick Look's keys while open, and Space-to-look in the list. None = not ours.
    fn look_key(&mut self, k: &Keystroke, cx: &mut Context<Self>) -> Option<bool> {
        let key = k.key.as_str();
        let copy = k.modifiers.platform && key == "c";
        if self.look.is_some() {
            if key == "enter" {
                if k.modifiers.platform {
                    if let Some(id) = self.look.as_ref().and_then(|l| l.result.page_frame()) {
                        self.open_page(id, cx);
                    }
                } else {
                    self.look_open_here(cx);
                }
                return Some(true);
            }
            if copy {
                self.look_copy(cx);
                return Some(true);
            }
            if plain(k) {
                match key {
                    "left" => self.look_scrub(-1, cx),
                    "right" => self.look_scrub(1, cx),
                    "up" => self.look_step(-1, cx),
                    "down" => self.look_step(1, cx),
                    "escape" | "space" | "tab" => self.look_close(),
                    _ => {
                        if types_text(k) {
                            self.look_close();
                            self.look_nav = false;
                        }
                        return None;
                    }
                }
                return Some(true);
            }
            return None;
        }
        let selected = self
            .results
            .selected
            .and_then(|i| self.visible_results().get(i).cloned());
        if (key == "down" || key == "up") && plain(k) {
            self.look_nav = true;
        } else if key == "space" && plain(k) && self.look_nav && selected.is_some() {
            self.look_show(selected.expect("selected"), cx);
            return Some(true);
        } else if types_text(k) {
            self.look_nav = false;
        }
        None
    }

    /// ↓'s Recent + Try: ↑/↓ pick · ↵ use · ⌫ forgets a Recent row · Esc closes.
    fn recent_key(&mut self, k: &Keystroke, cx: &mut Context<Self>) -> bool {
        if k.modifiers.platform || k.modifiers.control || k.modifiers.alt {
            return false;
        }
        let i = self.recent.index;
        let n = self.recent_count();
        match k.key.as_str() {
            "down" => self.recent.index = (i + 1).min(n.saturating_sub(1)),
            "up" => self.recent.index = i.saturating_sub(1),
            "enter" => self.use_recent(i, cx),
            "backspace" | "delete" if i < self.recent.list.len() => {
                self.forget_recent(i, cx);
                self.recent.index = i.min(n.saturating_sub(2));
            }
            "escape" => self.recent.open = false,
            _ => {
                if k.key_char.is_some() {
                    self.recent.open = false;
                }
                return false;
            }
        }
        true
    }

    /// No results: ↑/↓ + ↵ walk the ways out; ↵ on Search failed retries.
    fn empty_key(&mut self, k: &Keystroke, cx: &mut Context<Self>) -> bool {
        if !plain(k) || self.filters.pill.is_some() {
            return false;
        }
        match (self.empty_state(), k.key.as_str()) {
            (Some(EmptyState::Error), "enter") => {
                let q = self.results.query.clone();
                self.run_search(q, cx);
            }
            (Some(EmptyState::None), "down") if !self.ways.is_empty() => {
                self.way_sel = (self.way_sel + 1).min(self.ways.len() - 1)
            }
            (Some(EmptyState::None), "up") => self.way_sel = self.way_sel.saturating_sub(1),
            (Some(EmptyState::None), "enter") if !self.ways.is_empty() => {
                self.apply_way(self.way_sel, cx)
            }
            _ => return false,
        }
        true
    }

    /// AskPane's keys (`handleKey`); true = used.
    fn ask_key(&mut self, k: &Keystroke, cx: &mut Context<Self>) -> bool {
        let key = k.key.as_str();
        let p = plain(k);
        let cmd = k.modifiers.platform || k.modifiers.control;
        if key == "down" || key == "up" {
            self.ask.nav = true;
        } else if k.key_char.is_some() && key != "space" && !cmd {
            self.ask.nav = false;
        }
        if self.ask_view() == Some(AskView::Gate) || self.ask_notice().is_some() {
            if self.ask_view() == Some(AskView::Gate) && key == "enter" && p && !self.ask.nav {
                self.ask_now(cx);
                return true;
            }
            return false;
        }
        let n = self.ask.sources().len();
        let looks = self.looks();
        match key {
            "escape" if p && self.ask.live() => self.ask_stop(cx),
            "down" | "up" => {
                let d: isize = if key == "down" { 1 } else { -1 };
                self.ask.sel = (self.ask.sel as isize + d).clamp(0, looks.len() as isize) as usize;
                self.ask.cite = 0;
            }
            "left" | "right" if self.ask.nav && self.ask.sel == 0 && n > 0 && p => {
                let right = key == "right";
                let c = if self.ask.cite == 0 {
                    if right {
                        0
                    } else {
                        1
                    }
                } else {
                    self.ask.cite
                };
                self.ask.cite = ((c as isize - 1 + if right { 1 } else { -1 })
                    .rem_euclid(n as isize)) as usize
                    + 1;
            }
            "space" => {
                let i = if self.ask.sel > 0 {
                    self.ask.sel - 1
                } else {
                    self.ask.cite.max(1) - 1
                };
                if !self.ask.nav || i >= looks.len() {
                    return false;
                }
                let from_cite = self.ask.sel == 0;
                self.look_cited(looks, i, from_cite, cx);
            }
            "enter" => {
                let sel = self.ask.sel;
                if sel > n {
                    let Some(r) = self.more_moments().get(sel - n - 1).cloned() else {
                        return true;
                    };
                    if cmd {
                        if let Some(id) = r.page_frame() {
                            self.open_page(id, cx);
                        }
                    } else {
                        self.ask.handoff = true;
                        let q = self.ask.asked.clone();
                        self.hand_off(super::panel::Target::of(&r, None), q, cx);
                    }
                } else if sel > 0 {
                    self.open_cite(sel - 1, cmd, cx);
                } else if self.ask_waiting() {
                    self.ask_submit(cx);
                } else if self.ask.phase() == Some("done") {
                    self.continue_in_chat(cx);
                } else if self.ask.phase() == Some("error") {
                    let q = self.ask.asked.clone();
                    self.ask_start(q, cx);
                }
            }
            "c" if cmd => {
                let typed = !self.input.read(cx).selected_range().is_empty();
                if self.ask.sel != 0 || self.ask.phase() != Some("done") || typed {
                    return false;
                }
                self.copy_answer(cx);
            }
            _ => return false,
        }
        true
    }
}
