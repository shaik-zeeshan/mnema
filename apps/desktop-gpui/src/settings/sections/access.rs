//! Data › CLI Access (`Access.svelte`, `cli-access.svelte.ts`): the `mnema`
//! command, the tools with standing access (lapse bar, Block), the blocked
//! ones (Unblock asks first), and recent activity.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, SharedString};
use serde_json::Value;

use crate::settings::model::{relative, short_day, Json};
use crate::settings::row::{line, row, section};
use crate::settings::{click, confirm, icons, Settings};
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::{icon, Button, Tone};

const DAY_MS: i64 = 86_400_000;
/// `GRANT_IDLE_TTL_MS` (mirrors `BROKER_GRANT_IDLE_TTL_MS`).
const TTL_MS: i64 = 30 * DAY_MS;
/// `ACTIVITY_LIMIT`.
const ACTIVITY: usize = 20;

fn source_label(src: &str) -> &'static str {
    match src {
        "explicit" => "Declared",
        "env" => "From env",
        "inferred" => "Inferred",
        _ => "Unnamed",
    }
}

/// `formatScopeChip`.
fn scope_chip(scope: &Value) -> String {
    if scope.as_str() == Some("all_retained_history") {
        return "all retained".into();
    }
    match scope.num_at(&["recent_days", "days"]) {
        Some(d) if d <= 1. => "last day".into(),
        Some(d) => format!("last {d} days"),
        None => "limited scope".into(),
    }
}

/// `formatGrantScopeProse`.
fn scope_prose(scope: &Value) -> String {
    if scope.as_str() == Some("all_retained_history") {
        return "your entire retained history".into();
    }
    match scope.num_at(&["recent_days", "days"]) {
        Some(d) if d <= 1. => "your last 24 hours".into(),
        Some(d) => format!("your last {d} days"),
        None => "the history its permission still allows".into(),
    }
}

/// `grantLapse`: label, remaining fraction, soon.
pub fn lapse(last_used: i64, now: i64) -> (String, f32, bool) {
    let left = (last_used + TTL_MS - now).max(0);
    let days = (left + DAY_MS - 1) / DAY_MS;
    let label = if days <= 1 {
        "Lapses today".to_string()
    } else {
        format!("Lapses in {days} days")
    };
    (label, left as f32 / TTL_MS as f32, days <= 7)
}

fn tool_row(
    g: &Value,
    last_cmd: Option<String>,
    p: &Palette,
    cx: &mut Context<Settings>,
) -> AnyElement {
    let now = crate::ui::clock::now_ms(cx);
    let today = crate::ui::clock::now(cx);
    let label = g.str_at(&["label"]).unwrap_or_default().to_string();
    let key = g
        .str_at(&["normalizedLabel"])
        .unwrap_or_default()
        .to_string();
    let blocked = g.bool_at(&["blocked"]);
    let last = g.num_at(&["lastUsedAtUnixMs"]).unwrap_or(0.) as i64;
    let never = last == 0;
    let name = div()
        .flex()
        .items_center()
        .gap(space::S2)
        .child(
            div()
                .text_size(text::MD)
                .font_weight(theme::W_MEDIUM)
                .text_color(p.text_strong)
                .child(label.clone()),
        )
        .child(chip(
            source_label(g.str_at(&["identitySource"]).unwrap_or_default()),
            ChipKind::Neutral,
            p,
        ));
    let scope = g.at(&["scope"]).clone();
    let meta = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(text::SM)
        .text_color(p.text_subtle);
    let meta = if blocked {
        let at = g
            .num_at(&["blockedAtUnixMs"])
            .map(|ms| format!("blocked {}", short_day(ms as i64, today)));
        meta.child(if never {
            "Never allowed".into_any_element()
        } else {
            div()
                .flex()
                .gap(px(6.))
                .child("Remembers")
                .child(chip(scope_chip(&scope), ChipKind::Neutral, p))
                .into_any_element()
        })
        .children(at.map(|a| div().child(format!("\u{b7} {a}"))))
        .when(never, |d| d.child("\u{b7} will ask again if unblocked"))
    } else {
        meta.child(chip(scope_chip(&scope), ChipKind::Neutral, p))
            .child(format!("\u{b7} used {}", relative(last, now)))
            .when_some(last_cmd, |d, c| {
                d.child("\u{b7} last ran").child(
                    div()
                        .font_family(FONT_MONO)
                        .text_color(p.text_muted)
                        .child(c),
                )
            })
    };
    let (key2, label2) = (key.clone(), label.clone());
    let right: AnyElement = if blocked {
        div()
            .flex()
            .items_center()
            .gap(space::S3)
            .child(div().text_size(text::SM).font_weight(theme::W_SEMIBOLD).text_color(p.text_muted).child("Never lapses"))
            .child(Button::new(SharedString::from(format!("unblock-{key}")), if never { "Unblock" } else { "Unblock\u{2026}" }).sm().on_click(click(
                cx,
                move |s, cx| {
                    let key = key2.clone();
                    if never {
                        s.set_blocked(key, false, cx);
                    } else {
                        let msg = format!("{label2} will be able to read {} again, with no expiry, until you block it.", scope_prose(&scope));
                        confirm(cx, "Enable CLI access again?", &msg, "Enable access", move |s, cx| s.set_blocked(key, false, cx));
                    }
                },
            )))
            .into_any_element()
    } else {
        let (text_, fraction, soon) = lapse(last, now);
        let bar = div()
            .w(px(80.))
            .h(px(3.))
            .rounded(radius::PILL)
            .bg(p.wash_strong)
            .child(
                div()
                    .h_full()
                    .w(gpui_kit::relative(fraction))
                    .rounded(radius::PILL)
                    .bg(if soon { p.warn } else { p.text_subtle }),
            );
        div()
            .flex()
            .items_center()
            .gap(space::S3)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(text::SM)
                            .text_color(p.text_muted)
                            .child(text_),
                    )
                    .child(bar),
            )
            .child(
                Button::new(SharedString::from(format!("block-{key}")), "Block")
                    .ghost()
                    .danger()
                    .sm()
                    .tip("Revoke and refuse its future requests")
                    .on_click(click(cx, move |s, cx| s.set_blocked(key.clone(), true, cx))),
            )
            .into_any_element()
    };
    div()
        .flex()
        .items_center()
        .gap(space::S3)
        .px(space::S3)
        .py(px(10.))
        .border_b_1()
        .border_color(p.hairline)
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(32.))
                .rounded(radius::MD)
                .border_1()
                .border_color(if blocked { p.danger_border } else { p.hairline })
                .bg(if blocked { p.danger_bg } else { p.wash })
                .text_color(if blocked { p.danger_text } else { p.text_muted })
                .child(icon(
                    if blocked { icons::OFF } else { icons::TERMINAL },
                    px(15.),
                )),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .flex_1()
                .min_w_0()
                .child(name)
                .child(meta),
        )
        .child(right)
        .into_any_element()
}

fn list_section(title: &str, hint: &str, rows: Vec<AnyElement>, p: &Palette) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(space::S2)
        .pt(space::S5)
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(space::S3)
                .child(
                    div()
                        .text_size(text::MD)
                        .font_weight(theme::W_SEMIBOLD)
                        .text_color(p.text_strong)
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .text_size(text::BASE)
                        .text_color(p.text_subtle)
                        .child(hint.to_string()),
                ),
        )
        .child(
            div()
                .rounded(radius::LG)
                .border_1()
                .border_color(p.border)
                .children(rows),
        )
        .into_any_element()
}

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let cli = s.data.cli_status.as_ref();
    let cli_line = match cli {
        None => div()
            .text_size(text::BASE)
            .text_color(p.text_muted)
            .child(if s.data.complete {
                "Couldn\u{2019}t check the install."
            } else {
                "Checking the install\u{2026}"
            })
            .into_any_element(),
        Some(c) if !c.bool_at(&["installed"]) => line(
            Tone::Warn,
            icons::ALERT,
            format!(
                "Not installed at {} \u{2014} install it before tools can ask for access.",
                c.str_at(&["installPath"]).unwrap_or_default()
            ),
            p,
        )
        .into_any_element(),
        Some(c) => div()
            .text_size(text::BASE)
            .text_color(p.text_muted)
            .child(format!(
                "Installed at {}.",
                c.str_at(&["installPath"]).unwrap_or_default()
            ))
            .into_any_element(),
    };
    let installed = cli.is_some_and(|c| c.bool_at(&["installed"]));
    let mut rows = vec![
        div()
            .pb(space::S3)
            .text_size(text::BASE)
            .line_height(gpui_kit::relative(1.5))
            .text_color(p.text_muted)
            .child("Local tools and AI agents can read your searchable Mnema text \u{2014} screen text, transcripts and timeline results, with the app and window each came from. Never media files, raw database rows or full web addresses.")
            .into_any_element(),
        row("mnema command")
            .control(
                Button::new("cli-install", if installed { "Reinstall CLI" } else { "Install CLI" })
                    .disabled(s.ui.busy.contains("Install CLI"))
                    .on_click(crate::settings::click(cx, |s, cx| {
                        s.run_command("Install CLI", cx, crate::settings::live::install_cli)
                    })),
            )
            .below(cli_line)
            .build(p)
            .into_any_element(),
    ];
    let (blocked, active): (Vec<&Value>, Vec<&Value>) =
        s.data.grants.iter().partition(|g| g.bool_at(&["blocked"]));
    if active.is_empty() && blocked.is_empty() {
        rows.push(list_section(
            "Tools with access",
            "",
            vec![div()
                .p(space::S3)
                .child(empty(
                    EmptyLayout::Compact,
                    Tone::Neutral,
                    icons::TERMINAL,
                    "No tool has access",
                    "The first time an agent runs a command like mnema search, a window asks you. Tools you allow \u{2014} or block \u{2014} show up here.",
                    Vec::new(),
                    p,
                ))
                .into_any_element()],
            p,
        ));
    }
    // `lastCommandByTool`: the newest command per normalized tool identity.
    let last_cmd = |g: &Value| {
        let key = g.str_at(&["normalizedLabel"]);
        s.data
            .history
            .iter()
            .filter(|e| e.str_at(&["normalizedToolIdentity"]) == key)
            .max_by_key(|e| e.num_at(&["timestampUnixMs"]).unwrap_or(0.) as i64)
            .and_then(|e| e.str_at(&["commandType"]))
            .map(|c| c.replace('_', "-"))
    };
    if !active.is_empty() {
        let list = active
            .iter()
            .map(|g| tool_row(g, last_cmd(g), p, cx))
            .collect();
        rows.push(list_section(
            &format!("Tools with access {}", active.len()),
            "Each lapses 30 days after its last use.",
            list,
            p,
        ));
    }
    if !blocked.is_empty() {
        let list = blocked.iter().map(|g| tool_row(g, None, p, cx)).collect();
        rows.push(list_section(
            &format!("Blocked {}", blocked.len()),
            "Refused before any window opens. Never lapses.",
            list,
            p,
        ));
    }
    let now = crate::ui::clock::now_ms(cx);
    let events: Vec<AnyElement> = s
        .data
        .history
        .iter()
        .rev()
        .take(ACTIVITY)
        .map(|e| {
            let outcome = match e.str_at(&["outcome"]) {
                None | Some("success") => None,
                Some("scope_rejected") => Some("Out of scope"),
                Some("denied") => Some("Denied"),
                Some(_) => Some("Refused"),
            };
            let n = e.num_at(&["resultCount"]).unwrap_or(0.) as i64;
            let detail = outcome.map(str::to_string).unwrap_or_else(|| {
                if n == 1 {
                    "1 result".into()
                } else {
                    format!("{n} results")
                }
            });
            div()
                .flex()
                .items_center()
                .gap(space::S3)
                .py(px(6.))
                .text_size(text::SM)
                .child(
                    div()
                        .flex_1()
                        .text_color(p.text)
                        .child(e.str_at(&["toolIdentity"]).unwrap_or_default().to_string()),
                )
                .child(
                    div().font_family(FONT_MONO).text_color(p.text_muted).child(
                        e.str_at(&["commandType"])
                            .unwrap_or_default()
                            .replace('_', "-"),
                    ),
                )
                .child(
                    div()
                        .w(px(80.))
                        .text_color(if outcome.is_some() {
                            p.warn
                        } else {
                            p.text_subtle
                        })
                        .child(detail),
                )
                .child(
                    div()
                        .font_family(FONT_MONO)
                        .text_color(p.text_subtle)
                        .child(relative(
                            e.num_at(&["timestampUnixMs"]).unwrap_or(0.) as i64,
                            now,
                        )),
                )
                .into_any_element()
        })
        .collect();
    rows.push(
        div()
            .flex()
            .flex_col()
            .gap(space::S2)
            .pt(space::S5)
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .text_size(text::MD)
                            .font_weight(theme::W_SEMIBOLD)
                            .text_color(p.text_strong)
                            .child("Recent activity"),
                    )
                    .when(!s.data.history.is_empty(), |d| {
                        d.child(
                            div().ml_auto().child(
                                Button::new("cli-reveal-log", "Reveal log in Finder")
                                    .ghost()
                                    .sm()
                                    .icon(icons::FOLDER)
                                    .on_click(|_, _, cx| {
                                        if let Some(core) = crate::chat::core(cx) {
                                            cx.reveal_path(
                                                &core.paths().config.join("broker-audit.json"),
                                            );
                                        }
                                    }),
                            ),
                        )
                    }),
            )
            .map(|d| {
                if events.is_empty() {
                    d.child(
                        div().text_size(text::BASE).text_color(p.text_subtle).child(
                            "Nothing yet. Commands run by tools with access are recorded here.",
                        ),
                    )
                } else {
                    d.children(events)
                }
            })
            .into_any_element(),
    );
    section(
        "CLI Access",
        Some("One standing permission per tool. Unused for 30 days, it lapses."),
        Vec::new(),
        true,
        rows,
        p,
    )
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lapse_counts_down_from_last_use() {
        let now = 100 * DAY_MS;
        assert_eq!(lapse(now - 2 * 3_600_000, now).0, "Lapses in 30 days");
        assert_eq!(
            lapse(now - 26 * DAY_MS, now),
            ("Lapses in 4 days".into(), 4. / 30., true)
        );
        assert_eq!(lapse(now - 31 * DAY_MS, now).0, "Lapses today");
    }
}
