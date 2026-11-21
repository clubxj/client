use super::*;

impl LoaderApp {
    pub(super) fn store_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.auth.me().is_none() {
            match self.auth {
                Auth::Restoring => {
                    ui.label(RESTORING_SIGN_IN);
                }
                Auth::SigningIn(_) => {
                    ui.label(WAITING_FOR_BROWSER);
                }
                _ => {
                    ui.label("Sign in with Discord to browse and run scripts.");
                    if ui.button("Sign in").clicked() {
                        self.start_sign_in(&ctx);
                    }
                }
            }
            return;
        }
        if !self.store.loaded && !self.store.loading && self.store.error.is_none() {
            self.fetch_scripts(&ctx);
        }
        let target = self.run_target_ui(ui);
        let block = self.run_block(target.as_ref());
        let Some(me) = self.auth.me() else {
            return;
        };
        let has_site = !self.api.config().site_url.is_empty();
        match self.store.ui(ui, me, block.as_deref(), has_site) {
            Some(store::Action::OpenSite(path)) => open_site(&self.api.config().site_url, &path),
            Some(store::Action::Refresh) => {
                self.fetch_scripts(&ctx);
                self.api_job(&ctx, |api| Event::Me(api.me()));
            }
            Some(store::Action::Run(script)) => {
                if let Some(target) = target {
                    self.open_store_script(&ctx, *script, target);
                }
            }
            Some(store::Action::Redeem(code)) => {
                self.store.redeeming = true;
                self.store.redeem_result = None;
                self.api_job(&ctx, move |api| Event::Redeemed(api.redeem(&code)));
            }
            Some(store::Action::RemoveGrant(script)) => {
                let id = script.id.clone();
                self.api_job(&ctx, move |api| Event::GrantRemoved {
                    result: api.remove_grant(&id),
                    id,
                });
            }
            None => {}
        }
    }
}
