use super::*;

impl LoaderApp {
    fn read_client_name(&mut self) {
        if let Some(target) = loaded_in(&self.targets, self.focus_client).cloned() {
            self.background(move |settings| {
                Event::AccountName(
                    backend::state(&target, &settings).map(|state| state.account_name),
                )
            });
        }
    }

    pub(super) fn render_accounts(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accounts = self.accounts.clone();
        let client = loaded_in(&self.targets, self.focus_client).is_some();
        let home = self.chrome.focus.clone();
        let ui = self.accounts_ui.get_or_insert_with(|| {
            crate::accounts::Ui::new(
                &accounts,
                home,
                |app: &LoaderApp| app.page == Page::Accounts,
                window,
                cx,
            )
        });
        crate::accounts::render(
            &accounts,
            ui,
            client,
            |app: &mut LoaderApp, _| app.read_client_name(),
            window,
            cx,
        )
    }
}

#[cfg(test)]
fn entry(name: &str, account: &str, pin: &str, lamp_skill: &str) -> crate::accounts::Entry {
    crate::accounts::Entry {
        name: name.into(),
        hash: injector::bank_pin::hash_name(account),
        pin: pin.into(),
        lamp_skill: lamp_skill.into(),
    }
}

#[cfg(test)]
fn saved(app: &mut LoaderApp) {
    app.page = Page::Accounts;
    app.accounts = Arc::new(RwLock::new(crate::accounts::Accounts::with_entries(vec![
        entry("Main", "first account", "1234", "Slayer"),
        entry("Ironman (just for me)", "second account", "9876", ""),
        entry("Skiller", "third account", "0000", "Woodcutting"),
    ])));
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("accounts-empty", |app| {
            app.page = Page::Accounts;
            app.accounts = Arc::new(RwLock::new(crate::accounts::Accounts::with_entries(
                Vec::new(),
            )));
        }),
        ("accounts-saved", saved),
        ("accounts-editing", |app| {
            saved(app);
            crate::accounts::snapshot_editing(&app.accounts);
        }),
        ("accounts-adding", |app| {
            saved(app);
            crate::accounts::snapshot_adding(&app.accounts);
        }),
        ("accounts-changing", |app| {
            saved(app);
            crate::accounts::snapshot_changing(&app.accounts);
        }),
        ("accounts-locked", |app| {
            app.page = Page::Accounts;
            app.accounts = Arc::new(RwLock::new(crate::accounts::Accounts::locked(
                "the system keyring isn't available",
            )));
        }),
    ]
}
