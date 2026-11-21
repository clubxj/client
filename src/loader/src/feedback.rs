use std::{
    collections::HashMap,
    rc::Rc,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::Sizable;
use gpui_kit::{
    App, AppContext, ClickEvent, Div, Entity, InteractiveElement, ParentElement, Pixels,
    SharedString, Stateful, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder, px,
};
use injector::{GameState, Item};
use loader::backend::{self, Settings, Target};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::api::models::{RecentSession, Report};
use crate::layouts::{EQUIPMENT_CONTAINER as EQUIPMENT, INVENTORY_CONTAINER as INVENTORY};
use crate::ui::{theme, widgets};

pub const KEEP: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_LOG_BYTES: usize = 256 * 1024;
pub const MAX_DESCRIPTION_CHARS: usize = 2000;
pub const REFRESH_EVERY: Duration = Duration::from_secs(60);
const REDACTED: &str = "[account]";

pub struct Versions {
    pub agent: Option<String>,
    pub lib: Option<String>,
    pub gpu: Option<String>,
}

pub fn diagnostics(versions: &Versions, system: bool) -> Value {
    let mut map = Map::new();
    map.insert("loader".into(), json!(crate::update::VERSION));
    map.insert("host_api".into(), json!(injector::HOST_API));
    map.insert("protocol".into(), json!(injector::PROTOCOL_VERSION));
    map.insert("agent".into(), json!(versions.agent));
    map.insert("lib".into(), json!(versions.lib));
    map.insert(
        "platform".into(),
        json!(if cfg!(windows) { "native" } else { "wine" }),
    );
    if system {
        map.extend(system_info());
        map.insert("gpu".into(), json!(versions.gpu));
    }
    Value::Object(map)
}

pub fn system_info() -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("os".into(), json!(os_name()));
    map.insert("arch".into(), json!(std::env::consts::ARCH));
    if let Ok(cpus) = std::thread::available_parallelism() {
        map.insert("cpus".into(), json!(cpus.get()));
    }
    if let Some(cpu) = cpu_model() {
        map.insert("cpu".into(), json!(cpu));
    }
    if let Some(memory) = memory_gb() {
        map.insert("memory_gb".into(), json!(memory));
    }
    if let Some(wine) = wine_version() {
        map.insert("wine".into(), json!(wine));
    }
    map
}

fn os_name() -> String {
    if cfg!(target_os = "linux") {
        let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        let pretty = release
            .lines()
            .find_map(|line| line.strip_prefix("PRETTY_NAME="))
            .map(|name| name.trim_matches('"').to_owned());
        let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|k| k.trim().to_owned())
            .unwrap_or_default();
        return match pretty {
            Some(name) if !kernel.is_empty() => format!("{name} ({kernel})"),
            Some(name) => name,
            None if !kernel.is_empty() => format!("Linux {kernel}"),
            None => "Linux".to_owned(),
        };
    }
    if cfg!(windows)
        && let Some(version) = command_output("cmd", &["/c", "ver"])
    {
        return version;
    }
    std::env::consts::OS.to_owned()
}

fn cpu_model() -> Option<String> {
    if cfg!(target_os = "linux") {
        let info = std::fs::read_to_string("/proc/cpuinfo").ok()?;
        return info
            .lines()
            .find(|line| line.starts_with("model name"))
            .and_then(|line| line.split_once(':'))
            .map(|(_, model)| model.trim().to_owned());
    }
    std::env::var("PROCESSOR_IDENTIFIER").ok()
}

fn memory_gb() -> Option<f64> {
    if cfg!(target_os = "linux") {
        let info = std::fs::read_to_string("/proc/meminfo").ok()?;
        let kb: f64 = info
            .lines()
            .find(|line| line.starts_with("MemTotal:"))?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()?;
        return Some((kb / 1024.0 / 1024.0 * 10.0).round() / 10.0);
    }
    None
}

fn wine_version() -> Option<String> {
    if cfg!(windows) {
        return None;
    }
    command_output("wine", &["--version"])
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

pub fn names_of(state: &GameState) -> Vec<String> {
    [&state.account_name, &state.player_name]
        .into_iter()
        .flatten()
        .map(|name| name.trim().to_owned())
        .filter(|name| name.len() >= 2)
        .collect()
}

fn scrub(text: &str, names: &[String]) -> String {
    let mut keys: Vec<Vec<char>> = names
        .iter()
        .map(|name| normal(name))
        .filter(|key| key.len() >= 2)
        .collect();
    keys.sort();
    keys.dedup();
    keys.sort_by_key(|key| std::cmp::Reverse(key.len()));
    if keys.is_empty() {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        match keys.iter().find_map(|key| matches_at(&chars, i, key)) {
            Some(len) => {
                out.push_str(REDACTED);
                i += len;
            }
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

fn normal(name: &str) -> Vec<char> {
    let mut key = Vec::new();
    for c in name.trim().chars() {
        if is_separator(c) {
            if key.last() != Some(&' ') {
                key.push(' ');
            }
        } else {
            key.extend(c.to_lowercase());
        }
    }
    key
}

fn is_separator(c: char) -> bool {
    c == ' ' || c == '_' || c == '-' || c == '\u{a0}'
}

fn matches_at(text: &[char], at: usize, key: &[char]) -> Option<usize> {
    let mut i = at;
    for &k in key {
        if k == ' ' {
            let start = i;
            while i < text.len() && is_separator(text[i]) {
                i += 1;
            }
            if i == start {
                return None;
            }
        } else {
            let c = *text.get(i)?;
            if !c.to_lowercase().eq(std::iter::once(k)) {
                return None;
            }
            i += 1;
        }
    }
    Some(i - at)
}

pub fn snapshot(state: &GameState, containers: &[(i32, Vec<Item>)]) -> Value {
    let mut state = state.clone();
    state.account_name = None;
    state.player_name = None;
    let mut map = match serde_json::to_value(&state) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    };
    let items = |id: i32| {
        containers
            .iter()
            .find(|(container, _)| *container == id)
            .map(|(_, items)| {
                items
                    .iter()
                    .filter(|item| item.id >= 0)
                    .map(|item| json!({ "slot": item.slot, "id": item.id, "quantity": item.quantity }))
                    .collect::<Vec<_>>()
            })
    };
    map.insert("inventory".into(), json!(items(INVENTORY)));
    map.insert("equipment".into(), json!(items(EQUIPMENT)));
    map.insert("taken_at".into(), json!(crate::paths::unix_secs()));
    Value::Object(map)
}

pub fn read_snapshot(
    target: &Target,
    settings: &Settings,
    names: &mut Vec<String>,
) -> Option<Value> {
    let state = backend::state(target, settings).ok()?;
    names.extend(names_of(&state));
    let containers = backend::item_containers(target, settings).unwrap_or_default();
    Some(snapshot(&state, &containers))
}

pub fn scrubbed_log(log: &[String], names: &[String]) -> String {
    scrub(&tail(log, MAX_LOG_BYTES), names)
}

fn tail(lines: &[String], max_bytes: usize) -> String {
    let mut kept = Vec::new();
    let mut size = 0;
    for line in lines.iter().rev() {
        let add = line.len() + usize::from(!kept.is_empty());
        if size + add > max_bytes {
            break;
        }
        size += add;
        kept.push(line.as_str());
    }
    kept.reverse();
    kept.join("\n")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Record {
    pub session_id: String,
    pub title: String,
    pub log: String,
    pub snapshot: Option<Value>,
    pub ended_at: u64,
}

fn runs_dir() -> Option<PathBuf> {
    crate::paths::project_dirs().map(|dirs| dirs.data_dir().join("runs"))
}

fn record_path(dir: &Path, session_id: &str) -> Option<PathBuf> {
    crate::session::safe_id(session_id).map(|id| dir.join(format!("{id}.bin")))
}

pub fn store_record(record: &Record) -> Result<(), String> {
    let dir = runs_dir().ok_or("no data folder on this system")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    prune_in(&dir);
    let path = record_path(&dir, &record.session_id).ok_or("malformed session id")?;
    let key = crate::accounts::create_key()?;
    let plain = serde_json::to_vec(record).map_err(|e| e.to_string())?;
    let sealed = crate::accounts::seal(&key, &plain)?;
    crate::paths::write_atomic(&path, &sealed)
}

pub fn load_record(session_id: &str) -> Option<Record> {
    let path = record_path(&runs_dir()?, session_id)?;
    let sealed = std::fs::read(path).ok()?;
    let key = crate::accounts::existing_key().ok()??;
    let plain = crate::accounts::open(&key, &sealed)?;
    serde_json::from_slice(&plain).ok()
}

pub fn prune() {
    if let Some(dir) = runs_dir() {
        prune_in(&dir);
    }
}

fn prune_in(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > KEEP);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[derive(Default)]
pub struct Draft {
    pub description: String,
    pub pending_score: Option<u8>,
    pub sending: bool,
    pub open: bool,
    pub message: Option<Result<String, String>>,
}

#[derive(Default)]
pub struct Page {
    pub sessions: Vec<RecentSession>,
    pub loading: bool,
    pub error: Option<String>,
    pub fetched_at: Option<Instant>,
    pub drafts: HashMap<String, Draft>,
}

pub enum Action {
    Refresh,
    Score {
        session_id: String,
        score: u8,
    },
    Report {
        session_id: String,
        description: String,
    },
}

impl Page {
    pub fn stale(&self, now: Instant) -> bool {
        !self.loading
            && self
                .fetched_at
                .is_none_or(|at| now.duration_since(at) >= REFRESH_EVERY)
    }

    pub fn finish(&mut self, result: Result<Vec<RecentSession>, String>, now: Instant) {
        self.loading = false;
        self.fetched_at = Some(now);
        match result {
            Ok(sessions) => {
                self.sessions = sessions;
                self.error = None;
                let sessions = &self.sessions;
                self.drafts
                    .retain(|id, _| sessions.iter().any(|s| s.session_id == *id));
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub fn draft(&mut self, session_id: &str) -> &mut Draft {
        self.drafts.entry(session_id.to_owned()).or_default()
    }

    pub fn scored(&mut self, session_id: &str, result: Result<u8, String>) {
        let script = self
            .sessions
            .iter()
            .find(|s| s.session_id == session_id)
            .map(|s| s.script_id.clone());
        let draft = self.draft(session_id);
        draft.pending_score = None;
        match result {
            Ok(score) => {
                draft.message = Some(Ok(format!("Scored {score}/10.")));
                if let Some(script) = script {
                    for session in self.sessions.iter_mut().filter(|s| s.script_id == script) {
                        session.score = Some(score);
                    }
                }
            }
            Err(error) => draft.message = Some(Err(error)),
        }
    }

    pub fn reported(&mut self, session_id: &str, result: Result<(), String>) {
        let draft = self.draft(session_id);
        draft.sending = false;
        match result {
            Ok(()) => {
                draft.description.clear();
                draft.open = false;
                draft.message = Some(Ok("Report sent. Thank you!".to_owned()));
                if let Some(session) = self
                    .sessions
                    .iter_mut()
                    .find(|s| s.session_id == session_id)
                {
                    session.reports += 1;
                }
            }
            Err(error) => draft.message = Some(Err(error)),
        }
    }

    pub fn click(
        &mut self,
        session_id: &str,
        click: Click,
        include_system: &mut bool,
    ) -> Option<Action> {
        let saved_score = self
            .sessions
            .iter()
            .find(|s| s.session_id == session_id)
            .and_then(|s| s.score);
        let draft = self.draft(session_id);
        match click {
            Click::Score(score) => {
                if draft.pending_score.is_some() || saved_score == Some(score) {
                    return None;
                }
                draft.pending_score = Some(score);
                draft.message = None;
                Some(Action::Score {
                    session_id: session_id.to_owned(),
                    score,
                })
            }
            Click::Toggle => {
                draft.open = !draft.open;
                None
            }
            Click::IncludeSystem(include) => {
                *include_system = include;
                None
            }
            Click::Send => {
                if draft.sending {
                    return None;
                }
                draft.sending = true;
                draft.message = None;
                Some(Action::Report {
                    session_id: session_id.to_owned(),
                    description: draft.description.trim().to_owned(),
                })
            }
        }
    }

    pub fn render(
        &self,
        include_system: bool,
        running: &[String],
        descriptions: &HashMap<String, Entity<TextareaState>>,
        on_refresh: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        on_click: impl Fn(&str) -> OnClick,
    ) -> Div {
        let empty = Draft::default();
        let status = if self.loading && self.sessions.is_empty() {
            Some(div().child("Loading your sessions…"))
        } else if self.sessions.is_empty() {
            Some(widgets::weak(
                "No sessions in the last 24 hours. Run a script from the Scripts page first.",
            ))
        } else {
            None
        };
        let cards = self.sessions.iter().filter_map(|session| {
            let id = &session.session_id;
            let description = descriptions.get(id)?;
            let draft = self.drafts.get(id).unwrap_or(&empty);
            Some(
                widgets::card()
                    .w_full()
                    .mt(px(8.0))
                    .flex()
                    .flex_col()
                    .gap(theme::LINE_GAP)
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_x(theme::ITEM_GAP)
                            .gap_y(theme::LINE_GAP)
                            .child(widgets::strong(session.title.clone()))
                            .child(widgets::weak(format!("revision {}", session.revision)))
                            .child(widgets::weak(when(session, running.contains(id)))),
                    )
                    .child(session_ui(
                        session,
                        draft,
                        include_system,
                        description,
                        on_click(id),
                    )),
            )
        });
        div()
            .flex()
            .flex_col()
            .gap(theme::LINE_GAP)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x(theme::ITEM_GAP)
                    .gap_y(theme::LINE_GAP)
                    .child(widgets::weak(
                        "Score the scripts you ran in the last 24 hours and report bugs to their authors.",
                    ))
                    .child(widgets::button(
                        "feedback-refresh",
                        "Refresh",
                        !self.loading,
                        on_refresh,
                    )),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(widgets::error(error))
            })
            .children(status)
            .children(cards)
    }
}

pub enum Click {
    Score(u8),
    Toggle,
    IncludeSystem(bool),
    Send,
}

pub type OnClick = Rc<dyn Fn(Click, &mut Window, &mut App)>;

const SCORE_WIDTH: Pixels = px(28.0);
const REPORT_INDENT: Pixels = px(18.0);
const REPORT_ROWS: (usize, usize) = (3, 12);

pub fn description_input(window: &mut Window, cx: &mut App) -> Entity<TextareaState> {
    cx.new(|cx| {
        TextareaState::new(window, cx)
            .auto_grow(REPORT_ROWS.0, REPORT_ROWS.1)
            .placeholder(
                "What went wrong? Where were you, what was the script doing, what did you expect?",
            )
    })
}

pub fn limit_description(text: &str) -> Option<String> {
    (text.chars().count() > MAX_DESCRIPTION_CHARS)
        .then(|| text.chars().take(MAX_DESCRIPTION_CHARS).collect())
}

fn when(session: &RecentSession, running: bool) -> String {
    let started = short_time(&session.started_at);
    match (&session.ended_at, running) {
        (_, true) => format!("started {started} UTC, running"),
        (None, false) => format!("started {started} UTC"),
        (Some(ended), false) => format!("started {started}, ended {} UTC", short_time(ended)),
    }
}

pub fn short_time(iso: &str) -> String {
    let mut text: String = iso.chars().take(16).collect();
    if let Some(t) = text.find('T') {
        text.replace_range(t..=t, " ");
    }
    text
}

pub fn session_ui(
    session: &RecentSession,
    draft: &Draft,
    include_system: bool,
    description: &Entity<TextareaState>,
    on_click: OnClick,
) -> Stateful<Div> {
    let current = draft.pending_score.or(session.score);
    let scoring = draft.pending_score.is_none();
    let scores = (1..=10u8).map(|score| {
        let on_click = on_click.clone();
        widgets::button(("score", usize::from(score)), score.to_string(), scoring, move |_, window, cx| {
            on_click(Click::Score(score), window, cx)
        })
        .min_w(SCORE_WIDTH)
        .justify_center()
        .when(current == Some(score), |this| this.bg(theme::SEL))
        .when(scoring, |this| {
            this.tooltip(widgets::tooltip(
                "1 is the worst, 10 the best. Only the author and moderators see scores, and not who gave them.",
            ))
        })
    });
    let reports = match session.reports {
        0 => String::new(),
        n => format!(" ({n} sent on this run)"),
    };
    let toggle = on_click.clone();
    let header = div()
        .id("report-toggle")
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .h(theme::INTERACT_HEIGHT)
        .cursor_pointer()
        .hover(|style| style.text_color(theme::FG))
        .on_click(move |_, window, cx| toggle(Click::Toggle, window, cx))
        .child(
            div()
                .w(px(10.0))
                .font_family(theme::MONO)
                .text_color(theme::FG_DARK)
                .child(if draft.open { "\u{f0d7}" } else { "\u{f0da}" }),
        )
        .child(format!("Report a bug{reports}"));
    let body = draft.open.then(|| {
        let (include, send) = (on_click.clone(), on_click.clone());
        div()
            .pl(REPORT_INDENT)
            .flex()
            .flex_col()
            .gap(theme::LINE_GAP)
            .child(widgets::weak(
                "The report goes to the script's author with this run's log and game state. Account names are removed from both; don't put them in the description either.",
            ))
            .child(Textarea::new(description).small().w_full().disabled(draft.sending))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x(theme::ITEM_GAP)
                    .gap_y(theme::LINE_GAP)
                    .child(
                        Checkbox::new("include-system")
                            .label("Include system info")
                            .checked(include_system)
                            .tooltip("OS, Wine, CPU, memory and GPU. The loader, lib and agent versions always go with a report.")
                            .on_click(move |on, window, cx| include(Click::IncludeSystem(*on), window, cx)),
                    )
                    .child(widgets::button(
                        "send-report",
                        if draft.sending { "Sending…" } else { "Send report" },
                        !draft.sending,
                        move |_, window, cx| send(Click::Send, window, cx),
                    )),
            )
    });
    let message = draft.message.as_ref().map(|message| match message {
        Ok(text) => div().text_color(theme::GREEN).child(text.clone()),
        Err(text) => widgets::error(text.clone()),
    });
    div()
        .id(SharedString::from(session.session_id.clone()))
        .flex()
        .flex_col()
        .gap(theme::LINE_GAP)
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(theme::ITEM_GAP)
                .gap_y(theme::LINE_GAP)
                .child("Your score:")
                .children(scores)
                .when(session.score.is_some() && scoring, |this| {
                    this.child(widgets::weak("(scores apply to the script, not the run)"))
                }),
        )
        .child(header)
        .children(body)
        .children(message)
}

pub fn report(
    description: String,
    log: &[String],
    names: &[String],
    snapshot: Option<Value>,
    diagnostics: Value,
) -> Report {
    Report {
        description,
        log: Some(scrubbed_log(log, names)).filter(|log| !log.is_empty()),
        snapshot,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, script: &str, score: Option<u8>) -> RecentSession {
        RecentSession {
            session_id: id.into(),
            script_id: script.into(),
            title: "S".into(),
            url: "s".into(),
            revision: 1,
            started_at: "2026-11-04T21:10:33.123+00:00".into(),
            ended_at: None,
            score,
            reports: 0,
        }
    }

    #[test]
    fn scrub_takes_names_out_however_they_are_spelled() {
        let names = vec!["Zezima_Jr".to_owned(), "zezima jr".to_owned()];
        assert_eq!(
            scrub("Logged in as Zezima Jr", &names),
            "Logged in as [account]"
        );
        assert_eq!(
            scrub("name=zezima_jr; ZEZIMA-JR!", &names),
            "name=[account]; [account]!"
        );
        assert_eq!(scrub("zezimajr stays", &names), "zezimajr stays");
        assert_eq!(scrub("untouched", &[]), "untouched");
    }

    #[test]
    fn scrub_ignores_names_too_short_to_mean_anything() {
        assert_eq!(scrub("a b c", &["a".to_owned()]), "a b c");
    }

    #[test]
    fn snapshot_drops_the_names_and_adds_the_containers() {
        let state = GameState {
            account_name: Some("zezima".into()),
            player_name: Some("Zezima".into()),
            run_energy: Some(57),
            ..Default::default()
        };
        let containers = vec![
            (
                INVENTORY,
                vec![
                    Item {
                        slot: 0,
                        id: 995,
                        quantity: 100,
                    },
                    Item {
                        slot: 1,
                        id: -1,
                        quantity: 0,
                    },
                ],
            ),
            (
                EQUIPMENT,
                vec![Item {
                    slot: 3,
                    id: 1333,
                    quantity: 1,
                }],
            ),
        ];
        let value = snapshot(&state, &containers);
        assert!(value.get("account_name").is_none_or(Value::is_null));
        assert!(value.get("player_name").is_none_or(Value::is_null));
        assert_eq!(value["run_energy"], 57);
        assert_eq!(
            value["inventory"],
            json!([{ "slot": 0, "id": 995, "quantity": 100 }])
        );
        assert_eq!(
            value["equipment"],
            json!([{ "slot": 3, "id": 1333, "quantity": 1 }])
        );
        assert!(snapshot(&state, &[])["inventory"].is_null());
    }

    #[test]
    fn tail_keeps_whole_lines_from_the_end() {
        let lines: Vec<String> = ["one", "two", "three"].map(str::to_owned).to_vec();
        assert_eq!(tail(&lines, 100), "one\ntwo\nthree");
        assert_eq!(tail(&lines, 9), "two\nthree");
        assert_eq!(tail(&lines, 5), "three");
        assert_eq!(tail(&lines, 2), "");
    }

    #[test]
    fn report_scrubs_the_log_and_leaves_an_empty_one_out() {
        let log = vec!["hello zezima".to_owned()];
        let names = vec!["zezima".to_owned()];
        let report = report("d".into(), &log, &names, None, json!({}));
        assert_eq!(report.log.as_deref(), Some("hello [account]"));
        assert_eq!(
            super::report("d".into(), &[], &names, None, json!({})).log,
            None
        );
    }

    #[test]
    fn diagnostics_always_carry_versions_and_system_info_only_when_asked() {
        let versions = Versions {
            agent: Some("abc".into()),
            lib: Some("2026.11.01".into()),
            gpu: Some("Mesa".into()),
        };
        let bare = diagnostics(&versions, false);
        assert_eq!(bare["loader"], crate::update::VERSION);
        assert_eq!(bare["agent"], "abc");
        assert!(bare.get("os").is_none());
        let full = diagnostics(&versions, true);
        assert!(full["os"].is_string());
        assert_eq!(full["gpu"], "Mesa");
    }

    #[test]
    fn short_time_cuts_the_iso_stamp_to_the_minute() {
        assert_eq!(
            short_time("2026-11-04T21:10:33.123+00:00"),
            "2026-11-04 21:10"
        );
        assert_eq!(short_time("odd"), "odd");
    }

    #[test]
    fn a_score_shows_on_every_session_of_the_script() {
        let mut page = Page::default();
        page.finish(
            Ok(vec![
                session("a", "x", None),
                session("b", "x", None),
                session("c", "y", None),
            ]),
            Instant::now(),
        );
        page.draft("a").pending_score = Some(7);
        page.scored("a", Ok(7));
        assert_eq!(
            page.sessions.iter().map(|s| s.score).collect::<Vec<_>>(),
            [Some(7), Some(7), None]
        );
        assert!(page.draft("a").pending_score.is_none());
        page.draft("c").pending_score = Some(3);
        page.scored("c", Err("nope".into()));
        assert_eq!(page.sessions[2].score, None);
        assert!(matches!(page.draft("c").message, Some(Err(_))));
    }

    #[test]
    fn a_sent_report_clears_the_draft_and_counts() {
        let mut page = Page::default();
        page.finish(Ok(vec![session("a", "x", None)]), Instant::now());
        let draft = page.draft("a");
        draft.description = "bug".into();
        draft.sending = true;
        page.reported("a", Ok(()));
        assert_eq!(page.sessions[0].reports, 1);
        assert_eq!(page.draft("a").description, "");
        assert!(!page.draft("a").sending);
    }

    #[test]
    fn a_score_and_a_report_go_out_once() {
        let mut page = Page::default();
        page.finish(Ok(vec![session("a", "x", Some(5))]), Instant::now());
        let mut include = false;
        assert!(page.click("a", Click::Score(5), &mut include).is_none());
        assert!(matches!(
            page.click("a", Click::Score(7), &mut include),
            Some(Action::Score { score: 7, .. })
        ));
        assert!(page.click("a", Click::Score(8), &mut include).is_none());
        page.draft("a").description = "  bug \n".into();
        assert!(matches!(
            page.click("a", Click::Send, &mut include),
            Some(Action::Report { description, .. }) if description == "bug"
        ));
        assert!(page.click("a", Click::Send, &mut include).is_none());
        page.click("a", Click::IncludeSystem(true), &mut include);
        assert!(include);
        page.click("a", Click::Toggle, &mut include);
        assert!(page.draft("a").open);
    }

    #[test]
    fn a_description_is_cut_to_the_limit() {
        assert_eq!(limit_description("short"), None);
        let long = "é".repeat(MAX_DESCRIPTION_CHARS + 5);
        assert_eq!(
            limit_description(&long).map(|text| text.chars().count()),
            Some(MAX_DESCRIPTION_CHARS)
        );
    }

    #[test]
    fn the_list_goes_stale_after_a_minute() {
        let mut page = Page::default();
        let now = Instant::now();
        assert!(page.stale(now));
        page.loading = true;
        assert!(!page.stale(now));
        page.finish(Ok(Vec::new()), now);
        assert!(!page.stale(now));
        assert!(page.stale(now + REFRESH_EVERY));
    }

    #[test]
    fn records_survive_a_round_trip_through_json() {
        let record = Record {
            session_id: "s".into(),
            title: "t".into(),
            log: "l".into(),
            snapshot: Some(json!({ "a": 1 })),
            ended_at: 5,
        };
        let text = serde_json::to_vec(&record).unwrap();
        assert_eq!(serde_json::from_slice::<Record>(&text).unwrap(), record);
    }

    #[test]
    fn a_record_file_needs_a_well_formed_session_id() {
        let dir = Path::new("/x");
        assert!(record_path(dir, "../etc/passwd").is_none());
        assert_eq!(
            record_path(dir, "0f7f8a2e-5b1b-4f6d-9c2a-1d2e3f4a5b6c"),
            Some(dir.join("0f7f8a2e-5b1b-4f6d-9c2a-1d2e3f4a5b6c.bin"))
        );
    }
}
