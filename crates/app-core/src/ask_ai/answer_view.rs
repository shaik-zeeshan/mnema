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

use capture_types::{AnswerBlock, BarsItem, DossierItem, TimelineItem, TurnUpdate};

/// What the in-progress (uncommitted) tail of the answer currently is.
enum Mode {
    /// Outside any recognized fence. Uncommitted text is prose-in-progress.
    Prose,
    /// Inside a recognized `mnema-*` fence whose closing ` ``` ` has not yet
    /// arrived. `variant` is the matched kind; `opener_start` is the byte index in
    /// `raw` where the opener ` ``` ` begins (so we can recover the verbatim block
    /// text on degrade). `scan_from` is the absolute byte index in `raw` at which
    /// the next close-fence scan resumes — advanced past already-scanned text each
    /// delta so the body isn't re-scanned from the start every time (O(n²) → O(n)).
    InFence {
        variant: Variant,
        opener_start: usize,
        scan_from: usize,
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
                    scan_from,
                } => {
                    if !self.scan_in_fence(variant, opener_start, scan_from, ops) {
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
                    // Scanning resumes from the opener; `scan_in_fence` clamps this
                    // forward to the body start once the info line is terminated.
                    scan_from: opener_start,
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
        scan_from: usize,
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

        // Resume the close scan from the saved offset rather than re-scanning the
        // whole body each delta (the O(n²) hot-path bug). `scan_from` is never
        // earlier than `body_start`. It may land mid-line, so derive whether the
        // resume byte is a line start explicitly rather than assuming `i == 0` is.
        let region_start = scan_from.max(body_start);
        let region = &self.raw[region_start..];
        let region_at_line_start =
            region_start == body_start || self.raw.as_bytes()[region_start - 1] == b'\n';

        match find_fence_close(region, region_at_line_start) {
            None => {
                // No closing ``` line yet — keep pending, emit nothing. Advance the
                // resume offset past everything that can no longer become a close: a
                // close must START its line with ` ``` `, so the only live candidate
                // in the scanned tail is the LAST line, and only while it is still a
                // prefix of ` ``` ` (1–2 backticks) that could grow into one. If the
                // last line is already disqualified, resume at the very end so the
                // next delta only scans freshly-arrived bytes. Bounds per-delta work
                // to O(new bytes), making the whole stream O(n) instead of O(n²).
                let resume = next_close_resume(region, region_start, region_at_line_start);
                self.mode = Mode::InFence {
                    variant,
                    opener_start,
                    scan_from: resume,
                };
                false
            }
            Some(close) => {
                // The body is everything from `body_start` before the closing
                // fence. The whole verbatim block runs from `opener_start` to the
                // end of the close. `close` offsets are relative to `region_start`.
                let body_end_abs = region_start + close.body_end;
                let body = self.raw[body_start..body_end_abs].to_string();
                let block_end = region_start + close.fence_end;

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
pub fn parse_answer_to_blocks(answer: &str) -> Vec<AnswerBlock> {
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

/// Given a fence body region `[region_start, region_end)` that contained no close
/// (`find_fence_close` returned `None`), compute the absolute byte offset from
/// which the NEXT delta should resume scanning — skipping everything that can
/// never become a close.
///
/// A close must begin its line with ` ``` `. Every line fully present in this
/// region (i.e. followed by a `\n`) that wasn't already a close is immutable and
/// can never become one. The only live candidate is the final, still-growing
/// line, and only while it is a prefix of ` ``` ` (1–2 backticks). So:
/// - last line is `` ` `` or ` `` ` (could grow into ` ``` `) → resume at its
///   start (preserving the held-back partial close);
/// - otherwise → resume at `region_end` (next delta scans only new bytes; any
///   future close starts after a not-yet-seen `\n`, which the resume scan detects
///   via its own `bytes[i-1] == '\n'` check).
fn next_close_resume(region: &str, region_start: usize, region_at_line_start: bool) -> usize {
    let region_end = region_start + region.len();
    let last_line_rel = region.rfind('\n').map(|i| i + 1).unwrap_or(0);
    // The final line is a line start in the full text only if it follows a `\n`
    // within the region, or the region itself began at a line start.
    let last_line_is_line_start = region.contains('\n') || region_at_line_start;
    let last_line = &region[last_line_rel..];
    let is_growing_close_prefix = last_line_is_line_start
        && !last_line.is_empty()
        && last_line.len() <= 2
        && last_line.bytes().all(|b| b == b'`');
    if is_growing_close_prefix {
        region_start + last_line_rel
    } else {
        region_end
    }
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
///
/// `first_is_line_start` tells whether byte 0 of `s` sits at a line start in the
/// full text. The incremental scanner resumes mid-line (so byte 0 isn't always a
/// line start), so this must be passed rather than assumed from `i == 0`.
fn find_fence_close(s: &str, first_is_line_start: bool) -> Option<FenceClose> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let at_line_start = if i == 0 {
            first_is_line_start
        } else {
            bytes[i - 1] == b'\n'
        };
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
#[path = "answer_view_tests.rs"]
mod tests;
