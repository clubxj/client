use super::*;

impl LoaderApp {
    pub(super) fn feedback_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.heading("Feedback");
        if self.auth.me().is_none() {
            ui.label("Sign in to score scripts and report bugs.");
            return;
        }
        self.fetch_sessions_if_stale(&ctx);
        let running: Vec<String> = self.store_runs().map(|s| s.session_id.clone()).collect();
        let actions = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.feedback
                    .ui(ui, &mut self.settings.include_system_info, &running)
            })
            .inner;
        self.apply_feedback_actions(&ctx, actions);
    }
}
