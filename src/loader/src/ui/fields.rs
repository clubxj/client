use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{
    Input, InputEvent, InputState, NumberInput, NumberInputEvent, StepAction,
};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    AnyWindowHandle, App, AppContext, ElementId, Entity, Focusable, InteractiveElement,
    IntoElement, ParentElement, Pixels, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Window, div, prelude::FluentBuilder,
};

use super::{theme, widgets};

#[derive(Clone, Copy)]
pub struct Range {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub integer: bool,
}

impl Range {
    pub fn new(range: std::ops::RangeInclusive<f64>) -> Self {
        Self {
            min: *range.start(),
            max: *range.end(),
            step: 1.0,
            integer: false,
        }
    }

    pub fn integer(self) -> Self {
        Self {
            integer: true,
            ..self
        }
    }

    pub fn step(self, step: f64) -> Self {
        Self { step, ..self }
    }

    fn fit(self, value: f64) -> f64 {
        let value = value.max(self.min).min(self.max);
        if self.integer { value.round() } else { value }
    }

    fn format(self, value: f64) -> String {
        if self.integer {
            format!("{}", value.round() as i64)
        } else {
            format!("{}", (value * 1e6).round() / 1e6)
        }
    }

    fn allows(self, text: &str) -> bool {
        text.chars()
            .all(|c| c.is_ascii_digit() || c == '-' || (c == '.' && !self.integer))
    }
}

fn parse(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

fn show(state: &Entity<InputState>, window: AnyWindowHandle, text: String, cx: &mut App) {
    let state = state.clone();
    window
        .update(cx, move |_, window, cx| {
            state.update(cx, |state, cx| state.set_value(text, window, cx));
        })
        .ok();
}

type OnNumber = Rc<dyn Fn(f64, &mut App)>;

pub struct NumberField {
    state: Entity<InputState>,
    range: Rc<Cell<Range>>,
    shown: Rc<Cell<f64>>,
    _subscriptions: [Subscription; 2],
}

impl NumberField {
    pub fn new(
        range: Range,
        commit_on_blur: bool,
        on_change: impl Fn(f64, &mut App) + 'static,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let range = Rc::new(Cell::new(range));
        let state = {
            let range = range.clone();
            cx.new(|cx| {
                InputState::new(window, cx).validate(move |text, _| range.get().allows(text))
            })
        };
        let shown = Rc::new(Cell::new(f64::NAN));
        let handle = window.window_handle();
        let on_change: OnNumber = Rc::new(on_change);
        let typed = {
            let (shown, on_change, range) = (shown.clone(), on_change.clone(), range.clone());
            cx.subscribe(&state, move |state, event: &InputEvent, cx| {
                let range = range.get();
                let text = state.read(cx).value();
                match event {
                    InputEvent::Change if !commit_on_blur => {
                        if let Some(value) = parse(&text) {
                            let value = range.fit(value);
                            shown.set(value);
                            on_change(value, cx);
                        }
                    }
                    InputEvent::Blur | InputEvent::PressEnter { .. } => {
                        let value = parse(&text).map(|v| range.fit(v)).unwrap_or(shown.get());
                        if value.is_finite() {
                            shown.set(value);
                            show(&state, handle, range.format(value), cx);
                            on_change(value, cx);
                        }
                    }
                    _ => {}
                }
            })
        };
        let stepped = {
            let (shown, range) = (shown.clone(), range.clone());
            cx.subscribe(&state, move |state, event: &NumberInputEvent, cx| {
                let range = range.get();
                let NumberInputEvent::Step(action) = event;
                let base = parse(&state.read(cx).value())
                    .or(Some(shown.get()).filter(|v| v.is_finite()))
                    .unwrap_or(0.0);
                let value = range.fit(match action {
                    StepAction::Increment => base + range.step,
                    StepAction::Decrement => base - range.step,
                });
                shown.set(value);
                show(&state, handle, range.format(value), cx);
                on_change(value, cx);
            })
        };
        Self {
            state,
            range,
            shown,
            _subscriptions: [typed, stepped],
        }
    }

    pub fn set_range(&self, range: Range) {
        self.range.set(range);
    }

    pub fn sync(&self, value: f64, window: &mut Window, cx: &mut App) {
        if self.state.focus_handle(cx).is_focused(window) {
            return;
        }
        self.shown.set(value);
        let text = self.range.get().format(value);
        if self.state.read(cx).value() != text {
            self.state
                .update(cx, |state, cx| state.set_value(text, window, cx));
        }
    }

    pub fn input(&self, enabled: bool) -> NumberInput {
        NumberInput::new(&self.state)
            .small()
            .flex_none()
            .disabled(!enabled)
    }
}

pub struct TextField {
    state: Entity<InputState>,
    _subscription: Subscription,
}

impl TextField {
    pub fn new(
        placeholder: &str,
        on_change: impl Fn(String, &mut App) + 'static,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let placeholder = placeholder.to_owned();
        let state = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let subscription = cx.subscribe(&state, move |state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let text = state.read(cx).value().to_string();
                on_change(text, cx);
            }
        });
        Self {
            state,
            _subscription: subscription,
        }
    }

    pub fn sync(&self, text: &str, window: &mut Window, cx: &mut App) {
        if self.state.read(cx).value() != text {
            let text = text.to_owned();
            self.state
                .update(cx, |state, cx| state.set_value(text, window, cx));
        }
    }

    pub fn input(&self, enabled: bool) -> Input {
        Input::new(&self.state).small().disabled(!enabled)
    }
}

pub fn checkbox(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
    enabled: bool,
    on_change: impl Fn(bool, &mut App) + 'static,
) -> Checkbox {
    Checkbox::new(id)
        .label(label.into())
        .checked(checked)
        .small()
        .disabled(!enabled)
        .on_change(move |on, _, cx| on_change(*on, cx))
}

pub struct Pick {
    label: SharedString,
    selected: bool,
    refusal: Option<SharedString>,
    on_pick: Rc<dyn Fn(&mut App)>,
}

impl Pick {
    pub fn new(
        label: impl Into<SharedString>,
        selected: bool,
        on_pick: impl Fn(&mut App) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            selected,
            refusal: None,
            on_pick: Rc::new(on_pick),
        }
    }

    pub fn refused(label: impl Into<SharedString>, why: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            selected: false,
            refusal: Some(why.into()),
            on_pick: Rc::new(|_| {}),
        }
    }
}

pub fn select(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    picks: Vec<Pick>,
    enabled: bool,
    width: Option<Pixels>,
) -> impl IntoElement {
    let picks = Rc::new(picks);
    Button::new(id)
        .label(text.into())
        .small()
        .dropdown_caret(true)
        .disabled(!enabled || picks.is_empty())
        .when_some(width, |this, width| this.w(width))
        .dropdown_menu(move |menu, _, _| {
            picks.iter().enumerate().fold(
                menu.scrollable(widgets::menu_scrolls(picks.len())),
                |menu, (i, pick)| match &pick.refusal {
                    Some(why) => {
                        let (label, why) = (pick.label.clone(), why.clone());
                        let row = format!("pick-{i}");
                        menu.item(
                            PopupMenuItem::element(move |_, _| {
                                div()
                                    .id(row.clone())
                                    .text_color(theme::COMMENT)
                                    .tooltip(widgets::tooltip(why.clone()))
                                    .child(label.clone())
                            })
                            .disabled(true),
                        )
                    }
                    None => {
                        let on_pick = pick.on_pick.clone();
                        menu.item(
                            PopupMenuItem::new(pick.label.clone())
                                .checked(pick.selected)
                                .on_click(move |_, _, cx| on_pick(cx)),
                        )
                    }
                },
            )
        })
}

pub fn small_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    enabled: bool,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    widgets::button(id, label, enabled, on_click)
        .h(gpui_kit::px(18.0))
        .px(gpui_kit::px(4.0))
        .text_size(theme::SMALL)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui_kit::{Context, Render, TestAppContext, VisualTestContext};

    use super::*;

    struct Probe {
        field: Option<NumberField>,
        seen: Rc<RefCell<Vec<f64>>>,
    }

    impl Render for Probe {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let seen = self.seen.clone();
            let field = self.field.get_or_insert_with(|| {
                NumberField::new(
                    Range::new(0.0..=10.0).integer(),
                    false,
                    move |value, _| seen.borrow_mut().push(value),
                    window,
                    cx,
                )
            });
            let value = self.seen.borrow().last().copied().unwrap_or(9.0);
            field.sync(value, window, cx);
            div().child(field.input(true))
        }
    }

    #[gpui_kit::test]
    fn a_step_shows_and_reports_the_clamped_value(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let (probe, cx) = cx.add_window_view(|_, _| Probe {
            field: None,
            seen: seen.clone(),
        });
        let cx: &mut VisualTestContext = cx;
        cx.run_until_parked();
        let state = probe.read_with(cx, |probe, _| probe.field.as_ref().unwrap().state.clone());
        assert_eq!(state.read_with(cx, |state, _| state.value().to_string()), "9");
        cx.update(|window, cx| state.update(cx, |state, cx| state.focus(window, cx)));
        for _ in 0..2 {
            state.update(cx, |_, cx| cx.emit(NumberInputEvent::Step(StepAction::Increment)));
            cx.run_until_parked();
        }
        cx.update(|window, cx| assert!(state.focus_handle(cx).is_focused(window)));
        assert_eq!(*seen.borrow(), [10.0, 10.0]);
        assert_eq!(state.read_with(cx, |state, _| state.value().to_string()), "10");
    }
}
