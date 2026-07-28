use std::{collections::HashMap, path::Path};

use sqlx::{QueryBuilder, Row, Sqlite};

use super::{
    broker_optional_filter, invalid_speaker_handle_error, load_or_create_opaque_secret,
    opaque_issuing_grant, opaque_signature, opaque_signature_matches, outside_scope_error,
    scoped_date_range, sqlite_contains_like_pattern, BrokerErrorResponse, BrokerGrant,
    BrokerSpeaker, BrokerSpeakerCoverage, BrokerSpeakerHandle, BrokerSpeakerSummary,
    BrokerSpeakerTurn, BrokerSpeakersRequest, BrokerSpeakersResponse, DEFAULT_SEARCH_LIMIT,
    MAX_SEARCH_LIMIT, OPAQUE_SIGNATURE_HEX_LEN,
};
use crate::{AppInfra, Result, SearchDateRangeRefinement, SearchSpeakerRefinement};

/// A person profile: stable across sessions, channels, and renames.
pub(super) const SPEAKER_HANDLE_KIND_PERSON: &str = "person";
/// One voice inside one capture SESSION — so it spans every consecutive recording
/// in that sitting — fragmenting across sessions, and dead on re-diarization.
/// Marked apart from `person` on the wire because an agent that cannot tell them
/// apart will treat a voice as a human being.
pub(super) const SPEAKER_HANDLE_KIND_VOICE: &str = "voice";

/// Who was heard inside the grant's own time scope, ranked by how long they
/// spoke. The roster is SCOPED, never the global `person_profiles` list: grants
/// are time-bounded, so an unscoped roster would name people from audio this
/// caller was never granted. Scoped, it gives up only what `show-text` already
/// does for the same recordings.
///
/// Handles come from the same rule [`broker_collapse_speakers`] applies — one per
/// PERSON when named or recognized, one per CLUSTER when not — so a handle here is
/// the handle `show-text` publishes for that voice, and the SQL below is that rule
/// written as a `GROUP BY`. Keep the two in step.
pub(super) async fn broker_speakers(
    config_dir: &Path,
    infra: &AppInfra,
    grants: &[BrokerGrant],
    request: BrokerSpeakersRequest,
) -> Result<std::result::Result<BrokerSpeakersResponse, BrokerErrorResponse>> {
    if grants.is_empty() {
        return Ok(Err(BrokerErrorResponse::authorization_required()));
    }
    let limit = request
        .limit
        .unwrap_or(DEFAULT_SEARCH_LIMIT)
        .clamp(1, MAX_SEARCH_LIMIT);
    let name = broker_optional_filter(request.name, "name")?;
    // No `from`/`to`: the grant IS the scope here. `None` means All Retained
    // History, which needs no time predicate at all.
    let range = scoped_date_range(grants, None, None)?;

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT person.id AS person_id, person.display_name AS display_name, \
                CASE WHEN person.id IS NULL THEN turn.cluster_id END AS cluster_id, \
                SUM(turn.end_ms - turn.start_ms) AS speaking_ms, \
                SUM(CASE WHEN person.id IS NULL THEN 0 \
                         WHEN cluster.person_id IS NULL THEN 0 ELSE 1 END) AS assigned_turns, \
                SUM(CASE WHEN person.id IS NULL THEN 0 \
                         WHEN cluster.person_id IS NULL THEN 1 ELSE 0 END) AS recognized_turns \
         FROM speaker_turns turn \
         JOIN recording_speaker_clusters cluster ON cluster.id = turn.cluster_id ",
    );
    // Joined only when there is a range to apply. All Retained History has none,
    // and the join then filters nothing while costing one `audio_segments` rowid
    // lookup per turn in scope — every turn ever recorded, to answer `1 = 1`.
    if range.is_some() {
        query.push("JOIN audio_segments segment ON segment.id = turn.audio_segment_id ");
    }
    query.push(
        "LEFT JOIN person_profiles person \
                ON person.id = COALESCE(cluster.person_id, cluster.recognition_person_id) \
         WHERE 1 = 1",
    );
    if let Some(range) = range.as_ref() {
        query.push(" AND segment.started_at <= ");
        query.push_bind(range.end_at.clone());
        query.push(" AND segment.ended_at >= ");
        query.push_bind(range.start_at.clone());
    }
    if let Some(name) = name.as_deref() {
        query.push(" AND LOWER(person.display_name) LIKE LOWER(");
        query.push_bind(sqlite_contains_like_pattern(name));
        query.push(") ESCAPE '\\'");
    }
    query.push(
        " GROUP BY person.id, person.display_name, \
                  CASE WHEN person.id IS NULL THEN turn.cluster_id END \
          ORDER BY speaking_ms DESC, person.id ASC, cluster_id ASC LIMIT ",
    );
    // One past the cap, so truncation is observed rather than guessed: a ranked
    // page read as the whole roster is an agent reporting "that is everyone".
    query.push_bind(i64::from(limit) + 1);

    let rows = query.build().fetch_all(infra.read_pool()).await?;
    let truncated = rows.len() > limit as usize;
    let secret = load_or_create_opaque_secret(config_dir)?;
    let grant_id = opaque_issuing_grant(grants).map(|grant| grant.id.as_str());
    let speakers = rows
        .iter()
        .take(limit as usize)
        .map(|row| {
            let person_id: Option<i64> = row.get("person_id");
            let handle = match person_id {
                Some(person_id) => person_handle(person_id, grant_id, &secret),
                // No span: a cluster's turn offsets are relative to each audio
                // segment's own start, and one cluster spans several of them.
                None => voice_handle(row.get("cluster_id"), None, grant_id, &secret),
            };
            BrokerSpeakerSummary {
                name: row.get("display_name"),
                handle,
                speaking_ms: row.get::<i64, _>("speaking_ms").max(0) as u64,
                assigned_turns: row.get::<i64, _>("assigned_turns").max(0) as u32,
                recognized_turns: row.get::<i64, _>("recognized_turns").max(0) as u32,
            }
        })
        .collect();
    Ok(Ok(BrokerSpeakersResponse {
        speakers,
        limit,
        truncated,
    }))
}

/// Resolve a wire speaker handle into the row `search`/`timeline` filter on.
///
/// Gated exactly like a capture reference in `broker_authorize_opaque_reference`:
/// a handle this broker never signed does not decode, and a handle whose issuing
/// grant is gone (expired, revoked, or simply another client's) is out of scope.
/// Neither may quietly become "no filter" — an unfiltered page answered for a
/// person is a worse lie than an error.
pub(super) fn broker_speaker_refinement(
    config_dir: &Path,
    grants: &[BrokerGrant],
    handle: Option<String>,
) -> Result<std::result::Result<Option<SearchSpeakerRefinement>, BrokerErrorResponse>> {
    let Some(handle) = broker_optional_filter(handle, "speaker")? else {
        return Ok(Ok(None));
    };
    let secret = load_or_create_opaque_secret(config_dir)?;
    let Some(decoded) = decode_speaker_handle(&handle, &secret) else {
        return Ok(Err(invalid_speaker_handle_error()));
    };
    let in_scope = decoded
        .grant_id
        .as_deref()
        .is_some_and(|grant_id| grants.iter().any(|grant| grant.id == grant_id));
    if !in_scope {
        return Ok(Err(outside_scope_error()));
    }
    Ok(Ok(Some(if decoded.kind == SPEAKER_HANDLE_KIND_PERSON {
        SearchSpeakerRefinement::Person(decoded.id)
    } else {
        SearchSpeakerRefinement::Cluster(decoded.id)
    })))
}

/// This speaker's own turns, keyed by the recording they were heard in — the
/// matched set AND the words in one read. A recording with a matched turn is
/// always a key here, even when that turn carries no text, so the key set is the
/// filter and the values are only what can be quoted.
///
/// Other speakers' turns are deliberately absent: the agent asked what one person
/// said, and re-expanding the payload with everyone else undoes the narrowing the
/// filter just did.
///
/// `audio_segment_ids` is ALWAYS a page the caller already capped, never a range:
/// a range would make this read every turn the speaker ever spoke to publish one
/// page of them.
pub(super) async fn speaker_matched_turns_for_segments(
    infra: &AppInfra,
    speaker: &SearchSpeakerRefinement,
    audio_segment_ids: &[i64],
) -> Result<HashMap<i64, Vec<BrokerSpeakerTurn>>> {
    if audio_segment_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT speaker_turns.audio_segment_id AS audio_segment_id, \
                speaker_turns.start_ms AS start_ms, speaker_turns.end_ms AS end_ms, \
                speaker_turns.transcript_text AS transcript_text",
    );
    speaker.push_matching_turns_source(&mut query);
    query.push(" AND speaker_turns.audio_segment_id IN (");
    let mut separated = query.separated(", ");
    for id in audio_segment_ids {
        separated.push_bind(*id);
    }
    query.push(")");
    query.push(" ORDER BY speaker_turns.audio_segment_id, speaker_turns.start_ms");
    let rows = query.build().fetch_all(infra.read_pool()).await?;

    let mut matched: HashMap<i64, Vec<BrokerSpeakerTurn>> = HashMap::new();
    for row in rows {
        let turns = matched.entry(row.get("audio_segment_id")).or_default();
        // Same rule as `broker_collapse_speakers`: a turn with no words is not
        // published as empty text, which an agent reads as "said nothing". The
        // recording still counts as matched — the voice WAS heard there.
        if let Some(text) = row
            .get::<Option<String>, _>("transcript_text")
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            turns.push(BrokerSpeakerTurn {
                // One speaker per filtered response — the one that was asked for
                // — so there is no `speakers[]` to index into here.
                speaker: None,
                start_ms: row.get::<i64, _>("start_ms").max(0) as u64,
                end_ms: row.get::<i64, _>("end_ms").max(0) as u64,
                text: text.to_string(),
            });
        }
    }
    Ok(matched)
}

/// The recordings in `range` this speaker was heard in, with their words. The
/// timeline scans `audio_segments` rather than the search index, so it asks the
/// same question with the same predicate — `search` cannot answer "when was Priya
/// talking yesterday", which carries no query string at all.
///
/// TWO bounded reads, not one unbounded one: the page of recordings first, then
/// only that page's turns. Read as a single range query it fetched every turn the
/// speaker ever spoke inside the grant — on a months-wide grant, thousands of rows
/// and megabytes of transcript materialized to publish `limit` of them.
pub(super) async fn speaker_matched_turns_in_range(
    infra: &AppInfra,
    speaker: &SearchSpeakerRefinement,
    range: &SearchDateRangeRefinement,
    limit: u32,
) -> Result<HashMap<i64, Vec<BrokerSpeakerTurn>>> {
    let page = speaker_matched_segments_in_range(infra, speaker, range, limit).await?;
    speaker_matched_turns_for_segments(infra, speaker, &page).await
}

/// The first `limit` recordings in `range` this speaker was heard in.
///
/// Range predicate AND ordering copied from `list_overlapping_range`, whose rows
/// the timeline filters against this set and then takes the first `limit` of: a
/// different overlap convention would silently drop a boundary segment, and a
/// different order would return a different page than the unbounded scan did.
async fn speaker_matched_segments_in_range(
    infra: &AppInfra,
    speaker: &SearchSpeakerRefinement,
    range: &SearchDateRangeRefinement,
    limit: u32,
) -> Result<Vec<i64>> {
    let mut query =
        QueryBuilder::<Sqlite>::new("SELECT id FROM audio_segments WHERE started_at <= ");
    query.push_bind(range.end_at.clone());
    query.push(" AND ended_at >= ");
    query.push_bind(range.start_at.clone());
    query.push(" AND ");
    speaker.push_exists_predicate(&mut query, "audio_segments.id");
    query.push(" ORDER BY started_at ASC, ended_at ASC, id ASC LIMIT ");
    query.push_bind(i64::from(limit));
    let rows = query.build().fetch_all(infra.read_pool()).await?;
    Ok(rows.iter().map(|row| row.get("id")).collect())
}

/// How much audio in `range` a speaker filter could not check at all. ONE query
/// for both counts, and only on a filtered request — the ceiling of this whole
/// feature is diarization coverage, not the filter, so it is reported with every
/// answer instead of arriving later as a bug report.
///
/// `range` is `None` only for an All Retained History grant that asked for no
/// bounds, where "in range" means every retained recording.
// ponytail: a full `audio_segments` sweep with two indexed EXISTS probes per row.
// Bounded by the grant's own range on every path but All Retained History; if that
// one ever drags, cache the counts per range rather than adding a second query.
pub(super) async fn speaker_coverage(
    infra: &AppInfra,
    range: Option<&SearchDateRangeRefinement>,
) -> Result<BrokerSpeakerCoverage> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT SUM(CASE WHEN EXISTS ( \
                      SELECT 1 FROM speaker_turns \
                        JOIN recording_speaker_clusters \
                          ON recording_speaker_clusters.id = speaker_turns.cluster_id \
                       WHERE speaker_turns.audio_segment_id = audio_segments.id \
                         AND recording_speaker_clusters.person_id IS NULL \
                         AND recording_speaker_clusters.recognition_person_id IS NULL \
                    ) THEN 1 ELSE 0 END) AS unnamed_voices, \
                SUM(CASE WHEN NOT EXISTS ( \
                      SELECT 1 FROM speaker_turns \
                       WHERE speaker_turns.audio_segment_id = audio_segments.id \
                    ) THEN 1 ELSE 0 END) AS no_speaker_data \
         FROM audio_segments WHERE 1 = 1",
    );
    if let Some(range) = range {
        query.push(" AND started_at <= ");
        query.push_bind(range.end_at.clone());
        query.push(" AND ended_at >= ");
        query.push_bind(range.start_at.clone());
    }
    let row = query.build().fetch_one(infra.read_pool()).await?;
    Ok(BrokerSpeakerCoverage {
        // `SUM` over no rows is NULL, not 0.
        recordings_with_unnamed_voices: row
            .get::<Option<i64>, _>("unnamed_voices")
            .unwrap_or(0)
            .max(0) as u32,
        recordings_without_speaker_data: row
            .get::<Option<i64>, _>("no_speaker_data")
            .unwrap_or(0)
            .max(0) as u32,
    })
}

pub(super) async fn broker_speakers_for_audio(
    infra: &AppInfra,
    audio_segment_id: i64,
    grant_id: Option<&str>,
    secret: &[u8],
) -> Result<(Vec<BrokerSpeaker>, Vec<BrokerSpeakerTurn>)> {
    // A person the user vetoed for this cluster is already off the row: every write
    // path that records the veto clears the matching guess with it.
    let turns = infra
        .processing
        .list_speaker_turns_for_audio_segment(audio_segment_id)
        .await?;
    if turns.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let names: HashMap<i64, String> = infra
        .processing
        .list_person_profiles()
        .await?
        .into_iter()
        .map(|person| (person.id, person.display_name))
        .collect();
    Ok(broker_collapse_speakers(turns, &names, grant_id, secret))
}

/// Collapses an audio segment's speaker turns (already ordered by start) to one
/// entry per **person**, falling back to one per cluster for voices we cannot
/// name. A user assignment wins over a recognition suggestion, and a person id we
/// can't name is treated as unknown rather than a nameless claim.
///
/// Person, not cluster, because over-clustering routinely splits one voice across
/// several clusters — this pipeline's documented ceiling. Publishing each of them
/// makes an agent answering "who was in this meeting?" invent a participant.
/// Unnamed clusters stay separate: there is no identity to merge them on.
///
/// Also returns the attributed turns, each pointing at its speaker's index — the
/// index this collapse already had to build. Emitting it keeps collapse the single
/// authority on identity instead of letting callers re-derive who is who.
pub(super) fn broker_collapse_speakers(
    turns: Vec<crate::SpeakerTurnView>,
    names: &HashMap<i64, String>,
    grant_id: Option<&str>,
    secret: &[u8],
) -> (Vec<BrokerSpeaker>, Vec<BrokerSpeakerTurn>) {
    #[derive(PartialEq, Eq, Hash)]
    enum SpeakerKey {
        Person(i64),
        Cluster(i64),
    }
    let mut index_by_key: HashMap<SpeakerKey, usize> = HashMap::new();
    let mut speakers: Vec<BrokerSpeaker> = Vec::new();
    let mut attributed: Vec<BrokerSpeakerTurn> = Vec::new();
    for turn in turns {
        let assigned = turn
            .person_id
            .and_then(|id| names.get(&id).map(|name| (id, name)));
        let recognized = turn
            .suggested_person_id
            .and_then(|id| names.get(&id).map(|name| (id, name)));
        let (key, speaker) = match (assigned, recognized) {
            (Some((id, name)), _) => (
                SpeakerKey::Person(id),
                BrokerSpeaker {
                    name: Some(name.clone()),
                    attribution: "assigned".to_string(),
                    confidence: None,
                    handle: person_handle(id, grant_id, secret),
                },
            ),
            // An assignment we cannot name still overrides recognition: the user
            // said this voice is someone else, so fall through to `unknown`
            // rather than publishing the guess they overrode.
            (None, Some((id, name))) if turn.person_id.is_none() => (
                SpeakerKey::Person(id),
                BrokerSpeaker {
                    name: Some(name.clone()),
                    attribution: "recognized".to_string(),
                    confidence: turn.recognition_confidence,
                    handle: person_handle(id, grant_id, secret),
                },
            ),
            _ => (
                SpeakerKey::Cluster(turn.cluster_id),
                BrokerSpeaker {
                    name: None,
                    attribution: "unknown".to_string(),
                    confidence: None,
                    handle: voice_handle(
                        turn.cluster_id,
                        Some((turn.start_ms, turn.end_ms)),
                        grant_id,
                        secret,
                    ),
                },
            ),
        };
        let index = match index_by_key.get(&key) {
            // The same person confirmed on one cluster and merely guessed on
            // another is settled: publish what the user decided, not the guess.
            Some(&at) => {
                if speaker.attribution == "assigned" {
                    speakers[at] = speaker;
                }
                // A voice handle only means anything over the span it was heard
                // in, so it grows with every further turn of the same cluster.
                if let Some(end_ms) = speakers[at].handle.end_ms {
                    speakers[at].handle.end_ms = Some(end_ms.max(turn.end_ms));
                }
                at
            }
            None => {
                let at = speakers.len();
                index_by_key.insert(key, at);
                speakers.push(speaker);
                at
            }
        };
        // Turns with no words are dropped rather than published as empty text: an
        // agent reads `""` as "said nothing" (`user_context/capture_source.rs`
        // filters the same way). The overlay is allowed to cover less than `text`.
        if let Some(text) = turn
            .transcript_text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            attributed.push(BrokerSpeakerTurn {
                speaker: Some(index),
                start_ms: turn.start_ms,
                end_ms: turn.end_ms,
                text: text.to_string(),
            });
        }
    }
    (speakers, attributed)
}

fn person_handle(person_id: i64, grant_id: Option<&str>, secret: &[u8]) -> BrokerSpeakerHandle {
    BrokerSpeakerHandle {
        id: encode_speaker_handle(SPEAKER_HANDLE_KIND_PERSON, person_id, grant_id, secret),
        kind: SPEAKER_HANDLE_KIND_PERSON.to_string(),
        start_ms: None,
        end_ms: None,
    }
}

/// `span` is the stretch this voice was heard over WITHIN ONE RECORDING, in ms
/// from that recording's start. `None` where the caller spans several — the
/// offsets share no origin then, and a bound stitched across them is a lie.
fn voice_handle(
    cluster_id: i64,
    span: Option<(u64, u64)>,
    grant_id: Option<&str>,
    secret: &[u8],
) -> BrokerSpeakerHandle {
    BrokerSpeakerHandle {
        id: encode_speaker_handle(SPEAKER_HANDLE_KIND_VOICE, cluster_id, grant_id, secret),
        kind: SPEAKER_HANDLE_KIND_VOICE.to_string(),
        start_ms: span.map(|(start_ms, _)| start_ms),
        end_ms: span.map(|(_, end_ms)| end_ms),
    }
}

/// Signed like a capture reference (same secret, same grant binding) but in its
/// own kind space: the payload leads with `s`, which `decode_opaque_payload`
/// rejects outright, so a speaker handle handed to `show-text` or `open` fails to
/// decode instead of resolving to whatever frame shares its number. A person is
/// not captured content and must never be addressable as if it were.
fn encode_speaker_handle(kind: &str, id: i64, grant_id: Option<&str>, secret: &[u8]) -> String {
    let tag = if kind == SPEAKER_HANDLE_KIND_PERSON {
        'p'
    } else {
        'v'
    };
    let mut payload = format!("s{tag}{:x}", id.max(0));
    if let Some(grant_id) = grant_id {
        payload.push_str(":g");
        payload.push_str(grant_id);
    }
    let signature = opaque_signature(&payload, secret);
    format!("{payload}.{signature}")
}

/// A handle the broker itself signed, decoded back to the row it addresses.
/// `grant_id` is the grant it was issued under — callers scope by it exactly as
/// they do for capture references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DecodedSpeakerHandle {
    pub kind: &'static str,
    pub id: i64,
    pub grant_id: Option<String>,
}

pub(super) fn decode_speaker_handle(value: &str, secret: &[u8]) -> Option<DecodedSpeakerHandle> {
    let (payload, signature) = value.split_once('.')?;
    if signature.len() != OPAQUE_SIGNATURE_HEX_LEN
        || !signature.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    if !opaque_signature_matches(payload, signature, secret) {
        return None;
    }
    let (payload, grant_id) = payload
        .split_once(":g")
        .map_or((payload, None), |(payload, grant_id)| {
            (payload, Some(grant_id.to_string()))
        });
    let mut chars = payload.chars();
    if chars.next()? != 's' {
        return None;
    }
    let kind = match chars.next()? {
        'p' => SPEAKER_HANDLE_KIND_PERSON,
        'v' => SPEAKER_HANDLE_KIND_VOICE,
        _ => return None,
    };
    let id = i64::from_str_radix(chars.as_str(), 16).ok()?;
    Some(DecodedSpeakerHandle { kind, id, grant_id })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::super::*;
    use super::*;
    use crate::SpeakerTurnView;

    const SECRET: &[u8] = b"broker-opaque-secret-for-speaker-handles";

    fn turn(cluster_id: i64, person_id: Option<i64>, suggested: Option<i64>) -> SpeakerTurnView {
        SpeakerTurnView {
            id: cluster_id,
            audio_segment_id: 1,
            session_id: "s".to_string(),
            cluster_id,
            segment_cluster_id: None,
            provider_cluster_id: "0".to_string(),
            speaker_label: format!("Speaker {cluster_id}"),
            person_id,
            suggested_person_id: suggested,
            recognition_confidence: suggested.map(|_| "high".to_string()),
            recognition_score: None,
            start_ms: 0,
            end_ms: 1000,
            transcript_text: None,
            overlaps: false,
        }
    }

    fn collapse(turns: Vec<SpeakerTurnView>, names: &HashMap<i64, String>) -> Vec<BrokerSpeaker> {
        broker_collapse_speakers(turns, names, Some("grant-1"), SECRET).0
    }

    /// A timeline page is `limit` recordings, so the read behind it must be
    /// `limit` recordings of transcript — not every word the speaker ever spoke
    /// inside the grant. Retention defaults to **never**, so "inside the grant" on
    /// an All Retained History (or 30-day) timeline is months of continuous
    /// capture: at the real shape (5-minute segments, tens of thousands of
    /// `audio_segments` rows) an unbounded read materializes thousands of turns
    /// and megabytes of transcript to publish twenty of them.
    ///
    /// Counts ROWS, not milliseconds: the defect is work proportional to history
    /// where the page is the bound, and that is deterministic.
    #[test]
    fn a_timeline_page_reads_at_most_a_page_of_recordings() {
        const SEGMENTS: i64 = 400;
        const LIMIT: u32 = 20;

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");
        runtime.block_on(async {
            let save_dir = std::env::temp_dir().join(format!(
                "mnema-speaker-page-bound-{}-{}",
                std::process::id(),
                now_unix_ms()
            ));
            let _ = fs::remove_dir_all(&save_dir);
            let infra = AppInfra::initialize(&save_dir)
                .await
                .expect("infra should initialize");
            let priya = infra
                .create_person_profile("Priya", None)
                .await
                .expect("person profile should insert");

            // One recording every 5 minutes (the capture cap), every one of them
            // hers, each carrying a sentence of transcript.
            sqlx::query(
                "INSERT INTO audio_segments \
                    (id, source_kind, source_session_id, segment_index, file_path, started_at, ended_at) \
                 WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < ?1) \
                 SELECT n, 'microphone', 'sess-1', n, '/tmp/a' || n || '.m4a', \
                        strftime('%Y-%m-%dT%H:%M:%SZ', julianday('2026-01-01T00:00:00Z') + n * 300.0 / 86400.0), \
                        strftime('%Y-%m-%dT%H:%M:%SZ', julianday('2026-01-01T00:00:00Z') + (n * 300.0 + 300.0) / 86400.0) \
                 FROM seq",
            )
            .bind(SEGMENTS)
            .execute(infra.pool())
            .await
            .expect("segments should insert");
            sqlx::query(
                "INSERT INTO recording_speaker_clusters \
                    (id, session_id, provider, provider_cluster_id, stable_label, person_id) \
                 VALUES (1, 'sess-1', 'mock', 'speaker_00', 'Unknown 0', ?1)",
            )
            .bind(priya.id)
            .execute(infra.pool())
            .await
            .expect("cluster should insert");
            sqlx::query(
                "INSERT INTO speaker_turns \
                    (audio_segment_id, session_id, cluster_id, start_ms, end_ms, transcript_text) \
                 SELECT id, 'sess-1', 1, 0, 25000, \
                        'this is roughly what one diarized turn of speech looks like written out in full' \
                 FROM audio_segments",
            )
            .execute(infra.pool())
            .await
            .expect("turns should insert");

            let matched = speaker_matched_turns_in_range(
                &infra,
                &SearchSpeakerRefinement::Person(priya.id),
                &SearchDateRangeRefinement {
                    start_at: "2026-01-01T00:00:00Z".to_string(),
                    end_at: "2027-01-01T00:00:00Z".to_string(),
                    origin: None,
                },
                LIMIT,
            )
            .await
            .expect("matched turns should read");

            let transcript_bytes: usize = matched
                .values()
                .flatten()
                .map(|turn| turn.text.len())
                .sum();
            assert!(
                matched.len() <= LIMIT as usize,
                "a {LIMIT}-interval page read {} of {SEGMENTS} recordings \
                 ({transcript_bytes} bytes of transcript) to publish {LIMIT}",
                matched.len()
            );
            // The page it did read is the page the timeline publishes: the first
            // `limit` matches of `list_overlapping_range`'s own ordering.
            let mut published: Vec<i64> = matched.keys().copied().collect();
            published.sort_unstable();
            assert_eq!(published, (1..=i64::from(LIMIT)).collect::<Vec<_>>());

            let _ = fs::remove_dir_all(&save_dir);
        });
    }

    /// The user assigned this cluster to a person we can no longer name (the
    /// profile went away between the turn read and the profile read). A stale
    /// recognition suggestion for a DIFFERENT person must not fill the gap: the
    /// assignment is the user overriding recognition, so the honest answer is
    /// "unknown", never "this was Bo".
    #[test]
    fn an_unnameable_assignment_never_falls_back_to_the_recognition_suggestion() {
        let names = HashMap::from([(9, "Bo".to_string())]);

        let speakers = collapse(vec![turn(1, Some(404), Some(9))], &names);

        assert_eq!(speakers.len(), 1);
        assert_eq!(speakers[0].name, None);
        assert_eq!(speakers[0].attribution, "unknown");
        assert_eq!(speakers[0].confidence, None);
        assert_eq!(speakers[0].handle.kind, SPEAKER_HANDLE_KIND_VOICE);
    }

    /// One entry per person, not per cluster: over-clustering splits a voice
    /// across clusters routinely, and an agent asked "who was in this meeting?"
    /// must not count Ada twice because her voice landed in two of them.
    #[test]
    fn collapses_to_one_entry_per_person_and_never_leaks_internal_labels() {
        let names = HashMap::from([
            (7, "Ada".to_string()),
            (9, "Bo".to_string()),
            (11, "Cy".to_string()),
        ]);
        let speakers = collapse(
            vec![
                turn(1, Some(7), None),
                turn(1, Some(7), None),
                turn(2, None, Some(9)),
                turn(3, None, None),
                // Assignment wins over a competing recognition suggestion, and Ada
                // is already published from cluster 1 — one voice, one entry.
                turn(4, Some(7), Some(9)),
                // A person id with no profile row must not become a nameless claim,
                // and two of them stay separate: there is no identity to merge on.
                turn(5, Some(404), None),
                // Guessed on one cluster, confirmed on another: the user settled it.
                turn(6, None, Some(11)),
                turn(7, Some(11), None),
            ],
            &names,
        );

        let shape: Vec<_> = speakers
            .iter()
            .map(|s| {
                (
                    s.name.as_deref(),
                    s.attribution.as_str(),
                    s.confidence.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                (Some("Ada"), "assigned", None),
                (Some("Bo"), "recognized", Some("high")),
                (None, "unknown", None),
                (None, "unknown", None),
                (Some("Cy"), "assigned", None),
            ]
        );
        let json = serde_json::to_string(&speakers).expect("serializes");
        assert!(
            !json.contains("Speaker "),
            "internal labels must not leak: {json}"
        );
    }

    /// The wire shape a second crate re-serializes: `crates/cli/src/main.rs`
    /// embeds `Vec<BrokerSpeaker>` in its stdout JSON and `crates/cli/src/mcp.rs`
    /// documents it to every MCP client. Pins the emitted key names, the omitted
    /// `confidence`/`speakers` keys, and the `#[serde(default)]` that keeps a
    /// payload from an older client decoding.
    #[test]
    fn broker_speaker_wire_shape_round_trips() {
        let recognized = BrokerSpeaker {
            name: Some("Ada".to_string()),
            attribution: "recognized".to_string(),
            confidence: Some("high".to_string()),
            handle: BrokerSpeakerHandle {
                id: "sp7.sig".to_string(),
                kind: SPEAKER_HANDLE_KIND_PERSON.to_string(),
                start_ms: None,
                end_ms: None,
            },
        };
        assert_eq!(
            serde_json::to_value(&recognized).expect("speaker should serialize"),
            serde_json::json!({
                "name": "Ada",
                "attribution": "recognized",
                "confidence": "high",
                "handle": {"id": "sp7.sig", "kind": "person"},
            })
        );

        let unknown = BrokerSpeaker {
            name: None,
            attribution: "unknown".to_string(),
            confidence: None,
            handle: BrokerSpeakerHandle {
                id: "sv3.sig".to_string(),
                kind: SPEAKER_HANDLE_KIND_VOICE.to_string(),
                start_ms: Some(0),
                end_ms: Some(1_000),
            },
        };
        assert_eq!(
            serde_json::to_value(&unknown).expect("speaker should serialize"),
            serde_json::json!({
                "name": null,
                "attribution": "unknown",
                "handle": {"id": "sv3.sig", "kind": "voice", "startMs": 0, "endMs": 1000},
            })
        );
        assert_eq!(
            serde_json::from_value::<BrokerSpeaker>(
                serde_json::to_value(&unknown).expect("speaker should serialize")
            )
            .expect("speaker should decode"),
            unknown
        );

        let response = BrokerShowTextResponse {
            opaque_id: "op-1".to_string(),
            kind: "audio_microphone".to_string(),
            text: "hello".to_string(),
            speakers: Vec::new(),
            turns: Vec::new(),
        };
        assert_eq!(
            serde_json::to_value(&response).expect("response should serialize"),
            serde_json::json!({"opaqueId": "op-1", "kind": "audio_microphone", "text": "hello"})
        );

        let legacy: BrokerShowTextResponse = serde_json::from_value(
            serde_json::json!({"opaqueId": "op-1", "kind": "audio_microphone", "text": "hello"}),
        )
        .expect("a payload without speakers should decode");
        assert!(legacy.speakers.is_empty());
        assert!(legacy.turns.is_empty());
    }

    /// The `speakers` wire shape as BOTH published agent contracts describe it:
    /// `.agents/skills/mnema-data/SKILL.md` — "`data.speakers[]` contains `name`
    /// (absent for an unnamed voice)" — and `crates/cli/CONTEXT.md` — "Each entry
    /// returns `name` (absent for an unnamed voice)". The CLI serializes this
    /// response verbatim, so what this type omits is what an agent sees.
    #[test]
    fn an_unnamed_voice_summary_omits_the_name_key() {
        let unnamed = BrokerSpeakerSummary {
            name: None,
            handle: BrokerSpeakerHandle {
                id: "sv3.sig".to_string(),
                kind: SPEAKER_HANDLE_KIND_VOICE.to_string(),
                start_ms: None,
                end_ms: None,
            },
            speaking_ms: 4_000,
            assigned_turns: 0,
            recognized_turns: 0,
        };
        assert_eq!(
            serde_json::to_value(&unnamed).expect("summary should serialize"),
            serde_json::json!({
                "handle": {"id": "sv3.sig", "kind": "voice"},
                "speakingMs": 4_000,
                "assignedTurns": 0,
                "recognizedTurns": 0,
            })
        );
        // A named person still reports the name it was found under.
        let named = BrokerSpeakerSummary {
            name: Some("Ada".to_string()),
            ..unnamed.clone()
        };
        assert_eq!(
            serde_json::to_value(&named).expect("summary should serialize")["name"],
            serde_json::json!("Ada")
        );
        // And a payload without the key still decodes, so the omission is not a
        // one-way door for anything that round-trips this response.
        assert_eq!(
            serde_json::from_value::<BrokerSpeakerSummary>(
                serde_json::to_value(&unnamed).expect("summary should serialize")
            )
            .expect("a summary without a name should decode"),
            unnamed
        );
    }

    /// A handle is only useful if the broker can read back the exact row it
    /// addressed — a person handle must not decode as a voice handle, and neither
    /// may lose the grant it was issued under.
    #[test]
    fn speaker_handles_round_trip_to_their_person_or_cluster() {
        let person = encode_speaker_handle(SPEAKER_HANDLE_KIND_PERSON, 7, Some("grant-1"), SECRET);
        let voice = encode_speaker_handle(SPEAKER_HANDLE_KIND_VOICE, 7, Some("grant-1"), SECRET);

        assert_ne!(
            person, voice,
            "a person and a cluster sharing a row id must not share a handle"
        );
        assert_eq!(
            decode_speaker_handle(&person, SECRET),
            Some(DecodedSpeakerHandle {
                kind: SPEAKER_HANDLE_KIND_PERSON,
                id: 7,
                grant_id: Some("grant-1".to_string()),
            })
        );
        assert_eq!(
            decode_speaker_handle(&voice, SECRET),
            Some(DecodedSpeakerHandle {
                kind: SPEAKER_HANDLE_KIND_VOICE,
                id: 7,
                grant_id: Some("grant-1".to_string()),
            })
        );
        assert_eq!(
            decode_speaker_handle(&person, b"a different broker secret"),
            None,
            "a handle this broker never signed must not decode"
        );
    }

    /// A named person and an unnamed voice must be told apart from the wire
    /// alone: an agent that reads a `voice` handle as a person will merge two
    /// strangers, or claim the same human across recordings it cannot span.
    #[test]
    fn named_and_unnamed_speakers_carry_handles_the_wire_can_tell_apart() {
        let names = HashMap::from([(7, "Ada".to_string())]);

        let speakers = collapse(vec![turn(1, Some(7), None), turn(2, None, None)], &names);

        let json = serde_json::to_value(&speakers).expect("speakers should serialize");
        assert_eq!(json[0]["handle"]["kind"], "person");
        assert_eq!(json[1]["handle"]["kind"], "voice");
        assert!(
            json[0]["handle"]["startMs"].is_null(),
            "a person handle spans sessions, so it carries no recording bounds: {json}"
        );
        // The voice handle is only meaningful over the span it was heard in.
        assert_eq!(json[1]["handle"]["startMs"], 0);
        assert_eq!(json[1]["handle"]["endMs"], 1000);
        assert_ne!(json[0]["handle"]["id"], json[1]["handle"]["id"]);
        assert_eq!(
            decode_speaker_handle(json[0]["handle"]["id"].as_str().expect("handle id"), SECRET)
                .expect("person handle should decode")
                .id,
            7,
            "the person handle addresses the profile, not the cluster it was heard on"
        );
        assert_eq!(
            decode_speaker_handle(json[1]["handle"]["id"].as_str().expect("handle id"), SECRET)
                .expect("voice handle should decode")
                .id,
            2,
            "the voice handle addresses the cluster row"
        );
    }
}
