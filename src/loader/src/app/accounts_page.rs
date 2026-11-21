use super::*;

impl LoaderApp {
    pub(super) fn accounts_ui(&mut self, ui: &mut egui::Ui) {
        let target = self.selected_loaded_target();
        let action = crate::script::write(&self.accounts).ui(ui, target.is_some());
        if let Some(crate::accounts::Action::ReadClientName) = action
            && let Some(target) = target.cloned()
        {
            let ctx = ui.ctx().clone();
            self.background(&ctx, move |settings| {
                Event::AccountName(
                    backend::state(&target, &settings).map(|state| state.account_name),
                )
            });
        }
    }
}
