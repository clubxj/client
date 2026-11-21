use super::*;

pub(super) fn decision_window(
    ctx: &egui::Context,
    title: &str,
    width: f32,
    confirm: &str,
    body: impl FnOnce(&mut egui::Ui),
) -> Option<bool> {
    let mut decision = None;
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_max_width(width);
            body(ui);
            ui.horizontal(|ui| {
                if ui.button(confirm).clicked() {
                    decision = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    decision = Some(false);
                }
            });
        });
    decision
}

impl LoaderApp {
    pub(super) fn start_prompt_ui(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.start_prompt.take() else {
            return;
        };
        let Some(me) = self.auth.me() else {
            return;
        };
        use crate::api::models::{credits_label, fee_per_hour, per_month, rate_label};
        let fee_hour = fee_per_hour(prompt.fee, me.minutes_per_month);
        let fee_minute = fee_hour / 60.0;
        let price_hour = prompt.price as f64;
        let total_hour = price_hour + fee_hour;
        let total_month = per_month(prompt.price, prompt.fee, me.minutes_per_month);
        let extra = prompt.fee > 0;
        let nth = match prompt.nth {
            2 => "2nd".to_owned(),
            3 => "3rd".to_owned(),
            n => format!("{n}th"),
        };
        let heading = if prompt.start.replaces.is_some() {
            "Restart session"
        } else {
            "Extra session"
        };
        let decision = decision_window(ctx, heading, 440.0, "Confirm", |ui| {
            ui.label(egui::RichText::new(&prompt.start.script.title).strong());
            ui.add_space(6.0);
            if prompt.old_price.is_some() {
                ui.label("The price of this script changed since the session started.");
            }
            if extra {
                ui.label(format!(
                    "This would be your {nth} script running at once. Every script past the first adds a fee on top of its own price:"
                ));
            }
            ui.add_space(4.0);
            egui::Grid::new("start_cost")
                .num_columns(2)
                .spacing(egui::vec2(16.0, 4.0))
                .show(ui, |ui| {
                    if extra {
                        ui.label("extra fee");
                        ui.horizontal(|ui| {
                            crate::theme::credits(
                                ui,
                                egui::RichText::new(credits_label(fee_hour)).strong(),
                                "/h",
                            );
                            crate::theme::credits(
                                ui,
                                egui::RichText::new(credits_label(fee_minute)).small(),
                                "/min",
                            );
                        });
                        ui.end_row();
                    }
                    if let Some(old_price) = prompt.old_price {
                        ui.label("old price");
                        crate::theme::credits(
                            ui,
                            egui::RichText::new(rate_label(old_price as f64)),
                            "/h",
                        );
                        ui.end_row();
                    }
                    ui.label("script price");
                    crate::theme::credits(
                        ui,
                        egui::RichText::new(rate_label(price_hour)).strong(),
                        "/h",
                    );
                    ui.end_row();
                    if extra {
                        ui.label("in total");
                        ui.horizontal(|ui| {
                            crate::theme::credits(
                                ui,
                                egui::RichText::new(rate_label(total_hour)).strong(),
                                "/h",
                            );
                            crate::theme::credits(
                                ui,
                                egui::RichText::new(rate_label(total_month)).small(),
                                " per 30 days",
                            );
                        });
                        ui.end_row();
                    }
                });
            if extra {
                ui.add_space(6.0);
                ui.label(egui::RichText::new("The fee follows the order the scripts started in: when an older one stops, this one moves up and the oldest runs without a fee.").weak());
            }
            ui.add_space(8.0);
        });
        match decision {
            Some(true) => self.confirm_store_script(prompt.start),
            Some(false) => {}
            None => self.start_prompt = Some(prompt),
        }
    }

    pub(super) fn warning_ui(&mut self, ctx: &egui::Context) {
        let Some(start) = self.warning.take() else {
            return;
        };
        let script = &start.script;
        let decision = decision_window(ctx, "Community script", 420.0, "Run anyway", |ui| {
            ui.label(egui::RichText::new(&script.title).strong());
            ui.label(format!("by {}", script.author));
            ui.add_space(6.0);
            ui.label(
                "This is a community script. It was written by a community member and has not been reviewed by the Volcren team. \
                 It runs on your computer and controls your game client, so only run scripts from people you trust.",
            );
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(
                    "You won't be asked again until a new revision of this script comes out.",
                )
                .weak(),
            );
            ui.add_space(6.0);
        });
        match decision {
            Some(true) => {
                if let Some(revision) = script.revision {
                    self.confirmed.confirm(&script.id, revision);
                }
                self.start_store_script(start);
            }
            Some(false) => {}
            None => self.warning = Some(start),
        }
    }
}

impl LoaderApp {
    pub(super) fn render_start_prompt(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    pub(super) fn render_warning(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<AnyElement> {
        None
    }
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    Vec::new()
}
