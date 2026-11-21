use super::*;

pub(super) fn paint_arrow(
    painter: &egui::Painter,
    center: egui::Pos2,
    size: f32,
    folded: bool,
    color: egui::Color32,
) {
    let points = if folded {
        vec![
            center + egui::vec2(-size * 0.6, -size),
            center + egui::vec2(-size * 0.6, size),
            center + egui::vec2(size * 0.9, 0.0),
        ]
    } else {
        vec![
            center + egui::vec2(-size, -size * 0.6),
            center + egui::vec2(size, -size * 0.6),
            center + egui::vec2(0.0, size * 0.9),
        ]
    };
    painter.add(egui::Shape::convex_polygon(
        points,
        color,
        egui::Stroke::NONE,
    ));
}

pub(super) fn paint_matches(
    ui: &egui::Ui,
    output: &egui::text_edit::TextEditOutput,
    text: &str,
    matches: &[usize],
    find: &mut Find,
    visible: &[usize],
    on_screen: std::ops::Range<usize>,
) {
    let query_len = find.query.len();
    let origin = output.galley_pos.to_vec2();
    let mut walk = TextWalk::new(text);
    for (i, &at) in matches.iter().enumerate() {
        let (first_char, line) = walk.advance(at);
        let current = i == find.current;
        let shown = visible
            .binary_search(&line)
            .is_ok_and(|row| on_screen.contains(&row));
        if !shown && !(current && find.jump) {
            continue;
        }
        let Some(query_chars) = text.get(at..at + query_len).map(|q| q.chars().count()) else {
            continue;
        };
        let start = output
            .galley
            .pos_from_cursor(egui::text::CCursor::new(first_char));
        let end = output
            .galley
            .pos_from_cursor(egui::text::CCursor::new(first_char + query_chars));
        if (start.top() - end.top()).abs() > 1.0 {
            continue;
        }
        let rect = egui::Rect::from_min_max(start.min, egui::pos2(end.max.x, start.max.y))
            .translate(origin);
        let color = if current {
            egui::Color32::from_rgba_unmultiplied(255, 140, 0, 150)
        } else {
            egui::Color32::from_rgba_unmultiplied(255, 220, 0, 70)
        };
        ui.painter().rect_filled(rect, 2.0, color);
        if current && find.jump {
            ui.scroll_to_rect(rect, Some(egui::Align::Center));
        }
    }
    find.jump = false;
}

impl LoaderApp {
    pub(super) fn file_buttons(&mut self, ui: &mut egui::Ui, running: bool, width: f32) {
        if icon_button(ui, ICON_OPEN, "Open…", width, !running).clicked() {
            self.open_script();
        }
        let last = self.last_script.as_deref();
        let open_last = icon_button(
            ui,
            ICON_OPEN_LAST,
            "Open last",
            width,
            !running && last.is_some(),
        );
        let open_last = match last {
            Some(path) => open_last.on_hover_text(path),
            None => open_last,
        };
        if open_last.clicked()
            && let Some(path) = self.last_script.clone()
        {
            self.open_path(std::path::Path::new(&path));
        }
        if icon_button(
            ui,
            ICON_SAVE,
            "Save",
            width,
            !running && !self.script_file.path.is_empty(),
        )
        .clicked()
        {
            self.save_script();
        }
        if icon_button(ui, ICON_SAVE_AS, "Save as…", width, !running).clicked() {
            self.save_script_as();
        }
    }

    pub(super) fn run_buttons(
        &mut self,
        ui: &mut egui::Ui,
        running: bool,
        paused: bool,
        target: Option<&Target>,
        width: f32,
    ) {
        let can_run = !running && !self.script_file.text.trim().is_empty();
        if icon_button(ui, ICON_START, "Run", width, can_run).clicked() {
            match self.script_file.changed_on_disk() {
                Some(disk) => {
                    self.reload_prompt = Some(ReloadPrompt {
                        disk,
                        restart: None,
                    })
                }
                None => self.run_file(self.script_file.clone(), target, None, None),
            }
        }
        let (pause_icon, pause_text) = run_pause_label(paused);
        if icon_button(ui, pause_icon, pause_text, width, running).clicked()
            && let Some(id) = self.editor_run_mut().map(|r| r.id)
        {
            self.toggle_pause_of(id);
        }
        if icon_button(ui, ICON_STOP, "Stop", width, running).clicked()
            && let Some(run) = self.editor_run_mut()
        {
            run.stop();
        }
    }

    pub(super) fn scripts_ui(&mut self, ui: &mut egui::Ui) {
        let (running, paused) = self
            .editor_run_mut()
            .map_or((false, false), |r| (true, r.is_paused()));

        let (find, replace) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::F),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::R),
            )
        });
        if find || replace {
            self.find.open = true;
            self.find.replace |= replace;
            self.find.focus = true;
        }

        let target = self.egui_run_target_ui(ui);

        let width = icon_button_width(
            ui,
            &[
                (ICON_START, "Run"),
                PAUSE_BUTTONS[0],
                PAUSE_BUTTONS[1],
                (ICON_STOP, "Stop"),
                (ICON_OPEN, "Open…"),
                (ICON_OPEN_LAST, "Open last"),
                (ICON_SAVE, "Save"),
                (ICON_SAVE_AS, "Save as…"),
            ],
        );
        let group_gap = 8.0;
        let available = ui.available_width();
        let run_group = crate::theme::button_group_width(3, width);
        let file_group = crate::theme::button_group_width(4, width);
        let stack_groups =
            run_group + ui.spacing().item_spacing.x + group_gap + file_group > available;
        let stack_buttons = run_group.max(file_group) > available;
        egui_fit_row(ui, !stack_groups, |ui| {
            crate::theme::button_group_in(ui, stack_buttons, |ui| {
                self.run_buttons(ui, running, paused, target.as_ref(), width)
            });
            ui.add_space(group_gap);
            crate::theme::button_group_in(ui, stack_buttons, |ui| {
                self.file_buttons(ui, running, width)
            });
        });
        let status = match (running, paused) {
            (true, true) => "paused",
            (true, false) => "running",
            (false, _) => "not running",
        };
        ui.label(format!("{}, {status}", self.script_file.name()))
            .on_hover_text(&self.script_file.path);
        ui.add_space(4.0);
        let matches = if self.find.open {
            let matches = self.find_ui(ui);
            ui.add_space(4.0);
            matches
        } else {
            Vec::new()
        };

        let size = ui.available_size();
        let margin = egui::Margin::symmetric(4, 2);
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let (row_height, digit_width) =
            ui.fonts_mut(|fonts| (fonts.row_height(&font), fonts.glyph_width(&font, '0')));
        self.fold
            .sync(&self.script_file.path, &self.script_file.text);
        let lines = self.fold.line_count();
        let arrow_width = row_height;
        let gutter_width =
            digit_width * lines.to_string().len() as f32 + 3.0 * margin.leftf() + arrow_width;
        let gutter_color = ui.visuals().weak_text_color();
        if self.find.open
            && self.find.jump
            && let Some(&at) = matches.get(self.find.current)
        {
            self.fold
                .reveal(self.script_file.text[..at].matches('\n').count());
        }
        let view = self.fold.view(&self.script_file.text);
        let text_width = (self.fold.longest_line() as f32 * digit_width + margin.sum().x + 8.0)
            .max(size.x - gutter_width - ui.spacing().item_spacing.x);
        let gutter_height = size
            .y
            .max(view.visible.len() as f32 * row_height + margin.sum().y);
        let highlighter = &mut self.highlighter;
        let hidden = &view.hidden;
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, _wrap_width: f32| {
            let mut job = highlighter.layout(ui, text.as_str(), hidden);
            job.wrap.max_width = f32::INFINITY;
            ui.fonts_mut(|fonts| fonts.layout_job(job))
        };
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    let mut select_line = None;
                    let (gutter, response) = ui.allocate_exact_size(
                        egui::vec2(gutter_width, gutter_height),
                        egui::Sense::click(),
                    );
                    ui.painter()
                        .rect_filled(gutter, 0.0, ui.visuals().faint_bg_color);
                    let row_top =
                        |row: usize| gutter.top() + margin.topf() + row as f32 * row_height;
                    let first = ((ui.clip_rect().top() - gutter.top() - margin.topf()) / row_height)
                        .floor()
                        .max(0.0) as usize;
                    let last = ((ui.clip_rect().bottom() - gutter.top() - margin.topf())
                        / row_height)
                        .ceil()
                        .max(0.0) as usize;
                    let arrow_center = |row: usize| {
                        egui::pos2(
                            gutter.right() - arrow_width / 2.0,
                            row_top(row) + row_height / 2.0,
                        )
                    };
                    let hover_row = response
                        .hover_pos()
                        .map(|pos| ((pos.y - gutter.top() - margin.topf()) / row_height).floor())
                        .filter(|&row| row >= 0.0 && (row as usize) < view.visible.len())
                        .map(|row| row as usize);
                    let hover_fill = ui.visuals().widgets.hovered.bg_fill;
                    if let Some(row) = hover_row {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        let rect = egui::Rect::from_min_size(
                            egui::pos2(gutter.left(), row_top(row)),
                            egui::vec2(gutter_width, row_height),
                        );
                        ui.painter().rect_filled(rect, 0.0, hover_fill);
                    }
                    for row in first..last.min(view.visible.len()) {
                        let line = view.visible[row];
                        let number_color = if hover_row == Some(row) {
                            ui.visuals().text_color()
                        } else {
                            gutter_color
                        };
                        ui.painter().text(
                            egui::pos2(
                                gutter.right() - arrow_width - 2.0 * margin.leftf(),
                                row_top(row),
                            ),
                            egui::Align2::RIGHT_TOP,
                            line + 1,
                            font.clone(),
                            number_color,
                        );
                        if self.fold.is_foldable(line) {
                            paint_arrow(
                                ui.painter(),
                                arrow_center(row),
                                row_height * 0.3,
                                self.fold.is_folded(line),
                                gutter_color,
                            );
                        }
                    }
                    if response.clicked()
                        && let Some(pos) = response.interact_pointer_pos()
                    {
                        let row = ((pos.y - gutter.top() - margin.topf()) / row_height).floor();
                        if row >= 0.0
                            && let Some(&line) = view.visible.get(row as usize)
                        {
                            if pos.x >= gutter.right() - arrow_width - margin.leftf()
                                && self.fold.is_foldable(line)
                            {
                                self.fold.toggle(line);
                            } else {
                                select_line = Some(line);
                            }
                        }
                    }
                    let mut output = egui::TextEdit::multiline(&mut self.script_file.text)
                        .code_editor()
                        .margin(margin)
                        .desired_width(text_width)
                        .min_size(egui::vec2(text_width, size.y))
                        .layouter(&mut layouter)
                        .show(ui);
                    if let Some(row) = hover_row {
                        let rect = egui::Rect::from_min_size(
                            egui::pos2(output.response.rect.left(), row_top(row)),
                            egui::vec2(output.response.rect.width(), row_height),
                        );
                        ui.painter()
                            .rect_filled(rect, 0.0, hover_fill.gamma_multiply(0.4));
                    }
                    if let Some(line) = select_line {
                        let (start, end) = line_char_range(&self.script_file.text, line);
                        let range = egui::text::CCursorRange::two(
                            egui::text::CCursor::new(start),
                            egui::text::CCursor::new(end),
                        );
                        output.state.cursor.set_char_range(Some(range));
                        output.state.clone().store(ui.ctx(), output.response.id);
                        output.response.request_focus();
                    }
                    if let Some(range) = output.state.cursor.char_range() {
                        self.fold
                            .follow_cursor(&self.script_file.text, range.primary.index.0);
                    }
                    let on_screen = first.saturating_sub(1)..last + 1;
                    if self.find.open {
                        paint_matches(
                            ui,
                            &output,
                            &self.script_file.text,
                            &matches,
                            &mut self.find,
                            &view.visible,
                            on_screen.clone(),
                        );
                    }
                    let mut walk = TextWalk::new(&self.script_file.text);
                    for &(line, header_end) in &view.headers {
                        if !view
                            .visible
                            .binary_search(&line)
                            .is_ok_and(|row| on_screen.contains(&row))
                        {
                            continue;
                        }
                        let at = output
                            .galley
                            .pos_from_cursor(egui::text::CCursor::new(walk.advance(header_end).0));
                        let badge = egui::Rect::from_min_size(
                            egui::pos2(
                                output.galley_pos.x + at.max.x + 6.0,
                                output.galley_pos.y + at.min.y + 1.0,
                            ),
                            egui::vec2(2.0 * digit_width + 8.0, row_height - 2.0),
                        );
                        let click = ui.interact(
                            badge,
                            ui.id().with(("fold_badge", line)),
                            egui::Sense::click(),
                        );
                        let fill = if click.hovered() {
                            ui.visuals().widgets.hovered.bg_fill
                        } else {
                            ui.visuals().widgets.inactive.bg_fill
                        };
                        ui.painter().rect_filled(badge, 3.0, fill);
                        ui.painter().text(
                            badge.center(),
                            egui::Align2::CENTER_CENTER,
                            "...",
                            font.clone(),
                            gutter_color,
                        );
                        if click.on_hover_text("Click to unfold").clicked() {
                            self.fold.toggle(line);
                        }
                    }
                });
            });
    }

    pub(super) fn find_ui(&mut self, ui: &mut egui::Ui) -> Vec<usize> {
        let matches = find_matches(
            &self.script_file.text,
            &self.find.query,
            self.find.case_sensitive,
        );
        let count = matches.len();
        self.find.current = self.find.current.min(count.saturating_sub(1));

        let mut step = None;
        let mut replace_current = false;
        let mut replace_all = false;
        let mut query_changed = false;

        ui.horizontal(|ui| {
            let query = ui.add(
                egui::TextEdit::singleline(&mut self.find.query)
                    .hint_text("Find")
                    .desired_width(200.0),
            );
            if std::mem::take(&mut self.find.focus) {
                query.request_focus();
            }
            if query.changed() {
                self.find.current = 0;
                self.find.jump = true;
                query_changed = true;
            }
            if (query.has_focus() || query.lost_focus())
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
            {
                step = Some(ui.input(|i| i.modifiers.shift));
            }
            if ui
                .add_enabled(count > 0, egui::Button::new("Previous"))
                .on_hover_text("Shift+Enter")
                .clicked()
            {
                step = Some(true);
            }
            if ui
                .add_enabled(count > 0, egui::Button::new("Next"))
                .on_hover_text("Enter")
                .clicked()
            {
                step = Some(false);
            }
            let status = if self.find.query.is_empty() {
                String::new()
            } else if count == 0 {
                "No matches".into()
            } else {
                format!("{} of {count}", self.find.current + 1)
            };
            ui.label(status);
            if ui
                .checkbox(&mut self.find.case_sensitive, "Match case")
                .changed()
            {
                self.find.current = 0;
                self.find.jump = true;
                query_changed = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").on_hover_text("Escape").clicked() {
                    self.find.open = false;
                }
            });
        });

        if self.find.replace {
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.find.replacement)
                        .hint_text("Replace with")
                        .desired_width(200.0),
                );
                if ui
                    .add_enabled(count > 0, egui::Button::new("Replace"))
                    .clicked()
                {
                    replace_current = true;
                }
                if ui
                    .add_enabled(count > 0, egui::Button::new("Replace all"))
                    .clicked()
                {
                    replace_all = true;
                }
            });
        }

        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.find.open = false;
        }
        if let Some(back) = step.filter(|_| count > 0) {
            let current = self.find.current;
            self.find.current = if back {
                (current + count - 1) % count
            } else {
                (current + 1) % count
            };
            self.find.jump = true;
        }
        if replace_all {
            let query_len = self.find.query.len();
            let mut out = String::with_capacity(self.script_file.text.len());
            let mut last = 0;
            for &at in &matches {
                out.push_str(&self.script_file.text[last..at]);
                out.push_str(&self.find.replacement);
                last = at + query_len;
            }
            out.push_str(&self.script_file.text[last..]);
            self.script_file.text = out;
            self.find.current = 0;
            find_matches(
                &self.script_file.text,
                &self.find.query,
                self.find.case_sensitive,
            )
        } else if let (true, Some(&at)) = (replace_current, matches.get(self.find.current)) {
            let replacement = self.find.replacement.clone();
            let text = &mut self.script_file.text;
            text.replace_range(at..at + self.find.query.len(), &replacement);
            let after = at + replacement.len();
            let next = find_matches(text, &self.find.query, self.find.case_sensitive);
            self.find.current = next.iter().position(|&start| start >= after).unwrap_or(0);
            self.find.jump = true;
            next
        } else if query_changed {
            find_matches(
                &self.script_file.text,
                &self.find.query,
                self.find.case_sensitive,
            )
        } else {
            matches
        }
    }

    pub(super) fn reload_prompt_ui(&mut self, ctx: &egui::Context) {
        let Some(prompt) = &self.reload_prompt else {
            return;
        };
        let restart = prompt.restart;
        let file = match restart {
            Some(id) => match self.run_by_id(id).and_then(|r| r.file.clone()) {
                Some(file) => file,
                None => {
                    self.reload_prompt = None;
                    return;
                }
            },
            None => self.script_file.clone(),
        };
        let name = file.name();
        let unsaved =
            restart.is_none() && file.disk.as_ref().is_some_and(|known| known != &file.text);
        let mut choice: Option<Option<bool>> = None;
        egui::Window::new("Script changed on disk")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(if restart.is_some() {
                    format!(
                        "{name} changed on disk since it last ran. Reload it before restarting?"
                    )
                } else {
                    format!("{name} changed on disk since it was loaded. Reload it before running?")
                });
                if unsaved {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "Reloading discards your unsaved edits.",
                    );
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Reload and run").clicked() {
                        choice = Some(Some(true));
                    }
                    if ui
                        .button(if restart.is_some() {
                            "Run previous version"
                        } else {
                            "Run editor version"
                        })
                        .clicked()
                    {
                        choice = Some(Some(false));
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(None);
                    }
                });
            });
        let Some(choice) = choice else { return };
        let Some(ReloadPrompt { disk, .. }) = self.reload_prompt.take() else {
            return;
        };
        let Some(reload) = choice else { return };
        let mut file = file;
        file.disk = Some(disk.clone());
        if reload {
            file.text = disk;
        }
        match restart {
            Some(id) => self.rerun_file(id, file),
            None => {
                self.script_file = file.clone();
                let target = self.run_target_client();
                self.run_file(file, target.as_ref(), None, None);
            }
        }
    }
}

impl LoaderApp {
    pub(super) fn render_editor(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        not_ported("Editor")
    }

    pub(super) fn render_reload_prompt(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<AnyElement> {
        None
    }
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("editor", |app| app.tab = Tab::Scripts),
    ]
}
