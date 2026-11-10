//! One file per section (appendix D). Each renders its `SettingGroup`(s) from
//! the drafts and wires its controls to `Settings` through `super::on` /
//! `super::click`. `render` picks the active group's sections, in rail order.

mod access;
mod appearance;
mod ask_ai;
mod delete_recent;
mod developer;
mod languages;
mod license;
pub(crate) mod mcp;
mod microphone;
mod model_row;
mod ocr;
mod privacy;
pub(crate) mod providers;
mod semantic_search;
mod shortcuts;
mod sources;
mod speakers;
mod startup;
mod storage;
mod transcription;
mod updates;
mod user_context;
mod video;
mod voice;
mod voice_take;
#[cfg(test)]
mod pickers_tests;

use gpui_kit::prelude::*;
use gpui_kit::{div, AnyElement, Context, Window};

use super::groups::{GroupId, SectionId};
use super::Settings;

/// `(section, block)` for the active group, in render order.
pub fn render(
    s: &Settings,
    window: &mut Window,
    cx: &mut Context<Settings>,
) -> Vec<(SectionId, AnyElement)> {
    let p = crate::theme::tokens(cx);
    let stack = |els: Vec<AnyElement>| {
        div()
            .flex()
            .flex_col()
            .gap(crate::theme::space::S7)
            .children(els)
            .into_any_element()
    };
    match s.group {
        GroupId::General => vec![
            (SectionId::Appearance, appearance::render(s, &p, cx)),
            (SectionId::Startup, startup::render(s, &p, cx)),
            (SectionId::Shortcuts, shortcuts::render(s, &p, cx)),
        ],
        GroupId::Capture => vec![
            (SectionId::Capture, stack(sources::render(s, &p, cx))),
            (SectionId::Video, stack(video::render(s, &p, window, cx))),
            (SectionId::Audio, microphone::render(s, &p, cx)),
            (SectionId::Privacy, privacy::render(s, &p, cx)),
        ],
        GroupId::Intelligence => vec![
            (
                SectionId::Intelligence,
                stack(vec![
                    providers::render(s, &p, window, cx),
                    mcp::render(s, &p, cx),
                ]),
            ),
            (SectionId::AskAi, ask_ai::render(s, &p, cx)),
            (SectionId::UserContext, user_context::render(s, &p, cx)),
            (SectionId::Ocr, ocr::render(s, &p, cx)),
            (SectionId::Transcription, transcription::render(s, &p, cx)),
            (
                SectionId::Speakers,
                stack(vec![
                    speakers::render(s, &p, cx),
                    voice::render(s, &p, window, cx),
                ]),
            ),
            (
                SectionId::SemanticSearch,
                semantic_search::render(s, &p, window, cx),
            ),
        ],
        GroupId::Data => vec![
            (
                SectionId::Storage,
                stack(vec![
                    storage::render(s, &p, cx),
                    delete_recent::render(s, &p, cx),
                ]),
            ),
            (SectionId::Access, access::render(s, &p, cx)),
        ],
        GroupId::About => vec![
            (SectionId::License, license::render(s, &p, cx)),
            (SectionId::About, stack(updates::render(s, &p, cx))),
            (SectionId::Developer, developer::render(s, &p, cx)),
        ],
    }
}

/// Modals over the window (the MCP connector picker).
pub fn overlays(s: &Settings, _: &mut Window, cx: &mut Context<Settings>) -> Option<AnyElement> {
    let p = crate::theme::tokens(cx);
    mcp::picker(s, &p, cx)
}

/// A listening shortcut row takes the key (true = consumed).
pub fn capture_key(s: &mut Settings, k: &gpui_kit::Keystroke, cx: &mut Context<Settings>) -> bool {
    shortcuts::capture(s, k, cx)
}

/// Index of `value` in `options` (0 when absent).
pub fn ix<T: PartialEq>(options: &[T], value: &T) -> usize {
    options.iter().position(|o| o == value).unwrap_or(0)
}
