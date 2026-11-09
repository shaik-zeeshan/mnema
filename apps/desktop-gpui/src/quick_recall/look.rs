//! Quick Look (`detailStore.svelte.ts` + `quick-look.ts` + `context-highlight.ts`):
//! the preview under the bar. Screen: the frame nearest each 15 s step within
//! ±90 s of the match, a 7-cell strip that follows the scrub, the frame's text
//! and match boxes. Audio: the transcript turns as the strip.

use std::collections::HashMap;
use std::path::PathBuf;

use app_infra::MatchBox;
use gpui_kit::Context;

use super::model::{self, AudioHit, AudioSegment, Cite, Frame, FrameHit, Ranked, Turn};
use super::{QuickRecall, COPIED_LOOK};

pub const STEP_MS: i64 = 15_000;
/// ±6 steps = the ±90 s window.
pub const RANGE: i32 = 6;
pub const STRIP_CELLS: usize = 7;

pub struct Look {
    pub result: Ranked,
    /// Screen: step offset from the match (−6…6). Audio: turn index.
    pub k: i32,
    /// `strip[k + RANGE]`: `(frame id, captured ms)`.
    pub strip: Vec<Option<(i64, i64)>>,
    pub thumbs: HashMap<i64, PathBuf>,
    pub gone: HashMap<i64, String>,
    pub ocr: HashMap<i64, Option<String>>,
    pub boxes: HashMap<i64, Vec<MatchBox>>,
    pub turns: Option<Vec<Turn>>,
    /// The stage's full-size (1280) preview per frame.
    pub stage: HashMap<i64, PathBuf>,
    pub copied: bool,
    /// Opened from an answer: ↑/↓ walk its citations; `cited` = the one showing.
    pub cites: Option<Vec<Option<Ranked>>>,
    pub cited: usize,
    /// Opened from the answer's citations (vs an evidence row).
    pub from_cite: bool,
    gen: u64,
}

/// One entry per step k ∈ [−6, 6]: the frame nearest match + k·15 s. Step 0 is
/// always the hit's own frame; sparse capture can repeat a frame.
pub fn pick_strip(frames: &[(i64, i64)], m: (i64, i64)) -> Vec<Option<(i64, i64)>> {
    (-RANGE..=RANGE)
        .map(|k| {
            if k == 0 {
                return Some(m);
            }
            let at = m.1 + k as i64 * STEP_MS;
            frames.iter().copied().min_by_key(|f| (f.1 - at).abs())
        })
        .collect()
}

/// `size` consecutive positions in [lo, hi] around k, centred where room allows.
pub fn strip_window(k: i32, lo: i32, hi: i32, size: usize) -> Vec<i32> {
    let start = lo.max((k - (size as i32 - 1) / 2).min(hi - size as i32 + 1));
    (start..=hi).take(size).collect()
}

/// m:ss (h:mm:ss past the hour).
pub fn clock(ms: i64) -> String {
    let t = (ms.abs() as f64 / 1000.).round() as i64;
    let (m, ss) = (t / 60, t % 60);
    if m >= 60 {
        format!("{}:{:02}:{ss:02}", m / 60, m % 60)
    } else {
        format!("{m}:{ss:02}")
    }
}

/// "+0:15" / "−0:45" from the match.
pub fn signed_clock(ms: i64) -> String {
    format!("{}{}", if ms >= 0 { "+" } else { "−" }, clock(ms))
}

/// What the stage says instead of a picture.
pub fn missing_copy(reason: &str) -> (&'static str, &'static str) {
    match reason {
        "direct_file_missing" | "segment_video_missing" => (
            "This frame’s video is missing",
            "Its recording isn’t on disk any more. The text read off it is still here.",
        ),
        "frame_not_found" => ("This frame is gone", "It was deleted after the search ran."),
        "decode_failed" => (
            "This frame couldn’t be decoded",
            "Its recording is damaged at this point. The text read off it is still here.",
        ),
        _ => (
            "This frame can’t be shown",
            "Mnema couldn’t find it inside its recording. The text read off it is still here.",
        ),
    }
}

/// The turn the match span starts inside, else the nearest (end-exclusive).
pub fn match_turn_index(turns: &[Turn], span_start: u64) -> Option<usize> {
    let s = span_start as i64;
    turns
        .iter()
        .enumerate()
        .min_by_key(|(_, t)| (t.start_ms as i64 - s).max(s - t.end_ms as i64 + 1).max(0))
        .map(|(i, _)| i)
}

const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "been", "being", "but", "by", "can", "could", "did",
    "do", "does", "for", "from", "had", "has", "have", "he", "her", "him", "his", "how", "i", "if",
    "in", "into", "is", "it", "its", "me", "my", "of", "on", "or", "our", "over", "she", "should",
    "so", "some", "than", "that", "the", "their", "them", "then", "they", "this", "to", "was",
    "we", "were", "what", "when", "where", "who", "why", "will", "with", "would", "you", "your",
    "about", "all", "any", "may", "might", "shall",
];

fn words(t: &str) -> Vec<String> {
    t.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// The residual query's terms as the backend matches them (quote-aware, `OR` and
/// `-exclusions` dropped, filler dropped unless a group is all filler), longest first.
pub fn residual_terms(q: &str) -> Vec<String> {
    let mut groups: Vec<Vec<(String, bool)>> = vec![Vec::new()];
    let mut rest = q;
    while let Some(start) = rest.find(|c: char| !c.is_whitespace()) {
        rest = &rest[start..];
        let (tok, quoted, len) = if let Some(body) = rest.strip_prefix('"') {
            let end = body.find('"').unwrap_or(body.len());
            (body[..end].to_string(), true, (end + 2).min(rest.len()))
        } else {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            (rest[..end].to_string(), false, end)
        };
        rest = &rest[len..];
        if !quoted && tok == "OR" {
            groups.push(Vec::new());
        } else {
            groups.last_mut().expect("group").push((tok, quoted));
        }
    }
    let positive = |t: &(String, bool)| t.1 || (t.0 != "OR" && !t.0.starts_with('-'));
    let droppable = |t: &(String, bool)| {
        let w = words(&t.0);
        !t.1 && positive(t)
            && !t.0.ends_with('*')
            && !w.is_empty()
            && w.iter().all(|w| STOPWORDS.contains(&w.as_str()))
    };
    let mut terms: Vec<String> = Vec::new();
    for g in &groups {
        let drop = g.iter().any(|t| positive(t) && !droppable(t));
        for t in g {
            if !positive(t) || (drop && droppable(t)) {
                continue;
            }
            for w in words(&t.0) {
                if w.chars().count() >= 2 && !terms.contains(&w) {
                    terms.push(w);
                }
            }
        }
    }
    terms.sort_by_key(|t| std::cmp::Reverse(t.len()));
    terms
}

/// `text` split into runs whose `marked` parts are whole-word term matches.
pub fn highlight(text: &str, terms: &[String]) -> Vec<(String, bool)> {
    if terms.is_empty() || text.is_empty() {
        return vec![(text.to_string(), false)]
            .into_iter()
            .filter(|r| !r.0.is_empty())
            .collect();
    }
    let lower = text.to_lowercase();
    // Lowercasing can change byte lengths outside ASCII; fall back to plain text then.
    if lower.len() != text.len() {
        return vec![(text.to_string(), false)];
    }
    let is_word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    let mut out = Vec::new();
    let (mut i, mut last) = (0, 0);
    while i < text.len() {
        let hit = terms.iter().find(|t| {
            lower[i..].starts_with(t.as_str())
                && !is_word(text[..i].chars().next_back())
                && !is_word(text[i + t.len()..].chars().next())
        });
        match hit {
            Some(t) => {
                if i > last {
                    out.push((text[last..i].to_string(), false));
                }
                out.push((text[i..i + t.len()].to_string(), true));
                i += t.len();
                last = i;
            }
            None => i += text[i..].chars().next().map_or(1, char::len_utf8),
        }
    }
    if last < text.len() {
        out.push((text[last..].to_string(), false));
    }
    out
}

/// A cited source as a Quick Look result: its own search hit when the list has
/// it, else one built from the citation (no snippet, no boxes).
pub fn source_result(s: &Cite, ranked: &[Ranked]) -> Option<Ranked> {
    if s.kind == "frame" {
        let id = s.frame_id?;
        if let Some(hit) = ranked
            .iter()
            .find(|r| matches!(r, Ranked::Frame(f) if f.representative_frame.id == id))
        {
            return Some(hit.clone());
        }
        let frame = Frame {
            id,
            captured_at: s.started_at.clone(),
            app_name: s.app_name.clone(),
            window_title: s.window_title.clone(),
            url: s.url.clone(),
            ..Default::default()
        };
        return Some(Ranked::Frame(FrameHit {
            rank: 0.,
            group_key: format!("cite-f{id}"),
            representative_frame: frame,
            group_start_at: s.started_at.clone(),
            group_end_at: s.ended_at.clone(),
            match_count: 0,
            snippet: String::new(),
            app_bundle_id: None,
            app_name: s.app_name.clone(),
            window_title: s.window_title.clone(),
            url: s.url.clone(),
            thumbnail_frame_id: id,
            found_by_meaning: false,
            match_boxes: Vec::new(),
        }));
    }
    let seg = s.audio_segment_id?;
    let span = s.span_start_ms.unwrap_or(0);
    let hit = ranked
        .iter()
        .filter_map(|r| match r {
            Ranked::Audio(a) if a.audio_segment.id == seg => Some(a),
            _ => None,
        })
        .min_by_key(|a| (a.span_start_ms as i64 - span as i64).abs());
    if let Some(a) = hit {
        return Some(Ranked::Audio(a.clone()));
    }
    let kind = if s.mic() {
        "microphone"
    } else {
        "system_audio"
    };
    Some(Ranked::Audio(AudioHit {
        rank: 0.,
        group_key: format!("cite-a{seg}-{span}"),
        audio_segment: AudioSegment {
            id: seg,
            source_kind: kind.into(),
            started_at: s.started_at.clone(),
            ended_at: s.ended_at.clone(),
        },
        source_kind: kind.into(),
        span_start_ms: span,
        span_end_ms: span,
        absolute_start_at: s.started_at.clone(),
        absolute_end_at: s.started_at.clone(),
        match_count: 0,
        snippet: String::new(),
        aligned_frame: s.aligned_frame_id.map(|id| Frame {
            id,
            ..Default::default()
        }),
        found_by_meaning: false,
    }))
}

impl Look {
    pub fn is_screen(&self) -> bool {
        matches!(self.result, Ranked::Frame(_))
    }

    /// The frame on the stage.
    pub fn frame(&self) -> Option<(i64, i64)> {
        self.strip.get((self.k + RANGE) as usize).copied().flatten()
    }

    /// The frame whose pixels to paint: the hit's thumbnail frame at the match.
    pub fn pixel_id(&self, f: (i64, i64)) -> i64 {
        match &self.result {
            Ranked::Frame(h) if f.0 == h.representative_frame.id => h.thumbnail_frame_id,
            _ => f.0,
        }
    }

    pub fn preview_id(&self) -> Option<i64> {
        self.frame().map(|f| self.pixel_id(f))
    }

    pub fn turn(&self) -> Option<&Turn> {
        self.turns.as_ref()?.get(self.k.max(0) as usize)
    }
}

impl QuickRecall {
    /// Open on a result (a row's eye, the best moment, Space).
    pub fn look_show(&mut self, r: Ranked, cx: &mut Context<Self>) {
        self.load_look(r, None, false, cx);
    }

    /// Open on citation `at` of the answer's evidence rows.
    pub fn look_cited(
        &mut self,
        rows: Vec<Option<Ranked>>,
        at: usize,
        from_cite: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(Some(r)) = rows.get(at).cloned() else {
            return;
        };
        self.load_look(r, Some((rows, at)), from_cite, cx);
    }

    fn load_look(
        &mut self,
        r: Ranked,
        cites: Option<(Vec<Option<Ranked>>, usize)>,
        from_cite: bool,
        cx: &mut Context<Self>,
    ) {
        let gen = self.look.as_ref().map_or(0, |l| l.gen) + 1;
        self.select_pill(None);
        self.look_nav = true;
        let (cites, cited) = cites.map_or((None, 0), |(c, a)| (Some(c), a));
        let mut look = Look {
            result: r.clone(),
            k: 0,
            strip: Vec::new(),
            thumbs: HashMap::new(),
            gone: HashMap::new(),
            ocr: HashMap::new(),
            boxes: HashMap::new(),
            turns: None,
            stage: HashMap::new(),
            copied: false,
            cites,
            cited,
            from_cite,
            gen,
        };
        match &r {
            Ranked::Frame(hit) => {
                let id = hit.representative_frame.id;
                let at = model::ms(&hit.representative_frame.captured_at);
                look.boxes.insert(id, hit.match_boxes.clone());
                look.strip = pick_strip(&[], (id, at));
                self.look = Some(look);
                self.look_frame(id, gen, cx);
                self.fetch(
                    cx,
                    move |b| {
                        b.frame_times(at - RANGE as i64 * STEP_MS, at + RANGE as i64 * STEP_MS)
                    },
                    move |qr, frames, cx| {
                        let Some(l) = qr.look.as_mut().filter(|l| l.gen == gen) else {
                            return;
                        };
                        l.strip = pick_strip(&frames, (id, at));
                        let ids: Vec<i64> =
                            l.strip.iter().flatten().map(|f| l.pixel_id(*f)).collect();
                        qr.look_thumbs(ids, gen, cx);
                    },
                );
            }
            Ranked::Audio(a) => {
                let seg = a.audio_segment.id;
                let fallback = Turn {
                    start_ms: a.span_start_ms,
                    end_ms: a.span_end_ms,
                    speaker: String::new(),
                    text: model::plain_snippet(&a.snippet),
                };
                let span = a.span_start_ms;
                if let Some(f) = &a.aligned_frame {
                    let id = f.id;
                    self.look = Some(look);
                    self.look_thumbs(vec![id], gen, cx);
                } else {
                    self.look = Some(look);
                }
                self.fetch(
                    cx,
                    move |b| b.turns(seg),
                    move |qr, turns, _| {
                        let Some(l) = qr.look.as_mut().filter(|l| l.gen == gen) else {
                            return;
                        };
                        let turns = if turns.is_empty() {
                            vec![fallback]
                        } else {
                            turns
                        };
                        l.k = match_turn_index(&turns, span).unwrap_or(0) as i32;
                        l.turns = Some(turns);
                    },
                );
            }
        }
    }

    fn look_thumbs(&mut self, ids: Vec<i64>, gen: u64, cx: &mut Context<Self>) {
        self.fetch(
            cx,
            move |b| b.previews(&ids, None),
            move |qr, previews, _| {
                let Some(l) = qr.look.as_mut().filter(|l| l.gen == gen) else {
                    return;
                };
                for p in previews {
                    if let Some(m) = p.missing {
                        l.gone.insert(p.frame_id, m);
                    }
                    if let Some(path) = p.path {
                        l.thumbs.insert(p.frame_id, path);
                    }
                }
            },
        );
    }

    /// The stage frame's text, boxes and full-size picture.
    fn look_frame(&mut self, id: i64, gen: u64, cx: &mut Context<Self>) {
        let Some(l) = self.look.as_ref() else { return };
        let Ranked::Frame(hit) = &l.result else {
            return;
        };
        let pixel = l.pixel_id((id, 0));
        let (want_ocr, want_boxes, want_stage) = (
            !l.ocr.contains_key(&id),
            !l.boxes.contains_key(&id),
            !l.stage.contains_key(&pixel),
        );
        let (query, meaning) = (self.results.query.clone(), hit.found_by_meaning);
        self.fetch(
            cx,
            move |b| {
                let ocr = want_ocr.then(|| b.ocr_text(id));
                let boxes = want_boxes.then(|| b.match_boxes(id, &query, meaning));
                let stage = want_stage
                    .then(|| b.previews(&[pixel], Some(1280)))
                    .unwrap_or_default();
                (ocr, boxes, stage)
            },
            move |qr, (ocr, boxes, stage), _| {
                let Some(l) = qr.look.as_mut().filter(|l| l.gen == gen) else {
                    return;
                };
                if let Some(o) = ocr {
                    l.ocr.insert(id, o);
                }
                if let Some(b) = boxes {
                    l.boxes.insert(id, b);
                }
                for p in stage {
                    if let Some(m) = p.missing {
                        l.gone.insert(p.frame_id, m);
                    }
                    if let Some(path) = p.path {
                        l.stage.insert(p.frame_id, path);
                    }
                }
            },
        );
    }

    pub fn look_close(&mut self) {
        let Some(l) = self.look.take() else { return };
        if l.cites.is_some() {
            // The row it ended on is selected (or lit, from the answer's citations).
            let n = self.ask.sources().len();
            if l.from_cite && l.cited < n {
                self.ask.cite = l.cited + 1;
            } else {
                self.ask.sel = l.cited + 1;
                self.ask.cite = 0;
            }
        }
    }

    /// ←/→: ±15 s (screen) or the next transcript line (audio).
    pub fn look_scrub(&mut self, delta: i32, cx: &mut Context<Self>) {
        let Some(l) = self.look.as_ref() else { return };
        let (lo, hi) = if l.is_screen() {
            (-RANGE, RANGE)
        } else {
            (0, l.turns.as_ref().map_or(1, Vec::len) as i32 - 1)
        };
        let k = (l.k + delta).clamp(lo, hi.max(lo));
        self.look_go(k, cx);
    }

    pub fn look_go(&mut self, k: i32, cx: &mut Context<Self>) {
        let Some(l) = self.look.as_mut() else { return };
        l.k = k;
        let (screen, frame, gen) = (l.is_screen(), l.frame(), l.gen);
        if let (true, Some(f)) = (screen, frame) {
            self.look_frame(f.0, gen, cx);
        }
    }

    /// The position in what ↑/↓ walk ("N of M"); None for a citation the list lacks.
    pub fn look_index(&self) -> Option<(usize, usize)> {
        let l = self.look.as_ref()?;
        if let Some(c) = &l.cites {
            return Some((l.cited, c.len()));
        }
        let v = self.visible_results();
        v.iter()
            .position(|r| r.key() == l.result.key())
            .map(|i| (i, v.len()))
    }

    /// ↑/↓: the neighbouring result (selected as you go) or citation.
    pub fn look_step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(l) = self.look.as_ref() else { return };
        if let Some(c) = l.cites.clone() {
            let mut j = l.cited as isize + delta;
            while j >= 0 && (j as usize) < c.len() && c[j as usize].is_none() {
                j += delta;
            }
            if j >= 0 && (j as usize) < c.len() {
                let from = l.from_cite;
                self.look_cited(c, j as usize, from, cx);
            }
            return;
        }
        let Some((i, n)) = self.look_index() else {
            return;
        };
        let j = i as isize + delta;
        if j < 0 || j as usize >= n {
            return;
        }
        let next = self.visible_results()[j as usize].clone();
        self.results.selected = Some(j as usize);
        self.look_show(next, cx);
    }

    /// ↵: the scrubbed frame (audio: the scrubbed turn's start) in Timeline.
    pub fn look_open_here(&mut self, cx: &mut Context<Self>) {
        let Some(l) = self.look.as_ref() else { return };
        let target = match &l.result {
            Ranked::Frame(h) => {
                super::panel::Target::Frame(l.frame().map_or(h.representative_frame.id, |f| f.0))
            }
            Ranked::Audio(a) => {
                super::panel::Target::of(&l.result, l.turn().map(|t| t.start_ms)).or_aligned(a)
            }
        };
        let q = self.results.query.clone();
        self.hand_off(target, q, cx);
    }

    /// ⌘C: the stage frame's text, or the scrubbed line.
    pub fn look_copy(&mut self, cx: &mut Context<Self>) {
        let Some(l) = self.look.as_mut() else { return };
        let text = if l.is_screen() {
            l.frame().and_then(|f| l.ocr.get(&f.0).cloned().flatten())
        } else {
            l.turn().map(|t| t.text.clone())
        };
        let Some(text) = text else { return };
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
        l.copied = true;
        self.timers.copied = Some(self.after(COPIED_LOOK, cx, |qr, _| {
            if let Some(l) = qr.look.as_mut() {
                l.copied = false;
            }
        }));
    }

    /// ⌘C in the list: the result's OCR text or its matched line.
    pub fn copy_result(&mut self, r: Ranked, cx: &mut Context<Self>) {
        self.fetch(
            cx,
            move |b| match &r {
                Ranked::Frame(f) => b
                    .ocr_text(f.representative_frame.id)
                    .unwrap_or_else(|| model::plain_snippet(&f.snippet)),
                Ranked::Audio(a) => {
                    let turns = b.turns(a.audio_segment.id);
                    match_turn_index(&turns, a.span_start_ms)
                        .and_then(|i| turns.get(i).map(|t| t.text.clone()))
                        .unwrap_or_else(|| model::plain_snippet(&a.snippet))
                }
            },
            |_, text, cx| cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text)),
        );
    }

    /// The key row while Quick Look is open.
    pub fn look_keys(&self) -> Option<Vec<(&'static str, &'static str)>> {
        let l = self.look.as_ref()?;
        let mut keys = Vec::new();
        let steps = self.look_index().is_some();
        if !l.is_screen() {
            keys.push(("←→", "lines"));
            if steps {
                keys.push(("↑↓", "results"));
            }
            keys.extend([
                ("↵", "open at this line"),
                ("⌘C", "copy quote"),
                ("space", "back"),
            ]);
            return Some(keys);
        }
        let gone = l.preview_id().is_some_and(|id| l.gone.contains_key(&id));
        keys.push(("←→", "±15 s"));
        if steps {
            keys.push(("↑↓", "results"));
        }
        keys.push((
            "↵",
            if gone {
                "open in Timeline"
            } else {
                "open this frame"
            },
        ));
        if l.result.page_frame().is_some() {
            keys.push(("⌘↵", "page"));
        }
        keys.extend([("⌘C", "copy text"), ("space", "back")]);
        Some(keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_picks_the_nearest_frame_per_step() {
        let frames = [(1, 0), (2, 14_000), (3, 31_000), (4, -16_000)];
        let s = pick_strip(&frames, (9, 0));
        assert_eq!(s.len(), 13);
        assert_eq!(s[6], Some((9, 0)));
        assert_eq!(s[7], Some((2, 14_000)));
        assert_eq!(s[8], Some((3, 31_000)));
        assert_eq!(s[5], Some((4, -16_000)));
        assert!(pick_strip(&[], (9, 0))
            .iter()
            .enumerate()
            .all(|(i, f)| (i == 6) == f.is_some()));
    }

    #[test]
    fn window_follows_the_scrub() {
        assert_eq!(strip_window(0, -6, 6, 7), vec![-3, -2, -1, 0, 1, 2, 3]);
        assert_eq!(strip_window(-6, -6, 6, 7), vec![-6, -5, -4, -3, -2, -1, 0]);
        assert_eq!(strip_window(6, -6, 6, 7), vec![0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(clock(75_000), "1:15");
        assert_eq!(signed_clock(-45_000), "−0:45");
    }

    #[test]
    fn terms_and_highlights_are_whole_words() {
        let t = residual_terms("the pricing page -draft");
        assert_eq!(t, vec!["pricing", "page"]);
        let h = highlight("Pricing pages, the pricing page", &t);
        let marked: Vec<&str> = h.iter().filter(|r| r.1).map(|r| r.0.as_str()).collect();
        assert_eq!(marked, ["Pricing", "pricing", "page"]);
    }
}
