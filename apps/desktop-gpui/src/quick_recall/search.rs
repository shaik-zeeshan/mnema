//! Search mode (`searchStore.svelte.ts`, `filterSurfaces`, `states.svelte.ts`):
//! debounce and commit timers, the ranked results, selection, the ghost, the
//! pill popover with its counts, Recent and the no-results ways out.

use std::collections::HashSet;

use app_infra::{RecentQuery, SearchParseError};
use gpui_kit::Context;

use super::model::{merge_by_rank, FrameHit, Ranked, SearchResponse};
use super::rules::{self, PillOption, WayOut};
use super::tokens::{self, pill_operators, Pill};
use super::{QuickRecall, COMMIT, SEARCH_DEBOUNCE};

/// The answered search (or its failure) and how it shows.
#[derive(Default)]
pub struct Results {
    pub frames: Vec<FrameHit>,
    pub audio: Vec<super::model::AudioHit>,
    pub ranked: Vec<Ranked>,
    pub has_more: (bool, bool),
    pub loading: bool,
    pub error: Option<String>,
    /// The query the shown results belong to.
    pub query: String,
    pub residual: String,
    pub parse_errors: Vec<SearchParseError>,
    pub index_empty: bool,
    pub semantic_degraded: bool,
    pub untranscribed: u32,
    pub selected: Option<usize>,
    pub expanded: bool,
    gen: u64,
}

impl Results {
    pub fn has_results(&self) -> bool {
        !self.frames.is_empty() || !self.audio.is_empty()
    }

    pub fn capped(&self) -> bool {
        self.has_more.0 || self.has_more.1
    }

    fn reset(&mut self) {
        let gen = self.gen + 1;
        *self = Results {
            gen,
            ..Default::default()
        };
    }
}

impl QuickRecall {
    pub(super) fn schedule_commit(&mut self, cx: &mut Context<Self>) {
        self.timers.commit = Some(self.after(COMMIT, cx, |qr, cx| qr.commit_pills(cx)));
    }

    /// Lift the recognised phrases out of the text into pills.
    pub fn commit_pills(&mut self, cx: &mut Context<Self>) {
        self.timers.commit = None;
        if self.parse.spans.is_empty() {
            return;
        }
        self.pills = self.effective_pills();
        let r = self.parse.residual.clone();
        let text = if !r.is_empty() && !r.ends_with('?') {
            format!("{r} ")
        } else {
            r
        };
        self.set_text(&text, cx);
    }

    /// Esc's middle step: drop the text and the pills.
    pub fn clear_query(&mut self, cx: &mut Context<Self>) {
        self.timers.commit = None;
        self.pills.clear();
        self.filters.pill = None;
        self.filters.popover = false;
        self.set_text("", cx);
    }

    /// `raw` is the operator query; the typed text decides the pacing.
    pub(super) fn schedule_search(&mut self, raw: &str, cx: &mut Context<Self>) {
        self.timers.debounce = None;
        if tokens::is_bare_operator(&self.text) {
            self.results.gen += 1;
            self.results.loading = false;
            return;
        }
        let trimmed = raw.trim().to_string();
        if trimmed.chars().count() < rules::MIN_QUERY_LENGTH {
            self.results.reset();
            return;
        }
        let delay = if tokens::is_trailing_operator_partial(&self.text) {
            COMMIT
        } else {
            SEARCH_DEBOUNCE
        };
        self.timers.debounce =
            Some(self.after(delay, cx, move |qr, cx| qr.run_search(trimmed, cx)));
    }

    pub fn run_search(&mut self, query: String, cx: &mut Context<Self>) {
        self.timers.debounce = None;
        self.results.gen += 1;
        let gen = self.results.gen;
        self.results.loading = true;
        self.results.error = None;
        self.load_index_status(cx);
        // Narrow the fetch to the scope only for a same-query re-run (the cached
        // scope belongs to `results.query`).
        let (fl, al) = (rules::FRAME_FETCH_LIMIT, rules::AUDIO_FETCH_LIMIT);
        let q = query.clone();
        self.fetch(
            cx,
            move |b| b.search(&q, fl, al),
            move |qr, res, cx| {
                if gen != qr.results.gen {
                    return;
                }
                match res {
                    Ok(r) => qr.take_results(query, r, cx),
                    Err(e) => {
                        qr.results.reset();
                        qr.results.query = query;
                        log::warn!("quick recall: search failed ({})", error_kind(&e));
                        qr.results.error = Some(e);
                    }
                }
            },
        );
    }

    fn take_results(&mut self, query: String, r: SearchResponse, cx: &mut Context<Self>) {
        let s = &mut self.results;
        if query != s.query {
            s.expanded = false;
        }
        s.ranked = merge_by_rank(&r.frames, &r.audio);
        s.has_more = (r.has_more_frames, r.has_more_audio);
        s.query = query;
        s.residual = r.residual_query;
        s.parse_errors = r.parse_errors;
        s.index_empty = r.index_empty;
        s.semantic_degraded = r.semantic_degraded;
        s.untranscribed = r.untranscribed_segments_in_range;
        s.loading = false;
        s.frames = r.frames;
        s.audio = r.audio;
        self.reselect();
        let ids: Vec<String> = self
            .results
            .frames
            .iter()
            .filter_map(|f| f.app_bundle_id.clone().or_else(|| f.app_name.clone()))
            .collect();
        self.ensure_icons(ids, cx);
        let want: Vec<i64> = self
            .results
            .frames
            .iter()
            .map(|f| f.thumbnail_frame_id)
            .filter(|id| !self.thumbs.contains_key(id))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if !want.is_empty() {
            self.fetch(
                cx,
                move |b| b.previews(&want, None),
                |qr, previews, _| {
                    for p in previews {
                        if let Some(path) = p.path {
                            qr.thumbs.insert(p.frame_id, path);
                        }
                    }
                },
            );
        }
        if self.empty_state() == Some(super::EmptyState::None) {
            self.load_ways_out(cx);
        }
    }

    /// Each search (and a return to search) lights the top hit, so a hurried ↵
    /// opens it; under the gate or a notice nothing is lit.
    pub(super) fn reselect(&mut self) {
        let top = self.ask_view().is_none() && !self.visible_results().is_empty();
        self.results.selected = top.then_some(0);
    }

    pub(super) fn clear_search_state(&mut self, cx: &mut Context<Self>) {
        self.timers.debounce = None;
        self.clear_query(cx);
        self.results.reset();
        self.ways.clear();
    }

    pub fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = self.visible_results().len() as isize;
        if n == 0 {
            if delta > 0 && self.is_empty() {
                self.open_recent(cx);
            }
            return;
        }
        let base = match self.results.selected {
            Some(i) => i as isize,
            None if delta > 0 => -1,
            None => 0,
        };
        self.results.selected = Some((base + delta).rem_euclid(n) as usize);
    }

    pub fn select_result(&mut self, i: usize) {
        if i < self.visible_results().len() {
            self.results.selected = Some(i);
            self.look_nav = true;
        }
    }

    pub fn toggle_expanded(&mut self) {
        self.results.expanded = !self.results.expanded;
        let last = self.visible_results().len().saturating_sub(1);
        self.results.selected = self.results.selected.map(|s| s.min(last));
    }

    /// ↵: the visible result in the main window's Timeline.
    pub fn open_result_at(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(r) = self.visible_results().get(i).cloned() else {
            return;
        };
        let query = self.results.query.clone();
        self.hand_off(super::panel::Target::of(&r, None), query, cx);
    }

    /// ⌘↵: the captured page behind the selected frame.
    pub fn open_selected_page(&mut self, cx: &mut Context<Self>) {
        let frame = self
            .results
            .selected
            .and_then(|i| self.visible_results().get(i))
            .and_then(Ranked::page_frame);
        match frame {
            Some(id) => self.open_page(id, cx),
            None => super::panel::message(
                "Couldn't open page",
                "No openable page for this result.",
                cx,
            ),
        }
    }

    pub fn open_page(&mut self, frame_id: i64, cx: &mut Context<Self>) {
        self.fetch(
            cx,
            move |b| b.open_url(frame_id),
            |_, res, cx| match res {
                Ok(true) => {}
                Ok(false) => super::panel::message(
                    "Couldn't open page",
                    "No openable page for this result.",
                    cx,
                ),
                Err(_) => {
                    super::panel::message("Couldn't open URL", "The page could not be opened.", cx)
                }
            },
        );
    }

    // ── Ghost ───────────────────────────────────────────────────────────────

    pub fn ghost(&self) -> Option<tokens::Ghost> {
        if !self.caret_at_end || self.filters.pill.is_some() {
            return None;
        }
        tokens::ghost(&self.text, self.apps.as_deref().unwrap_or(&[]))
    }

    pub fn accept_ghost(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(g) = self.ghost() else { return false };
        self.set_text(&g.accepted, cx);
        self.schedule_commit(cx);
        true
    }

    // ── Pills and their popover ─────────────────────────────────────────────

    pub fn select_pill(&mut self, i: Option<usize>) {
        self.filters.pill = i;
        if i.is_none() {
            self.filters.popover = false;
        }
    }

    pub fn popover_options(&self) -> Vec<PillOption> {
        let Some(pill) = self.filters.pill.and_then(|i| self.pills.get(i)) else {
            return Vec::new();
        };
        if !self.filters.popover {
            return Vec::new();
        }
        let all = rules::pill_options(pill, self.apps.as_deref().unwrap_or(&[]));
        rules::shown_options(all, self.filters.counts.as_deref(), &pill_operators(pill))
    }

    pub fn open_popover(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(pill) = self.pills.get(i).cloned() else {
            return;
        };
        self.filters.pill = Some(i);
        let current = pill_operators(&pill);
        let all = rules::pill_options(&pill, self.apps.as_deref().unwrap_or(&[]));
        self.filters.index = all.iter().position(|o| o.op == current).unwrap_or(0);
        self.filters.popover = true;
        self.filters.counts = None;
        self.filters.gen += 1;
        let gen = self.filters.gen;
        let calls = rules::popover_calls(&self.parse.residual, &self.effective_pills(), &pill);
        let apps = self.apps.clone().unwrap_or_default();
        let now = self.now(cx);
        self.fetch(
            cx,
            move |b| {
                calls
                    .into_iter()
                    .filter_map(|(f, q)| Some((f, b.facet(&q, f)?)))
                    .collect::<Vec<_>>()
            },
            move |qr, results, _| {
                if gen != qr.filters.gen {
                    return;
                }
                let options = rules::pill_options(&pill, &apps);
                qr.filters.counts = Some(rules::popover_counts(&pill, &options, &results, now));
                // Zero-count rows just dropped out; keep the highlight on the current one.
                qr.filters.index = qr
                    .popover_options()
                    .iter()
                    .position(|o| o.op == current)
                    .unwrap_or(0);
            },
        );
        if matches!(self.pills.get(i), Some(Pill::App(_))) {
            self.ensure_apps(cx);
        }
    }

    pub fn popover_move(&mut self, delta: isize) {
        let n = self.popover_options().len() as isize;
        if n > 0 {
            self.filters.index = (self.filters.index as isize + delta).rem_euclid(n) as usize;
        }
    }

    /// Swap the pressed pill for an option (None drops it).
    pub fn apply_option(&mut self, choice: Option<PillOption>) {
        let Some(choice) =
            choice.or_else(|| self.popover_options().get(self.filters.index).cloned())
        else {
            return;
        };
        let Some(i) = self.filters.pill else { return };
        match choice.pill {
            Some(p) => self.pills[i] = p,
            None => {
                self.pills.remove(i);
            }
        }
        self.select_pill(None);
    }

    pub fn remove_pill(&mut self, i: Option<usize>) {
        let Some(i) = i.or(self.filters.pill) else {
            return;
        };
        if i < self.pills.len() {
            self.pills.remove(i);
        }
        self.select_pill(None);
    }

    /// Put the pill back into the text as the phrase it came from.
    pub fn edit_as_text(&mut self, cx: &mut Context<Self>) {
        let Some(pill) = self.filters.pill.and_then(|i| self.pills.get(i)).cloned() else {
            return;
        };
        self.remove_pill(None);
        let text = format!("{} {}", self.text.trim(), tokens::pill_edit_text(&pill))
            .trim()
            .to_string();
        self.set_text(&text, cx);
        self.schedule_commit(cx);
    }

    // ── Recent ──────────────────────────────────────────────────────────────

    pub fn tries(&self) -> Vec<(String, bool)> {
        let app = self
            .apps
            .as_ref()
            .and_then(|a| a.first())
            .cloned()
            .unwrap_or_else(|| "Arc".into());
        vec![
            (format!("notes in {app} yesterday afternoon"), false),
            ("what did I work on this morning?".into(), true),
            ("standup on a call this week".into(), false),
        ]
    }

    pub fn recent_count(&self) -> usize {
        self.recent.list.len() + self.tries().len()
    }

    pub fn open_recent(&mut self, cx: &mut Context<Self>) {
        self.recent.index = 0;
        self.recent.open = true;
        self.fetch(
            cx,
            |b| b.recent(),
            |qr, list, _| {
                if let Some(list) = list {
                    qr.recent.list = list;
                }
            },
        );
    }

    /// Every search acted on (and every ask) goes to the top of Recent.
    pub fn remember_query(&mut self, ask: bool, cx: &mut Context<Self>) {
        let text = rules::recent_text(&self.parse.residual, &self.effective_pills());
        if text.chars().count() < 2 {
            return;
        }
        let at = crate::ui::clock::now_ms(cx);
        self.fetch(
            cx,
            move |b| {
                let stored = b.recent()?;
                let queries = rules::with_recent(&stored, RecentQuery { text, ask, at });
                b.set_recent(queries.clone()).or(Some(queries))
            },
            |qr, list, _| {
                if let Some(list) = list {
                    qr.recent.list = list;
                }
            },
        );
    }

    pub fn forget_recent(&mut self, i: usize, cx: &mut Context<Self>) {
        if i >= self.recent.list.len() {
            return;
        }
        self.recent.list.remove(i);
        let list = self.recent.list.clone();
        self.fetch(cx, move |b| b.set_recent(list), |_, _, _| {});
    }

    /// Type a Recent / Try row in: its phrases tint, then hop into pills.
    pub fn use_recent(&mut self, i: usize, cx: &mut Context<Self>) {
        let text = self.recent.list.get(i).map(|q| q.text.clone()).or_else(|| {
            self.tries()
                .get(i - self.recent.list.len().min(i))
                .map(|t| t.0.clone())
        });
        let Some(text) = text else { return };
        self.recent.open = false;
        self.pills.clear();
        self.set_text(&text, cx);
        self.schedule_commit(cx);
    }

    // ── No results: ways out ────────────────────────────────────────────────

    fn load_ways_out(&mut self, cx: &mut Context<Self>) {
        self.ways_gen += 1;
        let gen = self.ways_gen;
        self.ways.clear();
        let ways = rules::ways_out(
            &self.parse.residual,
            &self.effective_pills(),
            self.ask_available(),
        );
        self.fetch(
            cx,
            move |b| {
                ways.into_iter()
                    .filter_map(|w| match &w {
                        WayOut::Ask { .. } => Some((w, None)),
                        WayOut::Pills { facet, query, .. } => {
                            let c = b.facet(query, *facet)?;
                            (c.total > 0).then(|| {
                                let n = rules::count_label(c.total as usize, c.capped);
                                (w, Some(n))
                            })
                        }
                    })
                    .collect::<Vec<_>>()
            },
            move |qr, rows, _| {
                if gen == qr.ways_gen {
                    qr.ways = rows;
                    qr.way_sel = 0;
                }
            },
        );
    }

    pub fn apply_way(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some((way, _)) = self.ways.get(i).cloned() else {
            return;
        };
        match way {
            WayOut::Ask { .. } => self.ask_now(cx),
            WayOut::Pills { pills, .. } => {
                let r = self.parse.residual.clone();
                self.pills = pills;
                self.set_text(&r, cx);
            }
        }
    }
}

/// A failed search's kind for the log: the backend's fixed prefix, never the
/// rest (an index error can quote the query).
fn error_kind(e: &str) -> &'static str {
    [
        "search isn't ready yet",
        "failed to search captured content",
        "unreadable search response",
    ]
    .into_iter()
    .find(|k| e.starts_with(k))
    .unwrap_or("other")
}
