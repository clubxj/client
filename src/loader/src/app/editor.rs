use gpui_kit::component::Sizable;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{Entity, Subscription};

use super::*;
use crate::ui::{theme, widgets};

mod code;

use code::{CodeEditor, Fold, Row, Shown};

const GROUP_GAP: Pixels = px(8.0);
const FIND_WIDTH: Pixels = px(200.0);
const PROMPT_WIDTH: Pixels = px(420.0);
const PROMPT_MARGIN: Pixels = px(6.0);

#[derive(Clone)]
struct Parts {
    page: FocusHandle,
    code: Entity<CodeEditor>,
    query: Entity<InputState>,
    replacement: Entity<InputState>,
}

#[derive(Default)]
pub(super) struct Ui {
    parts: Option<Parts>,
    path: Option<String>,
    subscriptions: Vec<Subscription>,
}

fn group_width(buttons: usize, width: Pixels) -> Pixels {
    (width + px(1.0)) * buttons as f32 + px(1.0)
}

fn page_width(window: &Window) -> Pixels {
    let width = window.viewport_size().width;
    let margin = (width * 0.02).clamp(px(8.0), px(16.0));
    let side = if width < px(SIDEBAR_MIN_WINDOW) {
        px(SIDEBAR_HANDLE_WIDTH)
    } else {
        px(SIDEBAR_WIDTH)
    };
    width - side - margin * 2.0
}

impl LoaderApp {
    fn editor_parts(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Parts {
        if let Some(parts) = &self.editor_ui.parts {
            return parts.clone();
        }
        let text = self.script_file.text.clone();
        let code = cx.new(|cx| CodeEditor::new(&text, cx));
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
        let replacement = cx.new(|cx| InputState::new(window, cx).placeholder("Replace with"));
        let subscriptions = vec![
            cx.subscribe_in(&code, window, |app, code, event: &code::Event, _, cx| {
                match event {
                    code::Event::Edited => {
                        app.script_file.text = code.read(cx).text().to_owned();
                    }
                    code::Event::Moved(at) => {
                        app.fold.follow_cursor(&app.script_file.text, *at);
                        app.push_shown(code, cx);
                    }
                    code::Event::Fold(line) => {
                        app.fold.toggle(*line);
                        app.push_shown(code, cx);
                    }
                }
                cx.notify();
            }),
            cx.subscribe_in(&query, window, |app, input, event: &InputEvent, _, cx| {
                match event {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        if value != app.find.query {
                            app.find.query = value;
                            app.find.current = 0;
                            app.find.jump = true;
                        }
                    }
                    InputEvent::PressEnter { shift, .. } => app.find_step(*shift),
                    _ => return,
                }
                cx.notify();
            }),
            cx.subscribe_in(
                &replacement,
                window,
                |app, input, event: &InputEvent, _, cx| {
                    if let InputEvent::Change = event {
                        app.find.replacement = input.read(cx).value().to_string();
                        cx.notify();
                    }
                },
            ),
        ];
        let parts = Parts {
            page: cx.focus_handle(),
            code,
            query,
            replacement,
        };
        self.editor_ui.parts = Some(parts.clone());
        self.editor_ui.subscriptions = subscriptions;
        parts
    }

    fn push_shown(&mut self, code: &Entity<CodeEditor>, cx: &mut Context<Self>) {
        self.fold
            .sync(&self.script_file.path, &self.script_file.text);
        let view = self.fold.view(&self.script_file.text);
        let rows = view
            .visible
            .iter()
            .map(|&line| Row {
                line,
                fold: if !self.fold.is_foldable(line) {
                    Fold::None
                } else if self.fold.is_folded(line) {
                    Fold::Folded
                } else {
                    Fold::Open
                },
            })
            .collect();
        let shown = Shown {
            rows,
            headers: view.headers,
            lines: self.fold.line_count(),
            longest: self.fold.longest_line(),
        };
        code.update(cx, |code, cx| code.set_shown(shown, cx));
    }

    fn sync_editor(&mut self, parts: &Parts, window: &mut Window, cx: &mut Context<Self>) -> usize {
        let text = self.script_file.text.clone();
        if self.editor_ui.path.as_deref() != Some(self.script_file.path.as_str()) {
            self.editor_ui.path = Some(self.script_file.path.clone());
            parts.code.update(cx, |code, cx| code.load(&text, cx));
        } else {
            parts.code.update(cx, |code, cx| code.set_text(&text, cx));
        }
        let matches = if self.find.open {
            find_matches(&text, &self.find.query, self.find.case_sensitive)
        } else {
            Vec::new()
        };
        let count = matches.len();
        self.find.current = self.find.current.min(count.saturating_sub(1));
        let jump = self.find.open && std::mem::take(&mut self.find.jump);
        if jump && let Some(&at) = matches.get(self.find.current) {
            self.fold.reveal(text[..at].matches('\n').count());
        }
        self.push_shown(&parts.code, cx);
        let query_len = self.find.query.len();
        let ranges = matches.iter().map(|&at| at..at + query_len).collect();
        let current = (count > 0).then_some(self.find.current);
        parts
            .code
            .update(cx, |code, cx| code.set_matches(ranges, current, jump, cx));
        for (input, value) in [
            (&parts.query, &self.find.query),
            (&parts.replacement, &self.find.replacement),
        ] {
            if input.read(cx).value().as_ref() != value.as_str() {
                let value = value.clone();
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            }
        }
        if self.find.open && std::mem::take(&mut self.find.focus) {
            parts.query.update(cx, |input, cx| input.focus(window, cx));
        }
        count
    }

    fn editor_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let command = keystroke.modifiers.secondary(cx) && !keystroke.modifiers.alt;
        let key = keystroke.key.as_str();
        if command && (key == "f" || key == "r") {
            self.find.open = true;
            self.find.replace |= key == "r";
            self.find.focus = true;
            cx.stop_propagation();
            cx.notify();
        } else if key == "escape" && self.find.open {
            self.find.open = false;
            cx.notify();
        }
    }

    fn run_editor_file(&mut self) {
        match self.script_file.changed_on_disk() {
            Some(disk) => {
                self.reload_prompt = Some(ReloadPrompt {
                    disk,
                    restart: None,
                })
            }
            None => {
                let target = self.run_target_client();
                self.run_file(self.script_file.clone(), target.as_ref(), None, None);
            }
        }
    }

    fn find_step(&mut self, back: bool) {
        let count = find_matches(
            &self.script_file.text,
            &self.find.query,
            self.find.case_sensitive,
        )
        .len();
        if count == 0 {
            return;
        }
        let current = self.find.current.min(count - 1);
        self.find.current = if back {
            (current + count - 1) % count
        } else {
            (current + 1) % count
        };
        self.find.jump = true;
    }

    fn replace_current(&mut self) {
        let matches = find_matches(
            &self.script_file.text,
            &self.find.query,
            self.find.case_sensitive,
        );
        let current = self.find.current.min(matches.len().saturating_sub(1));
        let Some(&at) = matches.get(current) else {
            return;
        };
        let replacement = self.find.replacement.clone();
        let text = &mut self.script_file.text;
        text.replace_range(at..at + self.find.query.len(), &replacement);
        let after = at + replacement.len();
        let next = find_matches(text, &self.find.query, self.find.case_sensitive);
        self.find.current = next.iter().position(|&start| start >= after).unwrap_or(0);
        self.find.jump = true;
    }

    fn replace_all(&mut self) {
        let matches = find_matches(
            &self.script_file.text,
            &self.find.query,
            self.find.case_sensitive,
        );
        if matches.is_empty() {
            return;
        }
        let query_len = self.find.query.len();
        let mut out = String::with_capacity(self.script_file.text.len());
        let mut last = 0;
        for &at in &matches {
            out.push_str(&self.script_file.text[last..at]);
            out.push_str(&self.find.replacement);
            last = at + query_len;
        }
        out.push_str(&self.script_file.text[last..]);
        self.script_file.text = out;
        self.find.current = 0;
    }

    fn render_run_buttons(
        &mut self,
        running: bool,
        paused: bool,
        width: Pixels,
        vertical: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let can_run = !running && !self.script_file.text.trim().is_empty();
        let (pause_icon, pause_text) = run_pause_label(paused);
        widgets::button_group(vertical)
            .child(widgets::icon_button(
                "editor-run",
                ICON_START,
                "Run",
                width,
                can_run,
                cx.listener(|app, _: &ClickEvent, _, cx| {
                    app.run_editor_file();
                    cx.notify();
                }),
            ))
            .child(widgets::icon_button(
                "editor-pause",
                pause_icon,
                pause_text,
                width,
                running,
                cx.listener(|app, _: &ClickEvent, _, cx| {
                    if let Some(id) = app.editor_run_mut().map(|r| r.id) {
                        app.toggle_pause_of(id);
                    }
                    cx.notify();
                }),
            ))
            .child(widgets::icon_button(
                "editor-stop",
                ICON_STOP,
                "Stop",
                width,
                running,
                cx.listener(|app, _: &ClickEvent, _, cx| {
                    if let Some(run) = app.editor_run_mut() {
                        run.stop();
                    }
                    cx.notify();
                }),
            ))
    }

    fn render_file_buttons(
        &mut self,
        running: bool,
        width: Pixels,
        vertical: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let last = self.last_script.clone();
        let open_last = !running && last.is_some();
        widgets::button_group(vertical)
            .child(widgets::icon_button(
                "editor-open",
                ICON_OPEN,
                "Open…",
                width,
                !running,
                cx.listener(|app, _: &ClickEvent, _, cx| {
                    app.open_script();
                    cx.notify();
                }),
            ))
            .child(
                widgets::icon_button(
                    "editor-open-last",
                    ICON_OPEN_LAST,
                    "Open last",
                    width,
                    open_last,
                    cx.listener(|app, _: &ClickEvent, _, cx| {
                        if let Some(path) = app.last_script.clone() {
                            app.open_path(std::path::Path::new(&path));
                        }
                        cx.notify();
                    }),
                )
                .when_some(last.filter(|_| open_last), |this, path| {
                    this.tooltip(widgets::tooltip(path))
                }),
            )
            .child(widgets::icon_button(
                "editor-save",
                ICON_SAVE,
                "Save",
                width,
                !running && !self.script_file.path.is_empty(),
                cx.listener(|app, _: &ClickEvent, _, cx| {
                    app.save_script();
                    cx.notify();
                }),
            ))
            .child(widgets::icon_button(
                "editor-save-as",
                ICON_SAVE_AS,
                "Save as…",
                width,
                !running,
                cx.listener(|app, _: &ClickEvent, _, cx| {
                    app.save_script_as();
                    cx.notify();
                }),
            ))
    }

    fn render_find(&mut self, parts: &Parts, count: usize, cx: &mut Context<Self>) -> Div {
        let status = if self.find.query.is_empty() {
            String::new()
        } else if count == 0 {
            "No matches".into()
        } else {
            format!("{} of {count}", self.find.current + 1)
        };
        let any = count > 0;
        let find = div()
            .flex()
            .items_center()
            .gap(theme::ITEM_GAP)
            .child(Input::new(&parts.query).small().w(FIND_WIDTH))
            .child(
                widgets::button(
                    "find-previous",
                    "Previous",
                    any,
                    cx.listener(|app, _: &ClickEvent, _, cx| {
                        app.find_step(true);
                        cx.notify();
                    }),
                )
                .when(any, |this| this.tooltip(widgets::tooltip("Shift+Enter"))),
            )
            .child(
                widgets::button(
                    "find-next",
                    "Next",
                    any,
                    cx.listener(|app, _: &ClickEvent, _, cx| {
                        app.find_step(false);
                        cx.notify();
                    }),
                )
                .when(any, |this| this.tooltip(widgets::tooltip("Enter"))),
            )
            .child(div().flex_none().whitespace_nowrap().child(status))
            .child(
                Checkbox::new("find-case")
                    .label("Match case")
                    .checked(self.find.case_sensitive)
                    .on_click(cx.listener(|app, checked: &bool, _, cx| {
                        app.find.case_sensitive = *checked;
                        app.find.current = 0;
                        app.find.jump = true;
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .child(
                widgets::button(
                    "find-close",
                    "Close",
                    true,
                    cx.listener(|app, _: &ClickEvent, _, cx| {
                        app.find.open = false;
                        cx.notify();
                    }),
                )
                .tooltip(widgets::tooltip("Escape")),
            );
        let replace = self.find.replace.then(|| {
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(theme::ITEM_GAP)
                .child(Input::new(&parts.replacement).small().w(FIND_WIDTH))
                .child(widgets::button(
                    "find-replace",
                    "Replace",
                    any,
                    cx.listener(|app, _: &ClickEvent, _, cx| {
                        app.replace_current();
                        cx.notify();
                    }),
                ))
                .child(widgets::button(
                    "find-replace-all",
                    "Replace all",
                    any,
                    cx.listener(|app, _: &ClickEvent, _, cx| {
                        app.replace_all();
                        cx.notify();
                    }),
                ))
        });
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(theme::LINE_GAP)
            .pt(theme::LINE_GAP)
            .child(find)
            .children(replace)
    }

    pub(super) fn render_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let parts = self.editor_parts(window, cx);
        if window
            .focused(cx)
            .is_none_or(|focused| focused == self.chrome.focus)
        {
            window.focus(&parts.page, cx);
        }
        let count = self.sync_editor(&parts, window, cx);
        let (running, paused) = self
            .editor_run_mut()
            .map_or((false, false), |r| (true, r.is_paused()));
        let (picker, _) = self.render_run_target(cx);
        let width = widgets::icon_button_width(
            window,
            &[
                (ICON_START, "Run"),
                PAUSE_BUTTONS[0],
                PAUSE_BUTTONS[1],
                (ICON_STOP, "Stop"),
                (ICON_OPEN, "Open…"),
                (ICON_OPEN_LAST, "Open last"),
                (ICON_SAVE, "Save"),
                (ICON_SAVE_AS, "Save as…"),
            ],
        );
        let available = page_width(window);
        let run_group = group_width(3, width);
        let file_group = group_width(4, width);
        let stack_groups = run_group + theme::ITEM_GAP + GROUP_GAP + file_group > available;
        let stack_buttons = run_group.max(file_group) > available;
        let run_buttons = self.render_run_buttons(running, paused, width, stack_buttons, cx);
        let file_buttons = self.render_file_buttons(running, width, stack_buttons, cx);
        let buttons =
            fit_row(!stack_groups)
                .flex_none()
                .when(stack_groups, |row| row.items_start())
                .child(run_buttons)
                .child(file_buttons.map(|group| {
                    if stack_groups {
                        group.mt(GROUP_GAP)
                    } else {
                        group.ml(GROUP_GAP)
                    }
                }));
        let status = match (running, paused) {
            (true, true) => "paused",
            (true, false) => "running",
            (false, _) => "not running",
        };
        let label = div()
            .id("editor-file")
            .flex_none()
            .child(format!("{}, {status}", self.script_file.name()))
            .when(!self.script_file.path.is_empty(), |this| {
                this.tooltip(widgets::tooltip(self.script_file.path.clone()))
            });
        let find = self.find.open.then(|| self.render_find(&parts, count, cx));
        div()
            .id("editor-page")
            .track_focus(&parts.page)
            .flex()
            .flex_col()
            .size_full()
            .gap(theme::LINE_GAP)
            .capture_key_down(
                cx.listener(|app, event: &KeyDownEvent, _, cx| app.editor_key(event, cx)),
            )
            .child(picker.flex_none())
            .child(buttons)
            .child(label)
            .children(find)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .pt(theme::LINE_GAP)
                    .child(parts.code.clone()),
            )
            .into_any_element()
    }

    fn answer_reload(&mut self, choice: Option<bool>) {
        let Some(ReloadPrompt { disk, restart }) = self.reload_prompt.take() else {
            return;
        };
        let Some(reload) = choice else {
            return;
        };
        let file = match restart {
            Some(id) => match self.run_by_id(id).and_then(|r| r.file.clone()) {
                Some(file) => file,
                None => return,
            },
            None => self.script_file.clone(),
        };
        let mut file = file;
        file.disk = Some(disk.clone());
        if reload {
            file.text = disk;
        }
        match restart {
            Some(id) => self.rerun_file(id, file),
            None => {
                self.script_file = file.clone();
                let target = self.run_target_client();
                self.run_file(file, target.as_ref(), None, None);
            }
        }
    }

    pub(super) fn render_reload_prompt(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let restart = self.reload_prompt.as_ref()?.restart;
        let file = match restart {
            Some(id) => match self.run_by_id(id).and_then(|r| r.file.clone()) {
                Some(file) => file,
                None => {
                    self.reload_prompt = None;
                    return None;
                }
            },
            None => self.script_file.clone(),
        };
        let name = file.name();
        let unsaved =
            restart.is_none() && file.disk.as_ref().is_some_and(|known| known != &file.text);
        let message = if restart.is_some() {
            format!("{name} changed on disk since it last ran. Reload it before restarting?")
        } else {
            format!("{name} changed on disk since it was loaded. Reload it before running?")
        };
        let answer = |choice: Option<bool>| {
            cx.listener(move |app: &mut LoaderApp, _: &ClickEvent, _, cx| {
                app.answer_reload(choice);
                cx.notify();
            })
        };
        let buttons = div()
            .flex()
            .flex_wrap()
            .gap(theme::ITEM_GAP)
            .child(widgets::button(
                "reload-run",
                "Reload and run",
                true,
                answer(Some(true)),
            ))
            .child(widgets::button(
                "reload-keep",
                if restart.is_some() {
                    "Run previous version"
                } else {
                    "Run editor version"
                },
                true,
                answer(Some(false)),
            ))
            .child(widgets::button(
                "reload-cancel",
                "Cancel",
                true,
                answer(None),
            ));
        let card = div()
            .id("reload-prompt")
            .occlude()
            .flex()
            .flex_col()
            .max_w(PROMPT_WIDTH)
            .bg(theme::BG_FLOAT)
            .border_1()
            .border_color(theme::BORDER)
            .child(
                div()
                    .flex()
                    .justify_center()
                    .px(PROMPT_MARGIN)
                    .py(theme::LINE_GAP)
                    .child(widgets::heading("Script changed on disk")),
            )
            .child(divider())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(theme::LINE_GAP)
                    .p(PROMPT_MARGIN)
                    .child(message)
                    .when(unsaved, |this| {
                        this.child(
                            div()
                                .text_color(theme::YELLOW)
                                .child("Reloading discards your unsaved edits."),
                        )
                    })
                    .child(widgets::separator())
                    .child(buttons),
            );
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(card)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
fn example(app: &mut LoaderApp) {
    app.tab = Tab::Scripts;
    app.script_file = ScriptFile::default();
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("editor", example),
        ("editor-folded", |app| {
            example(app);
            app.fold.sync(&app.script_file.path, &app.script_file.text);
            app.fold.toggle(3);
            app.fold.toggle(19);
        }),
        ("editor-find", |app| {
            example(app);
            app.find.open = true;
            app.find.replace = true;
            app.find.query = "state".into();
            app.find.current = 2;
            app.find.jump = true;
        }),
        ("editor-reload", |app| {
            example(app);
            app.script_file.disk = Some(String::new());
            app.reload_prompt = Some(ReloadPrompt {
                disk: String::new(),
                restart: None,
            });
        }),
    ]
}
