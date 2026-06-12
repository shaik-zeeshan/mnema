//! Streaming answer parser (issue #110, Slice 2).
//!
//! Accumulates a turn's raw answer text delta-by-delta and maintains an
//! append-only `Vec<AnswerBlock>` — the backend-owned render model. Each
//! `push_delta` returns the minimal `Vec<TurnUpdate>` ops it performed, so the
//! emitted op stream and a fresh reload from `blocks()` can never diverge (the
//! op-replay tests pin that invariant).
//!
//! This is the Rust port of the frontend regex parser that used to live in
//! `Chat.svelte` (`buildSegments` + `parseBarsBlock`/`parseDossierBlock`/
//! `parseTimelineBlock`). The recognized fences are exactly ` ```mnema-bars `,
//! ` ```mnema-dossier `, ` ```mnema-timeline ` (the info string may carry
//! trailing chars before the newline). Any other fenced block (e.g. ` ```rust `)
//! is ORDINARY PROSE and is never intercepted.
//!
//! `Prose.markdown` carries RAW markdown; the markdown→HTML pass stays on the
//! frontend (`AnswerProse`). This parser does no HTML.
//!
//! ## Streaming state machine
//! The mutable TAIL of the answer is one of:
//! - a growing `Prose` block (the trailing element of `blocks`), or
//! - a PENDING open `mnema-*` fence whose text is held back (NOT emitted as
//!   prose) until its closing ` ``` ` line arrives.
//!
//! We commit only fully-decided regions. A fence opener / closer / JSON body may
//! arrive split across deltas; we buffer the undecided tail and re-scan it on the
//! next delta. A valid closed fence becomes a typed block (`OpenBlock`); a
//! malformed one degrades to prose — its raw ` ```mnema-…\n…``` ` text is appended
//! verbatim via `AppendProse`, matching today's silent fallback (renders as a
//! code block, no visible regression). An unterminated fence at `finalize()`
//! likewise degrades to prose.

use capture_types::{
    AnswerBlock, BarsItem, DossierItem, TimelineItem, TurnUpdate,
};

/// What the in-progress (uncommitted) tail of the answer currently is.
enum Mode {
    /// Outside any recognized fence. Uncommitted text is prose-in-progress.
    Prose,
    /// Inside a recognized `mnema-*` fence whose closing ` ``` ` has not yet
    /// arrived. `variant` is the matched kind; `opener_start` is the byte index in
    /// `raw` where the opener ` ``` ` begins (so we can recover the verbatim block
    /// text on degrade).
    InFence {
        variant: Variant,
        opener_start: usize,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Variant {
    Bars,
    Dossier,
    Timeline,
}

impl Variant {
    /// Parse the recognized variant from a fence info string's tail (the text
    /// after the opening ` ``` ` up to the newline). Returns `None` for any
    /// non-`mnema-*` info string, which keeps the fence as ordinary prose.
    fn from_info(info: &str) -> Option<Self> {
        // The info string MUST start with the exact tag; trailing chars allowed.
        if info.starts_with("mnema-bars") {
            Some(Variant::Bars)
        } else if info.starts_with("mnema-dossier") {
            Some(Variant::Dossier)
        } else if info.starts_with("mnema-timeline") {
            Some(Variant::Timeline)
        } else {
            None
        }
    }
}

/// A streaming, append-only parser turning a turn's raw answer text into
/// render-ready [`AnswerBlock`]s while emitting the minimal [`TurnUpdate`] ops.
pub struct AnswerView {
    /// Full accumulated answer text seen so far.
    raw: String,
    /// The append-only render-ready blocks.
    blocks: Vec<AnswerBlock>,
    /// Byte offset into `raw` up to which everything has been committed into
    /// `blocks` (as prose) or recognized as the start of an open fence.
    committed: usize,
    /// Current parse mode for the uncommitted tail.
    mode: Mode,
}

impl Default for AnswerView {
    fn default() -> Self {
        Self::new()
    }
}

impl AnswerView {
    pub fn new() -> Self {
        Self {
            raw: String::new(),
            blocks: Vec::new(),
            committed: 0,
            mode: Mode::Prose,
        }
    }

    /// The append-only render-ready blocks accumulated so far.
    pub fn blocks(&self) -> &[AnswerBlock] {
        &self.blocks
    }

    /// Append a streamed text delta and return the minimal ops performed.
    pub fn push_delta(&mut self, text: &str) -> Vec<TurnUpdate> {
        self.raw.push_str(text);
        let mut ops = Vec::new();
        self.scan(&mut ops);
        ops
    }

    /// End of stream: an unterminated `mnema-*` fence degrades to prose (its raw
    /// held-back text is appended verbatim). Returns any final ops.
    pub fn finalize(&mut self) -> Vec<TurnUpdate> {
        let mut ops = Vec::new();
        if let Mode::InFence { opener_start, .. } = self.mode {
            // The opener was seen but never closed: flush everything from the
            // opener to end-of-text as prose, verbatim.
            let text = self.raw[opener_start..].to_string();
            self.append_prose(&text, &mut ops);
            self.committed = self.raw.len();
            self.mode = Mode::Prose;
        }
        ops
    }

    /// Re-scan the uncommitted tail, committing every fully-decided region and
    /// leaving any undecided partial token buffered for the next delta.
    fn scan(&mut self, ops: &mut Vec<TurnUpdate>) {
        loop {
            match self.mode {
                Mode::Prose => {
                    if !self.scan_prose(ops) {
                        break;
                    }
                }
                Mode::InFence {
                    variant,
                    opener_start,
                } => {
                    if !self.scan_in_fence(variant, opener_start, ops) {
                        break;
                    }
                }
            }
        }
    }

    /// In prose mode, emit as prose everything up to the next recognized fence
    /// opener, then switch into fence mode. Returns `true` if it made progress and
    /// the loop should continue (mode changed), `false` if it parked (buffered a
    /// partial opener / committed all available prose).
    fn scan_prose(&mut self, ops: &mut Vec<TurnUpdate>) -> bool {
        // Decide everything from the (immutably-borrowed) tail FIRST, capturing
        // owned values, so the borrow ends before we mutate `self`.
        let decision = {
            let tail = &self.raw[self.committed..];
            // Find the earliest fence opener ` ``` ` at a line start.
            match find_fence_open(tail) {
                FenceOpen::None => {
                    // No fence ahead. The tail might END with a partial backtick run
                    // ("`" / "``") that could grow into a fence — hold it back so we
                    // never emit prose we may later reinterpret.
                    let hold = trailing_open_candidate_len(tail);
                    let emit_end = tail.len() - hold;
                    ProseDecision::Park {
                        prose: tail[..emit_end].to_string(),
                        advance: emit_end,
                    }
                }
                FenceOpen::Partial { at } => {
                    // A ` ``` ` opener begins at `at` but its info line isn't
                    // newline-terminated — variant undecided. Commit prose before it;
                    // hold the rest pending.
                    ProseDecision::Park {
                        prose: tail[..at].to_string(),
                        advance: at,
                    }
                }
                FenceOpen::Found {
                    at,
                    info_start,
                    body_start,
                } => {
                    let pre = tail[..at].to_string();
                    // The info string runs from after the ` ``` ` up to the newline
                    // ending the opener line (`body_start` is just past it).
                    match Variant::from_info(&tail[info_start..body_start.saturating_sub(1)]) {
                        Some(variant) => ProseDecision::EnterFence {
                            pre,
                            opener_at: at,
                            variant,
                        },
                        None => ProseDecision::OrdinaryFence {
                            // Commit up to and through the opener line as prose, then
                            // keep scanning for a real mnema fence after it. The
                            // ordinary fence's own body/close stream in as prose.
                            prose: tail[..body_start].to_string(),
                            advance: body_start,
                        },
                    }
                }
            }
        };

        match decision {
            ProseDecision::Park { prose, advance } => {
                self.append_prose(&prose, ops);
                self.committed += advance;
                false
            }
            ProseDecision::OrdinaryFence { prose, advance } => {
                self.append_prose(&prose, ops);
                self.committed += advance;
                true
            }
            ProseDecision::EnterFence {
                pre,
                opener_at,
                variant,
            } => {
                self.append_prose(&pre, ops);
                let opener_start = self.committed + opener_at;
                self.committed = opener_start;
                self.mode = Mode::InFence {
                    variant,
                    opener_start,
                };
                true
            }
        }
    }

    /// In fence mode, look for the closing ` ``` ` line. If found, parse the body
    /// and either push a typed block or degrade to prose. Returns `true` if it
    /// made progress (mode returned to prose), `false` if the close has not yet
    /// fully arrived (stay pending, emit nothing).
    fn scan_in_fence(
        &mut self,
        variant: Variant,
        opener_start: usize,
        ops: &mut Vec<TurnUpdate>,
    ) -> bool {
        // The fence content begins after the opener's info line. Re-derive the
        // body start from `opener_start`.
        let after_opener = &self.raw[opener_start..];
        let Some(nl) = after_opener.find('\n') else {
            // The opener's info line itself isn't terminated yet — wait.
            return false;
        };
        let body_start = opener_start + nl + 1;
        let body_region = &self.raw[body_start..];

        match find_fence_close(body_region) {
            None => {
                // No closing ``` line yet — keep pending, emit nothing.
                false
            }
            Some(close) => {
                // The body is everything before the closing fence. The whole
                // verbatim block runs from `opener_start` to the end of the close.
                let body = body_region[..close.body_end].to_string();
                let block_end = body_start + close.fence_end;

                let parsed = match variant {
                    Variant::Bars => parse_bars(&body),
                    Variant::Dossier => parse_dossier(&body),
                    Variant::Timeline => parse_timeline(&body),
                };

                match parsed {
                    Some(block) => {
                        self.blocks.push(block.clone());
                        ops.push(TurnUpdate::OpenBlock { block });
                    }
                    None => {
                        // Degrade to prose: append the verbatim fenced block.
                        let verbatim = self.raw[opener_start..block_end].to_string();
                        self.append_prose(&verbatim, ops);
                    }
                }
                self.committed = block_end;
                self.mode = Mode::Prose;
                true
            }
        }
    }

    /// Append prose `text` to the trailing prose block (or start a new one) and
    /// record the matching `AppendProse` op. Mirrors the op's apply semantics so
    /// `blocks()` and the op replay can never diverge. Empty text is a no-op.
    fn append_prose(&mut self, text: &str, ops: &mut Vec<TurnUpdate>) {
        if text.is_empty() {
            return;
        }
        match self.blocks.last_mut() {
            Some(AnswerBlock::Prose { markdown }) => markdown.push_str(text),
            _ => self.blocks.push(AnswerBlock::Prose {
                markdown: text.to_string(),
            }),
        }
        ops.push(TurnUpdate::AppendProse {
            text: text.to_string(),
        });
    }
}

/// Parse a complete answer string into its render-ready [`AnswerBlock`]s by
/// running it through a fresh [`AnswerView`] (push the whole answer, then
/// finalize). This is the SAME parse the live stream produces, so a cold
/// reattach (parsing a persisted legacy `answer` on read) yields exactly what
/// the live turn showed. Used by the `get_conversation` command to backfill
/// `blocks` for legacy turns predating the `blocks` column.
pub(crate) fn parse_answer_to_blocks(answer: &str) -> Vec<AnswerBlock> {
    let mut view = AnswerView::new();
    view.push_delta(answer);
    view.finalize();
    view.blocks().to_vec()
}

/// A borrow-free decision computed from the prose tail, applied after the
/// immutable borrow of `raw` is released (so `self` can be mutated).
enum ProseDecision {
    /// Emit `prose`, advance the cursor by `advance`, and stop (stay in prose:
    /// either all available prose committed, or a partial opener buffered).
    Park { prose: String, advance: usize },
    /// An ordinary (non-mnema) fence opener: emit its opener line as prose,
    /// advance, and keep scanning in prose mode.
    OrdinaryFence { prose: String, advance: usize },
    /// A recognized mnema fence opener: emit the `pre` prose, then enter fence
    /// mode at `opener_at` (relative to the current cursor).
    EnterFence {
        pre: String,
        opener_at: usize,
        variant: Variant,
    },
}

/// Outcome of scanning prose text for the next fence opener.
enum FenceOpen {
    /// No opener ahead (and no held partial candidate at the tail).
    None,
    /// An opener begins at byte `at` but its info line isn't newline-terminated
    /// yet, so the variant is undecided — buffer it.
    Partial { at: usize },
    /// A complete opener line: ` ``` ` begins at `at`, the info string runs from
    /// `info_start` to just before `body_start`, and `body_start` is the byte
    /// after the opener line's newline.
    Found {
        at: usize,
        info_start: usize,
        body_start: usize,
    },
}

/// Scan `s` for the next fence opener (a "```" at the start of a line). Returns
/// the earliest one, distinguishing a complete opener line (newline-terminated)
/// from a partial one (still streaming).
fn find_fence_open(s: &str) -> FenceOpen {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let at_line_start = i == 0 || bytes[i - 1] == b'\n';
        if at_line_start && bytes[i..].starts_with(b"```") {
            let info_start = i + 3;
            // Find the newline ending the opener/info line.
            match s[info_start..].find('\n') {
                Some(rel_nl) => {
                    let body_start = info_start + rel_nl + 1;
                    return FenceOpen::Found {
                        at: i,
                        info_start,
                        body_start,
                    };
                }
                None => return FenceOpen::Partial { at: i },
            }
        }
        i += 1;
    }
    FenceOpen::None
}

/// Length of a trailing run at the end of prose that COULD become a fence opener
/// once more arrives. We hold back a line-start "`"/"``"/"```…" so we never emit
/// prose we might reinterpret. Returns 0 if the tail can't begin a fence.
fn trailing_open_candidate_len(s: &str) -> usize {
    // Look at the final line. If it consists solely of backticks (1..=3) at a line
    // start and could extend into a fence, hold it back. We also hold a partial
    // ``` info line — but `find_fence_open` already returns Partial for a full
    // "```", so here we only guard 1–2 trailing backticks that might grow to "```".
    let last_line_start = s.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let last_line = &s[last_line_start..];
    if !last_line.is_empty() && last_line.len() <= 2 && last_line.bytes().all(|b| b == b'`') {
        return last_line.len();
    }
    0
}

/// Result of locating a closing fence within a fence body region.
struct FenceClose {
    /// Byte index in the body region where the body ends (start of the close
    /// line, i.e. before the closing ` ``` `).
    body_end: usize,
    /// Byte index in the body region just past the closing ` ``` ` token.
    fence_end: usize,
}

/// Find the first closing fence (a "```" at a line start) within `s`. Mirrors the
/// frontend regex `([\s\S]*?)```` (non-greedy body up to the first ```` ``` ````).
/// The frontend regex doesn't anchor the close to a line start, but a line-start
/// match is the well-formed case and avoids swallowing inline backticks; we match
/// at a line start to be robust.
fn find_fence_close(s: &str) -> Option<FenceClose> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let at_line_start = i == 0 || bytes[i - 1] == b'\n';
        if at_line_start && bytes[i..].starts_with(b"```") {
            // Body ends at the start of this close line. If the close is preceded
            // by a newline, drop that trailing newline from the body so the
            // verbatim/body boundaries line up with the frontend's slice.
            let body_end = i;
            return Some(FenceClose {
                body_end,
                fence_end: i + 3,
            });
        }
        i += 1;
    }
    None
}

// ── Body parsers (ported from Chat.svelte parse* helpers) ────────────────────

fn parse_bars(body: &str) -> Option<AnswerBlock> {
    let data: serde_json::Value = serde_json::from_str(body).ok()?;
    let raw_bars = data.get("bars")?.as_array()?;
    let items: Vec<BarsItem> = raw_bars
        .iter()
        .filter_map(|b| {
            let obj = b.as_object()?;
            let label = obj.get("label").and_then(|v| v.as_str())?;
            let value = coerce_number(obj.get("value"))?;
            if !value.is_finite() {
                return None;
            }
            let sublabel = obj
                .get("sublabel")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            Some(BarsItem {
                label: label.to_string(),
                value,
                sublabel,
            })
        })
        .collect();
    if items.is_empty() {
        return None;
    }
    let title = data
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some(AnswerBlock::Bars { title, items })
}

fn parse_dossier(body: &str) -> Option<AnswerBlock> {
    let data: serde_json::Value = serde_json::from_str(body).ok()?;
    let raw_items = data.get("items")?.as_array()?;
    let items: Vec<DossierItem> = raw_items
        .iter()
        .filter_map(|it| {
            let obj = it.as_object()?;
            let statement = obj.get("statement").and_then(|v| v.as_str())?;
            if statement.trim().is_empty() {
                return None;
            }
            let subject = obj
                .get("subject")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let confidence = match coerce_number(obj.get("confidence")) {
                Some(c) if c.is_finite() => c.clamp(0.0, 1.0),
                _ => 0.0,
            };
            Some(DossierItem {
                subject,
                statement: statement.to_string(),
                confidence,
            })
        })
        .collect();
    if items.is_empty() {
        return None;
    }
    Some(AnswerBlock::Dossier { items })
}

fn parse_timeline(body: &str) -> Option<AnswerBlock> {
    let data: serde_json::Value = serde_json::from_str(body).ok()?;
    let raw_intervals = data.get("intervals")?.as_array()?;
    let items: Vec<TimelineItem> = raw_intervals
        .iter()
        .filter_map(|it| {
            let obj = it.as_object()?;
            let label = obj.get("label").and_then(|v| v.as_str())?;
            let start = obj.get("start").and_then(|v| v.as_str())?;
            let end = obj.get("end").and_then(|v| v.as_str()).map(str::to_string);
            let app = obj.get("app").and_then(|v| v.as_str()).map(str::to_string);
            let category = obj
                .get("category")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            Some(TimelineItem {
                label: label.to_string(),
                start: start.to_string(),
                end,
                app,
                category,
            })
        })
        .collect();
    if items.is_empty() {
        return None;
    }
    let title = data
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some(AnswerBlock::Timeline { title, items })
}

/// Coerce a JSON value to `f64` the way the frontend's `Number(x)` does: a JSON
/// number passes through; a numeric string is parsed; anything else is `None`
/// (mapping to JS `NaN`, which the callers reject as non-finite).
fn coerce_number(v: Option<&serde_json::Value>) -> Option<f64> {
    match v {
        Some(serde_json::Value::Number(n)) => n.as_f64(),
        Some(serde_json::Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                // JS Number("") === 0.
                Some(0.0)
            } else {
                t.parse::<f64>().ok()
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fold an op stream into blocks using the SAME apply semantics the frontend
    /// uses: `AppendProse` appends to the trailing prose block (or creates one);
    /// `OpenBlock` pushes a block. Other ops are irrelevant to this parser.
    fn apply_ops(ops: &[TurnUpdate]) -> Vec<AnswerBlock> {
        let mut blocks: Vec<AnswerBlock> = Vec::new();
        for op in ops {
            match op {
                TurnUpdate::AppendProse { text } => match blocks.last_mut() {
                    Some(AnswerBlock::Prose { markdown }) => markdown.push_str(text),
                    _ => blocks.push(AnswerBlock::Prose {
                        markdown: text.clone(),
                    }),
                },
                TurnUpdate::OpenBlock { block } => blocks.push(block.clone()),
                _ => {}
            }
        }
        blocks
    }

    /// Feed the whole text in one delta, then finalize; return (parser, all ops).
    fn run_whole(text: &str) -> (AnswerView, Vec<TurnUpdate>) {
        let mut v = AnswerView::new();
        let mut ops = v.push_delta(text);
        ops.extend(v.finalize());
        (v, ops)
    }

    /// Feed the text one char (one UTF-8 codepoint) at a time, then finalize.
    fn run_char_by_char(text: &str) -> (AnswerView, Vec<TurnUpdate>) {
        let mut v = AnswerView::new();
        let mut ops = Vec::new();
        for ch in text.chars() {
            let mut buf = [0u8; 4];
            ops.extend(v.push_delta(ch.encode_utf8(&mut buf)));
        }
        ops.extend(v.finalize());
        (v, ops)
    }

    const BARS_FENCE: &str = "```mnema-bars\n{\"title\":\"Top apps\",\"bars\":[{\"label\":\"Editor\",\"value\":42,\"sublabel\":\"2h\"},{\"label\":\"Browser\",\"value\":18}]}\n```";

    const DOSSIER_FENCE: &str = "```mnema-dossier\n{\"items\":[{\"subject\":\"Alice\",\"statement\":\"ships fast\",\"confidence\":0.8},{\"statement\":\"reviews carefully\"}]}\n```";

    const TIMELINE_FENCE: &str = "```mnema-timeline\n{\"title\":\"Day\",\"intervals\":[{\"label\":\"Coding\",\"start\":\"2026-06-13T10:00:00Z\",\"end\":\"2026-06-13T11:00:00Z\",\"app\":\"Editor\"}]}\n```";

    // 1. Plain prose only.
    #[test]
    fn plain_prose_only() {
        let text = "Hello world.\n\nThis is some **markdown** prose.";
        let (v, ops) = run_whole(text);
        assert_eq!(v.blocks().len(), 1);
        match &v.blocks()[0] {
            AnswerBlock::Prose { markdown } => assert_eq!(markdown, text),
            other => panic!("expected prose, got {other:?}"),
        }
        // Concatenation of all AppendProse text equals the input.
        let cat: String = ops
            .iter()
            .filter_map(|o| match o {
                TurnUpdate::AppendProse { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(cat, text);
        // No OpenBlock ops for pure prose.
        assert!(!ops
            .iter()
            .any(|o| matches!(o, TurnUpdate::OpenBlock { .. })));
    }

    // 2. prose → chart → prose.
    #[test]
    fn prose_chart_prose() {
        let text = format!("Before.\n\n{BARS_FENCE}\n\nAfter.");
        let (v, ops) = run_whole(&text);
        assert_eq!(v.blocks().len(), 3);
        assert!(matches!(v.blocks()[0], AnswerBlock::Prose { .. }));
        assert!(matches!(v.blocks()[1], AnswerBlock::Bars { .. }));
        assert!(matches!(v.blocks()[2], AnswerBlock::Prose { .. }));

        // OpenBlock{Bars} appears between AppendProse runs.
        let open_idx = ops
            .iter()
            .position(|o| matches!(o, TurnUpdate::OpenBlock { block: AnswerBlock::Bars { .. } }))
            .expect("OpenBlock{Bars}");
        assert!(open_idx > 0, "prose before the chart");
        assert!(
            ops[open_idx + 1..]
                .iter()
                .any(|o| matches!(o, TurnUpdate::AppendProse { .. })),
            "prose after the chart"
        );

        // The bars block carries the ported fields.
        if let AnswerBlock::Bars { title, items } = &v.blocks()[1] {
            assert_eq!(title.as_deref(), Some("Top apps"));
            assert_eq!(items.len(), 2);
            assert_eq!(items[0].label, "Editor");
            assert_eq!(items[0].value, 42.0);
            assert_eq!(items[0].sublabel.as_deref(), Some("2h"));
            assert_eq!(items[1].sublabel, None);
        } else {
            panic!("expected bars");
        }
    }

    // 3. A fence split across deltas yields the same single block.
    #[test]
    fn split_fence_equivalent_to_whole() {
        let text = format!("Intro.\n\n{BARS_FENCE}\n\nOutro.");
        let (whole, _) = run_whole(&text);
        let (split, _) = run_char_by_char(&text);
        assert_eq!(whole.blocks(), split.blocks());
        // Exactly one Bars block.
        assert_eq!(
            split
                .blocks()
                .iter()
                .filter(|b| matches!(b, AnswerBlock::Bars { .. }))
                .count(),
            1
        );
    }

    // 4. Malformed JSON inside a CLOSED fence degrades to prose.
    #[test]
    fn malformed_closed_fence_degrades_to_prose() {
        let fence = "```mnema-bars\n{not valid json}\n```";
        let text = format!("Pre.\n\n{fence}\n\nPost.");
        let (v, _) = run_whole(&text);
        // No Bars block.
        assert!(!v
            .blocks()
            .iter()
            .any(|b| matches!(b, AnswerBlock::Bars { .. })));
        // The raw fence text is present in prose.
        let prose: String = v
            .blocks()
            .iter()
            .filter_map(|b| match b {
                AnswerBlock::Prose { markdown } => Some(markdown.as_str()),
                _ => None,
            })
            .collect();
        assert!(prose.contains("```mnema-bars"));
        assert!(prose.contains("{not valid json}"));
    }

    // 4b. Closed fence with no valid items also degrades.
    #[test]
    fn empty_items_fence_degrades_to_prose() {
        let fence = "```mnema-bars\n{\"bars\":[]}\n```";
        let (v, _) = run_whole(fence);
        assert!(!v
            .blocks()
            .iter()
            .any(|b| matches!(b, AnswerBlock::Bars { .. })));
        assert!(matches!(v.blocks().first(), Some(AnswerBlock::Prose { .. })));
    }

    // 5. An unterminated fence at finalize() degrades to prose.
    #[test]
    fn unterminated_fence_degrades_on_finalize() {
        let text = "Lead.\n\n```mnema-bars\n{\"bars\":[{\"label\":\"x\",\"value\":1}]";
        let mut v = AnswerView::new();
        let ops_stream = v.push_delta(text);
        // While still open, no block and the held fence text is NOT yet prose.
        assert!(v
            .blocks()
            .iter()
            .all(|b| !matches!(b, AnswerBlock::Bars { .. })));
        let prose_before: String = v
            .blocks()
            .iter()
            .filter_map(|b| match b {
                AnswerBlock::Prose { markdown } => Some(markdown.as_str()),
                _ => None,
            })
            .collect();
        assert!(!prose_before.contains("mnema-bars"), "held back while open");

        let final_ops = v.finalize();
        // After finalize the raw fence is prose.
        let prose_after: String = v
            .blocks()
            .iter()
            .filter_map(|b| match b {
                AnswerBlock::Prose { markdown } => Some(markdown.as_str()),
                _ => None,
            })
            .collect();
        assert!(prose_after.contains("```mnema-bars"));
        assert!(!v
            .blocks()
            .iter()
            .any(|b| matches!(b, AnswerBlock::Bars { .. })));

        // Op-replay still reconstructs the blocks.
        let mut all = ops_stream;
        all.extend(final_ops);
        assert_eq!(apply_ops(&all), v.blocks());
    }

    // 6. Op-replay equivalence (the critical invariant).
    #[test]
    fn op_replay_equivalence_whole_vs_char() {
        let text = format!(
            "Top apps today.\n\n{BARS_FENCE}\n\nAnd findings:\n\n{DOSSIER_FENCE}\n\nTimeline:\n\n{TIMELINE_FENCE}\n\nDone."
        );

        let (whole, whole_ops) = run_whole(&text);
        let (split, split_ops) = run_char_by_char(&text);

        // Same final blocks regardless of chunking.
        assert_eq!(whole.blocks(), split.blocks());

        // Op replay reconstructs the parser's blocks for BOTH feeds.
        assert_eq!(apply_ops(&whole_ops), whole.blocks());
        assert_eq!(apply_ops(&split_ops), split.blocks());
    }

    // 6b. Op replay for a two-char-at-a-time feed too (boundary stress).
    #[test]
    fn op_replay_chunked_pairs() {
        let text = format!("A.\n\n{TIMELINE_FENCE}\n\nB.");
        let mut v = AnswerView::new();
        let mut ops = Vec::new();
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let chunk: String = chars[i..(i + 2).min(chars.len())].iter().collect();
            ops.extend(v.push_delta(&chunk));
            i += 2;
        }
        ops.extend(v.finalize());
        assert_eq!(apply_ops(&ops), v.blocks());
        assert!(v
            .blocks()
            .iter()
            .any(|b| matches!(b, AnswerBlock::Timeline { .. })));
    }

    // 7. Dossier happy path with ported field handling.
    #[test]
    fn dossier_happy_path() {
        let (v, _) = run_whole(DOSSIER_FENCE);
        assert_eq!(v.blocks().len(), 1);
        if let AnswerBlock::Dossier { items } = &v.blocks()[0] {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0].subject.as_deref(), Some("Alice"));
            assert_eq!(items[0].statement, "ships fast");
            assert_eq!(items[0].confidence, 0.8);
            // Missing subject → None; missing confidence → 0.0.
            assert_eq!(items[1].subject, None);
            assert_eq!(items[1].confidence, 0.0);
        } else {
            panic!("expected dossier");
        }
    }

    // 7b. Dossier confidence clamps to [0,1]; blank statement dropped.
    #[test]
    fn dossier_clamps_and_drops_blank() {
        let fence = "```mnema-dossier\n{\"items\":[{\"statement\":\"  \",\"confidence\":0.5},{\"statement\":\"hot\",\"confidence\":5.0},{\"statement\":\"cold\",\"confidence\":-2.0}]}\n```";
        let (v, _) = run_whole(fence);
        if let AnswerBlock::Dossier { items } = &v.blocks()[0] {
            assert_eq!(items.len(), 2, "blank-statement item dropped");
            assert_eq!(items[0].confidence, 1.0, "clamped high");
            assert_eq!(items[1].confidence, 0.0, "clamped low");
        } else {
            panic!("expected dossier");
        }
    }

    // 7c. Timeline happy path; drops items missing label or start.
    #[test]
    fn timeline_happy_path_and_drops() {
        let fence = "```mnema-timeline\n{\"intervals\":[{\"label\":\"Coding\",\"start\":\"2026-06-13T10:00:00Z\",\"app\":\"Editor\"},{\"start\":\"2026-06-13T11:00:00Z\"},{\"label\":\"Meeting\"}]}\n```";
        let (v, _) = run_whole(fence);
        if let AnswerBlock::Timeline { title, items } = &v.blocks()[0] {
            assert_eq!(*title, None);
            assert_eq!(items.len(), 1, "items missing label or start dropped");
            assert_eq!(items[0].label, "Coding");
            assert_eq!(items[0].start, "2026-06-13T10:00:00Z");
            assert_eq!(items[0].app.as_deref(), Some("Editor"));
            assert_eq!(items[0].end, None);
        } else {
            panic!("expected timeline");
        }
    }

    // Bars value coercion from a numeric string (matches frontend Number()).
    #[test]
    fn bars_value_coerces_numeric_string() {
        let fence = "```mnema-bars\n{\"bars\":[{\"label\":\"x\",\"value\":\"3.5\"},{\"label\":\"bad\",\"value\":\"oops\"}]}\n```";
        let (v, _) = run_whole(fence);
        if let AnswerBlock::Bars { items, .. } = &v.blocks()[0] {
            assert_eq!(items.len(), 1, "non-numeric value dropped");
            assert_eq!(items[0].value, 3.5);
        } else {
            panic!("expected bars");
        }
    }

    // An ordinary (non-mnema) code fence stays prose and is NOT intercepted.
    #[test]
    fn ordinary_code_fence_is_prose() {
        let text = "Here:\n\n```rust\nfn main() {}\n```\n\nDone.";
        let (v, _) = run_whole(text);
        assert!(v
            .blocks()
            .iter()
            .all(|b| matches!(b, AnswerBlock::Prose { .. })));
        let prose: String = v
            .blocks()
            .iter()
            .filter_map(|b| match b {
                AnswerBlock::Prose { markdown } => Some(markdown.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(prose, text);
    }

    // A fence whose info string merely CONTAINS but doesn't START with mnema-* is
    // ordinary prose.
    #[test]
    fn near_miss_info_string_is_prose() {
        let text = "```not-mnema-bars\n{}\n```";
        let (v, _) = run_whole(text);
        assert!(v
            .blocks()
            .iter()
            .all(|b| matches!(b, AnswerBlock::Prose { .. })));
    }

    // parse_answer_to_blocks: a whole answer (prose + a valid mnema-bars fence)
    // parses to [Prose, Bars] — the same result the live stream produces, which
    // is what the get_conversation command uses to backfill legacy turns.
    #[test]
    fn parse_answer_to_blocks_prose_then_bars() {
        let answer = format!("Top apps today.\n\n{BARS_FENCE}");
        let blocks = parse_answer_to_blocks(&answer);
        assert_eq!(blocks.len(), 2);
        assert!(matches!(blocks[0], AnswerBlock::Prose { .. }));
        assert!(matches!(blocks[1], AnswerBlock::Bars { .. }));
    }

    // An empty answer parses to no blocks (the command guards on this too,
    // but the parser itself must not invent a block).
    #[test]
    fn parse_answer_to_blocks_empty_is_no_blocks() {
        assert!(parse_answer_to_blocks("").is_empty());
    }

    // Trailing-backtick hold-back: a delta ending mid-opener must not leak prose
    // that later becomes a fence.
    #[test]
    fn split_at_backtick_boundary() {
        let mut v = AnswerView::new();
        let mut ops = v.push_delta("Intro\n``");
        ops.extend(v.push_delta("`mnema-bars\n{\"bars\":[{\"label\":\"x\",\"value\":1}]}\n```"));
        ops.extend(v.finalize());
        // Exactly one Bars block, prose "Intro\n" preserved.
        assert!(v
            .blocks()
            .iter()
            .any(|b| matches!(b, AnswerBlock::Bars { .. })));
        assert_eq!(apply_ops(&ops), v.blocks());
    }
}
