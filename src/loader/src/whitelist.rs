use std::rc::Rc;

use gpui_kit::component::Sizable;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::{
    AnchoredPositionMode, AnyElement, App, Div, Entity, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Pixels, Size, StatefulInteractiveElement, Styled, Window, anchored,
    deferred, div, point, prelude::FluentBuilder, px,
};
use injector::http;

use crate::api::Config;
use crate::ui::{theme, widgets};

const ACTIONS_WIDTH: Pixels = px(90.0);
const INPUT_WIDTH: Pixels = px(300.0);
const DIALOG_PADDING: Pixels = px(8.0);
const MAX_USER_ENTRIES: usize = 200;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Whitelist {
    builtin: Vec<String>,
    user: Vec<String>,
}

pub fn normalize(input: &str) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter a host or URL.".into());
    }
    let (wildcard, rest) = match input.strip_prefix("*.") {
        Some(rest) => (true, rest),
        None => (false, input),
    };
    let rest = if rest.contains("://") {
        rest.to_owned()
    } else {
        format!("https://{rest}")
    };
    if rest.contains('*') || input.contains(char::is_whitespace) {
        return Err("Use a host like example.com, or *.example.com for its subdomains.".into());
    }
    let uri: ureq::http::Uri = rest
        .parse()
        .map_err(|_| "That isn't a valid host or URL.".to_owned())?;
    let host = crate::http::host_of(&uri)
        .map_err(|_| "Only http and https URLs can be whitelisted.".to_owned())?;
    let label = |label: &str| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    let valid = host.starts_with('[') || host.split('.').all(label);
    if host.is_empty() || !valid {
        return Err("That isn't a valid host.".into());
    }
    if wildcard && !host.contains('.') {
        return Err("A wildcard needs a domain, like *.example.com.".into());
    }
    Ok(if wildcard { format!("*.{host}") } else { host })
}

fn matches(entry: &str, host: &str) -> bool {
    match entry.strip_prefix("*.") {
        Some(domain) => host
            .strip_suffix(domain)
            .is_some_and(|label| label.len() > 1 && label.ends_with('.')),
        None => entry == host,
    }
}

impl Whitelist {
    pub fn new(user: Vec<String>, config: &Config) -> Self {
        let mut builtin = Vec::new();
        for url in [
            &config.api_url,
            &config.supabase_url,
            &config.site_url,
            &config.prices_url,
        ] {
            if let Ok(uri) = url.parse::<ureq::http::Uri>()
                && let Ok(host) = crate::http::host_of(&uri)
                && !builtin.contains(&host)
            {
                builtin.push(host);
            }
        }
        Self {
            builtin,
            user: user
                .into_iter()
                .filter_map(|entry| normalize(&entry).ok())
                .collect(),
        }
    }

    pub fn allows(&self, host: &str) -> bool {
        self.builtin
            .iter()
            .chain(&self.user)
            .any(|entry| matches(entry, host))
    }

    pub fn user(&self) -> &[String] {
        &self.user
    }

    pub fn add(&mut self, input: &str) -> Result<(), String> {
        let entry = normalize(input)?;
        if self.builtin.contains(&entry) || self.user.contains(&entry) {
            return Err(format!("{entry} is already on the list."));
        }
        if self.user.len() >= MAX_USER_ENTRIES {
            return Err(format!(
                "The list holds at most {MAX_USER_ENTRIES} entries of your own."
            ));
        }
        self.user.push(entry);
        Ok(())
    }

    pub fn remove(&mut self, entry: &str) {
        self.user.retain(|have| have != entry);
    }

    pub fn perform(&self, request: &http::Request) -> Result<http::Response, String> {
        crate::http::perform(self, request)
    }
}

#[derive(Default)]
struct Draft {
    input: String,
    error: Option<String>,
}

#[derive(Default)]
pub struct Page {
    draft: Option<Draft>,
    deleting: Option<String>,
}

pub enum Click {
    Add,
    Delete(String),
    Save,
    Cancel,
}

pub type OnClick = Rc<dyn Fn(Click, &mut Window, &mut App)>;

impl Page {
    pub fn draft_input(&self) -> Option<&str> {
        self.draft.as_ref().map(|draft| draft.input.as_str())
    }

    pub fn set_draft_input(&mut self, input: String) {
        if let Some(draft) = &mut self.draft {
            draft.input = input;
        }
    }

    pub fn click(&mut self, click: Click, list: &mut Whitelist) {
        match click {
            Click::Add => {
                if self.draft.is_none() {
                    self.draft = Some(Draft::default());
                }
            }
            Click::Delete(host) => {
                if self.draft.is_some() {
                    return;
                }
                if self.deleting.as_deref() == Some(host.as_str()) {
                    list.remove(&host);
                    self.deleting = None;
                } else {
                    self.deleting = Some(host);
                }
            }
            Click::Save => {
                if let Some(draft) = &mut self.draft {
                    match list.add(&draft.input) {
                        Ok(()) => self.draft = None,
                        Err(why) => draft.error = Some(why),
                    }
                }
            }
            Click::Cancel => self.draft = None,
        }
    }

    pub fn render(
        &self,
        list: &Whitelist,
        input: &Entity<InputState>,
        window: &Window,
        on_click: OnClick,
    ) -> (Div, Option<AnyElement>) {
        let enabled = self.draft.is_none();
        let add = on_click.clone();
        let page = div()
            .flex()
            .flex_col()
            .gap(theme::LINE_GAP)
            .child(widgets::heading("HTTP whitelist"))
            .child(widgets::weak(
                "Scripts can only make HTTP requests to these hosts. Redirects to any other host are refused.",
            ))
            .child(widgets::warning("Don't add hosts you don't trust."))
            .child(
                div().flex().pt(px(6.0)).child(widgets::button(
                    "whitelist-add",
                    "Add URL",
                    enabled,
                    move |_, window, cx| add(Click::Add, window, cx),
                )),
            )
            .child(div().pt(px(6.0)).child(self.render_list(list, enabled, &on_click)));
        let dialog = self
            .draft
            .as_ref()
            .map(|draft| render_dialog(draft, input, window.viewport_size(), on_click));
        (page, dialog)
    }

    fn render_list(&self, list: &Whitelist, enabled: bool, on_click: &OnClick) -> Div {
        let rows = list
            .builtin
            .iter()
            .map(|host| (host, true))
            .chain(list.user.iter().map(|host| (host, false)))
            .enumerate()
            .map(|(index, (host, builtin))| {
                let action = if builtin {
                    widgets::weak("Built in")
                        .id(("whitelist-builtin", index))
                        .tooltip(widgets::tooltip("Built-in hosts are always allowed"))
                        .into_any_element()
                } else {
                    let armed = self.deleting.as_deref() == Some(host.as_str());
                    let (on_click, host) = (on_click.clone(), host.clone());
                    widgets::button(
                        ("whitelist-delete", index),
                        if armed { "Sure?" } else { "Delete" },
                        enabled,
                        move |_, window, cx| on_click(Click::Delete(host.clone()), window, cx),
                    )
                    .into_any_element()
                };
                widgets::body_row(("whitelist-row", index), index)
                    .child(
                        div()
                            .id(("whitelist-host", index))
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .tooltip(widgets::tooltip(host.clone()))
                            .child(host.clone()),
                    )
                    .child(actions_cell().child(action))
            });
        div()
            .flex()
            .flex_col()
            .child(
                widgets::table_row()
                    .child(widgets::strong("Host").flex_1())
                    .child(actions_cell()),
            )
            .children(rows)
    }
}

fn actions_cell() -> Div {
    div()
        .flex()
        .flex_none()
        .justify_end()
        .w(ACTIONS_WIDTH)
}

fn render_dialog(
    draft: &Draft,
    input: &Entity<InputState>,
    viewport: Size<Pixels>,
    on_click: OnClick,
) -> AnyElement {
    let (save, cancel, escape) = (on_click.clone(), on_click.clone(), on_click);
    let card = div()
        .id("whitelist-dialog")
        .occlude()
        .flex()
        .flex_col()
        .gap(theme::LINE_GAP)
        .w(INPUT_WIDTH + DIALOG_PADDING * 2.0)
        .p(DIALOG_PADDING)
        .bg(theme::BG_FLOAT)
        .border_1()
        .border_color(theme::BORDER)
        .child(widgets::heading("Add URL"))
        .child(widgets::separator())
        .child("A host or URL scripts may send requests to. Use *.example.com for every subdomain.")
        .child(Input::new(input).small().w(INPUT_WIDTH))
        .when_some(draft.error.clone(), |this, error| this.child(widgets::error(error)))
        .child(
            div()
                .pt(px(6.0))
                .flex()
                .justify_end()
                .gap(theme::ITEM_GAP)
                .child(widgets::button("whitelist-cancel", "Cancel", true, move |_, window, cx| {
                    cancel(Click::Cancel, window, cx)
                }))
                .child(widgets::button("whitelist-save", "Save", true, move |_, window, cx| {
                    save(Click::Save, window, cx)
                })),
        );
    deferred(
        anchored()
            .position(point(px(0.0), px(0.0)))
            .position_mode(AnchoredPositionMode::Window)
            .child(
                div()
                    .id("whitelist-backdrop")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            api_url: "https://api.volcren.com".into(),
            supabase_url: "https://db.volcren.com".into(),
            anon_key: String::new(),
            site_url: "https://volcren.com".into(),
            prices_url: "https://prices.example.org/api".into(),
        }
    }

    #[test]
    fn normalizes_hosts_and_urls() {
        for (input, host) in [
            ("example.com", "example.com"),
            ("  Example.COM ", "example.com"),
            ("https://Example.com:8080/a/b?c=d", "example.com"),
            ("http://example.com./", "example.com"),
            ("*.Example.com", "*.example.com"),
            ("https://user:pw@example.com/", "example.com"),
            ("127.0.0.1", "127.0.0.1"),
            ("localhost", "localhost"),
        ] {
            assert_eq!(normalize(input).as_deref(), Ok(host), "{input:?}");
        }
        for bad in [
            "",
            "  ",
            "ftp://example.com",
            "exa mple.com",
            "ex*mple.com",
            "*.com",
            "*",
            "https://",
            "a..b",
            "-.",
            "a/b c",
        ] {
            assert!(normalize(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn builtin_hosts_come_from_the_config_and_are_allowed() {
        let list = Whitelist::new(Vec::new(), &config());
        for host in [
            "api.volcren.com",
            "db.volcren.com",
            "volcren.com",
            "prices.example.org",
        ] {
            assert!(list.allows(host), "{host}");
        }
        assert!(!list.allows("example.com"));
        assert!(!list.allows("evil-volcren.com"));
        assert!(!list.allows("volcren.com.evil.com"));
        assert!(
            !list.allows("www.volcren.com"),
            "an exact host does not cover subdomains"
        );
    }

    #[test]
    fn wildcards_match_subdomains_only() {
        let mut list = Whitelist::new(Vec::new(), &config());
        list.add("*.example.com").unwrap();
        assert!(list.allows("a.example.com"));
        assert!(list.allows("a.b.example.com"));
        assert!(!list.allows("example.com"));
        assert!(!list.allows("badexample.com"));
        assert!(!list.allows(".example.com"));
        assert!(!list.allows("example.com.evil.com"));
    }

    #[test]
    fn adds_and_removes_user_entries_but_never_builtin_ones() {
        let mut list = Whitelist::new(Vec::new(), &config());
        assert_eq!(list.add("https://Example.com/x"), Ok(()));
        assert!(list.allows("example.com"));
        assert!(list.add("example.com").is_err(), "twice");
        assert!(list.add("volcren.com").is_err(), "already built in");
        assert!(list.add("prices.example.org").is_err(), "already built in");
        list.remove("volcren.com");
        assert!(list.allows("volcren.com"));
        list.remove("example.com");
        assert!(!list.allows("example.com"));
        assert!(list.user().is_empty());
    }

    #[test]
    fn saved_entries_that_are_no_longer_valid_are_dropped() {
        let list = Whitelist::new(
            vec!["Example.com".into(), "not a host".into(), "*.com".into()],
            &config(),
        );
        assert_eq!(list.user(), ["example.com"]);
    }

    #[test]
    fn the_dialog_keeps_a_bad_entry_and_deleting_asks_first() {
        let mut list = Whitelist::new(Vec::new(), &config());
        let mut page = Page::default();
        page.click(Click::Add, &mut list);
        page.set_draft_input("not a host".into());
        page.click(Click::Save, &mut list);
        assert!(page.draft.as_ref().is_some_and(|draft| draft.error.is_some()));
        page.set_draft_input("example.com".into());
        page.click(Click::Save, &mut list);
        assert!(page.draft.is_none());
        assert_eq!(list.user(), ["example.com"]);
        page.click(Click::Delete("example.com".into()), &mut list);
        assert_eq!(list.user(), ["example.com"]);
        page.click(Click::Delete("example.com".into()), &mut list);
        assert!(list.user().is_empty());
    }

    #[test]
    fn an_env_override_changes_the_builtin_hosts() {
        let mut local = config();
        local.api_url = "http://localhost:3000".into();
        assert!(Whitelist::new(Vec::new(), &local).allows("localhost"));
        assert!(!Whitelist::new(Vec::new(), &config()).allows("localhost"));
    }
}
