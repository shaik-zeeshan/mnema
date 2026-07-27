use std::collections::HashMap;

use super::{
    opaque_signature, opaque_signature_matches, BrokerSpeaker, BrokerSpeakerHandle,
    BrokerSpeakerTurn, OPAQUE_SIGNATURE_HEX_LEN,
};
use crate::{AppInfra, Result};

/// A person profile: stable across sessions, channels, and renames.
pub(super) const SPEAKER_HANDLE_KIND_PERSON: &str = "person";
/// One voice inside one recording: session-scoped, fragmenting, and dead on
/// re-diarization. Marked apart from `person` on the wire because an agent that
/// cannot tell them apart will treat a voice as a human being.
pub(super) const SPEAKER_HANDLE_KIND_VOICE: &str = "voice";

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
                        turn.start_ms,
                        turn.end_ms,
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
                speaker: index,
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

fn voice_handle(
    cluster_id: i64,
    start_ms: u64,
    end_ms: u64,
    grant_id: Option<&str>,
    secret: &[u8],
) -> BrokerSpeakerHandle {
    BrokerSpeakerHandle {
        id: encode_speaker_handle(SPEAKER_HANDLE_KIND_VOICE, cluster_id, grant_id, secret),
        kind: SPEAKER_HANDLE_KIND_VOICE.to_string(),
        start_ms: Some(start_ms),
        end_ms: Some(end_ms),
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
// Kept beside the encoder so the two halves cannot drift; the speaker filter is
// the first caller outside the round-trip test.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DecodedSpeakerHandle {
    pub kind: &'static str,
    pub id: i64,
    pub grant_id: Option<String>,
}

#[allow(dead_code)]
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
    Some(DecodedSpeakerHandle {
        kind,
        id,
        grant_id,
    })
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
            .map(|s| (s.name.as_deref(), s.attribution.as_str(), s.confidence.as_deref()))
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
        assert!(!json.contains("Speaker "), "internal labels must not leak: {json}");
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
