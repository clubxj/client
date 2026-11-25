use super::*;

use crate::ui::{theme, widgets};

const SIDEBAR_SLIDE: Duration = Duration::from_millis(200);
const SIDEBAR_FONT_SIZE: Pixels = px(15.0);
const SIDEBAR_INSET: Pixels = px(8.0);
const LOGO_SIZE: Pixels = px(32.0);

const GLOBAL_ITEMS: [(Page, &str, &str); 5] = [
    (Page::Terminal, "\u{f120}", "terminal"),
    (Page::Editor(None), "\u{f044}", "editor"),
    (Page::Accounts, "\u{f007}", "accounts"),
    (Page::Feedback, "\u{f075}", "feedback"),
    (Page::Settings, "\u{f013}", "settings"),
];

pub(super) type Pick = (Page, Option<u64>);

pub(super) struct Node {
    pub(super) pick: Pick,
    pub(super) name: String,
    label: String,
    color: Option<Rgba>,
    dim: bool,
    folds: bool,
    pub(super) children: Vec<Node>,
}

impl Node {
    fn new(pick: Pick, name: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            pick,
            name: name.into(),
            label: label.into(),
            color: None,
            dim: false,
            folds: false,
            children: Vec::new(),
        }
    }

    fn page(pick: Pick, label: &str) -> Self {
        let mut name = label.to_owned();
        if let Some(first) = name.get_mut(..1) {
            first.make_ascii_uppercase();
        }
        Self::new(pick, name, format!("{label}/"))
    }

    fn contains(&self, pick: Pick) -> bool {
        self.pick == pick || self.children.iter().any(|child| child.contains(pick))
    }
}

fn flatten(nodes: Vec<Node>, guide: &str, current: Pick, rows: &mut Vec<(String, Node)>) {
    let count = nodes.len();
    for (i, mut node) in nodes.into_iter().enumerate() {
        let last = i + 1 == count;
        let open = !node.folds || node.contains(current);
        let children = std::mem::take(&mut node.children);
        rows.push((format!("{guide}{}", if last { "└╴" } else { "├╴" }), node));
        if open {
            flatten(
                children,
                &format!("{guide}{}", if last { "  " } else { "│ " }),
                current,
                rows,
            );
        }
    }
}

pub(super) struct Chrome {
    pub(super) focus: FocusHandle,
    drawer: widgets::Slide,
    logo: Arc<gpui_kit::Image>,
}

impl Chrome {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            focus: cx.focus_handle(),
            drawer: widgets::Slide::new(SIDEBAR_SLIDE),
            logo: Arc::new(gpui_kit::Image::from_bytes(
                gpui_kit::ImageFormat::Svg,
                include_bytes!("../../assets/volcren.svg").to_vec(),
            )),
        }
    }
}

pub(super) fn sidebar_pick(page: Page, active_run: Option<u64>) -> Pick {
    match page {
        Page::Run(_) => (page, active_run),
        _ => (page, None),
    }
}

fn sidebar_row(
    id: impl Into<ElementId>,
    selected: bool,
    dim: bool,
    guide: String,
    label: (String, Option<Rgba>),
) -> Stateful<Div> {
    let text = if selected {
        theme::BLUE
    } else if dim {
        theme::COMMENT
    } else {
        theme::FG_DARK
    };
    div()
        .id(id)
        .flex()
        .items_center()
        .h(theme::INTERACT_HEIGHT)
        .px(px(8.0))
        .font_family(theme::MONO)
        .text_size(SIDEBAR_FONT_SIZE)
        .text_color(text)
        .cursor_pointer()
        .when(selected, |this| this.bg(theme::BG_HL))
        .when(!selected, |this| {
            this.hover(|style| {
                let style = style.bg(theme::BG_HL);
                if dim {
                    style
                } else {
                    style.text_color(theme::FG)
                }
            })
        })
        .child(
            div()
                .flex()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(div().flex_none().text_color(theme::COMMENT).child(guide))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .when_some(label.1, |this, color| this.text_color(color))
                        .child(label.0),
                ),
        )
}

impl LoaderApp {
    pub fn open_window(
        cx: &mut App,
        storage: Storage,
        instance: update::Instance,
    ) -> gpui_kit::Result<()> {
        let ui_font = storage.get(UI_FONT_KEY).unwrap_or_default();
        theme::install(cx, ui_font);
        let (wake, wakes) = Wake::channel();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(900.0), px(640.0)), cx)),
            window_min_size: Some(size(px(520.0), px(300.0))),
            titlebar: Some(TitlebarOptions {
                title: Some("Volcren".into()),
                ..Default::default()
            }),
            app_id: Some(crate::APP_ID.to_owned()),
            window_decorations: Some(WindowDecorations::Server),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, move |window, cx| {
            let gpu = window.gpu_specs().map(|gpu| {
                format!(
                    "{} ({}, {})",
                    gpu.device_name, gpu.driver_name, gpu.driver_info
                )
            });
            cx.new(|cx| {
                let mut app = Self::new(
                    storage,
                    Config::from_env(),
                    instance,
                    wake,
                    gpu,
                    Chrome::new(cx),
                );
                window.focus(&app.chrome.focus, cx);
                app.start();
                app.drive(wakes, cx);
                cx.on_app_quit(|app, _| {
                    app.on_exit();
                    async {}
                })
                .detach();
                app
            })
        })?;
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        Ok(())
    }

    fn drive(&mut self, wakes: async_channel::Receiver<Option<Duration>>, cx: &mut Context<Self>) {
        cx.spawn(async move |app, cx| {
            let mut due: Option<Instant> = None;
            loop {
                let wait = due.map(|at| at.saturating_duration_since(Instant::now()));
                let first = match wait {
                    None => match wakes.recv().await {
                        Ok(message) => Some(message),
                        Err(_) => return,
                    },
                    Some(wait) => {
                        let timer = cx.background_executor().timer(wait);
                        let message = std::pin::pin!(wakes.recv());
                        match futures::future::select(message, timer).await {
                            futures::future::Either::Left((Ok(message), _)) => Some(message),
                            futures::future::Either::Left((Err(_), _)) => return,
                            futures::future::Either::Right(_) => None,
                        }
                    }
                };
                let mut now = first.is_none();
                for message in first
                    .into_iter()
                    .chain(std::iter::from_fn(|| wakes.try_recv().ok()))
                {
                    match message {
                        None => now = true,
                        Some(delay) => {
                            let at = Instant::now() + delay;
                            due = Some(due.map_or(at, |due| due.min(at)));
                        }
                    }
                }
                if due.is_some_and(|at| at <= Instant::now()) {
                    now = true;
                }
                if now {
                    due = None;
                    if app.update(cx, |_, cx| cx.notify()).is_err() {
                        return;
                    }
                }
            }
        })
        .detach();
    }

    fn pick_page(
        pick: Pick,
        cx: &mut Context<Self>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |app: &mut Self, _: &ClickEvent, _, cx| {
            app.go(pick.0, pick.1);
            cx.notify();
        })
    }

    fn run_node(&self, run: &Run) -> Node {
        let mut node = Node::new(
            (Page::Run(RunPage::Overview), Some(run.id)),
            run.title.clone(),
            format!("{}/", sidebar_name(&run.title)),
        );
        node.dim = !run.is_live();
        let page = |sub: RunPage, label: &str| Node::page((Page::Run(sub), Some(run.id)), label);
        node.children.push(page(RunPage::Settings, "settings"));
        node.children.push(page(RunPage::Trackers, "trackers"));
        if run.store.is_some() {
            node.children.push(page(RunPage::Feedback, "feedback"));
        }
        node
    }

    pub(super) fn client_node(&self, pid: u32) -> Node {
        let mut node = Node::new(
            (Page::Client(pid), None),
            format!("Client {pid}"),
            format!("client {pid}/"),
        );
        match self.client_state(pid) {
            nav::ClientState::Unloaded => node.dim = true,
            nav::ClientState::Running => node.color = Some(theme::GREEN),
            nav::ClientState::Failed => node.color = Some(theme::RED),
            nav::ClientState::Idle => {}
        }
        let loaded = self.client_loaded(pid);
        let page = |page: Page, label: &str| Node::page((page, None), label);
        if self.queue_on(pid).is_some() {
            node.children.push(page(Page::Queue(pid), "queue"));
        }
        if let Some(run) = self.live_run_on(pid) {
            node.children.push(self.run_node(run));
        } else if loaded && self.queue_on(pid).is_none() {
            node.children.push(page(Page::Store(pid), "scripts"));
            node.children.push(page(Page::Queue(pid), "queue"));
            node.children.push(page(Page::Editor(Some(pid)), "editor"));
        }
        if loaded {
            node.children
                .push(page(Page::Development(pid), "development"));
        }
        let mut history = page(Page::History(pid), "history");
        history.folds = true;
        history.children = self
            .ended_runs_on(pid)
            .map(|run| {
                let mut run = self.run_node(run);
                run.folds = true;
                run
            })
            .collect();
        if !history.children.is_empty() {
            node.children.push(history);
        }
        node
    }

    fn sidebar_nodes(&self) -> Vec<Node> {
        let mut sessions = Node::new((Page::Sessions, None), "Sessions", "\u{f04b} sessions/");
        sessions.children = self
            .listed_clients()
            .into_iter()
            .map(|pid| self.client_node(pid))
            .collect();
        std::iter::once(sessions)
            .chain(GLOBAL_ITEMS.iter().map(|(page, icon, name)| {
                Node::new((*page, None), *name, format!("{icon} {name}/"))
            }))
            .collect()
    }

    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let current = sidebar_pick(self.page, self.active_run);
        let site_url = self.api.config().site_url.clone();
        let link = !site_url.is_empty();
        let mut flat = Vec::new();
        flatten(self.sidebar_nodes(), "", current, &mut flat);
        let rows: Vec<AnyElement> = flat
            .into_iter()
            .enumerate()
            .map(|(i, (guide, node))| {
                let pick = node.pick;
                sidebar_row(
                    ("sidebar-row", i),
                    current == pick,
                    node.dim,
                    guide,
                    (node.label, node.color),
                )
                .on_click(Self::pick_page(pick, cx))
                .into_any_element()
            })
            .collect();
        div()
            .id("sidebar")
            .size_full()
            .overflow_y_scroll()
            .bg(theme::BG_DARK)
            .pl(SIDEBAR_INSET)
            .py(px(6.0))
            .child(
                div()
                    .pt(px(4.0))
                    .pb(px(2.0))
                    .text_size(theme::SMALL)
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme::PURPLE)
                    .child("EXPLORER"),
            )
            .child(
                div()
                    .id("sidebar-root")
                    .text_size(SIDEBAR_FONT_SIZE)
                    .text_color(theme::YELLOW)
                    .child("\u{f233} volcren/")
                    .when(link, |this| {
                        this.cursor_pointer()
                            .hover(|style| style.text_color(theme::BLUE))
                            .on_click(move |_, _, _| open_site(&site_url, ""))
                    }),
            )
            .children(rows)
    }

    fn render_sidebar_handle(&self, open: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let hovered_color = theme::FG;
        let color = if open > 0.5 {
            theme::FG
        } else {
            theme::FG_DARK
        };
        div()
            .id("sidebar-handle")
            .group("sidebar-handle")
            .w(SIDEBAR_HANDLE_WIDTH_PX)
            .h_full()
            .flex_none()
            .bg(theme::BG_DARK)
            .cursor_pointer()
            .hover(|style| style.bg(theme::BG_HL))
            .on_click(cx.listener(|app, _: &ClickEvent, _, cx| {
                app.sidebar_open = !app.sidebar_open;
                cx.notify();
            }))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let center = bounds.center();
                        let (sin, cos) = (open * std::f32::consts::PI).sin_cos();
                        let mut path = PathBuilder::stroke(px(2.0));
                        for (i, (x, y)) in [(-3.0f32, -7.0f32), (3.0, 0.0), (-3.0, 7.0)]
                            .into_iter()
                            .enumerate()
                        {
                            let point = point(
                                center.x + px(x * cos - y * sin),
                                center.y + px(x * sin + y * cos),
                            );
                            if i == 0 {
                                path.move_to(point);
                            } else {
                                path.line_to(point);
                            }
                        }
                        if let Ok(path) = path.build() {
                            let hovered = window.mouse_position();
                            let color = if bounds.contains(&hovered) {
                                hovered_color
                            } else {
                                color
                            };
                            window.paint_path(path, color);
                        }
                    },
                )
                .size_full(),
            )
    }

    fn render_brand(&self, stacked: bool) -> Stateful<Div> {
        let site_url = self.api.config().site_url.clone();
        let link = !site_url.is_empty();
        let version = self.version_text();
        div()
            .id("header-brand")
            .group("header-brand")
            .flex()
            .flex_none()
            .items_center()
            .gap(theme::ITEM_GAP)
            .when(stacked, |this| this.flex_col())
            .child(
                img(ImageSource::Image(self.chrome.logo.clone()))
                    .size(LOGO_SIZE)
                    .flex_none(),
            )
            .child(
                div()
                    .text_size(px(22.0))
                    .font_weight(FontWeight::BOLD)
                    .when(link, |this| {
                        this.group_hover("header-brand", |style| style.text_color(theme::BLUE))
                    })
                    .child("VOLCREN"),
            )
            .tooltip(widgets::tooltip(version))
            .when(link, |this| {
                this.cursor_pointer()
                    .on_click(move |_, _, _| open_site(&site_url, ""))
            })
    }

    fn render_account(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
        let row = div().flex().items_center().gap(theme::ITEM_GAP).min_w_0();
        match &self.auth {
            Auth::SignedIn(me) => {
                let name = me.username.clone();
                let balance = crate::api::models::balance_label(me.microcredits as f64);
                let minutes_per_month = me.minutes_per_month;
                let rate: f64 = self
                    .store_runs()
                    .map(|s| s.rate_per_hour(minutes_per_month))
                    .sum();
                let site_url = self.api.config().site_url.clone();
                let sign_out_width =
                    widgets::icon_button_width(window, &[(ICON_SIGN_OUT, "Sign out")]);
                row.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::BOLD)
                        .child(name),
                )
                .child(if site_url.is_empty() {
                    widgets::credits(balance, "", false)
                        .id("balance")
                        .tooltip(widgets::tooltip("Your balance."))
                } else {
                    widgets::credits(balance, "", false)
                        .id("balance")
                        .cursor_pointer()
                        .hover(|style| style.text_color(theme::BLUE))
                        .tooltip(widgets::tooltip("Your balance. Click to add funds on the website."))
                        .on_click(move |_, _, _| open_site(&site_url, "/credits"))
                })
                .when(rate > 0.0, |this| {
                    this.child(
                        widgets::credits(crate::api::models::rate_label(rate), "/h", true)
                            .id("hourly-rate")
                            .tooltip(widgets::tooltip(
                                "What the scripts running from this loader cost an hour, prices and extra-session fees together.",
                            )),
                    )
                })
                .child(widgets::icon_button(
                    "sign-out",
                    ICON_SIGN_OUT,
                    "Sign out",
                    sign_out_width,
                    true,
                    cx.listener(|app, _: &ClickEvent, _, cx| {
                        app.sign_out(None);
                        cx.notify();
                    }),
                ))
            }
            Auth::SigningIn(cancel) => {
                let cancel = cancel.clone();
                row.child(Spinner::new())
                    .child("Signing in…")
                    .child(widgets::button(
                        "cancel-sign-in",
                        "Cancel",
                        true,
                        move |_, _, _| {
                            cancel.store(true, Ordering::Relaxed);
                        },
                    ))
            }
            Auth::Restoring => row.child(Spinner::new()),
            Auth::SignedOut => row.child(widgets::button(
                "sign-in",
                "Sign in",
                true,
                cx.listener(|app, _: &ClickEvent, _, cx| {
                    app.start_sign_in();
                    cx.notify();
                }),
            )),
        }
    }

    fn render_header(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
        let width = window.viewport_size().width;
        let wrap_account = width < px(HEADER_COMPACT_WINDOW);
        let stack_account = width < px(HEADER_STACK_WINDOW);
        let stack_brand = width < px(HEADER_TINY_WINDOW);
        let brand = self.render_brand(stack_brand);
        let account = self.render_account(window, cx);
        let header = div()
            .flex_none()
            .px(px(8.0))
            .py(px(6.0))
            .border_b_1()
            .border_color(theme::BORDER);
        if !wrap_account {
            header
                .flex()
                .items_center()
                .h(LOGO_SIZE + px(12.0))
                .gap(theme::ITEM_GAP)
                .child(brand)
                .child(div().flex().flex_1().min_w_0().justify_end().child(account))
        } else {
            header
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.0))
                .child(brand)
                .child(if stack_account {
                    account.flex_col()
                } else {
                    account
                })
        }
    }

    fn render_update_banner(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let available = self.update.as_ref()?;
        let (release, kind) = (available.release.clone(), available.kind);
        if kind == update::Kind::Dev {
            return None;
        }
        let running = self.start_in_progress();
        let failure = match &self.update_state {
            UpdateState::Failed(error) => Some(error.clone()),
            _ => None,
        };
        let main_button = match (&failure, kind) {
            (_, update::Kind::Manual) => "Download",
            (Some(_), _) => "Retry",
            _ => "Update and restart",
        };
        let enabled = kind == update::Kind::Manual || !running;
        let tip = if !enabled {
            Some("Stop running scripts to update.")
        } else if kind == (update::Kind::Auto { admin: true }) {
            Some("An administrator has to approve the update.")
        } else {
            None
        };
        let main = widgets::button(
            "update",
            main_button,
            enabled,
            cx.listener(move |app, _: &ClickEvent, _, cx| {
                if kind == update::Kind::Manual {
                    if let Some(url) = release.download_url(&app.api) {
                        let _ = webbrowser::open(&url);
                    }
                } else {
                    app.start_update(release.clone());
                }
                cx.notify();
            }),
        )
        .when_some(tip, |this, tip| this.tooltip(widgets::tooltip(tip)));
        let version = self.update.as_ref()?.release.version.clone();
        Some(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(theme::ITEM_GAP)
                .px(px(8.0))
                .py(px(2.0))
                .border_t_1()
                .border_color(theme::BORDER)
                .child(
                    div()
                        .id("update-version")
                        .min_w_0()
                        .truncate()
                        .text_color(theme::BLUE)
                        .tooltip(widgets::tooltip(format!(
                            "You have version {}",
                            update::VERSION
                        )))
                        .child(format!("Version {version} is available")),
                )
                .when_some(failure, |this, error| {
                    this.child(
                        div()
                            .id("update-error")
                            .min_w_0()
                            .truncate()
                            .text_color(theme::RED)
                            .tooltip(widgets::tooltip(error.clone()))
                            .child(error),
                    )
                })
                .child(div().flex_1())
                .child(main)
                .child(
                    div()
                        .id("dismiss-update")
                        .px(px(4.0))
                        .cursor_pointer()
                        .hover(|style| style.text_color(theme::BLUE))
                        .tooltip(widgets::tooltip("Dismiss"))
                        .on_click(cx.listener(|app, _: &ClickEvent, _, cx| {
                            app.update = None;
                            cx.notify();
                        }))
                        .child(ICON_CANCEL),
                ),
        )
    }

    fn render_updating(&mut self, cx: &mut Context<Self>) -> Div {
        let status = match &self.update_state {
            UpdateState::Downloading(progress) => progress_text(progress),
            _ => "Installing...".to_owned(),
        };
        let version = self.update.as_ref().map(|a| a.release.version.clone());
        let downloading = matches!(self.update_state, UpdateState::Downloading(_));
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(theme::LINE_GAP)
            .child(div().flex_none().h(relative(0.35)))
            .child(widgets::heading("Updating VOLCREN"))
            .when_some(version, |this, version| {
                this.child(format!("Version {version}"))
            })
            .child(status)
            .when(downloading, |this| {
                this.child(div().h(px(8.0))).child(widgets::button(
                    "skip-update",
                    "Skip for now",
                    true,
                    cx.listener(|app, _: &ClickEvent, _, cx| {
                        if let UpdateState::Downloading(progress) = &app.update_state {
                            progress.cancel();
                        }
                        app.update_state = UpdateState::Offered;
                        cx.notify();
                    }),
                ))
            })
    }

    fn render_login_gate(&mut self, cx: &mut Context<Self>) -> Div {
        let gate = div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.0))
            .child(div().flex_none().h(relative(0.3)).min_h(px(8.0)))
            .child(
                div()
                    .text_size(px(20.0))
                    .font_weight(FontWeight::BOLD)
                    .child("Sign in to continue"),
            );
        match &self.auth {
            Auth::Restoring => gate.child(Spinner::new()).child(RESTORING_SIGN_IN),
            Auth::SigningIn(cancel) => {
                let cancel = cancel.clone();
                gate.child(Spinner::new())
                    .child(WAITING_FOR_BROWSER)
                    .child(widgets::button(
                        "cancel-sign-in",
                        "Cancel",
                        true,
                        move |_, _, _| {
                            cancel.store(true, Ordering::Relaxed);
                        },
                    ))
            }
            _ => gate
                .child("Volcren needs a Discord account. Your browser will open to sign in.")
                .child(
                    widgets::button(
                        "sign-in-discord",
                        "Sign in with Discord",
                        true,
                        cx.listener(|app, _: &ClickEvent, _, cx| {
                            app.start_sign_in();
                            cx.notify();
                        }),
                    )
                    .text_size(px(16.0))
                    .h(px(28.0)),
                ),
        }
    }

    fn follow_tab(&mut self) {
        self.repair_page();
        if let Some(pid) = self.page_client() {
            self.focus_client = Some(pid);
        }
        self.repair_log_tab();
        let followed = (self.page, self.context_run());
        if followed != self.followed_tab {
            if self.page == Page::Sessions && self.followed_tab.0 != Page::Sessions && !self.scanning {
                self.scan();
            }
            self.followed_tab = followed;
            self.log_collapsed =
                matches!(self.page, Page::Accounts | Page::Settings | Page::Feedback);
            self.log_tab = followed.1.map_or(LogTab::Loader, LogTab::Run);
        }
    }

    fn render_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.page {
            Page::Sessions => self.render_sessions(window, cx),
            Page::Client(pid) => self.render_client(pid, window, cx),
            Page::Store(pid) => self.render_store(pid, window, cx),
            Page::Queue(pid) => self.render_queue(pid, window, cx),
            Page::Editor(owner) => self.render_editor(owner, window, cx),
            Page::Development(pid) => self.render_development(pid, window, cx),
            Page::History(pid) => self.render_history(pid, window, cx),
            Page::Run(sub) => self.render_run(sub, window, cx),
            Page::Terminal => self.render_terminal(window, cx),
            Page::Accounts => self.render_accounts(window, cx),
            Page::Feedback => self.render_feedback(window, cx),
            Page::Settings => self.render_settings(window, cx),
        }
    }

    fn render_central(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let width = window.viewport_size().width;
        let margin = (width * 0.02).clamp(px(8.0), px(16.0));
        self.follow_tab();
        let page = self.render_page(window, cx);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .relative()
            .p(margin)
            .child(div().flex().flex_col().flex_1().min_h_0().child(page))
    }

    fn render_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let narrow = window.viewport_size().width < px(SIDEBAR_MIN_WINDOW);
        let body = div().flex().flex_1().min_h_0().relative();
        if !narrow {
            self.sidebar_open = false;
            self.chrome.drawer.set(false);
            let sidebar = self.render_sidebar(cx);
            let central = self.render_central(window, cx);
            return body
                .child(
                    div()
                        .w(px(SIDEBAR_WIDTH))
                        .flex_none()
                        .h_full()
                        .child(sidebar),
                )
                .child(central);
        }
        self.chrome.drawer.set(self.sidebar_open);
        let slide = self.chrome.drawer.value(window);
        let width = px(SIDEBAR_WIDTH).min(window.viewport_size().width * 0.85);
        let handle = self.render_sidebar_handle(slide, cx);
        let central = self.render_central(window, cx);
        let drawer = (slide > 0.0).then(|| {
            let sidebar = self.render_sidebar(cx);
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .id("sidebar-backdrop")
                        .absolute()
                        .inset_0()
                        .bg(theme::alpha(gpui_kit::rgb(0x000000), 130.0 / 255.0 * slide))
                        .on_click(cx.listener(|app, _: &ClickEvent, _, cx| {
                            app.sidebar_open = false;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(-(width + SIDEBAR_HANDLE_WIDTH_PX) * (1.0 - slide))
                        .flex()
                        .child(div().w(width).h_full().occlude().child(sidebar))
                        .child(self.render_sidebar_handle(slide, cx)),
                )
        });
        body.child(handle).child(central).children(drawer)
    }

    fn render_root(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let root = div()
            .size_full()
            .flex()
            .flex_col()
            .relative()
            .bg(theme::BG)
            .text_color(theme::FG)
            .text_size(theme::BODY)
            .font_family(theme::ui_family(self.ui_font))
            .track_focus(&self.chrome.focus)
            .on_key_down(cx.listener(|app, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && app.sidebar_open {
                    app.sidebar_open = false;
                    cx.notify();
                }
            }));
        let mut root = root;
        root.text_style().font_fallbacks = Some(theme::fallbacks());
        if matches!(
            self.update_state,
            UpdateState::Downloading(_) | UpdateState::Installing
        ) {
            return root.child(self.render_updating(cx));
        }
        let reload_prompt = self.render_reload_prompt(window, cx);
        let header = self.render_header(window, cx);
        let banner = self.render_update_banner(cx);
        let signed_in = self.auth.me().is_some();
        let body = if signed_in {
            self.render_body(window, cx)
        } else {
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(self.render_login_gate(cx))
        };
        let output = self.render_output(window, cx);
        let dialogs: Vec<AnyElement> = if signed_in {
            [
                self.render_start_prompt(window, cx),
                self.render_warning(window, cx),
            ]
            .into_iter()
            .flatten()
            .collect()
        } else {
            Vec::new()
        };
        root.child(header)
            .child(body)
            .child(output)
            .children(banner)
            .children(dialogs)
            .children(reload_prompt)
    }
}

const SIDEBAR_HANDLE_WIDTH_PX: Pixels = px(SIDEBAR_HANDLE_WIDTH);

impl Render for LoaderApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.tick();
        if self.quit {
            cx.quit();
        }
        if let Some(text) = self.copy.take() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
        self.render_root(window, cx)
    }
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("signed-out", |app| app.auth = Auth::SignedOut),
        ("sidebar-tree", sidebar_tree),
    ]
}

#[cfg(test)]
fn sidebar_tree(app: &mut LoaderApp) {
    super::sessions::populated(app);
    app.page = Page::Run(RunPage::Settings);
    app.active_run = app.runs.iter().find(|r| r.is_running()).map(|r| r.id);
}
