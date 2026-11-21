use super::*;

impl LoaderApp {
    pub(super) fn accounts_ui(&mut self, ui: &mut egui::Ui) {
        let target = self.selected_loaded_target();
        let action = crate::script::write(&self.accounts).ui(ui, target.is_some());
        if let Some(crate::accounts::Action::ReadClientName) = action
            && let Some(target) = target.cloned()
        {
            let ctx = ui.ctx().clone();
            self.background(move |settings| {
                Event::AccountName(
                    backend::state(&target, &settings).map(|state| state.account_name),
                )
            });
        }
    }
}

impl LoaderApp {
    pub(super) fn render_accounts(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        not_ported("Accounts")
    }
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("accounts", |app| app.tab = Tab::Accounts),
    ]
}
