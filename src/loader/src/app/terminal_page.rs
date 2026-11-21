use super::*;

impl LoaderApp {
    pub(super) fn terminal_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.terminal.is_none() {
            let dir = crate::workspace::prepare(&self.settings)
                .map_err(|error| {
                    self.error(format!("Couldn't prepare the agent workspace: {error}"))
                })
                .ok();
            self.terminal = Some(crate::terminal::Terminal::new(&ctx, dir));
        }
        if let Some(terminal) = &mut self.terminal {
            terminal.ui(ui);
        }
    }
}
