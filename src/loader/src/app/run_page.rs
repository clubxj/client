use super::*;

impl LoaderApp {
    pub(super) fn running_ui(&mut self, ui: &mut egui::Ui) {
        let Some(run) = self.run() else {
            return;
        };
        let running = run.is_running();
        let paused = running && run.is_paused();
        let stopping = run.stop_requested();
        let is_store = run.store.is_some();
        let waiting = run.is_configuring();
        let form_sent = !waiting && run.form_sent.is_some();
        let can_start = waiting && !self.start_pending();
        let changed_form = form_sent && run.form.as_ref().is_some_and(|f| {
            matches!(f.submitted(self.script_layouts(), &self.known_items), crate::form::Outcome::Submitted(v) if Some(&v) != run.form_sent.as_ref())
        });
        let submit_label = run
            .form
            .as_ref()
            .map_or("Start", |f| f.submit_label())
            .to_owned();
        let has =
            |kind: crate::layouts::Kind| run.form.as_ref().is_some_and(|f| f.has_layout(kind));
        let has_equipment = has(crate::layouts::Kind::Equipment);
        let has_inventory = has(crate::layouts::Kind::Inventory);
        let has_prayer = has(crate::layouts::Kind::Prayer);
        let mut tools_tab = run.tools_tab;
        let run_id = run.id;
        let restartable = !running && !waiting;
        let restart_refusal = self.restart_refusal(run);
        let title_font = egui::FontId::new(
            egui::TextStyle::Heading.resolve(ui.style()).size,
            crate::theme::bold_family(),
        );
        ui.add_space(4.0);
        ui.add(egui::Label::new(egui::RichText::new(&run.title).font(title_font)).truncate());
        if let Some(progress) = self.queue_banner(run_id) {
            ui.label(egui::RichText::new(format!("Queue {progress}")).color(crate::theme::BLUE));
        }
        ui.add_space(8.0);
        let mut outcome = crate::form::Outcome::Open;
        let mut update = false;
        let mut restart = false;
        let width = icon_button_width(
            ui,
            &[
                (ICON_START, &submit_label),
                (ICON_UPDATE, "Update"),
                PAUSE_BUTTONS[0],
                PAUSE_BUTTONS[1],
                (ICON_STOP, "Stop"),
                (ICON_CANCEL, "Cancel"),
                (ICON_RESTART, "Restart"),
            ],
        );
        let buttons = 3 + usize::from(restartable);
        let stack_buttons = crate::theme::button_group_width(buttons, width) > ui.available_width();
        crate::theme::button_group_in(ui, stack_buttons, |ui| {
            if form_sent {
                update = icon_button(
                    ui,
                    ICON_UPDATE,
                    "Update",
                    width,
                    running && !stopping && changed_form,
                )
                .clicked();
            } else if icon_button(ui, ICON_START, &submit_label, width, can_start).clicked() {
                outcome = self.run().and_then(|r| r.form.as_ref()).map_or_else(
                    || crate::form::Outcome::Submitted(serde_json::json!({})),
                    |form| form.submitted(self.script_layouts(), &self.known_items),
                );
            }
            let (pause_icon, pause_text) = run_pause_label(paused);
            let pause = icon_button(
                ui,
                pause_icon,
                pause_text,
                width,
                running && !stopping && !waiting,
            );
            let pause = if is_store {
                pause.on_hover_text("Still billed while paused: stop the script to stop billing.")
            } else {
                pause
            };
            if pause.clicked()
                && let Some(id) = self.active_run
            {
                self.toggle_pause_of(id);
            }
            let (stop_icon, stop_text) = if waiting {
                (ICON_CANCEL, "Cancel")
            } else {
                (ICON_STOP, "Stop")
            };
            if icon_button(
                ui,
                stop_icon,
                stop_text,
                width,
                waiting || (running && !stopping),
            )
            .clicked()
            {
                if waiting {
                    outcome = crate::form::Outcome::Cancelled;
                } else if let Some(run) = self.run_mut() {
                    run.stop();
                }
            }
            if restartable {
                let button = icon_button(
                    ui,
                    ICON_RESTART,
                    "Restart",
                    width,
                    restart_refusal.is_none(),
                );
                restart = match restart_refusal.as_deref() {
                    Some(why) => button.on_disabled_hover_text(why),
                    None => button,
                }
                .clicked();
            }
        });
        let state = self.run().map_or(RunState::Ended, Run::state);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let color = state.color();
            let color = egui::Color32::from_rgb(
                (color.r * 255.0) as u8,
                (color.g * 255.0) as u8,
                (color.b * 255.0) as u8,
            );
            crate::theme::status_dot(ui, color);
            ui.label(egui::RichText::new(state.label()).color(color));
        });
        self.answer_form(run_id, outcome);
        if update {
            self.push_form_update();
        }
        if restart {
            let ctx = ui.ctx().clone();
            self.restart_run(run_id);
            return;
        }
        ui.add_space(16.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        if (tools_tab == ToolsTab::Equipment && !has_equipment)
            || (tools_tab == ToolsTab::Inventory && !has_inventory)
            || (tools_tab == ToolsTab::Prayer && !has_prayer)
            || (tools_tab == ToolsTab::Feedback && !is_store)
        {
            tools_tab = ToolsTab::Settings;
        }
        ui.horizontal_wrapped(|ui| {
            crate::theme::tab_value(ui, &mut tools_tab, ToolsTab::Settings, "Settings");
            if has_equipment {
                crate::theme::tab_value(ui, &mut tools_tab, ToolsTab::Equipment, "Equipment");
            }
            if has_inventory {
                crate::theme::tab_value(ui, &mut tools_tab, ToolsTab::Inventory, "Inventory");
            }
            if has_prayer {
                crate::theme::tab_value(ui, &mut tools_tab, ToolsTab::Prayer, "Prayer");
            }
            crate::theme::tab_value(ui, &mut tools_tab, ToolsTab::Xp, "Stats");
            crate::theme::tab_value(ui, &mut tools_tab, ToolsTab::Breaks, "Breaks");
            if is_store {
                crate::theme::tab_value(ui, &mut tools_tab, ToolsTab::Feedback, "Feedback");
            }
        });
        if let Some(run) = self.run_mut() {
            run.tools_tab = tools_tab;
        }
        egui_divider(ui);
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.add_space(8.0);
        let can_read = self.layout_target().is_some();
        let form_title = self
            .run()
            .and_then(|r| r.form.as_ref())
            .map(|form| form.title.clone())
            .unwrap_or_default();
        let mut read: Option<(crate::layouts::Kind, bool)> = None;
        let unknown: Vec<i32> = self
            .run()
            .and_then(|r| r.form.as_ref())
            .map(|form| {
                [
                    crate::layouts::Kind::Equipment,
                    crate::layouts::Kind::Inventory,
                    crate::layouts::Kind::Prayer,
                ]
                .into_iter()
                .flat_map(|kind| form.rules(kind).unknown(&self.known_items))
                .filter(|id| !self.asking_items.contains(id))
                .collect()
            })
            .unwrap_or_default();
        egui::ScrollArea::both()
            .min_scrolled_height(0.0)
            .id_salt(("tools_tab", tools_tab as u8))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                crate::theme::tab_content(ui, |ui| match tools_tab {
                    ToolsTab::Settings => match run_in(&mut self.runs, self.active_run)
                        .and_then(|r| r.form.as_mut())
                    {
                        Some(form) => {
                            let layouts = self.layouts.get(&form.title).unwrap_or(&EMPTY_LAYOUTS);
                            form.show_inline(ui, layouts);
                        }
                        None => {
                            ui.label(
                                egui::RichText::new("The script isn't asking for settings.").weak(),
                            );
                        }
                    },
                    ToolsTab::Equipment => {
                        let saved = self.layouts.entry(form_title).or_default();
                        if let Some(Run { editors, form, .. }) =
                            run_in(&mut self.runs, self.active_run)
                            && crate::layouts::equipment_ui(
                                ui,
                                &mut editors.equipment,
                                &mut saved.equipment,
                                &mut self.icons,
                                can_read,
                                rules_in(form, crate::layouts::Kind::Equipment),
                                &self.known_items,
                            ) == Some(crate::layouts::Action::Read)
                        {
                            read = Some((crate::layouts::Kind::Equipment, false));
                        }
                    }
                    ToolsTab::Inventory => {
                        let saved = self.layouts.entry(form_title).or_default();
                        if let Some(Run { editors, form, .. }) =
                            run_in(&mut self.runs, self.active_run)
                            && crate::layouts::inventory_ui(
                                ui,
                                &mut editors.inventory,
                                &mut saved.inventory,
                                &mut self.icons,
                                can_read,
                                rules_in(form, crate::layouts::Kind::Inventory),
                                &self.known_items,
                            ) == Some(crate::layouts::Action::Read)
                        {
                            read = Some((crate::layouts::Kind::Inventory, false));
                        }
                    }
                    ToolsTab::Prayer => {
                        let level = self
                            .run()
                            .and_then(|r| r.xp.rows().into_iter().find(|row| row.name == "Prayer"))
                            .and_then(|row| u32::try_from(row.level).ok());
                        let saved = self.layouts.entry(form_title).or_default();
                        let action = run_in(&mut self.runs, self.active_run).and_then(
                            |Run { editors, form, .. }| {
                                crate::layouts::prayer_ui(
                                    ui,
                                    &mut editors.prayer,
                                    &mut saved.prayer,
                                    &mut self.icons,
                                    can_read,
                                    level,
                                    rules_in(form, crate::layouts::Kind::Prayer),
                                )
                            },
                        );
                        match action {
                            Some(crate::layouts::Action::Read) => {
                                read = Some((crate::layouts::Kind::Prayer, false))
                            }
                            Some(crate::layouts::Action::ReadQuick) => {
                                read = Some((crate::layouts::Kind::Prayer, true))
                            }
                            None => {}
                        }
                    }
                    ToolsTab::Xp => self.xp_tracker_ui(ui),
                    ToolsTab::Breaks => self.breaks_ui(ui),
                    ToolsTab::Feedback => {}
                })
            });
        if let Some((kind, quick)) = read {
            self.read_layout(kind, quick);
        }
        if !unknown.is_empty() {
            self.describe_items(unknown);
        }
    }

    pub(super) fn xp_tracker_ui(&self, ui: &mut egui::Ui) {
        use crate::{
            counters::number,
            xp::{duration, thousands},
        };
        let Some(run) = self
            .run()
            .filter(|r| !(r.xp.is_empty() && r.counters.is_empty()))
        else {
            ui.weak("Waiting for the first read of the client's skills.");
            return;
        };
        let elapsed = run.run_time();
        ui.label(format!(
            "{} · {} xp",
            duration(elapsed),
            thousands(run.xp.total())
        ));
        let rows = run.xp.rows();
        ui.vertical(|ui| {
            if rows.is_empty() {
                ui.weak("No experience gained yet.");
            } else {
                egui::Grid::new("xp_grid")
                    .num_columns(5)
                    .spacing([12.0, 2.0])
                    .show(ui, |ui| {
                        for head in ["Skill", "Gained", "XP/hr", "Level", "To level"] {
                            ui.weak(head);
                        }
                        ui.end_row();
                        for row in &rows {
                            ui.label(&row.name);
                            ui.label(thousands(row.gained));
                            ui.label(row.per_hour.map_or_else(|| "-".into(), thousands));
                            ui.label(row.level.to_string());
                            ui.label(row.to_level.map_or_else(|| "-".into(), duration));
                            ui.end_row();
                        }
                    });
            }
            let counters = run.counters.rows();
            if !counters.is_empty() {
                ui.add_space(6.0);
                ui.separator();
                egui::Grid::new("counters_grid")
                    .num_columns(4)
                    .spacing([12.0, 2.0])
                    .show(ui, |ui| {
                        for head in ["", "Total", "Per hour", ""] {
                            ui.weak(head);
                        }
                        ui.end_row();
                        for row in &counters {
                            ui.label(&row.label);
                            ui.label(number(row.total));
                            ui.label(row.per_hour.map_or_else(|| "-".into(), number));
                            ui.label(row.ratio.as_deref().unwrap_or(""));
                            ui.end_row();
                        }
                    });
            }
        });
    }

    pub(super) fn breaks_ui(&mut self, ui: &mut egui::Ui) {
        use crate::{breaks::clock, xp::duration};
        let running = self.run().is_some_and(Run::is_running);
        let mut skip: Option<(&'static str, u32)> = None;
        let mut update = false;
        ui.vertical(|ui| {
            crate::breaks::header(ui);
            ui.add_space(4.0);
            let spacing = ui.spacing().item_spacing.y;
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.horizontal(|ui| {
                crate::theme::tab_value(ui, &mut self.breaks_tab, BreaksTab::Breaks, "Breaks");
                crate::theme::tab_value(ui, &mut self.breaks_tab, BreaksTab::Sleeps, "Sleeps");
            });
            let sleeps = self.breaks_tab == BreaksTab::Sleeps;
            crate::theme::tab_panel(ui, |ui| {
                ui.spacing_mut().item_spacing.y = spacing;
                if sleeps {
                    crate::breaks::sleeps_editor(ui, &mut self.settings.antiban);
                } else {
                    crate::breaks::breaks_editor(ui, &mut self.settings.antiban);
                }
            });
            ui.spacing_mut().item_spacing.y = spacing;
            ui.add_space(6.0);
            let valid = self.settings.antiban.validate();
            if let Err(why) = &valid {
                ui.colored_label(egui::Color32::LIGHT_RED, why);
            }
            if running {
                let dirty = self
                    .run()
                    .is_some_and(|r| r.antiban_sent != self.settings.antiban);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    update = ui
                        .add_enabled(dirty && valid.is_ok(), egui::Button::new("Update"))
                        .on_hover_text("Send these to the running script")
                        .clicked();
                    if dirty {
                        ui.weak("Not sent to the script yet");
                    }
                });
                ui.add_space(6.0);
            } else {
                ui.weak("Used by the scripts you start. It is saved as you edit.");
                return;
            }
            let Some(view) = self.run().and_then(|r| r.breaks.view(Instant::now())) else {
                ui.add_space(6.0);
                ui.weak("The script hasn't reported any breaks yet.");
                return;
            };
            ui.ctx().request_repaint_after(Duration::from_secs(1));
            ui.separator();
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Queued").strong());
            ui.add_space(4.0);
            if let Some(active) = &view.active {
                ui.label(
                    egui::RichText::new(format!(
                        "On a {} · {} left",
                        active.kind,
                        duration(active.remaining)
                    ))
                    .strong(),
                );
            }
            if !sleeps && !view.breaks.is_empty() {
                ui.add_space(4.0);
                egui::Grid::new("breaks_grid")
                    .num_columns(3)
                    .spacing([12.0, 2.0])
                    .show(ui, |ui| {
                        for row in &view.breaks {
                            ui.label(format!("in {}", duration(row.remaining)));
                            ui.label(format!("~{}", duration(row.length)));
                            if ui.small_button("Skip next").clicked() {
                                skip = Some(("break", row.index));
                            }
                            ui.end_row();
                        }
                    });
            }
            if sleeps && !view.sleeps.is_empty() {
                ui.add_space(4.0);
                egui::Grid::new("sleeps_grid")
                    .num_columns(4)
                    .spacing([12.0, 2.0])
                    .show(ui, |ui| {
                        for row in &view.sleeps {
                            ui.label(row.at.map_or_else(|| "-".into(), clock));
                            ui.label(format!("in {}", duration(row.remaining)));
                            ui.label(format!("~{}", duration(row.length)));
                            if ui.small_button("Skip next").clicked() {
                                skip = Some(("sleep", row.index));
                            }
                            ui.end_row();
                        }
                    });
            }
            if (sleeps && view.sleeps.is_empty()) || (!sleeps && view.breaks.is_empty()) {
                ui.weak("Nothing is scheduled.");
            }
        });
        if update {
            let config = self.settings.antiban.clone();
            if let Some(run) = self.run_mut() {
                match run.send(|s| s.push_antiban(&config)) {
                    Ok(()) => {
                        run.antiban_sent = config;
                        run.info("Breaks updated.");
                    }
                    Err(error) => run.error(error),
                }
            }
        }
        if let Some((kind, index)) = skip
            && let Some(run) = self.run_mut()
        {
            let result = run.send(|s| s.skip_next(kind, index));
            run.report(result, &format!("Skipping the next {kind}."));
        }
    }
}

impl LoaderApp {
    pub(super) fn render_run(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        not_ported("Run")
    }
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    Vec::new()
}
