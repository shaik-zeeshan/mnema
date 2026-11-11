//! What a turn reads beyond the stream: chip thumbnails, the frame peek (size,
//! image, OCR), the audio receipt's frame strip, app icons for tool rows and
//! the timeline block, and the inline ChatGPT sign-in a failed turn offers.
//! Sync, run on the background executor. `CoreSources` hands each call to the surface that
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
    /// What the peek shows beside the image: the preview's secret-redaction
    /// count and the frame's app bundle id (for its icon).
    fn peek_extras(&self, _id: i64) -> (u32, Option<String>) {
        (0, None)
    }
    /// `get_frame`'s `(app, window title, captured at ISO, url)`: the peek's
    /// meta when its source didn't carry them.
    fn frame_meta(
        &self,
        _id: i64,
    ) -> Option<(Option<String>, Option<String>, String, Option<String>)> {
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
    /// `resolve_app_icons`: (requested name or bundle id, icon PNG); a miss is absent.
    fn app_icons(&self, _apps: &[String]) -> Vec<(String, PathBuf)> {
        Vec::new()
    }
    /// `ai_runtime_chatgpt_begin_login`: (user code, verify URL).
    fn chatgpt_begin(&self, _provider: &str) -> Res<(String, String)> {
        Err("no app core".into())
    }
    /// `ai_runtime_chatgpt_cancel_login`.
    fn chatgpt_cancel(&self, _provider: &str) {}
    /// The saved settings (the MCP warm-up gate reads them).
    fn settings(&self) -> Option<capture_types::RecordingSettings> {
        None
    }
    /// `mcp_warm_connectors`.
    fn warm_mcp(&self) {}
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

    fn frame_meta(
        &self,
        id: i64,
    ) -> Option<(Option<String>, Option<String>, String, Option<String>)> {
        use app_core::app_infra::{get_frame, GetFrameRequest};
        let infra = self.0.try_state::<AppInfraState>()?;
        let f = self
            .0
            .block_on(get_frame(GetFrameRequest { frame_id: id }, &infra))
            .ok()??;
        Some((f.app_name, f.window_title, f.captured_at, f.url))
    }

    fn preview(&self, id: i64) -> Option<PathBuf> {
        use crate::insights::data::Backend as _;
        crate::insights::data::CoreBackend(self.0.clone()).frame_preview(id)
    }

    fn peek_extras(&self, id: i64) -> (u32, Option<String>) {
        use crate::timeline::data::Source as _;
        let src = crate::timeline::data::CoreSource(self.0.clone());
        let redactions = src
            .frame_preview(id)
            .ok()
            .flatten()
            .map_or(0, |p| p.redactions);
        let bundle = self.0.try_state::<AppInfraState>().and_then(|infra| {
            use app_core::app_infra::{get_frame, GetFrameRequest};
            let f = self
                .0
                .block_on(get_frame(GetFrameRequest { frame_id: id }, &infra))
                .ok()??;
            f.app_bundle_id.filter(|b| !b.trim().is_empty())
        });
        (redactions, bundle)
    }

    fn ocr(&self, id: i64) -> Res<Option<Vec<OcrBox>>> {
        use crate::timeline::data::Source as _;
        crate::timeline::data::CoreSource(self.0.clone()).ocr(id)
    }

    fn frames(&self, start_ms: i64, end_ms: i64) -> Res<Vec<(i64, i64)>> {
        use crate::insights::data::Backend as _;
        crate::insights::data::CoreBackend(self.0.clone()).frames(start_ms, end_ms)
    }

    fn app_icons(&self, apps: &[String]) -> Vec<(String, PathBuf)> {
        use crate::timeline::data::Source as _;
        crate::timeline::data::CoreSource(self.0.clone()).app_icons(apps)
    }

    fn chatgpt_begin(&self, provider: &str) -> Res<(String, String)> {
        use crate::onboarding::data::Backend as _;
        crate::onboarding::data::CoreBackend(self.0.clone()).chatgpt_begin(provider)
    }

    fn chatgpt_cancel(&self, provider: &str) {
        use crate::onboarding::data::Backend as _;
        crate::onboarding::data::CoreBackend(self.0.clone()).chatgpt_cancel(provider)
    }

    fn settings(&self) -> Option<capture_types::RecordingSettings> {
        use crate::quick_recall::data::Backend as _;
        crate::quick_recall::data::CoreBackend(self.0.clone()).settings()
    }

    fn warm_mcp(&self) {
        super::data::warm_mcp(&self.0)
    }
}
