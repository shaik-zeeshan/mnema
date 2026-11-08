//! "What Mnema is doing" (`StatusPop.svelte`, 340 px, opens upward from the REC
//! pill or the lane group): capture now, free space, and the engine when unhealthy.
//! The words come from `status-pop.ts`.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, Context, Hsla};

use super::model::{Lane, StatusBarModel};
use super::Shell;
use crate::keymap;
use crate::theme::{self, space, text, Palette};
use crate::ui::{kbd, kicker, pop_in, popover};

/// Free space below this always warns (`LOW_DISK_WARN_BYTES`).
const LOW_DISK_WARN_BYTES: u64 = 5 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopRow {
    pub name: &'static str,
    /// `None` = the neutral ring (in progress, not a problem).
    pub tone: Option<RowTone>,
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowTone {
    Ok,
    Warn,
    Danger,
    Neutral,
}

// shortcut: `status-pop.ts` reads `get_capture_health` (reconnecting / waiting
// rows) and the processing backlog ("Catching up"); neither is in app-core until
// S3. Rows here come from the runtime sources the status bar already holds.
pub fn capture_rows(m: &StatusBarModel) -> Option<Vec<PopRow>> {
    let rt = m.runtime.filter(|_| m.running)?;
    Some(
        Lane::ALL
            .iter()
            .zip(rt)
            .map(|(lane, src)| {
                let (tone, value) = if !src.requested {
                    (None, "Off for this recording")
                } else if src.privacy_failed {
                    (
                        Some(RowTone::Warn),
                        "Privacy filter couldn’t be applied, retrying",
                    )
                } else if src.paused && m.low_disk {
                    (Some(RowTone::Warn), "Paused until you free up space")
                } else if src.paused {
                    let by_you = m.user_paused;
                    (
                        None,
                        if by_you {
                            "Paused by you"
                        } else {
                            "Paused while you’re away"
                        },
                    )
                } else if src.live {
                    (Some(RowTone::Ok), "Recording")
                } else {
                    (None, "Off for this recording")
                };
                PopRow {
                    name: lane.name(),
                    tone,
                    value: value.into(),
                }
            })
            .collect(),
    )
}

/// `gb()` in capture-view.ts: binary GB, whole above 100, one decimal below
/// (trailing `.0` dropped, as JS `String(number)` does).
pub fn gb(bytes: u64) -> String {
    let v = bytes as f64 / (1u64 << 30) as f64;
    if v >= 100. {
        format!("{}", v.round())
    } else {
        format!("{}", (v * 10.).round() / 10.)
    }
}

pub fn disk_row(m: &StatusBarModel) -> Option<PopRow> {
    let free = m.free_bytes?;
    let gb = format!("{} GB", gb(free));
    Some(if m.low_disk || free < LOW_DISK_WARN_BYTES {
        PopRow {
            name: "Free space",
            tone: Some(RowTone::Warn),
            value: format!("{gb} left"),
        }
    } else {
        PopRow {
            name: "Free space",
            tone: Some(RowTone::Neutral),
            value: gb,
        }
    })
}

fn row(r: &PopRow, p: &Palette) -> impl IntoElement {
    let tone: Option<Hsla> = r.tone.map(|t| match t {
        RowTone::Ok => p.accent,
        RowTone::Warn => p.warn,
        RowTone::Danger => p.danger,
        RowTone::Neutral => p.text_faint,
    });
    let dot = div()
        .flex_none()
        .mt(px(5.))
        .size(px(7.))
        .rounded_full()
        .map(|d| match tone {
            Some(c) => d.bg(c),
            None => d.border(px(1.5)).border_color(p.text_muted),
        });
    div()
        .flex()
        .gap(space::S2)
        .text_size(text::BASE)
        .line_height(relative(1.4))
        .child(dot)
        .child(
            div()
                .flex_none()
                .w(px(92.))
                .font_weight(theme::W_MEDIUM)
                .text_color(p.text_strong)
                .child(r.name),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(p.text_muted)
                .child(r.value.clone()),
        )
}

fn group(label: &'static str, p: &Palette) -> gpui_kit::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .px(space::S4)
        .py(space::S2)
        .border_t_1()
        .border_color(p.hairline)
        .child(
            div()
                .text_size(text::BASE)
                .font_weight(theme::W_MEDIUM)
                .text_color(p.text_subtle)
                .child(label),
        )
}

pub fn render(shell: &Shell, p: &Palette, _cx: &mut Context<Shell>) -> impl IntoElement {
    let m = &shell.model.status;
    let capture = match capture_rows(m) {
        Some(rows) => group("Capture", p).child(
            div()
                .flex()
                .flex_col()
                .gap(px(7.))
                .children(rows.iter().map(|r| row(r, p))),
        ),
        None if m.loading_start => {
            group("Capture", p).child(div().text_color(p.text_subtle).child("Starting…"))
        }
        None => group("Capture", p).child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .text_color(p.text_subtle)
                .child("Not recording. Press Record or")
                .child(kbd(keymap::display(&shell.bindings.toggle_recording), p))
                .child("."),
        ),
    };
    let disk = disk_row(m).map(|r| group("Disk", p).child(row(&r, p)));
    let ai = m.engine.as_ref().map(|e| {
        let r = PopRow {
            name: "Ask AI",
            tone: Some(if e.label == "needs reconnect" {
                RowTone::Danger
            } else if e.error {
                RowTone::Warn
            } else {
                RowTone::Neutral
            }),
            // shortcut: the engine's full tip text comes with `AiRuntimeStatus` (S2).
            value: format!("Engine {}", e.label),
        };
        group("AI", p).child(row(&r, p))
    });
    pop_in(
        popover("status-popover", "What Mnema is doing", p)
            .left(space::S2)
            .w(theme::size::POP_STATUS_W)
            .pb(space::S2)
            .child(
                div()
                    .pt(space::S3)
                    .px(space::S4)
                    .pb(space::S2)
                    .child(kicker("What Mnema is doing", p)),
            )
            .child(capture)
            .children(disk)
            .children(ai),
        theme::size::STATUS_BAR + theme::size::POP_GAP,
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::model::RuntimeSource;

    #[test]
    fn capture_rows_follow_runtime_and_disk_warns_low() {
        let on = RuntimeSource {
            requested: true,
            paused: false,
            live: true,
            privacy_failed: false,
        };
        let mut m = StatusBarModel {
            running: true,
            runtime: Some([
                on,
                RuntimeSource { paused: true, ..on },
                RuntimeSource::default(),
            ]),
            free_bytes: Some(312_000_000_000),
            ..Default::default()
        };
        let rows = capture_rows(&m).unwrap();
        assert_eq!(rows[0].value, "Recording");
        assert_eq!(rows[1].value, "Paused while you’re away");
        assert_eq!(rows[2].value, "Off for this recording");
        assert_eq!(disk_row(&m).unwrap().value, "291 GB");
        assert_eq!(gb(3 * (1 << 30) / 2), "1.5");
        assert_eq!(gb(2 << 30), "2");
        m.free_bytes = Some(4_000_000_000);
        assert_eq!(disk_row(&m).unwrap().tone, Some(RowTone::Warn));
        m.running = false;
        assert_eq!(capture_rows(&m), None);
    }
}
