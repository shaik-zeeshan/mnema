//! The Logs tab (`tabs/LogsTab.svelte`): level segment, Follow, reveal, a text
//! filter and the feature chips over the app log's last 500 lines, oldest first,
//! with the tail filling the rest of the tab. Warnings and errors are tinted.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use crate::ui::input::FieldText as _;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    div, px, relative, AnyElement, Context, ElementId, Focusable as _, SharedString, Window,
};

use super::data::LOG_TAIL_LINES;
use super::readouts::{filter_entries, group_log_lines, Level, LogEntry, LOG_FEATURES};
use super::DebugWindow;
use crate::settings::model::Json;
use crate::theme::{self, radius, size, space, text, Palette, FONT_MONO};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::segmented::Segmented;
use crate::ui::switch::Switch;
use crate::ui::{mix, nid, Button, CopyButton, Tone};

const LEVELS: [Level; 3] = [Level::All, Level::Warn, Level::Error];

/// The tail parsed, filtered and joined once per new tail or filter change, not
/// on every redraw (hovering a line re-renders the tab).
#[derive(Default)]
pub struct LogCache(RefCell<Option<(LogKey, Rc<Parsed>)>>);

type LogKey = (Option<serde_json::Value>, i64, Level, usize, String);

pub struct Parsed {
    /// What the filters leave, oldest first (a backtrace stays with its line).
    shown: Vec<LogEntry>,
    /// `shown` joined, for "Copy visible lines".
    visible: SharedString,
    /// Lines in the tail before any filter.
    total: usize,
}

impl LogCache {
    fn get(
        &self,
        tail: Option<&serde_json::Value>,
        key: (i64, Level, usize, String),
    ) -> Rc<Parsed> {
        let mut cell = self.0.borrow_mut();
        if let Some((k, parsed)) = cell.as_ref() {
            if k.0.as_ref() == tail && (k.1, k.2, k.3, &k.4) == (key.0, key.1, key.2, &key.3) {
                return parsed.clone();
            }
        }
        let lines: Vec<String> = tail
            .and_then(|t| t["lines"].as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|l| l.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let grouped = group_log_lines(&lines, key.0);
        let shown: Vec<LogEntry> = filter_entries(&grouped, key.1, key.2, &key.3)
            .into_iter()
            .cloned()
            .collect();
        let visible = shown
            .iter()
            .map(|e| e.raw.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            .into();
        let parsed = Rc::new(Parsed {
            shown,
            visible,
            total: lines.len(),
        });
        *cell = Some(((tail.cloned(), key.0, key.1, key.2, key.3), parsed.clone()));
        parsed
    }
}

/// A log row, keyed by its text and how many identical lines sit above it:
/// its state (the copy button's "Copied") stays on that line when new lines
/// push the tail up (an index would hand it to another).
pub fn line_id(raw: &str, occurrence: usize) -> ElementId {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (raw, occurrence).hash(&mut h);
    nid("dv-line", h.finish())
}

/// A line's copy button; it sits inside its [`line_id`] row.
pub fn line_copy_id(raw: &str) -> ElementId {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    raw.hash(&mut h);
    nid("dv-line-copy", h.finish())
}

pub fn tab(
    w: &DebugWindow,
    window: &mut Window,
    cx: &mut Context<DebugWindow>,
    p: &Palette,
) -> AnyElement {
    let this = cx.entity().downgrade();
    let level_ix = LEVELS.iter().position(|l| *l == w.level).unwrap_or(0);
    let tail = w.data.log_tail.as_ref();
    let exists = tail.is_some_and(|t| t.bool_at(&["exists"]));
    let path = tail.and_then(|t| t.str_at(&["path"])).map(str::to_string);
    let this2 = this.clone();
    let offset = crate::ui::clock::now(cx).offset().whole_seconds() as i64;
    let needle = w.needle.read(cx).value().to_string();
    let parsed = w.log_cache.get(tail, (offset, w.level, w.feature, needle));
    let shown = &parsed.shown;
    let visible = parsed.visible.clone();
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
                    CopyButton::icon("dv-copy-visible", "Copy visible lines", visible)
                        .tip("Copy visible lines"),
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
                    .field_h(px(18.))
                    .field_text(text::BASE)
                    .into_any_element(),
                w.needle.focus_handle(cx).is_focused(window),
                p,
            )
            .h(size::H_SM)
            .pl(px(8.)),
        )
        // Wraps rather than clipping: at 440 px "Meaning index" and the rest
        // fell off the right edge with no sign more existed.
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(2.))
                .children(LOG_FEATURES.iter().enumerate().map(|(i, (label, _))| {
                    Button::new(nid("dv-feat", i as u64), *label)
                        .ghost()
                        .sm()
                        .h(px(22.))
                        .pressed(w.feature == i)
                        .on_click(cx.listener(move |w, _, _, cx| {
                            w.feature = i;
                            cx.notify();
                        }))
                })),
        );

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
        let mut seen: HashMap<&str, usize> = HashMap::new();
        let mut rows: Vec<AnyElement> = shown
            .iter()
            .map(|e| {
                let n = seen.entry(e.raw.as_str()).or_default();
                let row_id = line_id(&e.raw, *n);
                *n += 1;
                let tone = match e.head.as_ref().map(|h| h.level.as_str()) {
                    Some("WARN") => Some(Tone::Warn),
                    Some("ERROR") => Some(Tone::Danger),
                    _ => None,
                };
                let (tc, tbg, _) =
                    tone.map_or((p.text_subtle, p.surface_subtle, p.border), |t| t.colors(p));
                let raw = e.raw.clone();
                let copied = w.copied_line.as_deref() == Some(e.raw.as_str());
                let hovered = w.hovered_line.as_deref() == Some(e.raw.as_str());
                let wash = p.wash;
                let (on_click, on_hover, marked) = (raw.clone(), raw.clone(), raw.clone());
                let me = cx.entity().downgrade();
                div()
                    .id(row_id)
                    .test_support()
                    .role(gpui_kit::Role::ListItem)
                    .relative()
                    .flex()
                    .gap(px(8.))
                    .px(space::S4)
                    .py(px(2.))
                    .cursor_pointer()
                    .when_some(tone, |d, _| d.bg(mix(tbg, p.surface_subtle, 0.7)))
                    .hover(move |s| s.bg(wash))
                    .on_hover(cx.listener(move |w, on: &bool, _, cx| {
                        if *on {
                            w.hovered_line = Some(on_hover.clone());
                        } else if w.hovered_line.as_ref() == Some(&on_hover) {
                            w.hovered_line = None;
                        }
                        cx.notify();
                    }))
                    .on_click(cx.listener(move |w, _, _, cx| w.copy_line(on_click.clone(), cx)))
                    // The visible way to copy: a small button on the line under the pointer.
                    .when(hovered, |d| {
                        d.child(
                            div()
                                .absolute()
                                .top(px(1.))
                                .right(space::S2)
                                .rounded(radius::SM)
                                .border_1()
                                .border_color(p.hairline)
                                .bg(p.surface_raised)
                                .child(
                                    CopyButton::icon(line_copy_id(&raw), "Copy line", raw)
                                        .h(px(18.))
                                        .on_copied(move |_, cx| {
                                            let _ = me.update(cx, |w, cx| {
                                                w.mark_copied(marked.clone(), cx)
                                            });
                                        }),
                                ),
                        )
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
                            .child(e.head.as_ref().map_or(String::new(), |h| h.level.clone()))
                            // `.dv-line.is-copied .dv-line__l::after { content: " copied" }`.
                            .when(copied, |d| d.child("copied")),
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
            let total = parsed.total;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tail_is_parsed_once_per_tail_and_filter() {
        let line =
            |t: &str, l: &str| format!("[2026-11-04][09:01:{t}][app_infra::ocr][{l}] [ocr] {l}");
        let tail = serde_json::json!({ "lines": [line("40", "INFO"), line("41", "WARN")] });
        let all = || (0, Level::All, 0, String::new());
        let cache = LogCache::default();
        let a = cache.get(Some(&tail), all());
        assert!(
            Rc::ptr_eq(&a, &cache.get(Some(&tail), all())),
            "a redraw reuses it"
        );
        assert_eq!((a.total, a.shown.len()), (2, 2));
        let warn = cache.get(Some(&tail), (0, Level::Warn, 0, String::new()));
        assert_eq!(warn.shown.len(), 1, "a filter change re-filters");
        let grown = serde_json::json!({
            "lines": [line("40", "INFO"), line("41", "WARN"), line("42", "INFO")]
        });
        assert_eq!(
            cache.get(Some(&grown), all()).total,
            3,
            "a new tail re-parses"
        );
    }
}
