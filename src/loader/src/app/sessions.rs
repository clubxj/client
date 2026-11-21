use gpui_kit::component::scroll::ScrollableElement;

use super::*;
use crate::ui::{theme, widgets};

pub(super) fn restart_button(
    id: impl Into<ElementId>,
    width: Pixels,
    store: bool,
    refusal: Option<String>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let hint = refusal.clone().unwrap_or_else(|| {
        if store {
            "Closes this session and starts the script again on the same client."
        } else {
            "Runs the file again on the same client, asking first if it changed on disk."
        }
        .to_owned()
    });
    widgets::icon_button(
        id,
        ICON_RESTART,
        "Restart",
        width,
        refusal.is_none(),
        on_click,
    )
    .tooltip(widgets::tooltip(hint))
}

fn run_id(what: &str, run: &Run) -> ElementId {
    ElementId::Name(format!("session-{what}-{}", run.id).into())
}

impl LoaderApp {
    fn session_listener(
        id: u64,
        act: fn(&mut Self, u64),
        cx: &mut Context<Self>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |app, _: &ClickEvent, _, cx| {
            act(app, id);
            cx.notify();
        })
    }

    fn render_session_billing(&self, run: &Run, store: &StoreRun, rate: f64) -> Div {
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(theme::ITEM_GAP)
            .gap_y(theme::LINE_GAP)
            .child(
                div()
                    .id(run_id("rate", run))
                    .tooltip(widgets::tooltip(EXTRA_SESSION_FEE_HINT))
                    .child(
                        widgets::credits(crate::api::models::rate_label(rate), "/h", false)
                            .font_weight(FontWeight::BOLD),
                    ),
            )
            .when(store.fee > 0, |this| {
                this.child(
                    widgets::weak("includes the extra-session fee")
                        .text_size(theme::SMALL)
                        .id(run_id("fee", run))
                        .tooltip(widgets::tooltip(EXTRA_SESSION_FEE_HINT)),
                )
            })
            .when(store.fee_charged > 0, |this| {
                this.child(
                    widgets::credits(
                        crate::api::models::credits_label(store.fee_charged as f64),
                        " in fees so far",
                        false,
                    )
                    .text_size(theme::SMALL),
                )
            })
    }

    fn render_session_card(
        &self,
        run: &Run,
        minutes_per_month: i64,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> Div {
        let client = run.target.as_ref().map_or("no client".to_owned(), |t| {
            format!("client {}", t.client.pid)
        });
        let rate = run
            .store
            .as_ref()
            .map_or(0.0, |store| store.rate_per_hour(minutes_per_month));
        let billed = run
            .store
            .as_ref()
            .filter(|store| rate > 0.0 || store.fee_charged > 0);
        let state = run.state();
        let about = format!("{}, {client}", state.label());
        let title = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(theme::ITEM_GAP)
            .gap_y(theme::LINE_GAP)
            .child(
                widgets::status_dot(state.color())
                    .id(run_id("dot", run))
                    .tooltip(widgets::tooltip(about.clone())),
            )
            .child(
                widgets::strong(run.title.clone())
                    .text_size(theme::HEADING)
                    .id(run_id("title", run))
                    .tooltip(widgets::tooltip(about)),
            )
            .child(widgets::weak(crate::xp::duration(run.run_time())))
            .when(billed.is_none(), |this| {
                this.child(
                    widgets::weak("free")
                        .text_size(theme::SMALL)
                        .id(run_id("free", run))
                        .tooltip(widgets::tooltip(if run.store.is_some() {
                            "This session costs nothing."
                        } else {
                            "Run from the editor, not billed."
                        })),
                )
            });
        let running = run.is_running();
        let stopping = run.stop_requested();
        let (pause_icon, pause_text) = run_pause_label(run.is_paused());
        let restart = (!running && !run.is_configuring()).then(|| {
            restart_button(
                run_id("restart", run),
                width,
                run.store.is_some(),
                self.restart_refusal(run),
                Self::session_listener(run.id, Self::restart_run, cx),
            )
        });
        let buttons = widgets::button_group(false)
            .child(widgets::icon_button(
                run_id("open", run),
                ICON_OPEN_RUN,
                "Open",
                width,
                true,
                Self::session_listener(
                    run.id,
                    |app, id| {
                        app.active_run = Some(id);
                        app.tab = Tab::Run;
                    },
                    cx,
                ),
            ))
            .child(widgets::icon_button(
                run_id("pause", run),
                pause_icon,
                pause_text,
                width,
                running && !stopping,
                Self::session_listener(run.id, Self::toggle_pause_of, cx),
            ))
            .child(widgets::icon_button(
                run_id("stop", run),
                ICON_STOP,
                "Stop",
                width,
                running && !stopping,
                Self::session_listener(
                    run.id,
                    |app, id| {
                        if let Some(run) = app.run_by_id_mut(id) {
                            run.stop();
                        }
                    },
                    cx,
                ),
            ))
            .child(
                widgets::icon_button(
                    run_id("close", run),
                    ICON_CANCEL,
                    "Close",
                    width,
                    !running,
                    Self::session_listener(run.id, Self::close_run, cx),
                )
                .when(!running, |this| {
                    this.tooltip(widgets::tooltip(
                        "Takes the ended run and its log off the list.",
                    ))
                }),
            )
            .children(restart);
        widgets::card()
            .flex()
            .flex_col()
            .items_start()
            .gap(theme::LINE_GAP)
            .child(title)
            .children(billed.map(|store| self.render_session_billing(run, store, rate)))
            .child(div().mt(theme::LINE_GAP).child(buttons))
    }

    pub(super) fn render_sessions(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(theme::LINE_GAP)
            .child(widgets::heading("Sessions"));
        if self.runs.is_empty() {
            return page
                .child(widgets::weak(
                    "Nothing is running. Start a script from the Scripts page, or the editor.",
                ))
                .into_any_element();
        }
        let minutes_per_month = self.auth.me().map_or(43_200, |me| me.minutes_per_month);
        let width = widgets::icon_button_width(
            window,
            &[
                (ICON_OPEN_RUN, "Open"),
                PAUSE_BUTTONS[0],
                PAUSE_BUTTONS[1],
                (ICON_STOP, "Stop"),
                (ICON_CANCEL, "Close"),
                (ICON_RESTART, "Restart"),
            ],
        );
        let cards: Vec<Div> = self
            .runs
            .iter()
            .map(|run| self.render_session_card(run, minutes_per_month, width, cx))
            .collect();
        page.child(
            div()
                .id("sessions")
                .flex_1()
                .min_h_0()
                .mt(px(6.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(12.0))
                        .pt(px(8.0))
                        .children(cards),
                )
                .overflow_y_scrollbar(),
        )
        .into_any_element()
    }
}

#[cfg(test)]
fn fake_run(
    app: &mut LoaderApp,
    title: &str,
    script: Option<Script>,
    store: Option<(i64, i64, i64)>,
    minutes: u64,
) {
    let id = app.next_run_id();
    let store = store.map(|(price, fee, fee_charged)| StoreRun {
        session_id: format!("session-{id}"),
        script_id: title.to_lowercase(),
        community: false,
        closed: channel().1,
        lib_version: "1".into(),
        price,
        rank: 1,
        fee,
        fee_charged,
    });
    let target = super::store_page::fake_target(4000 + id as u32);
    let mut run = Run::new(
        id,
        title.into(),
        script,
        Some(target),
        store,
        &Settings::default(),
    );
    run.started = Instant::now() - Duration::from_secs(minutes * 60 + 17);
    if run
        .script
        .as_ref()
        .is_some_and(|script| !script.is_running())
    {
        run.ended = Some(Instant::now());
    }
    app.runs.push(run);
}

#[cfg(test)]
fn running(paused: bool) -> Option<Script> {
    Some(Script::stand_in(paused, false, false))
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("sessions-empty", |app| app.tab = Tab::Sessions),
        ("sessions", |app| {
            app.tab = Tab::Sessions;
            fake_run(app, "Tempoross", running(false), Some((180_000, 0, 0)), 83);
            fake_run(
                app,
                "Miner",
                running(true),
                Some((70_000, 2_000_000, 25_000)),
                12,
            );
            fake_run(app, "Brazier helper", running(false), Some((0, 0, 0)), 5);
            fake_run(
                app,
                "Stopping script",
                Some(Script::stand_in(false, true, false)),
                None,
                1,
            );
            fake_run(app, "editor.luau", running(false), None, 3);
            fake_run(
                app,
                "Raid helper",
                Some(Script::stand_in(false, false, true)),
                Some((120_000, 0, 0)),
                140,
            );
            fake_run(
                app,
                "old.luau",
                Some(Script::stand_in(false, false, true)),
                None,
                9,
            );
            fake_run(app, "Configuring", None, Some((180_000, 0, 0)), 0);
            app.targets = app
                .runs
                .iter()
                .filter_map(|run| run.target.clone())
                .collect();
        }),
    ]
}
