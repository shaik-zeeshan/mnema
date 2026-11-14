//! CSS `letter-spacing` for GPUI, which has none. It rides in `FontFeatures` as a
//! pseudo feature the vendored Core Text crate strips and turns into kerning
//! (`vendor/gpui-pre-apple/src/text_system.rs`, `MNEMA_TRACKING_TAG`). Like CSS it
//! inherits to children; `.tracking(0.)` resets it.

use std::sync::Arc;

use gpui_kit::{FontFeatures, Styled};

/// Must match `MNEMA_TRACKING_TAG` in the vendored crate.
const TAG: &str = "MNtk";

/// `features` with tracking `em` (0 removes it), other features kept.
pub fn with_tracking(features: &FontFeatures, em: f32) -> FontFeatures {
    let mut list: Vec<_> = features
        .tag_value_list()
        .iter()
        .filter(|(tag, _)| tag != TAG)
        .cloned()
        .collect();
    let milli = (em * 1000.).round() as i32;
    if milli != 0 {
        list.push((TAG.into(), milli as u32));
    }
    FontFeatures(Arc::new(list))
}

pub trait Tracking: Styled + Sized {
    /// CSS `letter-spacing: <em>em` on this element and its children.
    fn tracking(mut self, em: f32) -> Self {
        let style = self.text_style();
        let base = style.font_features.take().unwrap_or_default();
        style.font_features = Some(with_tracking(&base, em));
        self
    }
}

impl<T: Styled> Tracking for T {}
