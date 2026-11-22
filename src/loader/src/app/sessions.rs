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

    pub(super) fn render_session_billing(&self, run: &Run, store: &StoreRun, rate: f64) -> Div {
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
                            "Run from a local file, not billed."
                        })),
                )
            });
        let restart = (!run.is_running() && !run.is_configuring() && !run.agent).then(|| {
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
                    |app, id| app.go(Page::Run(RunPage::Overview), Some(id)),
                    cx,
                ),
            ))
            .child(
                widgets::icon_button(
                    run_id("close", run),
                    ICON_CANCEL,
                    "Close",
                    width,
                    true,
                    Self::session_listener(run.id, Self::close_run, cx),
                )
                .tooltip(widgets::tooltip(
                    "Takes the ended run and its log off the list.",
                )),
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

    fn client_activity(&self, pid: u32) -> (String, Rgba, Option<Duration>, f64) {
        let minutes_per_month = self.auth.me().map_or(43_200, |me| me.minutes_per_month);
        let rate = self
            .runs_on(pid)
            .filter(|r| r.is_running())
            .filter_map(|r| r.store.as_ref())
            .map(|store| store.rate_per_hour(minutes_per_month))
            .sum();
        if let Some(runner) = self.queue_on(pid) {
            return (
                format!("queue {}", runner.progress()),
                theme::GREEN,
                Some(runner.started.elapsed()),
                rate,
            );
        }
        if let Some(run) = self.live_run_on(pid) {
            let state = run.state();
            return (
                format!("{}, {}", run.title, state.label()),
                state.color(),
                Some(run.run_time()),
                rate,
            );
        }
        let text = match self.client_state(pid) {
            nav::ClientState::Unloaded => match self.detected(pid) {
                None => "closed",
                Some(t) if t.client.foreign_bot.is_some() => "used by another platform",
                Some(t) if !t.client.is_64bit => "32-bit, unsupported",
                Some(_) => "agent not loaded",
            },
            nav::ClientState::Failed => "last script failed",
            _ => "idle",
        };
        let color = match self.client_state(pid) {
            nav::ClientState::Failed => theme::RED,
            _ => theme::COMMENT,
        };
        (text.to_owned(), color, None, rate)
    }

    fn render_client_row(&self, index: usize, pid: u32, cx: &mut Context<Self>) -> Stateful<Div> {
        let (activity, color, time, rate) = self.client_activity(pid);
        let title = self
            .detected(pid)
            .map_or_else(String::new, |t| t.client.title.clone());
        let name_color = match self.client_state(pid) {
            nav::ClientState::Unloaded => theme::COMMENT,
            nav::ClientState::Running => theme::GREEN,
            nav::ClientState::Failed => theme::RED,
            nav::ClientState::Idle => theme::FG,
        };
        div()
            .id(("session-client", index))
            .flex()
            .items_center()
            .gap(px(16.0))
            .px(px(8.0))
            .py(px(2.0))
            .rounded(px(3.0))
            .when(index % 2 == 1, |this| this.bg(theme::BG_FLOAT))
            .cursor_pointer()
            .hover(|style| style.bg(theme::SEL))
            .on_click(cx.listener(move |app, _: &ClickEvent, _, cx| {
                app.go(Page::Client(pid), None);
                cx.notify();
            }))
            .child(
                div()
                    .flex_none()
                    .w(px(110.0))
                    .text_color(name_color)
                    .font_weight(FontWeight::BOLD)
                    .child(format!("client {pid}")),
            )
            .child(
                div()
                    .flex_basis(px(160.0))
                    .flex_shrink(1.0)
                    .min_w_0()
                    .truncate()
                    .child(title),
            )
            .child(
                div()
                    .flex_basis(px(240.0))
                    .flex_shrink(1.0)
                    .min_w_0()
                    .truncate()
                    .text_color(color)
                    .child(activity),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(70.0))
                    .text_color(theme::COMMENT)
                    .child(time.map(crate::xp::duration).unwrap_or_default()),
            )
            .child(div().flex_none().when(rate > 0.0, |this| {
                this.child(widgets::credits(
                    crate::api::models::rate_label(rate),
                    "/h",
                    true,
                ))
            }))
    }

    pub(super) fn render_sessions(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let header = widgets::heading("Sessions");
        let page = div()
            .flex_1()
            .min_h_0()
            .relative()
            .flex()
            .flex_col()
            .gap(theme::LINE_GAP)
            .child(crate::logo::background())
            .child(header);
        let pids = self.listed_clients();
        if pids.is_empty() {
            return page
                .child(widgets::weak(if self.scanning {
                    "Looking for game clients…"
                } else {
                    "No game clients found."
                }))
                .into_any_element();
        }
        let rows: Vec<Stateful<Div>> = pids
            .into_iter()
            .enumerate()
            .map(|(index, pid)| self.render_client_row(index, pid, cx))
            .collect();
        page.child(
            div()
                .id("session-clients")
                .flex_1()
                .min_h_0()
                .mx(px(-8.0))
                .child(div().flex().flex_col().children(rows))
                .overflow_y_scrollbar(),
        )
        .into_any_element()
    }

    pub(super) fn render_history(
        &mut self,
        pid: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let minutes_per_month = self.auth.me().map_or(43_200, |me| me.minutes_per_month);
        let width = widgets::icon_button_width(
            window,
            &[
                (ICON_OPEN_RUN, "Open"),
                (ICON_CANCEL, "Close"),
                (ICON_RESTART, "Restart"),
            ],
        );
        let cards: Vec<Div> = self
            .ended_runs_on(pid)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|run| self.render_session_card(run, minutes_per_month, width, cx))
            .collect();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(theme::LINE_GAP)
            .child(widgets::heading(format!("Client {pid}: history")))
            .child(
                div()
                    .id("history")
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
pub(super) fn fake_run(
    app: &mut LoaderApp,
    pid: u32,
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
        price,
        rank: 1,
        fee,
        fee_charged,
    });
    let target = super::store_page::fake_target(pid);
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
pub(super) fn running(paused: bool) -> Option<Script> {
    Some(Script::stand_in(paused, false, false))
}

#[cfg(test)]
pub(super) fn populated(app: &mut LoaderApp) {
    super::clients::populated(app);
    fake_run(
        app,
        4120,
        "Tempoross",
        running(false),
        Some((180_000, 0, 0)),
        83,
    );
    fake_run(
        app,
        4120,
        "Raid helper",
        Some(Script::stand_in(false, false, true)),
        Some((120_000, 0, 0)),
        140,
    );
    if let Some(run) = app.runs.last_mut() {
        run.finished = Some((Err("the boss despawned".into()), false));
    }
    fake_run(
        app,
        6012,
        "Miner",
        running(true),
        Some((70_000, 2_000_000, 25_000)),
        12,
    );
    fake_run(app, 5388, "Fletcher", None, Some((90_000, 0, 0)), 0);
    app.targets[1] = super::clients::target(
        5388,
        "Game client - second account",
        Some("2026.11.20-abc1234"),
        false,
    );
    fake_run(
        app,
        6012,
        "old.luau",
        Some(Script::stand_in(false, false, true)),
        None,
        9,
    );
    fake_run(
        app,
        9100,
        "editor.luau",
        Some(Script::stand_in(false, true, false)),
        None,
        1,
    );
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("sessions-empty", |app| app.page = Page::Sessions),
        ("sessions", |app| {
            populated(app);
            app.page = Page::Sessions;
        }),
        ("history", |app| {
            populated(app);
            app.page = Page::History(4120);
        }),
    ]
}
