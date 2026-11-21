use super::*;

pub(super) fn divider(ui: &mut egui::Ui) {
    let (line, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(
        line,
        0.0,
        ui.visuals().widgets.noninteractive.bg_stroke.color,
    );
}

pub(super) fn fit_row(ui: &mut egui::Ui, fits: bool, add: impl FnOnce(&mut egui::Ui)) {
    if fits {
        ui.horizontal(add);
    } else {
        ui.vertical(add);
    }
}

pub(super) fn error_label(ui: &mut egui::Ui, error: impl Into<egui::RichText>) {
    ui.colored_label(ui.visuals().error_fg_color, error);
}

impl LoaderApp {
    pub(super) fn run_target_ui(&mut self, ui: &mut egui::Ui) -> Option<Target> {
        let free: Vec<(&Target, String)> = self
            .targets
            .iter()
            .filter(|t| self.client_free(t))
            .map(|t| (t, t.key()))
            .collect();
        if !free
            .iter()
            .any(|(_, key)| Some(key) == self.run_target.as_ref())
        {
            self.run_target = free
                .iter()
                .find(|(_, key)| Some(key) == self.selected.as_ref())
                .or_else(|| free.first())
                .map(|(_, key)| key.clone());
        }
        ui.vertical(|ui| {
            ui.label("Client:");
            let label = |t: &Target| format!("client {} ({})", t.client.pid, t.client.title);
            let current = free.iter().find(|(_, key)| Some(key) == self.run_target.as_ref());
            egui::ComboBox::from_id_salt("run_target")
                .selected_text(current.map_or_else(|| "no free client".to_owned(), |(t, _)| label(t)))
                .show_ui(ui, |ui| {
                    for (t, key) in &free {
                        ui.selectable_value(&mut self.run_target, Some(key.clone()), label(t));
                    }
                });
            if free.is_empty() {
                ui.label(egui::RichText::new("Load the agent into a client on the Client page; a client runs one script at a time.").weak());
            }
        });
        ui.add_space(6.0);
        self.run_target_client()
    }
}

pub(super) fn not_ported(name: &str) -> AnyElement {
    div()
        .p(px(8.0))
        .text_color(crate::ui::theme::COMMENT)
        .child(format!("{name}: not ported to GPUI yet"))
        .into_any_element()
}
