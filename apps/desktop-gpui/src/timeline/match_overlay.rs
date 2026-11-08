//! Recall landing outlines (`RecallMatchBoxes.svelte`): the words a Quick Recall
//! hit matched, on the landed frame only, while its exact image is painted.
//! Solid accent = keyword match; dashed grey = found by meaning.

use app_infra::MatchBoxKind;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Bounds, Pixels};

use super::TimelineView;
use crate::theme::Palette;
use crate::ui::{alpha, inset};

pub fn render(v: &TimelineView, rect: Option<Bounds<Pixels>>, p: &Palette) -> Option<AnyElement> {
    let rect = rect?;
    let l = v.landing.as_ref()?;
    if !l.screen
        || v.paging.active_frame().map(|f| f.id) != Some(l.frame_id)
        || v.match_boxes.is_empty()
    {
        return None;
    }
    let (w, h) = (f32::from(rect.size.width), f32::from(rect.size.height));
    let boxes = v.match_boxes.iter().map(|b| {
        let el = div()
            .absolute()
            .left(px(b.x as f32 * w))
            .top(px(b.y as f32 * h))
            .w(px(b.w as f32 * w))
            .h(px(b.h as f32 * h))
            .rounded(px(3.));
        match b.kind {
            MatchBoxKind::Keyword => el
                .bg(alpha(p.accent, 0.12))
                .shadow(vec![inset(p.accent, px(1.5))]),
            MatchBoxKind::Meaning => el
                .bg(alpha(p.fg, 0.07))
                .border_1()
                .border_dashed()
                .border_color(p.text_subtle),
        }
    });
    Some(
        div()
            .absolute()
            .left(rect.origin.x)
            .top(rect.origin.y)
            .w(rect.size.width)
            .h(rect.size.height)
            .children(boxes)
            .into_any_element(),
    )
}
