use super::*;

impl LoaderApp {
    pub(super) fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let field = (ui.available_width() - 190.0).clamp(120.0, 760.0);
        let mut log_lines_changed = false;
        egui::ScrollArea::both().id_salt("settings_page").auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("settings").num_columns(3).max_col_width(field).show(ui, |ui| {
                ui.label("agent DLL");
                ui.add(egui::TextEdit::singleline(&mut self.settings.dll).desired_width(field));
                if ui.button("Browse…").clicked() {
                    pick_file(&mut self.settings.dll, "DLL", "dll");
                }
                ui.end_row();

                ui.label("lib folder").on_hover_text("Optional. Scripts you run from a file read require(\"../lib/...\") from this folder, e.g. your lib checkout. Store scripts always use the downloaded lib.");
                ui.add(egui::TextEdit::singleline(&mut self.settings.lib_dir).desired_width(field).hint_text("empty: the lib beside the script, or the built-in one"));
                if ui.button("Browse…").clicked() {
                    pick_folder(&mut self.settings.lib_dir);
                }
                ui.end_row();

                if !cfg!(windows) {
                    ui.label("injector.exe");
                    ui.add(egui::TextEdit::singleline(&mut self.settings.helper).desired_width(field));
                    if ui.button("Browse…").clicked() {
                        pick_file(&mut self.settings.helper, "Executable", "exe");
                    }
                    ui.end_row();
                }

                ui.label("output lines").on_hover_text("How many lines each output tab keeps. A tab lays out all of its lines again whenever one is added, so more lines cost more CPU and memory.");
                log_lines_changed = ui.add(egui::DragValue::new(&mut self.settings.log_lines).range(backend::LOG_LINES).speed(50.0)).changed();
                ui.end_row();

                ui.label("interface font").on_hover_text("The font of the pages, buttons and labels. The output, the editor, the terminal and the inspectors stay monospace.");
                let before = self.ui_font;
                egui::ComboBox::from_id_salt("ui_font")
                    .selected_text(self.ui_font.label())
                    .show_ui(ui, |ui| {
                        for font in crate::theme::UiFont::ALL {
                            ui.selectable_value(&mut self.ui_font, font, font.label());
                        }
                    });
                if self.ui_font != before {
                    crate::theme::set_ui_font(ui.ctx(), self.ui_font);
                }
                ui.end_row();
            });
            self.whitelist_page
                .ui(ui, &mut crate::script::write(&self.http_whitelist));
            self.transport_settings_ui(ui);
        });
        if log_lines_changed {
            let keep = self.settings.log_lines;
            self.loader_log.set_keep(keep);
            for run in &mut self.runs {
                run.log.set_keep(keep);
            }
        }
    }

    pub(super) fn transport_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.strong("Transports");
        ui.weak("What scripts may use to get around. A script that needs one that is off stops with an error saying so.");
        let mut changed = false;
        egui::Grid::new("transport_types")
            .num_columns(2)
            .show(ui, |ui| {
                for (i, (name, label)) in injector::transports::TYPES.iter().enumerate() {
                    let off = &mut self.settings.transports.off;
                    let mut on = !off.iter().any(|o| o == name);
                    if ui.checkbox(&mut on, *label).changed() {
                        if on {
                            off.retain(|o| o != name);
                        } else {
                            off.push((*name).to_owned());
                        }
                        changed = true;
                    }
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
        ui.add_space(6.0);
        let data = crate::transports::status();
        ui.label(format!("Data: {}", data.version));
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!data.busy, egui::Button::new("Check for update"))
                .on_hover_text(
                    "Asks for the newest data now. The loader also does this once a day.",
                )
                .clicked()
            {
                crate::transports::update_now();
            }
            if ui
                .add_enabled(
                    data.can_roll_back && !data.busy,
                    egui::Button::new("Roll back"),
                )
                .on_hover_text(
                    "Goes back to the version before this one, if this one causes trouble.",
                )
                .clicked()
            {
                crate::transports::roll_back_now();
            }
            if data.busy {
                ui.spinner();
                ui.ctx().request_repaint_after(Duration::from_millis(250));
            }
        });
        if !data.message.is_empty() {
            ui.weak(&data.message);
        }
        if changed {
            let settings = self.settings.transports.clone();
            for run in self.runs.iter_mut().filter(|r| r.is_running()) {
                if let Err(error) = run.send(|s| s.push_transport_settings(&settings)) {
                    run.error(error);
                }
            }
        }
    }
}

impl LoaderApp {
    pub(super) fn render_settings(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        not_ported("Settings")
    }
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("settings", |app| app.tab = Tab::Settings),
    ]
}
