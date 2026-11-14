//! The release notes band's two renderers: GitHub's notes as blocks, anything
//! else as Markdown (`.up-notes`).

use gpui_kit::base::TextViewStyle;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, Context};

use super::{model, UpdateWindow};
use crate::theme::{self, space, text, Palette};

/// `.up-notes`: muted body, strong 13 px headings, links in body ink.
/// `.up-notes` for GitHub's notes: a 13 px heading, 12/1.5 bullets 5 px apart
/// (faint markers), and the closing meta line at 11 px subtle with its link.
pub(super) fn notes_view(
    blocks: Vec<model::NoteBlock>,
    cx: &mut Context<UpdateWindow>,
    p: &Palette,
) -> gpui_kit::Div {
    use model::NoteBlock;
    let mut first = true;
    div()
        .flex()
        .flex_col()
        .children(blocks.into_iter().map(|b| {
            let top = !std::mem::take(&mut first);
            match b {
                NoteBlock::Heading(h) => div()
                    .when(top, |d| d.mt(px(4.)))
                    .mb(px(6.))
                    .font_weight(theme::W_SEMIBOLD)
                    .text_size(text::MD)
                    .line_height(relative(1.3))
                    .text_color(p.text_strong)
                    .child(h)
                    .into_any_element(),
                NoteBlock::List(items) => {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(5.))
                        .mb(space::S2)
                        .children(items.into_iter().map(|t| {
                            div()
                                .flex()
                                .text_size(text::BASE)
                                .line_height(relative(1.5))
                                .text_color(p.text_muted)
                                .child(
                                    // `li::marker`: a faint round bullet.
                                    div()
                                        .w(px(18.))
                                        .flex_none()
                                        .pl(px(6.))
                                        .pt(px(7.))
                                        .child(div().size(px(4.)).rounded_full().bg(p.text_faint)),
                                )
                                .child(div().flex_1().min_w_0().child(t))
                        }))
                        .into_any_element()
                }
                NoteBlock::Para(words, link) => {
                    // One text run so the link wraps inline after the words, as in CSS.
                    let url = link.unwrap_or_default();
                    let open = url.clone();
                    let mut highlights = Vec::new();
                    if !url.is_empty() {
                        highlights.push((
                            words.len()..words.len() + url.len(),
                            gpui_kit::HighlightStyle {
                                color: Some(p.text),
                                underline: Some(gpui_kit::UnderlineStyle {
                                    thickness: px(1.),
                                    color: Some(p.border_hover),
                                    wavy: false,
                                }),
                                ..Default::default()
                            },
                        ));
                    }
                    div()
                        .id("up-notes-link")
                        .when(top, |d| d.mt(space::S2))
                        .text_size(text::SM)
                        .line_height(relative(1.4))
                        .text_color(p.text_subtle)
                        .child(
                            gpui_kit::StyledText::new(format!("{words}{url}"))
                                .with_highlights(highlights),
                        )
                        .when(!open.is_empty(), |d| {
                            d.role(gpui_kit::Role::Link)
                                .cursor_pointer()
                                // Feed notes aren't covered by the update signature: gate the URL.
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    crate::ui::link::open(&open, cx);
                                }))
                        })
                        .into_any_element()
                }
            }
        }))
}

pub(super) fn notes_style(p: &Palette, dark: bool) -> TextViewStyle {
    let strong = p.text_strong;
    TextViewStyle::default()
        .with_dark(dark)
        .with_foreground(p.text_muted)
        .with_muted_foreground(p.text_subtle)
        .with_link(p.text)
        .with_border(p.border)
        .with_code_background(p.wash)
        .with_heading(move |_| {
            gpui_kit::StyleRefinement::default()
                .text_size(text::MD)
                .font_weight(theme::W_SEMIBOLD)
                .text_color(strong)
        })
}
