//! The Logs tab (`tabs/LogsTab.svelte`): level segment, Follow, reveal, a text
//! filter and the feature chips over the app log's last 500 lines, oldest first,
//! with the tail filling the rest of the tab. Warnings and errors are tinted.

use crate::ui::input::FieldText as _;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, ClipboardItem, Context, Window};

use super::data::LOG_TAIL_LINES;
use super::readouts::{filter_entries, group_log_lines, Level, LOG_FEATURES};
use super::DebugWindow;
use crate::settings::model::Json;
use crate::theme::{self, size, space, text, Palette, FONT_MONO};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::segmented::Segmented;
use crate::ui::switch::Switch;
use crate::ui::{mix, nid, Button, Tone};

const LEVELS: [Level; 3] = [Level::All, Level::Warn, Level::Error];

pub fn tab(
    w: &DebugWindow,
    _: &mut Window,
    cx: &mut Context<DebugWindow>,
    p: &Palette,
) -> AnyElement {
    let this = cx.entity().downgrade();
    let level_ix = LEVELS.iter().position(|l| *l == w.level).unwrap_or(0);
    let tail = w.data.log_tail.as_ref();
    let exists = tail.is_some_and(|t| t.bool_at(&["exists"]));
    let path = tail.and_then(|t| t.str_at(&["path"])).map(str::to_string);
    let this2 = this.clone();
    let bar = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .flex_none()
        .child(
            div()
                .flex()
                .items_center()
                .gap(space::S2)
                .child(
                    Segmented::new("dv-level", "Level", ["All", "Warnings", "Errors"], level_ix)
                        .on_change(move |ix, _, cx| {
                            let _ = this.update(cx, |w, cx| {
                                w.level = LEVELS[ix];
                                cx.notify();
                            });
                        }),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .font_weight(theme::W_MEDIUM)
                        .text_size(text::BASE)
                        .text_color(p.text_muted)
                        .child(Switch::new("dv-follow", "Follow", w.follow).on_toggle(
                            move |on, _, cx| {
                                let _ = this2.update(cx, |w, cx| {
                                    w.follow = on;
                                    if on {
                                        w.log_scroll.scroll_to_bottom();
                                    }
                                    cx.notify();
                                });
                            },
                        ))
                        .child("Follow"),
                )
                .child(
                    Button::icon_only("dv-reveal-log", "icons/folder.svg", "Reveal in Finder")
                        .ghost()
                        .sm()
                        .tip("Reveal in Finder")
                        .disabled(!exists)
                        .on_click(move |_, _, cx| {
                            if let Some(p) = &path {
                                cx.reveal_path(std::path::Path::new(p));
                            }
                        }),
                ),
        )
        .child(
            // `.dv-logbar .mx-search`: the search field at the small height.
            crate::chat::list::search_box(
                gpui_kit::component::input::Input::new(&w.needle)
                    .aria_label("Filter lines")
                    .appearance(false)
                    .px_0()
                    .h(px(18.))
                    .field_text(text::BASE)
                    .into_any_element(),
                p,
            )
            .h(size::H_SM)
            .pl(px(8.)),
        )
        .child(div().flex().gap(px(2.)).overflow_hidden().children(
            LOG_FEATURES.iter().enumerate().map(|(i, (label, _))| {
                Button::new(nid("dv-feat", i as u64), *label)
                    .ghost()
                    .sm()
                    .h(px(22.))
                    .pressed(w.feature == i)
                    .on_click(cx.listener(move |w, _, _, cx| {
                        w.feature = i;
                        cx.notify();
                    }))
            }),
        ));

    let offset = crate::ui::clock::now(cx).offset().whole_seconds() as i64;
    let lines: Vec<String> = tail
        .and_then(|t| t["lines"].as_array())
        .map(|a| {
            a.iter()
                .filter_map(|l| l.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let grouped = group_log_lines(&lines, offset);
    let needle = w.needle.read(cx).value().to_string();
    let shown = filter_entries(&grouped, w.level, w.feature, &needle);
    let compact = |tone, glyph: &'static str, title: &str, body: String| {
        empty(
            EmptyLayout::Compact,
            tone,
            glyph,
            title.to_string(),
            body,
            vec![],
            p,
        )
        .p(space::S4)
        .into_any_element()
    };
    let content: Vec<AnyElement> = if let Some(e) = &w.data.log_error {
        vec![compact(
            Tone::Danger,
            "icons/file-text.svg",
            "Couldn't read the log",
            e.clone(),
        )]
    } else if tail.is_some() && !exists {
        vec![compact(
            Tone::Neutral,
            "icons/file-text.svg",
            "Nothing logged yet",
            "Lines appear here as soon as the app writes them.".into(),
        )]
    } else if tail.is_some() && shown.is_empty() {
        vec![compact(
            Tone::Neutral,
            "icons/search.svg",
            "No matching lines",
            "Try another level or feature.".into(),
        )]
    } else {
        let mut rows: Vec<AnyElement> = shown
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let tone = match e.head.as_ref().map(|h| h.level.as_str()) {
                    Some("WARN") => Some(Tone::Warn),
                    Some("ERROR") => Some(Tone::Danger),
                    _ => None,
                };
                let (tc, tbg, _) =
                    tone.map_or((p.text_subtle, p.surface_subtle, p.border), |t| t.colors(p));
                let raw = e.raw.clone();
                let wash = p.wash;
                div()
                    .id(nid("dv-line", i as u64))
                    .flex()
                    .gap(px(8.))
                    .px(space::S4)
                    .py(px(2.))
                    .cursor_pointer()
                    .when_some(tone, |d, _| d.bg(mix(tbg, p.surface_subtle, 0.7)))
                    .hover(move |s| s.bg(wash))
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(raw.clone()))
                    })
                    .child(
                        div()
                            .flex_none()
                            .w(px(82.))
                            .text_color(p.text_subtle)
                            .child(e.head.as_ref().map_or(String::new(), |h| h.time.clone())),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(38.))
                            .text_size(text::XS)
                            .text_color(tc)
                            .when(tone.is_some(), |d| d.font_weight(theme::W_SEMIBOLD))
                            .child(e.head.as_ref().map_or(String::new(), |h| h.level.clone())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .children(e.head.as_ref().map(|h| {
                                crate::chat::inline_runs(&[
                                    (
                                        &h.prefix,
                                        Some((
                                            if tone.is_some() {
                                                theme::W_SEMIBOLD
                                            } else {
                                                theme::W_MEDIUM
                                            },
                                            if tone.is_some() { tc } else { p.text_muted },
                                        )),
                                    ),
                                    (" ", None),
                                    (&h.message, None),
                                ])
                            }))
                            .children(
                                e.more
                                    .iter()
                                    .map(|m| div().text_color(p.text_muted).child(m.clone())),
                            ),
                    )
                    .into_any_element()
            })
            .collect();
        if tail.is_some() {
            let total = lines.len();
            let mut foot = if total >= LOG_TAIL_LINES {
                format!("Showing the last {LOG_TAIL_LINES} lines")
            } else {
                format!("Showing all {total} lines")
            };
            if w.follow {
                foot.push_str(" \u{b7} following");
            }
            rows.push(
                div()
                    .px(space::S4)
                    .pt(px(6.))
                    .font_family(theme::FONT_UI)
                    .text_size(text::SM)
                    .text_color(p.text_subtle)
                    .child(foot)
                    .into_any_element(),
            );
        }
        rows
    };
    let tail_el = div()
        .id("dv-tail")
        .track_scroll(&w.log_scroll)
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .mx(-space::S4)
        .pt(px(4.))
        .pb(px(8.))
        .border_t_1()
        .border_color(p.hairline)
        .bg(p.surface_subtle)
        .font_family(FONT_MONO)
        .text_size(text::SM)
        .line_height(relative(1.5))
        .text_color(p.text)
        .children(content)
        .child(crate::ui::scroll::bar(&w.log_scroll));
    div()
        .id("dv-body")
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .gap(px(10.))
        .pt(px(14.))
        .px(space::S4)
        .child(bar)
        .child(tail_el)
        .into_any_element()
}
