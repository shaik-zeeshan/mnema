//! The speaker commands: turns, clusters, person profiles and recognition.

use super::*;

pub async fn list_speaker_turns(
    request: ListSpeakerTurnsRequest,
    state: &AppInfraState,
) -> Result<Vec<SpeakerTurnDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .list_speaker_turns_for_audio_segment(request.audio_segment_id)
        .await
        .map(|turns| turns.into_iter().map(SpeakerTurnDto::from).collect())
        .map_err(|error| {
            format!(
                "failed to list speaker turns for audio segment {}: {error}",
                request.audio_segment_id
            )
        })
}

pub async fn list_person_profiles(state: &AppInfraState) -> Result<Vec<PersonProfileDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .list_person_profiles()
        .await
        .map(|profiles| profiles.into_iter().map(PersonProfileDto::from).collect())
        .map_err(|error| format!("failed to list person profiles: {error}"))
}

pub async fn list_searchable_apps(
    state: &AppInfraState,
) -> Result<Vec<::app_infra::SearchableApp>, String> {
    let infra = Arc::clone(&*state);
    infra
        .list_searchable_apps()
        .await
        .map_err(|error| format!("failed to list searchable apps: {error}"))
}

pub async fn create_person_profile(
    request: CreatePersonProfileRequest,
    state: &AppInfraState,
) -> Result<PersonProfileDto, String> {
    let infra = Arc::clone(&*state);
    infra
        .create_person_profile(&request.display_name, request.notes.as_deref())
        .await
        .map(PersonProfileDto::from)
        .map_err(|error| format!("failed to create person profile: {error}"))
}

pub async fn delete_person_profile(
    request: DeletePersonProfileRequest,
    state: &AppInfraState,
) -> Result<(), String> {
    let infra = Arc::clone(&*state);
    infra
        .delete_person_profile(request.person_id)
        .await
        .map_err(|error| {
            format!(
                "failed to delete person profile {}: {error}",
                request.person_id
            )
        })
}

pub async fn list_speaker_clusters(
    request: ListSpeakerClustersRequest,
    state: &AppInfraState,
) -> Result<Vec<SpeakerClusterDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .list_speaker_clusters_for_session(&request.session_id)
        .await
        .map(|clusters| clusters.into_iter().map(SpeakerClusterDto::from).collect())
        .map_err(|error| {
            format!(
                "failed to list speaker clusters for session {}: {error}",
                request.session_id
            )
        })
}

pub async fn name_speaker_cluster(
    request: NameSpeakerClusterRequest,
    state: &AppInfraState,
) -> Result<SpeakerClusterDto, String> {
    let infra = Arc::clone(&*state);
    infra
        .name_speaker_cluster(request.cluster_id, &request.label)
        .await
        .map(SpeakerClusterDto::from)
        .map_err(|error| {
            format!(
                "failed to name speaker cluster {}: {error}",
                request.cluster_id
            )
        })
}

pub async fn link_speaker_cluster_to_person(
    request: LinkSpeakerClusterRequest,
    state: &AppInfraState,
) -> Result<SpeakerClusterDto, String> {
    let infra = Arc::clone(&*state);
    infra
        .link_speaker_cluster_to_person(
            request.cluster_id,
            request.person_id,
            request.add_embedding,
        )
        .await
        .map(SpeakerClusterDto::from)
        .map_err(|error| {
            format!(
                "failed to link speaker cluster {} to person {}: {error}",
                request.cluster_id, request.person_id
            )
        })
}

pub async fn unlink_speaker_cluster_from_person(
    request: SpeakerClusterRequest,
    state: &AppInfraState,
) -> Result<SpeakerClusterDto, String> {
    let infra = Arc::clone(&*state);
    infra
        .unlink_speaker_cluster_from_person(request.cluster_id)
        .await
        .map(SpeakerClusterDto::from)
        .map_err(|error| {
            format!(
                "failed to unlink speaker cluster {} from person: {error}",
                request.cluster_id
            )
        })
}

pub async fn confirm_speaker_recognition_suggestion(
    request: ConfirmSpeakerSuggestionRequest,
    state: &AppInfraState,
) -> Result<SpeakerClusterDto, String> {
    let infra = Arc::clone(&*state);
    infra
        .confirm_speaker_recognition_suggestion(request.cluster_id, request.add_embedding)
        .await
        .map(SpeakerClusterDto::from)
        .map_err(|error| {
            format!(
                "failed to confirm speaker recognition suggestion for cluster {}: {error}",
                request.cluster_id
            )
        })
}

pub async fn reject_speaker_recognition_suggestion(
    request: SpeakerClusterRequest,
    state: &AppInfraState,
) -> Result<SpeakerClusterDto, String> {
    let infra = Arc::clone(&*state);
    infra
        .reject_speaker_recognition_suggestion(request.cluster_id)
        .await
        .map(SpeakerClusterDto::from)
        .map_err(|error| {
            format!(
                "failed to reject speaker recognition suggestion for cluster {}: {error}",
                request.cluster_id
            )
        })
}

pub async fn merge_speaker_clusters(
    request: MergeSpeakerClustersRequest,
    state: &AppInfraState,
) -> Result<SpeakerClusterDto, String> {
    let infra = Arc::clone(&*state);
    infra
        .merge_speaker_clusters(request.source_cluster_id, request.target_cluster_id)
        .await
        .map(SpeakerClusterDto::from)
        .map_err(|error| {
            format!(
                "failed to merge speaker cluster {} into {}: {error}",
                request.source_cluster_id, request.target_cluster_id
            )
        })
}

pub async fn move_speaker_turn_to_cluster(
    request: MoveSpeakerTurnRequest,
    state: &AppInfraState,
) -> Result<SpeakerTurnDto, String> {
    let infra = Arc::clone(&*state);
    infra
        .move_speaker_turn_to_cluster(request.turn_id, request.target_cluster_id)
        .await
        .map(SpeakerTurnDto::from)
        .map_err(|error| {
            format!(
                "failed to move speaker turn {} to cluster {}: {error}",
                request.turn_id, request.target_cluster_id
            )
        })
}
