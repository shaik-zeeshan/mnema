//! `Timeline.svelte`: an answer's time-of-day breakdown as a vertical rail — a
//! 1 px spine tapered at both ends, a category dot per row (ringed in the
//! panel tone and its own colour at 22%), the range above the label, and an
//! app chip with the app's icon (its initial until one resolves). Chat mounts
//! it for a `mnema-timeline` block; the items arrive parsed.

use std::path::PathBuf;

use capture_types::{ActivityCategory, TimelineItem};
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, relative, AnyElement, Div, Role, SharedString};

use crate::insights::stats::category_color;
use crate::theme::{self, Palette};
use crate::ui::app_icon::icon_or_initial;
use crate::ui::{alpha, nid};

/// The dot's centre from the row top (`.dot` top 5 + half its 8).
const DOT_Y: f32 = 9.;

/// `appLabel`: the app without one trailing `(…)` the model sometimes adds
/// ("Zen (Zen)" → "Zen"); `None` when nothing is left to show.
pub fn app_label(app: Option<&str>) -> Option<String> {
    let app = app?;
    let t = app.trim_end();
    let stripped = t.strip_suffix(')').and_then(|r| {
        let open = r.rfind('(')?;
        (!r[open + 1..].contains(')')).then(|| r[..open].trim())
    });
    let label = stripped.filter(|s| !s.is_empty()).unwrap_or(app.trim());
    (!label.is_empty()).then(|| label.to_string())
}

fn chip(label: String, icon: Option<PathBuf>, p: &Palette) -> Div {
    let (bg, fg) = (p.surface_subtle, p.text_muted);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .pl(px(2.))
        .pr(px(6.))
        .py(px(1.))
        .border_1()
        .border_color(p.border)
        .rounded_full()
        .bg(p.surface)
        .whitespace_nowrap()
        .text_size(px(9.5))
        .text_color(p.text_muted)
        .child(icon_or_initial(icon, &label, 13., move || {
            div()
                .size(px(13.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .bg(bg)
                .text_color(fg)
                .text_size(px(8.))
                .font_weight(theme::W_SEMIBOLD)
        }))
        .child(div().min_w_0().truncate().child(SharedString::from(label)))
}

/// The rail, a labelled list (`id`, rows `<id>-row`); `icon` answers an app
/// label's resolved icon, if one has landed.
pub fn timeline(
    id: &str,
    title: Option<&str>,
    items: &[TimelineItem],
    icon: &dyn Fn(&str) -> Option<PathBuf>,
    p: &Palette,
) -> AnyElement {
    let n = items.len();
    let name = title.map_or_else(
        || {
            let what = if n == 1 { "interval" } else { "intervals" };
            format!("Activity timeline, {n} {what}")
        },
        str::to_string,
    );
    let row_id = format!("{id}-row");
    div()
        .id(SharedString::from(id.to_string()))
        .test_support()
        .role(Role::List)
        .aria_label(name)
        .flex()
        .flex_col()
        .children(title.filter(|_| n > 0).map(|t| {
            div()
                .mb(px(10.))
                .text_size(px(10.5))
                .text_color(p.text_muted)
                .child(SharedString::from(t.to_uppercase()))
        }))
        .children(items.iter().enumerate().map(|(i, it)| {
            let (first, last) = (i == 0, i + 1 == n);
            let color = category_color(
                it.category.as_ref().and_then(|c| {
                    serde_json::from_value::<ActivityCategory>(c.as_str().into()).ok()
                }),
                p,
            );
            let spine = div()
                .absolute()
                .left(px(7.5))
                .w(px(1.))
                .bg(p.border)
                .map(|d| match (first, last) {
                    (true, true) => d.h_0(),
                    (true, false) => d.top(px(DOT_Y)).bottom_0(),
                    (false, true) => d.top_0().h(px(DOT_Y)),
                    (false, false) => d.top_0().bottom_0(),
                });
            // box-shadow 2 px panel ring + 4 px colour ring, as nested discs.
            let dot = div()
                .absolute()
                .top(px(DOT_Y - 8.))
                .left_0()
                .size(px(16.))
                .rounded_full()
                .bg(alpha(color, 0.22))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .size(px(12.))
                        .rounded_full()
                        .bg(p.surface_subtle)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(div().size(px(8.)).rounded_full().bg(color)),
                );
            let range = match &it.end {
                Some(e) => format!("{} – {e}", it.start),
                None => it.start.clone(),
            };
            let app = app_label(it.app.as_deref());
            let said = [
                Some(range.as_str()),
                Some(it.label.as_str()),
                app.as_deref(),
            ];
            div()
                .id(nid(&row_id, i as u64))
                .test_support()
                .role(Role::ListItem)
                .aria_label(said.into_iter().flatten().collect::<Vec<_>>().join(", "))
                .flex()
                .gap(px(11.))
                .child(
                    div()
                        .relative()
                        .flex_none()
                        .w(px(16.))
                        .child(spine)
                        .child(dot),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.))
                        .when(!last, |d| d.pb(px(15.)))
                        .child(
                            div()
                                .text_size(px(10.5))
                                .font_weight(theme::W_MEDIUM)
                                .text_color(p.text_muted)
                                .child(SharedString::from(range)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_baseline()
                                .gap(px(7.))
                                .min_w_0()
                                .child(
                                    div()
                                        .min_w_0()
                                        .text_size(px(12.5))
                                        .line_height(relative(1.45))
                                        .text_color(p.text)
                                        .child(SharedString::from(it.label.clone())),
                                )
                                .children(app.map(|a| {
                                    let path = icon(&a);
                                    chip(a, path, p)
                                })),
                        ),
                )
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::app_label;

    #[test]
    fn app_label_drops_one_trailing_parenthetical() {
        assert_eq!(app_label(Some("Zen (Zen)")).as_deref(), Some("Zen"));
        assert_eq!(
            app_label(Some("Safari (Release notes) ")).as_deref(),
            Some("Safari")
        );
        assert_eq!(app_label(Some("Notes")).as_deref(), Some("Notes"));
        // Nested or bare parentheses stay as they are.
        assert_eq!(app_label(Some("a (b (c))")).as_deref(), Some("a (b (c))"));
        assert_eq!(app_label(Some("(x)")).as_deref(), Some("(x)"));
        assert_eq!(app_label(Some("  ")), None);
        assert_eq!(app_label(None), None);
    }
}
