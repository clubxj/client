use super::*;

pub(super) fn divider() -> Div {
    div().flex_none().h(px(1.0)).w_full().bg(crate::ui::theme::BORDER)
}

pub(super) fn fit_row(fits: bool) -> Div {
    let row = div().flex().gap(crate::ui::theme::ITEM_GAP);
    if fits {
        row.items_center()
    } else {
        row.flex_col().gap(crate::ui::theme::LINE_GAP)
    }
}

pub(super) fn error_label(error: impl Into<gpui_kit::SharedString>) -> Div {
    crate::ui::widgets::error(error)
}

impl LoaderApp {
    pub(super) fn render_run_target(&mut self, cx: &mut Context<Self>) -> (Div, Option<Target>) {
        let free: Vec<(String, String)> = self
            .targets
            .iter()
            .filter(|t| self.client_free(t))
            .map(|t| {
                (
                    t.key(),
                    format!("client {} ({})", t.client.pid, t.client.title),
                )
            })
            .collect();
        if !free
            .iter()
            .any(|(key, _)| Some(key) == self.run_target.as_ref())
        {
            self.run_target = free
                .iter()
                .find(|(key, _)| Some(key) == self.selected.as_ref())
                .or_else(|| free.first())
                .map(|(key, _)| key.clone());
        }
        let current = free
            .iter()
            .find(|(key, _)| Some(key) == self.run_target.as_ref())
            .map_or_else(|| "no free client".to_owned(), |(_, label)| label.clone());
        let app = cx.entity().downgrade();
        let choices = free
            .iter()
            .map(|(key, label)| {
                let (app, key) = (app.clone(), key.clone());
                crate::ui::widgets::Choice::new(
                    label.clone(),
                    Some(&key) == self.run_target.as_ref(),
                    move |_, cx| {
                        let key = key.clone();
                        app.update(cx, |app, cx| {
                            app.run_target = Some(key);
                            cx.notify();
                        })
                        .ok();
                    },
                )
            })
            .collect();
        let picker = div()
            .flex()
            .flex_col()
            .items_start()
            .gap(crate::ui::theme::LINE_GAP)
            .pb(px(6.0))
            .child("Client:")
            .child(crate::ui::widgets::combo("run-target", current, choices))
            .when(free.is_empty(), |this| {
                this.child(crate::ui::widgets::weak(
                    "Load the agent into a client on the Client page; a client runs one script at a time.",
                ))
            });
        (picker, self.run_target_client())
    }
}

pub(super) fn not_ported(name: &str) -> AnyElement {
    div()
        .p(px(8.0))
        .text_color(crate::ui::theme::COMMENT)
        .child(format!("{name}: not ported to GPUI yet"))
        .into_any_element()
}

pub(super) fn egui_divider(ui: &mut egui::Ui) {
    let (line, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(
        line,
        0.0,
        ui.visuals().widgets.noninteractive.bg_stroke.color,
    );
}

pub(super) fn egui_fit_row(ui: &mut egui::Ui, fits: bool, add: impl FnOnce(&mut egui::Ui)) {
    if fits {
        ui.horizontal(add);
    } else {
        ui.vertical(add);
    }
}

pub(super) fn egui_error_label(ui: &mut egui::Ui, error: impl Into<egui::RichText>) {
    ui.colored_label(ui.visuals().error_fg_color, error);
}

impl LoaderApp {
    pub(super) fn egui_run_target_ui(&mut self, ui: &mut egui::Ui) -> Option<Target> {
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
