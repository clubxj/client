use super::*;
use crate::ui::{theme, widgets};

struct Decision {
    id: &'static str,
    title: &'static str,
    width: Pixels,
    confirm: &'static str,
    decide: fn(&mut LoaderApp, bool),
}

fn answer(
    decide: fn(&mut LoaderApp, bool),
    yes: bool,
    cx: &mut Context<LoaderApp>,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    cx.listener(move |app, _: &ClickEvent, _, cx| {
        decide(app, yes);
        cx.notify();
    })
}

fn decision_window(
    decision: Decision,
    body: Div,
    window: &Window,
    cx: &mut Context<LoaderApp>,
) -> AnyElement {
    let buttons = div()
        .flex()
        .gap(theme::ITEM_GAP)
        .child(widgets::button(
            ElementId::Name(format!("{}-confirm", decision.id).into()),
            decision.confirm,
            true,
            answer(decision.decide, true, cx),
        ))
        .child(widgets::button(
            ElementId::Name(format!("{}-cancel", decision.id).into()),
            "Cancel",
            true,
            answer(decision.decide, false, cx),
        ));
    let width = decision
        .width
        .min(window.viewport_size().width - px(32.0))
        .max(px(0.0));
    div()
        .id(decision.id)
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::alpha(gpui_kit::rgb(0x000000), 130.0 / 255.0))
        .occlude()
        .child(
            div()
                .flex()
                .flex_col()
                .max_w(width)
                .bg(theme::BG_FLOAT)
                .border_1()
                .border_color(theme::BORDER)
                .child(
                    div()
                        .px(px(8.0))
                        .py(theme::LINE_GAP)
                        .text_size(theme::HEADING)
                        .child(decision.title),
                )
                .child(divider())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(theme::LINE_GAP)
                        .p(px(8.0))
                        .child(body)
                        .child(buttons),
                ),
        )
        .into_any_element()
}

const GRID_ROW: Pixels = px(20.0);
const PROXY_LOST: &str = "This script runs on an older client release that can't use proxies. Its agent replaces this client's, so the client's connections skip the proxy and go out on your own IP until the script ends.";

fn strong_credits(text: String, suffix: &str) -> Div {
    widgets::credits(text, suffix, false).font_weight(FontWeight::BOLD)
}

fn small_credits(text: String, suffix: &str) -> Div {
    widgets::credits(text, suffix, false).text_size(theme::SMALL)
}

fn pair(first: Div, second: Div) -> Div {
    div()
        .flex()
        .items_center()
        .gap(theme::ITEM_GAP)
        .child(first)
        .child(second)
}

impl LoaderApp {
    pub(super) fn render_start_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.auth.me().is_none() {
            self.start_prompt = None;
        }
        let prompt = self.start_prompt.as_ref()?;
        let me = self.auth.me()?;
        use crate::api::models::{credits_label, fee_per_hour, per_month, rate_label};
        let fee_hour = fee_per_hour(prompt.fee, me.minutes_per_month);
        let fee_minute = fee_hour / 60.0;
        let price_hour = prompt.price as f64;
        let total_hour = price_hour + fee_hour;
        let total_month = per_month(prompt.price, prompt.fee, me.minutes_per_month);
        let extra = prompt.fee > 0;
        let nth = match prompt.nth {
            2 => "2nd".to_owned(),
            3 => "3rd".to_owned(),
            n => format!("{n}th"),
        };
        let title = if prompt.start.replaces.is_some() {
            "Restart session"
        } else if extra {
            "Extra session"
        } else {
            "Run without the proxy"
        };
        let mut rows: Vec<(&str, Div)> = Vec::new();
        if extra {
            rows.push((
                "extra fee",
                pair(
                    strong_credits(credits_label(fee_hour), "/h"),
                    small_credits(credits_label(fee_minute), "/min"),
                ),
            ));
        }
        if let Some(old_price) = prompt.old_price {
            rows.push((
                "old price",
                widgets::credits(rate_label(old_price as f64), "/h", false),
            ));
        }
        rows.push(("script price", strong_credits(rate_label(price_hour), "/h")));
        if extra {
            rows.push((
                "in total",
                pair(
                    strong_credits(rate_label(total_hour), "/h"),
                    small_credits(rate_label(total_month), " per 30 days"),
                ),
            ));
        }
        let column = || div().flex().flex_col().gap(theme::LINE_GAP);
        let cell = |content: Div| div().flex().items_center().h(GRID_ROW).child(content);
        let (labels, values): (Vec<Div>, Vec<Div>) = rows
            .into_iter()
            .map(|(label, value)| (cell(div().child(label)), cell(value)))
            .unzip();
        let grid = div()
            .flex()
            .gap(px(16.0))
            .mt(theme::LINE_GAP)
            .child(column().children(labels))
            .child(column().children(values));
        let body = div()
            .flex()
            .flex_col()
            .items_start()
            .gap(theme::LINE_GAP)
            .pb(px(8.0))
            .child(widgets::strong(prompt.start.script.title.clone()).mb(px(6.0)))
            .when(prompt.start.proxy_lost, |this| {
                this.child(widgets::error(PROXY_LOST))
            })
            .when(prompt.old_price.is_some(), |this| {
                this.child("The price of this script changed since the session started.")
            })
            .when(extra, |this| {
                this.child(format!(
                    "This would be your {nth} script running at once. Every script past the first adds a fee on top of its own price:"
                ))
            })
            .child(grid)
            .when(extra, |this| {
                this.child(widgets::weak("The fee follows the order the scripts started in: when an older one stops, this one moves up and the oldest runs without a fee.").mt(px(6.0)))
            });
        Some(decision_window(
            Decision {
                id: "start-prompt",
                title,
                width: px(440.0),
                confirm: "Confirm",
                decide: |app, confirmed| {
                    if let Some(prompt) = app.start_prompt.take()
                        && confirmed
                    {
                        app.confirm_store_script(prompt.start);
                    }
                },
            },
            body,
            window,
            cx,
        ))
    }

    pub(super) fn render_warning(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let script = &self.warning.as_ref()?.script;
        let body = div()
            .flex()
            .flex_col()
            .items_start()
            .gap(theme::LINE_GAP)
            .pb(px(6.0))
            .child(widgets::strong(script.title.clone()))
            .child(format!("by {}", script.author))
            .child(
                div().mt(px(6.0)).child(
                    "This is a community script. It was written by a community member and has not been reviewed by the Volcren team. \
                     It runs on your computer and controls your game client, so only run scripts from people you trust.",
                ),
            )
            .child(
                widgets::weak("You won't be asked again until a new revision of this script comes out.")
                    .mt(px(6.0)),
            );
        Some(decision_window(
            Decision {
                id: "community-warning",
                title: "Community script",
                width: px(420.0),
                confirm: "Run anyway",
                decide: |app, confirmed| {
                    if let Some(start) = app.warning.take()
                        && confirmed
                    {
                        if let Some(revision) = start.script.revision {
                            app.confirmed.confirm(&start.script.id, revision);
                        }
                        app.start_store_script(start);
                    }
                },
            },
            body,
            window,
            cx,
        ))
    }
}

#[cfg(test)]
fn pending(script: StoreScript, replaces: Option<u64>) -> PendingStart {
    PendingStart {
        script,
        target: super::store_page::fake_target(4242),
        replaces,
        run: None,
        launch: Launch::default(),
        injector: crate::injectors::Pinned {
            version: crate::update::VERSION.into(),
            path: std::path::PathBuf::new(),
        },
        proxy_lost: false,
    }
}

#[cfg(test)]
fn prompt(app: &mut LoaderApp, replaces: Option<u64>, fee: i64, nth: i64, old_price: Option<i64>) {
    app.page = Page::Store(4242);
    app.targets = vec![super::store_page::fake_target(4242)];
    app.store.loaded = true;
    app.start_prompt = Some(StartPrompt {
        start: pending(super::store_page::fake_script("Miner", 180_000), replaces),
        price: 180_000,
        fee,
        nth,
        old_price,
    });
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("dialog-extra-session", |app| {
            prompt(app, None, 2_000_000, 2, None)
        }),
        ("dialog-restart-session", |app| {
            prompt(app, Some(1), 0, 1, Some(120_000))
        }),
        ("dialog-restart-extra", |app| {
            prompt(app, Some(1), 2_000_000, 3, Some(120_000))
        }),
        ("dialog-community-warning", |app| {
            app.page = Page::Store(4242);
            app.targets = vec![super::store_page::fake_target(4242)];
            app.store.loaded = true;
            app.warning = Some(pending(
                StoreScript {
                    community: true,
                    author: "someone".into(),
                    ..super::store_page::fake_script("Brazier helper", 70_000)
                },
                None,
            ));
        }),
    ]
}
