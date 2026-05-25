use std::path::Path;

use sqlx::{Sqlite, Transaction};

use crate::{
    hidden_segment_workspace::HiddenSegmentWorkspacePaths,
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

#[derive(Clone, Copy)]
enum CapturedFrameEquivalenceMatchKind {
    NearestEarlier,
    EarliestEarlier,
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
        self.find_equivalent_frame(
            frame,
            scope,
            CapturedFrameEquivalenceMatchKind::NearestEarlier,
        )
        .await
    }

    pub async fn find_earliest_earlier_equivalent_frame(
        &self,
        frame: &Frame,
        scope: &CapturedFrameEquivalenceScope,
    ) -> Result<Option<Frame>> {
        self.find_equivalent_frame(
            frame,
            scope,
            CapturedFrameEquivalenceMatchKind::EarliestEarlier,
        )
        .await
    }

    async fn find_equivalent_frame(
        &self,
        frame: &Frame,
        scope: &CapturedFrameEquivalenceScope,
        match_kind: CapturedFrameEquivalenceMatchKind,
    ) -> Result<Option<Frame>> {
        let mut transaction = self.processing.begin_transaction().await?;
        let resolved = self
            .find_equivalent_frame_in_transaction(&mut transaction, frame, scope, match_kind)
            .await?;
        transaction.commit().await?;
        Ok(resolved)
    }

    async fn find_equivalent_frame_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        frame: &Frame,
        scope: &CapturedFrameEquivalenceScope,
        match_kind: CapturedFrameEquivalenceMatchKind,
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

        Ok(select_equivalent_proof_match(
            proof,
            version,
            earlier_frames,
            match_kind,
        ))
    }

    /// Resolves the nearest earlier equivalent **Captured Frame** that satisfies OCR Fallback
    /// Eligibility: it already has a non-failed **OCR Job**. An equivalent frame that was
    /// admission-skipped (no job, no text) is intentionally ignored so it cannot suppress
    /// admission of a later frame. Used by the **Captured Frame Pipeline** admission gate.
    pub(crate) async fn find_nearest_earlier_ocr_eligible_equivalent_frame_in_transaction(
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
            .list_earlier_ocr_eligible_frames_with_equivalence_hint_in_scope_in_transaction(
                transaction,
                &frame.session_id,
                frame.id,
                equivalence_hint,
                scope.workspace_prefix(),
            )
            .await?;

        Ok(select_equivalent_proof_match(
            proof,
            version,
            earlier_frames,
            CapturedFrameEquivalenceMatchKind::NearestEarlier,
        ))
    }

    pub async fn find_nearest_earlier_ocr_eligible_equivalent_frame(
        &self,
        frame: &Frame,
        scope: &CapturedFrameEquivalenceScope,
    ) -> Result<Option<Frame>> {
        let mut transaction = self.processing.begin_transaction().await?;
        let resolved = self
            .find_nearest_earlier_ocr_eligible_equivalent_frame_in_transaction(
                &mut transaction,
                frame,
                scope,
            )
            .await?;
        transaction.commit().await?;
        Ok(resolved)
    }

    pub async fn get_frame_and_find_nearest_earlier_equivalent_frame(
        &self,
        frame_id: i64,
        scope: &CapturedFrameEquivalenceScope,
    ) -> Result<Option<Frame>> {
        self.get_frame_and_find_equivalent_frame(
            frame_id,
            Some(scope),
            CapturedFrameEquivalenceMatchKind::NearestEarlier,
        )
        .await
    }

    pub async fn get_frame_and_find_earliest_earlier_equivalent_frame(
        &self,
        frame_id: i64,
        scope: &CapturedFrameEquivalenceScope,
    ) -> Result<Option<Frame>> {
        self.get_frame_and_find_equivalent_frame(
            frame_id,
            Some(scope),
            CapturedFrameEquivalenceMatchKind::EarliestEarlier,
        )
        .await
    }

    pub(crate) async fn get_frame_and_find_nearest_earlier_equivalent_frame_in_default_scope(
        &self,
        frame_id: i64,
    ) -> Result<Option<Frame>> {
        self.get_frame_and_find_equivalent_frame(
            frame_id,
            None,
            CapturedFrameEquivalenceMatchKind::NearestEarlier,
        )
        .await
    }

    pub(crate) async fn get_frame_and_find_earliest_earlier_equivalent_frame_in_default_scope(
        &self,
        frame_id: i64,
    ) -> Result<Option<Frame>> {
        self.get_frame_and_find_equivalent_frame(
            frame_id,
            None,
            CapturedFrameEquivalenceMatchKind::EarliestEarlier,
        )
        .await
    }

    async fn get_frame_and_find_equivalent_frame(
        &self,
        frame_id: i64,
        scope: Option<&CapturedFrameEquivalenceScope>,
        match_kind: CapturedFrameEquivalenceMatchKind,
    ) -> Result<Option<Frame>> {
        let Some(frame) = self.processing.get_frame(frame_id).await? else {
            return Ok(None);
        };

        let resolved_scope = scope
            .cloned()
            .unwrap_or_else(|| CapturedFrameEquivalenceScope::from_frame(&frame));
        self.find_equivalent_frame(&frame, &resolved_scope, match_kind)
            .await
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

/// Returns the first `earlier_frame` whose equivalence proof matches the candidate's
/// `(proof, version)`, honoring quarantine and version guards. Ordering follows `match_kind`;
/// the candidate list is assumed to already be ordered newest-first.
fn select_equivalent_proof_match(
    proof: &[u8],
    version: i64,
    earlier_frames: Vec<Frame>,
    match_kind: CapturedFrameEquivalenceMatchKind,
) -> Option<Frame> {
    let earlier_frames: Vec<Frame> = match match_kind {
        CapturedFrameEquivalenceMatchKind::NearestEarlier => earlier_frames,
        CapturedFrameEquivalenceMatchKind::EarliestEarlier => {
            earlier_frames.into_iter().rev().collect()
        }
    };

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

        if capture_screen::captured_frame_equivalence_proofs_match(version, proof, earlier_proof) {
            return Some(earlier_frame);
        }
    }

    None
}

#[cfg(test)]
mod tests;
