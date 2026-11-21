use super::*;

pub(super) fn sidebar_tree(
    ui: &mut egui::Ui,
    tab: Tab,
    active_run: Option<u64>,
    runs: &[(u64, String, bool)],
    site_url: &str,
) -> Option<(Tab, Option<u64>)> {
    ui.spacing_mut().item_spacing.y = 0.0;
    crate::theme::sidebar_header(ui, "explorer");
    if crate::theme::sidebar_root(ui, "\u{f233}", "volcren", !site_url.is_empty()).clicked() {
        open_site(site_url, "");
    }
    let items = [
        (Tab::Clients, "\u{f108}", "client"),
        (Tab::Store, "\u{f489}", "scripts"),
        (Tab::Scripts, "\u{f044}", "editor"),
        (Tab::Sessions, "\u{f04b}", "sessions"),
        (Tab::Queue, "\u{f0cb}", "queue"),
        (Tab::Development, "\u{f188}", "development"),
        (Tab::Terminal, "\u{f120}", "terminal"),
        (Tab::Accounts, "\u{f007}", "accounts"),
        (Tab::Feedback, "\u{f075}", "feedback"),
        (Tab::Settings, "\u{f013}", "settings"),
    ];
    let mut current = if tab == Tab::Run {
        (Tab::Run, active_run)
    } else {
        (tab, None)
    };
    let before = current;
    for (i, (tab, icon, name)) in items.iter().enumerate() {
        crate::theme::sidebar_item(
            ui,
            &mut current,
            (*tab, None),
            icon,
            name,
            i == items.len() - 1,
        );
        if *tab == Tab::Sessions {
            for (j, (id, name, ended)) in runs.iter().enumerate() {
                crate::theme::sidebar_child(
                    ui,
                    &mut current,
                    (Tab::Run, Some(*id)),
                    name,
                    *ended,
                    j == runs.len() - 1,
                );
            }
        }
    }
    (current != before).then_some(current)
}

pub(super) fn centered_row(
    ui: &mut egui::Ui,
    id: &str,
    layout: egui::Layout,
    height: f32,
    add: impl FnOnce(&mut egui::Ui),
) {
    let id = ui.id().with(id);
    let known: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(0.0);
    let available = ui.available_width();
    let offset = ((available - known) / 2.0).max(0.0);
    let spacing = ui.spacing().item_spacing.x;
    let measured = ui
        .allocate_ui_with_layout(egui::vec2(available, height), layout, |ui| {
            ui.add_space(offset);
            add(ui);
            (ui.min_rect().width() - offset - spacing).max(0.0)
        })
        .inner;
    if (measured - known).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(id, measured));
        ui.ctx().request_repaint();
    }
}

impl LoaderApp {
    pub(super) fn sidebar_ui(&mut self, ui: &mut egui::Ui) {
        let rows: Vec<(u64, String, bool)> = self
            .runs
            .iter()
            .map(|r| (r.id, sidebar_name(&r.title), !r.is_running()))
            .collect();
        let picked = egui::ScrollArea::vertical()
            .id_salt("sidebar")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                sidebar_tree(
                    ui,
                    self.tab,
                    self.active_run,
                    &rows,
                    &self.api.config().site_url,
                )
            })
            .inner;
        if let Some((tab, run)) = picked {
            self.tab = tab;
            if run.is_some() {
                self.active_run = run;
            }
            self.sidebar_open = false;
        }
    }

    pub(super) fn updating_ui(&mut self, ui: &mut egui::Ui) {
        let status = match &self.update_state {
            UpdateState::Downloading(progress) => progress_text(progress),
            _ => "Installing...".to_owned(),
        };
        let version = self.update.as_ref().map(|a| a.release.version.clone());
        let mut skipped = false;
        egui::CentralPanel::default_margins().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.35);
                ui.heading("Updating VOLCREN");
                if let Some(version) = version {
                    ui.label(format!("Version {version}"));
                }
                ui.label(status);
                if matches!(self.update_state, UpdateState::Downloading(_)) {
                    ui.add_space(8.0);
                    skipped = ui.button("Skip for now").clicked();
                }
            });
        });
        if skipped {
            if let UpdateState::Downloading(progress) = &self.update_state {
                progress.cancel();
            }
            self.update_state = UpdateState::Offered;
        }
    }

    pub(super) fn update_banner_ui(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let Some(available) = &self.update else {
            return;
        };
        let (release, kind) = (&available.release, available.kind);
        if kind == update::Kind::Dev {
            return;
        }
        let running = self.start_in_progress();
        let failure = match &self.update_state {
            UpdateState::Failed(error) => Some(error.as_str()),
            _ => None,
        };
        let main_button = match (failure, kind) {
            (_, update::Kind::Manual) => "Download",
            (Some(_), _) => "Retry",
            _ => "Update and restart",
        };
        let mut clicked = false;
        let mut dismissed = false;
        egui::Panel::bottom("update").show(ui, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    dismissed = ui
                        .add(egui::Button::new(ICON_CANCEL).frame(false))
                        .on_hover_text("Dismiss")
                        .clicked();
                    let enabled = kind == update::Kind::Manual || !running;
                    let button = ui.add_enabled(enabled, egui::Button::new(main_button));
                    let button = button.on_disabled_hover_text("Stop running scripts to update.");
                    let button = if kind == (update::Kind::Auto { admin: true }) {
                        button.on_hover_text("An administrator has to approve the update.")
                    } else {
                        button
                    };
                    clicked = button.clicked();
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!(
                                    "Version {} is available",
                                    release.version
                                ))
                                .color(crate::theme::BLUE),
                            )
                            .truncate(),
                        )
                        .on_hover_text(format!("You have version {}", update::VERSION));
                        if let Some(error) = failure {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(error).color(ui.visuals().error_fg_color),
                                )
                                .truncate(),
                            )
                            .on_hover_text(error);
                        }
                    });
                });
            });
            ui.add_space(2.0);
        });
        if clicked {
            if kind == update::Kind::Manual {
                if let Some(url) = release.download_url(&self.api) {
                    let _ = webbrowser::open(&url);
                }
            } else {
                let release = release.clone();
                self.start_update(ctx, release);
            }
        }
        if dismissed {
            self.update = None;
            ctx.request_repaint();
        }
    }

    pub(super) fn account_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        match &self.auth {
            Auth::SignedIn(me) => {
                let name = me.username.clone();
                let balance = crate::api::models::balance_label(me.microcredits as f64);
                let minutes_per_month = me.minutes_per_month;
                let rate: f64 = self
                    .store_runs()
                    .map(|s| s.rate_per_hour(minutes_per_month))
                    .sum();
                let sign_out_width = icon_button_width(ui, &[(ICON_SIGN_OUT, "Sign out")]);
                if icon_button(ui, ICON_SIGN_OUT, "Sign out", sign_out_width, true).clicked() {
                    self.sign_out(None);
                    return;
                }
                if rate > 0.0 {
                    crate::theme::credits(ui, egui::RichText::new(crate::api::models::rate_label(rate)).weak(), "/h")
                        .on_hover_text("What the scripts running from this loader cost an hour, prices and extra-session fees together.");
                }
                let site_url = &self.api.config().site_url;
                let balance = egui::RichText::new(balance);
                if site_url.is_empty() {
                    crate::theme::credits(ui, balance, "").on_hover_text("Your balance.");
                } else if crate::theme::credits_link(ui, balance, "")
                    .on_hover_text("Your balance. Click to add funds on the website.")
                    .clicked()
                {
                    open_site(site_url, "/credits");
                }
                ui.add(egui::Label::new(egui::RichText::new(name).strong()).truncate());
            }
            Auth::SigningIn(cancel) => {
                if ui.button("Cancel").clicked() {
                    cancel.store(true, Ordering::Relaxed);
                }
                ui.spinner();
                ui.label("Signing in…");
            }
            Auth::Restoring => {
                ui.spinner();
            }
            Auth::SignedOut => {
                if ui.button("Sign in").clicked() {
                    self.start_sign_in(&ctx);
                }
            }
        }
    }

    pub(super) fn login_gate_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.3).max(16.0));
            ui.label(
                egui::RichText::new("Sign in to continue")
                    .size(20.0)
                    .strong(),
            );
            ui.add_space(8.0);
            match &self.auth {
                Auth::Restoring => {
                    ui.spinner();
                    ui.label(RESTORING_SIGN_IN);
                }
                Auth::SigningIn(cancel) => {
                    ui.spinner();
                    ui.label(WAITING_FOR_BROWSER);
                    ui.add_space(8.0);
                    if ui.button("Cancel").clicked() {
                        cancel.store(true, Ordering::Relaxed);
                    }
                }
                _ => {
                    ui.label("Volcren needs a Discord account. Your browser will open to sign in.");
                    ui.add_space(8.0);
                    if ui
                        .button(egui::RichText::new("Sign in with Discord").size(16.0))
                        .clicked()
                    {
                        self.start_sign_in(&ctx);
                    }
                }
            }
        });
    }
}
