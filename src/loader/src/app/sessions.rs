use super::*;

pub(super) fn restart_button(ui: &mut egui::Ui, width: f32, store: bool, refusal: Option<&str>) -> bool {
    let button = icon_button(ui, ICON_RESTART, "Restart", width, refusal.is_none()).on_hover_text(
        if store {
            "Closes this session and starts the script again on the same client."
        } else {
            "Runs the file again on the same client, asking first if it changed on disk."
        },
    );
    match refusal {
        Some(why) => button.on_disabled_hover_text(why).clicked(),
        None => button.clicked(),
    }
}

impl LoaderApp {
    pub(super) fn sessions_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Sessions");
        if self.runs.is_empty() {
            ui.label(
                egui::RichText::new(
                    "Nothing is running. Start a script from the Scripts page, or the editor.",
                )
                .weak(),
            );
            return;
        }
        let minutes_per_month = self.auth.me().map_or(43_200, |me| me.minutes_per_month);
        ui.add_space(6.0);
        let mut open = None;
        let mut pause = None;
        let mut stop = None;
        let mut close = None;
        let mut restart = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for run in &self.runs {
                    ui.add_space(8.0);
                    crate::theme::card().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let client = run.target.as_ref().map_or("no client".to_owned(), |t| {
                            format!("client {}", t.client.pid)
                        });
                        let rate = run
                            .store
                            .as_ref()
                            .map_or(0.0, |store| store.rate_per_hour(minutes_per_month));
                        let billed = run
                            .store
                            .as_ref()
                            .filter(|store| rate > 0.0 || store.fee_charged > 0);
                        ui.horizontal_wrapped(|ui| {
                            let state = run.state();
                            let about = format!("{}, {client}", state.label());
                            crate::theme::status_dot(ui, state.color()).on_hover_text(&about);
                            ui.label(egui::RichText::new(&run.title).strong().size(16.0))
                                .on_hover_text(&about);
                            ui.label(
                                egui::RichText::new(crate::xp::duration(run.run_time())).weak(),
                            );
                            if billed.is_none() {
                                ui.label(egui::RichText::new("free").weak().small())
                                    .on_hover_text(if run.store.is_some() {
                                        "This session costs nothing."
                                    } else {
                                        "Run from the editor, not billed."
                                    });
                            }
                        });
                        if let Some(store) = billed {
                            ui.horizontal_wrapped(|ui| {
                                crate::theme::credits(
                                    ui,
                                    egui::RichText::new(crate::api::models::rate_label(rate))
                                        .strong(),
                                    "/h",
                                )
                                .on_hover_text(EXTRA_SESSION_FEE_HINT);
                                if store.fee > 0 {
                                    ui.label(
                                        egui::RichText::new("includes the extra-session fee")
                                            .weak()
                                            .small(),
                                    )
                                    .on_hover_text(EXTRA_SESSION_FEE_HINT);
                                }
                                if store.fee_charged > 0 {
                                    crate::theme::credits(
                                        ui,
                                        egui::RichText::new(crate::api::models::credits_label(
                                            store.fee_charged as f64,
                                        ))
                                        .small(),
                                        " in fees so far",
                                    );
                                }
                            });
                        }
                        ui.add_space(4.0);
                        crate::theme::button_group(ui, |ui| {
                            let width = icon_button_width(
                                ui,
                                &[
                                    (ICON_OPEN_RUN, "Open"),
                                    PAUSE_BUTTONS[0],
                                    PAUSE_BUTTONS[1],
                                    (ICON_STOP, "Stop"),
                                    (ICON_CANCEL, "Close"),
                                    (ICON_RESTART, "Restart"),
                                ],
                            );
                            if icon_button(ui, ICON_OPEN_RUN, "Open", width, true).clicked() {
                                open = Some(run.id);
                            }
                            let running = run.is_running();
                            let stopping = run.stop_requested();
                            let (pause_icon, pause_text) = run_pause_label(run.is_paused());
                            if icon_button(ui, pause_icon, pause_text, width, running && !stopping)
                                .clicked()
                            {
                                pause = Some(run.id);
                            }
                            if icon_button(ui, ICON_STOP, "Stop", width, running && !stopping)
                                .clicked()
                            {
                                stop = Some(run.id);
                            }
                            if icon_button(ui, ICON_CANCEL, "Close", width, !running)
                                .on_hover_text("Takes the ended run and its log off the list.")
                                .clicked()
                            {
                                close = Some(run.id);
                            }
                            if !running
                                && !run.is_configuring()
                                && restart_button(
                                    ui,
                                    width,
                                    run.store.is_some(),
                                    self.restart_refusal(run).as_deref(),
                                )
                            {
                                restart = Some(run.id);
                            }
                        });
                    });
                }
            });
        if let Some(id) = open {
            self.active_run = Some(id);
            self.tab = Tab::Run;
        }
        if let Some(id) = pause {
            self.toggle_pause_of(id);
        }
        if let Some(id) = stop
            && let Some(run) = self.run_by_id_mut(id)
        {
            run.stop();
        }
        if let Some(id) = close {
            self.close_run(id);
        }
        if let Some(id) = restart {
            let ctx = ui.ctx().clone();
            self.restart_run(id);
        }
    }
}

impl LoaderApp {
    pub(super) fn render_sessions(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        not_ported("Sessions")
    }
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("sessions", |app| app.tab = Tab::Sessions),
    ]
}
