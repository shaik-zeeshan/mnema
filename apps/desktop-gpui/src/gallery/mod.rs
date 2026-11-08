//! The control gallery (`cargo run -p mnema-gpui --bin gallery`): every shared
//! control × variant × state, one section per group. The goldens render one
//! group each (`gallery-<group>-<theme>.png`); hover / focus / press are pinned
//! with `ui::Force`, never a moved mouse.

mod buttons;
mod feedback;
mod fields;
mod overlays;
mod pickers;
mod status;

use gpui_kit::base::slider::SliderState;
use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, Div, Entity, ScrollHandle, SharedString, Window};
use time::Date;

use crate::theme::{self, space, text, Palette};
use crate::ui::{self, toast::Toasts};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Buttons,
    Fields,
    Choice,
    Select,
    Date,
    Overlays,
    Feedback,
    Empty,
    Loading,
    Status,
}

impl Group {
    pub const ALL: [Group; 10] = [
        Group::Buttons,
        Group::Fields,
        Group::Choice,
        Group::Select,
        Group::Date,
        Group::Overlays,
        Group::Feedback,
        Group::Empty,
        Group::Loading,
        Group::Status,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Group::Buttons => "buttons",
            Group::Fields => "fields",
            Group::Choice => "choice",
            Group::Select => "select",
            Group::Date => "date",
            Group::Overlays => "overlays",
            Group::Feedback => "feedback",
            Group::Empty => "empty",
            Group::Loading => "loading",
            Group::Status => "status",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Group::Buttons => "Buttons",
            Group::Fields => "Segmented · switch · input · stepper · slider",
            Group::Choice => "Choice",
            Group::Select => "Select · combobox",
            Group::Date => "Date input",
            Group::Overlays => "Popover · menu · tooltip · toast · modal",
            Group::Feedback => "Notice · banner · inline",
            Group::Empty => "Empty · error · offline",
            Group::Loading => "Skeleton · spinner · progress",
            Group::Status => "Chips · badge · keys · window strip",
        }
    }

    /// The golden's window size (logical px) for this group. Keep heights
    /// ≤1000: taller offscreen windows get clamped to the display.
    pub fn viewport(self) -> (f32, f32) {
        match self {
            Group::Buttons => (900., 440.),
            Group::Fields => (900., 700.),
            Group::Choice => (900., 640.),
            Group::Select => (900., 580.),
            Group::Date => (900., 640.),
            Group::Overlays => (900., 800.),
            Group::Feedback => (900., 980.),
            Group::Empty => (900., 820.),
            Group::Loading => (900., 600.),
            Group::Status => (900., 580.),
        }
    }
}

/// The gallery's live control state (the gallery window is interactive).
pub struct Gallery {
    pub(crate) groups: Vec<Group>,
    pub(crate) seg: [usize; 3],
    pub(crate) switches: [bool; 2],
    pub(crate) choice: [usize; 2],
    pub(crate) select: Option<usize>,
    pub(crate) combo: Option<usize>,
    pub(crate) day: Date,
    pub(crate) week: Date,
    pub(crate) month: Date,
    pub(crate) today: Date,
    pub(crate) inputs: Vec<Entity<InputState>>,
    pub(crate) sliders: Vec<Entity<SliderState>>,
    pub(crate) toasts: Entity<Toasts>,
    pub(crate) modal_open: bool,
    scroll: ScrollHandle,
}

impl Gallery {
    /// Render these groups, in order (the window: all; a golden: one).
    pub fn new(groups: Vec<Group>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let today = ui::clock::now(cx).date();
        let input = |text: &str, ph: &str, window: &mut Window, cx: &mut Context<Self>| {
            let (text, ph) = (text.to_string(), ph.to_string());
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(ph)
                    .default_value(text)
            })
        };
        let inputs = vec![
            input("", "Label", window, cx),
            input("Ana's MacBook", "Label", window, cx),
            input("Ana's MacBook", "Label", window, cx),
            input("not-an-email", "Email", window, cx),
            input("Disabled", "Label", window, cx),
            input("1,204", "", window, cx),
            input("30", "", window, cx),
            input("2560", "", window, cx),
            input("12", "", window, cx),
            input("90", "", window, cx),
            input("30", "", window, cx),
        ];
        let slider = |v: f32, cx: &mut Context<Self>| {
            cx.new(|_| {
                SliderState::new()
                    .min(0.)
                    .max(100.)
                    .step(1.)
                    .default_value(v)
            })
        };
        let sliders = vec![slider(40., cx), slider(72., cx), slider(25., cx)];
        Self {
            groups,
            seg: [1, 2, 0],
            switches: [true, false],
            choice: [0, 1],
            select: Some(1),
            combo: Some(2),
            day: today - time::Duration::days(2),
            week: today,
            month: today - time::Duration::days(40),
            today,
            inputs,
            sliders,
            toasts: cx.new(|_| Toasts::default()),
            modal_open: false,
            scroll: ScrollHandle::new(),
        }
    }

    fn group(&mut self, g: Group, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match g {
            Group::Buttons => buttons::render(self, window, cx),
            Group::Fields => fields::render(self, window, cx),
            Group::Choice => pickers::choice(self, window, cx),
            Group::Select => pickers::select(self, window, cx),
            Group::Date => pickers::date(self, window, cx),
            Group::Overlays => overlays::render(self, window, cx),
            Group::Feedback => feedback::render(self, window, cx),
            Group::Empty => feedback::empties(self, window, cx),
            Group::Loading => feedback::loading(self, window, cx),
            Group::Status => status::render(self, window, cx),
        }
    }
}

impl Render for Gallery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        theme::request_fade_frames(window, cx);
        let p = theme::tokens(cx);
        let groups = self.groups.clone();
        let sections: Vec<AnyElement> = groups
            .into_iter()
            .map(|g| section(g.title(), self.group(g, window, cx), &p).into_any_element())
            .collect();
        div()
            .id("gallery")
            .relative()
            .size_full()
            .bg(p.bg)
            .font_family(theme::FONT_UI)
            .text_size(text::MD)
            .text_color(p.text)
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(space::S6)
                    .p(space::S5)
                    .children(sections),
            )
            .child(self.toasts.clone())
    }
}

/// A group: heading + body.
fn section(title: &'static str, body: AnyElement, p: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space::S4)
        .child(
            div()
                .font_weight(theme::W_SEMIBOLD)
                .text_size(text::XL)
                .line_height(gpui_kit::relative(1.25))
                .text_color(p.text_strong)
                .child(title),
        )
        .child(body)
}

/// A labelled row: a 96 px kicker column, then the specimens.
pub(crate) fn row(label: &str, items: impl IntoIterator<Item = AnyElement>, p: &Palette) -> Div {
    div()
        .flex()
        .items_start()
        .gap(space::S3)
        // Level with a 32 px control under its caption.
        .child(
            div()
                .flex_none()
                .w(px(96.))
                .pt(px(26.))
                .when(!label.is_empty(), |d| d.child(ui::kicker(label, p))),
        )
        .children(items)
}

/// One specimen with a mono caption above it (the state it shows).
pub(crate) fn cell(
    caption: impl Into<SharedString>,
    el: impl IntoElement,
    p: &Palette,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(6.))
        .child(
            div()
                .font_family(theme::FONT_MONO)
                .text_size(text::XS)
                .text_color(p.text_subtle)
                .child(caption.into()),
        )
        .child(el)
        .into_any_element()
}

/// A bordered panel (`.mx-panel`) to set specimens on.
pub(crate) fn panel(p: &Palette) -> Div {
    div()
        .bg(p.surface)
        .border_1()
        .border_color(p.border)
        .rounded(theme::radius::LG)
}
