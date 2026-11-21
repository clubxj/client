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
