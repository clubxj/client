use super::*;

pub(super) fn client_cell<R>(
    ui: &mut egui::Ui,
    width: f32,
    top: f32,
    spacing: egui::Vec2,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    ui.allocate_ui_with_layout(
        egui::vec2(width, 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing = spacing;
            ui.add_space(top);
            add(ui)
        },
    )
}

impl LoaderApp {
    pub(super) fn clients_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();

        ui.horizontal(|ui| {
            ui.heading("Clients");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(self.busy.is_none(), egui::Button::new("Refresh"))
                    .clicked()
                {
                    self.scan();
                }
            });
        });
        ui.add_space(4.0);

        if self.targets.is_empty() {
            if self.busy.is_none() {
                ui.label(
                    egui::RichText::new("No game clients found.").color(crate::theme::COMMENT),
                );
            }
        } else {
            self.step_selected_client(&ctx);
            let show_prefix = !cfg!(windows);
            let gap = 16.0;
            let spacing = ui.spacing().item_spacing;
            let body = egui::TextStyle::Body.resolve(ui.style()).size;
            let bold = egui::FontId::new(body, crate::theme::bold_family());
            let measure = |ui: &egui::Ui, text: &str| crate::theme::text_width(ui, &bold, text);
            let pid_width = self
                .targets
                .iter()
                .map(|t| measure(ui, &t.client.pid.to_string()))
                .fold(measure(ui, "PID"), f32::max);
            let title_width = self
                .targets
                .iter()
                .map(|t| measure(ui, &t.client.title))
                .fold(measure(ui, "Title"), f32::max);
            let mut flex = vec![("Title", title_width)];
            if show_prefix {
                flex.push(("Prefix", CLIENT_PREFIX_WIDTH));
            }
            flex.push(("Status", CLIENT_STATUS_WIDTH));
            let columns = flex.len() + 1;
            let natural: f32 = flex.iter().map(|(_, w)| w).sum();
            let other = pid_width + gap * (columns - 1) as f32 + 16.0;
            let room = ui.available_width() - other;
            let narrow = room < natural;
            if narrow {
                let scale = (room / natural).max(0.0);
                for (_, width) in &mut flex {
                    *width = (*width * scale).max(48.0);
                }
            }
            let widths: Vec<f32> = flex.iter().map(|(_, w)| *w).collect();
            egui::ScrollArea::both()
                .auto_shrink([false, true])
                .max_height((ui.available_height() - 120.0).max(60.0))
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        let header = |ui: &mut egui::Ui, width: f32, text: &str| {
                            client_cell(ui, width, 0.0, spacing, |ui| {
                                ui.label(egui::RichText::new(text).color(crate::theme::COMMENT));
                            });
                        };
                        header(ui, pid_width, "PID");
                        for (i, (title, _)) in flex.iter().enumerate() {
                            header(ui, widths[i], title);
                        }
                    });
                    ui.add_space(spacing.y);

                    for (index, target) in self.targets.iter().enumerate() {
                        let client = &target.client;
                        let enabled = client.foreign_bot.is_none();
                        let is_selected = self.selected.as_ref() == Some(&target.key());
                        let active = is_selected && enabled;
                        let background = ui.painter().add(egui::Shape::Noop);
                        let text_color = if active {
                            crate::theme::GREEN
                        } else if !enabled {
                            crate::theme::COMMENT
                        } else {
                            ui.visuals().text_color()
                        };
                        let styled = |text: &str| {
                            let text = egui::RichText::new(text).color(text_color);
                            if active {
                                text.font(bold.clone())
                            } else {
                                text
                            }
                        };
                        let (status, color, tip) = if client.foreign_bot.is_some() {
                            (
                                "Unsupported".to_string(),
                                crate::theme::RED,
                                "Being used by another platform".to_string(),
                            )
                        } else if let Some(module) = &client.loaded {
                            let version = client.version.as_deref().unwrap_or("unknown version");
                            let (text, color) =
                                if self.dll_version.as_deref().is_some_and(|v| v != version) {
                                    (format!("Loaded {version}, outdated"), crate::theme::YELLOW)
                                } else {
                                    (format!("Loaded {version}"), crate::theme::GREEN)
                                };
                            (text.clone(), color, format!("{text}\n{module}"))
                        } else if !client.is_64bit {
                            let text = "32-bit, unsupported".to_string();
                            (text.clone(), crate::theme::RED, text)
                        } else {
                            let text = "Not loaded".to_string();
                            (text.clone(), crate::theme::COMMENT, text)
                        };
                        let bold_status = active || !enabled;
                        let prefix = target.location.label().unwrap_or_default();

                        let painter = ui.painter().clone();
                        let normal = egui::TextStyle::Body.resolve(ui.style());
                        let font = if active { bold.clone() } else { normal.clone() };
                        let title_galley = painter.layout(
                            client.title.clone(),
                            font.clone(),
                            text_color,
                            widths[0],
                        );
                        let status_index = flex.len() - 1;
                        let status_font = if bold_status {
                            bold.clone()
                        } else {
                            normal.clone()
                        };
                        let status_galley = painter.layout(
                            status.clone(),
                            status_font,
                            color,
                            widths[status_index],
                        );
                        let pid_galley = painter.layout_no_wrap(
                            client.pid.to_string(),
                            font.clone(),
                            text_color,
                        );
                        let prefix_galley = show_prefix.then(|| {
                            let lines =
                                title_galley.rows.len().max(status_galley.rows.len()).max(1);
                            let mut job = egui::text::LayoutJob::simple(
                                prefix.clone(),
                                font.clone(),
                                text_color,
                                widths[1],
                            );
                            job.wrap.max_rows = lines;
                            job.wrap.break_anywhere = true;
                            painter.layout_job(job)
                        });
                        let row_height = [
                            pid_galley.size().y,
                            title_galley.size().y,
                            status_galley.size().y,
                            prefix_galley.as_ref().map_or(0.0, |g| g.size().y),
                        ]
                        .into_iter()
                        .fold(0.0, f32::max);
                        let pad = |height: f32| (row_height - height) / 2.0;

                        let row = ui
                            .horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing.x = gap;
                                client_cell(
                                    ui,
                                    pid_width,
                                    pad(pid_galley.size().y),
                                    spacing,
                                    |ui| {
                                        ui.add(
                                            egui::Label::new(styled(&client.pid.to_string()))
                                                .selectable(false),
                                        );
                                    },
                                );
                                client_cell(
                                    ui,
                                    widths[0],
                                    pad(title_galley.size().y),
                                    spacing,
                                    |ui| {
                                        ui.add(
                                            egui::Label::new(styled(&client.title))
                                                .selectable(false),
                                        )
                                        .on_hover_text(&client.exe);
                                    },
                                );
                                if let Some(galley) = prefix_galley {
                                    client_cell(
                                        ui,
                                        widths[1],
                                        pad(galley.size().y),
                                        spacing,
                                        |ui| {
                                            let (rect, response) = ui.allocate_exact_size(
                                                egui::vec2(widths[1], galley.size().y),
                                                egui::Sense::hover(),
                                            );
                                            ui.painter().galley(rect.min, galley, text_color);
                                            response.on_hover_text(&prefix);
                                        },
                                    );
                                }
                                client_cell(
                                    ui,
                                    widths[status_index],
                                    pad(status_galley.size().y),
                                    spacing,
                                    |ui| {
                                        let text = egui::RichText::new(&status).color(color);
                                        let text = if bold_status {
                                            text.font(bold.clone())
                                        } else {
                                            text
                                        };
                                        ui.add(egui::Label::new(text).selectable(false))
                                            .on_hover_text(&tip);
                                    },
                                );
                            })
                            .response
                            .rect
                            .expand2(egui::vec2(8.0, 2.0));
                        ui.add_space(spacing.y);
                        let hovered = enabled && ui.rect_contains_pointer(row);
                        if hovered {
                            ui.painter().set(
                                background,
                                egui::Shape::rect_filled(
                                    row,
                                    3.0,
                                    ui.visuals().widgets.hovered.weak_bg_fill,
                                ),
                            );
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            if ui.input(|i| i.pointer.primary_clicked()) {
                                self.selected = Some(target.key());
                                self.run_target.clone_from(&self.selected);
                            }
                        } else if index % 2 == 1 {
                            ui.painter().set(
                                background,
                                egui::Shape::rect_filled(row, 3.0, ui.visuals().faint_bg_color),
                            );
                        }
                    }
                });
        }

        let loaded = self
            .selected_target()
            .is_some_and(|t| t.client.loaded.is_some());
        let can_unload = self.busy.is_none() && loaded;

        ui.add_space(8.0);
        ui.heading("Client options");
        ui.add_space(4.0);
        self.client_buttons_ui(ui);
        ui.add_space(6.0);
        let mut changed = false;
        let mut set_name = false;
        ui.vertical(|ui| {
            egui::Grid::new("client_options").num_columns(2).spacing(egui::vec2(16.0, 6.0)).show(ui, |ui| {
                ui.label("Mouse cursor");
                ui.horizontal_wrapped(|ui| {
                    egui::ComboBox::from_id_salt("cursor_style")
                        .selected_text(cursor_style_label(self.cursor.style))
                        .show_ui(ui, |ui| {
                            for style in CURSOR_STYLES {
                                changed |= ui.selectable_value(&mut self.cursor.style, style, cursor_style_label(style)).changed();
                            }
                        });
                    ui.add_enabled_ui(self.cursor.style != CursorStyle::Off, |ui| {
                        changed |= ui.color_edit_button_srgb(&mut self.cursor.color).changed();
                        changed |= ui.add(egui::DragValue::new(&mut self.cursor.size).range(4..=64).suffix(" px")).changed();
                    });
                });
                ui.end_row();

                ui.label("Display name");
                ui.horizontal(|ui| {
                    let hint = "Shown instead of your display name, in this client only, and kept across logins until reset. Empty puts the real name back.";
                    let field = ui
                        .add_enabled(
                            can_unload,
                            egui::TextEdit::singleline(&mut self.display_name)
                                .hint_text("Real name")
                                .char_limit(injector::state::MAX_NAME_BYTES)
                                .desired_width(140.0),
                        )
                        .on_hover_text(hint);
                    let submitted = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let label = if self.display_name.trim().is_empty() { "Reset" } else { "Set" };
                    set_name = ui.add_enabled(can_unload, egui::Button::new(label)).on_hover_text(hint).clicked() || submitted;
                });
                ui.end_row();
            });
        });
        if (set_name || changed)
            && let Some(target) = self.selected_target().cloned()
        {
            if set_name {
                self.set_display_name(target.clone());
            }
            if changed {
                self.set_cursor(target);
            }
        }
    }

    pub(super) fn client_buttons_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let target = self
            .selected_target()
            .filter(|t| t.client.foreign_bot.is_none())
            .cloned();
        let client = target.as_ref().map(|t| &t.client);
        let idle = self.busy.is_none();
        let loaded = client.is_some_and(|c| c.loaded.is_some());
        let can_load = idle
            && !self.settings.dll.is_empty()
            && client.is_some_and(|c| {
                c.is_64bit && (c.loaded.is_none() || c.version != self.dll_version)
            });
        let can_unload = idle && loaded;
        let (load_icon, load_label) = if loaded {
            (ICON_UPDATE, "Reload")
        } else {
            (ICON_LOAD, "Load")
        };
        let mut action = None;
        crate::theme::button_group(ui, |ui| {
            let width = icon_button_width(
                ui,
                &[
                    (ICON_LOAD, "Load"),
                    (ICON_UPDATE, "Reload"),
                    (ICON_UNLOAD, "Unload"),
                    (ICON_CONSOLE, "Console"),
                ],
            );
            let load = icon_button(ui, load_icon, load_label, width, can_load)
                .on_hover_text("Loads the DLL from the settings, unloading another build first.");
            if load.clicked() {
                action = Some(ClientAction::Load);
            }
            if icon_button(ui, ICON_UNLOAD, "Unload", width, can_unload).clicked() {
                action = Some(ClientAction::Unload);
            }
            if icon_button(ui, ICON_CONSOLE, "Console", width, can_unload)
                .on_hover_text("Opens agent's console window in the client.")
                .clicked()
            {
                action = Some(ClientAction::Console);
            }
        });
        match (action, target) {
            (Some(ClientAction::Load), Some(target)) => self.load(target),
            (Some(ClientAction::Unload), Some(target)) => self.unload(target),
            (Some(ClientAction::Console), Some(target)) => self.console(target),
            _ => {}
        }
    }

    pub(super) fn step_selected_client(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (up, down) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            )
        });
        if !up && !down {
            return;
        }
        let keys: Vec<String> = self
            .targets
            .iter()
            .filter(|t| t.client.foreign_bot.is_none())
            .map(Target::key)
            .collect();
        if keys.is_empty() {
            return;
        }
        let current = keys.iter().position(|k| Some(k) == self.selected.as_ref());
        let next = match (current, down) {
            (None, true) => 0,
            (None, false) => keys.len() - 1,
            (Some(i), true) => (i + 1).min(keys.len() - 1),
            (Some(i), false) => i.saturating_sub(1),
        };
        self.selected = Some(keys[next].clone());
        self.run_target.clone_from(&self.selected);
    }
}

impl LoaderApp {
    pub(super) fn render_clients(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        not_ported("Clients")
    }
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("clients", |app| app.tab = Tab::Clients),
    ]
}
