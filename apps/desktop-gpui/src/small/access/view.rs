//! The request window's render: strip, then one of loading / load failed /
//! nothing waiting / the request / the receipt, the error line, and the footer.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, size, AnyElement, Context, KeyUpEvent, Render, Window};

use super::model::{identity_source, request_age, split_command, Receipt, ReceiptKind, Scope};
use super::{AccessRequest, Acting, Phase};
use crate::small::parts::{b, body_sm, frame, h2, inset_panel, note, para};
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::choice::{Choice, ChoiceOption};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::window_strip::WindowStrip;
use crate::ui::{icon, kicker, notice, skeleton, Button, Tone};

const TERMINAL: &str = "icons/square-terminal.svg";
const ALERT: &str = "icons/triangle-alert.svg";
const OCTAGON: &str = "icons/octagon-alert.svg";
const CLOCK: &str = "icons/clock.svg";
const CHECK: &str = "icons/check.svg";
const BAN: &str = "icons/ban.svg";
const ARROW: &str = "icons/arrow-right.svg";
const RETRY: &str = "icons/rotate-cw.svg";
const LOCK: &str = "icons/lock.svg";

fn is_enter(key: &str) -> bool {
    key == "enter"
}

impl AccessRequest {
    fn view_key(&self) -> String {
        match &self.phase {
            Phase::Receipt(r) => format!("receipt:{:?}", r.kind),
            Phase::Loading => "loading".into(),
            Phase::LoadFailed(_) => "error".into(),
            Phase::Empty => "none".into(),
            Phase::Request(r) => format!("request:{}", r.id),
        }
    }

    fn header(&self, p: &Palette) -> AnyElement {
        let r = self.request().expect("request");
        let widen = r.is_widen();
        let (k, title) = if widen {
            (
                "More history needed",
                format!("{} needs more of your history", r.tool),
            )
        } else {
            (
                "Access request",
                format!("{} wants to read your Mnema history", r.tool),
            )
        };
        let lede = match (widen, r.current) {
            (true, Some(c)) => format!(
                "Its command reaches back further than {}. Allowing updates the permission it already has \u{2014} results it\u{2019}s holding keep working.",
                c.prose()
            ),
            _ => "A command-line tool is asking for standing access to your searchable Mnema text."
                .into(),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(kicker(k, p))
            .child(h2(title, p))
            .child(body_sm(lede, p))
            .into_any_element()
    }

    fn who(&self, now_ms: i64, p: &Palette) -> AnyElement {
        let r = self.request().expect("request");
        let (chip_label, _detail, warn) = identity_source(&r.source);
        let (head, rest) = split_command(&r.command);
        let age = request_age(&r.created_at, now_ms);
        let top = div()
            .flex()
            .items_center()
            .gap(space::S3)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(32.))
                    .rounded(radius::MD)
                    .bg(p.wash)
                    .border_1()
                    .border_color(p.hairline)
                    .text_color(p.text_muted)
                    .child(icon(TERMINAL, px(16.))),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(space::S2)
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(theme::W_SEMIBOLD)
                            .text_size(text::LG)
                            .line_height(relative(1.3))
                            .text_color(p.text_strong)
                            .child(r.tool.clone()),
                    )
                    .child(chip(
                        chip_label,
                        if warn {
                            ChipKind::Warn
                        } else {
                            ChipKind::Neutral
                        },
                        p,
                    )),
            )
            .children(age.map(|a| {
                div()
                    .ml_auto()
                    .flex_none()
                    .font_family(FONT_MONO)
                    .text_size(text::SM)
                    .text_color(p.text_subtle)
                    .whitespace_nowrap()
                    .child(a)
            }));
        let cmd = div()
            .px(px(10.))
            .py(px(7.))
            .rounded(radius::SM)
            .bg(p.surface_subtle)
            .border_1()
            .border_color(p.hairline)
            .font_family(FONT_MONO)
            .text_size(text::SM)
            .line_height(relative(1.45))
            .text_color(p.text)
            .child(crate::chat::inline_runs(&[
                (&head, Some((theme::W_MEDIUM, p.text_strong))),
                (
                    &if rest.is_empty() {
                        String::new()
                    } else {
                        format!(" {rest}")
                    },
                    None,
                ),
            ]));
        let warn_note = note(
            Tone::Warn,
            ALERT,
            para(
                &[
                    ("This name is reported by the tool and can\u{2019}t be verified. Any program that can run ", None),
                    ("mnema", b(p)),
                    (" can use it.", None),
                ],
                text::BASE,
                p.text_muted,
            ),
            p,
        );
        inset_panel(p)
            .flex()
            .flex_col()
            .gap(space::S3)
            .px(space::S4)
            .py(px(14.))
            .child(top)
            .child(cmd)
            .child(warn_note)
            .into_any_element()
    }

    fn scopes(&self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let r = self.request().expect("request").clone();
        let blocked = self.acting != Acting::None || self.gone;
        let options = Scope::ALL
            .iter()
            .map(|s| {
                let tag = r.tag(*s);
                let o = ChoiceOption::new(s.label())
                    .desc(s.desc())
                    .disabled(r.disabled(*s) || blocked);
                if tag.is_empty() {
                    o
                } else {
                    o.tag(tag)
                }
            })
            .collect();
        let selected = Scope::ALL
            .iter()
            .position(|s| *s == self.selected)
            .unwrap_or(0);
        let this = cx.entity().downgrade();
        let choice = Choice::new("ac-scope", "What it can read", options, selected)
            .list()
            .on_change(move |ix, _, cx| {
                let _ = this.update(cx, |v, cx| v.select(Scope::ALL[ix], cx));
            });
        let was = match (r.is_widen(), r.current) {
            (true, Some(c)) => Some(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(space::S2)
                    .mb(space::S2)
                    .text_size(text::BASE)
                    .text_color(p.text_muted)
                    .child("Already allowed")
                    .child(chip(c.label(), ChipKind::Neutral, p))
                    .child(icon(ARROW, px(13.)))
                    .child("now asks for")
                    .child(chip(r.preferred.label(), ChipKind::Accent, p)),
            ),
            _ => None,
        };
        let narrows = r.narrows(self.selected).map(|held| {
            note(
                Tone::Danger,
                ALERT,
                para(
                    &[
                        ("This reduces access it already has. ", b(p)),
                        (
                            &format!(
                                "{} can currently read {}; allowing at this setting cuts it back to {}.",
                                r.tool,
                                held.prose(),
                                self.selected.prose()
                            ),
                            None,
                        ),
                    ],
                    text::BASE,
                    p.text_muted,
                ),
                p,
            )
            .mt(space::S2)
        });
        let after = if self.selected == Scope::AllRetained {
            note(
                Tone::Danger,
                OCTAGON,
                para(
                    &[
                        ("Reads your entire history. ", b(p)),
                        ("Everything Mnema has retained \u{2014} every app, every window, every transcript.", None),
                    ],
                    text::BASE,
                    p.text_muted,
                ),
                p,
            )
            .mt(space::S2)
            .into_any_element()
        } else {
            let hint = r.hint(self.selected);
            div()
                .when(!hint.is_empty(), |d| {
                    d.mt(space::S2)
                        .text_size(text::SM)
                        .text_color(p.text_subtle)
                        .child(hint)
                })
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap(space::S2)
            .child(
                div()
                    .mb(space::S2)
                    .font_weight(theme::W_MEDIUM)
                    .text_size(text::BASE)
                    .text_color(p.text_muted)
                    .child("What it can read"),
            )
            .children(was)
            .child(choice)
            .children(narrows)
            .child(after)
            .into_any_element()
    }

    fn standing(&self, p: &Palette) -> AnyElement {
        let tool = &self.request().expect("request").tool;
        note(
            Tone::Neutral,
            CLOCK,
            para(
                &[
                    ("Standing until idle. ", b(p)),
                    (
                        &format!("{tool} keeps this until you block it. If it goes unused for 30 days, the permission lapses and the next request asks again."),
                        None,
                    ),
                ],
                text::BASE,
                p.text_muted,
            ),
            p,
        )
        .into_any_element()
    }

    fn receipt(r: &Receipt, p: &Palette) -> AnyElement {
        let broad = r.kind == ReceiptKind::Granted && r.scope == Scope::AllRetained;
        let danger = r.kind == ReceiptKind::Blocked || broad;
        let (t, bg, bd) = if danger {
            (p.danger_strong, p.danger_bg, p.danger_border)
        } else {
            (p.accent, p.accent_bg, p.accent_border)
        };
        let glyph = match (r.kind, broad) {
            (ReceiptKind::Blocked, _) => BAN,
            (_, true) => OCTAGON,
            _ => CHECK,
        };
        let prose = |runs: &[(&str, bool)]| {
            let runs: Vec<_> = runs
                .iter()
                .map(|(s, strong)| (*s, if *strong { b(p) } else { None }))
                .collect();
            para(&runs, text::MD, p.text_muted).line_height(relative(1.6))
        };
        let (title, body) = match r.kind {
            ReceiptKind::Blocked => (
                format!("{} is blocked", r.tool),
                prose(&[(
                    &format!(
                        "Its requests are refused right away \u{2014} this window won\u{2019}t open for it again.{} Unblock it any time.",
                        r.current.map_or(String::new(), |c| format!(" Its {} access stops too.", c.label()))
                    ),
                    false,
                )]),
            ),
            ReceiptKind::Granted if r.widen => (
                format!("{} can now read {}", r.tool, r.scope.prose()),
                prose(&[("Same permission, widened in place. Results it already has keep working.", false)]),
            ),
            ReceiptKind::Granted => (
                "Access granted".into(),
                prose(&[
                    (&r.tool, true),
                    (" can read ", false),
                    (r.scope.prose(), true),
                    (" until you block it. Unused for 30 days, it lapses.", false),
                ]),
            ),
        };
        div()
            .id("ac-receipt")
            .role(gpui_kit::Role::Status)
            .flex_1()
            .flex()
            .flex_col()
            .items_start()
            .gap(space::S2)
            .pt(space::S6)
            .px(space::S5)
            .pb(space::S5)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(36.))
                    .mb(space::S1)
                    .rounded_full()
                    .bg(bg)
                    .border_1()
                    .border_color(bd)
                    .text_color(t)
                    .child(icon(glyph, px(18.))),
            )
            .child(h2(title, p))
            .child(body)
            .child(body_sm(
                "Manage it in Settings \u{2192} Data \u{2192} CLI Access.",
                p,
            ))
            .into_any_element()
    }

    fn loading(p: &Palette) -> AnyElement {
        div()
            .id("ac-loading")
            .role(gpui_kit::Role::Status)
            .aria_label("Loading access request")
            .flex()
            .flex_col()
            .gap(space::S4)
            .pt(space::S5)
            .px(space::S5)
            .pb(space::S4)
            .child(skeleton::title(p))
            .child(
                inset_panel(p)
                    .px(space::S4)
                    .py(px(14.))
                    .child(skeleton::row(true, 0.5, false, p)),
            )
            .child(skeleton::lines(3, p))
            .into_any_element()
    }

    fn footer(&self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let this = cx.entity().downgrade();
        let act = move |f: fn(&mut AccessRequest, &mut Window, &mut Context<AccessRequest>)| {
            let this = this.clone();
            move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut gpui_kit::App| {
                let _ = this.update(cx, |v, cx| f(v, window, cx));
            }
        };
        let busy = self.acting != Acting::None;
        let foot = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(space::S2)
            .px(space::S5)
            .pt(space::S3)
            .pb(space::S4)
            .border_t_1()
            .border_color(p.hairline);
        let spacer = || div().flex_1();
        match &self.phase {
            Phase::Receipt(_) => foot
                .child(spacer())
                .child(
                    Button::new("ac-receipt-close", "Close")
                        .focus_handle(&self.close_focus)
                        .on_click(|_, window, _| window.remove_window()),
                )
                .into_any_element(),
            Phase::Request(r) => {
                let deny_label = match (self.acting, self.gone) {
                    (Acting::Cancelling, true) => "Closing\u{2026}",
                    (Acting::Cancelling, false) => "Denying\u{2026}",
                    (_, true) => "Close",
                    _ => "Deny",
                };
                let mut deny = Button::new("ac-deny", deny_label)
                    .focus_handle(&self.deny_focus)
                    .disabled(busy)
                    .spinner(self.acting == Acting::Cancelling)
                    .on_click(act(AccessRequest::deny));
                if !self.gone && self.acting != Acting::Cancelling {
                    deny = deny.kbd("Esc");
                }
                let blocking = self.acting == Acting::Blocking;
                let approving = self.acting == Acting::Approving;
                let mut allow = Button::new(
                    "ac-allow",
                    if approving {
                        "Allowing\u{2026}".to_string()
                    } else {
                        r.allow_label(self.selected)
                    },
                )
                .disabled(busy || self.gone)
                .busy(approving)
                .spinner(approving)
                .on_click(act(AccessRequest::approve));
                if !approving {
                    // Deliberately NOT primary: a consent click gets no green pull.
                    allow = allow.icon(LOCK).accent_icon();
                }
                let mut block = Button::new(
                    "ac-block",
                    if blocking {
                        "Blocking\u{2026}"
                    } else {
                        "Block"
                    },
                )
                .ghost_danger()
                .disabled(busy || self.gone)
                .busy(blocking)
                .spinner(blocking)
                .tip("Refuse this tool now and every time \u{2014} no window, no asking")
                .on_click(act(AccessRequest::block));
                if !blocking {
                    block = block.icon(BAN);
                }
                foot.child(block)
                    .child(spacer())
                    .child(deny)
                    .child(allow)
                    .into_any_element()
            }
            _ => foot
                .child(spacer())
                .child(
                    Button::new(
                        "ac-close",
                        if self.acting == Acting::Cancelling {
                            "Closing\u{2026}"
                        } else {
                            "Close"
                        },
                    )
                    .focus_handle(&self.deny_focus)
                    .disabled(busy)
                    .on_click(act(AccessRequest::deny)),
                )
                .into_any_element(),
        }
    }

    /// Fit the window to the dialog's natural height (`fitAccessWindowHeight`).
    /// Fit the window to the dialog's natural height (`fitAccessWindowHeight`):
    /// a new view fits exactly, within one view it only grows, capped by the display.
    fn fit_window(&mut self, natural: f32, window: &mut Window, cx: &mut gpui_kit::App) {
        let view = self.view_key();
        let same = view == self.fitted_view;
        self.fitted_view = view;
        let vp = window.viewport_size();
        let max = window
            .display(cx)
            .map_or(900., |d| f32::from(d.bounds().size.height) - 48.);
        if let Some(h) = super::model::fit_height(f32::from(vp.height), natural, same, max) {
            window.resize(size(vp.width, px(h.max(super::MIN.1))));
        }
    }
}

impl Render for AccessRequest {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::theme::request_fade_frames(window, cx);
        let p = theme::tokens(cx);
        let now_ms = crate::ui::clock::now_ms(cx);
        let this = cx.entity().downgrade();
        let strip = WindowStrip::new("CLI Access").on_close(move |window, cx| {
            let _ = this.update(cx, |v, cx| {
                if matches!(v.phase, Phase::Receipt(_)) {
                    window.remove_window();
                } else {
                    v.deny(window, cx);
                }
            });
        });
        let body: AnyElement = match &self.phase {
            Phase::Receipt(r) => Self::receipt(r, &p),
            Phase::Loading => Self::loading(&p),
            Phase::LoadFailed(_) => {
                let this = cx.entity().downgrade();
                empty(
                    EmptyLayout::Panel,
                    Tone::Danger,
                    ALERT,
                    "Couldn\u{2019}t load the request",
                    "The access service didn\u{2019}t answer. The tool is still waiting \u{2014} try again.",
                    vec![Button::new("ac-retry", "Retry")
                        .sm()
                        .icon(RETRY)
                        .on_click(move |_, window, cx| {
                            let _ = this.update(cx, |v, cx| v.reload(window, cx));
                        })
                        .into_any_element()],
                    &p,
                )
                .px(space::S5)
                .into_any_element()
            }
            Phase::Empty => empty(
                EmptyLayout::Panel,
                Tone::Neutral,
                CLOCK,
                "No request waiting",
                "There\u{2019}s no pending CLI Access request right now. You can close this window.",
                vec![],
                &p,
            )
            .px(space::S5)
            .into_any_element(),
            Phase::Request(_) => div()
                .id("ac-body")
                .track_scroll(&self.body_scroll)
                .flex_shrink_1()
                .min_h_0()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(space::S4)
                .pt(space::S5)
                .px(space::S5)
                .pb(space::S4)
                .child(self.header(&p))
                .child(self.who(now_ms, &p))
                .child(self.scopes(cx, &p))
                .child(self.standing(&p))
                .into_any_element(),
        };
        let error = self
            .error
            .clone()
            .filter(|_| !matches!(self.phase, Phase::Receipt(_)))
            .map(|e| {
                div()
                    .flex_none()
                    .px(space::S5)
                    .pb(space::S3)
                    .child(notice::inline(
                        Tone::Danger,
                        Some(icon(ALERT, px(13.)).into_any_element()),
                        vec![div().child(e).into_any_element()],
                        &p,
                    ))
            });
        let fit = self.fit;
        let this = cx.entity().downgrade();
        let dialog = div()
            .id("ac-dialog")
            .role(gpui_kit::Role::Dialog)
            .aria_label("CLI Access request")
            .w_full()
            .max_h_full()
            .flex()
            .flex_col()
            .child(strip)
            .child(body)
            .children(error)
            .child(self.footer(cx, &p));
        frame(&p)
            .key_context("AccessRequest")
            // Enter presses nothing here: a reflexive Return must never answer a consent sheet.
            .capture_key_down(|e, window, _| {
                if is_enter(&e.keystroke.key) {
                    window.prevent_default();
                }
            })
            .capture_key_up(|e: &KeyUpEvent, window, _| {
                if is_enter(&e.keystroke.key) {
                    window.prevent_default();
                }
            })
            .on_key_down(cx.listener(Self::on_key))
            .child(dialog)
            .when(fit, |el| {
                let scroll = self.body_scroll.clone();
                el.on_children_prepainted(move |bounds, window, cx| {
                    let Some(d) = bounds.first().copied() else {
                        return;
                    };
                    // What the body still hides under the (invisible) overlay scrollbar.
                    let natural = f32::from(d.size.height + scroll.max_offset().y);
                    let (this, handle) = (this.clone(), window.window_handle());
                    cx.defer(move |cx| {
                        let _ = handle.update(cx, |_, window, cx| {
                            let _ = this.update(cx, |v, cx| v.fit_window(natural, window, cx));
                        });
                    });
                })
            })
    }
}
