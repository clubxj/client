use super::*;

use std::rc::Rc;

use gpui_kit::Entity;
use gpui_kit::component::input::{InputEvent, TextareaState};
use gpui_kit::component::scroll::ScrollableElement;

use crate::ui::{theme, widgets};

#[derive(Default)]
pub(super) struct Ui {
    descriptions: HashMap<String, Entity<TextareaState>>,
}

impl LoaderApp {
    fn feedback_description(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextareaState> {
        let input = match self.feedback_page.descriptions.get(session_id) {
            Some(input) => input.clone(),
            None => {
                let input = feedback::description_input(window, cx);
                let id = session_id.to_owned();
                cx.subscribe_in(
                    &input,
                    window,
                    move |app, input, event: &InputEvent, window, cx| {
                        if !matches!(event, InputEvent::Change) {
                            return;
                        }
                        let mut text = input.read(cx).value().to_string();
                        if let Some(limited) = feedback::limit_description(&text) {
                            text = limited;
                            let value = text.clone();
                            input.update(cx, |state, cx| state.set_value(value, window, cx));
                        }
                        app.feedback.draft(&id).description = text;
                    },
                )
                .detach();
                self.feedback_page
                    .descriptions
                    .insert(session_id.to_owned(), input.clone());
                input
            }
        };
        let wanted = self.feedback.draft(session_id).description.clone();
        if input.read(cx).value().as_ref() != wanted {
            input.update(cx, |state, cx| state.set_value(wanted, window, cx));
        }
        input
    }

    fn feedback_click(&mut self, session_id: &str, click: feedback::Click) {
        let action = self
            .feedback
            .click(session_id, click, &mut self.settings.include_system_info);
        self.apply_feedback_actions(action.into_iter().collect());
    }

    fn feedback_clicks(cx: &mut Context<Self>) -> impl Fn(&str) -> feedback::OnClick + use<> {
        let app = cx.entity().downgrade();
        move |session_id| {
            let (app, session_id) = (app.clone(), session_id.to_owned());
            Rc::new(move |click, _, cx| {
                app.update(cx, |app, cx| {
                    app.feedback_click(&session_id, click);
                    cx.notify();
                })
                .ok();
            })
        }
    }

    pub(super) fn render_feedback(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = div()
            .flex()
            .flex_col()
            .size_full()
            .gap(theme::LINE_GAP)
            .child(widgets::heading("Feedback"));
        if self.auth.me().is_none() {
            return page
                .child("Sign in to score scripts and report bugs.")
                .into_any_element();
        }
        self.fetch_sessions_if_stale();
        let running: Vec<String> = self.store_runs().map(|s| s.session_id.clone()).collect();
        let ids: Vec<String> = self
            .feedback
            .sessions
            .iter()
            .map(|s| s.session_id.clone())
            .collect();
        self.feedback_page
            .descriptions
            .retain(|id, _| ids.contains(id));
        for id in &ids {
            self.feedback_description(id, window, cx);
        }
        let content = self.feedback.render(
            self.settings.include_system_info,
            &running,
            &self.feedback_page.descriptions,
            cx.listener(|app, _: &ClickEvent, _, cx| {
                app.apply_feedback_actions(vec![feedback::Action::Refresh]);
                cx.notify();
            }),
            Self::feedback_clicks(cx),
        );
        page.child(
            div()
                .id("feedback-page")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .child(div().pr(px(12.0)).pb(px(8.0)).child(content)),
        )
        .into_any_element()
    }

    pub(super) fn render_run_feedback(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(session_id) = self.run().and_then(|r| r.session_id().map(str::to_owned)) else {
            return div().into_any_element();
        };
        self.fetch_sessions_if_stale();
        let Some(session) = self
            .feedback
            .sessions
            .iter()
            .find(|s| s.session_id == session_id)
            .cloned()
        else {
            return div()
                .flex()
                .flex_col()
                .gap(theme::LINE_GAP)
                .child(widgets::weak(if self.feedback.loading {
                    "Loading…"
                } else {
                    "This run isn't listed yet; it is once the server has it."
                }))
                .when_some(self.feedback.error.clone(), |this, error| {
                    this.child(error_label(error))
                })
                .into_any_element();
        };
        let description = self.feedback_description(&session_id, window, cx);
        let on_click = Self::feedback_clicks(cx)(&session_id);
        let draft = self.feedback.draft(&session_id);
        feedback::session_ui(
            &session,
            draft,
            self.settings.include_system_info,
            &description,
            on_click,
        )
        .into_any_element()
    }
}

#[cfg(test)]
fn session(
    id: &str,
    title: &str,
    score: Option<u8>,
    reports: u32,
) -> crate::api::models::RecentSession {
    crate::api::models::RecentSession {
        session_id: id.into(),
        script_id: format!("script-{title}"),
        title: title.into(),
        url: title.to_lowercase(),
        revision: 4,
        started_at: "2026-11-21T09:12:40.000+00:00".into(),
        ended_at: Some("2026-11-21T10:02:11.000+00:00".into()),
        score,
        reports,
    }
}

#[cfg(test)]
fn fake_sessions(app: &mut LoaderApp) {
    app.page = Page::Feedback;
    app.feedback.finish(
        Ok(vec![
            session("a", "Firemaking", Some(8), 0),
            session("b", "Farming", None, 0),
            session("c", "Fishing", None, 2),
        ]),
        Instant::now(),
    );
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("feedback", |app| app.page = Page::Feedback),
        ("feedback-sessions", |app| {
            fake_sessions(app);
            app.feedback.scored("a", Ok(8));
            let draft = app.feedback.draft("c");
            draft.open = true;
            draft.description = "It stood still after the last log was burnt.".into();
            app.settings.include_system_info = true;
        }),
        ("feedback-sending", |app| {
            fake_sessions(app);
            app.feedback.draft("b").pending_score = Some(6);
            let draft = app.feedback.draft("c");
            draft.open = true;
            draft.sending = true;
            draft.description = "Crashed on the second wave.".into();
            app.feedback.draft("a").message = Some(Err("The server is unreachable.".into()));
        }),
    ]
}
