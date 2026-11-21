use super::*;

pub(super) fn client_caption(ui: &mut egui::Ui, pid: Option<u32>) {
    match pid {
        Some(pid) => {
            ui.weak(format!("Client {pid}."));
        }
        None => {
            ui.label("Select a client with agent loaded in the Clients tab.");
        }
    }
}

pub(super) struct LogTable<'a> {
    id: &'a str,
    follow: bool,
    row_height: f32,
    max_height: Option<f32>,
    clickable: bool,
}

impl LogTable<'_> {
    pub(super) fn show(
        &self,
        ui: &mut egui::Ui,
        columns: &[(&str, egui_extras::Column)],
        rows: usize,
        row: impl FnMut(egui_extras::TableRow<'_, '_>),
    ) {
        ui.scope(|ui| {
            ui.style_mut().interaction.selectable_labels = !self.clickable;
            let mut table = egui_extras::TableBuilder::new(ui)
                .id_salt(self.id)
                .striped(true)
                .resizable(true)
                .stick_to_bottom(self.follow)
                .auto_shrink([false, false])
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
            if let Some(height) = self.max_height {
                table = table.max_scroll_height(height);
            }
            if self.clickable {
                table = table.sense(egui::Sense::click());
            }
            for (_, column) in columns {
                table = table.column(*column);
            }
            table
                .header(self.row_height, |header| {
                    crate::theme::table_header(header, columns.iter().map(|(title, _)| *title));
                })
                .body(|body| body.rows(self.row_height, rows, row));
        });
    }
}

pub(super) fn narrow(width: f32) -> egui_extras::Column {
    egui_extras::Column::initial(width)
        .at_least(24.0)
        .clip(true)
}

pub(super) fn rest(min: f32) -> egui_extras::Column {
    egui_extras::Column::remainder().at_least(min).clip(true)
}

pub(super) fn mono_row(ui: &egui::Ui) -> (egui::FontId, f32) {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font)) + ui.spacing().item_spacing.y;
    (font, row_height)
}

pub(super) fn packet_names_menu(ui: &mut egui::Ui, monitor: &mut crate::packets::Monitor) {
    use crate::packets::{NameState, UNKNOWN, name_state, with_name};

    let active = crate::packets::named_words(&monitor.filter, monitor.table())
        + usize::from(crate::packets::name_state(&monitor.filter, UNKNOWN) != NameState::Off);
    let title = if active > 0 {
        format!("Names ({active}) ⏷")
    } else {
        "Names ⏷".to_owned()
    };
    let config = egui::containers::menu::MenuConfig::new()
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    egui::containers::menu::MenuButton::new(title)
        .config(config)
        .ui(ui, |ui| {
            ui.set_width(300.0);
            ui.add(
                egui::TextEdit::singleline(&mut monitor.name_search)
                    .desired_width(f32::INFINITY)
                    .hint_text("🔍 Search names"),
            );
            ui.add_space(2.0);
            ui.weak("Click a name: show only → hide → off.");
            ui.separator();

            let search = monitor.name_search.trim().to_ascii_lowercase();
            let unknown_matches = UNKNOWN.to_ascii_lowercase().contains(&search);
            let mut rows: Vec<(bool, u16, String)> = monitor
                .table()
                .named()
                .map(|(sent, opcode, name)| (sent, opcode, name.to_owned()))
                .filter(|(_, _, name)| name.to_ascii_lowercase().contains(&search))
                .collect();
            rows.sort_by(|a, b| a.2.cmp(&b.2).then(a.0.cmp(&b.0)));
            let mut clicked = None;
            let mut row = |ui: &mut egui::Ui, name: &str, detail: String| {
                let state = name_state(&monitor.filter, name);
                let (glyph, color) = match state {
                    NameState::Off => ("", ui.visuals().weak_text_color()),
                    NameState::Only => ("✔", ui.visuals().hyperlink_color),
                    NameState::Hidden => ("✖", ui.visuals().error_fg_color),
                };
                let response = ui
                    .horizontal(|ui| {
                        ui.add_sized(
                            [20.0, 18.0],
                            egui::Label::new(egui::RichText::new(glyph).color(color).strong()),
                        );
                        let text = egui::RichText::new(name).monospace();
                        let text = if state == NameState::Off {
                            text
                        } else {
                            text.color(color)
                        };
                        ui.label(text);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.weak(detail)
                        });
                    })
                    .response
                    .interact(egui::Sense::click());
                if response.hovered() {
                    ui.painter().rect_filled(
                        response.rect,
                        3.0,
                        ui.visuals().widgets.hovered.weak_bg_fill,
                    );
                }
                if response.clicked() {
                    clicked = Some((name.to_owned(), state.next()));
                }
            };
            if unknown_matches {
                row(ui, UNKNOWN, "no name".to_owned());
                ui.separator();
            }
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if rows.is_empty() && !unknown_matches {
                        ui.weak("No name matches.");
                    }
                    for (sent, opcode, name) in &rows {
                        row(
                            ui,
                            name,
                            format!("{} {opcode}", if *sent { "→" } else { "←" }),
                        );
                    }
                });
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !monitor.filter.is_empty(),
                        egui::Button::new("Clear filter"),
                    )
                    .clicked()
                {
                    monitor.filter.clear();
                }
                ui.weak(format!("{} named", monitor.table().named().count()));
            });
            if let Some((name, state)) = clicked {
                monitor.filter = with_name(&monitor.filter, &name, state);
            }
        });
}

pub(super) fn mono(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).monospace()
}

pub(super) fn rows_buttons(
    ui: &mut egui::Ui,
    empty: bool,
    pipe: Option<&crate::pipe::Pipe>,
) -> Option<RowsAction> {
    let mut action = None;
    if ui
        .add_enabled(!empty, egui::Button::new("Copy all"))
        .on_hover_text("Copies the shown rows, a tab between columns.")
        .clicked()
    {
        action = Some(RowsAction::Copy);
    }
    if ui
        .add_enabled(!empty, egui::Button::new("Save"))
        .on_hover_text("Saves the shown rows to a text file, a tab between columns.")
        .clicked()
    {
        action = Some(RowsAction::Save);
    }
    if crate::pipe::button(ui, pipe) {
        action = Some(RowsAction::Pipe);
    }
    action
}

pub(super) fn inventory_items_ui(
    ui: &mut egui::Ui,
    inspector: &crate::inventory::Inspector,
    icons: &mut crate::icons::Icons,
    snapshot: &crate::inventory::Snapshot,
    before: Option<&crate::inventory::Snapshot>,
) {
    let mut icon = |ui: &mut egui::Ui, id: i32, quantity: i32| {
        let key = crate::icons::Key::item(id, quantity, inspector.is_stackable(id));
        icons.image(key);
        ui.label(inspector.item_name(id))
    };
    let mut line = |ui: &mut egui::Ui, id: i32, quantity: i32, color: egui::Color32| {
        ui.horizontal(|ui| {
            icon(ui, id, quantity);
            ui.colored_label(
                color,
                format!("{} x{quantity}  (id {id})", inspector.item_name(id)),
            );
        });
    };
    if let Some(before) = before {
        let (added, removed) = crate::inventory::compare(&before.items, &snapshot.items, |id| {
            inspector.is_stackable(id)
        });
        if !added.is_empty() {
            ui.strong("Items Added:");
            for (id, quantity) in added {
                line(ui, id, quantity, egui::Color32::from_rgb(90, 200, 90));
            }
        }
        if !removed.is_empty() {
            ui.strong("Items Removed:");
            for (id, quantity) in removed {
                line(ui, id, quantity, egui::Color32::from_rgb(230, 100, 100));
            }
        }
    }
    ui.strong("Items in Container:");
    let columns = if snapshot.items.len() == 28 {
        4
    } else {
        ((ui.available_width() / crate::icons::SLOT_CELL.width) as usize).max(1)
    };
    egui::Grid::new("inventory_slots")
        .spacing(egui::vec2(8.0, 6.0))
        .show(ui, |ui| {
            for (i, item) in snapshot.items.iter().enumerate() {
                if item.id < 0 {
                    ui.label("")
                        .on_hover_text(format!("Slot {}: empty", item.slot));
                } else {
                    icon(ui, item.id, item.quantity).on_hover_text(format!(
                        "Slot {}: {} x{}  (id {})",
                        item.slot,
                        inspector.item_name(item.id),
                        item.quantity,
                        item.id
                    ));
                }
                if (i + 1) % columns == 0 {
                    ui.end_row();
                }
            }
        });
}

pub(super) fn widget_node_ui(
    ui: &mut egui::Ui,
    inspector: &crate::widgets::Inspector,
    index: usize,
    open: &HashSet<usize>,
    reveal: bool,
    picked: &mut Option<(i32, i32)>,
) {
    let node = &inspector.built().nodes[index];
    if inspector.hide_hidden && node.widget.hidden {
        return;
    }
    let key = (node.widget.id, node.widget.index);
    let label = format!(
        "{} {}",
        node.kind.letter(),
        inspector.identifier(&node.widget)
    );
    let selected = *picked == Some(key);
    if node.children.is_empty() {
        let response = ui.selectable_label(selected, label);
        if response.clicked() {
            *picked = Some(key);
        }
        if selected && reveal {
            response.scroll_to_me(Some(egui::Align::Center));
        }
        return;
    }
    let mut header = egui::CollapsingHeader::new(egui::RichText::new(label).color(if selected {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().text_color()
    }))
    .id_salt(("widget", node.widget.id, node.widget.index, index));
    if reveal && open.contains(&index) {
        header = header.open(Some(true));
    }
    let shown = header.show(ui, |ui| {
        for &child in &node.children {
            widget_node_ui(ui, inspector, child, open, reveal, picked);
        }
    });
    if shown.header_response.clicked() {
        *picked = Some(key);
    }
    if selected && reveal {
        shown
            .header_response
            .scroll_to_me(Some(egui::Align::Center));
    }
}

impl LoaderApp {
    pub(super) fn development_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for (tab, label) in DevTab::ALL {
                ui.selectable_value(&mut self.dev_tab, tab, label);
            }
        });
        ui.separator();
        crate::theme::tab_content(ui, |ui| match self.dev_tab {
            DevTab::Tools => {
                egui::ScrollArea::vertical()
                    .id_salt("dev_tools")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.dev_tools_ui(ui));
            }
            DevTab::Inspectors => self.inspectors_ui(ui),
        });
    }

    pub(super) fn inspectors_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for (tab, label) in InspectorTab::ALL {
                ui.selectable_value(&mut self.inspector_tab, tab, label);
            }
        });
        ui.separator();
        crate::theme::tab_content(ui, |ui| match self.inspector_tab {
            InspectorTab::Widget => self.widgets_ui(ui),
            InspectorTab::Var => self.vars_ui(ui),
            InspectorTab::Script => self.script_log_ui(ui),
            InspectorTab::Inventory => self.inventory_ui(ui),
            InspectorTab::Packet => self.packets_ui(ui),
        });
    }

    pub(super) fn dev_tools_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Game state");
        ui.add_space(4.0);
        self.state_ui(ui);
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(12.0);
        ui.heading("Overlays");
        ui.add_space(4.0);
        let gap = 24.0;
        let font = egui::TextStyle::Body.resolve(ui.style());
        let widest = crate::theme::widest(ui, &font, &DEV_TOGGLES)
            + ui.spacing().icon_width
            + ui.spacing().icon_spacing;
        let per_group = DEV_TOGGLES.len().div_ceil(DEV_COLUMNS);
        let groups: Vec<std::ops::Range<usize>> = (0..DEV_TOGGLES.len())
            .step_by(per_group)
            .map(|start| start..(start + per_group).min(DEV_TOGGLES.len()))
            .collect();
        let beside = (((ui.available_width() + gap) / (widest + gap)).floor() as usize)
            .clamp(1, groups.len());
        for (row, row_groups) in groups.chunks(beside).enumerate() {
            if row > 0 {
                ui.add_space(8.0);
            }
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for group in row_groups {
                    ui.vertical(|ui| {
                        ui.set_width(widest);
                        for index in group.clone() {
                            ui.add_enabled(
                                DEV_LAYERS[index].is_some(),
                                egui::Checkbox::new(
                                    &mut self.dev_toggles[index],
                                    DEV_TOGGLES[index],
                                ),
                            )
                            .on_disabled_hover_text("Not ported yet.");
                        }
                    });
                }
            });
        }
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(12.0);
        ui.heading("Scripts");
        ui.add_space(4.0);
        if ui
            .checkbox(&mut self.settings.script_debugging, "Script debugging")
            .on_hover_text(
                "Scripts and the lib draw what they plan, such as the route of each walk click.",
            )
            .changed()
        {
            let on = self.settings.script_debugging;
            for run in self.runs.iter_mut().filter(|r| r.is_running()) {
                if let Err(error) = run.send(|s| s.push_debugging(on)) {
                    run.error(error);
                }
            }
        }
    }

    pub(super) fn state_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let target = self.loaded_target_ui(ui);

        let enabled = self.busy.is_none() && target.is_some();
        ui.horizontal_wrapped(|ui| {
            for (label, read) in [
                ("Game state", Read::GameState),
                ("Position", Read::Position),
                ("Chunk", Read::Chunk),
                ("Skills", Read::Skills),
                ("Tick count", Read::TickCount),
                ("Login screen", Read::Login),
                ("Top-level interface", Read::RootInterface),
            ] {
                if ui.add_enabled(enabled, egui::Button::new(label)).clicked()
                    && let Some(target) = target.clone()
                {
                    self.read_state(target, read);
                }
            }
        });
    }

    pub(super) fn varps_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let target = self.loaded_target_ui(ui);
        let enabled = self.busy.is_none() && target.is_some();
        let mut read = None;

        ui.horizontal(|ui| {
            ui.label("Varp");
            ui.add(egui::DragValue::new(&mut self.varp).range(0..=injector::MAX_VARPS - 1));
            if ui.add_enabled(enabled, egui::Button::new("Read")).clicked() {
                read = Some(Read::Varp(self.varp));
            }
            ui.separator();
            if ui.add_enabled(enabled, egui::Button::new("Read all known")).clicked() {
                read = Some(Read::KnownVarps);
            }
            if ui
                .add_enabled(enabled, egui::Button::new("Changes"))
                .on_hover_text("Lists every varp that changed since the previous read. Read, change something in game, then click this to find its varp.")
                .clicked()
            {
                read = Some(Read::VarpChanges);
            }
        });
        ui.add_space(6.0);

        egui::ScrollArea::vertical()
            .max_height(240.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                egui::Grid::new("known varps")
                    .num_columns(4)
                    .striped(true)
                    .show(ui, |ui| {
                        for header in ["Varp", "Id", "Last read", ""] {
                            ui.strong(header);
                        }
                        ui.end_row();
                        for &(name, id) in &KNOWN_VARPS {
                            ui.label(name);
                            ui.label(id.to_string());
                            match self.varps.as_ref().and_then(|varps| varps.get(id)) {
                                Some(&value) => ui.label(match describe_varp(id, value) {
                                    Some(text) => format!("{value} ({text})"),
                                    None => value.to_string(),
                                }),
                                None => ui.label("-"),
                            };
                            if ui.add_enabled(enabled, egui::Button::new("Read")).clicked() {
                                read = Some(Read::Varp(id));
                            }
                            ui.end_row();
                        }
                    });
            });

        if let (Some(read), Some(target)) = (read, target) {
            self.read_state(target, read);
        }
    }

    pub(super) fn widgets_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let pid = self.selected_loaded_target().map(|t| t.client.pid);
        let mut refresh = false;
        ui.horizontal_wrapped(|ui| {
            let busy = self.widgets.reading();
            refresh = ui.add_enabled(pid.is_some() && !busy, egui::Button::new("Refresh")).clicked();
            ui.checkbox(&mut self.widgets.hide_hidden, "Hide hidden");
            ui.checkbox(&mut self.widgets.follow_mouse, "Follow mouse")
                .on_hover_text("Selects the widget under the client's mouse, looking about ten times a second. Move the mouse over the client and the tree follows; untick it to keep the selection.");
        });
        client_caption(ui, pid);
        refresh |=
            !self.widgets.has_read() && !self.widgets.reading() && self.widgets.error.is_none();
        if refresh && let Some(target) = self.selected_loaded_target().cloned() {
            self.refresh_widgets(target);
        }
        if let Some(error) = &self.widgets.error {
            egui_error_label(ui, error);
        }
        ui.separator();
        let inspector = &mut self.widgets;
        let mut picked = inspector.selected;
        let reveal = std::mem::take(&mut inspector.reveal);
        let open = if reveal {
            inspector.open_path()
        } else {
            HashSet::new()
        };
        ui.columns(2, |columns| {
            egui::ScrollArea::both().id_salt("widget_tree").auto_shrink([false, false]).show(&mut columns[0], |ui| {
                for &root in &inspector.built().roots {
                    widget_node_ui(ui, inspector, root, &open, reveal, &mut picked);
                }
            });
            egui::ScrollArea::vertical().id_salt("widget_fields").auto_shrink([false, false]).show(&mut columns[1], |ui| match inspector.selected_node() {
                Some(node) => {
                    egui::Grid::new("widget_field_grid").num_columns(2).striped(true).show(ui, |ui| {
                        for (name, value) in inspector.fields(node) {
                            ui.label(name);
                            ui.add(egui::Label::new(value).selectable(true));
                            ui.end_row();
                        }
                    });
                }
                None => {
                    ui.weak("Pick a widget in the tree, or tick Follow mouse and point at one in the client.");
                }
            });
        });
        inspector.selected = picked;
    }

    pub(super) fn inventory_ui(&mut self, ui: &mut egui::Ui) {
        let pid = self.selected_loaded_target().map(|t| t.client.pid);
        let inspector = &mut self.inventory;
        let icons = &mut self.icons;
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut inspector.enabled, "Enable");
            if ui
                .button(format!("Refresh ({})", inspector.pending()))
                .on_hover_text("Adds the changes read since the last refresh to the list.")
                .clicked()
            {
                inspector.refresh();
            }
            if ui.button("Clear").clicked() {
                inspector.clear();
            }
        });
        client_caption(ui, pid);
        if let Some(error) = inspector.error.iter().chain(&icons.error).next() {
            egui_error_label(ui, error);
        }
        ui.separator();
        let mut picked = inspector.selected;
        ui.columns(2, |columns| {
            egui::ScrollArea::vertical()
                .id_salt("inventory_tree")
                .auto_shrink([false, false])
                .show(&mut columns[0], |ui| {
                    for (id, log) in inspector.logs() {
                        egui::CollapsingHeader::new(inspector.container_label(*id))
                            .id_salt(("inventory_container", *id))
                            .show(ui, |ui| {
                                for (index, snapshot) in log.iter().enumerate() {
                                    let label = snapshot.tick.map_or_else(
                                        || "Tick: ?".to_owned(),
                                        |tick| format!("Tick: {tick}"),
                                    );
                                    if ui
                                        .selectable_label(picked == Some((*id, index)), label)
                                        .clicked()
                                    {
                                        picked = Some((*id, index));
                                    }
                                }
                            });
                    }
                });
            egui::ScrollArea::vertical()
                .id_salt("inventory_items")
                .auto_shrink([false, false])
                .show(&mut columns[1], |ui| match inspector.picked() {
                    Some((snapshot, before)) => {
                        inventory_items_ui(ui, inspector, icons, snapshot, before)
                    }
                    None => {
                        ui.weak("Pick a tick to see the items.");
                    }
                });
        });
        inspector.selected = picked;
    }

    pub(super) fn script_log_ui(&mut self, ui: &mut egui::Ui) {
        let pid = self.selected_loaded_target().map(|t| t.client.pid);
        let mut load_names = false;
        let mut action = None;
        let inspector = &mut self.script_inspector;
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut inspector.enabled, "Enable");
            ui.checkbox(&mut inspector.triggers, "Trigger events").on_hover_text("Component and menu events, which find their script by trigger so their script id isn't known.");
            ui.checkbox(&mut inspector.follow, "Follow");
            ui.label("Blacklist");
            ui.add(egui::TextEdit::singleline(&mut inspector.blacklist).desired_width(140.0).hint_text("6697, 4730"))
                .on_hover_text("Scripts to leave out: ids, or names once the names are loaded. Separate with commas or spaces. A client runs a few scripts every frame; the default names the ones seen on an idle account.");
            let load = ui.add_enabled(!inspector.loading_names, egui::Button::new(if inspector.loading_names { "Loading names…" } else { "Load names" })).on_hover_text(
                "Downloads RuneLite's names for the client's scripts from github.com/runelite/cs2-scripts (about 10 MB) and shows them in the list. The client has none of its own. They match the game build the repository was last decompiled from.",
            );
            if load.clicked() {
                load_names = true;
            }
            if ui.button("Clear").clicked() {
                inspector.clear();
            }
            action = rows_buttons(ui, inspector.kept() == 0, inspector.pipe.as_ref());
        });
        self.rows_action(action, "script runs", "scripts.txt", |app| {
            &mut app.script_inspector
        });
        let inspector = &mut self.script_inspector;
        client_caption(ui, pid);
        if let Some(error) = &inspector.names_error {
            egui_error_label(ui, error);
        }
        let visible = inspector.visible();
        let names = &inspector.names;
        if !names.is_empty() {
            ui.weak(format!(
                "{} script names, from RuneLite's cs2-scripts at {}.",
                names.len(),
                names.source
            ));
        }
        ui.weak(format!(
            "{} shown of {} kept{}. Scripts started by trigger lookup aren't listed.",
            visible.len(),
            inspector.kept(),
            missed(inspector.dropped)
        ));
        if let Some(error) = &inspector.error {
            egui_error_label(ui, error);
        }
        ui.separator();
        let (_, row_height) = mono_row(ui);
        let table = LogTable {
            id: "script_log",
            follow: inspector.follow,
            row_height,
            max_height: None,
            clickable: false,
        };
        table.show(
            ui,
            &[
                ("Tick", narrow(60.0)),
                ("Script", narrow(240.0)),
                ("Arguments", narrow(260.0)),
                ("Time", rest(60.0)),
            ],
            visible.len(),
            |mut row| {
                let index = row.index();
                let entry = &visible[index];
                let first = index == 0 || visible[index - 1].run.tick != entry.run.tick;
                row.col(|ui| {
                    if first {
                        ui.label(mono(entry.tick_text()));
                    }
                });
                row.col(|ui| {
                    ui.label(mono(entry.indented_name(names)));
                });
                row.col(|ui| {
                    ui.label(mono(entry.args_text()));
                });
                row.col(|ui| {
                    ui.label(mono(entry.took()));
                });
            },
        );
        drop(visible);
        if load_names {
            inspector.loading_names = true;
            inspector.names_error = None;
            self.background(|_| {
                Event::ScriptNames(crate::script_names::Names::download())
            });
        }
    }

    pub(super) fn vars_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Varps")
            .default_open(false)
            .show(ui, |ui| self.varps_ui(ui));
        ui.separator();
        let pid = self.selected_loaded_target().map(|t| t.client.pid);
        let mut action = None;
        let inspector = &mut self.vars;
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut inspector.enabled, "Enable");
            ui.checkbox(&mut inspector.varbits, "Varbit");
            ui.checkbox(&mut inspector.varps, "VarPlayer");
            ui.checkbox(&mut inspector.varc_ints, "VarClientInt");
            ui.checkbox(&mut inspector.varc_strings, "VarClientStr");
            ui.checkbox(&mut inspector.follow, "Follow");
            if ui.button("Clear").clicked() {
                inspector.clear();
            }
            action = rows_buttons(ui, inspector.kept() == 0, inspector.pipe.as_ref());
        });
        self.rows_action(action, "var changes", "vars.txt", |app| {
            &mut app.vars
        });
        let inspector = &mut self.vars;
        client_caption(ui, pid);
        let visible = inspector.visible();
        ui.weak(format!(
            "{} shown of {} kept. Client variables are listed as their index.",
            visible.len(),
            inspector.kept()
        ));
        if let Some(error) = &inspector.error {
            egui_error_label(ui, error);
        }
        if let Some(error) = &inspector.varc_error {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("Client variables: {error}"),
            );
        }
        ui.separator();
        let (_, row_height) = mono_row(ui);
        let table = LogTable {
            id: "vars",
            follow: inspector.follow,
            row_height,
            max_height: None,
            clickable: false,
        };
        table.show(
            ui,
            &[
                ("Tick", narrow(60.0)),
                ("Kind", narrow(100.0)),
                ("Name", narrow(220.0)),
                ("Old", narrow(110.0)),
                ("New", rest(60.0)),
            ],
            visible.len(),
            |mut row| {
                let index = row.index();
                let entry = &visible[index];
                let first = index == 0 || visible[index - 1].tick != entry.tick;
                row.col(|ui| {
                    if first {
                        ui.label(mono(entry.tick_text()));
                    }
                });
                row.col(|ui| {
                    ui.label(mono(entry.kind.label()));
                });
                row.col(|ui| {
                    ui.label(mono(entry.name.as_str()));
                });
                row.col(|ui| {
                    ui.label(mono(entry.old.as_str()));
                });
                row.col(|ui| {
                    ui.label(mono(entry.new.as_str()));
                });
            },
        );
    }

    pub(super) fn packets_ui(&mut self, ui: &mut egui::Ui) {
        let pid = self.selected_loaded_target().map(|t| t.client.pid);
        let mut action = None;
        let monitor = &mut self.packets;
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut monitor.capture, "Enable");
            ui.checkbox(&mut monitor.received, "Received");
            ui.checkbox(&mut monitor.sent, "Sent");
            ui.checkbox(&mut monitor.follow, "Follow");
            if ui
                .button("Clear")
                .on_hover_text("Forget the packets read so far.")
                .clicked()
            {
                monitor.clear();
            }
            action = rows_buttons(ui, monitor.kept() == 0, monitor.pipe.as_ref());
        });
        self.rows_action(action, "packets", "packets.txt", |app| {
            &mut app.packets
        });
        let monitor = &mut self.packets;
        ui.horizontal(|ui| {
            ui.label("Filter");
            let menu_width = 170.0;
            ui.add(egui::TextEdit::singleline(&mut monitor.filter).desired_width((ui.available_width() - menu_width).max(120.0)).hint_text("40, VARP_LARGE or !PLAYER_INFO"))
                .on_hover_text("Only these packets, or with a ! in front, all but these. Opcodes or names (see the Names menu), separated by commas or spaces. UNKNOWN stands for the packets without a name.");
            packet_names_menu(ui, monitor);
            if ui.add_enabled(!monitor.filter.is_empty(), egui::Button::new("✖").small()).on_hover_text("Clear the filter").clicked() {
                monitor.filter.clear();
            }
        });
        client_caption(ui, pid);
        let visible = monitor.visible();
        let table = monitor.table();
        ui.weak(format!(
            "{} shown of {} kept{}",
            visible.len(),
            monitor.kept(),
            missed(monitor.dropped)
        ));
        if let Some(error) = &monitor.error {
            egui_error_label(ui, error);
        }
        ui.separator();

        let (font, row_height) = mono_row(ui);
        let sent_color = ui.visuals().hyperlink_color;
        let detail_height = 150.0;
        let mut picked = None;
        let height = (ui.available_height() - detail_height - 12.0).max(row_height * 4.0);
        let log = LogTable {
            id: "packets",
            follow: monitor.follow,
            row_height,
            max_height: Some(height),
            clickable: true,
        };
        let selected = monitor.selected;
        log.show(
            ui,
            &[
                ("Time", narrow(70.0)),
                ("", narrow(24.0)),
                ("Opcode", narrow(56.0)),
                ("Length", narrow(50.0)),
                ("Name", narrow(120.0)),
                ("Fields", narrow(240.0)),
                ("Hex", rest(80.0)),
            ],
            visible.len(),
            |mut row| {
                let packet = &visible[row.index()];
                let def = table.lookup(packet.sent, packet.opcode);
                let colored = |text: String| {
                    let text = mono(text);
                    if packet.sent {
                        text.color(sent_color)
                    } else {
                        text
                    }
                };
                row.set_selected(selected == Some(packet.sequence));
                row.col(|ui| {
                    ui.label(colored(crate::packets::time_text(packet)));
                });
                row.col(|ui| {
                    ui.label(colored(if packet.sent { "→" } else { "←" }.into()));
                });
                row.col(|ui| {
                    ui.label(colored(packet.opcode.to_string()));
                });
                row.col(|ui| {
                    ui.label(colored(packet.length.to_string()));
                });
                row.col(|ui| {
                    ui.label(colored(def.map_or(String::new(), |def| def.name.clone())));
                });
                row.col(|ui| {
                    ui.label(colored(crate::packets::fields_text(def, packet)));
                });
                row.col(|ui| {
                    ui.label(colored(crate::packets::payload_text(packet)));
                });
                if row.response().clicked() {
                    picked = Some(packet.sequence);
                }
            },
        );
        drop(visible);
        if picked.is_some() {
            monitor.selected = picked;
            monitor.follow = false;
        }

        ui.separator();
        match monitor.selected_packet() {
            None => {
                ui.weak("Select a packet to see it in full.");
            }
            Some(packet) => {
                let bytes = crate::packets::payload_bytes(packet);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new(if packet.sent { "Sent" } else { "Received" }).strong(),
                    );
                    ui.label(format!(
                        "opcode {}, {} bytes, at {:.3}s",
                        packet.opcode,
                        packet.length,
                        packet.time_ms as f64 / 1000.0
                    ));
                    if (bytes.len() as u32) < packet.length {
                        ui.weak(format!("(first {} kept)", bytes.len()));
                    }
                    if ui.small_button("Copy hex").clicked() {
                        ui.ctx().copy_text(crate::packets::payload_text(packet));
                    }
                });
                let values = crate::packets::leading_values(&bytes);
                if !values.is_empty() {
                    ui.weak(
                        values
                            .iter()
                            .map(|(kind, value)| format!("{kind} {value}"))
                            .collect::<Vec<_>>()
                            .join("  ·  ")
                            + "  (big endian, from the start)",
                    );
                }
                let def = monitor.table().lookup(packet.sent, packet.opcode);
                let decoded = def.map(|def| crate::packet_defs::decode(def, &bytes, packet.length));
                egui::ScrollArea::vertical().id_salt("packet_detail").auto_shrink([false, false]).show(ui, |ui| {
                    if let (Some(def), Some(decoded)) = (def, &decoded) {
                        let fields: Vec<String> = decoded.fields.iter().map(|(label, value)| format!("{label} = {value}")).collect();
                        let title = if def.name.is_empty() { format!("opcode {}", def.opcode) } else { def.name.clone() };
                        ui.label(egui::RichText::new(format!("{title}  {}", fields.join("  ·  "))).font(font.clone()).strong())
                            .on_hover_text(format!("Layout: {}.", monitor.table().source));
                        if !def.has_fields() {
                            ui.weak("The layout of this packet isn't known (its handler reads conditionally or in a loop).");
                        }
                        if !def.touches.is_empty() {
                            ui.weak(format!("the client's handler touches: {}", def.touches.join(", ")));
                        }
                        if let Some(note) = &decoded.note {
                            ui.colored_label(ui.visuals().warn_fg_color, note);
                        }
                    }
                    for line in crate::packets::hex_dump(&bytes) {
                        ui.label(egui::RichText::new(line).font(font.clone()));
                    }
                });
            }
        }
    }

    pub(super) fn loaded_target_ui(&self, ui: &mut egui::Ui) -> Option<Target> {
        let target = self.selected_loaded_target();
        match target {
            Some(target) => ui.label(format!("Client {}.", target.client.pid)),
            None => ui.label("Select a client with agent loaded in the Clients tab."),
        };
        ui.add_space(6.0);
        target.cloned()
    }
}

impl LoaderApp {
    pub(super) fn render_development(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        not_ported("Development")
    }
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("development", |app| app.tab = Tab::Development),
    ]
}
