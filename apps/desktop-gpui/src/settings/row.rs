//! The two building blocks every section uses: `SettingGroup` (a head line —
//! title + hint + actions — over rows under a top border) and `SettingRow`
//! (copy left, control right, a hairline below; `full` drops the control to
//! its own line). Plus the small bits rows share: the state word, the source
//! dot, a muted hint paragraph.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Div, Hsla, SharedString};

use crate::theme::{self, space, text, Palette, FONT_UI};
use crate::ui::baseline::{self, Line};
use crate::ui::Tracking as _;

/// `.setting-group`: the head line, then the rows (`bare` = no top border).
pub fn section(
    title: &str,
    hint: Option<&str>,
    actions: Vec<AnyElement>,
    bare: bool,
    rows: Vec<AnyElement>,
    p: &Palette,
) -> Div {
    // Title and hint share the first line's baseline (a long hint wraps under
    // itself); the actions centre on the whole head.
    let mut words = vec![(
        Line::new(FONT_UI, text::MD, baseline::PHI),
        div()
            .flex_none()
            .font_weight(theme::W_SEMIBOLD)
            .tracking(-0.01)
            .text_color(p.text_strong)
            .child(SharedString::from(title.to_string())),
    )];
    words.extend(hint.map(|h| {
        (
            Line::new(FONT_UI, text::BASE, 1.5),
            div()
                .flex_1()
                .min_w_0()
                .text_color(p.text_subtle)
                .child(SharedString::from(h.to_string())),
        )
    }));
    let head = div()
        .flex()
        .items_center()
        .gap(space::S3)
        .pb(space::S2)
        .min_w_0()
        .child(baseline::row_top(words).gap(space::S3).flex_1().min_w_0())
        .when(!actions.is_empty(), |d| {
            d.child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(space::S2)
                    .children(actions),
            )
        });
    div().flex().flex_col().min_w_0().child(head).child(
        div()
            .flex()
            .flex_col()
            .when(!bare, |d| d.border_t_1().border_color(p.border))
            .children(rows),
    )
}

/// A source family's leading 8 px square (`.setting-row__src`).
#[derive(Clone, Copy)]
pub enum Src {
    Screen,
    Mic,
    SysAudio,
}

impl Src {
    pub fn color(self, p: &Palette) -> Hsla {
        match self {
            Src::Screen => p.source_screen,
            Src::Mic => p.source_mic,
            Src::SysAudio => p.source_sysaudio,
        }
    }
}

/// `SettingRow` as a builder; `.build(p)` renders it.
pub struct Row {
    label: SharedString,
    desc: Option<SharedString>,
    control: Vec<AnyElement>,
    aside: Vec<AnyElement>,
    below: Vec<AnyElement>,
    extra: Vec<AnyElement>,
    src: Option<Src>,
    full: bool,
    warn: bool,
    disabled: bool,
    dim_copy: bool,
    divider: bool,
}

pub fn row(label: impl Into<SharedString>) -> Row {
    Row {
        label: label.into(),
        desc: None,
        control: Vec::new(),
        aside: Vec::new(),
        below: Vec::new(),
        extra: Vec::new(),
        src: None,
        full: false,
        warn: false,
        disabled: false,
        dim_copy: false,
        divider: true,
    }
}

impl Row {
    pub fn desc(mut self, d: impl Into<SharedString>) -> Self {
        self.desc = Some(d.into());
        self
    }
    pub fn desc_opt(mut self, d: Option<impl Into<SharedString>>) -> Self {
        self.desc = d.map(Into::into);
        self
    }
    pub fn control(mut self, el: impl IntoElement) -> Self {
        self.control.push(el.into_any_element());
        self
    }
    pub fn aside(mut self, el: impl IntoElement) -> Self {
        self.aside.push(el.into_any_element());
        self
    }
    pub fn below(mut self, el: impl IntoElement) -> Self {
        self.below.push(el.into_any_element());
        self
    }
    /// Inline content after the label (a chip).
    pub fn below_opt(self, el: Option<impl IntoElement>) -> Self {
        match el {
            Some(el) => self.below(el),
            None => self,
        }
    }
    pub fn extra(mut self, el: impl IntoElement) -> Self {
        self.extra.push(el.into_any_element());
        self
    }
    pub fn src(mut self, s: Src) -> Self {
        self.src = Some(s);
        self
    }
    pub fn full(mut self) -> Self {
        self.full = true;
        self
    }
    pub fn warn(mut self, on: bool) -> Self {
        self.warn = on;
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    pub fn when(self, cond: bool, f: impl FnOnce(Self) -> Self) -> Self {
        if cond {
            f(self)
        } else {
            self
        }
    }
    /// `.mcp-row--off`: dim the copy only; the controls stay fully usable.
    pub fn dim_copy(mut self, on: bool) -> Self {
        self.dim_copy = on;
        self
    }
    pub fn no_divider(mut self) -> Self {
        self.divider = false;
        self
    }

    pub fn build(self, p: &Palette) -> Div {
        let label = div()
            .flex()
            .items_center()
            .gap(space::S2)
            .text_size(text::MD)
            .font_weight(theme::W_MEDIUM)
            .line_height(gpui_kit::relative(1.35))
            .text_color(if self.warn { p.warn } else { p.text_strong })
            .children(self.src.map(|s| {
                div()
                    .flex_none()
                    .size(px(8.))
                    .rounded(px(2.))
                    .bg(s.color(p))
            }))
            // One line, cut with an ellipsis: a bare text child keeps its full
            // width and runs past the row (a long provider or connector name).
            .child(
                div()
                    .id(SharedString::from(format!("row-label-{}", self.label)))
                    .min_w_0()
                    .truncate()
                    // The copy column never drops under 240 px, which fits a
                    // label this short: only a longer one can be cut.
                    .when(self.label.chars().count() > 28, |d| {
                        crate::ui::with_tip(d, self.label.clone())
                    })
                    .child(self.label),
            )
            .children(self.extra);
        let copy = div()
            .flex()
            .flex_col()
            .gap(px(3.))
            .flex_1()
            .min_w_0()
            .when(self.dim_copy, |d| d.opacity(theme::DISABLED_OPACITY))
            .child(label)
            .children(self.desc.map(|d| {
                div()
                    .text_size(text::BASE)
                    .line_height(gpui_kit::relative(1.5))
                    .text_color(p.text_muted)
                    .child(d)
            }))
            .when(!self.below.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(px(6.))
                        .mt(px(3.))
                        .children(self.below),
                )
            });
        let main = div()
            .flex()
            .items_center()
            .gap(space::S6)
            .flex_1()
            .min_w_0()
            .child(copy)
            .when(!self.aside.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(space::S2)
                        .children(self.aside),
                )
            });
        let control = div()
            .flex()
            .items_center()
            .gap(space::S2)
            .min_w_0()
            .map(|d| {
                if self.full {
                    d.children(self.control)
                } else {
                    // Each control at its own width, so a `w_full` one (Select)
                    // can't take the whole wrapped line.
                    d.children(self.control.into_iter().map(|c| div().flex_none().child(c)))
                }
            });
        div()
            .py(px(14.))
            .min_w_0()
            .when(self.divider, |d| d.border_b_1().border_color(p.hairline))
            .when(self.disabled, |d| d.opacity(theme::DISABLED_OPACITY))
            .map(|d| {
                if self.full {
                    d.flex()
                        .flex_col()
                        .gap(space::S3)
                        .child(main)
                        .child(control.w_full())
                } else {
                    // `flex: 1 1 240px`: when the copy would drop under 240 px the
                    // control wraps onto its own line (right-aligned) instead of
                    // squeezing the copy to a word a line.
                    d.flex()
                        .flex_wrap()
                        .items_center()
                        .gap_x(space::S6)
                        .gap_y(space::S3)
                        .child(main.flex_basis(px(240.)))
                        // `.setting-row__control`: its own items wrap at the row's width.
                        .child(
                            control
                                .flex_none()
                                .flex_wrap()
                                .justify_end()
                                .max_w_full()
                                .ml_auto(),
                        )
                }
            })
    }
}

/// `.settings-state`: a quiet word, `--ok` / `--warn` / `--danger` add a dot.
pub fn state_word(
    text_: impl Into<SharedString>,
    tone: Option<crate::ui::Tone>,
    p: &Palette,
) -> Div {
    use crate::ui::Tone;
    let color = match tone {
        Some(Tone::Ok) => p.accent,
        Some(Tone::Warn) => p.warn,
        Some(Tone::Danger) => p.danger_text,
        _ => p.text_subtle,
    };
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(text::BASE)
        .font_weight(theme::W_MEDIUM)
        .text_color(color)
        .when(tone.is_some(), |d| {
            d.child(
                div()
                    .flex_none()
                    .size(px(6.))
                    .rounded_full()
                    .bg(color)
                    .when(tone == Some(Tone::Ok), |d| {
                        d.shadow(vec![crate::ui::ring(p.accent_glow)])
                    }),
            )
        })
        .child(text_.into())
}

/// `.group-hint`: a muted paragraph inside a full-width control.
pub fn hint(text_: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .text_size(text::BASE)
        .tracking(0.03)
        .line_height(gpui_kit::relative(1.5))
        .text_color(p.text_subtle)
        .child(text_.into())
}

/// `.mx-inline` with an icon and one line of text.
pub fn line(
    tone: crate::ui::Tone,
    icon_path: &'static str,
    text_: impl Into<SharedString>,
    p: &Palette,
) -> Div {
    // `.setting-row__below .mx-inline`: a long line wraps its text, not the
    // icon onto a line of its own.
    crate::ui::notice::inline(
        tone,
        Some(
            div()
                .flex_none()
                .mt(px(3.))
                .child(crate::ui::icon(icon_path, theme::size::ICON_SM))
                .into_any_element(),
        ),
        vec![div().min_w_0().child(text_.into()).into_any_element()],
        p,
    )
    .flex_nowrap()
    .items_start()
    .max_w_full()
}
