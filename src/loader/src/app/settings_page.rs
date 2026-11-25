use super::*;

use gpui_kit::component::Sizable;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState, NumberInput};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{Entity, Focusable};

use crate::ui::{theme, widgets};

const FIELD_MIN: Pixels = px(120.0);
const FIELD_MAX: Pixels = px(760.0);
const LOG_LINES_WIDTH: Pixels = px(120.0);
const LOG_LINES_STEP: f64 = 50.0;

#[derive(Clone, Copy)]
enum PathField {
    SdkDir,
    Helper,
}

impl PathField {
    fn label(self) -> &'static str {
        match self {
            Self::SdkDir => "SDK folder:",
            Self::Helper => "injector.exe:",
        }
    }

    fn tooltip(self) -> Option<&'static str> {
        match self {
            Self::SdkDir => Some(
                "Optional. Scripts you run from a file read require(\"@volcren/...\") from this folder's modules, e.g. your client checkout's sdk/luau, without rebuilding injector.exe. Store scripts always use the SDK built into their injector.",
            ),
            Self::Helper => None,
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            Self::SdkDir => "empty: the SDK built into injector.exe",
            Self::Helper => "",
        }
    }

    fn of(self, settings: &mut Settings) -> &mut String {
        match self {
            Self::SdkDir => &mut settings.sdk_dir,
            Self::Helper => &mut settings.helper,
        }
    }

    fn browse(self, path: &mut String) {
        match self {
            Self::SdkDir => pick_folder(path),
            Self::Helper => pick_file(path, "Executable", "exe"),
        }
    }

    fn shown() -> Vec<Self> {
        if cfg!(windows) {
            vec![Self::SdkDir]
        } else {
            vec![Self::SdkDir, Self::Helper]
        }
    }
}

const LOG_LINES_TOOLTIP: &str = "How many lines each output tab keeps. A tab lays out all of its lines again whenever one is added, so more lines cost more CPU and memory.";
const UI_FONT_TOOLTIP: &str = "The font of the pages, buttons and labels. The output, the editor, the terminal and the inspectors stay monospace.";

#[derive(Clone)]
struct Inputs {
    sdk_dir: Entity<InputState>,
    helper: Entity<InputState>,
    log_lines: Entity<InputState>,
    whitelist: Entity<InputState>,
    proxy: [Entity<InputState>; 5],
}

impl Inputs {
    fn path(&self, field: PathField) -> &Entity<InputState> {
        match field {
            PathField::SdkDir => &self.sdk_dir,
            PathField::Helper => &self.helper,
        }
    }
}

#[derive(Default)]
pub(super) struct Ui {
    inputs: Option<Inputs>,
}

fn sync_input(input: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut App) {
    if input.read(cx).value().as_ref() != value {
        let value = value.to_owned();
        input.update(cx, |state, cx| state.set_value(value, window, cx));
    }
}

impl LoaderApp {
    fn settings_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Inputs {
        if let Some(inputs) = &self.settings_page.inputs {
            return inputs.clone();
        }
        let path = |field: PathField, window: &mut Window, cx: &mut Context<Self>| {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(field.placeholder()));
            cx.subscribe_in(
                &input,
                window,
                move |app, input, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        *field.of(&mut app.settings) = input.read(cx).value().to_string();
                        cx.notify();
                    }
                },
            )
            .detach();
            input
        };
        let sdk_dir = path(PathField::SdkDir, window, cx);
        let helper = path(PathField::Helper, window, cx);
        let log_lines = cx.new(|cx| {
            InputState::new(window, cx)
                .step(LOG_LINES_STEP)
                .min(*backend::LOG_LINES.start() as f64)
                .max(*backend::LOG_LINES.end() as f64)
        });
        cx.subscribe_in(
            &log_lines,
            window,
            |app, input, event: &InputEvent, _, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let Ok(keep) = input.read(cx).value().trim().parse::<usize>() else {
                    return;
                };
                if backend::LOG_LINES.contains(&keep) && keep != app.settings.log_lines {
                    app.set_log_lines(keep);
                    cx.notify();
                }
            },
        )
        .detach();
        let whitelist =
            cx.new(|cx| InputState::new(window, cx).placeholder("e.g. api.example.com"));
        cx.subscribe_in(
            &whitelist,
            window,
            |app, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    app.whitelist_page
                        .set_draft_input(input.read(cx).value().to_string());
                }
                InputEvent::PressEnter { .. } => {
                    app.whitelist_click(crate::whitelist::Click::Save, window, cx);
                }
                InputEvent::Focus | InputEvent::Blur => {}
            },
        )
        .detach();
        let proxy = crate::proxies::Field::ALL.map(|field| {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(field.placeholder()));
            if field == crate::proxies::Field::Password {
                input.update(cx, |state, cx| state.set_masked(true, window, cx));
            }
            cx.subscribe_in(
                &input,
                window,
                move |app, input, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        app.proxies.set_draft_value(field, value);
                    }
                    InputEvent::PressEnter { .. } => {
                        app.proxies_click(crate::proxies::Click::Save, window, cx);
                    }
                    InputEvent::Focus | InputEvent::Blur => {}
                },
            )
            .detach();
            input
        });
        let inputs = Inputs {
            sdk_dir,
            helper,
            log_lines,
            whitelist,
            proxy,
        };
        self.settings_page.inputs = Some(inputs.clone());
        inputs
    }

    fn set_log_lines(&mut self, keep: usize) {
        self.settings.log_lines = keep;
        self.loader_log.set_keep(keep);
        for run in &mut self.runs {
            run.log.set_keep(keep);
        }
    }

    fn whitelist_click(
        &mut self,
        click: crate::whitelist::Click,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let adding = matches!(click, crate::whitelist::Click::Add);
        self.whitelist_page
            .click(click, &mut crate::script::write(&self.http_whitelist));
        if adding && let Some(inputs) = &self.settings_page.inputs {
            inputs.whitelist.update(cx, |state, cx| {
                state.set_value("", window, cx);
                state.focus(window, cx);
            });
        }
        cx.notify();
    }

    fn proxies_click(
        &mut self,
        click: crate::proxies::Click,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let opening = matches!(
            click,
            crate::proxies::Click::Add | crate::proxies::Click::Edit(_)
        );
        let assigned = &self.assigned_proxies;
        let change = self
            .proxies
            .click(click, &|name: &str| assigned.user_of(name));
        if let Some(change) = change {
            self.proxies_changed(change);
        }
        if opening && let Some(inputs) = &self.settings_page.inputs {
            for (field, input) in crate::proxies::Field::ALL.iter().zip(&inputs.proxy) {
                let value = self.proxies.draft_value(*field).unwrap_or_default();
                sync_input(input, value, window, cx);
            }
            inputs.proxy[0].update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    pub(super) fn render_settings(
        &mut self,
        section: Option<SettingsSection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let inputs = self.settings_inputs(window, cx);
        let (body, dialogs) = match section {
            None => {
                let system = self.render_system_settings(&inputs, window, cx);
                let interface = self.render_interface_settings(&inputs, window, cx);
                let sections = self.render_settings_sections(window, cx);
                let body = div()
                    .flex()
                    .flex_col()
                    .child(settings_block("System", system))
                    .child(div().mt(SECTION_GAP).child(settings_block("Interface", interface)))
                    .child(div().mt(SECTION_GAP).child(settings_block("Advanced", sections)));
                (body, Vec::new())
            }
            Some(section) => {
                let (body, dialogs) = match section {
                    SettingsSection::Network => self.render_network_settings(&inputs, window, cx),
                    SettingsSection::Transportation => {
                        (self.render_transport_settings(cx), Vec::new())
                    }
                };
                let page = div()
                    .flex()
                    .flex_col()
                    .child(div().flex().child(self.render_settings_back(window, cx)))
                    .child(div().mt(BACK_GAP).child(body));
                (page, dialogs)
            }
        };
        div()
            .id("settings-page")
            .size_full()
            .overflow_y_scrollbar()
            .child(div().flex().flex_col().pr(px(12.0)).child(body))
            .children(dialogs)
            .into_any_element()
    }

    fn render_settings_sections(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let labels: Vec<(&str, &str)> = SettingsSection::ALL
            .iter()
            .map(|section| (section.icon(), section.title()))
            .collect();
        let width = widgets::icon_button_width(window, &labels).max(SECTION_BUTTON_WIDTH);
        let buttons = SettingsSection::ALL.into_iter().map(|section| {
            widgets::icon_button(
                ("settings-section", section as usize),
                section.icon(),
                section.title(),
                width,
                true,
                cx.listener(move |app, _: &ClickEvent, _, cx| {
                    app.go(Page::Settings(Some(section)), None);
                    cx.notify();
                }),
            )
        });
        div().flex().flex_col().gap(theme::ITEM_GAP).children(buttons)
    }

    fn render_settings_back(&self, window: &Window, cx: &mut Context<Self>) -> Stateful<Div> {
        widgets::icon_button(
            "settings-back",
            BACK_ICON,
            "Back",
            widgets::icon_button_width(window, &[(BACK_ICON, "Back")]),
            true,
            cx.listener(|app, _: &ClickEvent, _, cx| {
                app.go(Page::Settings(None), None);
                cx.notify();
            }),
        )
    }

    fn render_interface_settings(
        &mut self,
        inputs: &Inputs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        if !inputs
            .log_lines
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
        {
            sync_input(
                &inputs.log_lines,
                &self.settings.log_lines.to_string(),
                window,
                cx,
            );
        }
        let app = cx.entity().downgrade();
        let fonts = crate::ui::theme::UiFont::ALL
            .into_iter()
            .map(|font| {
                let app = app.clone();
                widgets::Choice::new(font.label(), font == self.ui_font, move |_, cx| {
                    app.update(cx, |app, cx| {
                        if app.ui_font != font {
                            app.ui_font = font;
                            crate::ui::theme::set_ui_font(cx, font);
                        }
                        cx.notify();
                    })
                    .ok();
                })
            })
            .collect();
        settings_form(vec![
            (
                label("Output lines:", Some(LOG_LINES_TOOLTIP)),
                grid_cell().child(
                    div()
                        .w(LOG_LINES_WIDTH)
                        .child(NumberInput::new(&inputs.log_lines).small()),
                ),
            ),
            (
                label("Interface font:", Some(UI_FONT_TOOLTIP)),
                grid_cell().child(widgets::combo("ui-font", self.ui_font.label(), fonts)),
            ),
        ])
    }

    fn render_system_settings(
        &mut self,
        inputs: &Inputs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let rows = PathField::shown()
            .into_iter()
            .map(|field| {
                let value = field.of(&mut self.settings).clone();
                sync_input(inputs.path(field), &value, window, cx);
                let control = grid_cell()
                    .gap(theme::ITEM_GAP)
                    .child(
                        div()
                            .flex_1()
                            .min_w(FIELD_MIN)
                            .max_w(FIELD_MAX)
                            .child(Input::new(inputs.path(field)).small()),
                    )
                    .child(widgets::button(
                        (field.label(), 1usize),
                        "Browse…",
                        true,
                        cx.listener(move |app, _: &ClickEvent, _, cx| {
                            field.browse(field.of(&mut app.settings));
                            cx.notify();
                        }),
                    ));
                (label(field.label(), field.tooltip()), control)
            })
            .collect();
        settings_form(rows)
    }

    fn render_network_settings(
        &mut self,
        inputs: &Inputs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Div, Vec<AnyElement>) {
        if let Some(text) = self.whitelist_page.draft_input().map(str::to_owned) {
            sync_input(&inputs.whitelist, &text, window, cx);
        }
        for (field, input) in crate::proxies::Field::ALL.iter().zip(&inputs.proxy) {
            if let Some(value) = self.proxies.draft_value(*field) {
                sync_input(input, value, window, cx);
            }
        }
        let on_whitelist = cx.entity().downgrade();
        let (whitelist, whitelist_dialog) = self.whitelist_page.render(
            &crate::script::read(&self.http_whitelist),
            &inputs.whitelist,
            window,
            std::rc::Rc::new(move |click, window, cx| {
                on_whitelist
                    .update(cx, |app, cx| app.whitelist_click(click, window, cx))
                    .ok();
            }),
        );
        let on_proxies = cx.entity().downgrade();
        let (proxies, proxy_dialog) = self.proxies.render(
            &inputs.proxy,
            window,
            std::rc::Rc::new(move |click, window, cx| {
                on_proxies
                    .update(cx, |app, cx| app.proxies_click(click, window, cx))
                    .ok();
            }),
        );
        let body = div()
            .flex()
            .flex_col()
            .child(whitelist)
            .child(div().mt(SECTION_GAP).child(proxies));
        (
            body,
            whitelist_dialog.into_iter().chain(proxy_dialog).collect(),
        )
    }

    fn render_transport_settings(&mut self, cx: &mut Context<Self>) -> Div {
        let toggles = injector::transports::TYPES.iter().map(|&(name, label)| {
            let on = !self.settings.transports.off.iter().any(|off| off == name);
            Checkbox::new(name)
                .label(label)
                .checked(on)
                .on_click(cx.listener(move |app, on: &bool, _, cx| {
                    app.set_transport(name, *on);
                    cx.notify();
                }))
        });
        let toggles: Vec<_> = toggles.collect();
        let mut grid = div().flex().flex_col().gap(theme::LINE_GAP);
        let mut toggles = toggles.into_iter();
        while let Some(first) = toggles.next() {
            grid = grid.child(
                div()
                    .flex()
                    .gap(theme::ITEM_GAP)
                    .child(div().w(TRANSPORT_COLUMN).child(first))
                    .children(
                        toggles
                            .next()
                            .map(|second| div().w(TRANSPORT_COLUMN).child(second)),
                    ),
            );
        }
        let data = crate::transports::status();
        if data.busy {
            self.wake.wake_after(Duration::from_millis(250));
        }
        let update_enabled = !data.busy;
        let roll_back_enabled = data.can_roll_back && !data.busy;
        div()
            .flex()
            .flex_col()
            .gap(theme::LINE_GAP)
            .child(widgets::heading("Transportation"))
            .child(widgets::weak(
                "What scripts may use to get around. A script that needs one that is off stops with an error saying so.",
            ))
            .child(grid)
            .child(div().pt(px(6.0)).child(format!("Data: {}", data.version)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(theme::ITEM_GAP)
                    .child(
                        widgets::button(
                            "transports-update",
                            "Check for update",
                            update_enabled,
                            |_, _, _| crate::transports::update_now(),
                        )
                        .when(update_enabled, |this| {
                            this.tooltip(widgets::tooltip(
                                "Asks for the newest data now. The loader also does this once a day.",
                            ))
                        }),
                    )
                    .child(
                        widgets::button(
                            "transports-roll-back",
                            "Roll back",
                            roll_back_enabled,
                            |_, _, _| crate::transports::roll_back_now(),
                        )
                        .when(roll_back_enabled, |this| {
                            this.tooltip(widgets::tooltip(
                                "Goes back to the version before this one, if this one causes trouble.",
                            ))
                        }),
                    )
                    .when(data.busy, |this| this.child(Spinner::new().small())),
            )
            .when(!data.message.is_empty(), |this| {
                this.child(widgets::weak(data.message.clone()))
            })
    }

    fn set_transport(&mut self, name: &str, on: bool) {
        let off = &mut self.settings.transports.off;
        if on {
            off.retain(|o| o != name);
        } else if !off.iter().any(|o| o == name) {
            off.push(name.to_owned());
        }
        let settings = self.settings.transports.clone();
        for run in self.runs.iter_mut().filter(|r| r.is_running()) {
            if let Err(error) = run.send(|s| s.push_transport_settings(&settings)) {
                run.error(error);
            }
        }
    }
}

const TRANSPORT_COLUMN: Pixels = px(220.0);
const SECTION_GAP: Pixels = px(44.0);
const FIELD_GAP: Pixels = px(16.0);
const SECTION_BUTTON_WIDTH: Pixels = px(200.0);
const BACK_GAP: Pixels = px(20.0);
const BACK_ICON: &str = "\u{f060}";
const HEADING_GAP: Pixels = px(10.0);

impl SettingsSection {
    pub(super) const ALL: [Self; 2] = [Self::Network, Self::Transportation];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Transportation => "transportation",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Network => "Network",
            Self::Transportation => "Transportation",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Network => "\u{f0ac}",
            Self::Transportation => "\u{f277}",
        }
    }
}

fn grid_cell() -> Div {
    div().flex().items_center().h(theme::INTERACT_HEIGHT)
}

fn label(text: &'static str, tooltip: Option<&'static str>) -> Div {
    grid_cell().child(
        div()
            .id(text)
            .whitespace_nowrap()
            .child(text)
            .when_some(tooltip, |this, tooltip| {
                this.tooltip(widgets::tooltip(tooltip))
            }),
    )
}

fn settings_block(title: &'static str, body: Div) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(HEADING_GAP)
        .child(widgets::heading(title))
        .child(body)
}

fn settings_form(rows: Vec<(Div, Div)>) -> Div {
    div().flex().flex_col().gap(FIELD_GAP).children(
        rows.into_iter()
            .map(|(label, control)| div().flex().flex_col().child(label).child(control)),
    )
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("settings", |app| app.page = Page::Settings(None)),
        ("settings-network", |app| {
            app.page = Page::Settings(Some(SettingsSection::Network))
        }),
        ("settings-transportation", |app| {
            app.page = Page::Settings(Some(SettingsSection::Transportation))
        }),
        ("settings-proxies", |app| {
            app.page = Page::Settings(Some(SettingsSection::Network));
            app.proxies = crate::proxies::Proxies::with(snapshot_proxies());
        }),
        ("settings-proxy-dialog", |app| {
            app.page = Page::Settings(Some(SettingsSection::Network));
            app.proxies = crate::proxies::Proxies::with(snapshot_proxies());
            app.proxies.click(
                crate::proxies::Click::Edit("Residential US".into()),
                &|_| None,
            );
        }),
        ("settings-whitelist", |app| {
            app.page = Page::Settings(Some(SettingsSection::Network));
            let mut list = crate::script::write(&app.http_whitelist);
            list.add("api.example.com").unwrap();
            list.add("*.example.org").unwrap();
            drop(list);
            app.whitelist_page.click(
                crate::whitelist::Click::Delete("*.example.org".into()),
                &mut crate::script::write(&app.http_whitelist),
            );
        }),
        ("settings-whitelist-dialog", |app| {
            app.page = Page::Settings(Some(SettingsSection::Network));
            let mut list = crate::script::write(&app.http_whitelist);
            app.whitelist_page
                .click(crate::whitelist::Click::Add, &mut list);
            app.whitelist_page.set_draft_input("not a host".into());
            app.whitelist_page
                .click(crate::whitelist::Click::Save, &mut list);
        }),
    ]
}

#[cfg(test)]
pub(super) fn snapshot_proxies() -> Vec<crate::proxies::Saved> {
    vec![
        crate::proxies::Saved {
            name: "Residential US".into(),
            host: "us.proxy.example.com".into(),
            port: 1080,
            username: "alt-one".into(),
            password: "secret".into(),
        },
        crate::proxies::Saved {
            name: "Home server".into(),
            host: "203.0.113.7".into(),
            port: 9050,
            username: String::new(),
            password: String::new(),
        },
    ]
}
