use std::ops::Range;

use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{
    Input, InputEvent, InputState, NumberInput, NumberInputEvent, StepAction,
};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    DragMoveEvent, Empty, Entity, ListHorizontalSizingBehavior, ScrollStrategy, SharedString,
    Subscription, UniformListScrollHandle, WeakEntity, uniform_list,
};

use super::*;
use crate::packets::{NameState, UNKNOWN, name_state, with_name};
use crate::ui::{theme, widgets};

const ROW: Pixels = px(20.0);
const INDENT: Pixels = px(18.0);
const MIN_COLUMN: Pixels = px(24.0);
const CELL_PAD: Pixels = px(4.0);
const DETAIL_HEIGHT: Pixels = px(150.0);
const TOGGLE_GAP: Pixels = px(24.0);
const CHECK_INDICATOR: Pixels = px(22.0);
const SLOT_GAP_X: Pixels = px(8.0);
const SLOT_GAP_Y: Pixels = px(6.0);
const ADDED: u32 = 0x5ac85a;
const REMOVED: u32 = 0xe66464;

#[derive(Clone, Copy)]
enum Width {
    Fixed(f32),
    Rest(f32),
}

type Columns = [(&'static str, Width)];

const SCRIPT_COLUMNS: [(&str, Width); 4] = [
    ("Tick", Width::Fixed(60.0)),
    ("Script", Width::Fixed(240.0)),
    ("Arguments", Width::Fixed(260.0)),
    ("Time", Width::Rest(60.0)),
];

const VAR_COLUMNS: [(&str, Width); 5] = [
    ("Tick", Width::Fixed(60.0)),
    ("Kind", Width::Fixed(100.0)),
    ("Name", Width::Fixed(220.0)),
    ("Old", Width::Fixed(110.0)),
    ("New", Width::Rest(60.0)),
];

const PACKET_COLUMNS: [(&str, Width); 7] = [
    ("Time", Width::Fixed(70.0)),
    ("", Width::Fixed(24.0)),
    ("Opcode", Width::Fixed(56.0)),
    ("Length", Width::Fixed(50.0)),
    ("Name", Width::Fixed(120.0)),
    ("Fields", Width::Fixed(240.0)),
    ("Hex", Width::Rest(80.0)),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum TableId {
    Scripts,
    Vars,
    Packets,
}

#[derive(Clone, Copy)]
struct ColumnDrag {
    table: TableId,
    column: usize,
}

struct TableState {
    widths: Vec<Pixels>,
    scroll: UniformListScrollHandle,
}

impl TableState {
    fn new(columns: &Columns) -> Self {
        Self {
            widths: columns
                .iter()
                .map(|(_, width)| match width {
                    Width::Fixed(width) | Width::Rest(width) => px(*width),
                })
                .collect(),
            scroll: UniformListScrollHandle::new(),
        }
    }
}

struct LogRow {
    cells: Vec<String>,
    color: Option<Rgba>,
    selected: bool,
    pick: Option<u64>,
}

#[derive(Clone, Copy)]
struct TreeRow {
    node: usize,
    depth: usize,
    open: Option<bool>,
}

#[derive(Clone)]
struct Inputs {
    varp: Entity<InputState>,
    blacklist: Entity<InputState>,
    filter: Entity<InputState>,
    name_search: Entity<InputState>,
}

pub(super) struct Ui {
    inputs: Option<(Inputs, Vec<Subscription>)>,
    varps_open: bool,
    widget_open: HashSet<(i32, i32)>,
    widget_rows: Vec<TreeRow>,
    widget_tree: UniformListScrollHandle,
    containers_open: HashSet<i32>,
    scripts: TableState,
    vars: TableState,
    packets: TableState,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            inputs: None,
            varps_open: false,
            widget_open: HashSet::new(),
            widget_rows: Vec::new(),
            widget_tree: UniformListScrollHandle::new(),
            containers_open: HashSet::new(),
            scripts: TableState::new(&SCRIPT_COLUMNS),
            vars: TableState::new(&VAR_COLUMNS),
            packets: TableState::new(&PACKET_COLUMNS),
        }
    }
}

impl Inputs {
    fn new(
        app: &LoaderApp,
        window: &mut Window,
        cx: &mut Context<LoaderApp>,
    ) -> (Self, Vec<Subscription>) {
        let text = |value: &str,
                    placeholder: &'static str,
                    window: &mut Window,
                    cx: &mut Context<LoaderApp>| {
            let value = value.to_owned();
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value)
            })
        };
        let varp = cx.new(|cx| {
            InputState::new(window, cx)
                .min(0.0)
                .max((injector::MAX_VARPS - 1) as f64)
                .default_value(app.varp.to_string())
        });
        let blacklist = text(&app.script_inspector.blacklist, "6697, 4730", window, cx);
        let filter = text(
            &app.packets.filter,
            "40, VARP_LARGE or !PLAYER_INFO",
            window,
            cx,
        );
        let name_search = text(&app.packets.name_search, "🔍 Search names", window, cx);
        let subscriptions = vec![
            cx.subscribe_in(&varp, window, |app, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change)
                    && let Ok(varp) = input.read(cx).value().trim().parse::<usize>()
                {
                    app.varp = varp.min(injector::MAX_VARPS - 1);
                    cx.notify();
                }
            }),
            cx.subscribe_in(&varp, window, |app, _, event: &NumberInputEvent, _, cx| {
                let NumberInputEvent::Step(step) = event;
                app.varp = match step {
                    StepAction::Increment => (app.varp + 1).min(injector::MAX_VARPS - 1),
                    StepAction::Decrement => app.varp.saturating_sub(1),
                };
                cx.notify();
            }),
            cx.subscribe_in(
                &blacklist,
                window,
                |app, input, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        app.script_inspector.blacklist = input.read(cx).value().to_string();
                        cx.notify();
                    }
                },
            ),
            cx.subscribe_in(&filter, window, |app, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    app.packets.filter = input.read(cx).value().to_string();
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &name_search,
                window,
                |app, input, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        app.packets.name_search = input.read(cx).value().to_string();
                        cx.notify();
                    }
                },
            ),
        ];
        (
            Self {
                varp,
                blacklist,
                filter,
                name_search,
            },
            subscriptions,
        )
    }
}

impl Ui {
    fn table(&self, table: TableId) -> &TableState {
        match table {
            TableId::Scripts => &self.scripts,
            TableId::Vars => &self.vars,
            TableId::Packets => &self.packets,
        }
    }

    fn table_mut(&mut self, table: TableId) -> &mut TableState {
        match table {
            TableId::Scripts => &mut self.scripts,
            TableId::Vars => &mut self.vars,
            TableId::Packets => &mut self.packets,
        }
    }
}

fn sync_input(input: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut App) {
    if input.read(cx).value() != value {
        let value = value.to_owned();
        input.update(cx, |state, cx| state.set_value(value, window, cx));
    }
}

fn wrapped_row() -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x(theme::ITEM_GAP)
        .gap_y(theme::LINE_GAP)
}

fn selectable(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .h(theme::INTERACT_HEIGHT)
        .px(theme::BUTTON_PADDING)
        .whitespace_nowrap()
        .border_1()
        .cursor_pointer()
        .map(|this| {
            if selected {
                this.bg(theme::SEL)
                    .text_color(theme::BLUE)
                    .border_color(theme::alpha(theme::BLUE, 0.0))
            } else {
                this.border_color(theme::alpha(theme::BLUE, 0.0))
                    .hover(|style| style.bg(theme::SEL).border_color(theme::BLUE))
            }
        })
        .child(label.into())
}

fn caret(open: bool) -> Div {
    div()
        .flex_none()
        .w(INDENT)
        .text_color(theme::FG_DARK)
        .child(if open { "⏷" } else { "⏵" })
}

fn small_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    widgets::button(id, label, enabled, on_click)
        .h(px(18.0))
        .px(px(4.0))
}

fn client_caption(pid: Option<u32>) -> Div {
    match pid {
        Some(pid) => widgets::weak(format!("Client {pid}.")),
        None => div().child("Select a client with agent loaded in the Clients tab."),
    }
}

fn warning(text: impl Into<SharedString>) -> Div {
    div().text_color(theme::YELLOW).child(text.into())
}

fn cell(width: Pixels, rest: Option<Pixels>) -> Div {
    let cell = div()
        .h_full()
        .flex()
        .items_center()
        .px(CELL_PAD)
        .overflow_hidden()
        .whitespace_nowrap();
    match rest {
        Some(min) => cell.flex_1().min_w(min),
        None => cell.flex_none().w(width),
    }
}

fn rest_of(width: Width) -> Option<Pixels> {
    match width {
        Width::Fixed(_) => None,
        Width::Rest(min) => Some(px(min)),
    }
}

fn check(
    id: impl Into<ElementId>,
    label: &'static str,
    checked: bool,
    cx: &mut Context<LoaderApp>,
    set: impl Fn(&mut LoaderApp, bool) + 'static,
) -> Checkbox {
    Checkbox::new(id)
        .label(label)
        .checked(checked)
        .small()
        .on_click(cx.listener(move |app, checked: &bool, _, cx| {
            set(app, *checked);
            app.wake.wake();
            cx.notify();
        }))
}

fn rows_listener<T: Rows + 'static>(
    action: RowsAction,
    what: &'static str,
    file_name: &'static str,
    rows: fn(&mut LoaderApp) -> &mut T,
    cx: &mut Context<LoaderApp>,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    cx.listener(move |app, _: &ClickEvent, _, cx| {
        app.rows_action(Some(action), what, file_name, rows);
        cx.notify();
    })
}

fn rows_buttons<T: Rows + 'static>(
    id: &'static str,
    empty: bool,
    pipe: Option<&crate::pipe::Pipe>,
    what: &'static str,
    file_name: &'static str,
    rows: fn(&mut LoaderApp) -> &mut T,
    cx: &mut Context<LoaderApp>,
) -> [AnyElement; 3] {
    let name = |suffix: &str| ElementId::Name(format!("{id}-{suffix}").into());
    let pipe_tip = match pipe {
        Some(pipe) => format!(
            "Appending every new line to {}. Click to stop.",
            pipe.path().display()
        ),
        None => {
            "Appends every new line to a file you pick, and creates the file if it doesn't exist."
                .to_owned()
        }
    };
    [
        widgets::button(
            name("copy"),
            "Copy all",
            !empty,
            rows_listener(RowsAction::Copy, what, file_name, rows, cx),
        )
        .when(!empty, |this| {
            this.tooltip(widgets::tooltip(
                "Copies the shown rows, a tab between columns.",
            ))
        })
        .into_any_element(),
        widgets::button(
            name("save"),
            "Save",
            !empty,
            rows_listener(RowsAction::Save, what, file_name, rows, cx),
        )
        .when(!empty, |this| {
            this.tooltip(widgets::tooltip(
                "Saves the shown rows to a text file, a tab between columns.",
            ))
        })
        .into_any_element(),
        widgets::button(
            name("pipe"),
            "Pipe",
            true,
            rows_listener(RowsAction::Pipe, what, file_name, rows, cx),
        )
        .when(pipe.is_some(), |this| {
            this.bg(theme::SEL).border_color(theme::BLUE)
        })
        .tooltip(widgets::tooltip(pipe_tip))
        .into_any_element(),
    ]
}

fn script_rows(app: &LoaderApp, range: Range<usize>) -> Vec<LogRow> {
    let inspector = &app.script_inspector;
    let visible = inspector.visible();
    range
        .filter_map(|index| {
            let entry = visible.get(index)?;
            let first = index == 0 || visible[index - 1].run.tick != entry.run.tick;
            Some(LogRow {
                cells: vec![
                    if first {
                        entry.tick_text()
                    } else {
                        String::new()
                    },
                    entry.indented_name(&inspector.names),
                    entry.args_text(),
                    entry.took(),
                ],
                color: None,
                selected: false,
                pick: None,
            })
        })
        .collect()
}

fn var_rows(app: &LoaderApp, range: Range<usize>) -> Vec<LogRow> {
    let visible = app.vars.visible();
    range
        .filter_map(|index| {
            let entry = visible.get(index)?;
            let first = index == 0 || visible[index - 1].tick != entry.tick;
            Some(LogRow {
                cells: vec![
                    if first {
                        entry.tick_text()
                    } else {
                        String::new()
                    },
                    entry.kind.label().to_owned(),
                    entry.name.clone(),
                    entry.old.clone(),
                    entry.new.clone(),
                ],
                color: None,
                selected: false,
                pick: None,
            })
        })
        .collect()
}

fn packet_rows(app: &LoaderApp, range: Range<usize>) -> Vec<LogRow> {
    let monitor = &app.packets;
    let visible = monitor.visible();
    let table = monitor.table();
    range
        .filter_map(|index| {
            let packet = visible.get(index)?;
            let def = table.lookup(packet.sent, packet.opcode);
            Some(LogRow {
                cells: vec![
                    crate::packets::time_text(packet),
                    if packet.sent { "→" } else { "←" }.to_owned(),
                    packet.opcode.to_string(),
                    packet.length.to_string(),
                    def.map_or(String::new(), |def| def.name.clone()),
                    crate::packets::fields_text(def, packet),
                    crate::packets::payload_text(packet),
                ],
                color: packet.sent.then_some(theme::BLUE),
                selected: monitor.selected == Some(packet.sequence),
                pick: Some(packet.sequence),
            })
        })
        .collect()
}

fn widget_rows(inspector: &crate::widgets::Inspector, open: &HashSet<(i32, i32)>) -> Vec<TreeRow> {
    let built = inspector.built();
    let mut rows = Vec::new();
    let mut stack: Vec<(usize, usize)> = built.roots.iter().rev().map(|&root| (root, 0)).collect();
    while let Some((index, depth)) = stack.pop() {
        let node = &built.nodes[index];
        if inspector.hide_hidden && node.widget.hidden {
            continue;
        }
        let parent = !node.children.is_empty();
        let opened = parent && open.contains(&(node.widget.id, node.widget.index));
        rows.push(TreeRow {
            node: index,
            depth,
            open: parent.then_some(opened),
        });
        if opened {
            stack.extend(node.children.iter().rev().map(|&child| (child, depth + 1)));
        }
    }
    rows
}

impl LoaderApp {
    fn dev_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Inputs {
        if let Some((inputs, _)) = &self.dev_ui.inputs {
            return inputs.clone();
        }
        let (inputs, subscriptions) = Inputs::new(self, window, cx);
        self.dev_ui.inputs = Some((inputs.clone(), subscriptions));
        inputs
    }

    fn sync_dev_inputs(&mut self, inputs: &Inputs, window: &mut Window, cx: &mut Context<Self>) {
        let Inputs {
            varp,
            blacklist,
            filter,
            name_search,
        } = inputs;
        if varp.read(cx).value().trim().parse::<usize>().ok() != Some(self.varp) {
            sync_input(varp, &self.varp.to_string(), window, cx);
        }
        sync_input(blacklist, &self.script_inspector.blacklist, window, cx);
        sync_input(filter, &self.packets.filter, window, cx);
        sync_input(name_search, &self.packets.name_search, window, cx);
    }

    pub(super) fn render_development(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let inputs = self.dev_inputs(window, cx);
        self.sync_dev_inputs(&inputs, window, cx);
        let tabs = wrapped_row().children(DevTab::ALL.into_iter().map(|(tab, label)| {
            selectable(("dev-tab", tab as usize), label, self.dev_tab == tab).on_click(cx.listener(
                move |app, _: &ClickEvent, _, cx| {
                    app.dev_tab = tab;
                    app.wake.wake();
                    cx.notify();
                },
            ))
        }));
        let body = match self.dev_tab {
            DevTab::Tools => div()
                .flex()
                .flex_col()
                .child(self.render_dev_tools(window, cx))
                .overflow_y_scrollbar()
                .id("dev-tools")
                .into_any_element(),
            DevTab::Inspectors => self
                .render_inspectors(&inputs, window, cx)
                .into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .child(tabs.flex_none())
            .child(widgets::separator().flex_none())
            .child(
                widgets::tab_content()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(body),
            )
            .into_any_element()
    }

    fn render_inspectors(
        &mut self,
        inputs: &Inputs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let tabs = wrapped_row().children(InspectorTab::ALL.into_iter().map(|(tab, label)| {
            selectable(
                ("inspector-tab", tab as usize),
                label,
                self.inspector_tab == tab,
            )
            .on_click(cx.listener(move |app, _: &ClickEvent, _, cx| {
                app.inspector_tab = tab;
                app.wake.wake();
                cx.notify();
            }))
        }));
        let body = match self.inspector_tab {
            InspectorTab::Widget => self.render_widgets(window, cx),
            InspectorTab::Var => self.render_vars(inputs, cx),
            InspectorTab::Script => self.render_script_log(inputs, cx),
            InspectorTab::Inventory => self.render_inventory(cx),
            InspectorTab::Packet => self.render_packets(inputs, cx),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .child(tabs.flex_none())
            .child(widgets::separator().flex_none())
            .child(
                widgets::tab_content()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(body),
            )
    }

    fn render_dev_tools(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let state = self.render_state(cx);
        let widest = widgets::widest(window, &DEV_TOGGLES, theme::BODY) + CHECK_INDICATOR;
        let per_group = DEV_TOGGLES.len().div_ceil(DEV_COLUMNS);
        let groups = (0..DEV_TOGGLES.len())
            .step_by(per_group)
            .map(|start| {
                let group = start..(start + per_group).min(DEV_TOGGLES.len());
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .w(widest)
                    .gap(theme::LINE_GAP)
                    .children(group.map(|index| {
                        let enabled = DEV_LAYERS[index].is_some();
                        check(
                            ("dev-toggle", index),
                            DEV_TOGGLES[index],
                            self.dev_toggles[index],
                            cx,
                            move |app, on| app.dev_toggles[index] = on,
                        )
                        .disabled(!enabled)
                        .when(!enabled, |this| this.tooltip("Not ported yet."))
                    }))
            })
            .collect::<Vec<_>>();
        let debugging = check(
            "script-debugging",
            "Script debugging",
            self.settings.script_debugging,
            cx,
            |app, on| {
                app.settings.script_debugging = on;
                for run in app.runs.iter_mut().filter(|r| r.is_running()) {
                    if let Err(error) = run.send(|s| s.push_debugging(on)) {
                        run.error(error);
                    }
                }
            },
        )
        .tooltip("Scripts and the lib draw what they plan, such as the route of each walk click.");
        let section_gap = || {
            div()
                .flex()
                .flex_col()
                .py(px(12.0))
                .child(widgets::separator().flex_none())
        };
        div()
            .flex()
            .flex_col()
            .child(widgets::heading("Game state"))
            .child(div().h(px(4.0)))
            .child(state)
            .child(section_gap())
            .child(widgets::heading("Overlays"))
            .child(div().h(px(4.0)))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_start()
                    .gap_x(TOGGLE_GAP)
                    .gap_y(px(8.0))
                    .children(groups),
            )
            .child(section_gap())
            .child(widgets::heading("Scripts"))
            .child(div().h(px(4.0)))
            .child(div().flex().child(debugging))
    }

    fn render_loaded_target(&self) -> (Div, Option<Target>) {
        let target = self.selected_loaded_target();
        let label = match target {
            Some(target) => format!("Client {}.", target.client.pid),
            None => "Select a client with agent loaded in the Clients tab.".to_owned(),
        };
        (div().pb(px(6.0)).child(label), target.cloned())
    }

    fn render_state(&mut self, cx: &mut Context<Self>) -> Div {
        let (label, target) = self.render_loaded_target();
        let enabled = self.busy.is_none() && target.is_some();
        let buttons = [
            ("Game state", Read::GameState),
            ("Position", Read::Position),
            ("Chunk", Read::Chunk),
            ("Skills", Read::Skills),
            ("Tick count", Read::TickCount),
            ("Login screen", Read::Login),
            ("Top-level interface", Read::RootInterface),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (label, read))| {
            let target = target.clone();
            widgets::button(
                ("read-state", index),
                label,
                enabled,
                cx.listener(move |app, _: &ClickEvent, _, cx| {
                    if let Some(target) = target.clone() {
                        app.read_state(target, read);
                    }
                    cx.notify();
                }),
            )
        });
        div()
            .flex()
            .flex_col()
            .child(label)
            .child(wrapped_row().children(buttons))
    }

    fn render_varps(&mut self, inputs: &Inputs, cx: &mut Context<Self>) -> Div {
        let (label, target) = self.render_loaded_target();
        let enabled = self.busy.is_none() && target.is_some();
        let read_button =
            |id: ElementId, label: &'static str, read: Read, cx: &mut Context<Self>| {
                let target = target.clone();
                widgets::button(
                    id,
                    label,
                    enabled,
                    cx.listener(move |app, _: &ClickEvent, _, cx| {
                        if let Some(target) = target.clone() {
                            app.read_state(target, read);
                        }
                        cx.notify();
                    }),
                )
            };
        let controls = div()
            .flex()
            .items_center()
            .gap(theme::ITEM_GAP)
            .child("Varp")
            .child(
                div()
                    .w(px(120.0))
                    .child(NumberInput::new(&inputs.varp).small()),
            )
            .child(read_button("varp-read".into(), "Read", Read::Varp(self.varp), cx))
            .child(div().w(px(1.0)).h(theme::INTERACT_HEIGHT).bg(theme::BORDER))
            .child(read_button(
                "varp-known".into(),
                "Read all known",
                Read::KnownVarps,
                cx,
            ))
            .child(
                read_button("varp-changes".into(), "Changes", Read::VarpChanges, cx).when(
                    enabled,
                    |this| {
                        this.tooltip(widgets::tooltip("Lists every varp that changed since the previous read. Read, change something in game, then click this to find its varp."))
                    },
                ),
            );
        let widths = [px(170.0), px(50.0), px(170.0)];
        let header = div()
            .flex()
            .items_center()
            .h(theme::INTERACT_HEIGHT)
            .children(
                ["Varp", "Id", "Last read"]
                    .into_iter()
                    .zip(widths)
                    .map(|(title, width)| widgets::strong(title).w(width).px(CELL_PAD)),
            );
        let rows = KNOWN_VARPS.iter().enumerate().map(|(index, &(name, id))| {
            let value = match self.varps.as_ref().and_then(|varps| varps.get(id)) {
                Some(&value) => match describe_varp(id, value) {
                    Some(text) => format!("{value} ({text})"),
                    None => value.to_string(),
                },
                None => "-".to_owned(),
            };
            div()
                .flex()
                .items_center()
                .h(theme::INTERACT_HEIGHT + px(2.0))
                .when(index % 2 == 0, |this| this.bg(theme::BG_FLOAT))
                .child(div().w(widths[0]).px(CELL_PAD).truncate().child(name))
                .child(div().w(widths[1]).px(CELL_PAD).child(id.to_string()))
                .child(div().w(widths[2]).px(CELL_PAD).truncate().child(value))
                .child(read_button(
                    ("varp-row", index).into(),
                    "Read",
                    Read::Varp(id),
                    cx,
                ))
        });
        div()
            .flex()
            .flex_col()
            .child(label)
            .child(controls.flex_none())
            .child(div().h(px(6.0)))
            .child(
                div()
                    .max_h(px(240.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_start()
                            .child(header)
                            .children(rows),
                    )
                    .overflow_y_scrollbar()
                    .id("known-varps"),
            )
    }

    fn render_log_table(
        &mut self,
        table: TableId,
        columns: &'static Columns,
        rows: usize,
        follow: bool,
        build: fn(&LoaderApp, Range<usize>) -> Vec<LogRow>,
        cx: &mut Context<Self>,
    ) -> Div {
        let state = self.dev_ui.table(table);
        let (widths, scroll) = (state.widths.clone(), state.scroll.clone());
        if follow && scroll.is_scrolled_to_end().unwrap_or(true) {
            scroll.scroll_to_bottom();
        }
        let header = div().flex().flex_none().h(ROW).overflow_hidden().children(
            columns.iter().enumerate().map(|(column, &(title, width))| {
                let last = column + 1 == columns.len();
                cell(widths[column], rest_of(width))
                    .relative()
                    .font_weight(FontWeight::BOLD)
                    .child(title)
                    .when(!last, |this| {
                        this.child(
                            div()
                                .id(("column-resize", column))
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .right_0()
                                .w(px(5.0))
                                .flex()
                                .justify_end()
                                .cursor_col_resize()
                                .child(div().w(px(1.0)).h_full().bg(theme::BORDER))
                                .on_drag(ColumnDrag { table, column }, |_, _, _, cx| {
                                    cx.new(|_| Empty)
                                }),
                        )
                    })
            }),
        );
        let list = uniform_list(
            ("log-table", table as usize),
            rows,
            cx.processor(move |app, range: Range<usize>, _, cx| {
                let widths = app.dev_ui.table(table).widths.clone();
                let start = range.start;
                build(app, range)
                    .into_iter()
                    .enumerate()
                    .map(|(offset, row)| {
                        let index = start + offset;
                        div()
                            .id(("log-row", index))
                            .flex()
                            .w_full()
                            .h(ROW)
                            .overflow_hidden()
                            .font_family(theme::MONO)
                            .when(index % 2 == 1, |this| this.bg(theme::BG_FLOAT))
                            .when(row.selected, |this| this.bg(theme::SEL))
                            .when_some(row.color, |this, color| this.text_color(color))
                            .children(row.cells.into_iter().enumerate().map(|(column, text)| {
                                cell(widths[column], rest_of(columns[column].1)).child(text)
                            }))
                            .when_some(row.pick, |this, sequence| {
                                this.cursor_pointer()
                                    .hover(|style| style.bg(theme::BG_HL))
                                    .on_click(cx.listener(move |app, _: &ClickEvent, _, cx| {
                                        app.packets.selected = Some(sequence);
                                        app.packets.follow = false;
                                        cx.notify();
                                    }))
                            })
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&scroll)
        .size_full();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(if table == TableId::Packets {
                ROW * 5.0
            } else {
                px(0.0)
            })
            .on_drag_move(
                cx.listener(move |app, event: &DragMoveEvent<ColumnDrag>, _, cx| {
                    let drag = *event.drag(cx);
                    if drag.table != table {
                        return;
                    }
                    let state = app.dev_ui.table_mut(table);
                    let left = event.bounds.left()
                        + state.widths[..drag.column]
                            .iter()
                            .fold(px(0.0), |sum, &width| sum + width);
                    state.widths[drag.column] = (event.event.position.x - left).max(MIN_COLUMN);
                    cx.notify();
                }),
            )
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(list)
                    .vertical_scrollbar(&scroll),
            )
    }

    fn render_widgets(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let pid = self.selected_loaded_target().map(|t| t.client.pid);
        let busy = self.widgets.reading();
        let controls = wrapped_row()
            .child(widgets::button(
                "widgets-refresh",
                "Refresh",
                pid.is_some() && !busy,
                cx.listener(|app, _: &ClickEvent, _, cx| {
                    if let Some(target) = app.selected_loaded_target().cloned() {
                        app.refresh_widgets(target);
                    }
                    cx.notify();
                }),
            ))
            .child(check(
                "widgets-hide-hidden",
                "Hide hidden",
                self.widgets.hide_hidden,
                cx,
                |app, on| app.widgets.hide_hidden = on,
            ))
            .child(
                check(
                    "widgets-follow-mouse",
                    "Follow mouse",
                    self.widgets.follow_mouse,
                    cx,
                    |app, on| app.widgets.follow_mouse = on,
                )
                .tooltip("Selects the widget under the client's mouse, looking about ten times a second. Move the mouse over the client and the tree follows; untick it to keep the selection."),
            );
        if !self.widgets.has_read()
            && !self.widgets.reading()
            && self.widgets.error.is_none()
            && let Some(target) = self.selected_loaded_target().cloned()
        {
            self.refresh_widgets(target);
        }
        let reveal = std::mem::take(&mut self.widgets.reveal);
        let inspector = &self.widgets;
        let ui = &mut self.dev_ui;
        if reveal {
            let built = inspector.built();
            ui.widget_open
                .extend(inspector.open_path().into_iter().map(|index| {
                    let widget = &built.nodes[index].widget;
                    (widget.id, widget.index)
                }));
        }
        ui.widget_rows = widget_rows(inspector, &ui.widget_open);
        let selected_row = inspector.selected.and_then(|key| {
            ui.widget_rows.iter().position(|row| {
                let widget = &inspector.built().nodes[row.node].widget;
                (widget.id, widget.index) == key
            })
        });
        let scroll = ui.widget_tree.clone();
        if reveal && let Some(row) = selected_row {
            scroll.scroll_to_item(row, ScrollStrategy::Center);
        }
        let widest_row = ui
            .widget_rows
            .iter()
            .enumerate()
            .max_by_key(|(_, row)| {
                row.depth * 3
                    + inspector
                        .identifier(&inspector.built().nodes[row.node].widget)
                        .len()
            })
            .map(|(index, _)| index);
        let tree = uniform_list(
            "widget-tree",
            ui.widget_rows.len(),
            cx.processor(move |app, range: Range<usize>, _, cx| {
                let inspector = &app.widgets;
                let rows = &app.dev_ui.widget_rows;
                range
                    .filter_map(|index| {
                        let row = *rows.get(index)?;
                        let node = &inspector.built().nodes[row.node];
                        let key = (node.widget.id, node.widget.index);
                        let selected = inspector.selected == Some(key);
                        let label = format!(
                            "{} {}",
                            node.kind.letter(),
                            inspector.identifier(&node.widget)
                        );
                        let guides = (0..row.depth).map(|level| {
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left(INDENT * level as f32 + px(6.0))
                                .w(px(1.0))
                                .bg(theme::BORDER)
                        });
                        let line = div()
                            .id(("widget-row", index))
                            .relative()
                            .flex()
                            .items_center()
                            .h(ROW)
                            .pl(INDENT * row.depth as f32)
                            .whitespace_nowrap()
                            .cursor_pointer()
                            .children(guides)
                            .on_click(cx.listener(move |app, _: &ClickEvent, _, cx| {
                                app.widgets.selected = Some(key);
                                if row.open.is_some() {
                                    let open = &mut app.dev_ui.widget_open;
                                    if !open.remove(&key) {
                                        open.insert(key);
                                    }
                                }
                                app.wake.wake();
                                cx.notify();
                            }));
                        Some(match row.open {
                            Some(open) => line
                                .text_color(if selected { theme::BLUE } else { theme::FG })
                                .hover(|style| style.text_color(theme::BLUE))
                                .child(caret(open))
                                .child(label),
                            None => line.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .h_full()
                                    .px(px(4.0))
                                    .border_1()
                                    .border_color(theme::alpha(theme::BLUE, 0.0))
                                    .when(selected, |this| {
                                        this.bg(theme::SEL).text_color(theme::BLUE)
                                    })
                                    .when(!selected, |this| {
                                        this.hover(|style| {
                                            style.bg(theme::SEL).border_color(theme::BLUE)
                                        })
                                    })
                                    .child(label),
                            ),
                        })
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&scroll)
        .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
        .with_width_from_item(widest_row)
        .size_full();
        let fields = match inspector.selected_node() {
            Some(node) => {
                let fields = inspector.fields(node);
                let names: Vec<&str> = fields.iter().map(|(name, _)| *name).collect();
                let name_width = widgets::widest(window, &names, theme::BODY) + CELL_PAD * 2.0;
                div()
                    .flex()
                    .flex_col()
                    .children(
                        fields
                            .into_iter()
                            .enumerate()
                            .map(|(index, (name, value))| {
                                div()
                                    .flex()
                                    .items_start()
                                    .py(px(2.0))
                                    .when(index % 2 == 1, |this| this.bg(theme::BG_FLOAT))
                                    .child(div().flex_none().w(name_width).px(CELL_PAD).child(name))
                                    .child(div().flex_1().min_w_0().px(CELL_PAD).child(value))
                            }),
                    )
            }
            None => widgets::weak(
                "Pick a widget in the tree, or tick Follow mouse and point at one in the client.",
            ),
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(controls.flex_none())
            .child(client_caption(pid).flex_none())
            .when_some(self.widgets.error.clone(), |this, error| {
                this.child(error_label(error).flex_none())
            })
            .child(widgets::separator().flex_none())
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .gap(theme::ITEM_GAP)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .relative()
                            .child(tree)
                            .vertical_scrollbar(&scroll),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(fields.overflow_y_scrollbar().id("widget-fields")),
                    ),
            )
    }

    fn render_inventory(&mut self, cx: &mut Context<Self>) -> Div {
        let pid = self.selected_loaded_target().map(|t| t.client.pid);
        let controls = wrapped_row()
            .child(check(
                "inventory-enable",
                "Enable",
                self.inventory.enabled,
                cx,
                |app, on| app.inventory.enabled = on,
            ))
            .child(
                widgets::button(
                    "inventory-refresh",
                    format!("Refresh ({})", self.inventory.pending()),
                    true,
                    cx.listener(|app, _: &ClickEvent, _, cx| {
                        app.inventory.refresh();
                        cx.notify();
                    }),
                )
                .tooltip(widgets::tooltip(
                    "Adds the changes read since the last refresh to the list.",
                )),
            )
            .child(widgets::button(
                "inventory-clear",
                "Clear",
                true,
                cx.listener(|app, _: &ClickEvent, _, cx| {
                    app.inventory.clear();
                    cx.notify();
                }),
            ));
        let error = self
            .inventory
            .error
            .iter()
            .chain(&self.icons.error)
            .next()
            .cloned();
        let open = &self.dev_ui.containers_open;
        let tree = div()
            .flex()
            .flex_col()
            .children(self.inventory.logs().iter().map(|(&id, log)| {
                let opened = open.contains(&id);
                let header = div()
                    .id(("inventory-container", id as usize))
                    .flex()
                    .items_center()
                    .h(ROW)
                    .cursor_pointer()
                    .hover(|style| style.text_color(theme::BLUE))
                    .child(caret(opened))
                    .child(self.inventory.container_label(id))
                    .on_click(cx.listener(move |app, _: &ClickEvent, _, cx| {
                        let open = &mut app.dev_ui.containers_open;
                        if !open.remove(&id) {
                            open.insert(id);
                        }
                        cx.notify();
                    }));
                div().flex().flex_col().child(header).when(opened, |this| {
                    this.child(div().flex().flex_col().items_start().pl(INDENT).children(
                        log.iter().enumerate().map(|(index, snapshot)| {
                            let label = snapshot.tick.map_or_else(
                                || "Tick: ?".to_owned(),
                                |tick| format!("Tick: {tick}"),
                            );
                            selectable(
                                ElementId::Name(format!("inventory-{id}-{index}").into()),
                                label,
                                self.inventory.selected == Some((id, index)),
                            )
                            .on_click(cx.listener(
                                move |app, _: &ClickEvent, _, cx| {
                                    app.inventory.selected = Some((id, index));
                                    cx.notify();
                                },
                            ))
                        }),
                    ))
                })
            }));
        let items = match self.inventory.picked() {
            Some((snapshot, before)) => {
                inventory_items(&self.inventory, &mut self.icons, snapshot, before)
            }
            None => widgets::weak("Pick a tick to see the items."),
        };
        if self.icons.due(Instant::now()) {
            self.wake.wake();
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(controls.flex_none())
            .child(client_caption(pid).flex_none())
            .when_some(error, |this, error| {
                this.child(error_label(error).flex_none())
            })
            .child(widgets::separator().flex_none())
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .gap(theme::ITEM_GAP)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(tree.overflow_y_scrollbar().id("inventory-tree")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(items.overflow_y_scrollbar().id("inventory-items")),
                    ),
            )
    }
}

fn inventory_items(
    inspector: &crate::inventory::Inspector,
    icons: &mut crate::icons::Icons,
    snapshot: &crate::inventory::Snapshot,
    before: Option<&crate::inventory::Snapshot>,
) -> Div {
    let mut icon = |id: i32, quantity: i32| {
        let key = crate::icons::Key::item(id, quantity, inspector.is_stackable(id));
        crate::icons::slot(icons.image(key), inspector.item_name(id))
    };
    let mut lines = Vec::new();
    if let Some(before) = before {
        let (added, removed) = crate::inventory::compare(&before.items, &snapshot.items, |id| {
            inspector.is_stackable(id)
        });
        for (title, list, color) in [
            ("Items Added:", added, ADDED),
            ("Items Removed:", removed, REMOVED),
        ] {
            if list.is_empty() {
                continue;
            }
            lines.push(widgets::strong(title));
            for (id, quantity) in list {
                lines.push(
                    div()
                        .flex()
                        .items_center()
                        .gap(theme::ITEM_GAP)
                        .child(icon(id, quantity))
                        .child(div().text_color(gpui_kit::rgb(color)).child(format!(
                            "{} x{quantity}  (id {id})",
                            inspector.item_name(id)
                        ))),
                );
            }
        }
    }
    let slot_width = px(crate::icons::SLOT_SIZE.width);
    let grid = div()
        .flex()
        .flex_wrap()
        .gap_x(SLOT_GAP_X)
        .gap_y(SLOT_GAP_Y)
        .map(|this| {
            if snapshot.items.len() == 28 {
                this.w(slot_width * 4.0 + SLOT_GAP_X * 3.0)
            } else {
                this.w_full()
            }
        })
        .children(snapshot.items.iter().enumerate().map(|(index, item)| {
            if item.id < 0 {
                crate::icons::slot(None, "")
                    .id(("inventory-slot", index))
                    .tooltip(widgets::tooltip(format!("Slot {}: empty", item.slot)))
            } else {
                icon(item.id, item.quantity)
                    .id(("inventory-slot", index))
                    .tooltip(widgets::tooltip(format!(
                        "Slot {}: {} x{}  (id {})",
                        item.slot,
                        inspector.item_name(item.id),
                        item.quantity,
                        item.id
                    )))
            }
        }));
    div()
        .flex()
        .flex_col()
        .gap(theme::LINE_GAP)
        .children(lines)
        .child(widgets::strong("Items in Container:"))
        .child(grid)
}

impl LoaderApp {
    fn render_script_log(&mut self, inputs: &Inputs, cx: &mut Context<Self>) -> Div {
        let pid = self.selected_loaded_target().map(|t| t.client.pid);
        let inspector = &self.script_inspector;
        let loading = inspector.loading_names;
        let buttons = rows_buttons(
            "script-log",
            inspector.kept() == 0,
            inspector.pipe.as_ref(),
            "script runs",
            "scripts.txt",
            |app| &mut app.script_inspector,
            cx,
        );
        let controls = wrapped_row()
            .child(check("script-enable", "Enable", inspector.enabled, cx, |app, on| {
                app.script_inspector.enabled = on
            }))
            .child(
                check(
                    "script-triggers",
                    "Trigger events",
                    inspector.triggers,
                    cx,
                    |app, on| app.script_inspector.triggers = on,
                )
                .tooltip("Component and menu events, which find their script by trigger so their script id isn't known."),
            )
            .child(check("script-follow", "Follow", inspector.follow, cx, |app, on| {
                app.script_inspector.follow = on
            }))
            .child("Blacklist")
            .child(
                div()
                    .id("script-blacklist")
                    .w(px(140.0))
                    .child(Input::new(&inputs.blacklist).small())
                    .tooltip(widgets::tooltip("Scripts to leave out: ids, or names once the names are loaded. Separate with commas or spaces. A client runs a few scripts every frame; the default names the ones seen on an idle account.")),
            )
            .child(
                widgets::button(
                    "script-load-names",
                    if loading { "Loading names…" } else { "Load names" },
                    !loading,
                    cx.listener(|app, _: &ClickEvent, _, cx| {
                        app.script_inspector.loading_names = true;
                        app.script_inspector.names_error = None;
                        app.background(|_| {
                            Event::ScriptNames(crate::script_names::Names::download())
                        });
                        cx.notify();
                    }),
                )
                .when(!loading, |this| {
                    this.tooltip(widgets::tooltip("Downloads RuneLite's names for the client's scripts from github.com/runelite/cs2-scripts (about 10 MB) and shows them in the list. The client has none of its own. They match the game build the repository was last decompiled from."))
                }),
            )
            .child(widgets::button(
                "script-clear",
                "Clear",
                true,
                cx.listener(|app, _: &ClickEvent, _, cx| {
                    app.script_inspector.clear();
                    cx.notify();
                }),
            ))
            .children(buttons);
        let inspector = &self.script_inspector;
        let shown = inspector.visible().len();
        let names = &inspector.names;
        let summary = div()
            .flex()
            .flex_col()
            .child(client_caption(pid).flex_none())
            .when_some(inspector.names_error.clone(), |this, error| {
                this.child(error_label(error))
            })
            .when(!names.is_empty(), |this| {
                this.child(widgets::weak(format!(
                    "{} script names, from RuneLite's cs2-scripts at {}.",
                    names.len(),
                    names.source
                )))
            })
            .child(widgets::weak(format!(
                "{shown} shown of {} kept{}. Scripts started by trigger lookup aren't listed.",
                inspector.kept(),
                missed(inspector.dropped)
            )))
            .when_some(inspector.error.clone(), |this, error| {
                this.child(error_label(error))
            });
        let follow = inspector.follow;
        let table = self.render_log_table(
            TableId::Scripts,
            &SCRIPT_COLUMNS,
            shown,
            follow,
            script_rows,
            cx,
        );
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(controls.flex_none())
            .child(summary.flex_none())
            .child(widgets::separator().flex_none())
            .child(table)
    }

    fn render_vars(&mut self, inputs: &Inputs, cx: &mut Context<Self>) -> Div {
        let varps_open = self.dev_ui.varps_open;
        let varps = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("varps-header")
                    .flex()
                    .items_center()
                    .h(theme::INTERACT_HEIGHT)
                    .cursor_pointer()
                    .hover(|style| style.text_color(theme::BLUE))
                    .child(caret(varps_open))
                    .child("Varps")
                    .on_click(cx.listener(|app, _: &ClickEvent, _, cx| {
                        app.dev_ui.varps_open = !app.dev_ui.varps_open;
                        cx.notify();
                    })),
            )
            .when(varps_open, |this| {
                this.child(div().pl(INDENT).child(self.render_varps(inputs, cx)))
            });
        let pid = self.selected_loaded_target().map(|t| t.client.pid);
        let inspector = &self.vars;
        let buttons = rows_buttons(
            "vars",
            inspector.kept() == 0,
            inspector.pipe.as_ref(),
            "var changes",
            "vars.txt",
            |app| &mut app.vars,
            cx,
        );
        let controls = wrapped_row()
            .child(check(
                "vars-enable",
                "Enable",
                inspector.enabled,
                cx,
                |app, on| app.vars.enabled = on,
            ))
            .child(check(
                "vars-varbits",
                "Varbit",
                inspector.varbits,
                cx,
                |app, on| app.vars.varbits = on,
            ))
            .child(check(
                "vars-varps",
                "VarPlayer",
                inspector.varps,
                cx,
                |app, on| app.vars.varps = on,
            ))
            .child(check(
                "vars-varc-ints",
                "VarClientInt",
                inspector.varc_ints,
                cx,
                |app, on| app.vars.varc_ints = on,
            ))
            .child(check(
                "vars-varc-strings",
                "VarClientStr",
                inspector.varc_strings,
                cx,
                |app, on| app.vars.varc_strings = on,
            ))
            .child(check(
                "vars-follow",
                "Follow",
                inspector.follow,
                cx,
                |app, on| app.vars.follow = on,
            ))
            .child(widgets::button(
                "vars-clear",
                "Clear",
                true,
                cx.listener(|app, _: &ClickEvent, _, cx| {
                    app.vars.clear();
                    cx.notify();
                }),
            ))
            .children(buttons);
        let inspector = &self.vars;
        let shown = inspector.visible().len();
        let summary = div()
            .flex()
            .flex_col()
            .child(client_caption(pid).flex_none())
            .child(widgets::weak(format!(
                "{shown} shown of {} kept. Client variables are listed as their index.",
                inspector.kept()
            )))
            .when_some(inspector.error.clone(), |this, error| {
                this.child(error_label(error))
            })
            .when_some(inspector.varc_error.clone(), |this, error| {
                this.child(warning(format!("Client variables: {error}")))
            });
        let follow = inspector.follow;
        let table = self.render_log_table(TableId::Vars, &VAR_COLUMNS, shown, follow, var_rows, cx);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(varps.flex_none())
            .child(widgets::separator().flex_none())
            .child(controls.flex_none())
            .child(summary.flex_none())
            .child(widgets::separator().flex_none())
            .child(table)
    }

    fn render_names_menu(&self, inputs: &Inputs, cx: &mut Context<Self>) -> Popover {
        let monitor = &self.packets;
        let active = crate::packets::named_words(&monitor.filter, monitor.table())
            + usize::from(name_state(&monitor.filter, UNKNOWN) != NameState::Off);
        let title = if active > 0 {
            format!("Names ({active}) ⏷")
        } else {
            "Names ⏷".to_owned()
        };
        let app = cx.entity().downgrade();
        let search = inputs.name_search.clone();
        Popover::new("packet-names")
            .trigger(Button::new("packet-names-button").label(title).small())
            .content(move |_, _, cx| names_menu(&app, &search, cx))
    }

    fn render_packets(&mut self, inputs: &Inputs, cx: &mut Context<Self>) -> Div {
        let pid = self.selected_loaded_target().map(|t| t.client.pid);
        let monitor = &self.packets;
        let buttons = rows_buttons(
            "packets",
            monitor.kept() == 0,
            monitor.pipe.as_ref(),
            "packets",
            "packets.txt",
            |app| &mut app.packets,
            cx,
        );
        let controls = wrapped_row()
            .child(check(
                "packets-enable",
                "Enable",
                monitor.capture,
                cx,
                |app, on| app.packets.capture = on,
            ))
            .child(check(
                "packets-received",
                "Received",
                monitor.received,
                cx,
                |app, on| app.packets.received = on,
            ))
            .child(check(
                "packets-sent",
                "Sent",
                monitor.sent,
                cx,
                |app, on| app.packets.sent = on,
            ))
            .child(check(
                "packets-follow",
                "Follow",
                monitor.follow,
                cx,
                |app, on| app.packets.follow = on,
            ))
            .child(
                widgets::button(
                    "packets-clear",
                    "Clear",
                    true,
                    cx.listener(|app, _: &ClickEvent, _, cx| {
                        app.packets.clear();
                        cx.notify();
                    }),
                )
                .tooltip(widgets::tooltip("Forget the packets read so far.")),
            )
            .children(buttons);
        let filtered = !monitor.filter.is_empty();
        let filter = div()
            .flex()
            .items_center()
            .gap(theme::ITEM_GAP)
            .child("Filter")
            .child(
                div()
                    .id("packet-filter")
                    .flex_1()
                    .min_w(px(120.0))
                    .child(Input::new(&inputs.filter).small())
                    .tooltip(widgets::tooltip("Only these packets, or with a ! in front, all but these. Opcodes or names (see the Names menu), separated by commas or spaces. UNKNOWN stands for the packets without a name.")),
            )
            .child(self.render_names_menu(inputs, cx))
            .child(
                small_button(
                    "packet-filter-clear",
                    "✖",
                    filtered,
                    cx.listener(|app, _: &ClickEvent, _, cx| {
                        app.packets.filter.clear();
                        cx.notify();
                    }),
                )
                .when(filtered, |this| this.tooltip(widgets::tooltip("Clear the filter"))),
            );
        let monitor = &self.packets;
        let shown = monitor.visible().len();
        let summary = div()
            .flex()
            .flex_col()
            .child(client_caption(pid).flex_none())
            .child(widgets::weak(format!(
                "{shown} shown of {} kept{}",
                monitor.kept(),
                missed(monitor.dropped)
            )))
            .when_some(monitor.error.clone(), |this, error| {
                this.child(error_label(error))
            });
        let follow = monitor.follow;
        let table = self.render_log_table(
            TableId::Packets,
            &PACKET_COLUMNS,
            shown,
            follow,
            packet_rows,
            cx,
        );
        let detail = self.render_packet_detail(cx);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(controls.flex_none())
            .child(div().flex_none().pt(theme::LINE_GAP).child(filter))
            .child(summary.flex_none())
            .child(widgets::separator().flex_none())
            .child(table)
            .child(widgets::separator().flex_none())
            .child(detail)
    }

    fn render_packet_detail(&self, cx: &mut Context<Self>) -> Div {
        let monitor = &self.packets;
        let detail = div().flex().flex_col().flex_none().h(DETAIL_HEIGHT);
        let Some(packet) = monitor.selected_packet() else {
            return detail.child(widgets::weak("Select a packet to see it in full."));
        };
        let bytes = crate::packets::payload_bytes(packet);
        let hex = crate::packets::payload_text(packet);
        let heading = wrapped_row()
            .child(widgets::strong(if packet.sent {
                "Sent"
            } else {
                "Received"
            }))
            .child(format!(
                "opcode {}, {} bytes, at {:.3}s",
                packet.opcode,
                packet.length,
                packet.time_ms as f64 / 1000.0
            ))
            .when((bytes.len() as u32) < packet.length, |this| {
                this.child(widgets::weak(format!("(first {} kept)", bytes.len())))
            })
            .child(small_button(
                "packet-copy-hex",
                "Copy hex",
                true,
                cx.listener(move |app, _: &ClickEvent, _, cx| {
                    app.copy = Some(hex.clone());
                    cx.notify();
                }),
            ));
        let values = crate::packets::leading_values(&bytes);
        let table = monitor.table();
        let def = table.lookup(packet.sent, packet.opcode);
        let decoded = def.map(|def| crate::packet_defs::decode(def, &bytes, packet.length));
        let mut body = div().flex().flex_col();
        if let (Some(def), Some(decoded)) = (def, &decoded) {
            let fields: Vec<String> = decoded
                .fields
                .iter()
                .map(|(label, value)| format!("{label} = {value}"))
                .collect();
            let title = if def.name.is_empty() {
                format!("opcode {}", def.opcode)
            } else {
                def.name.clone()
            };
            body = body
                .child(
                    div()
                        .id("packet-layout")
                        .font_family(theme::MONO)
                        .font_weight(FontWeight::BOLD)
                        .child(format!("{title}  {}", fields.join("  ·  ")))
                        .tooltip(widgets::tooltip(format!("Layout: {}.", table.source))),
                )
                .when(!def.has_fields(), |this| {
                    this.child(widgets::weak("The layout of this packet isn't known (its handler reads conditionally or in a loop)."))
                })
                .when(!def.touches.is_empty(), |this| {
                    this.child(widgets::weak(format!(
                        "the client's handler touches: {}",
                        def.touches.join(", ")
                    )))
                })
                .when_some(decoded.note.clone(), |this, note| this.child(warning(note)));
        }
        body = body.children(crate::packets::hex_dump(&bytes).into_iter().map(|line| {
            div()
                .font_family(theme::MONO)
                .whitespace_nowrap()
                .child(line)
        }));
        detail
            .child(heading)
            .when(!values.is_empty(), |this| {
                this.child(widgets::weak(
                    values
                        .iter()
                        .map(|(kind, value)| format!("{kind} {value}"))
                        .collect::<Vec<_>>()
                        .join("  ·  ")
                        + "  (big endian, from the start)",
                ))
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(body.overflow_y_scrollbar().id("packet-detail")),
            )
    }
}

fn names_menu(
    app: &WeakEntity<LoaderApp>,
    search: &Entity<InputState>,
    cx: &mut App,
) -> AnyElement {
    let Some(entity) = app.upgrade() else {
        return Empty.into_any_element();
    };
    let monitor = &entity.read(cx).packets;
    let query = monitor.name_search.trim().to_ascii_lowercase();
    let unknown_matches = UNKNOWN.to_ascii_lowercase().contains(&query);
    let mut named: Vec<(bool, u16, String)> = monitor
        .table()
        .named()
        .map(|(sent, opcode, name)| (sent, opcode, name.to_owned()))
        .filter(|(_, _, name)| name.to_ascii_lowercase().contains(&query))
        .collect();
    named.sort_by(|a, b| a.2.cmp(&b.2).then(a.0.cmp(&b.0)));
    let states: Vec<(String, String, NameState)> = named
        .into_iter()
        .map(|(sent, opcode, name)| {
            let state = name_state(&monitor.filter, &name);
            let detail = format!("{} {opcode}", if sent { "→" } else { "←" });
            (name, detail, state)
        })
        .collect();
    let unknown = unknown_matches.then(|| name_state(&monitor.filter, UNKNOWN));
    let filtered = !monitor.filter.is_empty();
    let count = monitor.table().named().count();
    let row = |index: usize, name: String, detail: String, state: NameState| {
        let (glyph, color) = match state {
            NameState::Off => ("", theme::COMMENT),
            NameState::Only => ("✔", theme::BLUE),
            NameState::Hidden => ("✖", theme::RED),
        };
        let next = state.next();
        let app = app.clone();
        let target = name.clone();
        div()
            .id(("packet-name", index))
            .flex()
            .items_center()
            .h(px(18.0))
            .gap(theme::ITEM_GAP)
            .cursor_pointer()
            .hover(|style| style.bg(theme::SEL))
            .child(
                div()
                    .flex_none()
                    .w(px(20.0))
                    .text_color(color)
                    .font_weight(FontWeight::BOLD)
                    .child(glyph),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(theme::MONO)
                    .when(state != NameState::Off, |this| this.text_color(color))
                    .child(name),
            )
            .child(widgets::weak(detail).flex_none())
            .on_click(move |_: &ClickEvent, _, cx: &mut App| {
                app.update(cx, |loader, cx| {
                    loader.packets.filter = with_name(&loader.packets.filter, &target, next);
                    cx.notify();
                })
                .ok();
            })
    };
    let empty = states.is_empty() && unknown.is_none();
    let list = div()
        .flex()
        .flex_col()
        .when(empty, |this| this.child(widgets::weak("No name matches.")))
        .children(
            states
                .into_iter()
                .enumerate()
                .map(|(index, (name, detail, state))| row(index + 1, name, detail, state)),
        );
    let clear = {
        let app = app.clone();
        widgets::button(
            "packet-names-clear",
            "Clear filter",
            filtered,
            move |_: &ClickEvent, _, cx: &mut App| {
                app.update(cx, |loader, cx| {
                    loader.packets.filter.clear();
                    cx.notify();
                })
                .ok();
            },
        )
    };
    div()
        .flex()
        .flex_col()
        .w(px(300.0))
        .gap(theme::LINE_GAP)
        .child(Input::new(search).small())
        .child(widgets::weak("Click a name: show only → hide → off."))
        .child(widgets::separator().flex_none())
        .when_some(unknown, |this, state| {
            this.child(row(0, UNKNOWN.to_owned(), "no name".to_owned(), state))
                .child(widgets::separator().flex_none())
        })
        .child(
            div()
                .max_h(px(320.0))
                .child(list)
                .overflow_y_scrollbar()
                .id("packet-names-list"),
        )
        .child(widgets::separator().flex_none())
        .child(
            div()
                .flex()
                .items_center()
                .gap(theme::ITEM_GAP)
                .child(clear)
                .child(widgets::weak(format!("{count} named"))),
        )
        .into_any_element()
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("development-tools", |app| app.tab = Tab::Development),
        ("development-tools-client", |app| {
            fake_client(app);
            app.tab = Tab::Development;
            app.dev_toggles[0] = true;
            app.dev_toggles[5] = true;
            app.settings.script_debugging = true;
        }),
        ("development-tools-busy", |app| {
            fake_client(app);
            app.tab = Tab::Development;
            app.busy = Some("Reading…");
        }),
        ("development-widget-empty", |app| {
            inspector(app, InspectorTab::Widget)
        }),
        ("development-widget", |app| {
            fake_client(app);
            fake_widgets(app);
            inspector(app, InspectorTab::Widget);
        }),
        ("development-widget-error", |app| {
            fake_client(app);
            app.widgets.start(PID);
            app.widgets
                .finish(PID, Err("The agent didn't answer.".into()));
            inspector(app, InspectorTab::Widget);
        }),
        ("development-var", |app| {
            fake_client(app);
            fake_vars(app);
            inspector(app, InspectorTab::Var);
        }),
        ("development-var-varps", |app| {
            fake_client(app);
            fake_vars(app);
            app.dev_ui.varps_open = true;
            inspector(app, InspectorTab::Var);
        }),
        ("development-var-empty", |app| {
            inspector(app, InspectorTab::Var)
        }),
        ("development-script", |app| {
            fake_client(app);
            fake_scripts(app);
            inspector(app, InspectorTab::Script);
        }),
        ("development-script-empty", |app| {
            app.script_inspector.loading_names = true;
            inspector(app, InspectorTab::Script);
        }),
        ("development-inventory", |app| {
            fake_client(app);
            fake_inventory(app);
            inspector(app, InspectorTab::Inventory);
        }),
        ("development-inventory-empty", |app| {
            inspector(app, InspectorTab::Inventory)
        }),
        ("development-packet", |app| {
            fake_client(app);
            fake_packets(app);
            inspector(app, InspectorTab::Packet);
        }),
        ("development-packet-empty", |app| {
            inspector(app, InspectorTab::Packet)
        }),
    ]
}

#[cfg(test)]
const PID: u32 = 4242;

#[cfg(test)]
fn inspector(app: &mut LoaderApp, tab: InspectorTab) {
    app.tab = Tab::Development;
    app.dev_tab = DevTab::Inspectors;
    app.inspector_tab = tab;
}

#[cfg(test)]
fn fake_client(app: &mut LoaderApp) {
    let target: Target = serde_json::from_value(serde_json::json!({
        "client": {
            "pid": PID,
            "exe": "client.exe",
            "title": "Client",
            "is_64bit": true,
            "loaded": "dev",
        },
        "location": { "prefix": "/nonexistent", "wine": "/nonexistent/wine", "env": [] },
    }))
    .expect("a fake client target");
    app.selected = Some(target.key());
    app.targets = vec![target];
}

#[cfg(test)]
fn fake_widgets(app: &mut LoaderApp) {
    use injector::{SubInterface, TreeWidget, Widget, WidgetDetail, WidgetTree};
    let widget = |group: i32, child: i32, parent: i32, index: i32, hidden: bool| TreeWidget {
        widget: Widget {
            id: group << 16 | child,
            group,
            child,
            kind: 3,
            x: 10 * child,
            y: 4 * child,
            width: 40,
            height: 20,
            parent,
            sprite: -1,
            index,
            hidden,
            text: (child == 3).then(|| "Click here to continue".to_owned()),
            text_color: 0xff981f,
            scroll_x: 0,
            scroll_y: 0,
            scroll_width: 0,
            scroll_height: 0,
            action: None,
            item_id: -1,
            item_quantity: 0,
        },
        detail: WidgetDetail::default(),
        dynamic: index != -1,
        owner: if index != -1 { parent } else { -1 },
    };
    let id = |group: i32, child: i32| group << 16 | child;
    let tree = WidgetTree {
        root: 161,
        sub_interfaces: vec![SubInterface {
            group: 149,
            parent: id(161, 2),
            parent_group: 161,
            parent_child: 2,
        }],
        widgets: vec![
            widget(161, 0, -1, -1, false),
            widget(161, 1, id(161, 0), -1, false),
            widget(161, 2, id(161, 0), -1, false),
            widget(161, 3, id(161, 0), -1, true),
            widget(161, 4, id(161, 0), -1, false),
            widget(149, 0, -1, -1, false),
            widget(149, 1, id(149, 0), -1, false),
            widget(149, 0, id(149, 0), 0, false),
            widget(149, 0, id(149, 0), 1, false),
            widget(149, 2, id(149, 0), -1, false),
        ],
    };
    app.widgets.start(PID);
    app.widgets.finish(
        PID,
        Ok(crate::widgets::Poll {
            tree,
            names: vec![(161, "toplevel".into()), (149, "inventory".into())],
        }),
    );
    app.widgets.selected = Some((id(149, 0), 1));
    app.widgets.reveal = true;
}

#[cfg(test)]
fn fake_vars(app: &mut LoaderApp) {
    use injector::{VarcChange, VarcPage};
    let poll = |varps: Vec<i32>, tick: i32, changes: Vec<VarcChange>| crate::vars::Poll {
        varps: Some(varps),
        tick: Some(tick),
        varbits: Vec::new(),
        varcs: VarcPage {
            changes,
            next: 10,
            dropped: 0,
        },
        varc_error: None,
    };
    let change = |index: i32, string: bool, old: &str, new: &str| VarcChange {
        sequence: index as u64,
        time_ms: 0,
        index,
        string,
        old: Some(old.to_owned()),
        new: new.to_owned(),
    };
    app.vars.enabled = true;
    app.vars.start(PID);
    app.vars.finish(
        PID,
        Instant::now(),
        Ok(poll(vec![0; 400], 100, Vec::new())),
        |_| None,
    );
    let mut varps = vec![0; 400];
    varps[varp::RUN] = 1;
    varps[281] = 1000;
    app.vars.start(PID);
    app.vars.finish(
        PID,
        Instant::now(),
        Ok(poll(
            varps,
            101,
            vec![change(171, false, "0", "3"), change(335, true, "", "Hello")],
        )),
        |id| known_varp(id).map(str::to_owned),
    );
    app.vars.varc_error = Some("the client's varc table moved".into());
    app.varps = Some(vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
}

#[cfg(test)]
fn fake_scripts(app: &mut LoaderApp) {
    use injector::{ScriptPage, ScriptRun};
    let run = |sequence: u64, tick: i32, script: i32, depth: u8, end: bool| ScriptRun {
        sequence,
        time_us: sequence * 150,
        tick: Some(tick),
        end,
        event_type: 0,
        arg_count: 2,
        depth,
        args: vec![script.to_string(), format!("{}", tick % 7)],
    };
    let mut runs = Vec::new();
    for tick in 0..60 {
        let base = tick as u64 * 4;
        runs.push(run(base, 5000 + tick, 2046, 0, false));
        runs.push(run(base + 1, 5000 + tick, 3174, 1, false));
        runs.push(run(base + 2, 5000 + tick, 3174, 1, true));
        runs.push(run(base + 3, 5000 + tick, 2046, 0, true));
    }
    app.script_inspector.enabled = true;
    app.script_inspector.start(PID);
    app.script_inspector.finish(
        PID,
        Instant::now(),
        Ok(crate::script_log::Poll {
            page: ScriptPage {
                runs,
                next: 240,
                dropped: 3,
            },
        }),
    );
    app.script_inspector.names = crate::script_names::names_for_test(&[
        (2046, "chatbox_keyinput_matched"),
        (3174, "script3174"),
    ]);
}

#[cfg(test)]
fn fake_inventory(app: &mut LoaderApp) {
    use injector::{Item, ItemChange, ItemPage};
    let item = |slot: i32, id: i32, quantity: i32| Item { slot, id, quantity };
    let mut items: Vec<Item> = (0..28).map(|slot| item(slot, -1, 0)).collect();
    items[0] = item(0, 995, 1200);
    items[1] = item(1, 1511, 1);
    items[2] = item(2, 1511, 1);
    let page = |changes: Vec<ItemChange>, next: u64| ItemPage {
        changes,
        next,
        dropped: 0,
    };
    let poll = |page: ItemPage, baseline: Option<Vec<(i32, Vec<Item>)>>| crate::inventory::Poll {
        page,
        baseline,
        container_names: vec![(93, "inventory".into()), (94, "worn".into())],
        item_names: vec![
            (995, "Coins".into()),
            (1511, "Logs".into()),
            (590, "Tinderbox".into()),
        ],
        stackable: vec![(995, true), (1511, false), (590, false)],
    };
    let change =
        |sequence: u64, tick: i32, slot: i32, new_item: i32, new_quantity: i32| ItemChange {
            sequence,
            time_ms: 0,
            tick: Some(tick),
            container: 93,
            slot,
            old_known: true,
            old_item: -1,
            old_quantity: 0,
            new_item,
            new_quantity,
        };
    app.inventory.enabled = true;
    app.inventory.start(PID);
    app.inventory.finish(
        PID,
        Instant::now(),
        Ok(poll(page(Vec::new(), 0), Some(vec![(93, items)]))),
    );
    app.inventory.start(PID);
    app.inventory.finish(
        PID,
        Instant::now(),
        Ok(poll(
            page(
                vec![
                    change(0, 300, 3, 590, 1),
                    change(1, 301, 1, -1, 0),
                    change(2, 301, 0, 995, 1500),
                ],
                3,
            ),
            None,
        )),
    );
    app.inventory.refresh();
    app.inventory.selected = Some((93, 1));
    app.dev_ui.containers_open.insert(93);
}

#[cfg(test)]
fn fake_packets(app: &mut LoaderApp) {
    use injector::{Packet, PacketPage};
    let packets = (0..80)
        .map(|sequence| Packet {
            sequence,
            time_ms: 1000 + sequence * 37,
            sent: sequence % 3 == 0,
            opcode: [40, 12, 77, 5][sequence as usize % 4],
            length: 6,
            payload: format!("{:02x}01ff00{:02x}2a", sequence, sequence * 3 % 256),
        })
        .collect();
    app.packets.capture = true;
    app.packets.start(PID);
    app.packets.finish(
        PID,
        Instant::now(),
        Ok(PacketPage {
            packets,
            next: 80,
            dropped: 0,
        }),
    );
    app.packets.selected = Some(70);
    app.packets.follow = false;
    app.packets.filter = "!5".into();
}
