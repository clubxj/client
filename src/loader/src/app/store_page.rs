use std::{cell::Cell, rc::Rc};

use gpui_kit::component::Sizable;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{Entity, SharedString, Subscription};

use super::*;
use crate::store::{Action, Placement};
use crate::ui::{theme, widgets};

pub(super) struct Ui {
    search: Entity<InputState>,
    code: Entity<InputState>,
    width: Rc<Cell<Pixels>>,
    _subscriptions: [Subscription; 2],
}

impl Ui {
    fn new(store: &Store, window: &mut Window, cx: &mut Context<LoaderApp>) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search scripts")
                .default_value(store.search.clone())
        });
        let code = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Paste a share code")
                .default_value(store.code.clone())
        });
        let subscriptions = [
            cx.subscribe_in(&search, window, |app, input, event: &InputEvent, _, cx| {
                if let InputEvent::Change = event {
                    app.store.search = input.read(cx).value().to_string();
                    cx.notify();
                }
            }),
            cx.subscribe_in(&code, window, |app, input, event: &InputEvent, _, cx| {
                match event {
                    InputEvent::Change => app.store.code = input.read(cx).value().to_string(),
                    InputEvent::PressEnter { .. } => app.redeem_code(),
                    _ => return,
                }
                cx.notify();
            }),
        ];
        Self {
            search,
            code,
            width: Rc::new(Cell::new(window.viewport_size().width)),
            _subscriptions: subscriptions,
        }
    }

    fn sync(&self, store: &Store, window: &mut Window, cx: &mut App) {
        for (input, value) in [(&self.search, &store.search), (&self.code, &store.code)] {
            if input.read(cx).value().as_ref() != value.as_str() {
                let value = SharedString::from(value.clone());
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            }
        }
    }
}

fn measured_width(cell: Rc<Cell<Pixels>>) -> impl IntoElement {
    canvas(
        move |bounds, window, _| {
            if cell.get() != bounds.size.width {
                cell.set(bounds.size.width);
                window.request_animation_frame();
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .w_full()
    .h(px(0.0))
}

#[derive(Clone, Copy)]
struct Account<'a> {
    me: &'a Me,
    block: Option<&'a str>,
    has_site: bool,
}

fn script_id(what: &str, script: &StoreScript) -> ElementId {
    ElementId::Name(format!("store-{what}-{}", script.id).into())
}

impl LoaderApp {
    fn store_action(&mut self, action: Action) {
        match action {
            Action::OpenSite(path) => open_site(&self.api.config().site_url, &path),
            Action::Refresh => {
                self.fetch_scripts();
                self.api_job(|api| Event::Me(api.me()));
            }
            Action::Run(script) => {
                if let Some(target) = self.run_target_client() {
                    self.open_store_script(*script, target);
                }
            }
            Action::Redeem(code) => {
                self.store.redeeming = true;
                self.store.redeem_result = None;
                self.api_job(move |api| Event::Redeemed(api.redeem(&code)));
            }
            Action::RemoveGrant(script) => {
                let id = script.id.clone();
                self.api_job(move |api| Event::GrantRemoved {
                    result: api.remove_grant(&id),
                    id,
                });
            }
        }
    }

    fn redeem_code(&mut self) {
        if let Some(action) = self.store.redeem() {
            self.store_action(action);
        }
    }

    fn act(
        action: impl Fn() -> Action + 'static,
        cx: &mut Context<Self>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |app, _: &ClickEvent, _, cx| {
            app.store_action(action());
            cx.notify();
        })
    }

    fn render_store_signed_out(&mut self, cx: &mut Context<Self>) -> AnyElement {
        match self.auth {
            Auth::Restoring => div().child(RESTORING_SIGN_IN),
            Auth::SigningIn(_) => div().child(WAITING_FOR_BROWSER),
            _ => div()
                .flex()
                .flex_col()
                .items_start()
                .gap(theme::LINE_GAP)
                .child("Sign in with Discord to browse and run scripts.")
                .child(widgets::button(
                    "store-sign-in",
                    "Sign in",
                    true,
                    cx.listener(|app, _: &ClickEvent, _, cx| {
                        app.start_sign_in();
                        cx.notify();
                    }),
                )),
        }
        .into_any_element()
    }

    fn render_store_filters(
        &self,
        me: &Me,
        search: &Entity<InputState>,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> Div {
        let app = cx.entity().downgrade();
        let category = |name: String, label: String, selected: bool| {
            let app = app.clone();
            widgets::Choice::new(label, selected, move |_, cx| {
                let name = name.clone();
                app.update(cx, |app, cx| {
                    app.store.category = name;
                    cx.notify();
                })
                .ok();
            })
        };
        let choices = std::iter::once(category(
            String::new(),
            "All categories".into(),
            self.store.category.is_empty(),
        ))
        .chain(self.store.categories().into_iter().map(|name| {
            let selected = name == self.store.category;
            category(name.clone(), name, selected)
        }))
        .collect();
        let current = if self.store.category.is_empty() {
            "All categories".to_owned()
        } else {
            self.store.category.clone()
        };
        let community = me.can_browse_community().then(|| {
            let app = app.clone();
            Checkbox::new("store-community")
                .label("Community")
                .checked(self.store.community)
                .tooltip(
                    "List every community script you can see. They stay out of the default list.",
                )
                .small()
                .on_click(move |checked, _, cx| {
                    let checked = *checked;
                    app.update(cx, |app, cx| {
                        app.store.community = checked;
                        app.store_action(Action::Refresh);
                        cx.notify();
                    })
                    .ok();
                })
        });
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(theme::ITEM_GAP)
            .gap_y(theme::LINE_GAP)
            .child(
                Input::new(search)
                    .small()
                    .w((width * 0.4).clamp(px(100.0), px(220.0))),
            )
            .child(widgets::combo("store-category", current, choices))
            .child(widgets::button(
                "store-refresh",
                "Refresh",
                !self.store.loading,
                Self::act(|| Action::Refresh, cx),
            ))
            .children(community)
            .when(self.store.loading, |this| this.child(Spinner::new()))
    }

    fn render_store_redeem(
        &self,
        code: &Entity<InputState>,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> Div {
        let result = self.store.redeem_result.as_ref().map(|(ok, message)| {
            div()
                .text_color(if *ok { theme::GREEN } else { theme::RED })
                .child(message.clone())
        });
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(theme::ITEM_GAP)
            .gap_y(theme::LINE_GAP)
            .child(
                Input::new(code)
                    .small()
                    .disabled(self.store.redeeming)
                    .w((width - px(190.0)).clamp(px(120.0), px(420.0))),
            )
            .child(widgets::button(
                "store-redeem",
                "Redeem",
                self.store.can_redeem(),
                cx.listener(|app, _: &ClickEvent, _, cx| {
                    app.redeem_code();
                    cx.notify();
                }),
            ))
            .when(self.store.redeeming, |this| this.child(Spinner::new()))
            .children(result)
    }

    fn render_store_details(&self, script: &StoreScript, cx: &mut Context<Self>) -> Div {
        let origin = div()
            .id(script_id("origin", script))
            .text_size(theme::SMALL)
            .text_color(crate::store::origin_color(script))
            .tooltip(widgets::tooltip(crate::store::origin_hint(script)))
            .child(crate::store::origin_label(script));
        let tags = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(theme::ITEM_GAP)
            .gap_y(theme::LINE_GAP)
            .child(widgets::strong(script.title.clone()).text_size(theme::HEADING))
            .child(widgets::weak(format!("by {}", script.author)))
            .child(div().text_size(theme::SMALL).child(script.stage.clone()))
            .child(origin)
            .when(script.granted, |this| {
                this.child(
                    div()
                        .text_size(theme::SMALL)
                        .text_color(theme::GREEN)
                        .child("shared with you"),
                )
            });
        let grant =
            script.granted.then(|| {
                let row = div().flex().w_full().items_center().gap(theme::ITEM_GAP);
                if self.store.removing.as_deref() == Some(script.id.as_str()) {
                    let removed = script.clone();
                    row.child(div().flex_1().min_w_0().child(
                        "Remove it from your account? You'll need a new code to get it back.",
                    ))
                    .child(widgets::button(
                        script_id("remove", script),
                        "Remove",
                        true,
                        Self::act(move || Action::RemoveGrant(Box::new(removed.clone())), cx),
                    ))
                    .child(widgets::button(
                        script_id("keep", script),
                        "Keep",
                        true,
                        cx.listener(|app, _: &ClickEvent, _, cx| {
                            app.store.removing = None;
                            cx.notify();
                        }),
                    ))
                } else {
                    let id = script.id.clone();
                    row.child(
                        widgets::button(
                            script_id("start-remove", script),
                            "Remove from my account",
                            true,
                            cx.listener(move |app, _: &ClickEvent, _, cx| {
                                app.store.removing = Some(id.clone());
                                cx.notify();
                            }),
                        )
                        .h(px(18.0)),
                    )
                }
            });
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(theme::LINE_GAP)
            .child(tags)
            .child(format!("{} online", script.online))
            .child(crate::store::price(script))
            .when(!script.description.is_empty(), |this| {
                this.child(widgets::weak(script.description.clone()))
            })
            .children(grant)
    }

    fn render_store_buttons(
        &self,
        script: &StoreScript,
        account: &Account,
        layout: (Pixels, bool),
        cx: &mut Context<Self>,
    ) -> Div {
        let (width, vertical) = layout;
        let Account {
            me,
            block,
            has_site,
        } = *account;
        let blocked = block
            .map(str::to_owned)
            .or_else(|| script.blocked(me.microcredits, me.min_balance_minutes));
        let add_funds = has_site && script.short_of_funds(me.microcredits, me.min_balance_minutes);
        let website = has_site.then(|| {
            let path = format!("/scripts/{}", script.url);
            widgets::icon_button(
                script_id("website", script),
                crate::store::ICON_WEBSITE,
                "Website",
                width,
                true,
                Self::act(move || Action::OpenSite(path.clone()), cx),
            )
            .tooltip(widgets::tooltip("Opens the script's page on the website."))
        });
        let action = if add_funds {
            let reason = script
                .blocked(me.microcredits, me.min_balance_minutes)
                .unwrap_or_default();
            widgets::icon_button(
                script_id("add-funds", script),
                crate::store::ICON_ADD_FUNDS,
                "Add funds",
                width,
                true,
                Self::act(|| Action::OpenSite("/credits".to_owned()), cx),
            )
            .tooltip(widgets::tooltip(reason))
        } else {
            let run = script.clone();
            widgets::icon_button(
                script_id("run", script),
                ICON_START,
                "Run",
                width,
                blocked.is_none(),
                Self::act(move || Action::Run(Box::new(run.clone())), cx),
            )
            .when_some(blocked, |this, reason| {
                this.tooltip(widgets::tooltip(reason))
            })
        };
        widgets::button_group(vertical)
            .children(website)
            .child(action)
    }

    fn render_store_card(
        &self,
        script: &StoreScript,
        account: &Account,
        full: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let button_width = widgets::icon_button_width(
            window,
            &[
                (crate::store::ICON_WEBSITE, "Website"),
                (ICON_START, "Run"),
                (crate::store::ICON_ADD_FUNDS, "Add funds"),
            ],
        );
        let buttons = 1 + usize::from(account.has_site);
        let placement = Placement::of(full - theme::ITEM_GAP, buttons, button_width);
        let details = self.render_store_details(script, cx);
        let group = self.render_store_buttons(
            script,
            account,
            (button_width, placement == Placement::Stacked),
            cx,
        );
        let card = widgets::card().flex().gap(theme::ITEM_GAP);
        match placement {
            Placement::Below => card
                .flex_col()
                .items_start()
                .gap(theme::LINE_GAP)
                .child(details.w_full())
                .child(group),
            _ => card
                .items_end()
                .child(details.flex_1().min_w_0())
                .child(group),
        }
    }

    pub(super) fn render_store(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.auth.me().is_none() {
            return self.render_store_signed_out(cx);
        }
        if !self.store.loaded && !self.store.loading && self.store.error.is_none() {
            self.fetch_scripts();
        }
        let ui = self
            .store_page
            .get_or_insert_with(|| Ui::new(&self.store, window, cx));
        ui.sync(&self.store, window, cx);
        let (search, code, cell) = (ui.search.clone(), ui.code.clone(), ui.width.clone());
        let width = cell.get();
        let (picker, target) = self.render_run_target(cx);
        let block = self.run_block(target.as_ref());
        let Some(me) = self.auth.me().cloned() else {
            return div().into_any_element();
        };
        let has_site = !self.api.config().site_url.is_empty();
        let full = width - px(24.0);
        let account = Account {
            me: &me,
            block: block.as_deref(),
            has_site,
        };
        let cards: Vec<Div> = self
            .store
            .visible()
            .map(|script| self.render_store_card(script, &account, full, window, cx))
            .collect();
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(theme::LINE_GAP)
            .child(measured_width(cell))
            .child(picker)
            .child(self.render_store_filters(&me, &search, width, cx))
            .child(
                self.render_store_redeem(&code, width, cx)
                    .mb(theme::LINE_GAP),
            )
            .children(self.store.error.clone().map(error_label))
            .when(
                self.store.loaded && self.store.scripts.is_empty() && self.store.error.is_none(),
                |this| this.child("No scripts available."),
            )
            .child(
                div()
                    .id("store-scripts")
                    .flex_1()
                    .min_h_0()
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
pub(super) fn fake_script(title: &str, price: i64) -> StoreScript {
    StoreScript {
        id: title.to_lowercase().replace(' ', "-"),
        url: title.to_lowercase().replace(' ', "-"),
        title: title.into(),
        description: String::new(),
        categories: Vec::new(),
        stage: "stable".into(),
        price,
        revision: Some(3),
        cover_url: None,
        can_access: true,
        author: "volcren".into(),
        online: 0,
        community: false,
        granted: false,
        queueable: false,
    }
}

#[cfg(test)]
pub(super) fn fake_target(pid: u32) -> Target {
    serde_json::from_value(serde_json::json!({
        "client": {
            "pid": pid,
            "exe": "client.exe",
            "title": "the client",
            "is_64bit": true,
            "loaded": "agent",
        },
        "location": { "prefix": "/nonexistent/prefix", "wine": "wine", "env": [] },
    }))
    .expect("a target")
}

#[cfg(test)]
fn fake_scripts() -> Vec<StoreScript> {
    let free = StoreScript {
        description: "Plays the fishing minigame from start to finish and banks the rewards."
            .into(),
        categories: vec!["fishing".into()],
        online: 41,
        ..fake_script("Tempoross", 0)
    };
    let paid = StoreScript {
        description:
            "Mines and banks ore at a dozen sites, with world hopping when rocks are taken.".into(),
        categories: vec!["mining".into()],
        online: 7,
        ..fake_script("Miner", 180_000)
    };
    let community = StoreScript {
        description: "Keeps the braziers lit and fletches kindling between rounds.".into(),
        categories: vec!["firemaking".into()],
        community: true,
        author: "someone".into(),
        stage: "beta".into(),
        online: 2,
        ..fake_script("Brazier helper", 70_000)
    };
    let granted = StoreScript {
        description: "A private build shared with a share code.".into(),
        granted: true,
        community: true,
        author: "friend".into(),
        stage: "alpha".into(),
        ..fake_script("Shared farm run", 0)
    };
    let expensive = StoreScript {
        description: "Needs more credits than the account holds.".into(),
        can_access: false,
        online: 1,
        ..fake_script("Raid helper", 100_000_000)
    };
    let unreleased = StoreScript {
        revision: None,
        can_access: false,
        stage: "draft".into(),
        ..fake_script("Unreleased", 120_000)
    };
    vec![free, paid, community, granted, expensive, unreleased]
}

#[cfg(test)]
fn populated(app: &mut LoaderApp) {
    app.tab = Tab::Store;
    app.store.scripts = fake_scripts();
    app.store.loaded = true;
    app.targets = vec![fake_target(4242)];
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("store", populated),
        ("store-no-client", |app| {
            populated(app);
            app.targets.clear();
        }),
        ("store-tester-busy", |app| {
            populated(app);
            app.auth = Auth::SignedIn(Me {
                role: "tester".into(),
                ..super::snapshots::me()
            });
            app.store.community = true;
            app.store.loading = true;
            app.store.redeeming = true;
            app.store.code = "ab".repeat(32);
            app.store.search = "e".into();
            app.store.removing = Some("shared-farm-run".into());
        }),
        ("store-redeem-error", |app| {
            populated(app);
            app.store.code = "not a code".into();
            app.store.redeem_result = Some((
                false,
                "A share code is 64 characters: the digits 0-9 and the letters a-f.".into(),
            ));
            app.store.category = "mining".into();
        }),
        ("store-error", |app| {
            app.tab = Tab::Store;
            app.store.loaded = true;
            app.store.error = Some("Couldn't reach the server: connection refused".into());
        }),
        ("store-empty", |app| {
            app.tab = Tab::Store;
            app.store.loaded = true;
        }),
    ]
}
