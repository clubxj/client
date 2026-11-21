use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fmt::Write as _,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    thread,
    time::{Duration, Instant},
};

use eframe::egui;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::{
    AnyElement, App, AppContext, ClickEvent, ClipboardItem, Context, Div, ElementId, FocusHandle,
    FontWeight, ImageSource, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    PathBuilder, Pixels, Render, Rgba, Stateful, StatefulInteractiveElement, Styled,
    TitlebarOptions, Window, WindowBounds, WindowDecorations, WindowOptions, canvas, div, img,
    point, prelude::FluentBuilder, px, relative, size,
};
use injector::{Cursor, CursorStyle, DisplayName, GameState, LoadReport, Overlay, varp};
use serde::{Deserialize, Serialize};

use loader::backend::{self, Scan, Settings, Target};

use crate::account::{self, Auth};
use crate::api::{
    Api, ApiError, Config, Heartbeat, Me, Redeemed, Removed, StoreScript,
    models::{API_VERSION, Ended, Quote, RecentSession, Scored},
};
use crate::feedback;
use crate::highlight::Highlighter;
use crate::libs::Lib;
use crate::pipe::Rows;
use crate::script::{Launch, Script, ScriptEvent};
use crate::session::{self, Kept, Prepared};
use crate::store::{self, Confirmed, Store};
use crate::theme::{icon_button, icon_button_width, slide_button};
use crate::update;
use crate::storage::Storage;
use crate::wake::Wake;

mod accounts_page;
mod chrome;
mod clients;
mod development;
mod dialogs;
mod editor;
mod feedback_page;
mod output;
mod queue_page;
mod run_page;
mod sessions;
mod settings_page;
mod shared;
#[cfg(test)]
mod snapshots;
mod store_page;
mod terminal_page;

use chrome::*;
use output::*;
use shared::*;

enum Event {
    Scanned(Scan),
    Loaded {
        pid: u32,
        result: Result<LoadReport, String>,
    },
    Unloaded {
        pid: u32,
        result: Result<(), String>,
    },
    Done {
        pid: u32,
        what: &'static str,
        result: Result<(), String>,
    },
    State {
        pid: u32,
        read: Read,
        result: Result<GameState, String>,
    },
    Varps {
        pid: u32,
        read: Read,
        result: Result<Option<Vec<i32>>, String>,
    },
    XpState {
        run: u64,
        result: Result<GameState, String>,
    },
    AccountName(Result<Option<String>, String>),
    Name {
        pid: u32,
        name: String,
        result: Result<DisplayName, String>,
    },
    Tiles {
        pid: u32,
        result: Result<usize, String>,
    },
    Packets {
        pid: u32,
        result: Result<injector::PacketPage, String>,
    },
    PacketLayouts {
        pid: u32,
        result: Result<Option<Vec<injector::PacketLayout>>, String>,
    },
    ScriptNames(Result<crate::script_names::Names, String>),
    ScriptRuns {
        pid: u32,
        result: Result<crate::script_log::Poll, String>,
    },
    Vars {
        pid: u32,
        result: Result<crate::vars::Poll, String>,
    },
    Inventory {
        pid: u32,
        result: Result<crate::inventory::Poll, String>,
    },
    Icons {
        pictures: Vec<(crate::icons::Key, Option<injector::Image>)>,
        error: Option<String>,
    },
    LayoutRead {
        run: u64,
        kind: crate::layouts::Kind,
        result: Result<crate::layouts::Read, String>,
    },
    ItemsKnown(Result<Vec<(i32, String, bool)>, String>),
    Widgets {
        pid: u32,
        result: Result<crate::widgets::Poll, String>,
    },
    WidgetMouse {
        pid: u32,
        result: Result<Option<(i32, i32)>, String>,
    },
    Restored(Result<Option<account::Account>, ApiError>),
    SignedIn(Result<account::Account, String>),
    Me(Result<Me, ApiError>),
    Scripts(Result<Vec<StoreScript>, ApiError>),
    Redeemed(Result<Redeemed, ApiError>),
    GrantRemoved {
        id: String,
        result: Result<Removed, ApiError>,
    },
    StoreStarted {
        pid: u32,
        script: Box<StoreScript>,
        replaces: Option<u64>,
        run: Option<u64>,
        launch: Box<Launch>,
        result: Result<Prepared, String>,
    },
    RunQuoted {
        script: Box<StoreScript>,
        target: Box<Target>,
        run: Option<u64>,
        values: serde_json::Value,
        result: Result<session::Quoted, ApiError>,
    },
    ManifestFetched {
        script: Box<StoreScript>,
        target: Box<Target>,
        result: Result<crate::api::models::ScriptManifest, ApiError>,
    },
    QueueManifest {
        script_id: String,
        result: Result<crate::api::models::ScriptManifest, ApiError>,
    },
    QueueChecked {
        pid: u32,
        entry: bool,
        result: Result<Box<queue_page::Check>, ApiError>,
    },
    RestartQuoted {
        run: u64,
        result: Result<Option<session::Restart>, ApiError>,
    },
    Heartbeat {
        session_id: String,
        beat: Heartbeat,
    },
    SessionStopped {
        session_id: String,
        reason: String,
        sign_out: bool,
    },
    SessionEnded(Result<Ended, ApiError>),
    Update(Result<Option<update::Available>, update::CheckError>),
    UpdatePrepared {
        progress: Arc<update::Progress>,
        result: Result<update::Prepared, String>,
    },
    RecentSessions(Result<Vec<RecentSession>, ApiError>),
    Scored {
        session_id: String,
        result: Result<Scored, ApiError>,
    },
    Reported {
        session_id: String,
        result: Result<(), String>,
    },
    RunKept(Result<(), String>),
}

struct StoreRun {
    session_id: String,
    script_id: String,
    community: bool,
    closed: Receiver<()>,
    lib_version: String,
    price: i64,
    rank: i64,
    fee: i64,
    fee_charged: i64,
}

impl StoreRun {
    fn rate_per_hour(&self, minutes_per_month: i64) -> f64 {
        self.price as f64 + crate::api::models::fee_per_hour(self.fee, minutes_per_month)
    }
}

enum UpdateState {
    Offered,
    Downloading(Arc<update::Progress>),
    Installing,
    Failed(String),
}

fn progress_text(progress: &update::Progress) -> String {
    const MIB: u64 = 1024 * 1024;
    let done = progress.done.load(Ordering::Relaxed) / MIB;
    match progress.total.load(Ordering::Relaxed) / MIB {
        0 => format!("Downloading: {done} MB"),
        total => format!("Downloading: {done} of {total} MB"),
    }
}

#[derive(Clone)]
enum Source {
    File(ScriptFile),
    Store(StoreScript),
}

struct Run {
    id: u64,
    title: String,
    script: Option<Script>,
    source: Option<Source>,
    target: Option<Target>,
    store: Option<StoreRun>,
    file: Option<ScriptFile>,
    form: Option<crate::form::Form>,
    form_sent: Option<serde_json::Value>,
    last_breaks: Option<(injector::form::BreakPlan, Instant)>,
    exit: Option<injector::form::Exit>,
    finished: Option<(Result<(), String>, bool)>,
    antiban_sent: injector::antiban::Config,
    log: Log,
    xp: crate::xp::XpTracker,
    breaks: crate::breaks::BreaksTracker,
    counters: crate::counters::CountersTracker,
    started: Instant,
    ended: Option<Instant>,
    tools_tab: ToolsTab,
    names: Vec<String>,
    xp_next: Instant,
    xp_reading: bool,
    editors: crate::layouts::Editors,
}

impl Run {
    fn new(
        id: u64,
        title: String,
        script: Option<Script>,
        target: Option<Target>,
        store: Option<StoreRun>,
        settings: &Settings,
    ) -> Self {
        let now = Instant::now();
        Self {
            id,
            title,
            script,
            source: None,
            target,
            store,
            file: None,
            form: None,
            form_sent: None,
            last_breaks: None,
            exit: None,
            finished: None,
            antiban_sent: settings.antiban.clone(),
            log: Log::new(settings.log_lines),
            xp: Default::default(),
            breaks: Default::default(),
            counters: Default::default(),
            started: now,
            ended: None,
            tools_tab: ToolsTab::Settings,
            names: Vec::new(),
            xp_next: now,
            xp_reading: false,
            editors: Default::default(),
        }
    }

    fn is_running(&self) -> bool {
        self.script.as_ref().is_some_and(Script::is_running)
    }

    fn is_configuring(&self) -> bool {
        self.script.is_none()
    }

    fn stop(&mut self) {
        if let Some(script) = self.script.as_mut() {
            script.stop();
        }
    }

    fn stop_requested(&self) -> bool {
        self.script.as_ref().is_some_and(Script::stop_requested)
    }

    fn is_paused(&self) -> bool {
        self.script.as_ref().is_some_and(Script::is_paused)
    }

    fn send(&self, line: impl FnOnce(&Script) -> Result<(), String>) -> Result<(), String> {
        self.script
            .as_ref()
            .map_or_else(|| Err("The script isn't running yet.".into()), line)
    }

    fn run_time(&self) -> Duration {
        self.ended
            .unwrap_or_else(Instant::now)
            .saturating_duration_since(self.started)
    }

    fn loaded_target(&self) -> Option<&Target> {
        self.target.as_ref().filter(|t| t.client.loaded.is_some())
    }

    fn session_id(&self) -> Option<&str> {
        self.store.as_ref().map(|s| s.session_id.as_str())
    }

    fn info(&mut self, text: impl Into<String>) {
        self.log.push(false, text.into());
    }

    fn error(&mut self, text: impl Into<String>) {
        self.log.push(true, text.into());
    }

    fn report(&mut self, result: Result<(), String>, done: &str) {
        match result {
            Ok(()) => self.info(done),
            Err(error) => self.error(error),
        }
    }

    fn state(&self) -> RunState {
        if self.is_configuring() {
            RunState::Waiting
        } else if !self.is_running() {
            RunState::Ended
        } else if self.stop_requested() {
            RunState::Stopping
        } else if self.is_paused() {
            RunState::Paused
        } else {
            RunState::Running
        }
    }
}

#[derive(Clone, Copy)]
enum RunState {
    Running,
    Waiting,
    Paused,
    Stopping,
    Ended,
}

impl RunState {
    fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Paused => "paused",
            Self::Stopping => "stopping",
            Self::Ended => "ended",
        }
    }

    fn color(self) -> egui::Color32 {
        match self {
            Self::Running => crate::theme::GREEN,
            Self::Waiting => crate::theme::BLUE,
            Self::Paused => crate::theme::YELLOW,
            Self::Stopping => crate::theme::RED,
            Self::Ended => crate::theme::COMMENT,
        }
    }
}

const EXTRA_SESSION_FEE_HINT: &str = "Each script runs on its own client. Every script past the oldest pays the extra-session fee; the oldest runs without one.";

fn start_dir(path: &str) -> Option<&std::path::Path> {
    std::path::Path::new(path).parent().filter(|d| d.is_dir())
}

pub(crate) const ICON_START: &str = "\u{f04b}";
const ICON_PAUSE: &str = "\u{f04c}";
const ICON_STOP: &str = "\u{f04d}";
const ICON_CANCEL: &str = "\u{f00d}";
const ICON_RESTART: &str = "\u{f01e}";
const ICON_UPDATE: &str = "\u{f021}";
const ICON_EXPAND: &str = "\u{f077}";
const ICON_COLLAPSE: &str = "\u{f078}";
const ICON_CLEAR: &str = "\u{f1f8}";
const ICON_COPY: &str = "\u{f0c5}";
const ICON_SAVE: &str = "\u{f0c7}";
const ICON_SAVE_AS: &str = "\u{f0cfb}";
const ICON_OPEN: &str = "\u{f07c}";
const ICON_OPEN_LAST: &str = "\u{f1da}";
const ICON_PIPE: &str = "\u{f07e5}";
const ICON_OPEN_RUN: &str = "\u{f08e}";
const ICON_LOAD: &str = "\u{f1e6}";
const ICON_UNLOAD: &str = "\u{f052}";
const ICON_CONSOLE: &str = "\u{f120}";
const ICON_SIGN_OUT: &str = "\u{f08b}";

const PAUSE_BUTTONS: [(&str, &str); 2] = [(ICON_PAUSE, "Pause"), (ICON_START, "Resume")];

fn run_pause_label(paused: bool) -> (&'static str, &'static str) {
    PAUSE_BUTTONS[usize::from(paused)]
}

const RESTORING_SIGN_IN: &str = "Checking for a saved sign-in…";
const WAITING_FOR_BROWSER: &str = "Waiting for you to sign in in the browser…";

const SCRIPT_FILTER: &[&str] = &["luau", "lua"];

struct PendingStart {
    script: StoreScript,
    target: Target,
    replaces: Option<u64>,
    lib: Lib,
    run: Option<u64>,
    launch: Launch,
}

struct StartPrompt {
    start: PendingStart,
    price: i64,
    fee: i64,
    nth: i64,
    old_price: Option<i64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Clients,
    Store,
    Sessions,
    Queue,
    Run,
    Scripts,
    Development,
    Terminal,
    Accounts,
    Feedback,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DevTab {
    Tools,
    Inspectors,
}

impl DevTab {
    const ALL: [(DevTab, &'static str); 2] =
        [(DevTab::Tools, "Tools"), (DevTab::Inspectors, "Inspectors")];
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InspectorTab {
    Widget,
    Var,
    Script,
    Inventory,
    Packet,
}

impl InspectorTab {
    const ALL: [(InspectorTab, &'static str); 5] = [
        (InspectorTab::Widget, "Widget"),
        (InspectorTab::Var, "Var"),
        (InspectorTab::Script, "Script"),
        (InspectorTab::Inventory, "Inventory"),
        (InspectorTab::Packet, "Packet"),
    ];
}

fn sidebar_name(title: &str) -> String {
    const MAX: usize = 16;
    let name = title.to_lowercase();
    if name.chars().count() <= MAX {
        return name;
    }
    name.chars().take(MAX - 1).chain(['\u{2026}']).collect()
}

fn open_site(site_url: &str, path: &str) {
    if !site_url.is_empty() {
        let _ = webbrowser::open(&format!("{site_url}{path}"));
    }
}

const SIDEBAR_MIN_WINDOW: f32 = 760.0;
const SIDEBAR_WIDTH: f32 = 190.0;
const HEADER_COMPACT_WINDOW: f32 = 640.0;
const HEADER_STACK_WINDOW: f32 = 400.0;
const HEADER_TINY_WINDOW: f32 = 200.0;
const SIDEBAR_HANDLE_WIDTH: f32 = 18.0;
const CLIENT_PREFIX_WIDTH: f32 = 130.0;
const CLIENT_STATUS_WIDTH: f32 = 220.0;

#[derive(Clone, Copy)]
enum ClientAction {
    Load,
    Unload,
    Console,
}

const DEV_TOGGLES: [&str; 24] = [
    "Players",
    "NPCs",
    "Ground Items",
    "Ground Objects",
    "Game Objects",
    "Grid",
    "Spot Animations",
    "Walls",
    "Decorations",
    "Projectiles",
    "Location",
    "Tile Location",
    "Camera Position",
    "Zone Borders",
    "Map Squares",
    "Loading Lines",
    "Line Of Sight",
    "Valid Movement",
    "Player Path",
    "Movement Flags",
    "Interacting",
    "Sound Effects",
    "Tile flags",
    "World Entities",
];

const DEV_COLUMNS: usize = 4;

const CURSOR_STYLES: [CursorStyle; 4] = [
    CursorStyle::Off,
    CursorStyle::Cross,
    CursorStyle::X,
    CursorStyle::Dot,
];

pub(crate) const DEFAULT_CURSOR: Cursor = Cursor {
    style: CursorStyle::Cross,
    color: [255, 0, 0],
    size: 20,
};

fn cursor_style_label(style: CursorStyle) -> &'static str {
    match style {
        CursorStyle::Off => "Off",
        CursorStyle::Cross => "Cross",
        CursorStyle::Dot => "Dot",
        CursorStyle::X => "X",
    }
}

const DEV_LAYERS: [Option<&str>; DEV_TOGGLES.len()] = [
    Some("players"),
    Some("npcs"),
    Some("ground-items"),
    Some("ground-objects"),
    Some("objects"),
    Some("grid"),
    Some("spot-anims"),
    Some("walls"),
    Some("decorations"),
    Some("projectiles"),
    Some("location"),
    Some("tile-location"),
    Some("camera"),
    Some("zone-borders"),
    Some("map-squares"),
    Some("loading-lines"),
    Some("line-of-sight"),
    Some("valid-movement"),
    Some("player-path"),
    Some("movement-flags"),
    Some("interacting"),
    None,
    Some("tile-flags"),
    Some("world-entities"),
];
const TILES_GIVE_UP: Duration = Duration::from_secs(10);
const TILES_RETRY: Duration = Duration::from_millis(50);

struct TilesWorker {
    pid: u32,
    layers: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl TilesWorker {
    fn start(
        wake: Wake,
        tx: Sender<Event>,
        target: Target,
        settings: Settings,
        layers: Vec<String>,
    ) -> Self {
        let pid = target.client.pid;
        let mut worker = Self {
            pid,
            layers: Arc::new(Mutex::new(layers)),
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        };
        let (layers, stop) = (worker.layers.clone(), worker.stop.clone());
        worker.thread = Some(thread::spawn(move || {
            let mut frame = 0;
            let mut failing: Option<Instant> = None;
            while !stop.load(Ordering::Relaxed) {
                let current = layers.lock().map(|l| l.clone()).unwrap_or_default();
                match backend::scene_tiles_next_frame(
                    &target,
                    &settings,
                    &current.iter().map(String::as_str).collect::<Vec<_>>(),
                    frame,
                ) {
                    Ok(Some(painted)) => {
                        frame = painted;
                        failing = None;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let since = *failing.get_or_insert_with(|| {
                            let _ = backend::draw(&target, &settings, &Overlay::Clear);
                            Instant::now()
                        });
                        if since.elapsed() > TILES_GIVE_UP {
                            let _ = tx.send(Event::Tiles {
                                pid,
                                result: Err(error),
                            });
                            wake.wake();
                            break;
                        }
                        thread::sleep(TILES_RETRY);
                    }
                }
            }
            let _ = backend::draw(&target, &settings, &Overlay::Clear);
        }));
        worker
    }

    fn finish(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for TilesWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

const SCRIPT_KEY: &str = "script";
const LAST_SCRIPT_KEY: &str = "last_script";
const FORMS_KEY: &str = "forms";
const LAYOUTS_KEY: &str = "layouts";
pub(crate) const CURSOR_KEY: &str = "cursor";
const UI_FONT_KEY: &str = "ui_font";
const CONFIRMED_KEY: &str = "community_confirmed";
pub(crate) const HTTP_WHITELIST_KEY: &str = "http_whitelist";

const XP_READ_EVERY: Duration = Duration::from_secs(5);
const SAVE_EVERY: Duration = Duration::from_secs(30);

const EXAMPLE_SCRIPT: &str = r#"local GameState = require("game_state")
local Login = require("login")

local function current_state()
  local state, err = game_state()
  assert(state ~= nil, err)
  return state
end

local state = current_state()
while state.game_state ~= GameState.LOGGED_IN do
  local ok, err = Login.login()
  assert(ok ~= nil, err)
  sleep(1000)
  state = current_state()
end

local total_xp = 0
local hp = nil
for _, skill in ipairs(state.skills) do
  total_xp += skill.xp
  if skill.name == "Hitpoints" then
    hp = skill.level
  end
end
print("total xp: ", total_xp)
print("hp: ", hp)
"#;

#[derive(Default)]
struct Find {
    open: bool,
    replace: bool,
    query: String,
    replacement: String,
    case_sensitive: bool,
    current: usize,
    jump: bool,
    focus: bool,
}

fn find_matches(text: &str, query: &str, case_sensitive: bool) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }
    if case_sensitive {
        return text.match_indices(query).map(|(at, _)| at).collect();
    }
    let (text, query) = (text.as_bytes(), query.as_bytes());
    let mut found = Vec::new();
    let mut at = 0;
    while at + query.len() <= text.len() {
        if text[at..at + query.len()].eq_ignore_ascii_case(query) {
            found.push(at);
            at += query.len();
        } else {
            at += 1;
        }
    }
    found
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
#[derive(Clone)]
struct ScriptFile {
    path: String,
    text: String,
    disk: Option<String>,
}

impl Default for ScriptFile {
    fn default() -> Self {
        Self {
            path: String::new(),
            text: EXAMPLE_SCRIPT.into(),
            disk: None,
        }
    }
}

impl ScriptFile {
    fn name(&self) -> String {
        std::path::Path::new(&self.path).file_name().map_or_else(
            || "script".into(),
            |name| name.to_string_lossy().into_owned(),
        )
    }

    fn changed_on_disk(&self) -> Option<String> {
        let disk = std::fs::read_to_string(&self.path).ok()?;
        match &self.disk {
            Some(known) => (&disk != known).then_some(disk),
            None => (disk != self.text).then_some(disk),
        }
    }
}

struct ReloadPrompt {
    disk: String,
    restart: Option<u64>,
}

fn default_script_path() -> Option<std::path::PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let sibling =
        update::is_dev_install().then(|| exe_dir.join("../../../scripts/scripts/default.luau"));
    std::iter::once(exe_dir.join("scripts/default.luau"))
        .chain(sibling)
        .find(|path| path.is_file())
}

fn load_default_script() -> Option<ScriptFile> {
    let path = default_script_path()?;
    let text = std::fs::read_to_string(&path).ok()?;
    Some(ScriptFile {
        path: path.display().to_string(),
        disk: Some(text.clone()),
        text,
    })
}

const ACTIVE_PRAYERS: usize = 83;

const KNOWN_VARPS: [(&str, usize); 11] = [
    ("Attack style", varp::ATTACK_STYLE),
    ("Active prayers", ACTIVE_PRAYERS),
    ("Quest points", varp::QUEST_POINTS),
    ("Poison", varp::POISON),
    ("Run", varp::RUN),
    ("Tutorial progress", 281),
    ("Special attack", varp::SPECIAL_ATTACK),
    ("Special attack selected", varp::SPECIAL_ATTACK_ENABLED),
    ("Slayer task size", 394),
    ("Slayer task creature", 395),
    ("Last home teleport", 892),
];

fn paired<K: Copy, V, C: FromIterator<(K, V)>>(ids: &[K], values: Vec<Option<V>>) -> C {
    ids.iter()
        .zip(values)
        .filter_map(|(&id, value)| Some((id, value?)))
        .collect()
}

fn item_info<N, S>(target: &Target, settings: &Settings, ids: &[i32]) -> Result<(N, S), String>
where
    N: FromIterator<(i32, String)>,
    S: FromIterator<(i32, bool)>,
{
    let names = backend::item_names(target, settings, ids)?;
    let stacks = backend::item_stackable(target, settings, ids)?;
    Ok((paired(ids, names), paired(ids, stacks)))
}

fn missed(dropped: impl Into<u64>) -> String {
    let dropped = dropped.into();
    if dropped > 0 {
        format!(", {dropped} missed")
    } else {
        String::new()
    }
}

fn known_varp(id: usize) -> Option<&'static str> {
    KNOWN_VARPS
        .iter()
        .find(|&&(_, known)| known == id)
        .map(|&(name, _)| name)
}

fn describe_varp(id: usize, value: i32) -> Option<String> {
    Some(match id {
        varp::RUN | varp::SPECIAL_ATTACK_ENABLED => (if value == 1 { "on" } else { "off" }).into(),
        varp::SPECIAL_ATTACK => format!("{}%", value / 10),
        varp::POISON if value >= 1_000_000 => "envenomed".into(),
        varp::POISON if value > 0 => "poisoned".into(),
        varp::POISON => "healthy".into(),
        ACTIVE_PRAYERS => format!("bits {value:#b}"),
        _ => return None,
    })
}

#[derive(Clone, Copy)]
enum Read {
    GameState,
    Position,
    Chunk,
    Skills,
    TickCount,
    Login,
    RootInterface,
    Varp(usize),
    KnownVarps,
    VarpChanges,
}

impl Read {
    fn is_varp(self) -> bool {
        matches!(self, Read::Varp(_) | Read::KnownVarps | Read::VarpChanges)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tone {
    Normal,
    Warning,
    Error,
}

struct LogLine {
    tone: Tone,
    text: String,
    tag: Option<usize>,
}

#[derive(Default)]
struct Log {
    lines: Vec<LogLine>,
    tags: Vec<String>,
    tag_index: HashMap<String, usize>,
    filter: Option<usize>,
    view: Vec<usize>,
    text: String,
    revision: u64,
    keep: usize,
    pipe: Option<crate::pipe::Pipe>,
}

impl Log {
    fn new(keep: usize) -> Self {
        Self {
            keep,
            ..Self::default()
        }
    }

    fn set_keep(&mut self, keep: usize) {
        self.keep = keep;
        if self.lines.len() > keep {
            self.lines.drain(..self.lines.len() - keep);
            self.rebuild();
        }
    }

    fn push(&mut self, error: bool, text: String) {
        self.push_tone(if error { Tone::Error } else { Tone::Normal }, text);
    }

    fn push_tone(&mut self, tone: Tone, text: String) {
        let tag = line_tag(&text).map(|name| match self.tag_index.get(name) {
            Some(&index) => index,
            None => {
                let index = self.tags.len();
                self.tags.push(name.to_owned());
                self.tag_index.insert(name.to_owned(), index);
                index
            }
        });
        if self.filter.is_none() || self.filter == tag {
            self.show(self.lines.len(), &text);
        }
        let piped = crate::pipe::write(&mut self.pipe, [text.as_str()]);
        self.lines.push(LogLine { tone, text, tag });
        if self.lines.len() > self.keep + self.keep / 4 {
            self.lines.drain(..self.lines.len() - self.keep);
            self.rebuild();
        }
        if let Err(error) = piped {
            self.push_tone(Tone::Error, error);
        }
    }

    fn show(&mut self, index: usize, text: &str) {
        if !self.view.is_empty() {
            self.text.push('\n');
        }
        self.text.push_str(text);
        self.view.push(index);
        self.revision += 1;
    }

    fn last(&self) -> Option<&LogLine> {
        self.view.last().map(|&index| &self.lines[index])
    }

    fn texts(&self) -> Vec<String> {
        self.lines.iter().map(|line| line.text.clone()).collect()
    }

    fn set_filter(&mut self, filter: Option<usize>) {
        if self.filter == filter {
            return;
        }
        self.filter = filter;
        self.rebuild();
    }

    fn rebuild(&mut self) {
        self.view.clear();
        self.text.clear();
        self.revision += 1;
        let lines = std::mem::take(&mut self.lines);
        for (index, line) in lines.iter().enumerate() {
            if self.filter.is_none() || self.filter == line.tag {
                self.show(index, &line.text);
            }
        }
        self.lines = lines;
    }

    fn layout_job(
        &self,
        font: egui::FontId,
        normal: egui::Color32,
        warning: egui::Color32,
        error: egui::Color32,
    ) -> egui::text::LayoutJob {
        let mut job = egui::text::LayoutJob {
            text: self.text.clone(),
            ..Default::default()
        };
        let mut section = |range: std::ops::Range<usize>, tone: Tone| {
            job.sections.push(egui::text::LayoutSection {
                leading_space: 0.0,
                byte_range: egui::text::ByteIndex(range.start)..egui::text::ByteIndex(range.end),
                format: egui::TextFormat::simple(
                    font.clone(),
                    match tone {
                        Tone::Normal => normal,
                        Tone::Warning => warning,
                        Tone::Error => error,
                    },
                ),
            });
        };
        let mut start = 0;
        let mut run: Option<(usize, Tone)> = None;
        for (i, &index) in self.view.iter().enumerate() {
            let line = &self.lines[index];
            if let Some((from, was)) = run.filter(|&(_, was)| was != line.tone) {
                section(from..start, was);
                run = None;
            }
            run.get_or_insert((start, line.tone));
            start += line.text.len() + usize::from(i + 1 < self.view.len());
        }
        if let Some((from, was)) = run {
            section(from..self.text.len(), was);
        }
        job
    }

    fn entry_of_line(&self) -> Vec<usize> {
        self.view
            .iter()
            .enumerate()
            .flat_map(|(entry, &index)| {
                std::iter::repeat_n(entry, self.lines[index].text.matches('\n').count() + 1)
            })
            .collect()
    }

    fn clear(&mut self) {
        *self = Self {
            revision: self.revision + 1,
            pipe: self.pipe.take(),
            ..Self::new(self.keep)
        };
    }

    fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

fn line_tag(text: &str) -> Option<&str> {
    let rest = text.strip_prefix('[')?;
    let end = rest.find(']')?;
    (end > 0 && end <= 32 && !rest[..end].contains('[')).then(|| &rest[..end])
}

const SELECTION_SCROLL_MAX: f32 = 2400.0;
const SELECTION_SCROLL_RAMP: f32 = 200.0;

fn selection_scroll_speed(pointer: f32, visible: egui::Rangef, window: egui::Rangef) -> f32 {
    let past = pointer - pointer.clamp(visible.min, visible.max);
    let room = if past < 0.0 {
        visible.min - window.min
    } else {
        window.max - visible.max
    };
    let share = (past.abs() / room.clamp(1.0, SELECTION_SCROLL_RAMP)).min(1.0);
    past.signum() * share * SELECTION_SCROLL_MAX
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolsTab {
    Settings,
    Equipment,
    Inventory,
    Prayer,
    Xp,
    Breaks,
    Feedback,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BreaksTab {
    Breaks,
    Sleeps,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LogTab {
    Loader,
    Run(u64),
}

pub struct LoaderApp {
    settings: Settings,
    tab: Tab,
    sidebar_open: bool,
    terminal: Option<crate::terminal::Terminal>,
    display_name: String,
    varp: usize,
    varps: Option<Vec<i32>>,
    script_file: ScriptFile,
    last_script: Option<String>,
    find: Find,
    highlighter: Highlighter,
    fold: crate::fold::Folds,
    runs: Vec<Run>,
    active_run: Option<u64>,
    next_run: u64,
    run_target: Option<String>,
    start_prompt: Option<StartPrompt>,
    reload_prompt: Option<ReloadPrompt>,
    remembered_forms: HashMap<String, serde_json::Map<String, serde_json::Value>>,
    manifests: HashMap<String, (i64, injector::manifest::Manifest)>,
    queues: crate::queue::Saved,
    queue_runners: Vec<crate::queue::Runner>,
    queue_page: queue_page::Page,
    run_ui: run_page::Ui,
    dev_toggles: [bool; DEV_TOGGLES.len()],
    cursor: Cursor,
    ui_font: crate::theme::UiFont,
    dev_tab: DevTab,
    inspector_tab: InspectorTab,
    tiles: Option<TilesWorker>,
    packets: crate::packets::Monitor,
    vars: crate::vars::Inspector,
    script_inspector: crate::script_log::Inspector,
    inventory: crate::inventory::Inspector,
    widgets: crate::widgets::Inspector,
    api: Api,
    auth: Auth,
    store: Store,
    accounts: Arc<RwLock<crate::accounts::Accounts>>,
    http_whitelist: Arc<RwLock<crate::whitelist::Whitelist>>,
    whitelist_page: crate::whitelist::Page,
    confirmed: Confirmed,
    warning: Option<PendingStart>,
    update: Option<update::Available>,
    update_state: UpdateState,
    instance: update::Instance,
    starting: Option<u32>,
    targets: Vec<Target>,
    dll_version: Option<String>,
    selected: Option<String>,
    busy: Option<&'static str>,
    log_tab: LogTab,
    log_collapsed: bool,
    followed_tab: (Tab, Option<u64>),
    loader_log: Log,
    log_job: Option<LogJob>,
    breaks_tab: BreaksTab,
    feedback: feedback::Page,
    gpu: Option<String>,
    icons: crate::icons::Icons,
    layouts: crate::layouts::AllLayouts,
    layouts_unreadable: bool,
    known_items: crate::layouts::Known,
    asking_items: Vec<i32>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    wake: Wake,
    storage: Storage,
    last_save: Instant,
    quit: bool,
    copy: Option<String>,
    chrome: Chrome,
}

fn selected_in<'a>(targets: &'a [Target], selected: Option<&str>) -> Option<&'a Target> {
    let key = selected?;
    targets
        .iter()
        .find(|t| t.key() == key && t.client.foreign_bot.is_none())
}

fn loaded_in<'a>(targets: &'a [Target], selected: Option<&str>) -> Option<&'a Target> {
    selected_in(targets, selected).filter(|t| t.client.loaded.is_some())
}

fn run_in(runs: &mut [Run], id: Option<u64>) -> Option<&mut Run> {
    runs.iter_mut().find(|r| Some(r.id) == id)
}

fn editors_of(runs: &mut [Run], id: Option<u64>) -> Option<&mut crate::layouts::Editors> {
    run_in(runs, id).map(|r| &mut r.editors)
}

static EMPTY_LAYOUTS: std::sync::LazyLock<crate::layouts::ScriptLayouts> =
    std::sync::LazyLock::new(Default::default);

static EMPTY_RULES: crate::layouts::Rules = crate::layouts::Rules {
    required: Vec::new(),
    locked: Vec::new(),
};

fn rules_in(
    form: &Option<crate::form::Form>,
    kind: crate::layouts::Kind,
) -> &crate::layouts::Rules {
    form.as_ref().map_or(&EMPTY_RULES, |form| form.rules(kind))
}

impl LoaderApp {
    fn new(
        storage: Storage,
        config: Config,
        instance: update::Instance,
        wake: Wake,
        gpu: Option<String>,
        chrome: Chrome,
    ) -> Self {
        let mut settings: Settings = storage
            .get(crate::storage::SETTINGS_KEY)
            .unwrap_or_default();
        settings.fill_missing(update::is_dev_install());
        let data_dir = crate::paths::project_dirs().map(|dirs| dirs.data_dir().to_path_buf());
        let (layouts, layouts_unreadable) =
            crate::layouts::load(&storage, LAYOUTS_KEY, data_dir.as_deref());
        let (tx, rx) = channel();
        let loader_log = Log::new(settings.log_lines);
        let mut app = Self {
            settings,
            tab: Tab::Clients,
            sidebar_open: false,
            terminal: None,
            display_name: String::new(),
            varp: varp::RUN,
            varps: None,
            script_file: load_default_script()
                .or_else(|| storage.get(SCRIPT_KEY))
                .unwrap_or_default(),
            last_script: storage.get(LAST_SCRIPT_KEY).flatten(),
            find: Find::default(),
            highlighter: Highlighter::default(),
            fold: crate::fold::Folds::default(),
            reload_prompt: None,
            remembered_forms: storage.get(FORMS_KEY)
                .unwrap_or_default(),
            manifests: HashMap::new(),
            queues: storage.get(crate::queue::STORAGE_KEY)
                .unwrap_or_default(),
            queue_runners: Vec::new(),
            queue_page: queue_page::Page::default(),
            run_ui: run_page::Ui::default(),
            dev_toggles: [false; DEV_TOGGLES.len()],
            cursor: storage.get(CURSOR_KEY)
                .unwrap_or(DEFAULT_CURSOR),
            ui_font: storage.get(UI_FONT_KEY).unwrap_or_default(),
            dev_tab: DevTab::Tools,
            inspector_tab: InspectorTab::Widget,
            tiles: None,
            packets: crate::packets::Monitor::default(),
            vars: crate::vars::Inspector::default(),
            script_inspector: crate::script_log::Inspector::default(),
            inventory: crate::inventory::Inspector::default(),
            widgets: crate::widgets::Inspector::new(),
            api: Api::new(config.clone()),
            auth: Auth::Restoring,
            store: Store::default(),
            accounts: Arc::new(RwLock::new(crate::accounts::Accounts::load())),
            http_whitelist: Arc::new(RwLock::new(crate::whitelist::Whitelist::new(
                storage.get(HTTP_WHITELIST_KEY)
                    .unwrap_or_default(),
                &config,
            ))),
            whitelist_page: Default::default(),
            confirmed: storage.get(CONFIRMED_KEY)
                .unwrap_or_default(),
            warning: None,
            runs: Vec::new(),
            active_run: None,
            next_run: 1,
            run_target: None,
            start_prompt: None,
            update: None,
            update_state: UpdateState::Offered,
            instance,
            starting: None,
            targets: Vec::new(),
            dll_version: None,
            selected: None,
            busy: None,
            log_tab: LogTab::Loader,
            log_collapsed: false,
            followed_tab: (Tab::Clients, None),
            breaks_tab: BreaksTab::Breaks,
            icons: Default::default(),
            layouts,
            layouts_unreadable: layouts_unreadable.is_some(),
            known_items: Default::default(),
            asking_items: Vec::new(),
            feedback: feedback::Page::default(),
            gpu,
            loader_log,
            log_job: None,
            tx,
            rx,
            wake,
            storage,
            last_save: Instant::now(),
            quit: false,
            copy: None,
            chrome,
        };
        if let Some(message) = layouts_unreadable {
            app.error(message);
        }
        app
    }

    fn start(&mut self) {
        self.scan();
        thread::spawn(feedback::prune);
        self.restore_sign_in();
        self.api_job(|api| Event::Update(update::check(&api)));
        if let Some(failure) = update::failure_from_args() {
            self.error(failure.clone());
            self.update_state = UpdateState::Failed(failure);
        }
    }

    fn end_session_job(&self, session_id: String) {
        self.api_job(move |api| {
            Event::SessionEnded(api.end_session(&session_id))
        });
    }

    fn next_run_id(&mut self) -> u64 {
        let id = self.next_run;
        self.next_run += 1;
        id
    }

    fn api_job(&self, job: impl FnOnce(Api) -> Event + Send + 'static) {
        let (tx, wake, api) = (self.tx.clone(), self.wake.clone(), self.api.clone());
        thread::spawn(move || {
            let _ = tx.send(job(api));
            wake.wake();
        });
    }

    fn restore_sign_in(&mut self) {
        self.auth = Auth::Restoring;
        self.api_job(|api| Event::Restored(account::restore(&api)));
    }

    fn start_sign_in(&mut self) {
        let cancel = Arc::new(AtomicBool::new(false));
        self.auth = Auth::SigningIn(cancel.clone());
        self.info(WAITING_FOR_BROWSER);
        self.api_job(move |api| {
            Event::SignedIn(account::sign_in(&api, &cancel))
        });
    }

    fn stop_all(&mut self) {
        self.queue_runners.clear();
        let deadline = Instant::now() + Duration::from_secs(3);
        for run in &mut self.runs {
            run.stop();
        }
        for store in self.runs.iter().filter_map(|r| r.store.as_ref()) {
            let left = deadline.saturating_duration_since(Instant::now());
            let _ = store.closed.recv_timeout(left);
        }
    }

    fn sign_out(&mut self, why: Option<String>) {
        self.stop_all();
        self.api.sign_out();
        self.auth = Auth::SignedOut;
        self.store = Store::default();
        self.warning = None;
        match why {
            Some(why) => self.error(format!("Signed out: {why}")),
            None => self.info("Signed out."),
        }
    }

    fn signed_in(&mut self, account: account::Account) {
        let me = account.me;
        self.info(format!("Signed in as {}.", me.username));
        if me.api_version > API_VERSION {
            self.error("This launcher is too old for the server. Update it to run store scripts.");
        }
        self.auth = Auth::SignedIn(me);
        self.scripts_loaded(account.scripts);
    }

    fn scripts_loaded(&mut self, result: Result<Vec<StoreScript>, ApiError>) {
        self.store.loading = false;
        match result {
            Ok(scripts) => {
                self.store.scripts = scripts;
                self.store.loaded = true;
            }
            Err(ApiError::SignedOut(why)) => self.sign_out(Some(why)),
            Err(error) => self.store.error = Some(format!("Couldn't load the scripts: {error}")),
        }
    }

    fn store_changed<T>(
        &mut self,
        result: Result<T, ApiError>,
        done: &str,
    ) -> bool {
        match result {
            Ok(_) => {
                self.store.redeem_result = Some((true, done.into()));
                self.fetch_scripts();
                true
            }
            Err(ApiError::SignedOut(why)) => {
                self.sign_out(Some(why));
                false
            }
            Err(error) => {
                self.store.redeem_result = Some((false, error.to_string()));
                false
            }
        }
    }

    fn start_failed(&mut self, title: &str, error: impl std::fmt::Display) {
        self.script_error(format!("Couldn't start {title}: {error}"));
        self.fetch_scripts();
    }

    fn fetch_scripts(&mut self) {
        self.store.loading = true;
        self.store.error = None;
        let community = self.store.community;
        self.api_job(move |api| {
            Event::Scripts(api.scripts("", "", community))
        });
    }

    fn run_block(&self, target: Option<&Target>) -> Option<String> {
        if self
            .auth
            .me()
            .is_some_and(|me| me.api_version > API_VERSION)
        {
            return Some("Update the launcher to run store scripts.".into());
        }
        if self.starting.is_some() {
            return Some("A script is starting.".into());
        }
        if self.start_prompt.is_some() || self.warning.is_some() {
            return Some("Answer the open dialog first.".into());
        }
        target
            .is_none()
            .then(|| "Pick a client with the agent loaded that isn't running a script.".into())
    }

    fn client_free(&self, target: &Target) -> bool {
        target.client.loaded.is_some()
            && target.client.foreign_bot.is_none()
            && target.client.is_64bit
            && !self.client_in_use(target.client.pid)
    }

    fn free_client(&self, pid: u32) -> Option<&Target> {
        self.targets
            .iter()
            .find(|t| t.client.pid == pid && self.client_free(t))
    }

    fn restart_refusal(&self, run: &Run) -> Option<String> {
        if run.store.is_none() {
            return self.file_restart_refusal(run);
        }
        if self.auth.me().is_none() {
            return Some("Sign in to run store scripts.".into());
        }
        if let Some(why) = self.run_block(run.target.as_ref()) {
            return Some(why);
        }
        run.target
            .as_ref()
            .and_then(|t| self.free_client(t.client.pid))
            .is_none()
            .then(|| "Its client is gone or busy. Start the script from the Scripts page.".into())
    }

    fn file_restart_refusal(&self, run: &Run) -> Option<String> {
        if self.reload_prompt.is_some() || self.start_prompt.is_some() || self.warning.is_some() {
            return Some("Answer the open dialog first.".into());
        }
        if self
            .runs
            .iter()
            .any(|r| r.store.is_none() && r.is_running())
        {
            return Some("A script from the editor is running.".into());
        }
        run.target
            .as_ref()
            .is_some_and(|t| self.free_client(t.client.pid).is_none())
            .then(|| "Its client is gone or busy. Run the script from the editor.".into())
    }

    fn restart_run(&mut self, id: u64) {
        let Some(run) = self.run_by_id(id) else {
            return;
        };
        let Some(store) = &run.store else {
            self.restart_file(id);
            return;
        };
        let (session_id, script_id, community) = (
            store.session_id.clone(),
            store.script_id.clone(),
            store.community,
        );
        let cached_revision = self
            .manifests
            .get(&script_id)
            .map(|(revision, _)| *revision);
        self.starting = run.target.as_ref().map(|t| t.client.pid);
        self.active_run = Some(id);
        self.log_tab = LogTab::Run(id);
        self.remember_form(id);
        self.api_job(move |api| Event::RestartQuoted {
            run: id,
            result: session::restart_quote(
                &api,
                session::cache_root().as_deref(),
                &session_id,
                &script_id,
                community,
                cached_revision,
            ),
        });
    }

    fn restart_quoted(&mut self, id: u64, restart: session::Restart) {
        let session::Restart {
            mut script,
            quoted: session::Quoted { me, quote, lib },
            manifest,
        } = restart;
        self.refresh_me(me);
        let lib = match lib {
            Ok(lib) => lib,
            Err(error) => return self.script_error(format!("Couldn't restart: {error}")),
        };
        if let Some(fetched) = manifest {
            match injector::manifest::Manifest::parse(fetched.manifest) {
                Ok(manifest) => {
                    self.manifests
                        .insert(script.id.clone(), (fetched.revision, manifest));
                    script.revision = Some(fetched.revision);
                }
                Err(error) => return self.script_error(format!("Couldn't restart: {error}")),
            }
        }
        let Some((_, manifest)) = self.manifests.get(&script.id).cloned() else {
            return self.script_error("Couldn't restart: the script's settings couldn't be read.");
        };
        let Some(run) = self.run_by_id(id) else {
            return;
        };
        let (Some(old_price), Some(target)) =
            (run.store.as_ref().map(|s| s.price), run.target.clone())
        else {
            return;
        };
        let values = match (&manifest.settings, self.form_values(id)) {
            (None, _) => serde_json::json!({}),
            (Some(spec), Some((ran_with, values))) if *spec == ran_with => values,
            (Some(_), _) => {
                let title = script.title.clone();
                self.configure_store_script(script, target, &manifest);
                self.close_run(id);
                return self.script_error(format!(
                    "{title}'s settings changed since it last ran. Check them, then press Start."
                ));
            }
        };
        let start = PendingStart {
            script,
            target,
            replaces: Some(id),
            lib,
            run: None,
            launch: Launch::with(values),
        };
        self.start_at_quote(start, Some(old_price), quote);
    }

    fn form_values(&self, id: u64) -> Option<(serde_json::Value, serde_json::Value)> {
        let form = self.run_by_id(id)?.form.as_ref()?;
        let layouts = self.layouts.get(&form.title).unwrap_or(&EMPTY_LAYOUTS);
        let crate::form::Outcome::Submitted(values) = form.submitted(layouts, &self.known_items)
        else {
            return None;
        };
        Some((form.spec().clone(), values))
    }

    fn quote_store_script(
        &mut self,
        script: StoreScript,
        target: Target,
        run: Option<u64>,
        values: serde_json::Value,
    ) {
        self.starting = Some(target.client.pid);
        self.api_job(move |api| Event::RunQuoted {
            result: session::quote(&api, session::cache_root().as_deref(), &script.id),
            script: Box::new(script),
            target: Box::new(target),
            run,
            values,
        });
    }

    fn open_store_script(&mut self, script: StoreScript, target: Target) {
        match script.revision.and_then(|revision| {
            self.manifests
                .get(&script.id)
                .filter(|(r, _)| *r == revision)
        }) {
            Some((_, manifest)) => {
                let manifest = manifest.clone();
                self.configure_store_script(script, target, &manifest);
            }
            None => {
                self.starting = Some(target.client.pid);
                let id = script.id.clone();
                self.api_job(move |api| Event::ManifestFetched {
                    result: api.manifest(&id),
                    script: Box::new(script),
                    target: Box::new(target),
                });
            }
        }
    }

    fn manifest_fetched(
        &mut self,
        script: StoreScript,
        target: Target,
        fetched: crate::api::models::ScriptManifest,
    ) {
        match injector::manifest::Manifest::parse(fetched.manifest) {
            Ok(manifest) => {
                self.manifests
                    .insert(script.id.clone(), (fetched.revision, manifest.clone()));
                let script = StoreScript {
                    revision: Some(fetched.revision),
                    ..script
                };
                self.configure_store_script(script, target, &manifest);
            }
            Err(error) => self.start_failed(&script.title, error),
        }
    }

    fn configure_store_script(
        &mut self,
        script: StoreScript,
        target: Target,
        manifest: &injector::manifest::Manifest,
    ) {
        let Some(spec) = manifest.settings.clone() else {
            return self.quote_store_script(script, target, None, serde_json::json!({}));
        };
        let title = script.title.clone();
        let id = self.next_run_id();
        let mut run = Run::new(id, title.clone(), None, Some(target), None, &self.settings);
        run.source = Some(Source::Store(script));
        run.form = Some(self.settings_form(spec, &title));
        run.info(format!(
            "Set up {title}, then press Start. Nothing is billed until it starts."
        ));
        self.add_run(run);
    }

    fn settings_form(&self, spec: serde_json::Value, title: &str) -> crate::form::Form {
        let mut form = crate::form::Form::new(spec, self.remembered_forms.get(title));
        form.title = title.to_owned();
        form
    }

    fn refresh_me(&mut self, me: Me) {
        if let Some(current) = self.auth.me_mut() {
            *current = me;
        }
    }

    fn start_at_quote(
        &mut self,
        start: PendingStart,
        old_price: Option<i64>,
        quote: Quote,
    ) {
        let microcredits = self.auth.me().map_or(0, |me| me.microcredits);
        let target = self.free_client(start.target.client.pid).cloned();
        let refusal = session::start_block(&start.script, &quote, microcredits);
        let (Some(target), None) = (target, refusal.as_ref()) else {
            let why = refusal.unwrap_or_else(|| "Its client is gone or busy.".into());
            self.script_error(format!("Couldn't start {}: {why}", start.script.title));
            return;
        };
        let asks = session::asks_first(old_price, &quote);
        let start = PendingStart { target, ..start };
        if asks {
            self.start_prompt = Some(StartPrompt {
                start,
                price: quote.price,
                fee: quote.fee,
                nth: quote.rank + 1,
                old_price: old_price.filter(|old| *old != quote.price),
            });
        } else {
            self.confirm_store_script(start);
        }
    }

    fn start_store_script(&mut self, start: PendingStart) {
        let PendingStart {
            script,
            target,
            replaces,
            lib,
            run,
            launch,
        } = start;
        let pid = target.client.pid;
        self.starting = Some(pid);
        self.info(format!("Starting {}…", script.title));
        let warned = script.community.then_some(script.revision).flatten();
        self.api_job(move |api| {
            let result = session::prepare(&api, &script, lib, warned);
            Event::StoreStarted {
                pid,
                script: Box::new(script),
                replaces,
                run,
                launch: Box::new(launch),
                result,
            }
        });
    }

    fn run(&self) -> Option<&Run> {
        self.active_run.and_then(|id| self.run_by_id(id))
    }

    fn run_mut(&mut self) -> Option<&mut Run> {
        run_in(&mut self.runs, self.active_run)
    }

    fn run_by_id(&self, id: u64) -> Option<&Run> {
        self.runs.iter().find(|r| r.id == id)
    }

    fn run_by_id_mut(&mut self, id: u64) -> Option<&mut Run> {
        run_in(&mut self.runs, Some(id))
    }

    fn run_by_session(&mut self, session_id: &str) -> Option<&mut Run> {
        self.runs
            .iter_mut()
            .find(|r| r.session_id() == Some(session_id))
    }

    fn store_runs(&self) -> impl Iterator<Item = &StoreRun> {
        self.runs
            .iter()
            .filter(|r| r.is_running())
            .filter_map(|r| r.store.as_ref())
    }

    fn client_in_use(&self, pid: u32) -> bool {
        let pending = [
            self.start_prompt.as_ref().map(|prompt| &prompt.start),
            self.warning.as_ref(),
        ];
        self.starting == Some(pid)
            || self.queue_runners.iter().any(|r| r.pid == pid)
            || pending
                .into_iter()
                .flatten()
                .any(|start| start.target.client.pid == pid)
            || self
                .runs
                .iter()
                .any(|r| r.is_running() && r.target.as_ref().is_some_and(|t| t.client.pid == pid))
    }

    fn add_run(&mut self, run: Run) {
        self.active_run = Some(run.id);
        self.log_tab = LogTab::Run(run.id);
        self.runs.push(run);
        self.tab = Tab::Run;
        self.sidebar_open = false;
    }

    fn drive_xp(&mut self) {
        let now = Instant::now();
        let mut reads = Vec::new();
        for run in &mut self.runs {
            if !run.is_running() {
                continue;
            }
            self.wake.wake_after(Duration::from_secs(1));
            let Some(target) = run.loaded_target() else {
                continue;
            };
            if run.xp_reading || now < run.xp_next {
                self.wake.wake_after(
                    run.xp_next
                        .saturating_duration_since(now)
                        .max(Duration::from_millis(250)),
                );
                continue;
            }
            reads.push((run.id, target.clone()));
            run.xp_reading = true;
        }
        for (id, target) in reads {
            self.background(move |settings| Event::XpState {
                run: id,
                result: backend::state(&target, &settings),
            });
        }
    }

    fn script_host(&self) -> crate::script::Host {
        crate::script::Host {
            wake: self.wake.clone(),
            accounts: self.accounts.clone(),
            whitelist: self.http_whitelist.clone(),
            prices_url: self.api.config().prices_url.clone(),
        }
    }

    fn keep_session(
        &self,
        session_id: String,
        process: crate::script::Process,
        interval: Duration,
    ) -> Receiver<()> {
        let (tx, wake, api) = (self.tx.clone(), self.wake.clone(), self.api.clone());
        let (closing, closed) = channel();
        thread::spawn(move || {
            let _closing: Sender<()> = closing;
            session::keep(&api, &session_id, &process, interval, |kept| {
                let session_id = session_id.clone();
                let _ = tx.send(match kept {
                    Kept::Beat(beat) => Event::Heartbeat { session_id, beat },
                    Kept::Stop { reason, sign_out } => Event::SessionStopped {
                        session_id,
                        reason,
                        sign_out,
                    },
                    Kept::Ended(result) => Event::SessionEnded(result),
                });
                wake.wake();
            });
        });
        closed
    }

    fn confirm_store_script(&mut self, start: PendingStart) {
        let script = &start.script;
        let confirmed = script
            .revision
            .is_some_and(|revision| self.confirmed.has(&script.id, revision));
        if script.community && !confirmed {
            self.warning = Some(start);
        } else {
            self.start_store_script(start);
        }
    }

    fn fetch_sessions_if_stale(&mut self) {
        if self.feedback.stale(Instant::now()) {
            self.fetch_sessions();
        }
    }

    fn fetch_sessions(&mut self) {
        self.feedback.loading = true;
        self.api_job(|api| Event::RecentSessions(api.recent_sessions()));
    }

    fn apply_feedback_actions(&mut self, actions: Vec<feedback::Action>) {
        for action in actions {
            match action {
                feedback::Action::Refresh => self.fetch_sessions(),
                feedback::Action::Score { session_id, score } => self.api_job(move |api| {
                    let result = api.score_session(&session_id, score);
                    Event::Scored { session_id, result }
                }),
                feedback::Action::Report {
                    session_id,
                    description,
                } => self.send_report(session_id, description),
            }
        }
    }

    fn versions(&self, run: Option<&Run>) -> feedback::Versions {
        feedback::Versions {
            agent: run
                .and_then(|r| r.target.as_ref())
                .and_then(|t| t.client.version.clone())
                .or_else(|| self.dll_version.clone()),
            lib: run
                .and_then(|r| r.store.as_ref())
                .map(|s| s.lib_version.clone()),
            gpu: self.gpu.clone(),
        }
    }

    fn send_report(&mut self, session_id: String, description: String) {
        let run = self
            .runs
            .iter()
            .find(|r| r.session_id() == Some(session_id.as_str()));
        let versions = self.versions(run);
        let include_system = self.settings.include_system_info;
        let (tx, wake, api, settings) = (
            self.tx.clone(),
            self.wake.clone(),
            self.api.clone(),
            self.settings.clone(),
        );
        let live = run
            .filter(|r| r.is_running())
            .map(|r| (r.log.texts(), r.names.clone(), r.loaded_target().cloned()));
        thread::spawn(move || {
            let diagnostics = feedback::diagnostics(&versions, include_system);
            let report = match live {
                Some((log, mut names, target)) => {
                    let snapshot = target
                        .and_then(|target| feedback::read_snapshot(&target, &settings, &mut names));
                    feedback::report(description, &log, &names, snapshot, diagnostics)
                }
                None => {
                    let record = feedback::load_record(&session_id);
                    crate::api::models::Report {
                        description,
                        log: record
                            .as_ref()
                            .map(|r| r.log.clone())
                            .filter(|log| !log.is_empty()),
                        snapshot: record.and_then(|r| r.snapshot),
                        diagnostics,
                    }
                }
            };
            let result = api
                .report_session(&session_id, &report)
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = tx.send(Event::Reported { session_id, result });
            wake.wake();
        });
    }

    fn keep_run_record(&mut self, index: usize) {
        let run = &self.runs[index];
        let Some(session_id) = run.session_id().map(str::to_owned) else {
            return;
        };
        let title = run.title.clone();
        let (log, mut names, target) = (
            run.log.texts(),
            run.names.clone(),
            run.loaded_target().cloned(),
        );
        self.feedback.fetched_at = None;
        self.background(move |settings| {
            let snapshot =
                target.and_then(|target| feedback::read_snapshot(&target, &settings, &mut names));
            let log = feedback::scrubbed_log(&log, &names);
            let record = feedback::Record {
                session_id,
                title,
                log,
                snapshot,
                ended_at: crate::paths::unix_secs(),
            };
            Event::RunKept(feedback::store_record(&record))
        });
    }

    fn start_in_progress(&self) -> bool {
        self.runs.iter().any(|run| run.ended.is_none()) || self.start_pending()
    }

    fn start_pending(&self) -> bool {
        self.starting.is_some() || self.start_prompt.is_some() || self.warning.is_some()
    }

    fn update_failed(&mut self, error: String) {
        self.error(error.clone());
        self.update_state = UpdateState::Failed(error);
    }

    fn can_update(&mut self) -> bool {
        let alone = self.instance.alone();
        if !alone {
            self.update_failed(
                "Another VOLCREN window is open. Close it to update, so the update doesn't replace the files it runs from.".into(),
            );
        }
        alone
    }

    fn start_update(&mut self, release: update::Release) {
        if !self.can_update() {
            return;
        }
        let progress = Arc::new(update::Progress::default());
        self.update_state = UpdateState::Downloading(progress.clone());
        let repaint = self.wake.clone();
        self.api_job(move |api| {
            let result = update::prepare(&api, &release, &progress, &|| {
                repaint.wake();
            });
            Event::UpdatePrepared { progress, result }
        });
    }

    fn apply_update(&mut self, prepared: &update::Prepared) {
        if !self.can_update() {
            return;
        }
        match update::apply(prepared) {
            Ok(()) => {
                self.update_state = UpdateState::Installing;
                self.quit = true;
            }
            Err(error) => self.update_failed(error),
        }
    }

    fn version_text(&self) -> String {
        let mut text = format!("Version {}", update::VERSION);
        if let Some(available) = &self.update
            && available.kind == update::Kind::Dev
        {
            text.push_str(&format!(
                "\nVersion {} is available. Development build: updates aren't installed here.",
                available.release.version
            ));
        }
        text
    }

    fn info(&mut self, text: impl Into<String>) {
        self.loader_log.push(false, text.into());
    }

    fn error(&mut self, text: impl Into<String>) {
        self.loader_log.push(true, text.into());
    }

    fn script_log(&mut self, error: bool, text: impl Into<String>) {
        match self.run_mut() {
            Some(run) => run.log.push(error, text.into()),
            None => self.loader_log.push(error, text.into()),
        }
    }

    fn script_info(&mut self, text: impl Into<String>) {
        self.script_log(false, text);
    }

    fn script_error(&mut self, text: impl Into<String>) {
        self.script_log(true, text);
    }

    fn spawn(
        &mut self,
        label: &'static str,
        job: impl FnOnce(Settings) -> Event + Send + 'static,
    ) {
        self.busy = Some(label);
        self.background(job);
    }

    fn background(
        &self,
        job: impl FnOnce(Settings) -> Event + Send + 'static,
    ) {
        let (tx, wake, settings) = (self.tx.clone(), self.wake.clone(), self.settings.clone());
        thread::spawn(move || {
            let _ = tx.send(job(settings));
            wake.wake();
        });
    }

    fn tiles_ui(&mut self) {
        let mut layers: Vec<String> = DEV_LAYERS
            .iter()
            .zip(&self.dev_toggles)
            .filter(|(_, on)| **on)
            .filter_map(|(layer, _)| layer.map(str::to_owned))
            .collect();
        if self.widget_inspector_showing() {
            layers.extend(self.widgets.highlight_layers());
        }
        let Some(target) = (!layers.is_empty())
            .then(|| loaded_in(&self.targets, self.selected.as_deref()))
            .flatten()
        else {
            self.tiles = None;
            return;
        };
        if self
            .tiles
            .as_ref()
            .is_some_and(|worker| worker.pid != target.client.pid)
        {
            self.tiles = None;
        }
        match &self.tiles {
            Some(worker) => {
                if let Ok(mut current) = worker.layers.lock()
                    && *current != layers
                {
                    *current = layers;
                }
            }
            None => {
                self.tiles = Some(TilesWorker::start(
                    self.wake.clone(),
                    self.tx.clone(),
                    target.clone(),
                    self.settings.clone(),
                    layers,
                ))
            }
        }
    }

    fn poll_packets(&mut self) {
        if !self.packets.capture {
            return;
        }
        let Some(target) = loaded_in(&self.targets, self.selected.as_deref()) else {
            return;
        };
        let now = Instant::now();
        self.wake.wake_after(crate::packets::POLL_EVERY);
        if self.packets.wants_layouts(target.client.pid, now) {
            self.packets.start_layouts(target.client.pid);
            let (pid, target) = (target.client.pid, target.clone());
            self.background(move |settings| Event::PacketLayouts {
                pid,
                result: backend::packet_layouts(&target, &settings),
            });
        }
        if !self.packets.due(target.client.pid, now) {
            return;
        }
        let (pid, since) = (target.client.pid, self.packets.start(target.client.pid));
        let target = target.clone();
        self.background(move |settings| Event::Packets {
            pid,
            result: backend::packets(&target, &settings, since),
        });
    }

    fn poll_vars(&mut self) {
        if !self.vars.enabled {
            return;
        }
        let Some(target) = loaded_in(&self.targets, self.selected.as_deref()) else {
            return;
        };
        let now = Instant::now();
        self.wake.wake_after(crate::vars::POLL_EVERY);
        if !self.vars.due(target.client.pid, now) {
            return;
        }
        let pid = target.client.pid;
        let crate::vars::Request {
            previous,
            known,
            varc_since,
        } = self.vars.start(pid);
        let target = target.clone();
        self.background(move |settings| {
            let tick_count = || {
                backend::state(&target, &settings)
                    .ok()
                    .and_then(|state| state.tick_count)
            };
            let read = || -> Result<crate::vars::Poll, String> {
                let varps = backend::varps(&target, &settings)?;
                let (mut tick, mut varbits) = (None, Vec::new());
                if let (Some(varps), Some(previous)) = (&varps, &previous) {
                    let changed = crate::vars::Inspector::unknown(previous, varps, &known);
                    if previous != varps {
                        tick = tick_count();
                    }
                    for varp in changed {
                        varbits.push((varp, backend::varbits_of(&target, &settings, varp)?));
                    }
                }
                let (varcs, varc_error) =
                    match backend::varc_changes(&target, &settings, varc_since) {
                        Ok(page) => (page, None),
                        Err(error) => (
                            injector::VarcPage {
                                next: varc_since,
                                ..Default::default()
                            },
                            Some(error),
                        ),
                    };
                if tick.is_none() && !varcs.changes.is_empty() {
                    tick = tick_count();
                }
                Ok(crate::vars::Poll {
                    varps,
                    tick,
                    varbits,
                    varcs,
                    varc_error,
                })
            };
            Event::Vars {
                pid,
                result: read(),
            }
        });
    }

    fn poll_widget_mouse(&mut self) {
        if !self.widgets.follow_mouse || !self.widget_inspector_showing() {
            return;
        }
        let Some(target) = loaded_in(&self.targets, self.selected.as_deref()) else {
            return;
        };
        self.wake.wake_after(crate::widgets::FOLLOW_EVERY);
        if !self.widgets.follow_due(Instant::now()) {
            return;
        }
        let (pid, target) = (target.client.pid, target.clone());
        self.background(move |settings| {
            let result = backend::connection(&target, &settings).map(|connection| {
                connection
                    .filter(|c| c.mouse_x >= 0 && c.mouse_y >= 0)
                    .map(|c| (c.mouse_x, c.mouse_y))
            });
            Event::WidgetMouse { pid, result }
        });
    }

    fn refresh_widgets(&mut self, target: Target) {
        let pid = target.client.pid;
        self.widgets.start(pid);
        let known = self.widgets.known_groups();
        self.background(move |settings| {
            let read = || -> Result<crate::widgets::Poll, String> {
                let tree = backend::widget_tree(&target, &settings)?;
                let mut groups: Vec<i32> = tree
                    .widgets
                    .iter()
                    .map(|w| w.widget.group)
                    .filter(|group| !known.contains(group))
                    .collect();
                groups.sort_unstable();
                groups.dedup();
                let names = if groups.is_empty() {
                    Vec::new()
                } else {
                    let names = backend::game_val_names(
                        &target,
                        &settings,
                        crate::widgets::INTERFACE_GAME_VAL,
                        &groups,
                    )?;
                    paired(&groups, names)
                };
                Ok(crate::widgets::Poll { tree, names })
            };
            Event::Widgets {
                pid,
                result: read(),
            }
        });
    }

    fn poll_script_log(&mut self) {
        if !self.script_inspector.enabled {
            return;
        }
        let Some(target) = loaded_in(&self.targets, self.selected.as_deref()) else {
            return;
        };
        let now = Instant::now();
        self.wake.wake_after(crate::script_log::POLL_EVERY);
        if !self.script_inspector.due(target.client.pid, now) {
            return;
        }
        let (pid, since) = (
            target.client.pid,
            self.script_inspector.start(target.client.pid),
        );
        let target = target.clone();
        self.background(move |settings| Event::ScriptRuns {
            pid,
            result: backend::script_runs(&target, &settings, since)
                .map(|page| crate::script_log::Poll { page }),
        });
    }

    fn poll_inventory(&mut self) {
        if !self.inventory.enabled {
            return;
        }
        let Some(target) = loaded_in(&self.targets, self.selected.as_deref()) else {
            return;
        };
        let now = Instant::now();
        if let Some(wait) = self.inventory.retry_in(now) {
            self.wake.wake_after(wait);
        }
        if !self.inventory.due(target.client.pid, now) {
            return;
        }
        let pid = target.client.pid;
        let crate::inventory::Request {
            since,
            known_containers,
            known_items,
        } = self.inventory.start(pid);
        let target = target.clone();
        self.background(move |settings| {
            let read = || -> Result<crate::inventory::Poll, String> {
                let (page, baseline) = match since {
                    Some(since) => (
                        backend::item_changes(
                            &target,
                            &settings,
                            since,
                            crate::inventory::WAIT_FOR_CHANGE,
                        )?,
                        None,
                    ),
                    None => {
                        let page = backend::item_changes(
                            &target,
                            &settings,
                            injector::ITEM_CHANGES_NOW,
                            Duration::ZERO,
                        )?;
                        (page, Some(backend::item_containers(&target, &settings)?))
                    }
                };
                let missing = |known: &[i32], mut ids: Vec<i32>| {
                    ids.retain(|id| *id >= 0 && !known.contains(id));
                    ids.sort_unstable();
                    ids.dedup();
                    ids
                };
                let mut containers: Vec<i32> =
                    page.changes.iter().map(|change| change.container).collect();
                let mut items: Vec<i32> = page
                    .changes
                    .iter()
                    .flat_map(|change| [change.old_item, change.new_item])
                    .collect();
                for (id, held) in baseline.iter().flatten() {
                    containers.push(*id);
                    items.extend(held.iter().map(|item| item.id));
                }
                let new_containers = missing(&known_containers, containers);
                let new_items = missing(&known_items, items);
                let container_names = if new_containers.is_empty() {
                    Vec::new()
                } else {
                    paired(
                        &new_containers,
                        backend::game_val_names(
                            &target,
                            &settings,
                            crate::inventory::INV_GAME_VAL,
                            &new_containers,
                        )?,
                    )
                };
                let (item_names, stackable) = if new_items.is_empty() {
                    (Vec::new(), Vec::new())
                } else {
                    item_info(&target, &settings, &new_items)?
                };
                Ok(crate::inventory::Poll {
                    page,
                    baseline,
                    container_names,
                    item_names,
                    stackable,
                })
            };
            Event::Inventory {
                pid,
                result: read(),
            }
        });
    }

    fn poll_icons(&mut self) {
        let now = Instant::now();
        if let Some(wait) = self.icons.retry_in(now) {
            self.wake.wake_after(wait);
        }
        if !self.icons.due(now) {
            return;
        }
        let Some(target) = loaded_in(&self.targets, self.selected.as_deref()) else {
            return;
        };
        let (batch, target) = (self.icons.start(), target.clone());
        self.background(move |settings| {
            let mut pictures = Vec::new();
            let mut read = || -> Result<(), String> {
                if !batch.items.is_empty() {
                    let images = backend::item_icons(&target, &settings, &batch.items)?;
                    pictures.extend(
                        batch
                            .items
                            .iter()
                            .map(|r| crate::icons::Key::Item(*r))
                            .zip(images),
                    );
                }
                if !batch.sprites.is_empty() {
                    let images = backend::sprites(&target, &settings, &batch.sprites)?;
                    pictures.extend(
                        batch
                            .sprites
                            .iter()
                            .map(|id| crate::icons::Key::Sprite(*id))
                            .zip(images),
                    );
                }
                Ok(())
            };
            let error = read().err();
            Event::Icons { pictures, error }
        });
    }

    fn scan(&mut self) {
        self.spawn("Looking for clients…", |settings| {
            let mut scan = backend::scan(&settings);
            match settings.dll_version() {
                Ok(version) => scan.dll_version = Some(version),
                Err(error) if !settings.dll.is_empty() => scan.errors.push(error),
                Err(_) => {}
            }
            Event::Scanned(scan)
        });
    }

    fn unload(&mut self, target: Target) {
        let pid = target.client.pid;
        self.info(format!("Unloading agent from {pid}…"));
        self.spawn("Unloading…", move |settings| Event::Unloaded {
            pid,
            result: backend::unload(&target, &settings),
        });
    }

    fn console(&mut self, target: Target) {
        let pid = target.client.pid;
        self.spawn("Opening console…", move |settings| Event::Done {
            pid,
            what: "Opening the console",
            result: backend::console(&target, &settings),
        });
    }

    fn set_cursor(&mut self, target: Target) {
        let pid = target.client.pid;
        let cursor = self.cursor;
        self.background(move |settings| Event::Done {
            pid,
            what: "Setting the cursor",
            result: backend::set_cursor(&target, &settings, &cursor),
        });
    }

    fn load(&mut self, target: Target) {
        let pid = target.client.pid;
        self.info(format!("Loading into {pid}…"));
        let cursor = self.cursor;
        self.spawn("Loading…", move |settings| Event::Loaded {
            pid,
            result: backend::load(&target, &settings, &cursor),
        });
    }

    fn read_state(&mut self, target: Target, read: Read) {
        let pid = target.client.pid;
        if read.is_varp() {
            self.spawn("Reading…", move |settings| Event::Varps {
                pid,
                read,
                result: backend::varps(&target, &settings),
            });
            return;
        }
        self.spawn("Reading…", move |settings| Event::State {
            pid,
            read,
            result: backend::state(&target, &settings),
        });
    }

    fn set_display_name(&mut self, target: Target) {
        let pid = target.client.pid;
        let name = self.display_name.trim().to_owned();
        self.spawn("Setting the name…", move |settings| {
            let result = backend::set_display_name(&target, &settings, &name);
            Event::Name { pid, name, result }
        });
    }

    fn handle_events(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            if !matches!(
                event,
                Event::Tiles { .. }
                    | Event::XpState { .. }
                    | Event::AccountName(_)
                    | Event::Packets { .. }
                    | Event::PacketLayouts { .. }
                    | Event::Vars { .. }
                    | Event::ScriptRuns { .. }
                    | Event::Inventory { .. }
                    | Event::Icons { .. }
                    | Event::LayoutRead { .. }
                    | Event::Widgets { .. }
                    | Event::WidgetMouse { .. }
            ) {
                self.busy = None;
            }
            match event {
                Event::Scanned(scan) => {
                    self.targets = scan.targets;
                    self.dll_version = scan.dll_version;
                    for error in scan.errors {
                        self.error(error);
                    }
                    let usable = |t: &&Target| t.client.foreign_bot.is_none() && t.client.is_64bit;
                    if !self.targets.iter().filter(usable).any(|t| Some(t.key()) == self.selected) {
                        self.selected = self.targets.iter().find(usable).map(Target::key);
                    }
                }
                Event::Loaded { pid, result } => {
                    match result {
                        Ok(report) => {
                            self.report(pid, report);
                        }
                        Err(error) => self.error(format!("Loading into {pid} failed: {error}")),
                    }
                    self.scan();
                }
                Event::Unloaded { pid, result } => {
                    match result {
                        Ok(()) => self.info(format!("Unloaded agent from {pid}.")),
                        Err(error) => self.error(format!("Unloading agent from {pid} failed: {error}")),
                    }
                    self.scan();
                }
                Event::Tiles { pid, result } => {
                    if let Err(error) = result {
                        self.tiles = None;
                        for (on, layer) in self.dev_toggles.iter_mut().zip(DEV_LAYERS) {
                            if layer.is_some() {
                                *on = false;
                            }
                        }
                        self.error(format!("Scene tiles in {pid} failed: {error}"));
                    }
                }
                Event::Restored(result) => match result {
                    Ok(Some(account)) => self.signed_in(account),
                    Ok(None) => self.auth = Auth::SignedOut,
                    Err(error) => {
                        self.auth = Auth::SignedOut;
                        if let ApiError::SignedOut(why) = &error {
                            self.error(format!("Your saved sign-in is no longer valid: {why}"));
                        } else {
                            self.error(format!("Couldn't sign in with the saved session: {error}"));
                        }
                    }
                },
                Event::SignedIn(result) => match result {
                    Ok(account) => self.signed_in(account),
                    Err(error) => {
                        self.auth = Auth::SignedOut;
                        self.error(error);
                    }
                },
                Event::Update(result) => match result {
                    Ok(available) => {
                        let failed = matches!(self.update_state, UpdateState::Failed(_));
                        let busy = self.start_in_progress() || !self.instance.alone();
                        let automatic = available
                            .as_ref()
                            .filter(|a| update::installs_by_itself(a.kind, failed, busy))
                            .map(|a| a.release.clone());
                        self.update = available;
                        if let Some(release) = automatic {
                            self.start_update(release);
                        }
                    }
                    Err(update::CheckError::Unavailable(error)) => {
                        self.info(format!("Couldn't check for updates: {error}"));
                    }
                    Err(update::CheckError::BadSignature) => {
                        self.error("Update manifest signature is invalid");
                    }
                },
                Event::UpdatePrepared { progress, result } => {
                    let current = matches!(&self.update_state, UpdateState::Downloading(now) if Arc::ptr_eq(now, &progress));
                    match result {
                        Ok(prepared) if current => self.apply_update(&prepared),
                        Err(error) if current => self.update_failed(error),
                        _ => {}
                    }
                }
                Event::Me(result) => match result {
                    Ok(me) => self.refresh_me(me),
                    Err(ApiError::Unauthorized | ApiError::SignedOut(_)) => {
                        if self.auth.me().is_some() {
                            self.sign_out(Some("your session expired.".into()));
                        }
                    }
                    Err(error) => self.error(format!("Couldn't refresh your account: {error}")),
                },
                Event::Scripts(result) => self.scripts_loaded(result),
                Event::Redeemed(result) => {
                    self.store.redeeming = false;
                    if self.store_changed(result, "Added to your scripts.") {
                        self.store.code.clear();
                    }
                }
                Event::GrantRemoved { id, result } => {
                    self.store.removing = None;
                    if self.store_changed(result, "Removed from your account.") {
                        self.confirmed.forget(&id);
                    }
                }
                Event::StoreStarted { pid, script, replaces, run, launch, result } => {
                    self.starting = None;
                    let started = match result {
                        Ok(prepared) => {
                            let started = self.run_prepared(pid, &script, prepared, run, *launch);
                            if let (Some(new), Some(old)) = (started, replaces) {
                                self.carry_form(old, new);
                                self.close_run(old);
                            }
                            started
                        }
                        Err(error) => {
                            self.start_failed(&script.title, error);
                            None
                        }
                    };
                    self.queue_entry_started(pid, started);
                }
                Event::QueueManifest { script_id, result } => self.queue_manifest(script_id, result),
                Event::QueueChecked { pid, entry, result } => self.queue_checked(pid, entry, result),
                Event::ManifestFetched { script, target, result } => {
                    self.starting = None;
                    match result {
                        Ok(fetched) => self.manifest_fetched(*script, *target, fetched),
                        Err(ApiError::SignedOut(why)) => self.sign_out(Some(why)),
                        Err(error) => self.start_failed(&script.title, error),
                    }
                }
                Event::RunQuoted { script, target, run, values, result } => {
                    self.starting = None;
                    match result {
                        Ok(session::Quoted { me, quote, lib }) => {
                            self.refresh_me(me);
                            match lib {
                                Ok(lib) => {
                                    let start = PendingStart { script: *script, target: *target, replaces: None, lib, run, launch: Launch::with(values) };
                                    self.start_at_quote(start, None, quote);
                                }
                                Err(error) => self.start_failed(&script.title, error),
                            }
                        }
                        Err(ApiError::SignedOut(why)) => self.sign_out(Some(why)),
                        Err(error) => self.start_failed(&script.title, error),
                    }
                }
                Event::RestartQuoted { run, result } => {
                    self.starting = None;
                    match result {
                        Ok(Some(restart)) => self.restart_quoted(run, restart),
                        Ok(None) => self.script_error("Couldn't restart: the script is no longer available to this account."),
                        Err(ApiError::SignedOut(why)) => self.sign_out(Some(why)),
                        Err(error) => self.script_error(format!("Couldn't restart: {error}")),
                    }
                }
                Event::Heartbeat { session_id, beat } => {
                    if let (Some(microcredits), Some(me)) = (beat.microcredits, self.auth.me_mut()) {
                        me.microcredits = microcredits;
                    }
                    if let Some(store) = self.run_by_session(&session_id).and_then(|run| run.store.as_mut()) {
                        store.rank = beat.rank.unwrap_or(store.rank);
                        store.fee = beat.fee.unwrap_or(store.fee);
                        store.fee_charged = beat.fee_charged.unwrap_or(store.fee_charged);
                    }
                }
                Event::SessionStopped { session_id, reason, sign_out } => {
                    if let Some(run) = self.run_by_session(&session_id) {
                        run.stop();
                        run.error(reason.clone());
                    }
                    if sign_out {
                        self.sign_out(Some(reason));
                    }
                }
                Event::SessionEnded(result) => match result {
                    Ok(ended) => {
                        self.script_info(format!("Session ended, {} charged for the last minutes.", crate::api::models::credits_label(ended.charged as f64)));
                        if self.auth.me().is_some() {
                            self.api_job(|api| Event::Me(api.me()));
                        }
                    }
                    Err(error) => self.script_error(format!("Couldn't end the session: {error}. The server will end it within 5 minutes.")),
                },
                Event::Done { pid, what, result } => {
                    if let Err(error) = result {
                        self.error(format!("{what} of {pid} failed: {error}"));
                    }
                }
                Event::Name { pid, name, result } => match result {
                    Ok(DisplayName::Kept { real }) => self.info(format!(
                        "{pid}: showing \"{name}\" instead of {}, kept across logins. Text already on screen changes when the client redraws it (the chatbox's input line soon, the quest tab's summary after switching tabs).",
                        real.map_or_else(|| "the real name".to_owned(), |real| format!("\"{real}\""))
                    )),
                    Ok(DisplayName::Waiting) => {
                        self.info(format!("{pid}: \"{name}\" will show once that account is logged in."))
                    }
                    Ok(DisplayName::Off) => self.info(format!("{pid}: the real display name shows.")),
                    Err(error) => self.error(format!("Setting the display name in {pid} failed: {error}")),
                },
                Event::AccountName(result) => crate::script::write(&self.accounts).client_name(result),
                Event::XpState { run: id, result } => {
                    if let Some(run) = self.run_by_id_mut(id) {
                        run.xp_reading = false;
                        run.xp_next = Instant::now() + XP_READ_EVERY;
                        if let Ok(state) = &result {
                            for name in feedback::names_of(state) {
                                if !run.names.contains(&name) {
                                    run.names.push(name);
                                }
                            }
                        }
                        let skills = result.ok().and_then(|state| state.skills).filter(|skills| skills.iter().any(|s| s.xp > 0));
                        if let Some(skills) = skills.filter(|_| run.is_running()) {
                            let skills = skills
                                .into_iter()
                                .map(|s| crate::xp::SkillXp { name: s.name, level: s.real_level as i64, xp: s.xp as i64 })
                                .collect();
                            let elapsed = run.run_time();
                            run.xp.update(skills, elapsed);
                        }
                    }
                }
                Event::RecentSessions(result) => self
                    .feedback
                    .finish(result.map_err(|e| e.to_string()), Instant::now()),
                Event::Scored { session_id, result } => self
                    .feedback
                    .scored(&session_id, result.map(|s| s.score).map_err(|e| e.to_string())),
                Event::Reported { session_id, result } => self.feedback.reported(&session_id, result),
                Event::RunKept(Ok(())) => {}
                Event::RunKept(Err(error)) => {
                    self.error(format!("Couldn't keep the run for bug reports: {error}"))
                }
                Event::Packets { pid, result } => {
                    self.packets.finish(pid, Instant::now(), result);
                    let piped = self.packets.write_pipe();
                    self.report_pipe(piped.map(|()| None));
                }
                Event::PacketLayouts { pid, result } => self.packets.finish_layouts(pid, result),
                Event::ScriptNames(result) => self.script_inspector.finish_names(result),
                Event::ScriptRuns { pid, result } => {
                    self.script_inspector.finish(pid, Instant::now(), result);
                    let piped = self.script_inspector.write_pipe();
                    self.report_pipe(piped.map(|()| None));
                }
                Event::Inventory { pid, result } => self.inventory.finish(pid, Instant::now(), result),
                Event::Icons { pictures, error } => self.icons.finish(Instant::now(), pictures, error),
                Event::LayoutRead { run, kind, result } => {
                    if let Some(editors) = editors_of(&mut self.runs, Some(run)) {
                        match kind {
                            crate::layouts::Kind::Equipment => editors
                                .equipment
                                .finish_read(result, crate::layouts::apply_equipment),
                            crate::layouts::Kind::Inventory => editors
                                .inventory
                                .finish_read(result, crate::layouts::apply_inventory),
                            crate::layouts::Kind::Prayer => editors
                                .prayer
                                .finish_read(result, crate::layouts::apply_prayers),
                        }
                    }
                }
                Event::ItemsKnown(result) => match result {
                    Ok(items) => {
                        self.asking_items.clear();
                        self.known_items
                            .extend(items.into_iter().map(|(id, name, stacks)| (id, (name, stacks))));
                    }
                    Err(error) => self.error(format!("Couldn't read the script's required items: {error}")),
                },
                Event::Widgets { pid, result } => self.widgets.finish(pid, result),
                Event::WidgetMouse { pid, result } => {
                    self.widgets.mouse_done();
                    match result {
                        Ok(Some((x, y))) if self.widgets.follow_mouse && self.widgets.has_pid(pid) => self.widgets.pick_at(x, y),
                        Ok(_) => {}
                        Err(error) => self.widgets.error = Some(error),
                    }
                }
                Event::Vars { pid, result } => {
                    self.vars.finish(pid, Instant::now(), result, |id| known_varp(id).map(str::to_owned));
                    let piped = self.vars.write_pipe();
                    self.report_pipe(piped.map(|()| None));
                }
                Event::State { pid, read, result } => match result {
                    Ok(state) => self.report_state(pid, read, &state),
                    Err(error) => self.error(format!("Reading the game state of {pid} failed: {error}")),
                },
                Event::Varps { pid, read, result } => match result {
                    Ok(None) => self.info(format!("{pid}: No varps (not logged in).")),
                    Ok(Some(varps)) => {
                        let text = varps_report(read, &varps, self.varps.as_deref());
                        self.varps = Some(varps);
                        if let Some(text) = text {
                            self.info(format!("{pid}: {text}"));
                        }
                    }
                    Err(error) => self.error(format!("Reading the varps of {pid} failed: {error}")),
                },
            }
        }
    }

    fn report(&mut self, pid: u32, report: LoadReport) {
        if report.already_loaded {
            self.info(format!("This build of agent was already loaded in {pid}."));
        }
        if let Some(old) = &report.replaced {
            self.info(format!("Unloaded agent {old} from {pid}."));
        }
        match report.connection {
            Some(c) if !c.is_compatible() => self.error(format!(
                "agent {} in {pid} uses shared memory protocol {}, this loader uses {}. Load a matching agent.",
                c.version,
                c.protocol,
                injector::PROTOCOL_VERSION
            )),
            Some(c) if c.width > 0 => {
                self.info(format!("Connected to agent {} in {pid} ({}x{}).", c.version, c.width, c.height));
                self.report_offsets(pid, c.offsets_found);
            }
            Some(c) => {
                self.info(format!(
                    "Connected to agent {} in {pid}, waiting for the client to render a frame.",
                    c.version
                ));
                self.report_offsets(pid, c.offsets_found);
            }
            None => self.error(format!(
                "Loaded into {pid}, but agent's shared memory never appeared. It may have failed to start."
            )),
        }
    }

    fn report_state(&mut self, pid: u32, read: Read, state: &GameState) {
        let text = match (read, state.game_state, state.position) {
            (Read::GameState, Some(game_state), _) => {
                format!(
                    "Game state: {} ({game_state}).",
                    injector::game_state_name(game_state)
                )
            }
            (Read::GameState, None, _) => "Game state: unknown (offset not found).".into(),
            (Read::Position, _, Some(p)) => {
                let plane = p
                    .plane
                    .map_or_else(|| "?".into(), |plane| plane.to_string());
                format!(
                    "Position: tile ({}, {}, plane {plane}), local ({}, {}).",
                    p.x, p.y, p.local_x, p.local_y
                )
            }
            (Read::Chunk, _, Some(p)) => {
                let chunk = p.chunk();
                format!(
                    "Chunk: ({}, {}), region id {}.",
                    chunk.x,
                    chunk.y,
                    chunk.id()
                )
            }
            (Read::Position | Read::Chunk, _, None) => "No local player (logged out?).".into(),
            (Read::Skills, ..) => match &state.skills {
                Some(skills) => {
                    let total: i32 = skills.iter().map(|s| s.real_level).sum();
                    let lines: Vec<String> = skills
                        .iter()
                        .map(|s| format!("  {}: {}/{}, {} xp", s.name, s.level, s.real_level, s.xp))
                        .collect();
                    format!(
                        "Skills (current/real), total level {total}:\n{}",
                        lines.join("\n")
                    )
                }
                None => "No skills (logged out, or the offsets weren't found).".into(),
            },
            (Read::Login, ..) => match state.login {
                Some(l) => format!(
                    "Login screen {}: panel x {}, box x {}, center x {}, y {}, Steam sign-in {}.",
                    l.screen,
                    l.panel_x,
                    l.box_x,
                    l.center_x,
                    l.y,
                    match l.steam {
                        Some(true) => "yes",
                        Some(false) => "no",
                        None => "unknown",
                    }
                ),
                None => "No login layout (not on the login screen, or the offsets weren't found)."
                    .into(),
            },
            (Read::RootInterface, ..) => match state.root_interface {
                Some(165) => "Top-level interface: 165 (lobby).".into(),
                Some(root) => format!("Top-level interface: {root}."),
                None => {
                    "No top-level interface (not logged in, or the offsets weren't found).".into()
                }
            },
            (Read::TickCount, ..) => match state.tick_count {
                Some(ticks) => format!("Tick count: {ticks} ({:.1} s).", ticks as f64 * 0.6),
                None => "Tick count: unknown (offset not found).".into(),
            },
            (Read::Varp(_) | Read::KnownVarps | Read::VarpChanges, ..) => return,
        };
        self.info(format!("{pid}: {text}"));
    }

    fn report_offsets(&mut self, pid: u32, found: Option<bool>) {
        match found {
            Some(true) => self.info(format!("Game state offsets found in {pid}.")),
            Some(false) => self.error(format!(
                "Game state offsets were not found in {pid}: this client build isn't supported yet."
            )),
            None => self.info(format!("Still looking for game state offsets in {pid}.")),
        }
    }

    fn selected_target(&self) -> Option<&Target> {
        selected_in(&self.targets, self.selected.as_deref())
    }

    fn selected_loaded_target(&self) -> Option<&Target> {
        loaded_in(&self.targets, self.selected.as_deref())
    }

    fn handle_script_events(&mut self) {
        for index in 0..self.runs.len() {
            let events = {
                let run = &mut self.runs[index];
                let Some(script) = run.script.as_mut() else {
                    continue;
                };
                if script.stop_ignored(Duration::from_secs(2)) {
                    run.error("The script's process didn't exit after being killed. It will end when the loader closes.");
                } else if script.is_running() && script.stop_requested() {
                    self.wake.wake_after(Duration::from_millis(250));
                }
                match run.script.as_mut() {
                    Some(script) => script.poll(),
                    None => continue,
                }
            };
            for event in events {
                let finished = matches!(event, ScriptEvent::Finished { .. });
                let run = &mut self.runs[index];
                if let ScriptEvent::Finished { result, stopped } = &event {
                    run.finished = Some((result.clone(), *stopped));
                }
                match event {
                    ScriptEvent::Output(line) => run.info(line),
                    ScriptEvent::Warning(line) => run.log.push_tone(Tone::Warning, line),
                    ScriptEvent::Exit(exit) => {
                        match (&exit.status, &exit.reason) {
                            (injector::form::ExitStatus::Blocked, Some(reason)) => {
                                run.info(format!("The script can't go on: {reason}"));
                            }
                            (injector::form::ExitStatus::Blocked, None) => {
                                run.info("The script can't go on.");
                            }
                            (injector::form::ExitStatus::Done, Some(reason)) => {
                                run.info(reason.clone())
                            }
                            (injector::form::ExitStatus::Done, None) => {}
                        }
                        run.exit = Some(exit);
                    }
                    ScriptEvent::Counters(counters) => {
                        let elapsed = run.run_time();
                        run.counters.update(counters, elapsed);
                    }
                    ScriptEvent::Breaks(plan) => {
                        let now = Instant::now();
                        run.last_breaks = Some((plan.clone(), now));
                        run.breaks.update(plan, now);
                    }
                    ScriptEvent::Finished { stopped: true, .. } => run.info("Script stopped."),
                    ScriptEvent::Finished { result: Ok(()), .. } => run.info("Script finished."),
                    ScriptEvent::Finished {
                        result: Err(error), ..
                    } => run.error(format!("Script failed: {error}")),
                }
                if finished {
                    self.runs[index].ended = Some(Instant::now());
                    self.keep_run_record(index);
                }
            }
        }
    }

    fn close_run(&mut self, id: u64) {
        self.runs.retain(|r| r.id != id || r.is_running());
        if self.active_run == Some(id) && self.run().is_none() {
            self.active_run = None;
            if self.tab == Tab::Run {
                self.tab = Tab::Sessions;
            }
        }
        self.repair_log_tab();
    }

    fn repair_log_tab(&mut self) {
        if let LogTab::Run(id) = self.log_tab
            && self.run_by_id(id).is_none()
        {
            self.log_tab = LogTab::Loader;
        }
    }

    fn run_target_client(&self) -> Option<Target> {
        self.targets
            .iter()
            .find(|t| Some(t.key()) == self.run_target && self.client_free(t))
            .cloned()
    }

    fn carry_form(&mut self, from: u64, to: u64) {
        let form = run_in(&mut self.runs, Some(from)).and_then(|r| r.form.take());
        if let Some(run) = run_in(&mut self.runs, Some(to)) {
            run.form = form;
        }
    }

    fn remember_form(&mut self, id: u64) {
        if let Some(form) = self.run_by_id(id).and_then(|r| r.form.as_ref()) {
            let (title, remembered) = (form.title.clone(), form.remembered());
            self.remembered_forms.insert(title, remembered);
        }
    }

    fn answer_form(&mut self, id: u64, outcome: crate::form::Outcome) {
        match outcome {
            crate::form::Outcome::Submitted(values) => {
                self.remember_form(id);
                self.launch_run(id, values);
            }
            crate::form::Outcome::Cancelled => self.close_run(id),
        }
    }

    fn launch_run(&mut self, id: u64, values: serde_json::Value) {
        let Some(run) = self.run_by_id(id).filter(|r| r.is_configuring()) else {
            return;
        };
        let target = run.target.clone();
        match run.source.clone() {
            Some(Source::File(file)) => self.start_file(id, &file, target.as_ref(), values),
            Some(Source::Store(script)) => match target {
                Some(target) => self.quote_store_script(script, target, Some(id), values),
                None => self.script_error("Pick a client to run the script on."),
            },
            None => {}
        }
    }

    fn push_form_update(&mut self) {
        let Some(form) = self.run().and_then(|r| r.form.as_ref()) else {
            return;
        };
        let crate::form::Outcome::Submitted(values) =
            form.submitted(self.script_layouts(), &self.known_items)
        else {
            return;
        };
        if let Some(id) = self.active_run {
            self.remember_form(id);
        }
        if let Some(run) = self.run_mut() {
            match run.send(|s| s.push_update(&values)) {
                Ok(()) => {
                    run.form_sent = Some(values);
                    run.info("Settings updated.");
                }
                Err(error) => run.error(error),
            }
        }
    }

    fn describe_items(&mut self, ids: Vec<i32>) {
        let Some(target) = self.layout_target().cloned() else {
            return;
        };
        self.asking_items.extend(&ids);
        self.background(move |settings| {
            Event::ItemsKnown((|| {
                let names = backend::item_names(&target, &settings, &ids)?;
                let stacks = backend::item_stackable(&target, &settings, &ids)?;
                Ok(ids
                    .iter()
                    .zip(names)
                    .zip(stacks)
                    .map(|((&id, name), stacks)| {
                        (id, name.unwrap_or_default(), stacks.unwrap_or(false))
                    })
                    .collect())
            })())
        });
    }

    fn script_layouts(&self) -> &crate::layouts::ScriptLayouts {
        self.run()
            .and_then(|r| r.form.as_ref())
            .and_then(|form| self.layouts.get(&form.title))
            .unwrap_or(&EMPTY_LAYOUTS)
    }

    fn layout_target(&self) -> Option<&Target> {
        self.run()
            .and_then(Run::loaded_target)
            .or_else(|| self.selected_loaded_target())
    }

    fn read_layout(&mut self, kind: crate::layouts::Kind, quick: bool) {
        use crate::layouts::{self, Kind, Read};
        let Some(run) = self.active_run else {
            return;
        };
        let Some(target) = self.layout_target().cloned() else {
            if let Some(editors) = editors_of(&mut self.runs, Some(run)) {
                editors.equipment.reading = false;
                editors.inventory.reading = false;
                editors.prayer.reading = false;
            }
            return;
        };
        self.background(move |settings| {
            let read = || -> Result<Read, String> {
                let container = match kind {
                    Kind::Equipment => crate::layouts::EQUIPMENT_CONTAINER,
                    Kind::Inventory => crate::layouts::INVENTORY_CONTAINER,
                    Kind::Prayer => {
                        let mut keys = Vec::new();
                        if quick {
                            let mask = backend::varbit(
                                &target,
                                &settings,
                                layouts::QUICK_SELECTED_VARBIT,
                            )?
                            .ok_or("the client has no quick prayer selection")?;
                            let selected: Vec<&layouts::PrayerInfo> = layouts::PRAYERS
                                .iter()
                                .filter(|p| mask >> p.quick_bit & 1 == 1)
                                .collect();
                            let mut unlocked = Vec::new();
                            for p in &selected {
                                if let Some(unlock) = p.unlock
                                    && backend::varbit(&target, &settings, unlock)? == Some(1)
                                {
                                    unlocked.push(p.key);
                                }
                            }
                            for p in selected {
                                let replaced = layouts::PRAYERS.iter().any(|u| {
                                    u.replaces == Some(p.key) && unlocked.contains(&u.key)
                                });
                                let locked_upgrade =
                                    p.replaces.is_some() && !unlocked.contains(&p.key);
                                if !replaced && !locked_upgrade {
                                    keys.push(p.key.to_owned());
                                }
                            }
                        } else {
                            for p in &layouts::PRAYERS {
                                if backend::varbit(&target, &settings, p.varbit)? == Some(1) {
                                    keys.push(p.key.to_owned());
                                }
                            }
                        }
                        return Ok(Read::Prayers(keys));
                    }
                };
                let items = backend::item_container(&target, &settings, container)?
                    .ok_or("the client holds no such container")?;
                let describe = |ids: &[i32]| -> Result<crate::layouts::Described, String> {
                    let mut ids: Vec<i32> = ids.iter().copied().filter(|&id| id >= 0).collect();
                    ids.sort_unstable();
                    ids.dedup();
                    if ids.is_empty() {
                        return Ok(Default::default());
                    }
                    item_info(&target, &settings, &ids)
                };
                let ids: Vec<i32> = items.iter().map(|item| item.id).collect();
                let (names, stackable) = describe(&ids)?;
                let mut slots = layouts::items_of(&items, &names, &stackable);
                let mut single: Vec<i32> = slots
                    .iter()
                    .flatten()
                    .filter(|item| !item.stackable)
                    .map(|item| item.id)
                    .collect();
                single.sort_unstable();
                single.dedup();
                if !single.is_empty() {
                    let families: HashMap<i32, injector::config::ItemFamily> = single
                        .iter()
                        .copied()
                        .zip(backend::item_families(&target, &settings, &single)?)
                        .collect();
                    layouts::add_families(slots.iter_mut().flatten(), &families);
                }
                match kind {
                    Kind::Prayer => unreachable!("read above"),
                    Kind::Equipment => {
                        let mut slots: Vec<layouts::EquipmentSlot> = slots
                            .into_iter()
                            .map(|item| {
                                item.map_or(
                                    layouts::EquipmentSlot::Empty,
                                    layouts::EquipmentSlot::Item,
                                )
                            })
                            .collect();
                        slots.resize(layouts::SECONDARY_AMMO_SLOT, layouts::EquipmentSlot::Empty);
                        slots.push(
                            match layouts::read_secondary_ammo(&target, &settings, &items)? {
                                None => layouts::EquipmentSlot::Any,
                                Some(None) => layouts::EquipmentSlot::Empty,
                                Some(Some((id, quantity))) => {
                                    let (names, stacks) = describe(&[id])?;
                                    layouts::EquipmentSlot::Item(layouts::Item::read(
                                        id,
                                        quantity,
                                        names.get(&id).cloned().unwrap_or_default(),
                                        stacks.get(&id).copied().unwrap_or(true),
                                    ))
                                }
                            },
                        );
                        Ok(Read::Equipment {
                            autocast: backend::varbit(
                                &target,
                                &settings,
                                layouts::WEAPON_CATEGORY_VARBIT,
                            )?
                            .is_some_and(|category| {
                                layouts::AUTOCAST_WEAPON_CATEGORIES.contains(&category)
                            }),
                            slots,
                        })
                    }
                    Kind::Inventory => {
                        let has_pouch = slots.iter().flatten().any(layouts::Item::is_rune_pouch);
                        let pouch = if has_pouch {
                            let runes =
                                backend::enum_def(&target, &settings, layouts::POUCH_RUNES_ENUM)?
                                    .ok_or("the cache has no rune pouch enum")?;
                            let mut held = Vec::new();
                            for (rune, amount) in layouts::POUCH_RUNE_VARBITS
                                .iter()
                                .zip(layouts::POUCH_AMOUNT_VARBITS)
                            {
                                let index =
                                    backend::varbit(&target, &settings, *rune)?.unwrap_or(0);
                                let amount =
                                    backend::varbit(&target, &settings, amount)?.unwrap_or(0);
                                let id =
                                    runes.entries.iter().find(|e| e.key == index).and_then(|e| {
                                        match &e.value {
                                            injector::config::Value::Int(id) => {
                                                i32::try_from(*id).ok()
                                            }
                                            injector::config::Value::Str(_) => None,
                                        }
                                    });
                                if let Some(id) = id.filter(|&id| id > 0 && amount > 0) {
                                    held.push((id, amount));
                                }
                            }
                            let ids: Vec<i32> = held.iter().map(|(id, _)| *id).collect();
                            let (names, _) = describe(&ids)?;
                            Some(
                                held.into_iter()
                                    .map(|(id, quantity)| {
                                        layouts::Item::read(
                                            id,
                                            quantity,
                                            names.get(&id).cloned().unwrap_or_default(),
                                            true,
                                        )
                                    })
                                    .collect(),
                            )
                        } else {
                            None
                        };
                        Ok(Read::Inventory { slots, pouch })
                    }
                }
            };
            Event::LayoutRead {
                run,
                kind,
                result: read(),
            }
        });
    }

    fn toggle_pause_of(&mut self, id: u64) {
        let Some(run) = self.run_by_id_mut(id) else {
            return;
        };
        let paused = run.is_paused();
        if let Some(script) = run.script.as_mut() {
            script.set_paused(!paused);
        }
        run.info(if paused {
            "Script resumed."
        } else {
            "Script paused."
        });
    }

    fn editor_run_mut(&mut self) -> Option<&mut Run> {
        self.runs
            .iter_mut()
            .find(|r| r.store.is_none() && r.is_running())
    }

    fn open_script(&mut self) {
        let mut dialog = rfd::FileDialog::new().add_filter("Luau script", SCRIPT_FILTER);
        if let Some(dir) = start_dir(&self.script_file.path) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.open_path(&path);
        }
    }

    fn open_path(&mut self, path: &std::path::Path) {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                self.script_file = ScriptFile {
                    path: path.display().to_string(),
                    disk: Some(text.clone()),
                    text,
                };
                self.last_script = Some(self.script_file.path.clone());
                self.info(format!("Opened {}.", path.display()));
            }
            Err(error) => {
                if self.last_script.as_deref() == path.to_str() {
                    self.last_script = None;
                }
                self.error(format!("Cannot open {}: {error}", path.display()));
            }
        }
    }

    fn run_prepared(
        &mut self,
        pid: u32,
        script: &StoreScript,
        prepared: Prepared,
        configured: Option<u64>,
        launch: Launch,
    ) -> Option<u64> {
        let title = script.title.as_str();
        let session_id = prepared.session.session_id.clone();
        let Some(target) = self
            .targets
            .iter()
            .find(|t| t.client.pid == pid && t.client.loaded.is_some())
            .cloned()
        else {
            self.script_error(format!(
                "The client {pid} is gone, so {title} wasn't started."
            ));
            self.end_session_job(session_id);
            return None;
        };
        match Script::start_with(
            &self.script_host(),
            &prepared.source,
            &prepared.modules,
            Some(&prepared.entry),
            None,
            Some(&target),
            &self.settings,
            &launch,
        ) {
            Ok(process) => {
                let interval =
                    Duration::from_secs(prepared.session.heartbeat_seconds.clamp(10, 120));
                let closed =
                    self.keep_session(session_id.clone(), process.process(), interval);
                let store = StoreRun {
                    session_id,
                    script_id: script.id.clone(),
                    community: script.community,
                    closed,
                    lib_version: prepared.lib_version.clone(),
                    price: prepared.session.price,
                    rank: prepared.session.rank,
                    fee: prepared.session.fee,
                    fee_charged: 0,
                };
                let id = match configured
                    .filter(|id| self.run_by_id(*id).is_some_and(Run::is_configuring))
                {
                    Some(id) => id,
                    None => {
                        let id = self.next_run_id();
                        let run = Run::new(id, title.to_owned(), None, None, None, &self.settings);
                        if self.queue_runners.iter().any(|r| r.pid == pid) {
                            self.runs.push(run);
                        } else {
                            self.add_run(run);
                        }
                        id
                    }
                };
                let quiet = self.queue_runners.iter().any(|r| r.pid == pid);
                let run = self.run_by_id_mut(id)?;
                run.script = Some(process);
                run.target = Some(target);
                run.store = Some(store);
                run.source = Some(Source::Store(script.clone()));
                run.form_sent = Some(launch.values);
                run.started = Instant::now();
                if let Some(warning) = &prepared.warning {
                    run.info(format!("Warning: {warning}"));
                }
                run.info(format!(
                    "Running {title} (revision {}).",
                    prepared.session.revision
                ));
                if !quiet {
                    self.active_run = Some(id);
                    self.log_tab = LogTab::Run(id);
                }
                self.feedback.fetched_at = None;
                Some(id)
            }
            Err(error) => {
                self.script_error(format!("Couldn't start {title}: {error}"));
                self.end_session_job(session_id);
                None
            }
        }
    }

    fn restart_file(&mut self, id: u64) {
        let Some(file) = self.run_by_id(id).and_then(|r| r.file.clone()) else {
            return;
        };
        match file.changed_on_disk() {
            Some(disk) => {
                self.reload_prompt = Some(ReloadPrompt {
                    disk,
                    restart: Some(id),
                })
            }
            None => self.rerun_file(id, file),
        }
    }

    fn rerun_file(&mut self, id: u64, file: ScriptFile) {
        let Some(run) = self.run_by_id(id) else {
            return;
        };
        let target = match &run.target {
            Some(old) => match self.free_client(old.client.pid) {
                Some(target) => Some(target.clone()),
                None => {
                    self.script_error(format!(
                        "Couldn't restart {}: its client is gone or busy.",
                        file.name()
                    ));
                    return;
                }
            },
            None => None,
        };
        self.remember_form(id);
        let previous = self.form_values(id);
        self.run_file(file, target.as_ref(), Some(id), previous);
    }

    fn run_file(
        &mut self,
        file: ScriptFile,
        target: Option<&Target>,
        replaces: Option<u64>,
        previous: Option<(serde_json::Value, serde_json::Value)>,
    ) {
        let name = file.name();
        let path = std::path::Path::new(&file.path);
        let manifest = if path.is_file() {
            match injector::manifest::Manifest::beside(path) {
                Ok(manifest) => manifest.unwrap_or_default(),
                Err(error) => {
                    return self.script_error(format!("Couldn't start the script: {error}"));
                }
            }
        } else {
            injector::manifest::Manifest::default()
        };
        let id = self.next_run_id();
        let mut run = Run::new(
            id,
            name.clone(),
            None,
            target.cloned(),
            None,
            &self.settings,
        );
        run.source = Some(Source::File(file.clone()));
        run.file = Some(file.clone());
        let values = match (&manifest.settings, previous) {
            (None, _) => Some(manifest.defaults()),
            (Some(spec), Some((ran_with, values))) if *spec == ran_with => Some(values),
            (Some(_), _) => None,
        };
        if let Some(spec) = manifest.settings.clone() {
            run.form = Some(self.settings_form(spec, &name));
            if values.is_none() {
                run.info(format!("Set up {name}, then press Start."));
            }
        }
        self.add_run(run);
        if let Some(old) = replaces {
            self.close_run(old);
        }
        if let Some(values) = values {
            self.start_file(id, &file, target, values);
        }
    }

    fn start_file(
        &mut self,
        id: u64,
        file: &ScriptFile,
        target: Option<&Target>,
        values: serde_json::Value,
    ) {
        let name = file.name();
        let path = Some(std::path::Path::new(&file.path)).filter(|p| p.is_file());
        let lib = Some(std::path::PathBuf::from(self.settings.lib_dir.trim()))
            .filter(|l| path.is_some() && !l.as_os_str().is_empty());
        if let Some(lib) = lib.as_ref().filter(|l| !l.is_dir()) {
            self.script_error(format!("Couldn't start the script: the lib folder {} doesn't exist. Fix it in the settings.", lib.display()));
            return;
        }
        let launch = Launch::with(values.clone());
        match Script::start(
            &self.script_host(),
            &file.text,
            path,
            lib.as_deref(),
            target,
            &self.settings,
            &launch,
        ) {
            Ok(script) => {
                if let Some(run) = self.run_by_id_mut(id) {
                    run.script = Some(script);
                    run.form_sent = Some(values);
                    run.started = Instant::now();
                    run.info(format!("Running {name}…"));
                }
            }
            Err(error) => {
                if let Some(run) = self.run_by_id_mut(id) {
                    run.error(format!("Couldn't start the script: {error}"));
                }
            }
        }
    }

    fn save_script(&mut self) {
        let path = self.script_file.path.clone();
        match std::fs::write(&path, &self.script_file.text) {
            Ok(()) => {
                self.script_file.disk = Some(self.script_file.text.clone());
                self.info(format!("Saved {path}."));
            }
            Err(error) => self.error(format!("Cannot save {path}: {error}")),
        }
    }

    fn save_script_as(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .add_filter("Luau script", SCRIPT_FILTER)
            .set_file_name(self.script_file.name());
        if let Some(dir) = start_dir(&self.script_file.path) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            return;
        };
        self.script_file.path = path.display().to_string();
        self.last_script = Some(self.script_file.path.clone());
        self.save_script();
    }

    fn widget_inspector_showing(&self) -> bool {
        self.tab == Tab::Development
            && self.dev_tab == DevTab::Inspectors
            && self.inspector_tab == InspectorTab::Widget
    }

    fn log_ref(&self, tab: LogTab) -> &Log {
        match tab {
            LogTab::Run(id) => self.run_by_id(id).map_or(&self.loader_log, |run| &run.log),
            LogTab::Loader => &self.loader_log,
        }
    }

    fn log_of(&mut self, tab: LogTab) -> &mut Log {
        match tab {
            LogTab::Run(id) => match run_in(&mut self.runs, Some(id)) {
                Some(run) => &mut run.log,
                None => &mut self.loader_log,
            },
            LogTab::Loader => &mut self.loader_log,
        }
    }

    fn save_text(&mut self, what: &str, file_name: &str, text: String) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Text", &["txt"])
            .set_file_name(file_name)
            .save_file()
        else {
            return;
        };
        let shown = path.display().to_string();
        match std::fs::write(&path, text) {
            Ok(()) => self.info(format!("Saved {what} to {shown}.")),
            Err(error) => self.error(format!("Cannot save {what} to {shown}: {error}")),
        }
    }

    fn rows_action<T: crate::pipe::Rows>(
        &mut self,
        action: Option<RowsAction>,
        what: &str,
        file_name: &str,
        rows: fn(&mut Self) -> &mut T,
    ) {
        match action {
            None => {}
            Some(RowsAction::Copy) => self.copy = Some(rows(self).shown_text()),
            Some(RowsAction::Save) => {
                let text = rows(self).shown_text();
                self.save_text(what, file_name, text);
            }
            Some(RowsAction::Pipe) => {
                let rows = rows(self);
                let next = rows.end();
                let toggled = crate::pipe::toggle(rows.pipe(), file_name, next);
                self.report_pipe(toggled);
            }
        }
    }

    fn report_pipe(&mut self, toggled: Result<Option<String>, String>) {
        match toggled {
            Ok(Some(message)) => self.info(message),
            Ok(None) => {}
            Err(error) => self.error(error),
        }
    }

    fn shown_run_log(&self) -> Option<&Run> {
        let id = match self.log_tab {
            LogTab::Run(id) => Some(id),
            LogTab::Loader => self.active_run,
        };
        id.and_then(|id| self.run_by_id(id))
    }

}

fn line_char_range(text: &str, line: usize) -> (usize, usize) {
    let mut start = 0;
    for (i, content) in text.split('\n').enumerate() {
        let len = content.trim_end_matches('\r').chars().count();
        if i == line {
            return (start, start + len);
        }
        start += content.chars().count() + 1;
    }
    (start, start)
}

struct TextWalk<'a> {
    text: &'a str,
    byte: usize,
    char: usize,
    line: usize,
}

impl<'a> TextWalk<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            byte: 0,
            char: 0,
            line: 0,
        }
    }

    fn advance(&mut self, to: usize) -> (usize, usize) {
        if let Some(part) = self.text.get(self.byte..to) {
            self.char += part.chars().count();
            self.line += part.bytes().filter(|&b| b == b'\n').count();
            self.byte = to;
        }
        (self.char, self.line)
    }
}

fn varps_report(read: Read, varps: &[i32], previous: Option<&[i32]>) -> Option<String> {
    let varp = |id: usize| varps.get(id).copied();
    Some(match read {
        Read::Varp(id) => match varp(id) {
            Some(value) => format!("{}.", varp_text(None, id, value)),
            None => format!("Varp {id}: no such varp, the client has {}.", varps.len()),
        },
        Read::KnownVarps => {
            let lines: Vec<String> = KNOWN_VARPS
                .iter()
                .map(|&(name, id)| match varp(id) {
                    Some(value) => format!("  {}", varp_text(Some(name), id, value)),
                    None => format!("  {name} ({id}): missing"),
                })
                .collect();
            format!("Known varps:\n{}", lines.join("\n"))
        }
        Read::VarpChanges => match previous {
            Some(previous) => {
                let changes: Vec<String> = varps
                    .iter()
                    .enumerate()
                    .filter(|&(id, value)| previous.get(id) != Some(value))
                    .map(|(id, value)| {
                        let before = previous
                            .get(id)
                            .map_or_else(|| "none".to_owned(), i32::to_string);
                        format!("  {} (was {before})", varp_text(None, id, *value))
                    })
                    .collect();
                match changes.len() {
                    0 => "No varps changed since the previous read.".into(),
                    n => format!(
                        "{n} varps changed since the previous read:\n{}",
                        changes.join("\n")
                    ),
                }
            }
            None => "Varps read; click again after changing something in game to see what changed."
                .into(),
        },
        _ => return None,
    })
}

fn varp_text(name: Option<&str>, id: usize, value: i32) -> String {
    let mut text = match name.or_else(|| known_varp(id)) {
        Some(name) => format!("{name} ({id}): {value}"),
        None => format!("Varp {id}: {value}"),
    };
    if let Some(meaning) = describe_varp(id, value) {
        let _ = write!(text, " ({meaning})");
    }
    text
}

#[derive(Clone, Copy)]
enum RowsAction {
    Copy,
    Save,
    Pipe,
}

fn pick_file(path: &mut String, name: &str, extension: &str) {
    let mut dialog = rfd::FileDialog::new().add_filter(name, &[extension]);
    if let Some(dir) = start_dir(path.as_str()) {
        dialog = dialog.set_directory(dir);
    }
    if let Some(picked) = dialog.pick_file() {
        *path = picked.display().to_string();
    }
}

fn pick_folder(path: &mut String) {
    let mut dialog = rfd::FileDialog::new();
    if let Some(dir) = Some(std::path::Path::new(path.as_str())).filter(|d| d.is_dir()) {
        dialog = dialog.set_directory(dir);
    }
    if let Some(picked) = dialog.pick_folder() {
        *path = picked.display().to_string();
    }
}

impl LoaderApp {
    pub fn tick(&mut self) {
        self.handle_events();
        self.handle_script_events();
        self.drive_queues();
        self.drive_xp();
        if self.last_save.elapsed() >= SAVE_EVERY {
            self.save();
        }
        self.wake.wake_after(SAVE_EVERY.saturating_sub(self.last_save.elapsed()));
        if matches!(
            self.update_state,
            UpdateState::Downloading(_) | UpdateState::Installing
        ) {
            return;
        }
        self.tiles_ui();
        self.poll_packets();
        self.poll_vars();
        self.poll_script_log();
        self.poll_inventory();
        self.poll_icons();
        self.poll_widget_mouse();
    }

    pub fn save(&mut self) {
        self.last_save = Instant::now();
        self.sync_queue_form();
        let storage = &mut self.storage;
        storage.set(crate::storage::SETTINGS_KEY, &self.settings);
        storage.set(SCRIPT_KEY, &self.script_file);
        storage.set(LAST_SCRIPT_KEY, &self.last_script);
        storage.set(FORMS_KEY, &self.remembered_forms);
        storage.set(crate::queue::STORAGE_KEY, &self.queues);
        if !self.layouts_unreadable {
            storage.set(LAYOUTS_KEY, &self.layouts);
        }
        storage.set(CURSOR_KEY, &self.cursor);
        storage.set(UI_FONT_KEY, &self.ui_font);
        storage.set(CONFIRMED_KEY, &self.confirmed);
        storage.set(
            HTTP_WHITELIST_KEY,
            &crate::script::read(&self.http_whitelist).user(),
        );
        storage.flush();
    }

    pub fn on_exit(&mut self) {
        if let Some(tiles) = self.tiles.take() {
            tiles.finish();
        }
        if let Some(terminal) = &mut self.terminal {
            terminal.close();
        }
        self.stop_all();
        backend::shutdown();
        self.save();
        self.storage.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_ignores_ascii_case_like_lowercasing_does() {
        let old = |text: &str, query: &str| -> Vec<usize> {
            text.to_ascii_lowercase()
                .match_indices(&query.to_ascii_lowercase())
                .map(|(at, _)| at)
                .collect()
        };
        for (text, query) in [
            ("Foo foo FOO", "foo"),
            ("aaaa", "aa"),
            ("ÄpfelÄ äpfel", "ä"),
            ("ÄpfelÄ äpfel", "PFEL"),
            ("über Über", "Über"),
            ("", "x"),
            ("short", "longer query"),
        ] {
            assert_eq!(
                find_matches(text, query, false),
                old(text, query),
                "{text:?} / {query:?}"
            );
        }
        assert_eq!(find_matches("Foo foo", "foo", true), [4]);
        assert!(find_matches("anything", "", false).is_empty());
    }

    #[test]
    fn the_log_keeps_the_newest_lines_and_its_filter() {
        let keep = *backend::LOG_LINES.start() * 2;
        let mut log = Log::new(keep);
        for i in 0..keep + keep / 4 + 1 {
            log.push(
                false,
                if i % 2 == 0 {
                    format!("[even] {i}")
                } else {
                    format!("{i}")
                },
            );
        }
        assert_eq!(log.lines.len(), keep);
        assert_eq!(
            log.lines.last().unwrap().text,
            format!("[even] {}", keep + keep / 4)
        );
        assert_eq!(log.text.lines().count(), keep);
        log.set_filter(Some(0));
        assert_eq!(log.view.len(), keep / 2);
        assert!(log.text.lines().all(|line| line.starts_with("[even]")));
        log.set_keep(keep / 2);
        assert_eq!(log.lines.len(), keep / 2);
        assert_eq!(log.view.len(), keep / 4);
        let revision = log.revision;
        log.clear();
        assert!(log.revision > revision);
        assert_eq!(log.keep, keep / 2);
    }

    #[test]
    fn a_dragged_selection_scrolls_as_fast_down_as_up_for_the_same_share_of_room() {
        let visible = egui::Rangef::new(300.0, 590.0);
        let window = egui::Rangef::new(0.0, 600.0);
        let speed = |y| selection_scroll_speed(y, visible, window);
        assert_eq!(speed(400.0), 0.0);
        assert_eq!(
            speed(300.0 - SELECTION_SCROLL_RAMP / 2.0),
            -SELECTION_SCROLL_MAX / 2.0
        );
        assert_eq!(speed(595.0), SELECTION_SCROLL_MAX / 2.0);
        assert_eq!(speed(0.0), -SELECTION_SCROLL_MAX);
        assert_eq!(speed(600.0), SELECTION_SCROLL_MAX);
        assert_eq!(speed(700.0), SELECTION_SCROLL_MAX);
    }

    #[test]
    fn a_selection_dragged_above_the_output_keeps_scrolling_up() {
        let ctx = egui::Context::default();
        let text: String = (0..500)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut time = 0.0;
        let mut frame = |events: Vec<egui::Event>| {
            time += 0.016;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                time: Some(time),
                ..Default::default()
            };
            let mut offset = 0.0;
            let mut output = ctx.run_ui(input, |ui| {
                ui.add_space(300.0);
                offset = egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        let mut read_only = text.as_str();
                        let output = egui::TextEdit::multiline(&mut read_only).show(ui);
                        scroll_selection_past_edge(ui, &output.response);
                    })
                    .state
                    .offset
                    .y;
            });
            output.textures_delta.clear();
            offset
        };
        for _ in 0..3 {
            frame(Vec::new());
        }
        let inside = egui::pos2(50.0, 400.0);
        frame(vec![egui::Event::PointerMoved(inside)]);
        let bottom = frame(vec![egui::Event::PointerButton {
            pos: inside,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }]);
        for y in [380.0, 340.0, 300.0, 250.0] {
            frame(vec![egui::Event::PointerMoved(egui::pos2(50.0, y))]);
        }
        let moved = frame(Vec::new());
        let held = (0..10).map(|_| frame(Vec::new())).last().unwrap();
        assert!(moved < bottom && held < moved, "{bottom} {moved} {held}");
    }

    #[test]
    fn log_sections_cover_runs_of_one_color() {
        let mut log = Log::new(*backend::LOG_LINES.start());
        for (tone, text) in [
            (Tone::Normal, "a"),
            (Tone::Normal, "bb"),
            (Tone::Error, "c\nd"),
            (Tone::Warning, "w"),
            (Tone::Normal, "e"),
        ] {
            log.push_tone(tone, text.into());
        }
        let (normal, warning, error) = (
            egui::Color32::WHITE,
            egui::Color32::YELLOW,
            egui::Color32::RED,
        );
        let job = log.layout_job(egui::FontId::monospace(13.0), normal, warning, error);
        let sections: Vec<_> = job
            .sections
            .iter()
            .map(|s| {
                (
                    &job.text[s.byte_range.start.0..s.byte_range.end.0],
                    s.format.color,
                )
            })
            .collect();
        assert_eq!(
            sections,
            [
                ("a\nbb\n", normal),
                ("c\nd\n", error),
                ("w\n", warning),
                ("e", normal)
            ]
        );
    }

    #[test]
    fn the_text_walk_counts_chars_and_lines() {
        let text = "é\nab\nc";
        let mut walk = TextWalk::new(text);
        assert_eq!(walk.advance(3), (2, 1));
        assert_eq!(walk.advance(7), (6, 2));
        assert_eq!(walk.advance(1), (6, 2));
        assert_eq!(walk.advance(99), (6, 2));
    }
}
