use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard};

use gpui_kit::component::Sizable;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    AnyElement, App, AppContext, ClickEvent, Context, Div, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Pixels, StatefulInteractiveElement, Styled,
    Subscription, Window, canvas, div, prelude::FluentBuilder, px,
};
use injector::bank_pin::{hash_name, parse_pin};
use ring::{
    aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey},
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};

use crate::session::{hex_decode, hex_encode};
use crate::ui::theme::{self, BUTTON_PADDING, INTERACT_HEIGHT, ITEM_GAP, LINE_GAP, ROW_HEIGHT};
use crate::ui::widgets;

const KEYRING_SERVICE: &str = "volcren-loader";
const KEYRING_USER: &str = "accounts-key";
const FILE_NAME: &str = "accounts.bin";
const AAD: &[u8] = b"volcren-accounts-v1";
const PIN_LEN: usize = 4;
const HASH_SHOWN: usize = 12;
const MARGIN: Pixels = px(16.0);
const INPUT_WIDTH: Pixels = px(300.0);
const MIN_TEXT_WIDTH: Pixels = px(90.0);
const ACCOUNT_REQUIRED: &str = "Enter the account name.";
const PIN_INVALID: &str = "The bank pin is exactly 4 digits.";
const LAMP_SKILLS: [&str; 24] = [
    "Agility",
    "Attack",
    "Construction",
    "Cooking",
    "Crafting",
    "Defence",
    "Farming",
    "Firemaking",
    "Fishing",
    "Fletching",
    "Herblore",
    "Hitpoints",
    "Hunter",
    "Magic",
    "Mining",
    "Prayer",
    "Ranged",
    "Runecraft",
    "Sailing",
    "Slayer",
    "Smithing",
    "Strength",
    "Thieving",
    "Woodcutting",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub hash: String,
    pub pin: String,
    #[serde(default)]
    pub lamp_skill: String,
}

pub(crate) fn seal(key: &[u8; 32], plain: &[u8]) -> Result<Vec<u8>, String> {
    seal_with(key, AAD, plain)
}

pub(crate) fn seal_with(key: &[u8; 32], aad: &[u8], plain: &[u8]) -> Result<Vec<u8>, String> {
    let sealing = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).map_err(|e| e.to_string())?);
    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|e| e.to_string())?;
    let mut data = plain.to_vec();
    sealing
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(aad),
            &mut data,
        )
        .map_err(|e| e.to_string())?;
    let mut out = nonce.to_vec();
    out.extend(data);
    Ok(out)
}

pub(crate) fn open(key: &[u8; 32], sealed: &[u8]) -> Option<Vec<u8>> {
    open_with(key, AAD, sealed)
}

pub(crate) fn open_with(key: &[u8], aad: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    let (nonce, rest) = sealed.split_at_checked(NONCE_LEN)?;
    let opening = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).ok()?);
    let mut data = rest.to_vec();
    let plain_len = opening
        .open_in_place(
            Nonce::try_assume_unique_for_key(nonce).ok()?,
            Aad::from(aad),
            &mut data,
        )
        .ok()?
        .len();
    data.truncate(plain_len);
    Some(data)
}

fn encode(key: &[u8; 32], entries: &[Entry]) -> Result<Vec<u8>, String> {
    seal(
        key,
        &serde_json::to_vec(entries).map_err(|e| e.to_string())?,
    )
}

fn decode(key: &[u8; 32], sealed: &[u8]) -> Result<Vec<Entry>, String> {
    let plain = open(key, sealed)
        .ok_or("the file can't be decrypted with this machine's key (changed or corrupt)")?;
    serde_json::from_slice(&plain).map_err(|e| e.to_string())
}

fn path() -> Option<PathBuf> {
    crate::paths::project_dirs().map(|dirs| dirs.data_dir().join(FILE_NAME))
}

fn keyring_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())
}

fn parse_key(text: &str) -> Result<[u8; 32], String> {
    hex_decode(text)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| "the stored encryption key is malformed".to_owned())
}

pub(crate) fn existing_key() -> Result<Option<[u8; 32]>, String> {
    match keyring_entry()?.get_password() {
        Ok(text) => parse_key(&text).map(Some),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("the system keyring isn't available ({e})")),
    }
}

pub(crate) fn create_key() -> Result<[u8; 32], String> {
    let entry = keyring_entry()?;
    match entry.get_password() {
        Ok(text) => parse_key(&text),
        Err(keyring::Error::NoEntry) => {
            let mut fresh = [0u8; 32];
            SystemRandom::new()
                .fill(&mut fresh)
                .map_err(|e| e.to_string())?;
            entry
                .set_password(&hex_encode(&fresh))
                .map_err(|e| e.to_string())?;
            Ok(fresh)
        }
        Err(e) => Err(format!("the system keyring isn't available ({e})")),
    }
}

fn load() -> Result<Vec<Entry>, String> {
    let path = path().ok_or("no data folder on this system")?;
    let sealed = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let key = existing_key()?.ok_or("the encryption key is missing from the system keyring")?;
    decode(&key, &sealed)
}

fn store(entries: &[Entry]) -> Result<(), String> {
    let path = path().ok_or("no data folder on this system")?;
    let key = create_key()?;
    crate::paths::write_atomic(&path, &encode(&key, entries)?)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Pin,
}

#[derive(Clone, Copy)]
enum Column {
    Name,
    Hash,
    Pin,
}

struct Editing {
    index: usize,
    field: Field,
    text: String,
}

impl Column {
    fn field(self) -> Option<Field> {
        match self {
            Column::Name => Some(Field::Name),
            Column::Hash => None,
            Column::Pin => Some(Field::Pin),
        }
    }
}

#[derive(Default)]
struct Draft {
    target: Option<usize>,
    name: String,
    account: String,
    pin: String,
    error: Option<String>,
}

#[derive(Default)]
pub struct Accounts {
    entries: Vec<Entry>,
    working: Vec<Entry>,
    locked: Option<String>,
    error: Option<String>,
    draft: Option<Draft>,
    editing: Option<Editing>,
    deleting: Option<usize>,
    reading: bool,
}

impl Accounts {
    pub fn load() -> Self {
        match load() {
            Ok(entries) => Self::with_entries(entries),
            Err(why) => Self {
                locked: Some(why),
                ..Self::default()
            },
        }
    }

    pub fn with_entries(entries: Vec<Entry>) -> Self {
        Self {
            working: entries.clone(),
            entries,
            ..Self::default()
        }
    }

    fn saved(&self) -> Result<&[Entry], String> {
        match &self.locked {
            Some(why) => Err(format!("the saved accounts can't be read: {why}")),
            None => Ok(&self.entries),
        }
    }

    pub fn pin_for(&self, hash: &str) -> Result<Option<String>, String> {
        Ok(self
            .saved()?
            .iter()
            .find(|entry| entry.hash == hash)
            .map(|entry| entry.pin.clone()))
    }

    pub fn lamp_skill_for(&self, hash: &str) -> Result<Option<String>, String> {
        Ok(self
            .saved()?
            .iter()
            .find(|entry| entry.hash == hash && !entry.lamp_skill.is_empty())
            .map(|entry| entry.lamp_skill.clone()))
    }

    fn persist(&mut self) {
        self.error = store(&self.entries)
            .err()
            .map(|why| format!("Couldn't save the accounts: {why}"));
    }

    fn submit(&mut self, draft: &Draft) -> Result<(), String> {
        let (name, account) = (draft.name.trim(), draft.account.trim());
        if let Some(entry) = draft.target.and_then(|index| self.working.get_mut(index)) {
            if account.is_empty() {
                return Err(ACCOUNT_REQUIRED.into());
            }
            entry.hash = hash_name(account);
            return Ok(());
        }
        if name.is_empty() {
            return Err("Give the entry a name.".into());
        }
        if account.is_empty() {
            return Err(ACCOUNT_REQUIRED.into());
        }
        if parse_pin(&draft.pin).is_none() {
            return Err(PIN_INVALID.into());
        }
        let entry = Entry {
            name: name.to_owned(),
            hash: hash_name(account),
            pin: draft.pin.clone(),
            lamp_skill: String::new(),
        };
        self.entries.push(entry.clone());
        self.working.push(entry);
        self.persist();
        Ok(())
    }

    fn apply(&mut self, edit: &Editing) -> Result<(), String> {
        let text = edit.text.trim();
        let Some(entry) = self.working.get_mut(edit.index) else {
            return Ok(());
        };
        match edit.field {
            Field::Name if text.is_empty() => return Err("The name can't be empty.".into()),
            Field::Name => entry.name = text.to_owned(),
            Field::Pin if parse_pin(text).is_none() => return Err(PIN_INVALID.into()),
            Field::Pin => entry.pin = text.to_owned(),
        }
        Ok(())
    }

    pub fn client_name(&mut self, result: Result<Option<String>, String>) {
        self.reading = false;
        let Some(draft) = &mut self.draft else { return };
        match result {
            Ok(Some(name)) => {
                draft.account = name.replace('_', " ");
                draft.error = None;
            }
            Ok(None) => draft.error = Some("Log in to the game first.".into()),
            Err(why) => {
                draft.error = Some(format!("Couldn't read the name from the client: {why}"))
            }
        }
    }

    fn add(&mut self) {
        self.draft = Some(Draft::default());
    }

    fn start_edit(&mut self, index: usize, field: Field) {
        self.finish_edit(true);
        let Some(entry) = self.working.get(index) else {
            return;
        };
        let text = match field {
            Field::Name => entry.name.clone(),
            Field::Pin => entry.pin.clone(),
        };
        self.editing = Some(Editing { index, field, text });
        self.deleting = None;
    }

    fn finish_edit(&mut self, commit: bool) {
        if let Some(edit) = self.editing.take()
            && commit
        {
            self.error = self.apply(&edit).err();
        }
    }

    fn change_account(&mut self, index: usize) {
        self.draft = Some(Draft {
            target: Some(index),
            ..Draft::default()
        });
        self.editing = None;
        self.deleting = None;
    }

    fn save(&mut self, index: usize) {
        if let Some(entry) = self.working.get(index) {
            self.entries[index] = entry.clone();
            self.persist();
        }
    }

    fn delete(&mut self, index: usize) {
        if self.deleting == Some(index) {
            self.entries.remove(index);
            self.working.remove(index);
            self.deleting = None;
            self.editing = None;
            self.persist();
        } else {
            self.deleting = Some(index);
        }
    }

    fn set_lamp_skill(&mut self, index: usize, skill: String) {
        if let Some(entry) = self.working.get_mut(index) {
            entry.lamp_skill = skill;
        }
    }

    fn close_dialog(&mut self, save: bool) {
        let Some(mut draft) = self.draft.take() else {
            return;
        };
        if save && let Err(why) = self.submit(&draft) {
            draft.error = Some(why);
            self.draft = Some(draft);
        }
    }

    fn escape(&mut self) -> bool {
        if self.draft.is_some() {
            self.close_dialog(false);
        } else if self.editing.is_some() {
            self.finish_edit(false);
        } else {
            return false;
        }
        true
    }
}

fn read(accounts: &RwLock<Accounts>) -> RwLockReadGuard<'_, Accounts> {
    accounts.read().unwrap_or_else(PoisonError::into_inner)
}

fn pin_allowed(text: &str, _: &mut App) -> bool {
    text.chars().count() <= PIN_LEN && text.chars().all(|c| c.is_ascii_digit())
}

fn sync(input: &Entity<InputState>, text: &str, window: &mut Window, cx: &mut App) {
    if input.read(cx).value().as_ref() != text {
        let text = text.to_owned();
        input.update(cx, |input, cx| input.set_value(text, window, cx));
    }
}

type DraftField = fn(&mut Draft) -> &mut String;

pub struct Ui {
    name: Entity<InputState>,
    account: Entity<InputState>,
    pin: Entity<InputState>,
    edit_name: Entity<InputState>,
    edit_pin: Entity<InputState>,
    width: Rc<Cell<Pixels>>,
    _subscriptions: Vec<Subscription>,
}

impl Ui {
    pub fn new<V: 'static>(
        accounts: &Arc<RwLock<Accounts>>,
        home: FocusHandle,
        active: impl Fn(&V) -> bool + 'static,
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> Self {
        let input =
            |placeholder: &'static str, pin: bool, window: &mut Window, cx: &mut Context<V>| {
                cx.new(|cx| {
                    let input = InputState::new(window, cx).placeholder(placeholder);
                    if pin {
                        input.validate(pin_allowed)
                    } else {
                        input
                    }
                })
            };
        let name = input("e.g. Ironman (it's just for you)", false, window, cx);
        let account = input("In-game name (not stored)", false, window, cx);
        let pin = input("4 digits", true, window, cx);
        pin.update(cx, |pin, cx| pin.set_masked(true, window, cx));
        let edit_name = input("name", false, window, cx);
        let edit_pin = input("4 digits", true, window, cx);
        let mut subscriptions = Vec::new();
        let draft_fields: [(&Entity<InputState>, DraftField); 3] = [
            (&name, |draft| &mut draft.name),
            (&account, |draft| &mut draft.account),
            (&pin, |draft| &mut draft.pin),
        ];
        for (input, field) in draft_fields {
            let accounts = accounts.clone();
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                move |_, input, event: &InputEvent, _, cx| {
                    if let InputEvent::Change = event
                        && let Some(draft) = &mut crate::script::write(&accounts).draft
                    {
                        *field(draft) = input.read(cx).value().to_string();
                    }
                },
            ));
        }
        for (input, field) in [(&edit_name, Field::Name), (&edit_pin, Field::Pin)] {
            let accounts = accounts.clone();
            let home = home.clone();
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                move |_, input, event: &InputEvent, window, cx| {
                    let mut accounts = crate::script::write(&accounts);
                    let Some(edit) = accounts.editing.as_mut().filter(|edit| edit.field == field)
                    else {
                        return;
                    };
                    match event {
                        InputEvent::Change => edit.text = input.read(cx).value().to_string(),
                        InputEvent::PressEnter { .. } => {
                            accounts.finish_edit(true);
                            window.focus(&home, cx);
                        }
                        InputEvent::Blur if !input.focus_handle(cx).is_focused(window) => {
                            accounts.finish_edit(true);
                        }
                        _ => return,
                    }
                    cx.notify();
                },
            ));
        }
        let keys = accounts.clone();
        subscriptions.push(cx.observe_keystrokes(move |view, event, window, cx| {
            if event.action.is_none()
                && event.keystroke.key == "escape"
                && active(view)
                && crate::script::write(&keys).escape()
            {
                window.focus(&home, cx);
                cx.notify();
            }
        }));
        Self {
            name,
            account,
            pin,
            edit_name,
            edit_pin,
            width: Rc::new(Cell::new(window.viewport_size().width)),
            _subscriptions: subscriptions,
        }
    }

    fn edit_input(&self, field: Field) -> &Entity<InputState> {
        match field {
            Field::Name => &self.edit_name,
            Field::Pin => &self.edit_pin,
        }
    }

    fn sync(&self, accounts: &Accounts, window: &mut Window, cx: &mut App) {
        if let Some(draft) = &accounts.draft {
            sync(&self.name, &draft.name, window, cx);
            sync(&self.account, &draft.account, window, cx);
            sync(&self.pin, &draft.pin, window, cx);
        }
        if let Some(edit) = &accounts.editing {
            sync(self.edit_input(edit.field), &edit.text, window, cx);
        }
    }
}

fn change<V: 'static>(
    accounts: &Arc<RwLock<Accounts>>,
    cx: &mut Context<V>,
    apply: impl Fn(&mut Accounts) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let accounts = accounts.clone();
    cx.listener(move |_, _: &ClickEvent, _, cx| {
        apply(&mut crate::script::write(&accounts));
        cx.notify();
    })
}

const ADD_ICON: &str = "\u{f234}";

pub fn render<V: 'static>(
    accounts: &Arc<RwLock<Accounts>>,
    ui: &Ui,
    client: bool,
    on_read: impl Fn(&mut V, &mut Context<V>) + 'static,
    window: &mut Window,
    cx: &mut Context<V>,
) -> AnyElement {
    let state = read(accounts);
    ui.sync(&state, window, cx);
    let width = ui.width.clone();
    let view = cx.entity_id();
    let probe = canvas(
        move |bounds, _, cx| {
            if width.replace(bounds.size.width) != bounds.size.width {
                cx.notify(view);
            }
        },
        |_, (), _, _| {},
    )
    .absolute()
    .inset_0();
    let page = div()
        .flex_1()
        .min_h_0()
        .relative()
        .flex()
        .flex_col()
        .child(probe);
    if let Some(why) = &state.locked {
        return page
            .gap(LINE_GAP)
            .child(widgets::error(format!("Saved accounts can't be opened: {why}.")))
            .child("Nothing is shown or saved so the file isn't overwritten. Fix the keyring, or delete accounts.bin in the Volcren data folder to start over.")
            .into_any_element();
    }
    let enabled = state.draft.is_none();
    let body = div()
        .flex()
        .flex_col()
        .items_start()
        .gap(LINE_GAP)
        .when(!enabled, |this| this.opacity(0.5))
        .child(widgets::icon_button(
            "add-account",
            ADD_ICON,
            "Add account",
            widgets::icon_button_width(window, &[(ADD_ICON, "Add account")]),
            enabled,
            change(accounts, cx, Accounts::add),
        ))
        .child(
            widgets::weak("Encrypted on this machine. Account names are not stored.")
                .mt(theme::ITEM_GAP),
        )
        .children(state.error.clone().map(widgets::error))
        .child(
            div()
                .mt(MARGIN)
                .w_full()
                .child(if state.working.is_empty() {
                    widgets::weak("No accounts saved yet.").into_any_element()
                } else {
                    list(accounts, &state, ui, enabled, window, cx).into_any_element()
                }),
        );
    let dialog = state.draft.as_ref().map(|draft| {
        dialog(
            accounts,
            draft,
            ui,
            client && !state.reading,
            on_read,
            window,
            cx,
        )
    });
    page.child(body).children(dialog).into_any_element()
}

fn list<V: 'static>(
    accounts: &Arc<RwLock<Accounts>>,
    state: &Accounts,
    ui: &Ui,
    enabled: bool,
    window: &mut Window,
    cx: &mut Context<V>,
) -> Div {
    let button_width =
        widgets::widest(window, &["Save", "Delete", "Sure?"], theme::BODY) + BUTTON_PADDING * 2.0;
    let row_actions = button_width * 2.0 + ITEM_GAP;
    let stacked = ui.width.get() < MIN_TEXT_WIDTH * 4.0 + ITEM_GAP * 5.0 + row_actions;
    let actions_width = if stacked { button_width } else { row_actions };
    let row_height = if stacked {
        ROW_HEIGHT.max(INTERACT_HEIGHT * 2.0 + LINE_GAP)
    } else {
        ROW_HEIGHT
    };
    let text_column = || {
        div()
            .flex_1()
            .min_w(MIN_TEXT_WIDTH)
            .h_full()
            .flex()
            .items_center()
            .overflow_hidden()
    };
    let row = |height: Pixels| div().flex().w_full().h(height).gap(ITEM_GAP);
    let header = ["Name", "Account hash", "Bank pin", "Lamp skill"]
        .into_iter()
        .fold(row(ROW_HEIGHT), |header, title| {
            header.child(text_column().child(widgets::strong(title)))
        })
        .child(div().flex_none().w(actions_width));
    let rows = state.working.iter().enumerate().map(|(index, entry)| {
        let cells = [Column::Name, Column::Hash, Column::Pin].map(|column| {
            let field = column.field();
            if let Some(edit) = state
                .editing
                .as_ref()
                .filter(|edit| edit.index == index && Some(edit.field) == field)
            {
                return text_column().child(
                    div()
                        .w_full()
                        .child(Input::new(ui.edit_input(edit.field)).small()),
                );
            }
            let (id, shown, hover) = match column {
                Column::Name => ("name", entry.name.clone(), "Click to rename".to_owned()),
                Column::Hash => (
                    "hash",
                    format!("{}…", &entry.hash[..HASH_SHOWN.min(entry.hash.len())]),
                    format!("{}\nClick to change the account", entry.hash),
                ),
                Column::Pin => (
                    "pin",
                    "*".repeat(entry.pin.len()),
                    "Click to show and change the pin".to_owned(),
                ),
            };
            let cell = div()
                .id(id)
                .size_full()
                .flex()
                .items_center()
                .text_color(theme::FG_DARK)
                .child(div().w_full().truncate().child(shown))
                .tooltip(widgets::tooltip(hover));
            let cell = if enabled {
                let accounts = accounts.clone();
                let input = field.map(|field| ui.edit_input(field).clone());
                cell.cursor_pointer()
                    .hover(|style| style.text_color(theme::FG))
                    .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                        let mut accounts = crate::script::write(&accounts);
                        match field {
                            None => accounts.change_account(index),
                            Some(field) => accounts.start_edit(index, field),
                        }
                        if let (Some(input), Some(edit)) = (&input, &accounts.editing) {
                            let text = edit.text.clone();
                            input.update(cx, |input, cx| {
                                input.set_value(text, window, cx);
                                input.focus(window, cx);
                            });
                        }
                        cx.notify();
                    }))
            } else {
                cell
            };
            text_column().child(cell)
        });
        let app = cx.entity().downgrade();
        let skills = if enabled {
            std::iter::once(("None", String::new()))
                .chain(
                    LAMP_SKILLS
                        .into_iter()
                        .map(|skill| (skill, skill.to_owned())),
                )
                .map(|(label, skill)| {
                    let (accounts, app) = (accounts.clone(), app.clone());
                    let selected = skill == entry.lamp_skill;
                    widgets::Choice::new(label, selected, move |_, cx| {
                        crate::script::write(&accounts).set_lamp_skill(index, skill.clone());
                        app.update(cx, |_, cx| cx.notify()).ok();
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        let shown = if entry.lamp_skill.is_empty() {
            "None".to_owned()
        } else {
            entry.lamp_skill.clone()
        };
        let lamp = text_column().child(
            div()
                .id("lamp-skill")
                .child(widgets::combo(("lamp-skill", index), shown, skills))
                .tooltip(widgets::tooltip(
                    "What a random event's lamp is spent on. None keeps the lamp.",
                )),
        );
        let armed = state.deleting == Some(index);
        let changed = state.entries.get(index) != Some(entry);
        let save = widgets::button(
            "save",
            "Save",
            enabled && changed,
            change(accounts, cx, move |accounts| accounts.save(index)),
        )
        .min_w(button_width)
        .justify_center();
        let delete = widgets::button(
            "delete",
            if armed { "Sure?" } else { "Delete" },
            enabled,
            change(accounts, cx, move |accounts| accounts.delete(index)),
        )
        .min_w(button_width)
        .justify_center();
        let actions = div()
            .flex_none()
            .w(actions_width)
            .flex()
            .when(stacked, |this| {
                this.flex_col().justify_center().gap(LINE_GAP)
            })
            .when(!stacked, |this| {
                this.items_center().justify_end().gap(ITEM_GAP)
            })
            .child(save)
            .child(delete);
        widgets::striped(row(row_height).id(("account", index)), index)
            .children(cells)
            .child(lamp)
            .child(actions)
    });
    div()
        .flex()
        .flex_col()
        .w_full()
        .child(header)
        .children(rows)
}

fn dialog<V: 'static>(
    accounts: &Arc<RwLock<Accounts>>,
    draft: &Draft,
    ui: &Ui,
    can_read: bool,
    on_read: impl Fn(&mut V, &mut Context<V>) + 'static,
    window: &mut Window,
    cx: &mut Context<V>,
) -> Div {
    let changing = draft.target.is_some();
    let mut fields = vec![("Account", &ui.account)];
    if !changing {
        fields.insert(0, ("Name", &ui.name));
        fields.push(("Bank pin", &ui.pin));
    }
    let labels: Vec<&str> = fields.iter().map(|(label, _)| *label).collect();
    let label_width = widgets::widest(window, &labels, theme::BODY).ceil();
    let grid = fields.into_iter().fold(
        div().flex().flex_col().gap(ITEM_GAP),
        |grid, (label, input)| {
            grid.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(div().flex_none().w(label_width).child(label))
                    .child(div().w(INPUT_WIDTH).child(Input::new(input).small())),
            )
        },
    );
    let read_accounts = accounts.clone();
    let load = widgets::button(
        "read-client-name",
        "Load account from client",
        can_read,
        cx.listener(move |view, _: &ClickEvent, _, cx| {
            crate::script::write(&read_accounts).reading = true;
            on_read(view, cx);
            cx.notify();
        }),
    )
    .tooltip(widgets::tooltip(if can_read {
        "Fill in the name of the account logged in on the client you opened last"
    } else {
        "Open a client with the agent loaded in the sidebar, and log in"
    }));
    let decide = div()
        .flex()
        .gap(ITEM_GAP)
        .child(widgets::button(
            "cancel",
            "Cancel",
            true,
            change(accounts, cx, |accounts| accounts.close_dialog(false)),
        ))
        .child(widgets::button(
            "save",
            "Save",
            true,
            change(accounts, cx, |accounts| accounts.close_dialog(true)),
        ));
    let card = div()
        .id("accounts-dialog")
        .occlude()
        .flex()
        .flex_col()
        .bg(theme::BG_FLOAT)
        .border_1()
        .border_color(theme::BORDER)
        .child(
            div()
                .px(px(8.0))
                .py(LINE_GAP)
                .child(widgets::heading(if changing {
                    "Change account"
                } else {
                    "Add account"
                })),
        )
        .child(div().h(px(1.0)).bg(theme::BORDER))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(LINE_GAP)
                .p(px(8.0))
                .child(grid)
                .children(draft.error.clone().map(widgets::error))
                .child(
                    div()
                        .mt(px(6.0))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(ITEM_GAP)
                        .child(load)
                        .child(decide),
                ),
        );
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .child(card)
}

#[cfg(test)]
impl Accounts {
    pub fn locked(why: &str) -> Self {
        Self {
            locked: Some(why.to_owned()),
            ..Self::default()
        }
    }
}

#[cfg(test)]
pub fn snapshot_editing(accounts: &RwLock<Accounts>) {
    let mut accounts = crate::script::write(accounts);
    accounts.working[1].lamp_skill = "Mining".into();
    accounts.start_edit(0, Field::Pin);
    accounts.deleting = Some(2);
}

#[cfg(test)]
pub fn snapshot_adding(accounts: &RwLock<Accounts>) {
    let mut accounts = crate::script::write(accounts);
    accounts.add();
    if let Some(draft) = &mut accounts.draft {
        draft.name = "Alt".into();
        draft.pin = "12".into();
        draft.error = Some(PIN_INVALID.into());
    }
}

#[cfg(test)]
pub fn snapshot_changing(accounts: &RwLock<Accounts>) {
    crate::script::write(accounts).change_account(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_round_trip_and_the_wrong_key_fails() {
        let entries = vec![Entry {
            name: "main".into(),
            hash: hash_name("a"),
            pin: "1234".into(),
            lamp_skill: "Slayer".into(),
        }];
        let sealed = encode(&[7; 32], &entries).unwrap();
        assert!(
            !sealed.windows(4).any(|w| w == b"1234"),
            "the pin must not appear in the file"
        );
        assert_eq!(decode(&[7; 32], &sealed).unwrap(), entries);
        assert!(decode(&[8; 32], &sealed).is_err());
        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(decode(&[7; 32], &tampered).is_err());
        assert!(decode(&[7; 32], &[1, 2, 3]).is_err());
    }

    #[test]
    fn a_file_from_before_lamp_skills_still_loads() {
        let old = br#"[{"name":"main","hash":"abc","pin":"1234"}]"#;
        let sealed = seal(&[7; 32], old).unwrap();
        let entries = decode(&[7; 32], &sealed).unwrap();
        assert_eq!(entries[0].lamp_skill, "");
    }

    #[test]
    fn the_lamp_skill_comes_from_what_is_saved() {
        let entry = |hash: &str, skill: &str| Entry {
            name: "main".into(),
            hash: hash.into(),
            pin: "1234".into(),
            lamp_skill: skill.into(),
        };
        let accounts = Accounts {
            entries: vec![entry("set", "Slayer"), entry("unset", "")],
            working: vec![entry("set", "Mining")],
            ..Accounts::default()
        };
        assert_eq!(accounts.lamp_skill_for("set"), Ok(Some("Slayer".into())));
        assert_eq!(accounts.lamp_skill_for("unset"), Ok(None));
        assert_eq!(accounts.lamp_skill_for("nobody"), Ok(None));
    }
}
