use gpui_kit::component::Sizable;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::scroll::ScrollableElement;

use super::*;
use crate::ui::{theme, widgets};

const TOGGLE_COLUMN: Pixels = px(240.0);
const TABS: [(AccountTab, &str); 2] = [
    (AccountTab::House, "House"),
    (AccountTab::Transportation, "Transportation"),
];

#[derive(Default)]
pub(super) struct AccountPage {
    client: Option<u32>,
    reading: bool,
    note: Option<(usize, Note)>,
    pushed: u64,
}

enum Note {
    Read(String),
    Outside,
    Failed(String),
}

fn two_columns(cells: Vec<Checkbox>) -> Div {
    let mut grid = div().flex().flex_col().gap(theme::LINE_GAP);
    let mut cells = cells.into_iter();
    while let Some(first) = cells.next() {
        grid = grid.child(
            div()
                .flex()
                .gap(theme::ITEM_GAP)
                .child(div().w(TOGGLE_COLUMN).child(first))
                .children(
                    cells
                        .next()
                        .map(|second| div().w(TOGGLE_COLUMN).child(second)),
                ),
        );
    }
    grid
}

impl LoaderApp {
    fn push_accounts(&mut self) {
        let (version, line) = {
            let accounts = crate::script::read(&self.accounts);
            (accounts.version(), accounts.settings_line())
        };
        if version == self.account_page.pushed {
            return;
        }
        self.account_page.pushed = version;
        for run in self.runs.iter_mut().filter(|r| r.is_running()) {
            if let Err(error) = run.send(|s| s.push_accounts(&line)) {
                run.error(error);
            }
        }
    }

    fn house_client(&self) -> Option<u32> {
        let loaded = |pid: &u32| self.client_loaded(*pid);
        self.account_page
            .client
            .filter(loaded)
            .or(self.focus_client.filter(loaded))
            .or_else(|| self.listed_clients().into_iter().find(loaded))
    }

    fn read_house(&mut self, index: usize, pid: u32) {
        let Some(target) = loaded_in(&self.targets, Some(pid)).cloned() else {
            return;
        };
        let Some(hash) = crate::script::read(&self.accounts)
            .saved_entry(index)
            .map(|entry| entry.hash.clone())
        else {
            return;
        };
        self.account_page.client = Some(pid);
        self.account_page.reading = true;
        self.account_page.note = None;
        self.background(move |settings| {
            let result =
                backend::state(&target, &settings).and_then(|state| match state.account_name {
                    None => Err("Log in to the game on that client first.".to_owned()),
                    Some(name) if injector::bank_pin::hash_name(&name) != hash => {
                        Err(format!("Client {pid} is logged in to a different account."))
                    }
                    Some(_) => backend::house(&target, &settings),
                });
            Event::HouseRead {
                index,
                hash,
                pid,
                result,
            }
        });
    }

    pub(super) fn house_read(
        &mut self,
        index: usize,
        hash: String,
        pid: u32,
        result: Result<injector::house::Survey, String>,
    ) {
        self.account_page.reading = false;
        let note = match result {
            Ok(survey) if !survey.inside => Note::Outside,
            Ok(survey) => {
                let mut accounts = crate::script::write(&self.accounts);
                if accounts.saved_entry(index).map(|entry| &entry.hash) != Some(&hash) {
                    return;
                }
                let found = survey.pieces.len();
                accounts.set_house(index, survey.pieces);
                Note::Read(match found {
                    0 => format!("Read from client {pid}: none of these are built."),
                    1 => format!("Read from client {pid}: 1 piece found."),
                    n => format!("Read from client {pid}: {n} pieces found."),
                })
            }
            Err(why) => Note::Failed(why),
        };
        self.account_page.note = Some((index, note));
    }

    pub(super) fn render_account_page(
        &mut self,
        index: usize,
        tab: AccountTab,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.push_accounts();
        let Some(entry) = crate::script::read(&self.accounts)
            .saved_entry(index)
            .cloned()
        else {
            return div().into_any_element();
        };
        let strip = TABS
            .iter()
            .fold(widgets::tab_strip(), |strip, &(shown, label)| {
                strip.child(widgets::tab_value(
                    ("account-tab", shown as usize),
                    label,
                    shown == tab,
                    cx.listener(move |app, _: &ClickEvent, _, cx| {
                        app.go(Page::Account(index, shown), None);
                        cx.notify();
                    }),
                ))
            });
        let content = match tab {
            AccountTab::House => self.render_house(index, &entry, cx),
            AccountTab::Transportation => self.render_account_transports(index, &entry, cx),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex_none()
                    .child(widgets::heading(entry.name.clone())),
            )
            .child(div().flex_none().mt(theme::LINE_GAP).child(strip))
            .child(divider())
            .child(
                div().flex_1().min_h_0().mt(px(8.0)).child(
                    div()
                        .flex()
                        .flex_col()
                        .child(widgets::tab_content().child(content))
                        .overflow_y_scrollbar()
                        .id(("account-scroll", tab as usize)),
                ),
            )
            .into_any_element()
    }

    fn render_house(
        &mut self,
        index: usize,
        entry: &crate::accounts::Entry,
        cx: &mut Context<Self>,
    ) -> Div {
        let chosen = self.house_client();
        let choices = self
            .listed_clients()
            .into_iter()
            .filter(|pid| self.client_loaded(*pid))
            .map(|pid| {
                let app = cx.entity().downgrade();
                widgets::Choice::new(
                    format!("Client {pid}"),
                    Some(pid) == chosen,
                    move |_, cx| {
                        app.update(cx, |app, cx| {
                            app.account_page.client = Some(pid);
                            cx.notify();
                        })
                        .ok();
                    },
                )
            })
            .collect();
        let shown = chosen.map_or_else(|| "No client".to_owned(), |pid| format!("Client {pid}"));
        let can_read = chosen.is_some() && !self.account_page.reading;
        let read = widgets::button(
            "read-house",
            "Read from client",
            can_read,
            cx.listener(move |app, _: &ClickEvent, _, cx| {
                if let Some(pid) = chosen {
                    app.read_house(index, pid);
                }
                cx.notify();
            }),
        )
        .tooltip(widgets::tooltip(if chosen.is_some() {
            "Ticks what the client's house has. Stand in your own house: a friend's reads as theirs."
        } else {
            "Load the agent into a client logged in to this account"
        }));
        let note = match &self.account_page.note {
            Some((shown, note)) if *shown == index => Some(match note {
                Note::Read(text) => widgets::weak(text.clone()),
                Note::Outside => widgets::warning(
                    "That client isn't in a house. Enter your own house on it and read again: only furniture inside can be read.",
                ),
                Note::Failed(why) => widgets::error(why.clone()),
            }),
            _ => None,
        };
        let pieces = injector::house::PIECES
            .iter()
            .map(|&(kind, label, _)| {
                let accounts = self.accounts.clone();
                Checkbox::new(kind)
                    .label(label)
                    .checked(entry.house.iter().any(|built| built == kind))
                    .on_click(cx.listener(move |_, on: &bool, _, cx| {
                        crate::script::write(&accounts).set_house_piece(index, kind, *on);
                        cx.notify();
                    }))
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(theme::LINE_GAP)
            .child(widgets::weak(
                "What this account's house has, so scripts can use it before they go inside. Scripts read the house's town, the Portal Nexus and a fancy or ornate jewellery box from the game themselves.",
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(theme::ITEM_GAP)
                    .child(widgets::combo("house-client", shown, choices))
                    .child(read)
                    .when(self.account_page.reading, |this| this.child(Spinner::new().small())),
            )
            .children(note)
            .child(div().mt(px(6.0)).child(two_columns(pieces)))
    }

    fn render_account_transports(
        &mut self,
        index: usize,
        entry: &crate::accounts::Entry,
        cx: &mut Context<Self>,
    ) -> Div {
        let toggles = injector::transports::TYPES
            .iter()
            .map(|&(name, label)| {
                let accounts = self.accounts.clone();
                Checkbox::new(name)
                    .label(label)
                    .checked(!entry.transports_off.iter().any(|off| off == name))
                    .on_click(cx.listener(move |_, on: &bool, _, cx| {
                        crate::script::write(&accounts).set_transport(index, name, *on);
                        cx.notify();
                    }))
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(theme::LINE_GAP)
            .child(widgets::weak(
                "What scripts may use to get around on this account. A script that needs one that is off stops with an error saying so.",
            ))
            .child(div().mt(px(6.0)).child(two_columns(toggles)))
    }

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
        self.push_accounts();
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
            |app: &mut LoaderApp, index, _| app.go(Page::Account(index, AccountTab::House), None),
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
        ..crate::accounts::Entry::default()
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
        ("account-house", |app| {
            saved(app);
            crate::accounts::snapshot_account(&app.accounts, &["glory", "pool"], &[]);
            app.page = Page::Account(0, AccountTab::House);
            app.account_page.note = Some((0, Note::Outside));
        }),
        ("account-transportation", |app| {
            saved(app);
            crate::accounts::snapshot_account(&app.accounts, &[], &["items"]);
            app.page = Page::Account(0, AccountTab::Transportation);
        }),
        ("accounts-locked", |app| {
            app.page = Page::Accounts;
            app.accounts = Arc::new(RwLock::new(crate::accounts::Accounts::locked(
                "the system keyring isn't available",
            )));
        }),
    ]
}
