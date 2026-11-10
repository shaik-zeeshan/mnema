//! What an answer's sources read beyond the cited facts: chip thumbnails, the
//! frame peek (size, image, OCR) and the audio receipt's frame strip. Sync, run
//! on the background executor. `CoreSources` hands each call to the surface that
//! already reads that app-core command; tests serve a fixture.

use std::path::PathBuf;
use std::sync::Arc;

use app_core::{AppCore, AppInfraState};

use crate::quick_recall::model::Preview;
use crate::timeline::model::OcrBox;

pub type Res<T> = Result<T, String>;

pub trait SourceBackend: Send + Sync + 'static {
    /// `get_frame_scrub_previews` at its 200 px default: never a full-size frame.
    fn thumbs(&self, _ids: &[i64]) -> Vec<Preview> {
        Vec::new()
    }
    /// `get_frame`: the frame's pixel size. `Ok(None)` = the frame is gone.
    fn frame(&self, _id: i64) -> Res<Option<(Option<u32>, Option<u32>)>> {
        Err("no app core".into())
    }
    /// `get_frame_preview`: the image the peek and the receipt paint.
    fn preview(&self, _id: i64) -> Option<PathBuf> {
        None
    }
    /// `loadOcrForFrame`: `None` = never OCR'd.
    fn ocr(&self, _id: i64) -> Res<Option<Vec<OcrBox>>> {
        Ok(None)
    }
    /// `list_frame_summaries_in_range`: `(id, captured ms)`, the receipt's strip.
    fn frames(&self, _start_ms: i64, _end_ms: i64) -> Res<Vec<(i64, i64)>> {
        Err("no app core".into())
    }
}

/// No `AppCore` (a bare view).
pub struct NoSources;

impl SourceBackend for NoSources {}

pub struct CoreSources(pub Arc<AppCore>);

impl SourceBackend for CoreSources {
    fn thumbs(&self, ids: &[i64]) -> Vec<Preview> {
        use crate::quick_recall::data::Backend as _;
        crate::quick_recall::data::CoreBackend(self.0.clone()).previews(ids, None)
    }

    fn frame(&self, id: i64) -> Res<Option<(Option<u32>, Option<u32>)>> {
        use app_core::app_infra::{get_frame, GetFrameRequest};
        let infra = self
            .0
            .try_state::<AppInfraState>()
            .ok_or("The capture index isn't open yet.")?;
        let dto = self
            .0
            .block_on(get_frame(GetFrameRequest { frame_id: id }, &infra))?;
        let dim = |v: Option<i64>| v.and_then(|v| u32::try_from(v).ok());
        Ok(dto.map(|f| (dim(f.width), dim(f.height))))
    }

    fn preview(&self, id: i64) -> Option<PathBuf> {
        use crate::insights::data::Backend as _;
        crate::insights::data::CoreBackend(self.0.clone()).frame_preview(id)
    }

    fn ocr(&self, id: i64) -> Res<Option<Vec<OcrBox>>> {
        use crate::timeline::data::Source as _;
        crate::timeline::data::CoreSource(self.0.clone()).ocr(id)
    }

    fn frames(&self, start_ms: i64, end_ms: i64) -> Res<Vec<(i64, i64)>> {
        use crate::insights::data::Backend as _;
        crate::insights::data::CoreBackend(self.0.clone()).frames(start_ms, end_ms)
    }
}
