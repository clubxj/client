use super::*;

pub(super) fn scroll_selection_past_edge(ui: &egui::Ui, text: &egui::Response) {
    let Some(pointer) = ui
        .input(|i| i.pointer.interact_pos())
        .filter(|_| text.dragged())
    else {
        return;
    };
    let speed = selection_scroll_speed(
        pointer.y,
        ui.clip_rect().y_range(),
        ui.ctx().viewport_rect().y_range(),
    );
    if speed != 0.0 {
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        ui.scroll_with_delta(egui::vec2(0.0, -speed * dt));
        ui.ctx().request_repaint();
    }
}

pub(super) type LogJobKey = (
    LogTab,
    u64,
    egui::FontId,
    egui::Color32,
    egui::Color32,
    egui::Color32,
);

pub(super) struct LaidOut {
    wrap_width: f32,
    pixels_per_point: f32,
    probe: Arc<egui::Galley>,
    galley: Arc<egui::Galley>,
}

pub(super) struct LogJob {
    key: LogJobKey,
    job: egui::text::LayoutJob,
    entry_of_line: Vec<usize>,
    laid_out: RefCell<Option<LaidOut>>,
}

pub(super) fn line_copy_button(ui: &mut egui::Ui, line: egui::Rect) -> egui::Response {
    let height = (line.height() - 2.0).max(1.0);
    let rect = egui::Rect::from_min_max(
        egui::pos2(line.right() - height * 1.6, line.top() + 1.0),
        egui::pos2(line.right(), line.top() + 1.0 + height),
    );
    let response = ui
        .interact(rect, ui.id().with("copy_line"), egui::Sense::click())
        .on_hover_text("Copy this line")
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let visuals = ui.style().interact(&response);
    let painter = ui.painter();
    painter.rect(
        rect,
        egui::CornerRadius::ZERO,
        visuals.weak_bg_fill,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        ICON_COPY,
        egui::FontId::monospace(height * 0.7),
        visuals.text_color(),
    );
    response
}

pub(super) fn log_collapse_button(ui: &mut egui::Ui, collapsed: bool) -> bool {
    let (icon, hint) = if collapsed {
        (ICON_EXPAND, "Show the output")
    } else {
        (ICON_COLLAPSE, "Hide the output, keeping its last line")
    };
    ui.add(egui::Button::new(icon).frame(false))
        .on_hover_text(hint)
        .clicked()
}

impl LoaderApp {
    pub(super) fn log_controls_ui(&mut self, ui: &mut egui::Ui) {
        if let LogTab::Run(_) = self.log_tab {
            let tab = self.log_tab;
            let log = self.log_of(tab);
            let mut filter = log.filter;
            egui::ComboBox::from_id_salt("log_tag")
                .selected_text(filter.map_or("All", |index| log.tags[index].as_str()))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut filter, None, "All");
                    for (index, tag) in log.tags.iter().enumerate() {
                        ui.selectable_value(&mut filter, Some(index), tag);
                    }
                });
            log.set_filter(filter);
        }
        let tab = self.log_tab;
        let (mut save, mut pipe) = (false, false);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            crate::theme::button_group(ui, |ui| {
                let log = self.log_of(tab);
                let filled = !log.is_empty();
                if slide_button(ui, ICON_CLEAR, "Clear", filled, false).clicked() {
                    log.clear();
                }
                if slide_button(ui, ICON_COPY, "Copy all", filled, false).clicked() {
                    ui.ctx().copy_text(log.text.clone());
                }
                save = slide_button(ui, ICON_SAVE, "Save", filled, false).clicked();
                let piping = log
                    .pipe
                    .as_ref()
                    .map(|pipe| pipe.path().display().to_string());
                let button = slide_button(ui, ICON_PIPE, "Pipe", true, piping.is_some());
                pipe = match piping {
                    Some(path) => button.on_hover_text(format!("Appending to {path}")),
                    None => button,
                }
                .clicked();
            });
        });
        if save {
            let text = self.log_of(tab).text.clone();
            self.save_text("output", "output.txt", text);
        }
        if pipe {
            let toggled = crate::pipe::toggle(&mut self.log_of(tab).pipe, "output.txt", 0);
            self.report_pipe(toggled);
        }
    }

    pub(super) fn log_line_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let tab = self.log_tab;
        let shown_run = self.shown_run_log().map(|run| (run.id, run.title.clone()));
        let title = match (tab, &shown_run) {
            (LogTab::Run(_), Some((_, title))) => title.clone(),
            _ => "Loader".to_owned(),
        };
        let last = self.log_ref(tab).last();
        let (text, tone) = last.map_or(("", Tone::Normal), |line| {
            (line.text.lines().last().unwrap_or_default(), line.tone)
        });
        let color = match tone {
            Tone::Normal => ui.visuals().text_color(),
            Tone::Warning => ui.visuals().warn_fg_color,
            Tone::Error => ui.visuals().error_fg_color,
        };
        let mut expand = false;
        let mut picked = tab;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                expand = log_collapse_button(ui, true);
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let title = egui::RichText::new(title).color(crate::theme::COMMENT);
                    match &shown_run {
                        Some((id, run_title)) => {
                            egui::ComboBox::from_id_salt("log_line_tab")
                                .selected_text(title)
                                .width(0.0)
                                .truncate()
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut picked, LogTab::Loader, "Loader");
                                    ui.selectable_value(&mut picked, LogTab::Run(*id), run_title);
                                });
                        }
                        None => {
                            ui.label(title);
                        }
                    }
                    ui.add(
                        egui::Label::new(egui::RichText::new(text).monospace().color(color))
                            .truncate(),
                    );
                });
            });
        });
        self.log_tab = picked;
        expand
    }

    pub(super) fn log_panel_ui(&mut self, ui: &mut egui::Ui) -> bool {
        ui.shrink_clip_rect(ui.available_rect_before_wrap());
        ui.spacing_mut().item_spacing.y = 0.0;
        let shown_run = self.shown_run_log().map(|run| (run.id, run.title.clone()));
        let mut collapse = false;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                collapse = log_collapse_button(ui, false);
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                    crate::theme::tab_value(ui, &mut self.log_tab, LogTab::Loader, "Loader");
                    if let Some((id, title)) = &shown_run {
                        crate::theme::tab_value(ui, &mut self.log_tab, LogTab::Run(*id), title);
                    }
                });
            });
        });
        egui_divider(ui);
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.horizontal_wrapped(|ui| self.log_controls_ui(ui));
        self.log_ui(ui);
        collapse
    }

    pub(super) fn log_ui(&mut self, ui: &mut egui::Ui) {
        let tab = self.log_tab;
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let normal = ui.visuals().text_color();
        let error = ui.visuals().error_fg_color;
        let warning = ui.visuals().warn_fg_color;
        let key = (
            tab,
            self.log_ref(tab).revision,
            font.clone(),
            normal,
            warning,
            error,
        );
        if self.log_job.as_ref().is_none_or(|cached| cached.key != key) {
            let log = self.log_ref(tab);
            self.log_job = Some(LogJob {
                job: log.layout_job(font.clone(), normal, warning, error),
                entry_of_line: log.entry_of_line(),
                laid_out: RefCell::new(None),
                key,
            });
        }
        let Some(log_job) = self.log_job.as_ref() else {
            return;
        };
        let entry_of_line = &log_job.entry_of_line;
        let log = self.log_ref(tab);
        let text = log.text.as_str();
        let probe = ui.fonts_mut(|fonts| fonts.layout_no_wrap(" ".into(), font, normal));
        let pixels_per_point = ui.ctx().pixels_per_point();
        let mut layouter = |ui: &egui::Ui, _: &dyn egui::TextBuffer, wrap_width: f32| {
            let wrap_width = wrap_width.round();
            let mut laid_out = log_job.laid_out.borrow_mut();
            if let Some(kept) = laid_out.as_ref()
                && kept.wrap_width == wrap_width
                && kept.pixels_per_point == pixels_per_point
                && Arc::ptr_eq(&kept.probe, &probe)
            {
                return kept.galley.clone();
            }
            let mut job = log_job.job.clone();
            job.wrap.max_width = wrap_width;
            let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
            *laid_out = Some(LaidOut {
                wrap_width,
                pixels_per_point,
                probe: probe.clone(),
                galley: galley.clone(),
            });
            galley
        };
        egui::ScrollArea::vertical()
            .min_scrolled_height(0.0)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                let mut read_only = text;
                let output = egui::TextEdit::multiline(&mut read_only)
                    .font(egui::TextStyle::Monospace)
                    .frame(egui::Frame::NONE)
                    .desired_width(ui.available_width())
                    .layouter(&mut layouter)
                    .show(ui);
                scroll_selection_past_edge(ui, &output.response);

                let Some(pointer) = ui
                    .input(|i| i.pointer.hover_pos())
                    .filter(|pos| output.response.rect.contains(*pos))
                else {
                    return;
                };
                let origin = output.galley_pos.to_vec2();
                let mut paragraph = 0;
                let mut hovered = None;
                for row in &output.galley.rows {
                    let rect = row.rect().translate(origin);
                    if rect.y_range().contains(pointer.y) {
                        hovered = Some(paragraph);
                        break;
                    }
                    if row.ends_with_newline {
                        paragraph += 1;
                    }
                }
                let Some(entry) = hovered.and_then(|line| entry_of_line.get(line).copied()) else {
                    return;
                };
                let mut index = 0;
                let mut first = None;
                let mut span = egui::Rect::NOTHING;
                for row in &output.galley.rows {
                    let rect = row.rect().translate(origin);
                    if entry_of_line.get(index) == Some(&entry) {
                        first.get_or_insert(rect);
                        span = span.union(rect);
                    }
                    if row.ends_with_newline {
                        index += 1;
                    }
                }
                let Some(rect) = first else {
                    return;
                };
                let band = egui::Rect::from_min_max(
                    egui::pos2(output.response.rect.left(), span.top()),
                    egui::pos2(output.response.rect.right(), span.bottom()),
                );
                ui.painter()
                    .rect_filled(band, 0.0, ui.visuals().text_color().gamma_multiply(0.08));
                let line_rect = egui::Rect::from_min_max(
                    egui::pos2(output.response.rect.left(), rect.top()),
                    egui::pos2(output.response.rect.right(), rect.bottom()),
                );
                if line_copy_button(ui, line_rect).clicked() {
                    ui.ctx().copy_text(log.lines[log.view[entry]].text.clone());
                }
            });
    }
}

impl LoaderApp {
    pub(super) fn render_output(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex_none()
            .h(px(24.0))
            .border_t_1()
            .border_color(crate::ui::theme::BORDER)
            .child(not_ported("Output"))
            .into_any_element()
    }
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    Vec::new()
}
