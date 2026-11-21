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
        if let Some(parts) = &self.editor.ui.parts {
            return parts.clone();
        }
        let text = self.editor.file.text.clone();
        let code = cx.new(|cx| CodeEditor::new(&text, cx));
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
        let replacement = cx.new(|cx| InputState::new(window, cx).placeholder("Replace with"));
        let subscriptions = vec![
            cx.subscribe_in(&code, window, |app, code, event: &code::Event, _, cx| {
                match event {
                    code::Event::Edited => {
                        app.editor.file.text = code.read(cx).text().to_owned();
                    }
                    code::Event::Moved(at) => {
                        app.editor.fold.follow_cursor(&app.editor.file.text, *at);
                        app.push_shown(code, cx);
                    }
                    code::Event::Fold(line) => {
                        app.editor.fold.toggle(*line);
                        app.push_shown(code, cx);
                    }
                }
                cx.notify();
            }),
            cx.subscribe_in(&query, window, |app, input, event: &InputEvent, _, cx| {
                match event {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        if value != app.editor.find.query {
                            app.editor.find.query = value;
                            app.editor.find.current = 0;
                            app.editor.find.jump = true;
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
                        app.editor.find.replacement = input.read(cx).value().to_string();
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
        self.editor.ui.parts = Some(parts.clone());
        self.editor.ui.subscriptions = subscriptions;
        parts
    }

    fn push_shown(&mut self, code: &Entity<CodeEditor>, cx: &mut Context<Self>) {
        self.editor
            .fold
            .sync(&self.editor.file.path, &self.editor.file.text);
        let view = self.editor.fold.view(&self.editor.file.text);
        let rows = view
            .visible
            .iter()
            .map(|&line| Row {
                line,
                fold: if !self.editor.fold.is_foldable(line) {
                    Fold::None
                } else if self.editor.fold.is_folded(line) {
                    Fold::Folded
                } else {
                    Fold::Open
                },
            })
            .collect();
        let shown = Shown {
            rows,
            headers: view.headers,
            lines: self.editor.fold.line_count(),
            longest: self.editor.fold.longest_line(),
        };
        code.update(cx, |code, cx| code.set_shown(shown, cx));
    }

    fn sync_editor(&mut self, parts: &Parts, window: &mut Window, cx: &mut Context<Self>) -> usize {
        let text = self.editor.file.text.clone();
        if self.editor.ui.path.as_deref() != Some(self.editor.file.path.as_str()) {
            self.editor.ui.path = Some(self.editor.file.path.clone());
            parts.code.update(cx, |code, cx| code.load(&text, cx));
        } else {
            parts.code.update(cx, |code, cx| code.set_text(&text, cx));
        }
        let matches = if self.editor.find.open {
            find_matches(
                &text,
                &self.editor.find.query,
                self.editor.find.case_sensitive,
            )
        } else {
            Vec::new()
        };
        let count = matches.len();
        self.editor.find.current = self.editor.find.current.min(count.saturating_sub(1));
        let jump = self.editor.find.open && std::mem::take(&mut self.editor.find.jump);
        if jump && let Some(&at) = matches.get(self.editor.find.current) {
            self.editor.fold.reveal(text[..at].matches('\n').count());
        }
        self.push_shown(&parts.code, cx);
        let query_len = self.editor.find.query.len();
        let ranges = matches.iter().map(|&at| at..at + query_len).collect();
        let current = (count > 0).then_some(self.editor.find.current);
        parts
            .code
            .update(cx, |code, cx| code.set_matches(ranges, current, jump, cx));
        for (input, value) in [
            (&parts.query, &self.editor.find.query),
            (&parts.replacement, &self.editor.find.replacement),
        ] {
            if input.read(cx).value().as_ref() != value.as_str() {
                let value = value.clone();
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            }
        }
        if self.editor.find.open && std::mem::take(&mut self.editor.find.focus) {
            parts.query.update(cx, |input, cx| input.focus(window, cx));
        }
        count
    }

    fn editor_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let command = keystroke.modifiers.secondary(cx) && !keystroke.modifiers.alt;
        let key = keystroke.key.as_str();
        if command && (key == "f" || key == "r") {
            self.editor.find.open = true;
            self.editor.find.replace |= key == "r";
            self.editor.find.focus = true;
            cx.stop_propagation();
            cx.notify();
        } else if key == "escape" && self.editor.find.open {
            self.editor.find.open = false;
            cx.notify();
        }
    }

    fn run_editor_file(&mut self, pid: u32) {
        match self.editor.file.changed_on_disk() {
            Some(disk) => {
                self.reload_prompt = Some(ReloadPrompt {
                    disk,
                    restart: None,
                    editor: Some(pid),
                })
            }
            None => self.run_editor_on(pid, self.editor.file.clone()),
        }
    }

    fn run_editor_on(&mut self, pid: u32, file: ScriptFile) {
        match self.free_client(pid).cloned() {
            Some(target) => self.run_file(file, Some(&target), None, None),
            None => self.error(format!(
                "Couldn't run {}: client {pid} needs the agent loaded and no script running.",
                file.name()
            )),
        }
    }

    fn find_step(&mut self, back: bool) {
        let count = find_matches(
            &self.editor.file.text,
            &self.editor.find.query,
            self.editor.find.case_sensitive,
        )
        .len();
        if count == 0 {
            return;
        }
        let current = self.editor.find.current.min(count - 1);
        self.editor.find.current = if back {
            (current + count - 1) % count
        } else {
            (current + 1) % count
        };
        self.editor.find.jump = true;
    }

    fn replace_current(&mut self) {
        let matches = find_matches(
            &self.editor.file.text,
            &self.editor.find.query,
            self.editor.find.case_sensitive,
        );
        let current = self
            .editor
            .find
            .current
            .min(matches.len().saturating_sub(1));
        let Some(&at) = matches.get(current) else {
            return;
        };
        let replacement = self.editor.find.replacement.clone();
        let text = &mut self.editor.file.text;
        text.replace_range(at..at + self.editor.find.query.len(), &replacement);
        let after = at + replacement.len();
        let next = find_matches(
            text,
            &self.editor.find.query,
            self.editor.find.case_sensitive,
        );
        self.editor.find.current = next.iter().position(|&start| start >= after).unwrap_or(0);
        self.editor.find.jump = true;
    }

    fn replace_all(&mut self) {
        let matches = find_matches(
            &self.editor.file.text,
            &self.editor.find.query,
            self.editor.find.case_sensitive,
        );
        if matches.is_empty() {
            return;
        }
        let query_len = self.editor.find.query.len();
        let mut out = String::with_capacity(self.editor.file.text.len());
        let mut last = 0;
        for &at in &matches {
            out.push_str(&self.editor.file.text[last..at]);
            out.push_str(&self.editor.find.replacement);
            last = at + query_len;
        }
        out.push_str(&self.editor.file.text[last..]);
        self.editor.file.text = out;
        self.editor.find.current = 0;
    }

    fn render_run_button(&mut self, pid: u32, width: Pixels, cx: &mut Context<Self>) -> Div {
        let block = self.run_block(self.free_client(pid));
        let can_run = block.is_none() && !self.editor.file.text.trim().is_empty();
        widgets::button_group(false).child(
            widgets::icon_button(
                "editor-run",
                ICON_START,
                "Run",
                width,
                can_run,
                cx.listener(move |app, _: &ClickEvent, _, cx| {
                    app.run_editor_file(pid);
                    cx.notify();
                }),
            )
            .when_some(block, |this, why| this.tooltip(widgets::tooltip(why))),
        )
    }

    fn render_file_buttons(
        &mut self,
        width: Pixels,
        vertical: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let last = self.last_script.clone();
        let open_last = last.is_some();
        widgets::button_group(vertical)
            .child(widgets::icon_button(
                "editor-open",
                ICON_OPEN,
                "Open…",
                width,
                true,
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
                !self.editor.file.path.is_empty(),
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
                true,
                cx.listener(|app, _: &ClickEvent, _, cx| {
                    app.save_script_as();
                    cx.notify();
                }),
            ))
    }

    fn render_find(&mut self, parts: &Parts, count: usize, cx: &mut Context<Self>) -> Div {
        let status = if self.editor.find.query.is_empty() {
            String::new()
        } else if count == 0 {
            "No matches".into()
        } else {
            format!("{} of {count}", self.editor.find.current + 1)
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
                    .checked(self.editor.find.case_sensitive)
                    .on_click(cx.listener(|app, checked: &bool, _, cx| {
                        app.editor.find.case_sensitive = *checked;
                        app.editor.find.current = 0;
                        app.editor.find.jump = true;
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
                        app.editor.find.open = false;
                        cx.notify();
                    }),
                )
                .tooltip(widgets::tooltip("Escape")),
            );
        let replace = self.editor.find.replace.then(|| {
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
        owner: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.show_editor(owner);
        let parts = self.editor_parts(window, cx);
        if window
            .focused(cx)
            .is_none_or(|focused| focused == self.chrome.focus)
        {
            window.focus(&parts.page, cx);
        }
        let count = self.sync_editor(&parts, window, cx);
        let width = widgets::icon_button_width(
            window,
            &[
                (ICON_START, "Run"),
                (ICON_OPEN, "Open…"),
                (ICON_OPEN_LAST, "Open last"),
                (ICON_SAVE, "Save"),
                (ICON_SAVE_AS, "Save as…"),
            ],
        );
        let available = page_width(window);
        let run_group = if owner.is_some() {
            group_width(1, width)
        } else {
            px(0.0)
        };
        let file_group = group_width(4, width);
        let stack_groups = run_group + theme::ITEM_GAP + GROUP_GAP + file_group > available;
        let stack_buttons = file_group > available;
        let run_button = owner.map(|pid| self.render_run_button(pid, width, cx));
        let file_buttons = self.render_file_buttons(width, stack_buttons, cx);
        let buttons = fit_row(!stack_groups)
            .flex_none()
            .when(stack_groups, |row| row.items_start())
            .children(run_button)
            .child(file_buttons.map(|group| match (owner, stack_groups) {
                (None, _) => group,
                (Some(_), true) => group.mt(GROUP_GAP),
                (Some(_), false) => group.ml(GROUP_GAP),
            }));
        let heading = match owner {
            Some(pid) => format!("Client {pid}: editor"),
            None => "Editor".to_owned(),
        };
        let label = div()
            .id("editor-file")
            .flex_none()
            .child(self.editor.file.name())
            .when(!self.editor.file.path.is_empty(), |this| {
                this.tooltip(widgets::tooltip(self.editor.file.path.clone()))
            });
        let find = self
            .editor
            .find
            .open
            .then(|| self.render_find(&parts, count, cx));
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
            .child(widgets::heading(heading).flex_none())
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
        let Some(ReloadPrompt {
            disk,
            restart,
            editor,
        }) = self.reload_prompt.take()
        else {
            return;
        };
        let Some(reload) = choice else {
            return;
        };
        self.show_editor(editor);
        let file = match restart {
            Some(id) => match self.run_by_id(id).and_then(|r| r.file.clone()) {
                Some(file) => file,
                None => return,
            },
            None => self.editor.file.clone(),
        };
        let mut file = file;
        file.disk = Some(disk.clone());
        if reload {
            file.text = disk;
        }
        match restart {
            Some(id) => self.rerun_file(id, file),
            None => {
                self.editor.file = file.clone();
                if let Some(pid) = editor {
                    self.run_editor_on(pid, file);
                }
            }
        }
    }

    pub(super) fn render_reload_prompt(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (restart, editor) = self
            .reload_prompt
            .as_ref()
            .map(|prompt| (prompt.restart, prompt.editor))?;
        let file = match restart {
            Some(id) => match self.run_by_id(id).and_then(|r| r.file.clone()) {
                Some(file) => file,
                None => {
                    self.reload_prompt = None;
                    return None;
                }
            },
            None if self.editor.owner == editor => self.editor.file.clone(),
            None => match self.editors.get(&editor) {
                Some(parked) => parked.file.clone(),
                None => {
                    self.reload_prompt = None;
                    return None;
                }
            },
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
    super::clients::populated(app);
    app.page = Page::Editor(Some(4120));
    app.show_editor(Some(4120));
    app.editor.file = ScriptFile::default();
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    vec![
        ("editor", example),
        ("editor-global", |app| {
            app.page = Page::Editor(None);
            app.editor.file = ScriptFile::default();
        }),
        ("editor-folded", |app| {
            example(app);
            app.editor
                .fold
                .sync(&app.editor.file.path, &app.editor.file.text);
            app.editor.fold.toggle(3);
            app.editor.fold.toggle(19);
        }),
        ("editor-find", |app| {
            example(app);
            app.editor.find.open = true;
            app.editor.find.replace = true;
            app.editor.find.query = "state".into();
            app.editor.find.current = 2;
            app.editor.find.jump = true;
        }),
        ("editor-reload", |app| {
            example(app);
            app.editor.file.disk = Some(String::new());
            app.reload_prompt = Some(ReloadPrompt {
                disk: String::new(),
                restart: None,
                editor: Some(4120),
            });
        }),
    ]
}
