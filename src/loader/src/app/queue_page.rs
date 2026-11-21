use std::collections::HashMap;
use std::time::Instant;

use gpui_kit::component::Sizable;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{Entity, Subscription};
use injector::manifest::Manifest;
use serde_json::Value;

use super::*;
use crate::api::models::ScriptManifest;
use crate::api::{Api, ApiError, Me, StoreScript};
use crate::libs::Lib;
use crate::queue::{self, Entry, Goal, OnChange, Outcome, Phase, Runner, WhenFinished};
use crate::script::{Launch, Script};
use crate::session;
use crate::ui::fields::{self, NumberField, Range};
use crate::ui::{theme, widgets};

const LOGOUT_SOURCE: &str = "local Logout = require(\"@lib/main/interfaces/gametabs/logout\")\nif not Logout.logout() then\n\terror(\"Couldn't log out\", 0)\nend\n";

pub(super) struct Check {
    me: Me,
    scripts: Vec<StoreScript>,
    manifests: HashMap<String, Result<ScriptManifest, String>>,
    quotes: HashMap<String, Result<crate::api::models::Quote, String>>,
    lib: Result<Lib, String>,
}

fn check(api: &Api, ids: &[String]) -> Result<Box<Check>, ApiError> {
    let me = api.me()?;
    let mut scripts = api.scripts("", "", false)?;
    if me.can_browse_community() && ids.iter().any(|id| scripts.iter().all(|s| &s.id != id)) {
        scripts.extend(api.scripts("", "", true)?);
    }
    let mut manifests = HashMap::new();
    let mut quotes = HashMap::new();
    for id in ids {
        if manifests.contains_key(id) {
            continue;
        }
        manifests.insert(id.clone(), api.manifest(id).map_err(|e| e.to_string()));
        quotes.insert(id.clone(), api.quote(id).map_err(|e| e.to_string()));
    }
    let lib = session::cache_root()
        .ok_or_else(|| "Can't find a cache folder on this system.".to_owned())
        .and_then(|root| crate::libs::ensure(api, &root));
    Ok(Box::new(Check {
        me,
        scripts,
        manifests,
        quotes,
        lib,
    }))
}

pub fn spec_hash(manifest: &Manifest) -> Option<String> {
    manifest
        .settings
        .as_ref()
        .map(|spec| session::sha256_hex(spec.to_string().as_bytes()))
}

#[derive(Debug)]
struct Ready {
    script: StoreScript,
    manifest: Manifest,
    price: i64,
}

impl Check {
    fn ready(&self, entry: &Entry, confirmed: &crate::store::Confirmed) -> Result<Ready, String> {
        let title = &entry.title;
        let script = self
            .scripts
            .iter()
            .find(|s| s.id == entry.script_id)
            .ok_or_else(|| format!("{title} is no longer available to this account."))?;
        if !script.can_access {
            return Err(format!("You can't run {title} right now."));
        }
        if !script.queueable {
            return Err(format!("{title}'s live revision can't run in a queue."));
        }
        let fetched = match self.manifests.get(&entry.script_id) {
            Some(Ok(fetched)) => fetched,
            Some(Err(error)) => return Err(format!("{title}: {error}")),
            None => return Err(format!("{title} wasn't checked.")),
        };
        let manifest =
            Manifest::parse(fetched.manifest.clone()).map_err(|e| format!("{title}: {e}"))?;
        if !manifest.queueable {
            return Err(format!("{title}'s live revision can't run in a queue."));
        }
        if spec_hash(&manifest) != entry.spec_hash {
            return Err(format!(
                "{title}'s settings changed. Open its entry to check them."
            ));
        }
        if script.community && !confirmed.has(&script.id, fetched.revision) {
            return Err(format!(
                "{title} has a revision you haven't reviewed. Run it once from a client's scripts page."
            ));
        }
        let price = match self.quotes.get(&entry.script_id) {
            Some(Ok(quote)) => quote.price,
            Some(Err(error)) => return Err(format!("{title}: {error}")),
            None => return Err(format!("{title} wasn't quoted.")),
        };
        Ok(Ready {
            script: StoreScript {
                revision: Some(fetched.revision),
                ..script.clone()
            },
            manifest,
            price,
        })
    }
}

pub(super) struct Prompt {
    pid: u32,
    queue: queue::Queue,
    rows: Vec<(String, i64, Option<i64>)>,
    approved: HashMap<String, i64>,
}

#[derive(Default)]
pub(super) struct Page {
    selected: Option<usize>,
    form: Option<(usize, String, crate::form::Form)>,
    save_name: String,
    add: Option<String>,
    checking: Option<u32>,
    fetching: std::collections::HashSet<String>,
    prompt: Option<Prompt>,
    ui: Ui,
}

impl LoaderApp {
    pub(super) fn sync_queue_form(&mut self) {
        let Some((index, script_id, form)) = &self.queue_page.form else {
            return;
        };
        let hash = self
            .manifests
            .get(script_id)
            .and_then(|(_, m)| spec_hash(m));
        if let Some(entry) = self
            .queues
            .current
            .entries
            .get_mut(*index)
            .filter(|e| &e.script_id == script_id)
        {
            entry.settings = form.remembered();
            entry.spec_hash = hash;
        }
    }

    fn select_entry(&mut self, index: Option<usize>) {
        self.sync_queue_form();
        self.queue_page.form = None;
        self.queue_page.selected = index;
    }

    fn entry_form(&mut self) {
        let Some(index) = self.queue_page.selected else {
            return;
        };
        let Some(entry) = self.queues.current.entries.get(index) else {
            self.queue_page.selected = None;
            return;
        };
        if self
            .queue_page
            .form
            .as_ref()
            .is_some_and(|(i, id, _)| *i == index && id == &entry.script_id)
        {
            return;
        }
        let script_id = entry.script_id.clone();
        match self.manifests.get(&script_id) {
            Some((_, manifest)) => {
                let spec = manifest
                    .settings
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({ "fields": [] }));
                let mut form = crate::form::Form::new(spec, Some(&entry.settings));
                form.title = entry.title.clone();
                self.queue_page.form = Some((index, script_id, form));
                self.sync_queue_form();
            }
            None if !self.queue_page.fetching.contains(&script_id) => {
                self.queue_page.fetching.insert(script_id.clone());
                self.api_job(move |api| Event::QueueManifest {
                    result: api.manifest(&script_id),
                    script_id,
                });
            }
            None => {}
        }
    }

    pub(super) fn queue_manifest(
        &mut self,
        script_id: String,
        result: Result<ScriptManifest, ApiError>,
    ) {
        self.queue_page.fetching.remove(&script_id);
        match result.map_err(|e| e.to_string()).and_then(|fetched| {
            Manifest::parse(fetched.manifest).map(|manifest| (fetched.revision, manifest))
        }) {
            Ok(cached) => {
                self.manifests.insert(script_id, cached);
            }
            Err(error) => {
                self.script_error(format!("Couldn't load the script's settings: {error}"))
            }
        }
    }

    fn queueable_scripts(&self) -> Vec<StoreScript> {
        let mut scripts: Vec<StoreScript> = self
            .store
            .scripts
            .iter()
            .filter(|s| s.queueable && s.can_access)
            .cloned()
            .collect();
        scripts.sort_by(|a, b| a.title.cmp(&b.title));
        scripts
    }

    fn check_queue(&mut self, pid: u32) {
        self.sync_queue_form();
        self.queue_page.checking = Some(pid);
        let ids: Vec<String> = self
            .queues
            .current
            .entries
            .iter()
            .map(|e| e.script_id.clone())
            .collect();
        self.api_job(move |api| Event::QueueChecked {
            pid,
            entry: false,
            result: check(&api, &ids),
        });
    }

    pub(super) fn queue_checked(
        &mut self,
        pid: u32,
        entry: bool,
        result: Result<Box<Check>, ApiError>,
    ) {
        let check = match result {
            Ok(check) => check,
            Err(ApiError::SignedOut(why)) => return self.sign_out(Some(why)),
            Err(error) => {
                self.queue_page.checking = None;
                return if entry {
                    self.end_queue(pid, format!("Queue stopped: {error}"))
                } else {
                    self.script_error(format!("Couldn't check the queue: {error}"))
                };
            }
        };
        self.refresh_me(check.me.clone());
        for (id, fetched) in &check.manifests {
            if let Ok(fetched) = fetched
                && let Ok(manifest) = Manifest::parse(fetched.manifest.clone())
            {
                self.manifests
                    .insert(id.clone(), (fetched.revision, manifest));
            }
        }
        if entry {
            self.start_checked_entry(pid, &check);
        } else {
            self.queue_page.checking = None;
            self.prompt_queue(pid, &check);
        }
    }

    fn prompt_queue(&mut self, pid: u32, check: &Check) {
        if let Err(error) = &check.lib {
            return self.script_error(format!("Couldn't start the queue: {error}"));
        }
        let queue = self.queues.current.clone();
        let mut rows = Vec::new();
        let mut approved = HashMap::new();
        let mut problems = Vec::new();
        for entry in &queue.entries {
            match check.ready(entry, &self.confirmed) {
                Ok(ready) => {
                    rows.push((
                        entry.title.clone(),
                        ready.price,
                        entry.most_cost(ready.price),
                    ));
                    approved.insert(entry.script_id.clone(), ready.price);
                }
                Err(problem) => problems.push(problem),
            }
        }
        if !problems.is_empty() {
            return self.script_error(format!("Couldn't start the queue: {}", problems.join(" ")));
        }
        self.queue_page.prompt = Some(Prompt {
            pid,
            queue,
            rows,
            approved,
        });
    }

    fn start_queue_entry(&mut self, pid: u32) {
        let Some(runner) = self.queue_runners.iter_mut().find(|r| r.pid == pid) else {
            return;
        };
        runner.phase = Phase::Starting;
        let Some(entry) = runner.entry() else {
            return;
        };
        let ids = vec![entry.script_id.clone()];
        self.api_job(move |api| Event::QueueChecked {
            pid,
            entry: true,
            result: check(&api, &ids),
        });
    }

    fn start_checked_entry(&mut self, pid: u32, check: &Check) {
        let Some(runner) = self
            .queue_runners
            .iter()
            .find(|r| r.pid == pid && r.phase == Phase::Starting)
        else {
            return;
        };
        let Some(entry) = runner.entry().cloned() else {
            return;
        };
        let approved = runner.approved.get(&entry.script_id).copied();
        let on_change = runner.queue.on_change;
        let resume = runner
            .resume
            .as_ref()
            .map(|(plan, at)| queue::resumed(plan, *at, Instant::now()));
        let ready = check
            .ready(&entry, &self.confirmed)
            .and_then(|ready| match approved {
                Some(price) if price == ready.price => Ok(ready),
                _ => Err(format!(
                    "{}'s price changed since the queue started.",
                    entry.title
                )),
            });
        let ready = match ready {
            Ok(ready) => ready,
            Err(change) => {
                return match on_change {
                    OnChange::Skip => {
                        self.script_info(format!("Skipping it: {change}"));
                        self.next_queue_entry(pid);
                    }
                    OnChange::Stop => self.end_queue(pid, format!("Queue stopped: {change}")),
                };
            }
        };
        let lib = match &check.lib {
            Ok(lib) => Lib {
                version: lib.version.clone(),
                warning: lib.warning.clone(),
            },
            Err(error) => return self.end_queue(pid, format!("Queue stopped: {error}")),
        };
        let quote = match check.quotes.get(&entry.script_id) {
            Some(Ok(quote)) => quote.clone(),
            _ => return self.end_queue(pid, "Queue stopped: no quote.".into()),
        };
        if let Some(why) = session::start_block(&ready.script, &quote, check.me.microcredits) {
            return self.end_queue(pid, format!("Queue stopped: {why}"));
        }
        let Some(target) = self
            .targets
            .iter()
            .find(|t| t.client.pid == pid && t.client.loaded.is_some())
            .cloned()
        else {
            return self.end_queue(pid, "Queue stopped: its client is gone.".into());
        };
        let spec = ready
            .manifest
            .settings
            .clone()
            .unwrap_or_else(|| serde_json::json!({ "fields": [] }));
        let form = crate::form::Form::new(spec, Some(&entry.settings));
        let layouts = self
            .layouts
            .get(&entry.title)
            .unwrap_or(&super::EMPTY_LAYOUTS);
        let values = match form.submitted(layouts, &self.known_items) {
            crate::form::Outcome::Submitted(values) => values,
            _ => Value::Object(Default::default()),
        };
        self.start_store_script(PendingStart {
            script: ready.script,
            target,
            replaces: None,
            lib,
            run: None,
            launch: Launch { values, resume },
        });
    }

    pub(super) fn queue_entry_started(&mut self, pid: u32, run: Option<u64>) {
        let Some(runner) = self
            .queue_runners
            .iter_mut()
            .find(|r| r.pid == pid && r.phase == Phase::Starting)
        else {
            return;
        };
        match run {
            Some(id) => {
                runner.phase = Phase::Running {
                    run: id,
                    finish_sent: None,
                };
                let progress = runner.progress();
                if let Some(run) = self.run_by_id_mut(id) {
                    run.info(format!("Queue {progress}."));
                }
            }
            None => self.end_queue(pid, "Queue stopped: the script didn't start.".into()),
        }
    }

    fn next_queue_entry(&mut self, pid: u32) {
        let Some(runner) = self.queue_runners.iter_mut().find(|r| r.pid == pid) else {
            return;
        };
        if !runner.finishing && runner.advance() {
            self.start_queue_entry(pid);
        } else {
            self.finish_queue(pid, "Queue finished.".into());
        }
    }

    fn finish_queue(&mut self, pid: u32, message: String) {
        let Some(runner) = self.queue_runners.iter().find(|r| r.pid == pid) else {
            return;
        };
        if runner.queue.when_finished == WhenFinished::StayLoggedIn {
            return self.end_queue_info(pid, message);
        }
        match self.start_logout(pid) {
            Ok(run) => {
                self.script_info(format!("{message} Logging out."));
                if let Some(runner) = self.queue_runners.iter_mut().find(|r| r.pid == pid) {
                    runner.phase = Phase::LoggingOut { run };
                }
            }
            Err(error) => self.end_queue(pid, format!("{message} Couldn't log out: {error}")),
        }
    }

    fn start_logout(&mut self, pid: u32) -> Result<u64, String> {
        let target = self
            .targets
            .iter()
            .find(|t| t.client.pid == pid && t.client.loaded.is_some())
            .cloned()
            .ok_or("its client is gone")?;
        let root = session::cache_root().ok_or("no cache folder")?;
        let script = Script::start(
            &self.script_host(),
            LOGOUT_SOURCE,
            Some(&root.join("queue-logout.luau")),
            Some(&root.join("lib")),
            Some(&target),
            &self.settings,
            &Launch::default(),
        )?;
        let id = self.next_run_id();
        let mut run = Run::new(
            id,
            "Log out".into(),
            Some(script),
            Some(target),
            None,
            &self.settings,
        );
        run.info("The queue is logging the account out.");
        self.runs.push(run);
        Ok(id)
    }

    fn end_queue_info(&mut self, pid: u32, message: String) {
        self.queue_runners.retain(|r| r.pid != pid);
        self.script_info(message);
    }

    pub(super) fn end_queue(&mut self, pid: u32, message: String) {
        self.queue_runners.retain(|r| r.pid != pid);
        self.script_error(message);
    }

    pub(super) fn drive_queues(&mut self) {
        if self.queue_runners.is_empty() {
            return;
        }
        self.wake.wake_after(std::time::Duration::from_secs(1));
        let pids: Vec<u32> = self.queue_runners.iter().map(|r| r.pid).collect();
        for pid in pids {
            self.drive_queue(pid);
        }
    }

    fn drive_queue(&mut self, pid: u32) {
        let Some(runner) = self.queue_runners.iter().find(|r| r.pid == pid) else {
            return;
        };
        match runner.phase.clone() {
            Phase::Starting => {}
            Phase::LoggingOut { run } => {
                if self.run_by_id(run).is_none_or(|r| !r.is_running()) {
                    self.end_queue_info(pid, "Queue finished.".into());
                }
            }
            Phase::Running { run, finish_sent } => {
                let Some(current) = self.run_by_id(run) else {
                    return self.end_queue(pid, "Queue stopped: its run was closed.".into());
                };
                if let Some((result, stopped)) = current.finished.clone() {
                    let session_open = current.store.as_ref().is_some_and(|store| {
                        !matches!(
                            store.closed.try_recv(),
                            Err(std::sync::mpsc::TryRecvError::Disconnected)
                        )
                    });
                    if session_open {
                        return;
                    }
                    let killed = runner.killed;
                    let title = current.title.clone();
                    let outcome = queue::outcome(&result, current.exit.as_ref(), killed, stopped);
                    let breaks = current.last_breaks.clone();
                    if let Some(runner) = self.queue_runners.iter_mut().find(|r| r.pid == pid) {
                        runner.resume = breaks.or(runner.resume.take());
                    }
                    return match outcome {
                        Outcome::Done => self.next_queue_entry(pid),
                        Outcome::Blocked => {
                            self.script_info(format!(
                                "{title} couldn't go on, so the queue moves on."
                            ));
                            self.next_queue_entry(pid);
                        }
                        Outcome::Failed => {
                            self.finish_queue(pid, format!("Queue stopped: {title} failed."))
                        }
                        Outcome::Stopped => {
                            self.end_queue(pid, format!("Queue stopped: {title} was stopped."))
                        }
                    };
                }
                let Some(entry) = runner.entry() else {
                    return;
                };
                match finish_sent {
                    None => {
                        let counter = entry
                            .goal
                            .as_ref()
                            .and_then(|g| current.counters.value(&g.counter));
                        if runner.finishing || entry.should_finish(current.run_time(), counter) {
                            let sent = current.send(Script::finish);
                            if let Some(run) = self.run_by_id_mut(run) {
                                run.report(sent, "The queue asked the script to wrap up.");
                            }
                            if let Some(runner) =
                                self.queue_runners.iter_mut().find(|r| r.pid == pid)
                            {
                                runner.phase = Phase::Running {
                                    run,
                                    finish_sent: Some(Instant::now()),
                                };
                            }
                        }
                    }
                    Some(_) => {
                        let grace = self.manifests.get(&entry.script_id).map_or(
                            injector::manifest::DEFAULT_FINISH_GRACE_MINUTES,
                            |(_, m)| m.finish_grace_minutes,
                        );
                        let late = runner.grace(grace).is_some_and(|left| left.is_zero());
                        if late && !runner.killed {
                            if let Some(runner) =
                                self.queue_runners.iter_mut().find(|r| r.pid == pid)
                            {
                                runner.killed = true;
                            }
                            if let Some(run) = self.run_by_id_mut(run) {
                                run.error(format!(
                                    "The script didn't wrap up within {grace} minutes, so it was stopped."
                                ));
                                run.stop();
                            }
                        }
                    }
                }
            }
        }
    }

    pub(super) fn queue_banner(&self, run: u64) -> Option<String> {
        self.queue_runners
            .iter()
            .find(|r| r.run_id() == Some(run))
            .map(Runner::progress)
    }
}

fn limits_text(entry: &Entry) -> String {
    let mut parts = Vec::new();
    if let Some(goal) = &entry.goal {
        parts.push(format!(
            "goal {} {}",
            crate::counters::number(goal.target),
            goal.counter
        ));
    }
    if let Some(min) = entry.min_minutes {
        parts.push(format!("at least {min} min"));
    }
    if let Some(max) = entry.max_minutes {
        parts.push(format!("at most {max} min"));
    }
    if parts.is_empty() {
        "until it ends".into()
    } else {
        parts.join(", ")
    }
}

const SAVE_NAME_WIDTH: Pixels = px(160.0);
const NUMBER_WIDTH: Pixels = px(110.0);
const PROMPT_WIDTH: Pixels = px(480.0);

struct SaveName {
    input: Entity<InputState>,
    hint: String,
    _subscription: Subscription,
}

struct Limits {
    index: usize,
    goal: NumberField,
    min: NumberField,
    max: NumberField,
}

#[derive(Default)]
struct Ui {
    save_name: Option<SaveName>,
    limits: Option<Limits>,
    form: Option<((usize, String), crate::form::FormUi)>,
}

#[derive(Clone, Copy)]
enum Move {
    Up,
    Down,
    Duplicate,
    Remove,
}

#[derive(Clone, Copy)]
enum Bound {
    AtLeast,
    AtMost,
}

impl Bound {
    fn label(self) -> &'static str {
        match self {
            Self::AtLeast => "At least",
            Self::AtMost => "At most",
        }
    }

    fn of(self, entry: &mut Entry) -> &mut Option<u32> {
        match self {
            Self::AtLeast => &mut entry.min_minutes,
            Self::AtMost => &mut entry.max_minutes,
        }
    }

    fn field(self, limits: &Limits) -> &NumberField {
        match self {
            Self::AtLeast => &limits.min,
            Self::AtMost => &limits.max,
        }
    }
}

fn pick_goal(entry: &mut Entry, pick: Option<String>) {
    match pick {
        None => entry.goal = None,
        Some(counter) => {
            let target = entry.goal.as_ref().map_or(1.0, |g| g.target);
            entry.goal = Some(Goal { counter, target });
        }
    }
}

fn edit_entry(
    cx: &mut Context<LoaderApp>,
    index: usize,
    change: impl Fn(&mut Entry) + 'static,
) -> impl Fn(&mut App) + 'static {
    let app = cx.entity().downgrade();
    move |cx| {
        app.update(cx, |app, cx| {
            if let Some(entry) = app.queues.current.entries.get_mut(index) {
                change(entry);
            }
            cx.notify();
        })
        .ok();
    }
}

impl Limits {
    fn new(index: usize, window: &mut Window, cx: &mut Context<LoaderApp>) -> Self {
        let app = cx.entity().downgrade();
        let number = |change: fn(&mut Entry, f64)| {
            let app = app.clone();
            move |value: f64, cx: &mut App| {
                app.update(cx, |app, cx| {
                    if let Some(entry) = app.queues.current.entries.get_mut(index) {
                        change(entry, value);
                    }
                    cx.notify();
                })
                .ok();
            }
        };
        let minutes = Range::new(1.0..=f64::from(queue::MAX_MINUTES)).integer();
        Self {
            index,
            goal: NumberField::new(
                Range::new(1.0..=1e9),
                false,
                number(|entry, value| {
                    if let Some(goal) = entry.goal.as_mut() {
                        goal.target = value;
                    }
                }),
                window,
                cx,
            ),
            min: NumberField::new(
                minutes,
                false,
                number(|entry, value| {
                    if entry.min_minutes.is_some() {
                        entry.min_minutes = Some(value as u32);
                    }
                }),
                window,
                cx,
            ),
            max: NumberField::new(
                minutes,
                false,
                number(|entry, value| {
                    if entry.max_minutes.is_some() {
                        entry.max_minutes = Some(value as u32);
                    }
                }),
                window,
                cx,
            ),
        }
    }
}

fn row() -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x(theme::ITEM_GAP)
        .gap_y(theme::LINE_GAP)
}

fn vertical_separator() -> Div {
    div()
        .flex_none()
        .w(px(1.0))
        .h(theme::INTERACT_HEIGHT)
        .bg(theme::BORDER)
}

fn selectable(
    id: impl Into<ElementId>,
    label: String,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .h(theme::INTERACT_HEIGHT)
        .px(theme::BUTTON_PADDING)
        .cursor_pointer()
        .when(selected, |this| this.bg(theme::SEL).text_color(theme::BLUE))
        .hover(|style| style.bg(theme::SEL))
        .on_click(on_click)
        .child(label)
}

fn minutes_suffix() -> Div {
    div().text_color(theme::COMMENT).child("min")
}

impl LoaderApp {
    fn ask_queue_to_finish(&mut self, pid: u32) {
        if let Some(runner) = self.queue_runners.iter_mut().find(|r| r.pid == pid) {
            runner.finishing = true;
        }
    }

    fn stop_queue(&mut self, pid: u32) {
        self.end_queue(pid, "Queue stopped.".into());
        if let Some(run) = self
            .runs
            .iter_mut()
            .find(|r| r.target.as_ref().is_some_and(|t| t.client.pid == pid) && r.is_running())
        {
            run.stop();
        }
    }

    fn load_saved_queue(&mut self, name: String) {
        self.select_entry(None);
        self.queues.load(&name);
        self.queue_page.save_name = name;
    }

    fn save_queue(&mut self) {
        self.sync_queue_form();
        let name = if self.queue_page.save_name.trim().is_empty() {
            self.queues.current.name.clone()
        } else {
            self.queue_page.save_name.clone()
        };
        match self.queues.save_as(&name) {
            Ok(()) => self.info(format!("Saved the queue {}.", name.trim())),
            Err(error) => self.error(error),
        }
    }

    fn delete_saved_queue(&mut self) {
        let name = self.queues.current.name.clone();
        self.queues.delete(&name);
        self.info(format!("Deleted the saved queue {name}."));
    }

    fn add_queue_entry(&mut self, script: &StoreScript) {
        self.sync_queue_form();
        self.queues
            .current
            .entries
            .push(Entry::new(script.id.clone(), script.title.clone()));
        let last = self.queues.current.entries.len() - 1;
        self.select_entry(Some(last));
    }

    fn move_entry(&mut self, i: usize, what: Move) {
        self.sync_queue_form();
        self.queue_page.form = None;
        let entries = &mut self.queues.current.entries;
        match what {
            Move::Remove => {
                entries.remove(i);
                self.queue_page.selected = None;
            }
            Move::Duplicate if entries.len() < queue::MAX_ENTRIES => {
                entries.insert(i + 1, entries[i].clone());
                self.queue_page.selected = Some(i + 1);
            }
            Move::Up if i > 0 => {
                entries.swap(i, i - 1);
                self.queue_page.selected = Some(i - 1);
            }
            Move::Down if i + 1 < entries.len() => {
                entries.swap(i, i + 1);
                self.queue_page.selected = Some(i + 1);
            }
            _ => {}
        }
    }

    fn answer_queue_prompt(&mut self, start: bool) {
        if !start {
            self.queue_page.prompt = None;
            return;
        }
        if let Some(prompt) = self.queue_page.prompt.take() {
            if self.free_client(prompt.pid).is_none() {
                return self.script_error("The queue's client is gone or busy.");
            }
            let pid = prompt.pid;
            self.queue_runners
                .push(Runner::new(prompt.queue, pid, prompt.approved));
            self.script_info("Queue started.");
            self.start_queue_entry(pid);
        }
    }

    pub(super) fn render_queue(
        &mut self,
        pid: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(theme::LINE_GAP)
            .child(widgets::heading(format!("Client {pid}: queue")));
        if self.auth.me().is_none() {
            return page
                .child("Sign in to queue store scripts.")
                .into_any_element();
        }
        if !self.store.loaded && !self.store.loading && self.store.error.is_none() {
            self.fetch_scripts();
        }
        if let Some(running) = self.render_running_queue(pid, cx) {
            return page
                .child(running)
                .child(widgets::separator())
                .child(
                    div()
                        .id("queue-running")
                        .flex_1()
                        .min_h_0()
                        .child(self.render_running_entries(pid))
                        .overflow_y_scrollbar(),
                )
                .into_any_element();
        }
        let saved = self.render_saved_queues(window, cx);
        let options = self.render_queue_options(cx);
        let add = self.render_add_entry(pid, cx);
        self.entry_form();
        let entries = self.render_entries(cx);
        let editor = self.render_entry_editor(window, cx);
        let prompt = self
            .queue_page
            .prompt
            .as_ref()
            .is_some_and(|prompt| prompt.pid == pid)
            .then(|| self.render_queue_prompt(window, cx))
            .flatten();
        page.child(saved.mt(px(8.0)))
            .child(options.mt(px(8.0)))
            .child(add.mt(px(8.0)))
            .child(widgets::separator())
            .child(
                div()
                    .id("queue")
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(theme::LINE_GAP)
                            .child(entries)
                            .child(widgets::separator())
                            .children(editor),
                    )
                    .overflow_y_scrollbar(),
            )
            .children(prompt)
            .into_any_element()
    }

    fn render_running_entries(&self, pid: u32) -> Div {
        let Some(runner) = self.queue_on(pid) else {
            return div();
        };
        div().flex().flex_col().gap(theme::LINE_GAP).children(
            runner.queue.entries.iter().enumerate().map(|(i, entry)| {
                let current = i == runner.index;
                row()
                    .child(
                        div()
                            .when(current, |this| {
                                this.text_color(theme::GREEN).font_weight(FontWeight::BOLD)
                            })
                            .when(i < runner.index, |this| this.text_color(theme::COMMENT))
                            .child(format!("{}. {}", i + 1, entry.title)),
                    )
                    .child(widgets::weak(limits_text(entry)))
            }),
        )
    }

    fn render_running_queue(&self, pid: u32, cx: &mut Context<Self>) -> Option<Div> {
        self.queue_on(pid).map(|runner| {
            row()
                .child(widgets::status_dot(theme::GREEN))
                .child(runner.progress())
                .child(widgets::weak(crate::xp::duration(runner.started.elapsed())))
                .child(
                    widgets::button(
                        ("queue-finish", pid as usize),
                        "Finish",
                        true,
                        cx.listener(move |app, _: &ClickEvent, _, cx| {
                            app.ask_queue_to_finish(pid);
                            cx.notify();
                        }),
                    )
                    .tooltip(widgets::tooltip(
                        "Lets the current script wrap up, then ends the queue.",
                    )),
                )
                .child(
                    widgets::button(
                        ("queue-stop", pid as usize),
                        "Stop",
                        true,
                        cx.listener(move |app, _: &ClickEvent, _, cx| {
                            app.stop_queue(pid);
                            cx.notify();
                        }),
                    )
                    .tooltip(widgets::tooltip(
                        "Stops the current script now and ends the queue.",
                    )),
                )
        })
    }

    fn save_name_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let hint = &self.queues.current.name;
        let save = self.queue_page.ui.save_name.get_or_insert_with(|| {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(hint.clone()));
            let subscription =
                cx.subscribe_in(&input, window, |app, input, event: &InputEvent, _, cx| {
                    if let InputEvent::Change = event {
                        app.queue_page.save_name = input.read(cx).value().to_string();
                        cx.notify();
                    }
                });
            SaveName {
                input,
                hint: hint.clone(),
                _subscription: subscription,
            }
        });
        if &save.hint != hint {
            save.hint.clone_from(hint);
            let hint = hint.clone();
            save.input
                .update(cx, |input, cx| input.set_placeholder(hint, window, cx));
        }
        let name = &self.queue_page.save_name;
        if save.input.read(cx).value().as_ref() != name.as_str() {
            let name = name.clone();
            save.input
                .update(cx, |input, cx| input.set_value(name, window, cx));
        }
        save.input.clone()
    }

    fn render_saved_queues(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let input = self.save_name_input(window, cx);
        let app = cx.entity().downgrade();
        let choices = self
            .queues
            .saved
            .iter()
            .map(|saved| {
                let (app, name) = (app.clone(), saved.name.clone());
                widgets::Choice::new(saved.name.clone(), false, move |_, cx| {
                    let name = name.clone();
                    app.update(cx, |app, cx| {
                        app.load_saved_queue(name);
                        cx.notify();
                    })
                    .ok();
                })
            })
            .collect();
        let saved = self
            .queues
            .saved
            .iter()
            .any(|q| q.name == self.queues.current.name);
        row()
            .child(widgets::combo(
                "saved_queues",
                "Load a saved queue",
                choices,
            ))
            .child(Input::new(&input).small().w(SAVE_NAME_WIDTH))
            .child(widgets::button(
                "queue-save",
                "Save",
                true,
                cx.listener(|app, _: &ClickEvent, _, cx| {
                    app.save_queue();
                    cx.notify();
                }),
            ))
            .when(saved, |this| {
                this.child(widgets::button(
                    "queue-delete-saved",
                    "Delete saved",
                    true,
                    cx.listener(|app, _: &ClickEvent, _, cx| {
                        app.delete_saved_queue();
                        cx.notify();
                    }),
                ))
            })
    }

    fn render_queue_options(&self, cx: &mut Context<Self>) -> Div {
        let queue = &self.queues.current;
        let app = cx.entity().downgrade();
        let set = |label: &'static str, selected: bool, apply: fn(&mut queue::Queue)| {
            let app = app.clone();
            widgets::Choice::new(label, selected, move |_, cx| {
                app.update(cx, |app, cx| {
                    apply(&mut app.queues.current);
                    cx.notify();
                })
                .ok();
            })
        };
        let repeat = {
            let app = app.clone();
            fields::checkbox(
                "queue-repeat",
                "Repeat",
                queue.repeat,
                true,
                move |on, cx| {
                    app.update(cx, |app, cx| {
                        app.queues.current.repeat = on;
                        cx.notify();
                    })
                    .ok();
                },
            )
            .tooltip("Starts over from the first entry after the last one.")
        };
        let when_finished = widgets::combo(
            "queue_when_finished",
            match queue.when_finished {
                WhenFinished::LogOut => "log out",
                WhenFinished::StayLoggedIn => "stay logged in",
            },
            vec![
                set(
                    "log out",
                    queue.when_finished == WhenFinished::LogOut,
                    |q| q.when_finished = WhenFinished::LogOut,
                ),
                set(
                    "stay logged in",
                    queue.when_finished == WhenFinished::StayLoggedIn,
                    |q| q.when_finished = WhenFinished::StayLoggedIn,
                ),
            ],
        );
        let on_change = widgets::combo(
            "queue_on_change",
            match queue.on_change {
                OnChange::Stop => "stop the queue",
                OnChange::Skip => "skip it",
            },
            vec![
                set("stop the queue", queue.on_change == OnChange::Stop, |q| {
                    q.on_change = OnChange::Stop
                }),
                set("skip it", queue.on_change == OnChange::Skip, |q| {
                    q.on_change = OnChange::Skip
                }),
            ],
        );
        row()
            .child(repeat)
            .child("When finished:")
            .child(when_finished)
            .child(
                div()
                    .id("queue-on-change-label")
                    .child("If a script changed:")
                    .tooltip(widgets::tooltip(
                        "Its price, its settings or a new community revision since the queue started.",
                    )),
            )
            .child(on_change)
    }

    fn render_add_entry(&mut self, pid: u32, cx: &mut Context<Self>) -> Div {
        let scripts = self.queueable_scripts();
        let chosen = self
            .queue_page
            .add
            .as_ref()
            .and_then(|id| scripts.iter().find(|s| &s.id == id))
            .cloned();
        let app = cx.entity().downgrade();
        let choices = scripts
            .iter()
            .map(|script| {
                let (app, id) = (app.clone(), script.id.clone());
                widgets::Choice::new(script.title.clone(), false, move |_, cx| {
                    let id = id.clone();
                    app.update(cx, |app, cx| {
                        app.queue_page.add = Some(id);
                        cx.notify();
                    })
                    .ok();
                })
            })
            .collect();
        let can_add = chosen.is_some() && self.queues.current.entries.len() < queue::MAX_ENTRIES;
        let picked = chosen
            .as_ref()
            .map_or("Pick a queueable script".to_owned(), |s| s.title.clone());
        let add = widgets::button(
            "queue-add-entry",
            "Add",
            can_add,
            cx.listener(move |app, _: &ClickEvent, _, cx| {
                if let Some(script) = &chosen {
                    app.add_queue_entry(script);
                }
                cx.notify();
            }),
        );
        let target = self.free_client(pid).cloned();
        let problem = self
            .queues
            .current
            .problem()
            .or_else(|| self.run_block(target.as_ref()))
            .or_else(|| {
                self.queue_page
                    .checking
                    .map(|_| "Checking the queue…".into())
            });
        let start = widgets::button(
            "queue-start",
            "Start queue",
            problem.is_none(),
            cx.listener(move |app, _: &ClickEvent, _, cx| {
                app.check_queue(pid);
                cx.notify();
            }),
        )
        .when_some(problem, |this, why| this.tooltip(widgets::tooltip(why)));
        div()
            .flex()
            .flex_col()
            .gap(theme::LINE_GAP)
            .child(
                row()
                    .child(widgets::combo("queue_add", picked, choices))
                    .child(add)
                    .child(vertical_separator())
                    .child(start),
            )
            .when(scripts.is_empty() && self.store.loaded, |this| {
                this.child(widgets::weak(
                    "None of your scripts can run in a queue yet.",
                ))
            })
    }

    fn render_entries(&self, cx: &mut Context<Self>) -> Div {
        let entries = &self.queues.current.entries;
        let rows = entries.iter().enumerate().map(|(i, entry)| {
            let picked = self.queue_page.selected == Some(i);
            let action = |what: Move, label: &'static str| {
                fields::small_button(
                    (label, i),
                    label,
                    true,
                    cx.listener(move |app, _: &ClickEvent, _, cx| {
                        app.move_entry(i, what);
                        cx.notify();
                    }),
                )
            };
            row()
                .child(selectable(
                    ("queue-entry", i),
                    format!("{}. {}", i + 1, entry.title),
                    picked,
                    cx.listener(move |app, _: &ClickEvent, _, cx| {
                        app.select_entry(Some(i));
                        cx.notify();
                    }),
                ))
                .child(widgets::weak(limits_text(entry)))
                .when(picked, |this| {
                    this.child(action(Move::Up, "Up"))
                        .child(action(Move::Down, "Down"))
                        .child(action(Move::Duplicate, "Duplicate"))
                        .child(action(Move::Remove, "Remove"))
                })
        });
        div()
            .flex()
            .flex_col()
            .gap(theme::LINE_GAP)
            .children(rows)
            .when(entries.is_empty(), |this| {
                this.child(widgets::weak(
                    "Add scripts to build a queue. Each entry keeps its own settings.",
                ))
            })
    }

    fn render_minutes(
        &mut self,
        index: usize,
        bound: Bound,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let Some(entry) = self.queues.current.entries.get_mut(index) else {
            return div();
        };
        let value = *bound.of(entry);
        let label = bound.label();
        let toggle = edit_entry(cx, index, move |entry| {
            let value = bound.of(entry);
            *value = if value.is_some() { None } else { Some(60) };
        });
        let field = self
            .queue_page
            .ui
            .limits
            .as_ref()
            .map(|limits| bound.field(limits));
        let input = value.zip(field).map(|(value, field)| {
            field.sync(f64::from(value), window, cx);
            field.input(true).w(NUMBER_WIDTH).suffix(minutes_suffix())
        });
        div()
            .flex()
            .items_center()
            .gap(theme::ITEM_GAP)
            .child(fields::checkbox(
                (label, index),
                label,
                value.is_some(),
                true,
                move |_, cx| toggle(cx),
            ))
            .children(input)
    }

    fn render_entry_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Div> {
        let index = self.queue_page.selected?;
        let entry = self.queues.current.entries.get(index)?;
        let goals = self
            .manifests
            .get(&entry.script_id)
            .map(|(_, m)| m.goals.clone())
            .unwrap_or_default();
        if self
            .queue_page
            .ui
            .limits
            .as_ref()
            .is_none_or(|limits| limits.index != index)
        {
            self.queue_page.ui.limits = Some(Limits::new(index, window, cx));
        }
        let entry = &self.queues.current.entries[index];
        let title = entry.title.clone();
        let current = entry
            .goal
            .as_ref()
            .and_then(|g| goals.iter().find(|goal| goal.id == g.counter))
            .map_or("none".to_owned(), |g| g.label.clone());
        let target = entry.goal.as_ref().map(|g| g.target);
        let mut pick = |label: String, selected: bool, counter: Option<String>| {
            let change = edit_entry(cx, index, move |entry| pick_goal(entry, counter.clone()));
            widgets::Choice::new(label, selected, move |_, cx| change(cx))
        };
        let choices = std::iter::once(pick("none".into(), target.is_none(), None))
            .chain(
                goals
                    .iter()
                    .map(|goal| pick(goal.label.clone(), false, Some(goal.id.clone()))),
            )
            .collect();
        let goal_input = target.and_then(|target| {
            let field = &self.queue_page.ui.limits.as_ref()?.goal;
            field.sync(target, window, cx);
            Some(field.input(true).w(NUMBER_WIDTH))
        });
        let goal = row()
            .child("Goal:")
            .child(widgets::combo(("queue_goal", index), current, choices))
            .children(goal_input)
            .when(goals.is_empty(), |this| {
                this.child(widgets::weak("(this script names no goals)"))
            });
        let at_least = self.render_minutes(index, Bound::AtLeast, window, cx);
        let at_most = self.render_minutes(index, Bound::AtMost, window, cx);
        let settings = self.render_entry_form(index, &title, window, cx);
        Some(
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap(theme::LINE_GAP)
                .child(widgets::strong(title))
                .child(goal)
                .child(at_least)
                .child(at_most)
                .child(div().mt(px(8.0)).child(settings)),
        )
    }

    fn render_entry_form(
        &mut self,
        index: usize,
        title: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = cx.entity().downgrade();
        let Page { form, ui, .. } = &mut self.queue_page;
        let Some((_, script_id, form)) = form.as_mut().filter(|(i, _, _)| *i == index) else {
            return div()
                .flex()
                .items_center()
                .gap(theme::ITEM_GAP)
                .child(Spinner::new())
                .child("Loading the script's settings…")
                .into_any_element();
        };
        let key = (index, script_id.clone());
        if ui.form.as_ref().is_none_or(|(shown, _)| shown != &key) {
            let wanted = key.clone();
            let form_ui = crate::form::FormUi::new(move |cx, change| {
                app.update(cx, |app, cx| {
                    if let Some((i, id, form)) = app.queue_page.form.as_mut()
                        && (*i, id.as_str()) == (wanted.0, wanted.1.as_str())
                    {
                        change(form);
                    }
                    cx.notify();
                })
                .ok();
            });
            ui.form = Some((key, form_ui));
        }
        let Some((_, form_ui)) = ui.form.as_mut() else {
            return div().into_any_element();
        };
        let layouts = self.layouts.get(title).unwrap_or(&EMPTY_LAYOUTS);
        form.render(form_ui, layouts, window, cx)
    }

    fn render_queue_prompt(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let prompt = self.queue_page.prompt.as_ref()?;
        use crate::api::models::{credits_label, rate_label};
        let rows: Vec<[String; 3]> = prompt
            .rows
            .iter()
            .map(|(title, price, most)| {
                [
                    title.clone(),
                    rate_label(*price as f64),
                    most.map_or_else(|| "no limit".to_owned(), |m| credits_label(m as f64)),
                ]
            })
            .collect();
        let heads = ["Script", "Per hour", "At most"];
        let cell = |content: Div, stripe: bool| {
            div()
                .px(px(6.0))
                .py(px(2.0))
                .when(stripe, |this| this.bg(theme::BG_HL))
                .child(content)
        };
        let grid = rows.iter().enumerate().fold(
            heads.iter().fold(
                div().mt(theme::LINE_GAP).grid().grid_cols_max_content(3),
                |grid, head| grid.child(cell(widgets::strong(*head), false)),
            ),
            |grid, (i, row)| {
                row.iter().fold(grid, |grid, text| {
                    grid.child(cell(div().child(text.clone()), i % 2 == 1))
                })
            },
        );
        let unlimited =
            prompt.queue.repeat || prompt.rows.iter().any(|(_, p, m)| *p > 0 && m.is_none());
        let buttons = div()
            .flex()
            .gap(theme::ITEM_GAP)
            .child(widgets::button(
                "queue-prompt-start",
                "Start",
                true,
                cx.listener(|app, _: &ClickEvent, _, cx| {
                    app.answer_queue_prompt(true);
                    cx.notify();
                }),
            ))
            .child(widgets::button(
                "queue-prompt-cancel",
                "Cancel",
                true,
                cx.listener(|app, _: &ClickEvent, _, cx| {
                    app.answer_queue_prompt(false);
                    cx.notify();
                }),
            ));
        let body = div()
            .flex()
            .flex_col()
            .items_start()
            .gap(theme::LINE_GAP)
            .p(px(8.0))
            .child(format!(
                "{} on client {}{}",
                prompt.queue.name,
                prompt.pid,
                if prompt.queue.repeat { ", repeating" } else { "" }
            ))
            .child(grid)
            .when(unlimited, |this| {
                this.child(widgets::weak(
                    "Entries without a maximum time are billed for as long as they run.",
                ))
            })
            .child(widgets::weak("If a script's price or settings change, or a community script gets a new revision, the queue follows the option you picked."))
            .child(div().w_full().child(widgets::separator()))
            .child(buttons);
        let width = PROMPT_WIDTH
            .min(window.viewport_size().width - px(32.0))
            .max(px(0.0));
        Some(
            div()
                .id("queue-prompt")
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
                                .child("Start the queue?"),
                        )
                        .child(divider())
                        .child(body),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
fn queueable(title: &str, price: i64) -> StoreScript {
    StoreScript {
        queueable: true,
        ..super::store_page::fake_script(title, price)
    }
}

#[cfg(test)]
fn fake_manifest() -> Manifest {
    Manifest::parse(serde_json::json!({
        "queueable": true,
        "goals": [
            { "id": "kills", "label": "Kills" },
            { "id": "trips", "label": "Trips" }
        ],
        "settings": {
            "title": "Bosses",
            "fields": [
                { "kind": "label", "text": "Fight", "heading": 2 },
                { "kind": "choice", "id": "boss", "label": "Boss", "options": ["Giant mole", "Kalphite queen"], "default": "Giant mole" },
                { "kind": "number", "id": "eat_at", "label": "Eat at % health", "min": 0, "max": 100, "integer": true, "default": 45 },
                { "kind": "check", "id": "hop", "label": "Hop worlds when a player is near", "default": true },
                { "kind": "text", "id": "note", "label": "Note", "default": "bank at the castle" }
            ]
        }
    }))
    .expect("a manifest")
}

#[cfg(test)]
fn fake_queue() -> queue::Queue {
    let manifest = fake_manifest();
    let entry = |id: &str, title: &str| Entry {
        spec_hash: spec_hash(&manifest),
        ..Entry::new(id.into(), title.into())
    };
    queue::Queue {
        name: "Evening".into(),
        entries: vec![
            Entry {
                goal: Some(Goal {
                    counter: "kills".into(),
                    target: 150.0,
                }),
                min_minutes: Some(30),
                max_minutes: Some(120),
                ..entry("bosses", "Bosses")
            },
            Entry {
                max_minutes: Some(90),
                ..entry("miner", "Miner")
            },
            entry("tempoross", "Tempoross"),
        ],
        repeat: true,
        when_finished: WhenFinished::StayLoggedIn,
        on_change: OnChange::Skip,
    }
}

#[cfg(test)]
fn queue_page(app: &mut LoaderApp) {
    app.page = super::Page::Queue(4242);
    app.store.loaded = true;
    app.store.scripts = vec![
        queueable("Bosses", 400_000),
        queueable("Miner", 180_000),
        queueable("Tempoross", 0),
        super::store_page::fake_script("Not queueable", 0),
    ];
    app.targets = vec![super::store_page::fake_target(4242)];
    for id in ["bosses", "miner", "tempoross"] {
        app.manifests.insert(id.into(), (4, fake_manifest()));
    }
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("queue-empty", |app| {
            queue_page(app);
            app.store.scripts.retain(|s| !s.queueable);
        }),
        ("queue-entries", |app| {
            queue_page(app);
            app.queues.current = fake_queue();
            app.queues.saved = vec![fake_queue()];
            app.queue_page.add = Some("miner".into());
            app.queue_page.selected = Some(0);
        }),
        ("queue-running", |app| {
            queue_page(app);
            app.queues.current = fake_queue();
            let mut runner = Runner::new(fake_queue(), 4242, HashMap::new());
            runner.index = 1;
            runner.pass = 2;
            runner.started = Instant::now() - std::time::Duration::from_secs(47 * 60);
            app.queue_runners.push(runner);
        }),
        ("queue-prompt", |app| {
            queue_page(app);
            let queue = fake_queue();
            app.queues.current = queue.clone();
            app.queue_page.prompt = Some(Prompt {
                pid: 4242,
                rows: vec![
                    (
                        "Bosses".into(),
                        400_000,
                        queue.entries[0].most_cost(400_000),
                    ),
                    ("Miner".into(), 180_000, queue.entries[1].most_cost(180_000)),
                    ("Tempoross".into(), 0, None),
                ],
                approved: HashMap::new(),
                queue,
            });
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(queueable: bool, community: bool) -> StoreScript {
        serde_json::from_value(serde_json::json!({
            "id": "s1", "url": "s1", "title": "Bosses", "revision": 3,
            "can_access": true, "queueable": queueable, "community": community
        }))
        .unwrap()
    }

    fn check(script: StoreScript, manifest: Value, price: i64) -> Check {
        Check {
            me: serde_json::from_value(serde_json::json!({
                "username": "u", "microcredits": 1_000_000, "min_balance_minutes": 10, "api_version": 3
            }))
            .unwrap(),
            scripts: vec![script],
            manifests: HashMap::from([(
                "s1".to_owned(),
                Ok(ScriptManifest { revision: 4, manifest }),
            )]),
            quotes: HashMap::from([(
                "s1".to_owned(),
                Ok(crate::api::models::Quote { price, rank: 0, fee: 0, required: 0, requires_host: None }),
            )]),
            lib: Err("unused".into()),
        }
    }

    fn entry(manifest: &Value) -> Entry {
        let mut entry = Entry::new("s1".into(), "Bosses".into());
        entry.spec_hash = spec_hash(&Manifest::parse(manifest.clone()).unwrap());
        entry
    }

    #[test]
    fn a_queueable_script_with_unchanged_settings_is_ready_at_its_live_revision() {
        let manifest = serde_json::json!({ "queueable": true, "settings": { "fields": [{ "id": "a", "kind": "check" }] } });
        let ready = check(script(true, false), manifest.clone(), 500)
            .ready(&entry(&manifest), &Default::default())
            .unwrap();
        assert_eq!(ready.price, 500);
        assert_eq!(ready.script.revision, Some(4));
    }

    #[test]
    fn changes_since_the_entry_was_set_up_are_refused() {
        let manifest = serde_json::json!({ "queueable": true, "settings": { "fields": [{ "id": "a", "kind": "check" }] } });
        let old = entry(&serde_json::json!({ "queueable": true }));
        assert!(
            check(script(true, false), manifest.clone(), 0)
                .ready(&old, &Default::default())
                .unwrap_err()
                .contains("settings changed")
        );
        let not_queueable =
            serde_json::json!({ "settings": { "fields": [{ "id": "a", "kind": "check" }] } });
        assert!(
            check(script(true, false), not_queueable, 0)
                .ready(&entry(&manifest), &Default::default())
                .is_err()
        );
        assert!(
            check(script(false, false), manifest.clone(), 0)
                .ready(&entry(&manifest), &Default::default())
                .is_err()
        );
    }

    #[test]
    fn a_community_revision_runs_only_once_reviewed() {
        let manifest = serde_json::json!({ "queueable": true });
        let check = check(script(true, true), manifest.clone(), 0);
        let mut confirmed = crate::store::Confirmed::default();
        assert!(
            check
                .ready(&entry(&manifest), &confirmed)
                .unwrap_err()
                .contains("reviewed")
        );
        confirmed.confirm("s1", 4);
        assert!(check.ready(&entry(&manifest), &confirmed).is_ok());
    }

    #[test]
    fn a_script_gone_from_the_list_is_refused() {
        let manifest = serde_json::json!({ "queueable": true });
        let mut check = check(script(true, false), manifest.clone(), 0);
        check.scripts.clear();
        assert!(
            check
                .ready(&entry(&manifest), &Default::default())
                .unwrap_err()
                .contains("no longer available")
        );
    }
}
