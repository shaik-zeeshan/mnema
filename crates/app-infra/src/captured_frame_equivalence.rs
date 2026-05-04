use std::path::Path;

use sqlx::{Sqlite, Transaction};

use crate::{
    frame_batches::HiddenSegmentWorkspacePaths,
    processing::{Frame, ProcessingStore},
    Result,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapturedFrameEquivalenceScope {
    Session,
    HiddenSegmentWorkspace { frames_dir_prefix: String },
}

#[derive(Clone)]
pub struct CapturedFrameEquivalenceResolver {
    processing: ProcessingStore,
}

impl CapturedFrameEquivalenceResolver {
    pub(crate) fn new(processing: ProcessingStore) -> Self {
        Self { processing }
    }

    pub async fn find_nearest_earlier_equivalent_frame(
        &self,
        frame: &Frame,
        scope: &CapturedFrameEquivalenceScope,
    ) -> Result<Option<Frame>> {
        let mut transaction = self.processing.begin_transaction().await?;
        let resolved = self
            .find_nearest_earlier_equivalent_frame_in_transaction(&mut transaction, frame, scope)
            .await?;
        transaction.commit().await?;
        Ok(resolved)
    }

    pub(crate) async fn find_nearest_earlier_equivalent_frame_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        frame: &Frame,
        scope: &CapturedFrameEquivalenceScope,
    ) -> Result<Option<Frame>> {
        let Some((equivalence_hint, proof, version)) = frame.equivalence.ready_parts() else {
            return Ok(None);
        };

        let earlier_frames = self
            .processing
            .list_earlier_frames_with_equivalence_hint_in_scope_in_transaction(
                transaction,
                &frame.session_id,
                frame.id,
                equivalence_hint,
                scope.workspace_prefix(),
            )
            .await?;

        for earlier_frame in earlier_frames {
            if earlier_frame.equivalence.is_quarantined() {
                continue;
            }

            let Some((_hint, earlier_proof, earlier_version)) = earlier_frame.equivalence.ready_parts()
            else {
                continue;
            };

            if version != earlier_version {
                continue;
            }

            if capture_screen::captured_frame_equivalence_proofs_match(
                version,
                proof,
                earlier_proof,
            ) {
                return Ok(Some(earlier_frame));
            }
        }

        Ok(None)
    }

    pub async fn get_frame_and_find_nearest_earlier_equivalent_frame(
        &self,
        frame_id: i64,
        scope: &CapturedFrameEquivalenceScope,
    ) -> Result<Option<Frame>> {
        let Some(frame) = self.processing.get_frame(frame_id).await? else {
            return Ok(None);
        };

        self.find_nearest_earlier_equivalent_frame(&frame, scope).await
    }
}

impl CapturedFrameEquivalenceScope {
    pub fn from_frame(frame: &Frame) -> Self {
        Self::from_frame_path(&frame.file_path)
    }

    pub fn from_frame_path(file_path: &str) -> Self {
        HiddenSegmentWorkspacePaths::from_frame_artifact_path(Path::new(file_path))
            .map(|paths| Self::HiddenSegmentWorkspace {
                frames_dir_prefix: format!("{}/", paths.frames_dir),
            })
            .unwrap_or(Self::Session)
    }

    fn workspace_prefix(&self) -> Option<&str> {
        match self {
            Self::Session => None,
            Self::HiddenSegmentWorkspace { frames_dir_prefix } => Some(frames_dir_prefix),
        }
    }
}

#[cfg(test)]
mod tests;
