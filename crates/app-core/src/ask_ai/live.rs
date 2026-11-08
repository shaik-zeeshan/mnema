//! The in-flight turn registries: cancel flags, pending cancels, the live
//! render-ready `TurnView` per conversation, and the last turn's token usage.

use super::*;

/// Process registry mapping a conversation id (the whole thread) to the
/// cooperative cancel flag of its CURRENTLY in-flight turn. Module-level so it
/// survives across separate Tauri command invocations without touching lib.rs
/// state wiring.
///
/// There is no resident session, prompt channel, or ownership token anymore: a
/// turn is a detached task that streams once and exits. The registry exists only
/// so `ask_ai_cancel` can find a running turn's flag, and so a new
/// start/follow-up can cancel a still-running prior turn for the same
/// conversation before launching its replacement.
pub static ASK_AI_INFLIGHT: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

pub fn ask_ai_inflight() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    ASK_AI_INFLIGHT.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Conversations whose cancel arrived BEFORE their turn registered its in-flight
/// flag — the "cancel at the very start of a request" race. The frontend reveals
/// the Stop button the instant a turn is sent, but `ask_ai_start` /
/// `ask_ai_followup` run on the async runtime, so a Stop's `ask_ai_cancel` can be
/// scheduled first. A cancel landing in that window finds no in-flight
/// flag; rather than drop it, `ask_ai_cancel` records the id + arrival
/// time here and the imminent `register_inflight` consumes it, bringing the turn
/// up already cancelled so it short-circuits before any model call.
///
/// The entry expires after [`PENDING_CANCEL_TTL`]: a no-flag cancel is just as
/// often a STALE one aimed at a turn that already finished (Quick Recall's
/// teardown after "Continue in Chat"), and an unexpiring entry pre-cancelled the
/// user's next, unrelated Chat follow-up — a blank answer and no saved row.
pub static ASK_AI_PENDING_CANCEL: OnceLock<Mutex<HashMap<String, std::time::Instant>>> =
    OnceLock::new();

/// How long a pending cancel waits for its turn to register. The race it covers
/// is two commands scheduled out of order (milliseconds).
// ponytail: fixed window; a cancel request carrying the turn it targets would
// remove the heuristic, add when a real race outlasts it.
pub const PENDING_CANCEL_TTL: std::time::Duration = std::time::Duration::from_secs(2);

pub fn ask_ai_pending_cancel() -> &'static Mutex<HashMap<String, std::time::Instant>> {
    ASK_AI_PENDING_CANCEL.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record a cancel that arrived before the conversation's turn registered. A
/// `register_inflight` for this id within [`PENDING_CANCEL_TTL`] consumes the
/// entry and starts its flag already set. Used by `ask_ai_cancel` only when
/// it found no in-flight flag.
pub fn record_cancel_before_register(conversation_id: &str) {
    if let Ok(mut pending) = ask_ai_pending_cancel().lock() {
        // Expired entries can never apply; drop them so stale ids don't pile up.
        pending.retain(|_, at| at.elapsed() < PENDING_CANCEL_TTL);
        pending.insert(conversation_id.to_string(), std::time::Instant::now());
    }
}

/// Register a fresh cancel flag for a conversation's new in-flight turn,
/// returning the flag. Any flag already registered for that conversation (a prior
/// turn still streaming) is CANCELLED before being replaced, so a racing
/// start/follow-up cooperatively stops the displaced turn rather than letting two
/// turns interleave `ask_ai_update` output under the same conversation id. The
/// displaced turn observes its flag between stream items and ends cleanly.
pub fn register_inflight(conversation_id: &str) -> Arc<AtomicBool> {
    register_inflight_at(conversation_id, std::time::Instant::now())
}

pub fn register_inflight_at(conversation_id: &str, now: std::time::Instant) -> Arc<AtomicBool> {
    // Consume a cancel that raced ahead of this registration (see
    // ASK_AI_PENDING_CANCEL): the turn comes up already cancelled and stops at its
    // first check, before any model call. Consume exactly once so the pending entry
    // never poisons a later, legitimate turn for the same conversation, and only
    // while fresh — an expired one was aimed at an earlier turn.
    let pre_cancelled = ask_ai_pending_cancel()
        .lock()
        .ok()
        .and_then(|mut pending| pending.remove(conversation_id))
        .is_some_and(|at| now.saturating_duration_since(at) < PENDING_CANCEL_TTL);
    let cancel = Arc::new(AtomicBool::new(pre_cancelled));
    if let Ok(mut map) = ask_ai_inflight().lock() {
        if let Some(previous) = map.insert(conversation_id.to_string(), cancel.clone()) {
            previous.store(true, Ordering::SeqCst);
        }
    }
    cancel
}

/// Remove the in-flight flag for a conversation only if it is still the exact
/// `cancel` instance the finishing turn registered. A turn calls this on exit so
/// it never evicts a NEWER turn that displaced it (the newer turn registered a
/// different `Arc`).
pub fn remove_inflight_if_owner(conversation_id: &str, cancel: &Arc<AtomicBool>) {
    if let Ok(mut map) = ask_ai_inflight().lock() {
        match map.get(conversation_id) {
            // A newer turn owns the id now — its mark is not ours to drop.
            Some(flag) if !Arc::ptr_eq(flag, cancel) => {}
            owned => {
                if owned.is_some() {
                    map.remove(conversation_id);
                }
                // With our flag gone no cancel can mark us any more; drop a mark
                // that landed after finalize read it. Under the lock, so none
                // slips in between.
                abandoned::take_superseded(conversation_id);
            }
        }
    }
}

/// Whether a turn is still running for this conversation (its in-flight flag
/// is registered). `delete_last_turn` refuses while it is: the running turn's
/// next save would re-insert the row it deleted.
pub fn turn_in_flight(conversation_id: &str) -> bool {
    ask_ai_inflight()
        .lock()
        .is_ok_and(|map| map.contains_key(conversation_id))
}

/// Take (remove and return) the in-flight flag for a conversation: the tests'
/// stand-in for `ask_ai_cancel`, which does the same take inline so it can mark
/// a discard under the lock.
#[cfg(test)]
pub fn take_inflight(conversation_id: &str) -> Option<Arc<AtomicBool>> {
    ask_ai_inflight()
        .lock()
        .ok()
        .and_then(|mut map| map.remove(conversation_id))
}

// ── In-memory LiveTurn store (issue #110, Slice 4) ───────────────────────────
//
// The backend-owned, render-ready view model for the CURRENTLY in-flight turn of
// each conversation. Mirrors the `ASK_AI_INFLIGHT` registry: module-level static
// keyed by conversation id, so it survives across separate Tauri command
// invocations without touching lib.rs state wiring.
//
// Each entry carries a monotonic `version` (bumped under the map lock per applied
// update) and a unique `turn_token`. The token is the LiveTurn analogue of the
// `cancel` Arc ownership in the inflight registry: a newer turn for the same
// conversation OVERWRITES the map entry with a fresh token, and the displaced
// older turn — which still holds its own (now-stale) token — must NOT mutate or
// evict the newer turn's LiveTurn. Ownership checks key off the token.

/// The render-ready view of one in-flight turn plus its monotonic version and the
/// owning turn's unique token.
pub struct LiveTurn {
    pub view: TurnView,
    pub version: u64,
    pub turn_token: u64,
}

pub static ASK_AI_LIVE_TURNS: OnceLock<Mutex<HashMap<String, LiveTurn>>> = OnceLock::new();

pub fn ask_ai_live_turns() -> &'static Mutex<HashMap<String, LiveTurn>> {
    ASK_AI_LIVE_TURNS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The most recent completion request's token usage, for the debug page's
/// "last turn" stat.
///
/// ponytail: in-memory only and last-turn only — persisting usage per turn needs
/// a conversation migration, which is explicitly out of scope. It is therefore
/// empty after a restart and does not attribute usage to a conversation. Add the
/// migration if history or per-conversation totals are ever wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskAiTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Context-window occupancy for this report — `input + output`, or the
    /// provider's lump-sum total when it reported no split. Carried separately
    /// so the debug row and the live context bar cannot disagree: the bar
    /// applies the same fallback, and a total-only provider would otherwise
    /// show a real number there and `in 0 / out 0` here.
    pub context_tokens: u64,
    /// When the turn's agent loop started (unix ms) — the debug row's clock.
    pub started_at_ms: i64,
    /// Visible tool calls this turn (excludes the `reference_captures`
    /// presentation signal, matching the activity rail).
    pub tool_calls: u64,
    /// Elapsed ms from loop start to the usage report. Usage arrives at the end
    /// of the final completion, so this is within a stream-tail of the full turn.
    pub duration_ms: i64,
}

/// Context-window occupancy for one provider usage report.
///
/// The latest completion request's input+output is the turn's occupancy —
/// input already includes the system prompt, history, and tool results. rig
/// keeps `total_tokens` separate "as some providers may only report one
/// number", and such a provider leaves the split at 0, so fall back to its
/// total rather than showing an empty context bar.
///
/// A plain `fn` so the two surfaces that show this number — the live context
/// bar and the debug turn row — cannot drift apart.
pub fn context_tokens(input_tokens: u64, output_tokens: u64, total_tokens: u64) -> u64 {
    let split = input_tokens.saturating_add(output_tokens);
    if split > 0 {
        split
    } else {
        total_tokens
    }
}

pub static ASK_AI_LAST_USAGE: OnceLock<Mutex<Option<AskAiTokenUsage>>> = OnceLock::new();

pub fn ask_ai_last_usage() -> &'static Mutex<Option<AskAiTokenUsage>> {
    ASK_AI_LAST_USAGE.get_or_init(|| Mutex::new(None))
}

pub fn record_last_turn_usage(usage: AskAiTokenUsage) {
    if let Ok(mut cell) = ask_ai_last_usage().lock() {
        *cell = Some(usage);
    }
}

/// The last completion request's `{input, output}` tokens, or `None` when no
/// turn has run since the app started.
pub fn get_ask_ai_last_turn_usage() -> Option<AskAiTokenUsage> {
    ask_ai_last_usage().lock().ok().and_then(|cell| *cell)
}

/// Process-global monotonic counter minting a unique `turn_token` per turn.
pub static ASK_AI_TURN_TOKEN: AtomicU64 = AtomicU64::new(1);

/// Mint a fresh, process-unique turn token.
pub fn next_turn_token() -> u64 {
    ASK_AI_TURN_TOKEN.fetch_add(1, Ordering::SeqCst)
}

/// The canonical reducer: the ONE place that mutates a [`TurnView`] from a
/// [`TurnUpdate`]. Reused by (a) the live map mutation, (b) the replay-equivalence
/// test, and (c) Slice 5's DB→view build, so streaming and reload can never
/// diverge. The `AppendProse` arm MUST match [`AnswerView`]'s prose-coalescing.
pub fn apply_update_to_view(view: &mut TurnView, update: &TurnUpdate) {
    match update {
        TurnUpdate::Phase { phase } => view.phase = phase.clone(),
        TurnUpdate::AppendProse { text } => match view.blocks.last_mut() {
            Some(capture_types::AnswerBlock::Prose { markdown }) => markdown.push_str(text),
            _ => view.blocks.push(capture_types::AnswerBlock::Prose {
                markdown: text.clone(),
            }),
        },
        TurnUpdate::OpenBlock { block } => view.blocks.push(block.clone()),
        TurnUpdate::Reasoning { text } => match view.reasoning.as_mut() {
            Some(existing) => existing.push_str(text),
            None => view.reasoning = Some(text.clone()),
        },
        TurnUpdate::ToolActivity { entry } => view.tool_activities.push(entry.clone()),
        TurnUpdate::LiveActivity { entry } => view.live_activity = entry.clone(),
        TurnUpdate::Sources { sources } => view.sources = sources.clone(),
        TurnUpdate::ContextTokens { tokens } => view.context_tokens = Some(*tokens),
        TurnUpdate::Engine { provider, model } => {
            view.provider = Some(provider.clone());
            view.model = Some(model.clone());
        }
        TurnUpdate::Error { message, .. } => {
            view.error_message = Some(message.clone());
            view.phase = "error".to_string();
        }
        TurnUpdate::Done => {
            view.phase = "done".to_string();
            view.live_activity = None;
        }
    }
}

/// Register a fresh [`LiveTurn`] for a conversation (overwriting any prior entry —
/// a newer turn displacing an older one). Version starts at 0; the first applied
/// update becomes version 1.
pub fn register_live_turn(conversation_id: &str, turn_token: u64, view: TurnView) {
    if let Ok(mut map) = ask_ai_live_turns().lock() {
        map.insert(
            conversation_id.to_string(),
            LiveTurn {
                view,
                version: 0,
                turn_token,
            },
        );
    }
}

/// Pure state core: apply `update` to the conversation's LiveTurn IF this turn
/// (identified by `turn_token`) still owns the conversation. Returns
/// `Some((version, turn_index, update))` on success (so the caller can emit), or
/// `None` when this turn was displaced (a newer turn overwrote the entry) — the
/// caller MUST NOT emit on `None`. Versions are assigned strictly monotonically
/// under the map lock.
pub fn apply_live_update(
    conversation_id: &str,
    turn_token: u64,
    update: TurnUpdate,
) -> Option<(u64, i64, TurnUpdate)> {
    let mut map = ask_ai_live_turns().lock().ok()?;
    let live = map.get_mut(conversation_id)?;
    if live.turn_token != turn_token {
        // Displaced: a newer turn owns this conversation now.
        return None;
    }
    apply_update_to_view(&mut live.view, &update);
    live.version += 1;
    Some((live.version, live.view.turn_index, update))
}

/// Remove the LiveTurn for a conversation only if `turn_token` still owns it.
/// Mirrors [`remove_inflight_if_owner`]: a displaced turn's teardown must never
/// evict the newer turn that replaced it.
pub fn remove_live_turn_if_owner(conversation_id: &str, turn_token: u64) {
    if let Ok(mut map) = ask_ai_live_turns().lock() {
        if map
            .get(conversation_id)
            .is_some_and(|live| live.turn_token == turn_token)
        {
            map.remove(conversation_id);
        }
    }
}

/// Clone the conversation's current LiveTurn into a versioned [`TurnSnapshot`],
/// or `None` when no turn is in flight. Backs the `ask_ai_snapshot` command so a
/// reattaching frontend can self-heal to the exact current view + version.
pub fn snapshot_live_turn(conversation_id: &str) -> Option<TurnSnapshot> {
    let map = ask_ai_live_turns().lock().ok()?;
    let live = map.get(conversation_id)?;
    Some(TurnSnapshot {
        conversation_id: conversation_id.to_string(),
        version: live.version,
        view: live.view.clone(),
    })
}
