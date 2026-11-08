//! The turn's local-time grounding and its All-Retained broker plumbing:
//! tool requests, results back to the model, and the nominated Answer Sources.

use super::*;

/// "Now" in unix milliseconds (UTC), stamped Rust-side on persist so the store
/// stays deterministic. Mirrors the conversation command module's `now_ms`.
pub fn now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

/// The user's wall-clock context for one turn. The user thinks and asks in LOCAL
/// time ("this morning", "yesterday"), but every capture timestamp and every
/// `from`/`to` bound the broker speaks is UTC. The frontend supplies the offset +
/// IANA zone because that is the only SOUND source here: the `time` crate is
/// built without the `local-offset` feature, and `current_local_offset()` is
/// unsound under Tauri's multithreading. Absent (older payloads / unknown) → the
/// grounding falls back to UTC only.
#[derive(Debug, Clone, Default)]
pub struct ClientClock {
    /// Minutes to ADD to UTC to reach the user's local wall clock (e.g. PST = -480,
    /// IST = 330). This is `-Date.getTimezoneOffset()` on the JS side.
    pub utc_offset_minutes: Option<i32>,
    /// IANA zone name for display only (e.g. "America/Los_Angeles").
    pub time_zone: Option<String>,
}

/// Format an `OffsetDateTime` as `YYYY-MM-DD HH:MM` (no seconds — the model reasons
/// in minutes, not instants). Done by hand to avoid pulling in a format-description.
pub fn format_ymd_hm(dt: time::OffsetDateTime) -> String {
    let date = dt.date();
    let clock = dt.time();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        date.year(),
        u8::from(date.month()),
        date.day(),
        clock.hour(),
        clock.minute(),
    )
}

/// Parse a UTC RFC3339 timestamp and return "YYYY-MM-DD HH:MM" in the user's
/// local time, or `None` if the string cannot be parsed.
pub fn utc_rfc3339_to_local_display(utc: &str, offset_minutes: i32) -> Option<String> {
    let dt =
        time::OffsetDateTime::parse(utc, &time::format_description::well_known::Rfc3339).ok()?;
    let local = dt + time::Duration::minutes(i64::from(offset_minutes));
    Some(format_ymd_hm(local))
}

/// Walk a JSON value recursively and inject `startedAtLocal`/`endedAtLocal`
/// siblings next to any `startedAt`/`endedAt` string fields, pre-converting
/// UTC RFC3339 values to the user's local time. This removes the need for the
/// model to do timezone arithmetic when presenting times to the user.
pub fn annotate_local_times(value: &mut serde_json::Value, offset_minutes: i32) {
    match value {
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                annotate_local_times(item, offset_minutes);
            }
        }
        serde_json::Value::Object(map) => {
            let mut to_insert: Vec<(String, serde_json::Value)> = Vec::new();
            for (key, val) in map.iter_mut() {
                annotate_local_times(val, offset_minutes);
                if key == "startedAt" || key == "endedAt" {
                    if let serde_json::Value::String(utc) = val {
                        if let Some(local) = utc_rfc3339_to_local_display(utc, offset_minutes) {
                            to_insert
                                .push((format!("{key}Local"), serde_json::Value::String(local)));
                        }
                    }
                }
            }
            for (k, v) in to_insert {
                map.insert(k, v);
            }
        }
        _ => {}
    }
}

/// Build the per-turn **temporal grounding** line prepended to the prompt: the
/// current local date/time + UTC offset, plus the rule that all capture
/// timestamps and tool `from`/`to` bounds are UTC. Without this the model has no
/// anchor for "today"/"yesterday" and cannot translate the user's local-time
/// phrasing into the UTC windows `search`/`timeline` expect.
pub fn build_temporal_grounding(now_ms: i64, clock: &ClientClock) -> String {
    let now_utc = time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(now_ms) * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);

    let mut grounding = String::from("Temporal grounding: ");
    match clock.utc_offset_minutes {
        Some(offset_minutes) => {
            let local = now_utc + time::Duration::minutes(i64::from(offset_minutes));
            let sign = if offset_minutes < 0 { '-' } else { '+' };
            let abs = offset_minutes.abs();
            let zone = clock
                .time_zone
                .as_deref()
                .map(|zone| format!("{zone}, "))
                .unwrap_or_default();
            grounding.push_str(&format!(
                "the user's current local time is {} ({}UTC{}{:02}:{:02}); in UTC that is {} UTC. ",
                format_ymd_hm(local),
                zone,
                sign,
                abs / 60,
                abs % 60,
                format_ymd_hm(now_utc),
            ));
        }
        None => {
            grounding.push_str(&format!(
                "the current time is {} UTC (the user's local offset is unknown). ",
                format_ymd_hm(now_utc),
            ));
        }
    }
    grounding.push_str(
        "Every capture timestamp you see, and every `from`/`to` bound you pass to `search`, \
`timeline`, and `recall_context`, is in UTC (RFC3339 `Z`). Resolve the user's relative or \
local-time phrasing — \
\"today\", \"yesterday\", \"this morning\", \"last week\" — against the local time above, convert \
it to UTC for tool calls, and present times back to the user in their local time. \
Each search and timeline result also includes pre-converted `startedAtLocal`/`endedAtLocal` \
fields in the same YYYY-MM-DD HH:MM format — use these directly when citing times to the user \
instead of converting the UTC fields yourself.\n\n",
    );
    grounding
}

/// The two-layer Ask AI access gate. `Ok(())` only when Ask AI is enabled in
/// settings AND the shared Reasoning Engine prerequisite passes; the error is a
/// human string the frontend surfaces.
pub async fn ensure_ask_ai_access_ready(core: &AppCore) -> Result<(), String> {
    if !read_ask_ai_enabled(core) {
        return Err("Ask AI access is disabled in settings".to_string());
    }
    let settings = read_recording_settings(core);
    crate::ai_runtime::ask_ai_engine_prerequisite(&settings.ai_runtime, &settings.access).await
}

/// Execute one brokered data-tool request under the Ask AI seam: re-checks access
/// readiness, then runs it through the All-Retained Ask-AI broker scope
/// (`execute_for_ask_ai`) with redaction/audit attributed to the "Ask AI"
/// identity. `open`/`open_in_mnema` never reach here — they are rejected by
/// `broker_request_from_tool` before this is called.
pub async fn execute_ask_ai_broker_request(
    core: Arc<AppCore>,
    request: BrokeredCaptureRequest,
) -> Result<BrokeredCaptureResponse, String> {
    ensure_ask_ai_access_ready(&core).await?;
    broker_access(&core)?
        .execute_for_ask_ai(ask_ai_broker_identity()?, request)
        .await
        .map_err(|error| format!("failed to execute Ask AI broker request: {error}"))
}

/// Map an Ask AI tool name + camelCase params object onto a brokered request.
///
/// Only the Ask AI data tools (`search`, `timeline`, `show_text`, `speakers`,
/// `recall_context`, `activities`) are accepted; `open`/`open_in_mnema`, the presentation-only
/// `reference_captures`, and anything else fall into the unknown branch and are
/// rejected, so they can never be issued as Ask AI data tools.
pub fn broker_request_from_tool(
    tool: &str,
    params: serde_json::Value,
) -> Result<BrokeredCaptureRequest, String> {
    match tool {
        "search" => {
            let request: BrokerSearchRequest = serde_json::from_value(params)
                .map_err(|error| format!("invalid Ask AI search params: {error}"))?;
            Ok(BrokeredCaptureRequest::Search(request))
        }
        "timeline" => {
            let request: BrokerTimelineRequest = serde_json::from_value(params)
                .map_err(|error| format!("invalid Ask AI timeline params: {error}"))?;
            Ok(BrokeredCaptureRequest::Timeline(request))
        }
        "show_text" => {
            let opaque_id = params
                .get("opaqueId")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "Ask AI show_text requires a non-empty opaqueId".to_string())?
                .to_string();
            Ok(BrokeredCaptureRequest::ShowText { opaque_id })
        }
        "speakers" => {
            let request: app_infra::brokered_access::BrokerSpeakersRequest =
                serde_json::from_value(params)
                    .map_err(|error| format!("invalid Ask AI speakers params: {error}"))?;
            Ok(BrokeredCaptureRequest::Speakers(request))
        }
        "activities" => {
            let request: app_infra::brokered_access::BrokerActivitiesRequest =
                serde_json::from_value(params)
                    .map_err(|error| format!("invalid Ask AI activities params: {error}"))?;
            Ok(BrokeredCaptureRequest::Activities(request))
        }
        "recall_context" => {
            let request: BrokerRecallContextRequest = serde_json::from_value(params)
                .map_err(|error| format!("invalid Ask AI recall_context params: {error}"))?;
            Ok(BrokeredCaptureRequest::RecallContext(request))
        }
        other => Err(format!("unknown Ask AI tool: {other}")),
    }
}

/// Convert a brokered response into the JSON value handed back to the model as a
/// tool result, or an error message (broker error envelopes become `Err`).
pub fn broker_response_to_tool_value(
    response: BrokeredCaptureResponse,
) -> Result<serde_json::Value, String> {
    match response {
        BrokeredCaptureResponse::Search(mut response) => {
            // Strip internal frame DB ids + sub-segment timing before the result
            // leaves the device for the (possibly cloud) model. The model never
            // uses these — source cards read them locally from `search_metadata`
            // via the HMAC-signed opaque ref, populated from the raw response
            // before this call. Nulling each `Option` (all `skip_serializing_if`)
            // drops them from the serialized value entirely.
            for result in &mut response.results {
                result.aligned_frame_id = None;
                result.span_start_ms = None;
                result.span_end_ms = None;
            }
            serde_json::to_value(response)
                .map_err(|error| format!("failed to serialize Ask AI search result: {error}"))
        }
        BrokeredCaptureResponse::Timeline(response) => serde_json::to_value(response)
            .map_err(|error| format!("failed to serialize Ask AI timeline result: {error}")),
        BrokeredCaptureResponse::ShowText(response) => serde_json::to_value(response)
            .map_err(|error| format!("failed to serialize Ask AI show_text result: {error}")),
        BrokeredCaptureResponse::RecallContext(response) => serde_json::to_value(response)
            .map_err(|error| format!("failed to serialize Ask AI recall_context result: {error}")),
        BrokeredCaptureResponse::Activities(response) => serde_json::to_value(response)
            .map_err(|error| format!("failed to serialize Ask AI activities result: {error}")),
        BrokeredCaptureResponse::Speakers(response) => serde_json::to_value(response)
            .map_err(|error| format!("failed to serialize Ask AI speakers result: {error}")),
        BrokeredCaptureResponse::Error(error) => Err(error.message),
        BrokeredCaptureResponse::AuthStatus(_)
        | BrokeredCaptureResponse::OpenInMnema(_)
        | BrokeredCaptureResponse::OpenCapturedUrl(_) => {
            Err("unexpected Ask AI broker response".to_string())
        }
    }
}

/// Retain the opaque-id → metadata entries a brokered response handed the model,
/// so a later `reference_captures` can both prove the model only cites ids it
/// actually received AND attach the metadata the source cards render.
///
/// `search` results are AUTHORITATIVE (they alone carry the Audio Search Result
/// Anchor) and overwrite; a `timeline` interval only fills a gap, so a
/// search-then-timeline turn never loses the anchor that lands the dashboard on
/// the cited moment.
pub fn retain_source_metadata(
    response: &BrokeredCaptureResponse,
    metadata: &mut HashMap<String, BrokerSearchResult>,
) {
    match response {
        BrokeredCaptureResponse::Search(response) => {
            for result in &response.results {
                metadata.insert(result.opaque_id.clone(), result.clone());
            }
        }
        BrokeredCaptureResponse::Timeline(response) => {
            for interval in &response.intervals {
                // An interval with no representative capture points at nothing;
                // there is no id for the model to cite.
                let Some(opaque_id) = interval.opaque_id.clone() else {
                    continue;
                };
                metadata
                    .entry(opaque_id.clone())
                    .or_insert_with(|| BrokerSearchResult {
                        opaque_id,
                        // Carried verbatim: `audio_microphone`/`audio_system`/
                        // `frame`, which is what colors the card's mic-vs-system
                        // badge (see `ask_ai_source_kind`).
                        kind: interval.kind.clone(),
                        // Timeline is coarse activity, not a text match — the
                        // source card never renders a snippet.
                        snippet: String::new(),
                        started_at: interval.started_at.clone(),
                        // An open interval has no end yet; the card needs a
                        // non-empty timestamp, and start is the honest one.
                        ended_at: interval
                            .ended_at
                            .clone()
                            .unwrap_or_else(|| interval.started_at.clone()),
                        context: interval.context.clone(),
                        // No sub-segment anchor on a timeline interval: the card
                        // opens at the segment start, as it always has.
                        span_start_ms: None,
                        span_end_ms: None,
                        aligned_frame_id: None,
                        turns: Vec::new(),
                    });
            }
        }
        BrokeredCaptureResponse::Activities(response) => {
            for activity in &response.activities {
                // Absent when every frame that grounded this episode has aged out
                // of Retention — the episode outlives them (ADR 0029), so there is
                // simply nothing left to open.
                let Some(opaque_id) = activity.opaque_id.clone() else {
                    continue;
                };
                metadata
                    .entry(opaque_id.clone())
                    .or_insert_with(|| BrokerSearchResult {
                        opaque_id,
                        // The evidence behind an episode is always a frame — the
                        // broker's headline lookup filters to `subject_type =
                        // 'frame'` — so the card renders as a screen capture with
                        // no mic/system badge.
                        kind: "frame".to_string(),
                        // The card shows the episode's own words rather than a
                        // text match, which is what the user is actually citing.
                        snippet: activity.title.clone(),
                        started_at: activity.started_at.clone(),
                        ended_at: activity.ended_at.clone(),
                        // The evidence frame's own app/window, NOT a claim about
                        // the whole episode: this card renders that one capture,
                        // and without it the card reads "Unknown app".
                        context: activity.context.clone(),
                        span_start_ms: None,
                        span_end_ms: None,
                        aligned_frame_id: None,
                        turns: Vec::new(),
                    });
            }
        }
        _ => {}
    }
}

/// One Answer Source resolved from a nominated opaque id: authoritative
/// frame/audio identity from the signed reference, plus retained metadata from
/// the search result the model actually received.
pub struct ResolvedAskAiSource {
    pub kind: String,
    pub frame_id: Option<i64>,
    pub audio_segment_id: Option<i64>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    /// Guarded host+path of the page behind this source (already sanitized +
    /// secret-redacted at the broker boundary). Frame sources only; audio → None.
    pub url: Option<String>,
    /// The search result's text match (`<mark>` runs kept) — the evidence row's
    /// second line. An activity's title; `None` for a timeline interval.
    pub snippet: Option<String>,
    pub started_at: String,
    pub ended_at: String,
    // Audio Search Result Anchor: sub-segment match timing + aligned frame for
    // audio sources so the dashboard lands on the cited moment rather than the
    // segment start. Always `None` for frame sources.
    pub span_start_ms: Option<i64>,
    pub aligned_frame_id: Option<i64>,
    /// Microphone-vs-system distinction for audio sources, read off the broker
    /// search result's kind. `None` for frames (and for any unknown kind).
    pub source_kind: Option<&'static str>,
}

/// Map a broker search-result kind onto the `sourceKind` vocabulary the source
/// cards render (`AnswerSourceCard.svelte`). The broker already splits audio by
/// channel on the wire, so this needs no database round-trip. Frames (and any
/// unknown kind) carry no source kind.
pub fn ask_ai_source_kind(result_kind: &str) -> Option<&'static str> {
    match result_kind {
        "audio_microphone" => Some("microphone"),
        "audio_system" => Some("system"),
        _ => None,
    }
}

/// Build the capped, de-duped, ordered Answer Source list from nominated opaque
/// ids. `resolve` returns `None` for any id that fails HMAC validation or has no
/// retained metadata (dropped). Returns `(sources_json, accepted, dropped)`,
/// capped to `ASK_AI_SOURCE_FRAME_CAP` frame + `ASK_AI_SOURCE_AUDIO_CAP` audio
/// sources, preserving nomination order.
pub fn build_ask_ai_sources<F>(
    opaque_ids: &[String],
    mut resolve: F,
) -> (Vec<serde_json::Value>, usize, usize)
where
    F: FnMut(&str) -> Option<ResolvedAskAiSource>,
{
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut sources: Vec<serde_json::Value> = Vec::new();
    let mut frame_count = 0usize;
    let mut audio_count = 0usize;

    for id in opaque_ids {
        // Skip duplicates so a repeated nomination is dropped, not double-counted.
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(source) = resolve(id) else {
            continue;
        };

        // Classify by authoritative kind; ignore anything that is not a known
        // frame/audio source, and skip once that kind's cap is full.
        match source.kind.as_str() {
            "frame" => {
                if frame_count >= ASK_AI_SOURCE_FRAME_CAP {
                    continue;
                }
                frame_count += 1;
            }
            "audio" => {
                if audio_count >= ASK_AI_SOURCE_AUDIO_CAP {
                    continue;
                }
                audio_count += 1;
            }
            _ => continue,
        }

        sources.push(serde_json::json!({
            "kind": source.kind,
            "frameId": source.frame_id,
            "audioSegmentId": source.audio_segment_id,
            "appName": source.app_name,
            "windowTitle": source.window_title,
            // Guarded host+path of the cited page (frame sources only; audio:
            // null). Already sanitized + secret-redacted at the broker boundary.
            "url": source.url,
            "snippet": source.snippet,
            "startedAt": source.started_at,
            "endedAt": source.ended_at,
            // Audio Search Result Anchor: sub-segment match span + aligned frame
            // so the dashboard lands on the cited moment (frame sources: null).
            // Field names mirror `selectAudio`'s open payload so the dashboard
            // consumer (`payload.spanStartMs`, `payload.alignedFrameId`) works
            // unchanged.
            "spanStartMs": source.span_start_ms,
            "alignedFrameId": source.aligned_frame_id,
            // Microphone/system distinction for audio sources, carried on the
            // broker search result the model received (frame sources: null).
            "sourceKind": source.source_kind,
        }));
    }

    let accepted = sources.len();
    let dropped = opaque_ids.len().saturating_sub(accepted);
    (sources, accepted, dropped)
}

/// Handle the model's `reference_captures` presentation tool: validate + decode
/// the nominated opaque ids, attach retained metadata, cap the set, and return
/// `{ accepted, dropped }` as the tool result. This never touches the broker
/// dispatch path. The resolved sources JSON is handed back to the caller so the
/// turn driver can emit the `Sources` view update and persist it on the turn row.
pub async fn handle_reference_captures(
    core: &AppCore,
    search_metadata: &Arc<Mutex<HashMap<String, BrokerSearchResult>>>,
    params: serde_json::Value,
) -> Result<(serde_json::Value, Vec<serde_json::Value>), String> {
    let opaque_ids: Vec<String> = params
        .get("opaqueIds")
        .and_then(|value| value.as_array())
        .map(|array| {
            array
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let config_dir = access_config_dir(core)?;

    // Snapshot the retained search-metadata map, then drop the guard so the
    // resolver closure does not hold the lock across decode work.
    let snapshot: HashMap<String, BrokerSearchResult> = search_metadata
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();

    let (sources, accepted, dropped) = build_ask_ai_sources(&opaque_ids, |id| {
        // Authoritative frame/audio identity via the signed reference. A failed
        // HMAC validation or unparseable id yields `None` (dropped).
        let reference =
            app_infra::brokered_access::signed_opaque_capture_reference(&config_dir, id)
                .ok()
                .flatten()?;
        // The model may only reference ids it actually received from `search`.
        let result = snapshot.get(id)?;
        let context = result.context.as_ref();
        Some(ResolvedAskAiSource {
            kind: reference.kind,
            frame_id: reference.frame_id,
            audio_segment_id: reference.audio_segment_id,
            app_name: context.and_then(|context| {
                context
                    .app_name
                    .clone()
                    .or_else(|| context.app_bundle_id.clone())
            }),
            window_title: context.and_then(|context| context.window_title.clone()),
            // The broker already guarded this URL when it built the search
            // result context (frame results only); copy it through verbatim.
            url: result.context.as_ref().and_then(|c| c.url.clone()),
            snippet: Some(result.snippet.clone()).filter(|s| !s.trim().is_empty()),
            started_at: result.started_at.clone(),
            ended_at: result.ended_at.clone(),
            // Audio Search Result Anchor retained from the search result the
            // model received. Present only for audio results; the search mapper
            // leaves these `None` for frames.
            span_start_ms: result.span_start_ms,
            aligned_frame_id: result.aligned_frame_id,
            source_kind: ask_ai_source_kind(&result.kind),
        })
    });

    Ok((
        serde_json::json!({ "accepted": accepted, "dropped": dropped }),
        sources,
    ))
}
