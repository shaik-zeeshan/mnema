use std::collections::HashMap;

use super::BrokerSpeaker;
use crate::{AppInfra, Result};

pub(super) async fn broker_speakers_for_audio(
    infra: &AppInfra,
    audio_segment_id: i64,
) -> Result<Vec<BrokerSpeaker>> {
    // A person the user vetoed for this cluster is already off the row: every write
    // path that records the veto clears the matching guess with it.
    let turns = infra
        .processing
        .list_speaker_turns_for_audio_segment(audio_segment_id)
        .await?;
    if turns.is_empty() {
        return Ok(Vec::new());
    }
    let names: HashMap<i64, String> = infra
        .processing
        .list_person_profiles()
        .await?
        .into_iter()
        .map(|person| (person.id, person.display_name))
        .collect();
    Ok(broker_collapse_speakers(turns, &names))
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
pub(super) fn broker_collapse_speakers(
    turns: Vec<crate::SpeakerTurnView>,
    names: &HashMap<i64, String>,
) -> Vec<BrokerSpeaker> {
    #[derive(PartialEq, Eq, Hash)]
    enum SpeakerKey {
        Person(i64),
        Cluster(i64),
    }
    let mut index_by_key: HashMap<SpeakerKey, usize> = HashMap::new();
    let mut speakers: Vec<BrokerSpeaker> = Vec::new();
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
                },
            ),
            _ => (
                SpeakerKey::Cluster(turn.cluster_id),
                BrokerSpeaker {
                    name: None,
                    attribution: "unknown".to_string(),
                    confidence: None,
                },
            ),
        };
        match index_by_key.get(&key) {
            // The same person confirmed on one cluster and merely guessed on
            // another is settled: publish what the user decided, not the guess.
            Some(&at) if speaker.attribution == "assigned" => speakers[at] = speaker,
            Some(_) => {}
            None => {
                index_by_key.insert(key, speakers.len());
                speakers.push(speaker);
            }
        }
    }
    speakers
}
