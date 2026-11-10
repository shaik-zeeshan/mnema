//! Answer prose (`AnswerProse.svelte`): raw markdown through gpui-kit's
//! TextView. While the turn streams: a still-open `**` is closed, code blocks
//! stay unhighlighted (highlighting runs once, when the turn settles), and a
//! static 7×15 accent caret marks the insertion point (never a blink).

use std::sync::{Arc, LazyLock};

use gpui_kit::base::text::{
    markdown_ast, CodeBlock, InlineElement, InlineRenderContext, MarkdownNode,
    MarkdownParseContext, MarkdownPlugin,
};
use gpui_kit::base::TextView;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, App, Div, ElementId, HighlightStyle, Window};

use super::format::close_dangling_bold;
use crate::theme::{self, text, Palette};

/// The caret box (`.answer-prose.is-streaming > :last-child::after`).
pub const CARET_W: gpui_kit::Pixels = px(7.);
pub const CARET_H: gpui_kit::Pixels = px(15.);
/// `vertical-align: -2px`: the caret's bottom sits 2 px under the baseline.
const CARET_BASELINE: gpui_kit::Pixels = px(13.);
const CARET_URL: &str = "mnema:caret";
const CARET_NODE: &str = "mnema-caret";

type Highlighter =
    dyn Fn(&CodeBlock) -> Vec<(std::ops::Range<usize>, HighlightStyle)> + Send + Sync;

/// One shared no-op highlighter: code stays plain while streaming, and the
/// same `Arc` every frame keeps the text view's highlight cache valid.
static NO_HIGHLIGHT: LazyLock<Arc<Highlighter>> = LazyLock::new(|| Arc::new(|_| Vec::new()));

/// Turns the `[](mnema:caret)` marker into the inline caret box.
struct CaretPlugin(gpui_kit::Hsla);

impl MarkdownPlugin for CaretPlugin {
    fn name(&self) -> &str {
        CARET_NODE
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        _: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        match node {
            markdown_ast::Node::Link(link) if link.url == CARET_URL => {
                Some(MarkdownNode::new(CARET_NODE, ()).text(""))
            }
            _ => None,
        }
    }

    fn render_inline(
        &self,
        _: &MarkdownNode,
        _: &InlineRenderContext,
        _: &mut Window,
        _: &mut App,
    ) -> Option<InlineElement> {
        let caret = div()
            .ml(px(2.))
            .w(CARET_W)
            .h(CARET_H)
            .rounded(px(1.))
            .bg(self.0);
        Some(InlineElement::new(caret).with_baseline(CARET_BASELINE))
    }
}

/// The source as rendered: while streaming, dangling bold closed and the caret
/// marker appended (not inside an open code fence, where it would print).
pub fn display_source(source: &str, streaming: bool) -> String {
    if !streaming {
        return source.to_string();
    }
    let closed = close_dangling_bold(source);
    let body = closed.trim_end();
    let fences = body
        .lines()
        .filter(|l| l.trim_start().starts_with("```") || l.trim_start().starts_with("~~~"))
        .count();
    let in_table = body
        .lines()
        .last()
        .is_some_and(|l| l.trim_start().starts_with('|'));
    if fences % 2 == 1 || in_table || body.is_empty() {
        return closed.into_owned();
    }
    format!("{body}[]({CARET_URL})")
}

pub fn prose(id: impl Into<ElementId>, source: &str, streaming: bool, p: &Palette) -> Div {
    let view = TextView::markdown(id, display_source(source, streaming))
        .plugin(CaretPlugin(p.accent))
        .on_link_click(crate::ui::link::prose_click);
    let view = if streaming {
        view.shared_code_block_highlighter(NO_HIGHLIGHT.clone())
    } else {
        view
    };
    div()
        .w_full()
        .min_w_0()
        .font_family(theme::FONT_UI)
        .text_size(text::MD)
        .line_height(relative(1.6))
        .text_color(p.text)
        .child(view)
}
