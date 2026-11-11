//! Step 1 of the Add connector picker (`McpConnectorCatalog.svelte`): the
//! search field, the hosted / local preset grids with their brand marks and
//! "✓ added" badges, and the dashed Custom tile. Search filters the tiles;
//! Enter picks the first one left (Custom when none matches).

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, Context, SharedString, Window};

use super::logic::{self, Preset, PRESETS};
use super::{form, oauth, preset, McpUi};
use crate::settings::{commands, Settings};
use crate::theme::{self, space, text, Palette};
use crate::ui::input::TextInput;

/// Lucide `sliders-vertical`, the Custom tile's glyph.
const CUSTOM_ICON: &str = "icons/sliders-vertical.svg";

/// The search field lives while the catalog shows (`McpUi::search`); a fresh
/// one (empty, focused) each time the catalog appears, as Svelte's remount.
pub fn ensure_search(s: &mut Settings, window: &mut Window, cx: &mut Context<Settings>) {
    if s.ui.mcp.search.is_some() {
        return;
    }
    let state = cx.new(|cx| InputState::new(window, cx).placeholder("Search services\u{2026}"));
    state.update(cx, |st, cx| st.focus(window, cx));
    let sub = cx.subscribe_in(&state, window, |s, _, e: &InputEvent, window, cx| match e {
        InputEvent::Change => cx.notify(),
        InputEvent::PressEnter { .. } => pick_first(s, window, cx),
        _ => {}
    });
    s.ui.mcp.search = Some((state, sub));
}

fn query(ui: &McpUi, cx: &gpui_kit::App) -> String {
    ui.search
        .as_ref()
        .map(|(st, _)| st.read(cx).value().to_string())
        .unwrap_or_default()
}

/// What a tile does: the OAuth preset opens its Connect stages, any other its
/// step-2 Connect body.
pub fn pick(s: &mut Settings, i: usize, window: &mut Window, cx: &mut Context<Settings>) {
    if PRESETS[i].oauth {
        oauth::open_preset(s, i, window, cx);
    } else {
        preset::open(s, i, None, window, cx);
    }
}

/// Enter in the search: the first visible tile, hosted before local, else Custom.
fn pick_first(s: &mut Settings, window: &mut Window, cx: &mut Context<Settings>) {
    let q = query(&s.ui.mcp, cx);
    let first = [true, false].into_iter().find_map(|hosted| {
        PRESETS
            .iter()
            .position(|p| p.hosted() == hosted && logic::matches(p, &q))
    });
    match first {
        Some(i) => pick(s, i, window, cx),
        None => form::open(s, None, window, cx),
    }
}

/// `.tile`: an icon box, then the name (+ badge) over the tagline.
fn tile(
    id: SharedString,
    icon: &'static str,
    label: &'static str,
    tagline: &'static str,
    added: bool,
    p: &Palette,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .role(gpui_kit::Role::Button)
        .aria_label(label)
        .flex()
        .items_center()
        .gap(px(11.))
        .py(px(11.))
        .px(space::S3)
        .rounded(px(10.))
        .border_1()
        .border_color(p.border)
        .bg(p.surface)
        .cursor_pointer()
        .hover(|st| st.bg(p.surface_hover).border_color(p.border_hover))
        .active(|st| st.bg(p.surface_active))
        .child(
            div()
                .size(px(30.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(7.))
                .border_1()
                .border_color(p.border_strong)
                .text_color(p.text_muted)
                .child(crate::ui::icon(icon, px(16.))),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .text_size(text::BASE)
                        .font_weight(theme::W_SEMIBOLD)
                        .text_color(p.text_strong)
                        .child(label)
                        .when(added, |d| {
                            // `.tile__badge--added`.
                            d.child(
                                div()
                                    .px(px(5.))
                                    .py(px(1.))
                                    .rounded(px(4.))
                                    .border_1()
                                    .border_color(p.accent_border)
                                    .bg(p.accent_bg)
                                    .text_color(p.accent)
                                    .text_size(px(9.))
                                    .child("\u{2713} ADDED"),
                            )
                        }),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .line_height(gpui_kit::relative(1.35))
                        .text_color(p.text_muted)
                        .child(tagline),
                ),
        )
}

fn group(title: &'static str, first: bool, tiles: Vec<AnyElement>, p: &Palette) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .when(!first, |d| d.mt(px(14.)))
        .child(
            div()
                .mb(space::S2)
                .ml(px(2.))
                .text_size(text::XS)
                .font_weight(theme::W_SEMIBOLD)
                .text_color(p.text_subtle)
                .child(SharedString::from(title.to_uppercase())),
        )
        .child(div().grid().grid_cols(2).gap(space::S2).children(tiles))
        .into_any_element()
}

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let q = query(&s.ui.mcp, cx);
    let servers = s.d().ai_runtime.mcp_servers;
    let tiles = |hosted: bool, cx: &mut Context<Settings>| -> Vec<AnyElement> {
        PRESETS
            .iter()
            .enumerate()
            .filter(|(_, pr)| pr.hosted() == hosted && logic::matches(pr, &q))
            .map(|(i, pr): (usize, &Preset)| {
                tile(
                    SharedString::from(format!("mcp-preset-{}", pr.id)),
                    pr.icon,
                    pr.label,
                    pr.tagline,
                    logic::added(pr, &servers),
                    p,
                )
                .on_click(commands::click_w(cx, move |s, window, cx| {
                    pick(s, i, window, cx)
                }))
                .test_support()
                .into_any_element()
            })
            .collect()
    };
    let hosted = tiles(true, cx);
    let local = tiles(false, cx);
    let none = hosted.is_empty() && local.is_empty();
    let has_hosted = !hosted.is_empty();
    let custom = tile(
        "mcp-preset-custom".into(),
        CUSTOM_ICON,
        "Custom",
        "Any other MCP server \u{2014} fill in every field yourself.",
        false,
        p,
    )
    .w_full()
    .border_dashed()
    .bg(gpui_kit::transparent_black())
    .on_click(commands::click_w(cx, |s, window, cx| {
        form::open(s, None, window, cx)
    }))
    .test_support();
    div()
        .flex()
        .flex_col()
        .px(space::S5)
        .pb(space::S5)
        .children(s.ui.mcp.search.as_ref().map(|(st, _)| {
            div()
                .mb(px(14.))
                .child(TextInput::new("mcp-catalog-search", "Search services", st))
        }))
        .when(has_hosted, |d| {
            d.child(group("Hosted \u{2014} just paste a token", true, hosted, p))
        })
        .when(!local.is_empty(), |d| {
            d.child(group("Runs on this Mac", !has_hosted, local, p))
        })
        .when(none, |d| {
            d.child(
                div()
                    .pt(space::S1)
                    .pb(space::S3)
                    .px(px(2.))
                    .text_size(text::SM)
                    .text_color(p.text_subtle)
                    .child(format!(
                        "No service matches \u{201c}{}\u{201d} \u{2014} add it yourself:",
                        q.trim()
                    )),
            )
        })
        .child(div().mt(space::S2).child(custom))
        .into_any_element()
}
