use gpui_kit::component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_kit::component::input::{Input, InputEvent, InputState, NumberInput};
use gpui_kit::component::{Disableable, Sizable, WindowExt};
use gpui_kit::{Entity, Focusable, Hsla, KeystrokeEvent, Subscription};

use super::*;
use crate::ui::{theme, widgets};

const COLUMN_GAP: Pixels = px(16.0);
const MIN_COLUMN: Pixels = px(48.0);
const ROW_INSET: Pixels = px(8.0);
const SWATCH: Pixels = px(20.0);
const DISPLAY_NAME_WIDTH: Pixels = px(140.0);
const CURSOR_SIZE_WIDTH: Pixels = px(100.0);
const CURSOR_SIZES: std::ops::RangeInclusive<u8> = 4..=64;
const DISPLAY_NAME_HINT: &str = "Shown instead of your display name, in this client only, and kept across logins until reset. Empty puts the real name back.";

pub(super) struct Ui {
    display_name: Entity<InputState>,
    cursor_color: Entity<ColorPickerState>,
    cursor_size: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

fn cursor_hsla(color: [u8; 3]) -> Hsla {
    Rgba {
        r: f32::from(color[0]) / 255.0,
        g: f32::from(color[1]) / 255.0,
        b: f32::from(color[2]) / 255.0,
        a: 1.0,
    }
    .into()
}

fn cursor_rgb(color: Hsla) -> [u8; 3] {
    let rgb = color.to_rgb();
    [rgb.r, rgb.g, rgb.b].map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn parse_cursor_size(text: &str) -> Option<u8> {
    text.trim()
        .parse()
        .ok()
        .filter(|size| CURSOR_SIZES.contains(size))
}

impl Ui {
    fn new(app: &LoaderApp, window: &mut Window, cx: &mut Context<LoaderApp>) -> Self {
        let display_name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Real name")
                .default_value(app.display_name.clone())
                .validate(|text, _| text.chars().count() <= injector::state::MAX_NAME_BYTES)
        });
        let cursor_color = cx.new(|cx| {
            ColorPickerState::new(window, cx).default_value(cursor_hsla(app.cursor.color))
        });
        let cursor_size = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(app.cursor.size.to_string())
                .step(1.0)
                .min(f64::from(*CURSOR_SIZES.start()))
                .max(f64::from(*CURSOR_SIZES.end()))
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &display_name,
                window,
                |app, input, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => {
                        app.display_name = input.read(cx).value().to_string();
                    }
                    InputEvent::PressEnter { .. } => {
                        window.focus(&app.chrome.focus, cx);
                        app.submit_display_name();
                        cx.notify();
                    }
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &cursor_color,
                window,
                |app, _, event: &ColorPickerEvent, _, cx| {
                    let ColorPickerEvent::Change(Some(color)) = event else {
                        return;
                    };
                    let color = cursor_rgb(*color);
                    if color != app.cursor.color {
                        app.cursor.color = color;
                        app.cursor_changed();
                        cx.notify();
                    }
                },
            ),
            cx.subscribe_in(
                &cursor_size,
                window,
                |app, input, event: &InputEvent, _, cx| {
                    if let InputEvent::Change = event
                        && let Some(size) = parse_cursor_size(&input.read(cx).value())
                        && size != app.cursor.size
                    {
                        app.cursor.size = size;
                        app.cursor_changed();
                        cx.notify();
                    }
                },
            ),
            cx.observe_keystrokes(|app, event: &KeystrokeEvent, window, cx| {
                if app.tab == Tab::Clients
                    && let Some(down) = step_key(event, window, cx)
                {
                    app.step_selected_client(down);
                    cx.notify();
                }
            }),
        ];
        Self {
            display_name,
            cursor_color,
            cursor_size,
            _subscriptions: subscriptions,
        }
    }

    fn sync(&self, app: &LoaderApp, window: &mut Window, cx: &mut App) {
        if !self.display_name.focus_handle(cx).is_focused(window)
            && self.display_name.read(cx).value().as_ref() != app.display_name
        {
            let text = app.display_name.clone();
            self.display_name
                .update(cx, |input, cx| input.set_value(text, window, cx));
        }
        let shown = self.cursor_color.read(cx).value().map(cursor_rgb);
        if shown != Some(app.cursor.color) {
            let color = cursor_hsla(app.cursor.color);
            self.cursor_color
                .update(cx, |picker, cx| picker.set_value(color, window, cx));
        }
        if !self.cursor_size.focus_handle(cx).is_focused(window)
            && parse_cursor_size(&self.cursor_size.read(cx).value()) != Some(app.cursor.size)
        {
            let text = app.cursor.size.to_string();
            self.cursor_size
                .update(cx, |input, cx| input.set_value(text, window, cx));
        }
    }
}

fn step_key(event: &KeystrokeEvent, window: &mut Window, cx: &mut App) -> Option<bool> {
    if event.action.is_some()
        || event.keystroke.modifiers.modified()
        || window.has_focused_input(cx)
    {
        return None;
    }
    match event.keystroke.key.as_str() {
        "down" => Some(true),
        "up" => Some(false),
        _ => None,
    }
}

fn status_of(client: &injector::Client, dll_version: Option<&str>) -> (String, Rgba, String) {
    if client.foreign_bot.is_some() {
        return (
            "Unsupported".to_owned(),
            theme::RED,
            "Being used by another platform".to_owned(),
        );
    }
    if let Some(module) = &client.loaded {
        let version = client.version.as_deref().unwrap_or("unknown version");
        let (text, color) = if dll_version.is_some_and(|v| v != version) {
            (format!("Loaded {version}, outdated"), theme::YELLOW)
        } else {
            (format!("Loaded {version}"), theme::GREEN)
        };
        let tip = format!("{text}\n{module}");
        return (text, color, tip);
    }
    let (text, color) = if client.is_64bit {
        ("Not loaded", theme::COMMENT)
    } else {
        ("32-bit, unsupported", theme::RED)
    };
    (text.to_owned(), color, text.to_owned())
}

impl LoaderApp {
    fn cursor_changed(&mut self) {
        if let Some(target) = self.selected_target().cloned() {
            self.set_cursor(target);
        }
    }

    fn can_set_display_name(&self) -> bool {
        self.busy.is_none()
            && self
                .selected_target()
                .is_some_and(|t| t.client.loaded.is_some())
    }

    fn submit_display_name(&mut self) {
        if !self.can_set_display_name() {
            return;
        }
        if let Some(target) = self.selected_target().cloned() {
            self.set_display_name(target);
        }
    }

    fn client_action(&mut self, action: ClientAction) {
        let Some(target) = self
            .selected_target()
            .filter(|t| t.client.foreign_bot.is_none())
            .cloned()
        else {
            return;
        };
        match action {
            ClientAction::Load => self.load(target),
            ClientAction::Unload => self.unload(target),
            ClientAction::Console => self.console(target),
        }
    }

    pub(super) fn step_selected_client(&mut self, down: bool) {
        let keys: Vec<String> = self
            .targets
            .iter()
            .filter(|t| t.client.foreign_bot.is_none())
            .map(Target::key)
            .collect();
        if keys.is_empty() {
            return;
        }
        let current = keys.iter().position(|k| Some(k) == self.selected.as_ref());
        let next = match (current, down) {
            (None, true) => 0,
            (None, false) => keys.len() - 1,
            (Some(i), true) => (i + 1).min(keys.len() - 1),
            (Some(i), false) => i.saturating_sub(1),
        };
        self.selected = Some(keys[next].clone());
        self.run_target.clone_from(&self.selected);
    }

    fn render_client_table(&self, window: &Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let show_prefix = !cfg!(windows);
        let bold_width =
            |text: &str| (widgets::text_width(window, text, theme::BODY, true) + px(1.0)).ceil();
        let pid_width = self
            .targets
            .iter()
            .map(|t| bold_width(&t.client.pid.to_string()))
            .fold(bold_width("PID"), Pixels::max);
        let title_width = self
            .targets
            .iter()
            .map(|t| bold_width(&t.client.title))
            .fold(bold_width("Title"), Pixels::max);
        let mut columns = vec![("Title", title_width)];
        if show_prefix {
            columns.push(("Prefix", px(CLIENT_PREFIX_WIDTH)));
        }
        columns.push(("Status", px(CLIENT_STATUS_WIDTH)));
        let width = |index: usize| columns[index].1;
        let pid_cell = || div().flex_none().w(pid_width);
        let flex_cell = |width: Pixels| {
            div()
                .flex_basis(width)
                .flex_shrink(1.0)
                .min_w(width.min(MIN_COLUMN))
        };
        let row = || {
            div()
                .flex()
                .items_center()
                .gap(COLUMN_GAP)
                .px(ROW_INSET)
                .py(px(2.0))
        };
        let header = columns.iter().fold(
            row()
                .text_color(theme::COMMENT)
                .child(pid_cell().child("PID")),
            |header, (title, width)| header.child(flex_cell(*width).child(*title)),
        );
        let rows = self.targets.iter().enumerate().map(|(index, target)| {
            let client = &target.client;
            let enabled = client.foreign_bot.is_none();
            let key = target.key();
            let active = enabled && self.selected.as_ref() == Some(&key);
            let text_color = if active {
                theme::GREEN
            } else if enabled {
                theme::FG
            } else {
                theme::COMMENT
            };
            let (status, color, tip) = status_of(client, self.dll_version.as_deref());
            let bold_status = active || !enabled;
            let prefix = target.location.label().unwrap_or_default();
            let title = div()
                .id("title")
                .child(client.title.clone())
                .tooltip(widgets::tooltip(client.exe.clone()));
            let prefix = show_prefix.then(|| {
                flex_cell(width(1)).child(
                    div()
                        .id("prefix")
                        .truncate()
                        .child(prefix.clone())
                        .tooltip(widgets::tooltip(prefix)),
                )
            });
            let status = div()
                .id("status")
                .text_color(color)
                .when(bold_status, |this| this.font_weight(FontWeight::BOLD))
                .child(status)
                .tooltip(widgets::tooltip(tip));
            row()
                .id(("client", index))
                .max_w_full()
                .rounded(px(3.0))
                .text_color(text_color)
                .when(active, |this| this.font_weight(FontWeight::BOLD))
                .when(index % 2 == 1, |this| this.bg(theme::BG_FLOAT))
                .when(enabled, |this| {
                    this.cursor_pointer()
                        .hover(|style| style.bg(theme::SEL))
                        .on_click(cx.listener(move |app, _: &ClickEvent, _, cx| {
                            app.selected = Some(key.clone());
                            app.run_target.clone_from(&app.selected);
                            cx.notify();
                        }))
                })
                .child(pid_cell().child(client.pid.to_string()))
                .child(flex_cell(width(0)).child(title))
                .children(prefix)
                .child(flex_cell(width(columns.len() - 1)).child(status))
        });
        div()
            .id("clients")
            .flex_shrink(1.0)
            .min_h(px(60.0))
            .mx(-ROW_INSET)
            .overflow_scroll()
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_start()
                    .child(header.max_w_full().mb(theme::LINE_GAP))
                    .children(rows),
            )
    }

    fn render_client_buttons(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let client = self
            .selected_target()
            .filter(|t| t.client.foreign_bot.is_none())
            .map(|t| &t.client);
        let idle = self.busy.is_none();
        let loaded = client.is_some_and(|c| c.loaded.is_some());
        let can_load = idle
            && !self.settings.dll.is_empty()
            && client.is_some_and(|c| {
                c.is_64bit && (c.loaded.is_none() || c.version != self.dll_version)
            });
        let can_unload = idle && loaded;
        let (load_icon, load_label) = if loaded {
            (ICON_UPDATE, "Reload")
        } else {
            (ICON_LOAD, "Load")
        };
        let width = widgets::icon_button_width(
            window,
            &[
                (ICON_LOAD, "Load"),
                (ICON_UPDATE, "Reload"),
                (ICON_UNLOAD, "Unload"),
                (ICON_CONSOLE, "Console"),
            ],
        );
        let button = |id: &'static str,
                      icon: &'static str,
                      label: &'static str,
                      enabled: bool,
                      action: ClientAction,
                      cx: &mut Context<Self>| {
            widgets::icon_button(
                id,
                icon,
                label,
                width,
                enabled,
                cx.listener(move |app, _: &ClickEvent, _, cx| {
                    app.client_action(action);
                    cx.notify();
                }),
            )
        };
        widgets::button_group(false)
            .child(
                button(
                    "load",
                    load_icon,
                    load_label,
                    can_load,
                    ClientAction::Load,
                    cx,
                )
                .tooltip(widgets::tooltip(
                    "Loads the DLL from the settings, unloading another build first.",
                )),
            )
            .child(button(
                "unload",
                ICON_UNLOAD,
                "Unload",
                can_unload,
                ClientAction::Unload,
                cx,
            ))
            .child(
                button(
                    "console",
                    ICON_CONSOLE,
                    "Console",
                    can_unload,
                    ClientAction::Console,
                    cx,
                )
                .tooltip(widgets::tooltip(
                    "Opens agent's console window in the client.",
                )),
            )
    }

    fn render_client_options(&self, ui: &Ui, window: &Window, cx: &mut Context<Self>) -> Div {
        let labels = ["Mouse cursor", "Display name"];
        let label_width = widgets::widest(window, &labels, theme::BODY).ceil();
        let option = |label: &'static str, control: Div| {
            div()
                .flex()
                .items_center()
                .gap(COLUMN_GAP)
                .child(div().flex_none().w(label_width).child(label))
                .child(control)
        };
        let app = cx.entity().downgrade();
        let styles = CURSOR_STYLES
            .into_iter()
            .map(|style| {
                let app = app.clone();
                widgets::Choice::new(
                    cursor_style_label(style),
                    style == self.cursor.style,
                    move |_, cx| {
                        app.update(cx, |app, cx| {
                            if app.cursor.style != style {
                                app.cursor.style = style;
                                app.cursor_changed();
                                cx.notify();
                            }
                        })
                        .ok();
                    },
                )
            })
            .collect();
        let cursor_on = self.cursor.style != CursorStyle::Off;
        let color = if cursor_on {
            ColorPicker::new(&ui.cursor_color)
                .small()
                .into_any_element()
        } else {
            div()
                .flex_none()
                .size(SWATCH)
                .border_1()
                .border_color(theme::BORDER)
                .bg(cursor_hsla(self.cursor.color))
                .opacity(0.5)
                .into_any_element()
        };
        let cursor = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(theme::ITEM_GAP)
            .child(widgets::combo(
                "cursor-style",
                cursor_style_label(self.cursor.style),
                styles,
            ))
            .child(color)
            .child(
                NumberInput::new(&ui.cursor_size)
                    .small()
                    .w(CURSOR_SIZE_WIDTH)
                    .suffix("px")
                    .disabled(!cursor_on),
            );
        let can_set = self.can_set_display_name();
        let label = if self.display_name.trim().is_empty() {
            "Reset"
        } else {
            "Set"
        };
        let name = div()
            .flex()
            .items_center()
            .gap(theme::ITEM_GAP)
            .child(
                div()
                    .id("display-name")
                    .w(DISPLAY_NAME_WIDTH)
                    .child(Input::new(&ui.display_name).small().disabled(!can_set))
                    .tooltip(widgets::tooltip(DISPLAY_NAME_HINT)),
            )
            .child(
                widgets::button(
                    "set-display-name",
                    label,
                    can_set,
                    cx.listener(|app, _: &ClickEvent, _, cx| {
                        app.submit_display_name();
                        cx.notify();
                    }),
                )
                .tooltip(widgets::tooltip(DISPLAY_NAME_HINT)),
            );
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(option(labels[0], cursor))
            .child(option(labels[1], name))
    }

    pub(super) fn render_clients(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ui = match self.clients_ui.take() {
            Some(ui) => ui,
            None => Ui::new(self, window, cx),
        };
        ui.sync(self, window, cx);
        let idle = self.busy.is_none();
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .child(widgets::heading("Clients"))
            .child(widgets::button(
                "refresh",
                "Refresh",
                idle,
                cx.listener(|app, _: &ClickEvent, _, cx| {
                    app.scan();
                    cx.notify();
                }),
            ));
        let table = if self.targets.is_empty() {
            idle.then(|| {
                widgets::weak("No game clients found.")
                    .flex_none()
                    .into_any_element()
            })
        } else {
            Some(self.render_client_table(window, cx).into_any_element())
        };
        let options = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(theme::LINE_GAP)
            .mt(theme::ITEM_GAP)
            .child(widgets::heading("Client options"))
            .child(div().flex().child(self.render_client_buttons(window, cx)))
            .child(
                div()
                    .mt(px(6.0))
                    .child(self.render_client_options(&ui, window, cx)),
            );
        self.clients_ui = Some(ui);
        div()
            .flex_1()
            .min_h_0()
            .relative()
            .flex()
            .flex_col()
            .gap(theme::LINE_GAP)
            .child(crate::logo::background())
            .child(header)
            .children(table)
            .child(options)
            .into_any_element()
    }
}

#[cfg(test)]
fn target(pid: u32, title: &str, loaded: Option<&str>, foreign: bool) -> Target {
    let location = if cfg!(windows) {
        serde_json::Value::Null
    } else {
        serde_json::json!({
            "prefix": "/home/tester/.local/share/launcher/game_wine_prefix",
            "wine": "/usr/bin/wine",
            "env": [],
        })
    };
    serde_json::from_value(serde_json::json!({
        "client": {
            "pid": pid,
            "exe": format!("C:\\Program Files\\Game\\{title}.exe"),
            "title": title,
            "is_64bit": true,
            "loaded": loaded.map(|_| "Z:\\home\\tester\\agent.dll"),
            "version": loaded,
            "foreign_bot": foreign.then_some("other"),
        },
        "location": location,
    }))
    .expect("a fake target")
}

#[cfg(test)]
fn populated(app: &mut LoaderApp) {
    app.tab = Tab::Clients;
    app.dll_version = Some("2026.11.20-abc1234".into());
    app.targets = vec![
        target(4120, "Game client", Some("2026.11.20-abc1234"), false),
        target(5388, "Game client - second account", None, false),
        target(6012, "Game client", Some("2026.10.02-0f0e0d0"), false),
        target(7301, "Game client", None, true),
    ];
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("clients-populated", populated),
        ("clients-selected", |app| {
            populated(app);
            app.selected = Some(app.targets[0].key());
            app.display_name = "Alt name".into();
        }),
        ("clients-selected-unloaded", |app| {
            populated(app);
            app.selected = Some(app.targets[1].key());
            app.cursor.style = CursorStyle::Off;
        }),
        ("clients-busy", |app| {
            populated(app);
            app.selected = Some(app.targets[0].key());
            app.busy = Some("Loading…");
        }),
    ]
}

#[cfg(test)]
mod tests {
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{Bounds, TestAppContext};

    use super::*;

    struct Probe {
        input: Entity<InputState>,
        focus: FocusHandle,
        steps: Vec<bool>,
        _keys: Subscription,
    }

    impl Render for Probe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .track_focus(&self.focus)
                .child(Input::new(&self.input))
        }
    }

    #[gpui_kit::test]
    fn arrow_keys_step_only_while_no_text_input_has_focus(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (handle, probe) = cx
            .update(|cx| {
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds {
                            origin: Default::default(),
                            size: size(px(400.0), px(200.0)),
                        })),
                        ..Default::default()
                    },
                    cx,
                    |window, cx| {
                        cx.new(|cx| {
                            let focus = cx.focus_handle();
                            window.focus(&focus, cx);
                            Probe {
                                input: cx.new(|cx| InputState::new(window, cx)),
                                focus,
                                steps: Vec::new(),
                                _keys: cx.observe_keystrokes(
                                    |probe: &mut Probe, event, window, cx| {
                                        if let Some(down) = step_key(event, window, cx) {
                                            probe.steps.push(down);
                                        }
                                    },
                                ),
                            }
                        })
                    },
                )
            })
            .unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press("down", cx);
            window.press("up", cx);
            window.press("shift-down", cx);
            window.press("left", cx);
        })
        .unwrap();
        assert_eq!(
            probe.read_with(cx, |probe, _| probe.steps.clone()),
            [true, false]
        );
        cx.update_window(handle, |_, window, cx| {
            let input = probe.read(cx).input.clone();
            input.update(cx, |input, cx| input.focus(window, cx));
            window.render_frame(cx);
            window.press("down", cx);
            window.press("up", cx);
        })
        .unwrap();
        assert_eq!(
            probe.read_with(cx, |probe, _| probe.steps.clone()),
            [true, false]
        );
    }
}
