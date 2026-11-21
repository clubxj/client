use std::time::{Duration, Instant};

use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{
    App, Div, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
    Window, div,
    prelude::FluentBuilder, px,
};
use injector::antiban::{Break, Config, Sleep};
use injector::form::BreakPlan;

use crate::ui::fields::{self, NumberField, Range, TextField};
use crate::ui::theme::{self, ROW_HEIGHT};
use crate::ui::widgets;

#[derive(Default)]
pub struct BreaksTracker {
    plan: Option<(BreakPlan, Instant)>,
}

pub struct BreakRow {
    pub remaining: Duration,
    pub length: Duration,
    pub at: Option<u32>,
    pub index: u32,
}

pub struct ActiveRow {
    pub kind: String,
    pub remaining: Duration,
}

pub struct BreaksView {
    pub breaks: Vec<BreakRow>,
    pub sleeps: Vec<BreakRow>,
    pub active: Option<ActiveRow>,
}

fn ms(value: f64) -> Duration {
    Duration::from_millis(value.max(0.0).round() as u64)
}

impl BreaksTracker {
    pub fn update(&mut self, plan: BreakPlan, at: Instant) {
        self.plan = Some((plan, at));
    }

    pub fn view(&self, at: Instant) -> Option<BreaksView> {
        let (plan, received) = self.plan.as_ref()?;
        let passed = at.saturating_duration_since(*received);
        let counting = if plan.active.is_some() {
            Duration::ZERO
        } else {
            passed
        };
        let rows = |list: &[injector::form::PlannedBreak]| {
            let mut rows: Vec<BreakRow> = list
                .iter()
                .map(|item| BreakRow {
                    remaining: ms(item.remaining).saturating_sub(counting),
                    length: ms(item.length),
                    at: item.at.map(|secs| secs.rem_euclid(86400.0) as u32),
                    index: item.index,
                })
                .collect();
            rows.sort_by_key(|row| row.remaining);
            rows
        };
        Some(BreaksView {
            breaks: rows(&plan.breaks),
            sleeps: rows(&plan.sleeps),
            active: plan.active.as_ref().map(|a| ActiveRow {
                kind: a.kind.clone(),
                remaining: ms(a.remaining).saturating_sub(passed),
            }),
        })
    }
}

const MINUTE_MS: f64 = 60_000.0;
const HOUR_MS: f64 = 3_600_000.0;

pub type WithConfig = Rc<dyn Fn(&mut App, &mut dyn FnMut(&mut Config))>;

pub struct Ui {
    with: WithConfig,
    numbers: HashMap<(usize, usize), NumberField>,
    time: Option<TextField>,
}

impl Ui {
    pub fn new(with: impl Fn(&mut App, &mut dyn FnMut(&mut Config)) + 'static) -> Self {
        Self {
            with: Rc::new(with),
            numbers: HashMap::new(),
            time: None,
        }
    }

    fn change(&self, change: impl Fn(&mut Config) + 'static) -> impl Fn(&mut App) + 'static {
        let with = self.with.clone();
        move |cx| with(cx, &mut |config| change(config))
    }
}

const SLEEP_ROW: usize = usize::MAX;

#[derive(Clone, Copy)]
struct Number {
    scale: f64,
    range: Range,
    suffix: &'static str,
}

impl Number {
    const fn new(scale: f64, min: f64, max: f64, suffix: &'static str) -> Self {
        Self {
            scale,
            range: Range {
                min,
                max,
                step: 1.0,
                integer: false,
            },
            suffix,
        }
    }
}

const INTERVAL: Number = Number::new(MINUTE_MS, 1.0, 1440.0, " min");
const LENGTH: Number = Number::new(MINUTE_MS, 1.0, 1440.0, " min");
const PERCENT: Number = Number::new(0.01, 0.0, 100.0, "%");
const SLEEP_LENGTH: Number = Number::new(HOUR_MS, 0.1, 24.0, " h");

type Field = fn(&mut Config, usize) -> Option<&mut Option<f64>>;

#[allow(clippy::too_many_arguments)]
fn number(
    ui: &mut Ui,
    at: (usize, usize),
    value: f64,
    number: Number,
    enabled: bool,
    field: impl Fn(&mut Config) -> Option<&mut f64> + 'static,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let with = ui.with.clone();
    let input = ui.numbers.entry(at).or_insert_with(|| {
        NumberField::new(
            number.range,
            false,
            move |shown, cx| {
                with(cx, &mut |config| {
                    if let Some(value) = field(config) {
                        *value = shown * number.scale;
                    }
                })
            },
            window,
            cx,
        )
    });
    input.sync(value / number.scale, window, cx);
    div().child(
        input
            .input(enabled)
            .stepper(false)
            .suffix(div().text_color(theme::COMMENT).child(number.suffix))
            .w(px(NUMBER_WIDTH)),
    )
}

#[allow(clippy::too_many_arguments)]
fn auto_number(
    ui: &mut Ui,
    at: (usize, usize),
    value: Option<f64>,
    manual: f64,
    number_of: Number,
    enabled: bool,
    field: Field,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let row = at.0;
    let with = ui.with.clone();
    let auto = fields::checkbox(
        format!("auto-{}-{}", at.0, at.1),
        "Auto",
        value.is_none(),
        enabled,
        move |auto, cx| {
            with(cx, &mut |config| {
                if let Some(value) = field(config, row) {
                    *value = if auto { None } else { Some(manual) };
                }
            });
        },
    )
    .tooltip("Picked from the account's biometrics");
    div()
        .flex()
        .items_center()
        .gap(theme::ITEM_GAP)
        .child(auto)
        .when_some(value, |this, value| {
            this.child(number(
                ui,
                at,
                value,
                number_of,
                enabled,
                move |config| field(config, row).and_then(Option::as_mut),
                window,
                cx,
            ))
        })
}

const ACTIONS_WIDTH: f32 = 70.0;
const VISIBLE_ROWS: usize = 5;
const NUMBER_WIDTH: f32 = 80.0;

pub fn header() -> Div {
    div()
        .mt(px(6.0))
        .child("Breaks are short pauses taken every so often; sleeps are long ones at night.")
}

fn section_bar(ui: &Ui, enabled: bool, what: &str, field: fn(&mut Config) -> &mut bool) -> Div {
    let change = ui.with.clone();
    div().mb(px(4.0)).child(
        fields::checkbox(("section-enabled", what.len()), "Enabled", enabled, true, move |on, cx| {
            change(cx, &mut |config| *field(config) = on)
        })
        .tooltip(format!("Off means no {what} are taken")),
    )
}

fn reset_button(ui: &Ui, what: &'static str, enabled: bool, reset: fn(&mut Config)) -> impl IntoElement {
    let reset = ui.change(reset);
    widgets::button(("reset", what.len()), "Reset", enabled, move |_, _, cx| reset(cx))
        .tooltip(widgets::tooltip(format!("Put the {what} back to the defaults")))
}

fn break_field(config: &mut Config, row: usize, column: usize) -> Option<&mut f64> {
    let b = config.breaks.get_mut(row)?;
    match column {
        0 => Some(&mut b.interval),
        _ => Some(&mut b.length),
    }
}

fn break_randomness(config: &mut Config, row: usize) -> Option<&mut Option<f64>> {
    Some(&mut config.breaks.get_mut(row)?.randomness)
}

fn break_logout(config: &mut Config, row: usize) -> Option<&mut Option<f64>> {
    Some(&mut config.breaks.get_mut(row)?.logout_chance)
}

fn sleep_length(config: &mut Config, _: usize) -> Option<&mut Option<f64>> {
    Some(&mut config.sleeps.first_mut()?.length)
}

fn sleep_randomness(config: &mut Config, _: usize) -> Option<&mut Option<f64>> {
    Some(&mut config.sleeps.first_mut()?.randomness)
}

fn sleep_logout(config: &mut Config, _: usize) -> Option<&mut Option<f64>> {
    Some(&mut config.sleeps.first_mut()?.logout_chance)
}

fn cell(child: impl IntoElement) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .flex()
        .items_center()
        .child(child)
}

pub fn breaks_editor(config: &mut Config, ui: &mut Ui, window: &mut Window, cx: &mut App) -> Div {
    let enabled = config.breaks_enabled;
    let bar = section_bar(ui, enabled, "breaks", |config| &mut config.breaks_enabled);
    ui.numbers
        .retain(|&(row, _), _| row == SLEEP_ROW || row < config.breaks.len());
    let head = ["Every", "Length", "Randomness", "Log out"].into_iter().fold(
        div().flex().h(ROW_HEIGHT).items_center(),
        |head, label| head.child(cell(widgets::strong(label))),
    );
    let mut body = div().flex().flex_col();
    for (i, b) in config.breaks.iter().enumerate() {
        let remove = ui.change(move |config| {
            if i < config.breaks.len() {
                config.breaks.remove(i);
            }
        });
        let row = div()
            .flex()
            .h(ROW_HEIGHT)
            .items_center()
            .when(i % 2 == 1, |this| this.bg(theme::BG_FLOAT))
            .child(cell(number(ui, (i, 0), b.interval, INTERVAL, enabled, move |config| break_field(config, i, 0), window, cx)))
            .child(cell(number(ui, (i, 1), b.length, LENGTH, enabled, move |config| break_field(config, i, 1), window, cx)))
            .child(cell(auto_number(ui, (i, 2), b.randomness, 0.33, PERCENT, enabled, break_randomness, window, cx)))
            .child(cell(auto_number(ui, (i, 3), b.logout_chance, 0.05, PERCENT, enabled, break_logout, window, cx)))
            .child(
                div().w(px(ACTIONS_WIDTH)).flex_none().child(widgets::button(
                    ("remove-break", i),
                    "Remove",
                    enabled,
                    move |_, _, cx| remove(cx),
                )),
            );
        body = body.child(row);
    }
    let add = ui.change(|config| {
        config.breaks.push(Break {
            interval: 60.0 * MINUTE_MS,
            length: 5.0 * MINUTE_MS,
            randomness: None,
            logout_chance: None,
        })
    });
    let body_height = VISIBLE_ROWS as f32 * (f32::from(ROW_HEIGHT) + f32::from(theme::LINE_GAP));
    div()
        .flex()
        .flex_col()
        .child(bar)
        .child(head.pr(px(ACTIONS_WIDTH)))
        .child(
            div()
                .id("breaks-edit")
                .h(px(body_height))
                .overflow_y_scrollbar()
                .child(body),
        )
        .child(
            div()
                .mt(px(8.0))
                .flex()
                .items_center()
                .justify_between()
                .child(widgets::button("add-break", "Add break", enabled, move |_, _, cx| add(cx)))
                .child(reset_button(ui, "breaks", enabled, |config| {
                    config.breaks = Config::default().breaks
                })),
        )
}

pub fn sleeps_editor(config: &mut Config, ui: &mut Ui, window: &mut Window, cx: &mut App) -> Div {
    let enabled = config.sleeps_enabled;
    let bar = section_bar(ui, enabled, "sleeps", |config| &mut config.sleeps_enabled);
    if config.sleeps.is_empty() {
        config.sleeps.push(Sleep::default());
    }
    let s = &config.sleeps[0];
    let set_time = ui.with.clone();
    let time_field = ui.time.get_or_insert_with(|| {
        TextField::new(
            "",
            move |typed, cx| {
                set_time(cx, &mut |config| {
                    if let Some(sleep) = config.sleeps.first_mut() {
                        sleep.time = Some(typed.clone());
                    }
                })
            },
            window,
            cx,
        )
    });
    let time = s.time.clone();
    if let Some(time) = &time {
        time_field.sync(time, window, cx);
    }
    let bad = time
        .as_deref()
        .is_some_and(|time| injector::antiban::parse_time(time).is_none());
    let time_input = time.is_some().then(|| {
        div()
            .id("sleep-time")
            .tooltip(widgets::tooltip("HH:MM or HH:MM:SS"))
            .child(
                time_field
                    .input(enabled)
                    .w(px(60.0))
                    .when(bad, |this| this.text_color(theme::RED)),
            )
    });
    let with = ui.with.clone();
    let auto_time = fields::checkbox("sleep-auto-time", "Auto", time.is_none(), enabled, move |auto, cx| {
        with(cx, &mut |config| {
            if let Some(sleep) = config.sleeps.first_mut() {
                sleep.time = if auto { None } else { Some("23:00".into()) };
            }
        });
    })
    .tooltip("The account's usual bedtime");
    let (length, randomness, logout) = (s.length, s.randomness, s.logout_chance);
    let label = |text: &'static str| div().h(px(f32::from(theme::INTERACT_HEIGHT))).flex().items_center().child(text);
    let grid = div()
        .grid()
        .grid_cols_max_content(2)
        .gap_x(px(12.0))
        .gap_y(px(6.0))
        .items_center()
        .child(label("At"))
        .child(
            div()
                .flex()
                .items_center()
                .gap(theme::ITEM_GAP)
                .child(auto_time)
                .children(time_input),
        )
        .child(label("Length"))
        .child(auto_number(ui, (SLEEP_ROW, 0), length, 8.0, SLEEP_LENGTH, enabled, sleep_length, window, cx))
        .child(label("Randomness"))
        .child(auto_number(ui, (SLEEP_ROW, 1), randomness, 0.1, PERCENT, enabled, sleep_randomness, window, cx))
        .child(label("Log out"))
        .child(auto_number(ui, (SLEEP_ROW, 2), logout, 0.8, PERCENT, enabled, sleep_logout, window, cx));
    div()
        .flex()
        .flex_col()
        .items_start()
        .child(bar)
        .child(grid)
        .child(div().mt(px(8.0)).child(reset_button(ui, "sleeps", enabled, |config| {
            config.sleeps = Config::default().sleeps
        })))
}

pub fn clock(secs: u32) -> String {
    format!("{:02}:{:02}", secs / 3600 % 24, secs / 60 % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use injector::form::{ActiveBreak, PlannedBreak};

    fn planned(remaining: f64, at: Option<f64>) -> PlannedBreak {
        PlannedBreak {
            remaining,
            length: 60_000.0,
            at,
            index: 0,
        }
    }

    #[test]
    fn counts_down_between_reports() {
        let start = Instant::now();
        let mut tracker = BreaksTracker::default();
        assert!(tracker.view(start).is_none());
        let plan = BreakPlan {
            breaks: vec![planned(90_000.0, None), planned(10_000.0, None)],
            sleeps: vec![planned(5_000.0, Some(90_000.0))],
            active: None,
        };
        tracker.update(plan, start);
        let view = tracker.view(start + Duration::from_secs(4)).unwrap();
        assert_eq!(view.breaks[0].remaining, Duration::from_secs(6));
        assert_eq!(view.breaks[1].remaining, Duration::from_secs(86));
        assert_eq!(view.sleeps[0].remaining, Duration::from_secs(1));
        assert_eq!(view.sleeps[0].at, Some(3600 * 25 % 86400));
        assert_eq!(
            tracker
                .view(start + Duration::from_secs(60))
                .unwrap()
                .sleeps[0]
                .remaining,
            Duration::ZERO
        );
    }

    #[test]
    fn a_break_in_progress_freezes_the_rest() {
        let start = Instant::now();
        let mut tracker = BreaksTracker::default();
        let active = ActiveBreak {
            kind: "break".into(),
            remaining: 30_000.0,
        };
        tracker.update(
            BreakPlan {
                breaks: vec![planned(10_000.0, None)],
                sleeps: vec![],
                active: Some(active),
            },
            start,
        );
        let view = tracker.view(start + Duration::from_secs(5)).unwrap();
        assert_eq!(view.breaks[0].remaining, Duration::from_secs(10));
        assert_eq!(view.active.unwrap().remaining, Duration::from_secs(25));
        tracker = Default::default();
        assert!(tracker.view(start).is_none());
    }

    #[test]
    fn formats_the_time_of_day() {
        assert_eq!(clock(3600 * 3 + 60 * 7 + 59), "03:07");
    }
}
