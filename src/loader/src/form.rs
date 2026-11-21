use std::collections::HashMap;

use std::rc::Rc;

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{
    AnyElement, App, Div, InteractiveElement, IntoElement, ParentElement, Rgba,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};
use injector::form::{self as spec_form};
use serde_json::{Map, Value, json};

use crate::layouts::{self, Kind, Known, Rules, ScriptLayouts};
use crate::ui::fields::{self, NumberField, Range, TextField};
use crate::ui::{theme, widgets};

const STACK_BELOW: f32 = 420.0;

const COLUMN_GAP: f32 = 8.0;

const FIELD_GAP: f32 = 10.0;

const LIST_HEIGHT: f32 = 240.0;

const GRID_HEIGHT: f32 = 320.0;

const GRID_CELL_WIDTH: f32 = 64.0;

const GRID_RADIO: f32 = 12.0;

const NUMBER_WIDTH: f32 = 110.0;

const TEXT_WIDTH: f32 = 200.0;

const ROW_GAP: f32 = 24.0;

const HEADING_GAP: f32 = 12.0;

#[derive(Clone, Copy)]
struct Gaps {
    row: f32,
    heading: f32,
}

fn with_colon(label: &str) -> String {
    if label.ends_with(':') {
        label.to_owned()
    } else {
        format!("{label}:")
    }
}

pub enum FieldValue {
    Check(bool),
    Number(f64),
    Text(String),
    List(Vec<(String, String, bool)>),
    Grid(GridValue),
    Layout(Option<String>),
}

pub struct GridValue {
    grid: spec_form::Grid,
    picked: Vec<usize>,
    query: String,
}

fn layout_kind(field: &Value) -> Option<Kind> {
    match spec_form::kind(field) {
        "equipment" => Some(Kind::Equipment),
        "inventory" => Some(Kind::Inventory),
        "prayer" => Some(Kind::Prayer),
        _ => None,
    }
}

struct KindRules([Rules; 3]);

impl KindRules {
    fn of(&self, kind: Kind) -> &Rules {
        &self.0[match kind {
            Kind::Equipment => 0,
            Kind::Inventory => 1,
            Kind::Prayer => 2,
        }]
    }
}

pub struct Form {
    pub title: String,
    spec: Value,
    values: HashMap<String, FieldValue>,
    rules: KindRules,
}

pub enum Outcome {
    Submitted(Value),
    Cancelled,
}

fn collect_one(value: &FieldValue) -> Value {
    match value {
        FieldValue::Check(b) => json!(b),
        FieldValue::Number(n) if n.fract() == 0.0 && n.abs() < 9e15 => json!(*n as i64),
        FieldValue::Number(n) => json!(n),
        FieldValue::Text(s) => json!(s),
        FieldValue::List(rows) => Value::Object(
            rows.iter()
                .map(|(id, _, on)| (id.clone(), json!(on)))
                .collect(),
        ),
        FieldValue::Grid(value) => value.grid.value(&value.picked),
        FieldValue::Layout(name) => json!(name),
    }
}

fn collect(values: &HashMap<String, FieldValue>) -> Map<String, Value> {
    values
        .iter()
        .map(|(id, value)| (id.clone(), collect_one(value)))
        .collect()
}

fn value_of(field: &Value, saved: Option<&Value>) -> Option<FieldValue> {
    if layout_kind(field).is_some() {
        return Some(FieldValue::Layout(
            saved.and_then(Value::as_str).map(str::to_owned),
        ));
    }
    if spec_form::kind(field) == "grid" {
        let grid = spec_form::Grid::parse(field).ok()?;
        return Some(FieldValue::Grid(GridValue {
            picked: grid.picked(saved),
            grid,
            query: String::new(),
        }));
    }
    let default = spec_form::default_value(field)?;
    if spec_form::kind(field) == "list" {
        let rows = spec_form::items(field)
            .into_iter()
            .map(|(id, label, on)| {
                let ticked = saved
                    .and_then(|s| s.get(&id))
                    .and_then(Value::as_bool)
                    .unwrap_or(on);
                (id, label, ticked)
            })
            .collect();
        return Some(FieldValue::List(rows));
    }
    let pick = |v: &Value| match spec_form::kind(field) {
        "check" => v.as_bool().map(FieldValue::Check),
        "number" => v.as_f64().map(FieldValue::Number),
        "choice" => v
            .as_str()
            .filter(|s| spec_form::options(field).iter().any(|o| o == s))
            .map(|s| FieldValue::Text(s.to_owned())),
        _ => v.as_str().map(|s| FieldValue::Text(s.to_owned())),
    };
    saved.and_then(pick).or_else(|| pick(&default))
}

impl Form {
    pub fn new(spec: Value, remembered: Option<&Map<String, Value>>) -> Self {
        let title = spec
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Script settings")
            .to_owned();
        let mut values = HashMap::new();
        for field in spec_form::fields(&spec) {
            if let Some(id) = spec_form::id(field)
                && let Some(value) = value_of(field, remembered.and_then(|r| r.get(id)))
            {
                values.insert(id.to_owned(), value);
            }
        }
        let rules = KindRules(
            [Kind::Equipment, Kind::Inventory, Kind::Prayer].map(|kind| {
                let mut rules = Rules::default();
                for field in spec_form::fields(&spec) {
                    if layout_kind(field) == Some(kind) {
                        rules.required.extend(spec_form::required(field));
                        rules.locked.extend(spec_form::locked(field));
                    }
                }
                rules
            }),
        );
        Self {
            title,
            spec,
            values,
            rules,
        }
    }

    pub fn rules(&self, kind: Kind) -> &Rules {
        self.rules.of(kind)
    }

    pub fn has_layout(&self, kind: Kind) -> bool {
        spec_form::fields(&self.spec)
            .iter()
            .any(|field| layout_kind(field) == Some(kind))
    }

    pub fn spec(&self) -> &Value {
        &self.spec
    }

    pub fn remembered(&self) -> Map<String, Value> {
        collect(&self.values)
    }

    pub fn submit_label(&self) -> &str {
        self.spec
            .get("submit")
            .and_then(Value::as_str)
            .unwrap_or("Start")
    }

    pub fn submitted(&self, layouts: &ScriptLayouts, known: &Known) -> Outcome {
        let mut values = collect(&self.values);
        let rules = &self.rules;
        for field in spec_form::fields(&self.spec) {
            if let (Some(kind), Some(id)) = (layout_kind(field), spec_form::id(field)) {
                let chosen = values.get(id).and_then(Value::as_str).unwrap_or("");
                values.insert(
                    id.to_owned(),
                    kind.value_of(layouts, chosen, rules.of(kind), known),
                );
            }
        }
        Outcome::Submitted(Value::Object(values))
    }

    pub fn render(
        &mut self,
        ui: &mut FormUi,
        layouts: &ScriptLayouts,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let Self {
            spec,
            values,
            rules,
            ..
        } = self;
        let fields = spec_form::fields(spec);
        let columns = spec.get("columns").and_then(Value::as_bool).unwrap_or(false);
        let gaps = Gaps {
            row: spec_form::row_gap(spec).map_or(ROW_GAP, |g| g as f32),
            heading: spec_form::heading_gap(spec).map_or(HEADING_GAP, |g| g as f32),
        };
        let mut place = Place {
            ui,
            layouts,
            rules,
            gaps,
        };
        if !columns {
            return show_fields(fields, values, &mut place, window, cx).into_any_element();
        }
        let (checks, others): (Vec<&Value>, Vec<&Value>) = fields
            .iter()
            .partition(|field| spec_form::kind(field) == "check");
        let column = |fields: Div| {
            fields
                .flex_1()
                .min_w(px(STACK_BELOW / 2.0 - COLUMN_GAP / 2.0))
        };
        div()
            .flex()
            .flex_wrap()
            .items_start()
            .gap_x(px(COLUMN_GAP))
            .gap_y(px(FIELD_GAP))
            .child(column(show_fields(others, values, &mut place, window, cx)))
            .child(column(show_fields(checks, values, &mut place, window, cx)))
            .into_any_element()
    }
}

pub type WithForm = Rc<dyn Fn(&mut App, &mut dyn FnMut(&mut Form))>;

pub struct FormUi {
    with: WithForm,
    numbers: HashMap<String, NumberField>,
    texts: HashMap<String, TextField>,
}

impl FormUi {
    pub fn new(with: impl Fn(&mut App, &mut dyn FnMut(&mut Form)) + 'static) -> Self {
        Self {
            with: Rc::new(with),
            numbers: HashMap::new(),
            texts: HashMap::new(),
        }
    }

    fn change(&self, id: &str) -> impl Fn(&mut App, &mut dyn FnMut(&mut FieldValue)) + 'static {
        let (with, id) = (self.with.clone(), id.to_owned());
        move |cx, change| {
            with(cx, &mut |form| {
                if let Some(value) = form.values.get_mut(&id) {
                    change(value);
                }
            })
        }
    }
}

struct Place<'a> {
    ui: &'a mut FormUi,
    layouts: &'a ScriptLayouts,
    rules: &'a KindRules,
    gaps: Gaps,
}

fn visible(field: &Value, values: &HashMap<String, FieldValue>) -> bool {
    spec_form::visible_rules(field)
        .into_iter()
        .all(|(id, equals)| {
            values
                .get(id)
                .is_some_and(|v| spec_form::matches(&collect_one(v), equals))
        })
}

fn show_fields<'a>(
    fields: impl IntoIterator<Item = &'a Value>,
    values: &mut HashMap<String, FieldValue>,
    place: &mut Place,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let shown: Vec<&Value> = fields
        .into_iter()
        .filter(|field| visible(field, values))
        .collect();
    let mut column = div().flex().flex_col().items_start().gap(px(FIELD_GAP));
    let mut rest = shown.as_slice();
    while let Some(first) = rest.first() {
        let len = spec_form::row(first).map_or(1, |row| {
            rest.iter()
                .take_while(|field| spec_form::row(field) == Some(row))
                .count()
        });
        let at_top = rest.len() == shown.len();
        let (line, after) = rest.split_at(len);
        rest = after;
        if !at_top && spec_form::kind(first) == "label" && spec_form::heading(first).is_some() {
            column = column.child(div().h(px(place.gaps.heading - FIELD_GAP).max(px(0.0))));
        }
        if len == 1 {
            column = column.children(show_field(first, values, place, window, cx));
        } else {
            let row = line.iter().fold(
                div()
                    .flex()
                    .flex_wrap()
                    .items_start()
                    .gap_x(px(place.gaps.row))
                    .gap_y(px(FIELD_GAP)),
                |row, field| row.children(show_field(field, values, place, window, cx)),
            );
            column = column.child(row);
        }
    }
    column
}

fn show_field(
    field: &Value,
    values: &mut HashMap<String, FieldValue>,
    place: &mut Place,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    if !visible(field, values) {
        return None;
    }
    let checked = |id: &str| matches!(values.get(id), Some(FieldValue::Check(true)));
    let usable = spec_form::enabled_by(field).is_none_or(checked)
        && !spec_form::disabled_by(field).is_some_and(checked);
    let value = spec_form::id(field).and_then(|id| values.get_mut(id));
    field_ui(field, value, usable, place, window, cx)
}

fn text(label: &str, usable: bool) -> Div {
    div()
        .when(!usable, |this| this.text_color(theme::COMMENT))
        .child(label.to_owned())
}

fn field_ui(
    field: &Value,
    value: Option<&mut FieldValue>,
    usable: bool,
    place: &mut Place,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let label = field
        .get("label")
        .or_else(|| field.get("text"))
        .and_then(Value::as_str)
        .or_else(|| spec_form::id(field))
        .unwrap_or("");
    let id = spec_form::id(field).unwrap_or(label).to_owned();
    let number = |key: &str| field.get(key).and_then(Value::as_f64);
    let element = match (spec_form::kind(field), value) {
        ("label", _) => match spec_form::heading(field) {
            Some(1) => widgets::heading(label.to_owned()),
            Some(_) => widgets::strong(label.to_owned()).text_size((theme::BODY + theme::HEADING) / 2.0),
            None => text(label, usable),
        }
        .into_any_element(),
        ("list", Some(FieldValue::List(rows))) => {
            let change = place.ui.change(&id);
            let change = Rc::new(change);
            let list = rows.iter().enumerate().fold(
                div().flex().flex_col(),
                |list, (i, (_, row_label, on))| {
                    let change = change.clone();
                    list.child(
                        div()
                            .px(px(4.0))
                            .py(px(3.0))
                            .when(i % 2 == 1, |this| this.bg(theme::BG_FLOAT))
                            .child(fields::checkbox(
                                ("form-list", i),
                                row_label.clone(),
                                *on,
                                usable,
                                move |on, cx| {
                                    change(cx, &mut |value| {
                                        if let FieldValue::List(rows) = value
                                            && let Some(row) = rows.get_mut(i)
                                        {
                                            row.2 = on;
                                        }
                                    })
                                },
                            )),
                    )
                },
            );
            div()
                .flex()
                .flex_col()
                .w_full()
                .gap(theme::LINE_GAP)
                .when(!label.is_empty(), |this| this.child(text(label, usable)))
                .child(
                    div()
                        .w_full()
                        .p(px(6.0))
                        .border_1()
                        .border_color(theme::BORDER)
                        .rounded(px(2.0))
                        .child(
                            div()
                                .id(format!("form-list-scroll-{id}"))
                                .max_h(px(LIST_HEIGHT))
                                .overflow_y_scrollbar()
                                .child(list),
                        ),
                )
                .into_any_element()
        }
        ("grid", Some(FieldValue::Grid(value))) => {
            grid_ui(&id, label, value, usable, place, window, cx)
        }
        ("check", Some(FieldValue::Check(on))) => {
            let change = place.ui.change(&id);
            fields::checkbox(
                format!("form-check-{id}"),
                label.to_owned(),
                *on,
                usable,
                move |on, cx| {
                    change(cx, &mut |value| {
                        if let FieldValue::Check(b) = value {
                            *b = on;
                        }
                    })
                },
            )
            .into_any_element()
        }
        ("number", Some(FieldValue::Number(n))) => {
            let range = Range::new(number("min").unwrap_or(f64::MIN)..=number("max").unwrap_or(f64::MAX));
            let range = if field.get("integer").and_then(Value::as_bool).unwrap_or(false) {
                range.integer()
            } else {
                number("step").map_or(range, |step| range.step(step))
            };
            let change = place.ui.change(&id);
            let input = place.ui.numbers.entry(id.clone()).or_insert_with(|| {
                NumberField::new(
                    range,
                    false,
                    move |shown, cx| {
                        change(cx, &mut |value| {
                            if let FieldValue::Number(n) = value {
                                *n = shown;
                            }
                        })
                    },
                    window,
                    cx,
                )
            });
            input.sync(*n, window, cx);
            let input = input.input(usable).w(px(NUMBER_WIDTH));
            let label = text(label, usable);
            let row = div().flex().items_center().gap(theme::ITEM_GAP);
            if spec_form::box_first(field) {
                row.child(input).child(label)
            } else {
                row.child(label).child(input)
            }
            .into_any_element()
        }
        ("choice", Some(FieldValue::Text(selected))) => {
            let change = Rc::new(place.ui.change(&id));
            let picks = spec_form::options(field)
                .into_iter()
                .map(|option| {
                    let change = change.clone();
                    let chosen = option.clone();
                    fields::Pick::new(option.clone(), *selected == option, move |cx| {
                        change(cx, &mut |value| {
                            if let FieldValue::Text(text) = value {
                                text.clone_from(&chosen);
                            }
                        })
                    })
                })
                .collect();
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap(theme::LINE_GAP)
                .when(!label.is_empty(), |this| this.child(text(&with_colon(label), usable)))
                .child(fields::select(
                    format!("form-choice-{id}"),
                    selected.clone(),
                    picks,
                    usable,
                    None,
                ))
                .into_any_element()
        }
        (_, Some(FieldValue::Text(value))) => {
            let change = place.ui.change(&id);
            let input = place.ui.texts.entry(id.clone()).or_insert_with(|| {
                TextField::new(
                    "",
                    move |typed, cx| {
                        change(cx, &mut |value| {
                            if let FieldValue::Text(text) = value {
                                text.clone_from(&typed);
                            }
                        })
                    },
                    window,
                    cx,
                )
            });
            input.sync(value, window, cx);
            div()
                .flex()
                .items_center()
                .gap(theme::ITEM_GAP)
                .child(text(label, usable))
                .child(input.input(usable).w(px(TEXT_WIDTH)))
                .into_any_element()
        }
        (_, Some(FieldValue::Layout(chosen))) => {
            let kind = layout_kind(field)?;
            let change = Rc::new(place.ui.change(&id));
            let picker = layouts::picker(
                kind,
                place.layouts,
                chosen,
                &id,
                place.rules.of(kind),
                usable,
                move |name, cx| {
                    change(cx, &mut |value| {
                        if let FieldValue::Layout(chosen) = value {
                            chosen.clone_from(&name);
                        }
                    })
                },
            );
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap(theme::LINE_GAP)
                .child(text(
                    &with_colon(if label.is_empty() { kind.label() } else { label }),
                    usable,
                ))
                .child(picker)
                .into_any_element()
        }
        _ => return None,
    };
    Some(element)
}

fn grid_radio(color: Option<Rgba>, selected: bool, open: bool) -> Div {
    let ring = div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(GRID_RADIO))
        .rounded_full();
    match (selected, color) {
        (true, Some(color)) => ring.bg(color),
        (true, None) => ring
            .border_1()
            .border_color(theme::FG)
            .child(div().size(px(GRID_RADIO / 2.0)).rounded_full().bg(theme::FG)),
        (false, _) => ring
            .border_1()
            .border_color(theme::alpha(theme::COMMENT, if open { 1.0 } else { 0.35 })),
    }
}

fn grid_ui(
    id: &str,
    label: &str,
    value: &GridValue,
    usable: bool,
    place: &mut Place,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let GridValue {
        grid,
        picked,
        query,
    } = value;
    let change = Rc::new(place.ui.change(id));
    let set = {
        let change = change.clone();
        move |rows: Vec<usize>, column: usize, cx: &mut App| {
            change(cx, &mut |value| {
                if let FieldValue::Grid(value) = value {
                    for &row in &rows {
                        if let Some(pick) = value.picked.get_mut(row) {
                            *pick = column;
                        }
                    }
                }
            })
        }
    };
    let set = Rc::new(set);
    let search = place.ui.texts.entry(id.to_owned()).or_insert_with(|| {
        let change = change.clone();
        TextField::new(
            &grid.search,
            move |typed, cx| {
                change(cx, &mut |value| {
                    if let FieldValue::Grid(value) = value {
                        value.query.clone_from(&typed);
                    }
                })
            },
            window,
            cx,
        )
    });
    search.sync(query, window, cx);
    let needle = query.trim().to_lowercase();
    let shown: Vec<usize> = grid
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| needle.is_empty() || item.label.to_lowercase().contains(&needle))
        .map(|(i, _)| i)
        .collect();
    let counts: Vec<usize> = (0..grid.columns.len())
        .map(|c| grid.count(picked, c))
        .collect();
    let color = |c: usize| grid.columns[c].color.map(gpui_kit::rgb);
    let small = |text: String| div().text_size(theme::SMALL).text_color(theme::COMMENT).child(text);
    let header = grid.columns.iter().enumerate().fold(
        div()
            .flex()
            .items_center()
            .px(px(6.0))
            .pb(px(4.0))
            .border_b_1()
            .border_color(theme::BORDER)
            .child(widgets::strong(label.to_owned()).flex_1().min_w_0()),
        |header, (c, column)| {
            let title = div()
                .flex()
                .items_baseline()
                .gap(px(3.0))
                .child(widgets::strong(column.label.clone()))
                .when_some(column.max, |this, max| {
                    this.child(small(format!("{}/{max}", counts[c])))
                });
            let below = match column.max {
                Some(_) => small("\u{2014}".to_owned()).into_any_element(),
                None => {
                    let (set, rows) = (set.clone(), shown.clone());
                    small("all".to_owned())
                        .id(format!("form-grid-all-{id}-{c}"))
                        .when(usable && !shown.is_empty(), |this| {
                            this.cursor_pointer()
                                .hover(|style| style.text_color(theme::FG))
                                .on_click(move |_, _, cx| set(rows.clone(), c, cx))
                        })
                        .into_any_element()
                }
            };
            header.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .flex_none()
                    .w(px(GRID_CELL_WIDTH))
                    .child(title)
                    .child(below),
            )
        },
    );
    let rows = shown.iter().enumerate().fold(div().flex().flex_col(), |rows, (n, &i)| {
        let item = &grid.items[i];
        let name = div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_baseline()
            .gap(px(6.0))
            .overflow_hidden()
            .whitespace_nowrap()
            .child(item.label.clone())
            .when_some(item.detail.clone(), |this, detail| this.child(small(detail)));
        let row = grid.columns.iter().enumerate().fold(
            div()
                .flex()
                .items_center()
                .h(theme::ROW_HEIGHT)
                .px(px(6.0))
                .when(n % 2 == 1, |this| this.bg(theme::BG_FLOAT))
                .when(!usable, |this| this.text_color(theme::COMMENT))
                .child(name),
            |row, (c, column)| {
                let selected = picked[i] == c;
                let open = usable && column.max.is_none_or(|max| counts[c] < max);
                let set = set.clone();
                row.child(
                    div()
                        .id(format!("form-grid-{id}-{i}-{c}"))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .w(px(GRID_CELL_WIDTH))
                        .h(theme::ROW_HEIGHT - px(4.0))
                        .border_1()
                        .border_color(theme::alpha(theme::BLUE, 0.0))
                        .when(selected, |this| this.bg(theme::BG_HL))
                        .child(grid_radio(color(c), selected, open))
                        .when(open && !selected, |this| {
                            this.cursor_pointer()
                                .hover(|style| style.border_color(theme::BLUE))
                                .on_click(move |_, _, cx| set(vec![i], c, cx))
                        }),
                )
            },
        );
        rows.child(row)
    });
    let rows = if shown.is_empty() {
        rows.child(widgets::weak("No matches").px(px(6.0)).py(px(4.0)))
    } else {
        rows
    };
    let summary = grid.columns.iter().enumerate().fold(
        div().flex().flex_wrap().items_baseline().gap_x(px(4.0)),
        |summary, (c, column)| {
            summary
                .when(c > 0, |this| this.child(small("\u{b7}".to_owned())))
                .child(
                    widgets::strong(counts[c].to_string())
                        .text_color(color(c).unwrap_or(theme::FG)),
                )
                .child(small(column.summary.clone()))
        },
    );
    div()
        .flex()
        .flex_col()
        .w_full()
        .gap(px(6.0))
        .p(px(6.0))
        .border_1()
        .border_color(theme::BORDER)
        .rounded(px(2.0))
        .child(search.input(usable).w_full())
        .child(header)
        .child(
            div()
                .id(format!("form-grid-scroll-{id}"))
                .max_h(px(GRID_HEIGHT))
                .overflow_y_scrollbar()
                .child(rows),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_baseline()
                .justify_between()
                .gap_x(px(12.0))
                .text_size(theme::SMALL)
                .child(summary)
                .when_some(grid.note.clone(), |this, note| this.child(small(note))),
        )
        .into_any_element()
}
