use std::time::{Duration, Instant};

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    App, ClickEvent, Div, ElementId, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Pixels, SharedString, Stateful, StatefulInteractiveElement, Styled, TextRun,
    Window, div, font, prelude::FluentBuilder, px,
};

use super::theme;

type OnSelect = std::rc::Rc<dyn Fn(&mut Window, &mut App)>;

pub fn text_width(window: &Window, text: &str, size: Pixels, bold: bool) -> Pixels {
    let mut font = font(theme::current_ui_family());
    font.fallbacks = Some(theme::fallbacks());
    if bold {
        font.weight = FontWeight::BOLD;
    }
    let run = TextRun {
        len: text.len(),
        font,
        color: theme::FG.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text.to_owned().into(), size, &[run], None)
        .width
}

pub fn widest(window: &Window, labels: &[&str], size: Pixels) -> Pixels {
    labels
        .iter()
        .map(|label| text_width(window, label, size, false))
        .fold(px(0.0), Pixels::max)
}

pub fn tooltip(
    text: impl Into<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> gpui_kit::AnyView {
    let text = text.into();
    move |window, cx| Tooltip::new(text.clone()).build(window, cx)
}

const ICON_GAP: Pixels = px(6.0);

pub fn icon_button_width(window: &Window, buttons: &[(&str, &str)]) -> Pixels {
    buttons
        .iter()
        .map(|(icon, label)| {
            text_width(window, icon, theme::BODY, false)
                + ICON_GAP
                + text_width(window, label, theme::BODY, false)
        })
        .fold(px(0.0), Pixels::max)
        + theme::BUTTON_PADDING * 2.0
}

fn button_frame(id: impl Into<ElementId>, enabled: bool, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .h(theme::INTERACT_HEIGHT)
        .px(theme::BUTTON_PADDING)
        .text_size(theme::BODY)
        .border_1()
        .border_color(theme::alpha(theme::BLUE, 0.0))
        .bg(if selected { theme::SEL } else { theme::BG_HL })
        .text_color(if enabled { theme::FG } else { theme::COMMENT })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(theme::SEL).border_color(theme::BLUE))
                .active(|style| style.bg(theme::SEL).border_color(theme::BLUE))
        })
}

pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    button_frame(id, enabled, false)
        .child(label.into())
        .when(enabled, |this| this.on_click(on_click))
}

pub fn icon_button(
    id: impl Into<ElementId>,
    icon: &'static str,
    label: impl Into<SharedString>,
    width: Pixels,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    button_frame(id, enabled, false)
        .w(width)
        .justify_center()
        .gap(ICON_GAP)
        .child(icon)
        .child(label.into())
        .when(enabled, |this| this.on_click(on_click))
}

pub struct Slide {
    open: bool,
    from: f32,
    since: Instant,
    length: Duration,
}

impl Slide {
    pub fn new(length: Duration) -> Self {
        Self {
            open: false,
            from: 0.0,
            since: Instant::now() - length,
            length,
        }
    }

    pub fn set(&mut self, open: bool) {
        if open != self.open {
            self.from = self.value_at(Instant::now());
            self.open = open;
            self.since = Instant::now();
        }
    }


    fn value_at(&self, now: Instant) -> f32 {
        let t = (now.duration_since(self.since).as_secs_f32() / self.length.as_secs_f32()).min(1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        let to = if self.open { 1.0 } else { 0.0 };
        self.from + (to - self.from) * eased
    }

    pub fn value(&self, window: &mut Window) -> f32 {
        if self.since.elapsed() < self.length {
            window.request_animation_frame();
        }
        self.value_at(Instant::now())
    }
}

pub const SLIDE_LENGTH: Duration = Duration::from_millis(150);

#[derive(Clone, Copy)]
pub struct SlideButton {
    pub icon: &'static str,
    pub label: &'static str,
    pub enabled: bool,
    pub selected: bool,
}

pub fn slide_button(
    id: impl Into<ElementId>,
    button: SlideButton,
    open: f32,
    window: &Window,
    on_hover: impl Fn(&bool, &mut Window, &mut App) + 'static,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let SlideButton {
        icon,
        label,
        enabled,
        selected,
    } = button;
    let label_width = text_width(window, label, theme::BODY, false);
    let shown = (ICON_GAP + label_width) * open;
    button_frame(id, enabled, selected)
        .overflow_hidden()
        .on_hover(on_hover)
        .child(icon)
        .child(
            div()
                .w(shown)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(theme::alpha(
                    if enabled { theme::FG } else { theme::COMMENT },
                    open,
                ))
                .child(div().pl(ICON_GAP).child(label)),
        )
        .when(enabled, |this| this.on_click(on_click))
}

pub fn button_group(vertical: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .when(vertical, |this| this.flex_col())
        .gap(px(1.0))
        .p(px(1.0))
        .bg(theme::BORDER)
}

pub fn tab_value(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex_none()
        .px(px(10.0))
        .py(px(4.0))
        .text_size(theme::BODY)
        .cursor_pointer()
        .text_color(if selected { theme::FG } else { theme::COMMENT })
        .when(selected, |this| {
            this.bg(theme::BG_HL).child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(2.0))
                    .bg(theme::BLUE),
            )
        })
        .when(!selected, |this| {
            this.hover(|style| style.text_color(theme::FG_DARK))
                .on_click(on_click)
        })
        .child(label.into())
}

pub fn tab_strip() -> Div {
    div().flex().flex_wrap().items_end()
}

pub fn tab_content() -> Div {
    div().p(px(12.0))
}

pub fn tab_panel() -> Div {
    div().p(px(12.0)).border_1().border_color(theme::BORDER)
}

pub fn card() -> Div {
    div().bg(theme::BG_FLOAT).p(px(12.0))
}

pub fn status_dot(color: gpui_kit::Rgba) -> Div {
    let size = theme::BODY;
    div()
        .flex_none()
        .w(size * 0.6)
        .h(size)
        .flex()
        .items_center()
        .justify_center()
        .child(div().size(size * 0.5).rounded_full().bg(color))
}

pub fn credits(text: impl Into<SharedString>, suffix: &str, weak: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .whitespace_nowrap()
        .text_color(if weak { theme::COMMENT } else { theme::FG })
        .child(text.into())
        .when(!suffix.is_empty(), |this| {
            this.child(
                div()
                    .text_size(theme::BODY * 0.85)
                    .text_color(theme::COMMENT)
                    .child(suffix.to_owned()),
            )
        })
}


pub fn heading(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(theme::HEADING)
        .font_weight(FontWeight::BOLD)
        .child(text.into())
}

pub fn strong(text: impl Into<SharedString>) -> Div {
    div().font_weight(FontWeight::BOLD).child(text.into())
}

pub fn weak(text: impl Into<SharedString>) -> Div {
    div().text_color(theme::COMMENT).child(text.into())
}

pub fn error(text: impl Into<SharedString>) -> Div {
    div().text_color(theme::RED).child(text.into())
}


pub fn separator() -> Div {
    div().h(px(1.0)).my(theme::LINE_GAP).bg(theme::BORDER)
}


pub fn mono_font() -> gpui_kit::Font {
    font(theme::MONO)
}

const MENU_SCROLLS_PAST: usize = 12;

pub fn menu_scrolls(items: usize) -> bool {
    items > MENU_SCROLLS_PAST
}

pub struct Choice {
    label: SharedString,
    selected: bool,
    on_select: OnSelect,
}

impl Choice {
    pub fn new(
        label: impl Into<SharedString>,
        selected: bool,
        on_select: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            selected,
            on_select: std::rc::Rc::new(on_select),
        }
    }
}

pub fn combo(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    choices: Vec<Choice>,
) -> impl IntoElement {
    use gpui_kit::component::button::Button;
    use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
    use gpui_kit::component::{Disableable, Sizable};
    let choices = std::rc::Rc::new(choices);
    let empty = choices.is_empty();
    Button::new(id)
        .label(text.into())
        .small()
        .dropdown_caret(true)
        .disabled(empty)
        .dropdown_menu(move |menu, _, _| {
            choices.iter().fold(
                menu.scrollable(menu_scrolls(choices.len())),
                |menu, choice| {
                    let on_select = choice.on_select.clone();
                    menu.item(
                        PopupMenuItem::new(choice.label.clone())
                            .checked(choice.selected)
                            .on_click(move |_, window, cx| on_select(window, cx)),
                    )
                },
            )
        })
}
