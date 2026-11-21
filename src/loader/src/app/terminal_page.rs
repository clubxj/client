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

impl LoaderApp {
    pub(super) fn render_terminal(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        not_ported("Terminal")
    }
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("terminal", |app| app.tab = Tab::Terminal),
    ]
}
