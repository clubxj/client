use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::Sizable;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::{
    AnchoredPositionMode, AnyElement, App, Div, Entity, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Pixels, Size, StatefulInteractiveElement, Styled, Window,
    anchored, deferred, div, point, prelude::FluentBuilder, px,
};
use injector::{Proxy, ProxyStatus, proxy_state};
use serde::{Deserialize, Serialize};

use crate::accounts::{create_key, existing_key, open_with, seal_with};
use crate::ui::{theme, widgets};

const FILE_NAME: &str = "proxies.bin";
const AAD: &[u8] = b"volcren-proxies-v1";
const MAX_PROXIES: usize = 200;
const FIELD_LEN: usize = 255;
const ACTIONS_WIDTH: Pixels = px(150.0);
const ADDRESS_WIDTH: Pixels = px(240.0);
const LOGIN_WIDTH: Pixels = px(140.0);
const LABEL_WIDTH: Pixels = px(80.0);
const INPUT_WIDTH: Pixels = px(300.0);
const DIALOG_PADDING: Pixels = px(8.0);
pub const POLL_EVERY: Duration = Duration::from_secs(2);
pub const HINT: &str = "Pick a proxy before you log in. Changing it shuts down the client's open connections, so the client reconnects through the new one.";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    pub name: String,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
}

impl Saved {
    pub fn proxy(&self) -> Proxy {
        Proxy {
            host: self.host.clone(),
            port: self.port,
            username: self.username.clone(),
            password: self.password.clone(),
        }
    }

    fn address(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Name,
    Host,
    Port,
    Username,
    Password,
}

impl Field {
    pub const ALL: [Field; 5] = [
        Field::Name,
        Field::Host,
        Field::Port,
        Field::Username,
        Field::Password,
    ];

    fn label(self) -> &'static str {
        match self {
            Field::Name => "Name",
            Field::Host => "Host",
            Field::Port => "Port",
            Field::Username => "Username",
            Field::Password => "Password",
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            Field::Name => "how you recognise it",
            Field::Host => "proxy.example.com or an IP",
            Field::Port => "1080",
            Field::Username | Field::Password => "optional",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Default)]
struct Draft {
    editing: Option<String>,
    values: [String; 5],
    error: Option<String>,
}

impl Draft {
    fn of(saved: &Saved) -> Self {
        Self {
            editing: Some(saved.name.clone()),
            values: [
                saved.name.clone(),
                saved.host.clone(),
                saved.port.to_string(),
                saved.username.clone(),
                saved.password.clone(),
            ],
            error: None,
        }
    }

    fn value(&self, field: Field) -> &str {
        self.values[field.index()].trim()
    }
}

pub enum Click {
    Add,
    Edit(String),
    Delete(String),
    Save,
    Cancel,
}

pub enum Change {
    Saved { was: Option<String>, saved: Saved },
}

pub type OnClick = Rc<dyn Fn(Click, &mut Window, &mut App)>;

#[derive(Default)]
pub struct Proxies {
    saved: Vec<Saved>,
    locked: Option<String>,
    error: Option<String>,
    draft: Option<Draft>,
    deleting: Option<String>,
}

fn path() -> Option<PathBuf> {
    crate::paths::project_dirs().map(|dirs| dirs.data_dir().join(FILE_NAME))
}

fn load() -> Result<Vec<Saved>, String> {
    let path = path().ok_or("no data folder on this system")?;
    let sealed = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let key = existing_key()?.ok_or("the encryption key is missing from the system keyring")?;
    let plain = open_with(&key, AAD, &sealed)
        .ok_or("the file can't be decrypted with this machine's key (changed or corrupt)")?;
    serde_json::from_slice(&plain).map_err(|e| e.to_string())
}

fn store(saved: &[Saved]) -> Result<(), String> {
    let path = path().ok_or("no data folder on this system")?;
    let key = create_key()?;
    let plain = serde_json::to_vec(saved).map_err(|e| e.to_string())?;
    crate::paths::write_atomic(&path, &seal_with(&key, AAD, &plain)?)
}

fn checked(draft: &Draft, taken: impl Fn(&str) -> bool) -> Result<Saved, String> {
    let name = draft.value(Field::Name);
    let host = draft.value(Field::Host);
    let username = draft.value(Field::Username);
    let password = &draft.values[Field::Password.index()];
    if name.is_empty() {
        return Err("Give the proxy a name.".into());
    }
    if taken(name) {
        return Err(format!("A proxy named {name} already exists."));
    }
    if host.is_empty() || host.contains(char::is_whitespace) {
        return Err("Enter the proxy's host, like proxy.example.com or 203.0.113.7.".into());
    }
    let port = match draft.value(Field::Port).parse::<u16>() {
        Ok(port) if port > 0 => port,
        _ => return Err("The port is a number from 1 to 65535.".into()),
    };
    if username.is_empty() && !password.is_empty() {
        return Err("A password needs a username.".into());
    }
    for (field, value) in [
        (Field::Host, host),
        (Field::Username, username),
        (Field::Password, password.as_str()),
    ] {
        if value.len() > FIELD_LEN {
            return Err(format!(
                "The {} is longer than {FIELD_LEN} bytes.",
                field.label().to_lowercase()
            ));
        }
    }
    Ok(Saved {
        name: name.to_owned(),
        host: host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned(),
        port,
        username: username.to_owned(),
        password: password.clone(),
    })
}

impl Proxies {
    pub fn load() -> Self {
        match load() {
            Ok(saved) => Self::with(saved),
            Err(why) => Self {
                locked: Some(why),
                ..Self::default()
            },
        }
    }

    pub fn with(saved: Vec<Saved>) -> Self {
        Self {
            saved,
            ..Self::default()
        }
    }

    pub fn saved(&self) -> &[Saved] {
        &self.saved
    }

    pub fn named(&self, name: &str) -> Option<&Saved> {
        self.saved.iter().find(|saved| saved.name == name)
    }

    pub fn draft_value(&self, field: Field) -> Option<&str> {
        self.draft
            .as_ref()
            .map(|draft| draft.values[field.index()].as_str())
    }

    pub fn set_draft_value(&mut self, field: Field, value: String) {
        if let Some(draft) = &mut self.draft {
            draft.values[field.index()] = value;
        }
    }

    fn persist(&mut self) -> Result<(), String> {
        let result = store(&self.saved).map_err(|why| format!("Couldn't save the proxies: {why}"));
        self.error = result.as_ref().err().cloned();
        result
    }

    pub fn click(&mut self, click: Click, in_use: &dyn Fn(&str) -> Option<u32>) -> Option<Change> {
        if let Some(why) = &self.locked {
            self.error = Some(format!("The saved proxies can't be read: {why}"));
            return None;
        }
        match click {
            Click::Add => {
                if self.saved.len() >= MAX_PROXIES {
                    self.error = Some(format!("The list holds at most {MAX_PROXIES} proxies."));
                } else if self.draft.is_none() {
                    self.draft = Some(Draft::default());
                }
                None
            }
            Click::Edit(name) => {
                if self.draft.is_none()
                    && let Some(saved) = self.named(&name)
                {
                    self.draft = Some(Draft::of(saved));
                }
                None
            }
            Click::Delete(name) => {
                if self.draft.is_some() {
                    return None;
                }
                if let Some(pid) = in_use(&name) {
                    self.deleting = None;
                    self.error = Some(format!(
                        "{name} is in use by client {pid}. Pick another proxy for it first."
                    ));
                    return None;
                }
                if self.deleting.as_deref() == Some(name.as_str()) {
                    let kept = self.saved.clone();
                    self.saved.retain(|saved| saved.name != name);
                    if self.persist().is_err() {
                        self.saved = kept;
                    }
                    self.deleting = None;
                } else {
                    self.deleting = Some(name);
                }
                None
            }
            Click::Save => self.save(),
            Click::Cancel => {
                self.draft = None;
                None
            }
        }
    }

    fn save(&mut self) -> Option<Change> {
        let draft = self.draft.as_mut()?;
        let editing = draft.editing.clone();
        let taken = |name: &str| {
            editing.as_deref() != Some(name) && self.saved.iter().any(|saved| saved.name == name)
        };
        let saved = match checked(draft, taken) {
            Ok(saved) => saved,
            Err(why) => {
                draft.error = Some(why);
                return None;
            }
        };
        let kept = self.saved.clone();
        match editing
            .as_deref()
            .and_then(|name| self.saved.iter().position(|s| s.name == name))
        {
            Some(index) => self.saved[index] = saved.clone(),
            None => self.saved.push(saved.clone()),
        }
        if let Err(why) = self.persist() {
            self.saved = kept;
            if let Some(draft) = &mut self.draft {
                draft.error = Some(why);
            }
            return None;
        }
        self.draft = None;
        Some(Change::Saved {
            was: editing,
            saved,
        })
    }

    pub fn render(
        &self,
        inputs: &[Entity<InputState>; 5],
        window: &Window,
        on_click: OnClick,
    ) -> (Div, Option<AnyElement>) {
        let enabled = self.draft.is_none() && self.locked.is_none();
        let add = on_click.clone();
        let page = div()
            .flex()
            .flex_col()
            .gap(theme::LINE_GAP)
            .pt(px(12.0))
            .child(widgets::heading("Proxies"))
            .child(widgets::weak(
                "SOCKS5 proxies a client's connections can go through. Pick one on a client's page. They're stored encrypted with your accounts' key.",
            ))
            .when_some(self.locked.clone(), |this, why| {
                this.child(widgets::error(format!("The saved proxies can't be read: {why}")))
            })
            .when_some(self.error.clone(), |this, error| this.child(widgets::error(error)))
            .child(
                div().flex().pt(px(6.0)).child(widgets::button(
                    "proxy-add",
                    "Add proxy",
                    enabled,
                    move |_, window, cx| add(Click::Add, window, cx),
                )),
            )
            .child(div().pt(px(6.0)).child(self.render_list(enabled, &on_click)));
        let dialog = self
            .draft
            .as_ref()
            .map(|draft| render_dialog(draft, inputs, window.viewport_size(), on_click));
        (page, dialog)
    }

    fn render_list(&self, enabled: bool, on_click: &OnClick) -> Div {
        let rows = self.saved.iter().enumerate().map(|(index, saved)| {
            let armed = self.deleting.as_deref() == Some(saved.name.as_str());
            let (edit, delete) = (on_click.clone(), on_click.clone());
            let (edit_name, delete_name) = (saved.name.clone(), saved.name.clone());
            table_row()
                .when(index % 2 == 0, |this| this.bg(theme::BG_FLOAT))
                .child(
                    div()
                        .id(("proxy-name", index))
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .tooltip(widgets::tooltip(saved.name.clone()))
                        .child(saved.name.clone()),
                )
                .child(
                    div()
                        .id(("proxy-address", index))
                        .w(ADDRESS_WIDTH)
                        .truncate()
                        .tooltip(widgets::tooltip(saved.address()))
                        .child(saved.address()),
                )
                .child(
                    div()
                        .w(LOGIN_WIDTH)
                        .truncate()
                        .child(if saved.username.is_empty() {
                            widgets::weak("no login").into_any_element()
                        } else {
                            saved.username.clone().into_any_element()
                        }),
                )
                .child(
                    actions_cell()
                        .gap(theme::ITEM_GAP)
                        .child(widgets::button(
                            ("proxy-edit", index),
                            "Edit",
                            enabled,
                            move |_, window, cx| edit(Click::Edit(edit_name.clone()), window, cx),
                        ))
                        .child(widgets::button(
                            ("proxy-delete", index),
                            if armed { "Sure?" } else { "Delete" },
                            enabled,
                            move |_, window, cx| {
                                delete(Click::Delete(delete_name.clone()), window, cx)
                            },
                        )),
                )
        });
        div()
            .flex()
            .flex_col()
            .child(
                table_row()
                    .child(widgets::strong("Name").flex_1())
                    .child(widgets::strong("Address").w(ADDRESS_WIDTH))
                    .child(widgets::strong("Username").w(LOGIN_WIDTH))
                    .child(actions_cell()),
            )
            .children(rows)
    }
}

fn table_row() -> Div {
    div()
        .flex()
        .items_center()
        .h(theme::ROW_HEIGHT)
        .gap(theme::ITEM_GAP)
}

fn actions_cell() -> Div {
    div().flex().flex_none().justify_end().w(ACTIONS_WIDTH)
}

fn render_dialog(
    draft: &Draft,
    inputs: &[Entity<InputState>; 5],
    viewport: Size<Pixels>,
    on_click: OnClick,
) -> AnyElement {
    let (save, cancel, escape) = (on_click.clone(), on_click.clone(), on_click);
    let fields = Field::ALL.iter().map(|&field| {
        div()
            .flex()
            .items_center()
            .gap(theme::ITEM_GAP)
            .child(div().w(LABEL_WIDTH).child(field.label()))
            .child(Input::new(&inputs[field.index()]).small().w(INPUT_WIDTH))
    });
    let title = if draft.editing.is_some() {
        "Edit proxy"
    } else {
        "Add proxy"
    };
    let card = div()
        .id("proxy-dialog")
        .occlude()
        .flex()
        .flex_col()
        .gap(theme::LINE_GAP)
        .w(LABEL_WIDTH + INPUT_WIDTH + theme::ITEM_GAP + DIALOG_PADDING * 2.0)
        .p(DIALOG_PADDING)
        .bg(theme::BG_FLOAT)
        .border_1()
        .border_color(theme::BORDER)
        .child(widgets::heading(title))
        .child(widgets::separator())
        .child("A SOCKS5 proxy. Leave the username and password empty if it doesn't need a login.")
        .children(fields)
        .when_some(draft.error.clone(), |this, error| {
            this.child(widgets::error(error))
        })
        .child(
            div()
                .pt(px(6.0))
                .flex()
                .justify_end()
                .gap(theme::ITEM_GAP)
                .child(widgets::button(
                    "proxy-cancel",
                    "Cancel",
                    true,
                    move |_, window, cx| cancel(Click::Cancel, window, cx),
                ))
                .child(widgets::button(
                    "proxy-save",
                    "Save",
                    true,
                    move |_, window, cx| save(Click::Save, window, cx),
                )),
        );
    deferred(
        anchored()
            .position(point(px(0.0), px(0.0)))
            .position_mode(AnchoredPositionMode::Window)
            .child(
                div()
                    .id("proxy-backdrop")
                    .occlude()
                    .w(viewport.width)
                    .h(viewport.height)
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme::alpha(theme::BG_DARK, 0.5))
                    .capture_key_down(move |event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" {
                            escape(Click::Cancel, window, cx);
                            cx.stop_propagation();
                        }
                    })
                    .child(card),
            ),
    )
    .with_priority(1)
    .into_any_element()
}

#[derive(Default)]
pub struct Assigned {
    names: HashMap<u32, String>,
    status: HashMap<u32, Result<ProxyStatus, String>>,
    polled_at: Option<Instant>,
}

pub enum Shown {
    Off,
    Waiting,
    NoAgent,
    On(ProxyStatus),
    Problem(String),
}

impl Assigned {
    pub fn name(&self, pid: u32) -> Option<&str> {
        self.names.get(&pid).map(String::as_str)
    }

    pub fn user_of(&self, name: &str) -> Option<u32> {
        self.names
            .iter()
            .find(|(_, assigned)| *assigned == name)
            .map(|(pid, _)| *pid)
    }

    pub fn users_of(&self, name: &str) -> Vec<u32> {
        self.names
            .iter()
            .filter(|(_, assigned)| *assigned == name)
            .map(|(pid, _)| *pid)
            .collect()
    }

    pub fn assign(&mut self, pid: u32, name: Option<String>) {
        match name {
            Some(name) => self.names.insert(pid, name),
            None => self.names.remove(&pid),
        };
        self.status.remove(&pid);
    }

    pub fn rename(&mut self, was: &str, now: &str) {
        for assigned in self.names.values_mut().filter(|name| *name == was) {
            *assigned = now.to_owned();
        }
    }

    pub fn retain(&mut self, alive: impl Fn(u32) -> bool) {
        self.names.retain(|pid, _| alive(*pid));
        self.status.retain(|pid, _| alive(*pid));
    }

    pub fn answered(&mut self, pid: u32, result: Result<ProxyStatus, String>) {
        self.status.insert(pid, result);
    }

    pub fn due(&mut self, now: Instant) -> Option<Duration> {
        match self.polled_at {
            Some(at) if now.duration_since(at) < POLL_EVERY => {
                Some(POLL_EVERY - now.duration_since(at))
            }
            _ => {
                self.polled_at = Some(now);
                None
            }
        }
    }

    pub fn shown(&self, pid: u32, agent_loaded: bool) -> Shown {
        if !self.names.contains_key(&pid) {
            return Shown::Off;
        }
        match self.status.get(&pid) {
            None => Shown::Waiting,
            Some(Err(why)) => Shown::Problem(why.clone()),
            Some(Ok(_)) if !agent_loaded => Shown::NoAgent,
            Some(Ok(status)) if !status.applied() => Shown::Waiting,
            Some(Ok(status)) => match status.state {
                proxy_state::ON => Shown::On(*status),
                proxy_state::UNHOOKED => Shown::Problem(
                    "The agent couldn't hook the client's connections, so they don't go through the proxy. Close the client before you log in.".into(),
                ),
                proxy_state::UNREADABLE => Shown::Problem(
                    "The agent couldn't read the proxy setting, so it blocks every outside connection.".into(),
                ),
                _ => Shown::Waiting,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(name: &str) -> Saved {
        Saved {
            name: name.into(),
            host: "proxy.example.com".into(),
            port: 1080,
            username: String::new(),
            password: String::new(),
        }
    }

    fn draft(values: [&str; 5]) -> Draft {
        Draft {
            editing: None,
            values: values.map(str::to_owned),
            error: None,
        }
    }

    #[test]
    fn a_draft_is_checked_before_it_is_saved() {
        let free = |_: &str| false;
        let ok = checked(&draft([" Home ", " 203.0.113.7 ", "1080", "", ""]), free).unwrap();
        assert_eq!(
            (ok.name.as_str(), ok.host.as_str(), ok.port),
            ("Home", "203.0.113.7", 1080)
        );
        let v6 = checked(&draft(["v6", "[2001:db8::1]", "1080", "u", "p"]), free).unwrap();
        assert_eq!(v6.host, "2001:db8::1");
        assert_eq!(v6.address(), "[2001:db8::1]:1080");
        for bad in [
            ["", "host", "1080", "", ""],
            ["a", "", "1080", "", ""],
            ["a", "bad host", "1080", "", ""],
            ["a", "host", "0", "", ""],
            ["a", "host", "70000", "", ""],
            ["a", "host", "port", "", ""],
            ["a", "host", "1080", "", "secret"],
        ] {
            assert!(checked(&draft(bad), free).is_err(), "{bad:?}");
        }
        assert!(checked(&draft(["Home", "h", "1", "", ""]), |name| name == "Home").is_err());
    }

    #[test]
    fn a_proxy_in_use_is_not_deleted() {
        let mut proxies = Proxies::with(vec![saved("Home")]);
        let used = |name: &str| (name == "Home").then_some(4120);
        proxies.click(Click::Delete("Home".into()), &used);
        proxies.click(Click::Delete("Home".into()), &used);
        assert_eq!(proxies.saved().len(), 1);
        assert!(proxies.error.as_ref().is_some_and(|e| e.contains("4120")));
    }

    #[test]
    fn a_client_shows_its_proxy_only_once_the_agent_applied_it() {
        let mut assigned = Assigned::default();
        assert!(matches!(assigned.shown(1, true), Shown::Off));
        assigned.assign(1, Some("Home".into()));
        assert!(matches!(assigned.shown(1, true), Shown::Waiting));
        let mut status = ProxyStatus {
            written: 4,
            adopted: 2,
            state: proxy_state::ON,
            ..ProxyStatus::default()
        };
        assigned.answered(1, Ok(status));
        assert!(matches!(assigned.shown(1, true), Shown::Waiting));
        assert!(matches!(assigned.shown(1, false), Shown::NoAgent));
        status.adopted = 4;
        assigned.answered(1, Ok(status));
        assert!(matches!(assigned.shown(1, true), Shown::On(_)));
        status.state = proxy_state::UNHOOKED;
        assigned.answered(1, Ok(status));
        assert!(matches!(assigned.shown(1, true), Shown::Problem(_)));
        assigned.rename("Home", "Office");
        assert_eq!(assigned.name(1), Some("Office"));
        assert_eq!(assigned.users_of("Office"), [1]);
        assigned.retain(|pid| pid != 1);
        assert!(matches!(assigned.shown(1, true), Shown::Off));
    }
}
