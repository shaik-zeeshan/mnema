use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OcrProvider {
    AppleVision,
}

impl OcrProvider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AppleVision => "apple_vision",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrRequest {
    pub image_path: PathBuf,
    pub payload_json: Option<String>,
}

impl OcrRequest {
    pub fn new(image_path: impl Into<PathBuf>) -> Self {
        Self {
            image_path: image_path.into(),
            payload_json: None,
        }
    }

    pub fn with_payload_json(mut self, payload_json: Option<String>) -> Self {
        self.payload_json = payload_json;
        self
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrOutput {
    pub text: String,
    pub structured_payload_json: Option<String>,
    pub engine_version: Option<String>,
}

impl OcrOutput {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            structured_payload_json: None,
            engine_version: None,
        }
    }

    pub fn with_structured_payload_json(mut self, payload_json: impl Into<String>) -> Self {
        self.structured_payload_json = Some(payload_json.into());
        self
    }

    pub fn with_engine_version(mut self, version: impl Into<String>) -> Self {
        self.engine_version = Some(version.into());
        self
    }
}

#[async_trait]
pub trait OcrEngine: Send + Sync {
    fn provider(&self) -> OcrProvider;

    async fn recognize(&self, request: OcrRequest) -> Result<OcrOutput>;
}

#[derive(Debug, Default)]
pub struct AppleVisionOcrEngine;

impl AppleVisionOcrEngine {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl OcrEngine for AppleVisionOcrEngine {
    fn provider(&self) -> OcrProvider {
        OcrProvider::AppleVision
    }

    async fn recognize(&self, request: OcrRequest) -> Result<OcrOutput> {
        run_apple_vision_ocr(request)
    }
}

fn run_apple_vision_ocr(request: OcrRequest) -> Result<OcrOutput> {
    super::apple_vision::recognize(request)
}
