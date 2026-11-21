use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::{
    Bounds, DispatchPhase, HighlightStyle, IndentAdjustment, LineFragment, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Point, SharedString, StyledText, TextRun,
    UniformListScrollHandle, uniform_list,
};

use super::*;
use crate::ui::{theme, widgets};

const LINE_HEIGHT: Pixels = px(16.0);
const TEXT_MARGIN: Pixels = px(4.0);
const PANEL_MARGIN_X: Pixels = px(8.0);
const PANEL_MARGIN_Y: Pixels = px(2.0);
const SEPARATOR: Pixels = px(1.0);
const COLLAPSED_HEIGHT: Pixels = px(27.0);
const RESIZE_GRAB: Pixels = px(5.0);
const MIN_HEIGHT: Pixels = px(20.0);
const SELECTION_SCROLL_MAX: f32 = 2400.0;
const SELECTION_SCROLL_RAMP: f32 = 200.0;

fn selection_scroll_speed(pointer: f32, visible: Range<f32>, window: Range<f32>) -> f32 {
    let past = pointer - pointer.clamp(visible.start, visible.end);
    let room = if past < 0.0 {
        visible.start - window.start
    } else {
        window.end - visible.end
    };
    let share = (past.abs() / room.clamp(1.0, SELECTION_SCROLL_RAMP)).min(1.0);
    past.signum() * share * SELECTION_SCROLL_MAX
}

fn tone_color(tone: Tone) -> gpui_kit::Rgba {
    match tone {
        Tone::Normal => theme::FG_DARK,
        Tone::Warning => theme::YELLOW,
        Tone::Error => theme::RED,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) struct Spot {
    entry: usize,
    byte: usize,
}

impl Log {
    fn entry(&self, entry: usize) -> Option<&LogLine> {
        self.view.get(entry).map(|&index| &self.lines[index])
    }

    fn appended(&self, revision: u64, entries: usize) -> Option<Range<usize>> {
        let added = self.view.len().checked_sub(entries)?;
        (self.revision.checked_sub(revision)? == added as u64).then_some(entries..self.view.len())
    }

    fn selected_text(&self, from: Spot, to: Spot) -> String {
        (from.entry..=to.entry)
            .filter_map(|entry| {
                let text = &self.entry(entry)?.text;
                let end = if entry == to.entry {
                    to.byte.min(text.len())
                } else {
                    text.len()
                };
                let start = if entry == from.entry {
                    from.byte.min(end)
                } else {
                    0
                };
                text.get(start..end)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn word_at(text: &str, byte: usize) -> Range<usize> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let Some((before, after)) = text.split_at_checked(byte.min(text.len())) else {
        return byte..byte;
    };
    let start = before
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_word(c))
        .last()
        .map_or(before.len(), |(index, _)| index);
    let end = after
        .char_indices()
        .find(|&(_, c)| !is_word(c))
        .map_or(text.len(), |(index, _)| before.len() + index);
    start..end
}

#[derive(Clone, Copy)]
struct Row {
    entry: usize,
    start: usize,
    end: usize,
    first: bool,
}

#[derive(Clone, Copy)]
struct Wrapped {
    tab: LogTab,
    revision: u64,
    entries: usize,
    width: Pixels,
}

pub(super) trait ShowsLog: 'static {
    fn log_parts(&mut self) -> (&mut LogView, &Log);
}

impl ShowsLog for LoaderApp {
    fn log_parts(&mut self) -> (&mut LogView, &Log) {
        let log = match self.log_tab {
            LogTab::Run(id) => self
                .runs
                .iter()
                .find(|run| run.id == id)
                .map_or(&self.loader_log, |run| &run.log),
            LogTab::Loader => &self.loader_log,
        };
        (&mut self.output_ui.view, log)
    }
}

pub(super) struct LogView {
    scroll: UniformListScrollHandle,
    rows: Vec<Row>,
    wrapped: Option<Wrapped>,
    focus: Option<FocusHandle>,
    selection: Option<(Spot, Spot)>,
    selecting: bool,
    stepped: Option<Instant>,
    pointer_inside: bool,
    hovered: Option<usize>,
}

impl Default for LogView {
    fn default() -> Self {
        Self {
            scroll: UniformListScrollHandle::new(),
            rows: Vec::new(),
            wrapped: None,
            focus: None,
            selection: None,
            selecting: false,
            stepped: None,
            pointer_inside: false,
            hovered: None,
        }
    }
}

impl LogView {
    fn viewport(&self) -> Bounds<Pixels> {
        self.scroll.0.borrow().base_handle.bounds()
    }

    fn offset(&self) -> Point<Pixels> {
        self.scroll.0.borrow().base_handle.offset()
    }

    fn max_offset(&self) -> Pixels {
        self.scroll.0.borrow().base_handle.max_offset().y
    }

    fn wrap_width(&self) -> Pixels {
        (self.viewport().size.width - TEXT_MARGIN * 2.0).max(px(0.0))
    }

    fn sync(&mut self, tab: LogTab, log: &Log, window: &Window) {
        let width = self.wrap_width();
        let appended = self
            .wrapped
            .filter(|wrapped| wrapped.tab == tab)
            .and_then(|wrapped| log.appended(wrapped.revision, wrapped.entries));
        if appended.is_none() {
            self.selection = None;
            self.selecting = false;
        }
        let from = match appended {
            Some(added) if self.wrapped.is_some_and(|wrapped| wrapped.width == width) => {
                added.start
            }
            _ => {
                self.rows.clear();
                0
            }
        };
        self.wrapped = Some(Wrapped {
            tab,
            revision: log.revision,
            entries: log.view.len(),
            width,
        });
        if from == log.view.len() {
            return;
        }
        let mut wrapper = window
            .text_system()
            .line_wrapper(widgets::mono_font(), theme::BODY);
        for entry in from..log.view.len() {
            let Some(line) = log.entry(entry) else {
                continue;
            };
            let mut start = 0;
            for paragraph in line.text.split('\n') {
                let mut row_start = start;
                let first = start == 0;
                if width > px(0.0) && !paragraph.is_empty() {
                    let fragments = [LineFragment::text(paragraph)];
                    for boundary in wrapper.wrap_line(&fragments, width, IndentAdjustment::NoIndent)
                    {
                        self.rows.push(Row {
                            entry,
                            start: row_start,
                            end: start + boundary.ix,
                            first: first && row_start == 0,
                        });
                        row_start = start + boundary.ix;
                    }
                }
                self.rows.push(Row {
                    entry,
                    start: row_start,
                    end: start + paragraph.len(),
                    first: first && row_start == 0,
                });
                start += paragraph.len() + 1;
            }
        }
    }

    fn row_at(&self, y: Pixels) -> Option<usize> {
        let viewport = self.viewport();
        let y = y.clamp(viewport.top(), viewport.bottom() - px(1.0));
        let row = ((y - viewport.top() - self.offset().y) / LINE_HEIGHT).max(0.0) as usize;
        Some(row.min(self.rows.len().checked_sub(1)?))
    }

    fn spot_at(&self, log: &Log, position: Point<Pixels>, window: &Window) -> Option<Spot> {
        let row = self.rows[self.row_at(position.y)?];
        let text = log.entry(row.entry)?.text.get(row.start..row.end)?;
        let run = TextRun {
            len: text.len(),
            font: widgets::mono_font(),
            color: theme::FG.into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let x = position.x - self.viewport().left() - TEXT_MARGIN;
        let byte = window
            .text_system()
            .shape_line(
                SharedString::from(text.to_owned()),
                theme::BODY,
                &[run],
                None,
            )
            .closest_index_for_x(x);
        Some(Spot {
            entry: row.entry,
            byte: row.start + byte,
        })
    }

    fn press(&mut self, log: &Log, event: &MouseDownEvent, window: &Window) {
        let Some(spot) = self.spot_at(log, event.position, window) else {
            self.selection = None;
            return;
        };
        self.selecting = true;
        self.stepped = None;
        let text = log.entry(spot.entry).map_or("", |line| line.text.as_str());
        let at = |byte| Spot {
            entry: spot.entry,
            byte,
        };
        self.selection = Some(match (event.click_count, self.selection) {
            (1, Some((anchor, _))) if event.modifiers.shift => (anchor, spot),
            (2, _) => {
                let word = word_at(text, spot.byte);
                (at(word.start), at(word.end))
            }
            (count, _) if count >= 3 => (at(0), at(text.len())),
            _ => (spot, spot),
        });
    }

    fn drag(&mut self, log: &Log, position: Point<Pixels>, window: &Window) {
        if let Some(spot) = self.spot_at(log, position, window)
            && let Some((_, head)) = self.selection.as_mut()
        {
            *head = spot;
        }
    }

    fn step_past_edge(&mut self, log: &Log, window: &Window) {
        if !self.selecting {
            self.stepped = None;
            return;
        }
        let viewport = self.viewport();
        let pointer = window.mouse_position();
        let speed = selection_scroll_speed(
            pointer.y.as_f32(),
            viewport.top().as_f32()..viewport.bottom().as_f32(),
            0.0..window.viewport_size().height.as_f32(),
        );
        if speed == 0.0 {
            self.stepped = None;
            return;
        }
        let now = Instant::now();
        let dt = self
            .stepped
            .map_or(1.0 / 60.0, |at| now.duration_since(at).as_secs_f32())
            .min(0.1);
        self.stepped = Some(now);
        let offset = self.offset();
        let y = (offset.y - px(speed * dt)).clamp(-self.max_offset(), px(0.0));
        self.scroll
            .0
            .borrow()
            .base_handle
            .set_offset(point(offset.x, y));
        self.drag(log, pointer, window);
        window.request_animation_frame();
    }

    fn select_all(&mut self, log: &Log) {
        self.selection = log.view.len().checked_sub(1).map(|last| {
            (
                Spot { entry: 0, byte: 0 },
                Spot {
                    entry: last,
                    byte: log.entry(last).map_or(0, |line| line.text.len()),
                },
            )
        });
    }

    fn ordered(&self) -> Option<(Spot, Spot)> {
        self.selection
            .map(|(anchor, head)| (anchor.min(head), anchor.max(head)))
            .filter(|(from, to)| from != to)
    }

    fn selected_bytes(&self, row: Row) -> Option<Range<usize>> {
        let (from, to) = self.ordered()?;
        if row.entry < from.entry || row.entry > to.entry {
            return None;
        }
        let start = if row.entry == from.entry {
            from.byte.max(row.start)
        } else {
            row.start
        };
        let end = if row.entry == to.entry {
            to.byte.min(row.end)
        } else {
            row.end
        };
        (start < end).then(|| start - row.start..end - row.start)
    }

    fn rows_in(&self, range: Range<usize>, log: &Log) -> Vec<AnyElement> {
        range
            .filter_map(|index| {
                let row = *self.rows.get(index)?;
                let line = log.entry(row.entry)?;
                let text = line.text.get(row.start..row.end)?;
                let hovered = self.hovered == Some(row.entry);
                let highlights = self.selected_bytes(row).map(|range| {
                    (
                        range,
                        HighlightStyle {
                            background_color: Some(theme::SEL.into()),
                            ..Default::default()
                        },
                    )
                });
                Some(
                    div()
                        .relative()
                        .h(LINE_HEIGHT)
                        .px(TEXT_MARGIN)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_color(tone_color(line.tone))
                        .when(hovered, |this| this.bg(theme::alpha(theme::FG_DARK, 0.08)))
                        .child(
                            StyledText::new(SharedString::from(text.to_owned()))
                                .with_highlights(highlights),
                        )
                        .when(hovered && row.first, |this| {
                            this.child(line_copy_button(index, line.text.clone()))
                        })
                        .into_any_element(),
                )
            })
            .collect()
    }

    fn copy_selection(&self, log: &Log) -> Option<String> {
        self.ordered().map(|(from, to)| log.selected_text(from, to))
    }

    fn render<V: ShowsLog>(
        &mut self,
        tab: LogTab,
        log: &Log,
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> AnyElement {
        let focus = self.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        self.sync(tab, log, window);
        self.step_past_edge(log, window);
        if -self.offset().y >= self.max_offset() - px(0.5) {
            self.scroll.scroll_to_bottom();
        }
        self.hovered = self
            .pointer_inside
            .then(|| self.row_at(window.mouse_position().y))
            .flatten()
            .map(|row| self.rows[row].entry);
        let entity = cx.entity().downgrade();
        let selecting = self.selecting;
        div()
            .id("log-view")
            .relative()
            .flex_1()
            .min_h_0()
            .font_family(theme::MONO)
            .text_size(theme::BODY)
            .line_height(LINE_HEIGHT)
            .cursor_text()
            .track_focus(&focus)
            .on_hover(cx.listener(|owner: &mut V, hovered: &bool, _, cx| {
                owner.log_parts().0.pointer_inside = *hovered;
                cx.notify();
            }))
            .on_mouse_move(cx.listener(|owner: &mut V, event: &MouseMoveEvent, _, cx| {
                let view = owner.log_parts().0;
                let hovered = view
                    .row_at(event.position.y)
                    .map(|row| view.rows[row].entry);
                if hovered != view.hovered {
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|owner: &mut V, event: &MouseDownEvent, window, cx| {
                    let (view, log) = owner.log_parts();
                    view.press(log, event, window);
                    if let Some(focus) = &view.focus {
                        window.focus(focus, cx);
                    }
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(|owner: &mut V, event: &KeyDownEvent, _, cx| {
                if !event.keystroke.modifiers.secondary(cx) {
                    return;
                }
                let (view, log) = owner.log_parts();
                match event.keystroke.key.as_str() {
                    "c" => {
                        if let Some(text) = view.copy_selection(log) {
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                        }
                        cx.stop_propagation();
                    }
                    "a" => {
                        view.select_all(log);
                        cx.stop_propagation();
                        cx.notify();
                    }
                    _ => {}
                }
            }))
            .child(
                uniform_list(
                    "log-rows",
                    self.rows.len(),
                    cx.processor(|owner: &mut V, range: Range<usize>, _, cx| {
                        let (view, log) = owner.log_parts();
                        if view
                            .wrapped
                            .is_some_and(|wrapped| wrapped.width != view.wrap_width())
                        {
                            cx.notify();
                        }
                        view.rows_in(range, log)
                    }),
                )
                .track_scroll(&self.scroll)
                .size_full(),
            )
            .child(Scrollbar::vertical(&self.scroll))
            .when(selecting, |this| {
                this.child(
                    canvas(
                        |_, _, _| {},
                        move |_, _, window, _| {
                            let moved = entity.clone();
                            window.on_mouse_event(
                                move |event: &MouseMoveEvent, phase, window, cx| {
                                    if phase != DispatchPhase::Bubble {
                                        return;
                                    }
                                    moved
                                        .update(cx, |owner, cx| {
                                            let (view, log) = owner.log_parts();
                                            if event.pressed_button == Some(MouseButton::Left) {
                                                view.drag(log, event.position, window);
                                            } else {
                                                view.selecting = false;
                                            }
                                            cx.notify();
                                        })
                                        .ok();
                                },
                            );
                            let released = entity.clone();
                            window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                                if phase == DispatchPhase::Bubble
                                    && event.button == MouseButton::Left
                                {
                                    released
                                        .update(cx, |owner, cx| {
                                            owner.log_parts().0.selecting = false;
                                            cx.notify();
                                        })
                                        .ok();
                                }
                            });
                        },
                    )
                    .absolute()
                    .size_full(),
                )
            })
            .into_any_element()
    }
}

fn line_copy_button(index: usize, text: String) -> Stateful<Div> {
    let height = LINE_HEIGHT - px(2.0);
    div()
        .id(("log-copy-line", index))
        .absolute()
        .top(px(1.0))
        .right_0()
        .w(height * 1.6)
        .h(height)
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::BG_HL)
        .border_1()
        .border_color(theme::alpha(theme::BLUE, 0.0))
        .text_color(theme::FG)
        .text_size(height * 0.7)
        .line_height(height)
        .cursor_pointer()
        .hover(|style| style.bg(theme::SEL).border_color(theme::BLUE))
        .tooltip(widgets::tooltip("Copy this line"))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone())))
        .child(ICON_COPY)
}

fn collapse_button(collapsed: bool, cx: &mut Context<LoaderApp>) -> Stateful<Div> {
    let (icon, hint) = if collapsed {
        (ICON_EXPAND, "Show the output")
    } else {
        (ICON_COLLAPSE, "Hide the output, keeping its last line")
    };
    div()
        .id("log-collapse")
        .flex()
        .flex_none()
        .items_center()
        .h(theme::INTERACT_HEIGHT)
        .px(theme::BUTTON_PADDING)
        .text_color(theme::FG)
        .tooltip(widgets::tooltip(hint))
        .on_click(cx.listener(move |app, _: &ClickEvent, _, cx| {
            app.log_collapsed = !collapsed;
            cx.notify();
        }))
        .child(icon)
}

struct Resize {
    from: Pixels,
}

pub(super) struct Ui {
    height: Option<Pixels>,
    resize: Option<Resize>,
    bottom: Rc<Cell<Pixels>>,
    slides: [widgets::Slide; 4],
    view: LogView,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            height: None,
            resize: None,
            bottom: Rc::default(),
            slides: std::array::from_fn(|_| widgets::Slide::new(widgets::SLIDE_LENGTH)),
            view: LogView::default(),
        }
    }
}

fn pick_log_tab(
    app: gpui_kit::WeakEntity<LoaderApp>,
    tab: LogTab,
) -> impl Fn(&mut Window, &mut App) + 'static {
    move |_, cx| {
        app.update(cx, |app, cx| {
            app.log_tab = tab;
            cx.notify();
        })
        .ok();
    }
}

struct Control {
    slot: usize,
    button: widgets::SlideButton,
}

const CLEAR: usize = 0;
const COPY_ALL: usize = 1;
const SAVE: usize = 2;
const PIPE: usize = 3;

impl LoaderApp {
    fn shown_log_tab(&self) -> LogTab {
        match self.log_tab {
            LogTab::Run(id) if self.context_run() == Some(id) => LogTab::Run(id),
            _ => LogTab::Loader,
        }
    }

    fn render_log_line(&mut self, cx: &mut Context<Self>) -> Div {
        let tab = self.log_tab;
        let shown_run = self.shown_run_log().map(|run| (run.id, run.title.clone()));
        let title = match (tab, &shown_run) {
            (LogTab::Run(_), Some((_, title))) => title.clone(),
            _ => "Loader".to_owned(),
        };
        let (text, tone) = self
            .log_ref(tab)
            .last()
            .map_or((String::new(), Tone::Normal), |line| {
                (
                    line.text.lines().last().unwrap_or_default().to_owned(),
                    line.tone,
                )
            });
        let app = cx.entity().downgrade();
        let picker = match shown_run {
            Some((id, run_title)) => div()
                .flex_none()
                .min_w_0()
                .max_w(relative(0.5))
                .overflow_hidden()
                .text_color(theme::COMMENT)
                .child(widgets::combo(
                    "log-line-tab",
                    title,
                    vec![
                        widgets::Choice::new(
                            "Loader",
                            tab == LogTab::Loader,
                            pick_log_tab(app.clone(), LogTab::Loader),
                        ),
                        widgets::Choice::new(
                            run_title,
                            tab == LogTab::Run(id),
                            pick_log_tab(app.clone(), LogTab::Run(id)),
                        ),
                    ],
                )),
            None => div().flex_none().text_color(theme::COMMENT).child(title),
        };
        div()
            .flex()
            .items_center()
            .gap(theme::ITEM_GAP)
            .h(theme::INTERACT_HEIGHT)
            .child(picker)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(theme::MONO)
                    .text_color(tone_color(tone))
                    .child(text),
            )
            .child(collapse_button(true, cx))
    }

    fn render_log_tabs(&mut self, cx: &mut Context<Self>) -> Div {
        let shown_run = self.shown_run_log().map(|run| (run.id, run.title.clone()));
        let tab = |id: ElementId, label: String, value: LogTab, current: LogTab| {
            widgets::tab_value(
                id,
                label,
                current == value,
                cx.listener(move |app, _: &ClickEvent, _, cx| {
                    app.log_tab = value;
                    cx.notify();
                }),
            )
        };
        let current = self.log_tab;
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .child(tab(
                "log-tab-loader".into(),
                "Loader".to_owned(),
                LogTab::Loader,
                current,
            ))
            .when_some(shown_run, |this, (id, title)| {
                this.child(tab("log-tab-run".into(), title, LogTab::Run(id), current))
            })
    }

    fn render_log_slide(
        &mut self,
        control: Control,
        window: &mut Window,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        let slot = control.slot;
        let open = self.output_ui.slides[slot].value(window);
        widgets::slide_button(
            ("log-control", slot),
            control.button,
            open,
            window,
            cx.listener(move |app, hovered: &bool, _, cx| {
                app.output_ui.slides[slot].set(*hovered);
                cx.notify();
            }),
            cx.listener(move |app, _: &ClickEvent, window, cx| {
                on_click(app, window, cx);
                cx.notify();
            }),
        )
    }

    fn render_log_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let tab = self.log_tab;
        let filter = if let LogTab::Run(_) = tab {
            let log = self.log_ref(tab);
            let current = log.filter;
            let app = cx.entity().downgrade();
            let choice = |label: String, value: Option<usize>| {
                let app = app.clone();
                widgets::Choice::new(label, current == value, move |_, cx| {
                    app.update(cx, |app, cx| {
                        app.log_of(tab).set_filter(value);
                        cx.notify();
                    })
                    .ok();
                })
            };
            let choices = std::iter::once(choice("All".to_owned(), None))
                .chain(
                    log.tags
                        .iter()
                        .enumerate()
                        .map(|(index, tag)| choice(tag.clone(), Some(index))),
                )
                .collect();
            let text = current.map_or("All".to_owned(), |index| log.tags[index].clone());
            Some(widgets::combo("log-tag", text, choices))
        } else {
            None
        };
        let log = self.log_ref(tab);
        let filled = !log.is_empty();
        let piping = log
            .pipe
            .as_ref()
            .map(|pipe| pipe.path().display().to_string());
        let control = |slot, icon, label, enabled, selected| Control {
            slot,
            button: widgets::SlideButton {
                icon,
                label,
                enabled,
                selected,
            },
        };
        let clear = self.render_log_slide(
            control(CLEAR, ICON_CLEAR, "Clear", filled, false),
            window,
            cx,
            move |app, _, _| app.log_of(tab).clear(),
        );
        let copy = self.render_log_slide(
            control(COPY_ALL, ICON_COPY, "Copy all", filled, false),
            window,
            cx,
            move |app, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(app.log_of(tab).text.clone()));
            },
        );
        let save = self.render_log_slide(
            control(SAVE, ICON_SAVE, "Save", filled, false),
            window,
            cx,
            move |app, _, _| {
                let text = app.log_of(tab).text.clone();
                app.save_text("output", "output.txt", text);
            },
        );
        let pipe = self
            .render_log_slide(
                control(PIPE, ICON_PIPE, "Pipe", true, piping.is_some()),
                window,
                cx,
                move |app, _, _| {
                    let toggled = crate::pipe::toggle(&mut app.log_of(tab).pipe, "output.txt", 0);
                    app.report_pipe(toggled);
                },
            )
            .when_some(piping, |this, path| {
                this.tooltip(widgets::tooltip(format!("Appending to {path}")))
            });
        div()
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap(theme::ITEM_GAP)
            .children(filter)
            .child(
                div().flex().flex_1().justify_end().child(
                    widgets::button_group(false)
                        .child(clear)
                        .child(copy)
                        .child(save)
                        .child(pipe),
                ),
            )
    }

    fn render_resize_handle(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let resizing = self.output_ui.resize.is_some();
        div()
            .id("output-resize")
            .group("output-resize")
            .absolute()
            .left_0()
            .right_0()
            .top(-RESIZE_GRAB)
            .h(RESIZE_GRAB * 2.0)
            .cursor_row_resize()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|app, event: &MouseDownEvent, _, cx| {
                    if event.click_count == 2 {
                        app.output_ui.resize = None;
                        app.log_collapsed = true;
                    } else {
                        app.output_ui.resize = app.output_ui.height.map(|from| Resize { from });
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(RESIZE_GRAB)
                    .h(SEPARATOR)
                    .bg(if resizing { theme::FG } else { theme::BORDER })
                    .group_hover("output-resize", |style| style.bg(theme::FG)),
            )
    }

    fn drag_resize(&mut self, y: Pixels, window_height: Pixels) {
        let Some(from) = self.output_ui.resize.as_ref().map(|resize| resize.from) else {
            return;
        };
        let raw = self.output_ui.bottom.get() - y;
        let previous = self.output_ui.height.unwrap_or(from);
        if raw < COLLAPSED_HEIGHT && raw < previous {
            self.output_ui.resize = None;
            self.output_ui.height = Some(from);
            self.log_collapsed = true;
            return;
        }
        let max = (window_height * 0.5).max(px(60.0));
        self.output_ui.height = Some(raw.clamp(MIN_HEIGHT.min(max), max));
    }

    fn resize_listeners(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let app = cx.entity().downgrade();
        let bottom = self.output_ui.bottom.clone();
        let resizing = self.output_ui.resize.is_some();
        canvas(
            move |bounds, _, _| bottom.set(bounds.bottom()),
            move |_, _, window, _| {
                if !resizing {
                    return;
                }
                let moved = app.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let window_height = window.viewport_size().height;
                    moved
                        .update(cx, |app, cx| {
                            if event.pressed_button == Some(MouseButton::Left) {
                                app.drag_resize(event.position.y, window_height);
                            } else {
                                app.output_ui.resize = None;
                            }
                            cx.notify();
                        })
                        .ok();
                });
                let released = app.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                        released
                            .update(cx, |app, cx| {
                                app.output_ui.resize = None;
                                cx.notify();
                            })
                            .ok();
                    }
                });
            },
        )
        .absolute()
        .size_full()
    }

    fn render_log_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let tabs = self.render_log_tabs(cx);
        let controls = self.render_log_controls(window, cx);
        let tab = self.shown_log_tab();
        let (view, log) = self.log_parts();
        let log_view = view.render(tab, log, window, cx);
        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .pt(px(4.0))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_start()
                    .child(tabs)
                    .child(collapse_button(false, cx)),
            )
            .child(divider())
            .child(controls.mt(theme::LINE_GAP))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .mt(theme::LINE_GAP)
                    .child(log_view),
            )
    }

    pub(super) fn render_output(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let window_height = window.viewport_size().height;
        let panel = div()
            .relative()
            .flex_none()
            .w_full()
            .bg(theme::BG)
            .pt(SEPARATOR)
            .child(self.resize_listeners(cx));
        if self.log_collapsed {
            self.output_ui.resize = None;
            let line = self.render_log_line(cx);
            return panel
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(SEPARATOR)
                        .bg(theme::BORDER),
                )
                .child(div().px(PANEL_MARGIN_X).py(PANEL_MARGIN_Y).child(line))
                .into_any_element();
        }
        let max = (window_height * 0.5).max(px(60.0));
        let height = self
            .output_ui
            .height
            .unwrap_or_else(|| (window_height * 0.22).clamp(px(70.0), px(180.0)))
            .clamp(MIN_HEIGHT.min(max), max);
        self.output_ui.height = Some(height);
        let content = self.render_log_panel(window, cx);
        let handle = self.render_resize_handle(cx);
        panel
            .h(height)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .overflow_hidden()
                    .px(PANEL_MARGIN_X)
                    .py(PANEL_MARGIN_Y)
                    .child(content),
            )
            .child(handle)
            .into_any_element()
    }
}

#[cfg(test)]
pub(super) fn snapshots() -> Vec<super::snapshots::State> {
    fn mixed_loader_log(app: &mut LoaderApp) {
        for i in 0..400 {
            let (tone, text) = match i % 9 {
                4 => (
                    Tone::Warning,
                    format!("Client {i} is slow to answer; retrying the read."),
                ),
                7 => (
                    Tone::Error,
                    format!(
                        "Cannot reach the agent in client {i}: the pipe was closed by the other side before the reply arrived, so the request was dropped."
                    ),
                ),
                _ => (
                    Tone::Normal,
                    format!("Scanned clients: {i} found, agent version 2026.11.{i:02}"),
                ),
            };
            app.loader_log.push_tone(tone, text);
        }
        app.loader_log.push_tone(
            Tone::Normal,
            "A line with\ntwo paragraphs in one entry".into(),
        );
    }
    fn run_with_log(app: &mut LoaderApp) {
        let target = super::store_page::fake_target(4120);
        app.targets = vec![target.clone()];
        let mut run = Run::new(
            7,
            "Agility course".into(),
            None,
            Some(target),
            None,
            &app.settings,
        );
        for i in 0..120 {
            let (tone, text) = match i % 5 {
                0 => (Tone::Normal, format!("[walker] Walking to obstacle {i}")),
                1 => (
                    Tone::Normal,
                    format!("[agility] Crossed obstacle {i}, +12 xp"),
                ),
                2 => (
                    Tone::Warning,
                    format!("[antiban] Taking a short break of {i} s"),
                ),
                3 => (Tone::Normal, format!("Lap {i} done")),
                _ => (
                    Tone::Error,
                    format!("[walker] Path to obstacle {i} is blocked, trying the next one"),
                ),
            };
            run.log.push_tone(tone, text);
        }
        app.runs.push(run);
        app.active_run = Some(7);
        app.page = Page::Run(RunPage::Overview);
    }
    vec![
        ("output-collapsed", |app| {
            mixed_loader_log(app);
            app.page = Page::Settings;
        }),
        ("output-collapsed-run", |app| {
            run_with_log(app);
            app.page = Page::Accounts;
        }),
        ("output-loader-long", |app| {
            mixed_loader_log(app);
            app.output_ui.height = Some(px(300.0));
        }),
        ("output-run-tab", |app| {
            run_with_log(app);
            app.output_ui.height = Some(px(300.0));
        }),
        ("output-run-filtered", |app| {
            run_with_log(app);
            app.runs[0].log.set_filter(Some(0));
            app.output_ui.height = Some(px(300.0));
        }),
        ("output-empty", |_| {}),
    ]
}

#[cfg(test)]
mod tests {
    use gpui_kit::test::{TestEventExt, TestWindowExt};
    use gpui_kit::{Render, TestAppContext, WindowBounds, WindowOptions};

    use super::*;

    #[test]
    fn a_dragged_selection_scrolls_as_fast_down_as_up_for_the_same_share_of_room() {
        let speed = |y| selection_scroll_speed(y, 300.0..590.0, 0.0..600.0);
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

    fn log_of(texts: &[(Tone, &str)]) -> Log {
        let mut log = Log::new(*backend::LOG_LINES.start());
        for (tone, text) in texts {
            log.push_tone(*tone, (*text).into());
        }
        log
    }

    #[test]
    fn only_lines_added_since_count_as_appended() {
        let mut log = log_of(&[(Tone::Normal, "a"), (Tone::Error, "[x] b")]);
        let (revision, entries) = (log.revision, log.view.len());
        assert_eq!(log.appended(revision, entries), Some(2..2));
        log.push_tone(Tone::Warning, "c".into());
        assert_eq!(log.appended(revision, entries), Some(2..3));
        log.set_filter(Some(0));
        assert_eq!(log.appended(revision, entries), None);
        let (revision, entries) = (log.revision, log.view.len());
        log.clear();
        assert_eq!(log.appended(revision, entries), None);
    }

    #[test]
    fn a_selection_copies_the_text_between_its_ends() {
        let log = log_of(&[
            (Tone::Normal, "first line"),
            (Tone::Error, "second\nwith two paragraphs"),
            (Tone::Normal, "third"),
        ]);
        let spot = |entry, byte| Spot { entry, byte };
        assert_eq!(log.selected_text(spot(0, 6), spot(0, 10)), "line");
        assert_eq!(
            log.selected_text(spot(0, 6), spot(2, 3)),
            "line\nsecond\nwith two paragraphs\nthi"
        );
        assert_eq!(log.selected_text(spot(1, 99), spot(1, 99)), "");
    }

    #[test]
    fn a_double_click_picks_the_word_under_it() {
        assert_eq!(word_at("load agent_v2 now", 7), 5..13);
        assert_eq!(word_at("load agent_v2 now", 4), 0..4);
        assert_eq!(word_at("é tail", 0), 0..2);
    }

    struct Harness {
        view: LogView,
        log: Log,
    }

    impl ShowsLog for Harness {
        fn log_parts(&mut self) -> (&mut LogView, &Log) {
            (&mut self.view, &self.log)
        }
    }

    impl Render for Harness {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let view = self.view.render(LogTab::Loader, &self.log, window, cx);
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(div().flex_none().h(px(300.0)))
                .child(div().flex().flex_col().flex_none().h(px(200.0)).child(view))
                .child(div().flex_1())
        }
    }

    #[gpui_kit::test]
    fn a_selection_dragged_above_the_output_keeps_scrolling_up(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut log = Log::new(*backend::LOG_LINES.start());
        for i in 0..500 {
            log.push(false, format!("line {i}"));
        }
        let (window, harness) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(800.0), px(600.0)),
                    })),
                    ..Default::default()
                },
                cx,
                |_, cx| {
                    cx.new(|_| Harness {
                        view: LogView::default(),
                        log,
                    })
                },
            )
            .unwrap()
        });
        let offset = |cx: &mut TestAppContext| harness.read_with(cx, |h, _| h.view.offset().y);
        for _ in 0..3 {
            cx.update_window(window, |_, window, cx| window.render_frame(cx))
                .unwrap();
        }
        let bottom = offset(cx);
        assert!(bottom < px(0.0), "{bottom:?}");
        let inside = point(px(50.0), px(400.0));
        cx.update_window(window, |_, window, cx| {
            window.pointer_move(inside, None, cx);
            window.pointer_down(inside, MouseButton::Left, cx);
            for y in [380.0, 340.0, 300.0, 250.0] {
                window.pointer_move(point(px(50.0), px(y)), Some(MouseButton::Left), cx);
            }
        })
        .unwrap();
        let moved = offset(cx);
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(16));
            cx.update_window(window, |_, window, cx| window.render_frame(cx))
                .unwrap();
        }
        let held = offset(cx);
        assert!(
            moved > bottom && held > moved,
            "{bottom:?} {moved:?} {held:?}"
        );
        cx.update_window(window, |_, window, cx| {
            window.pointer_up(point(px(50.0), px(250.0)), MouseButton::Left, cx);
        })
        .unwrap();
        let selection = harness.read_with(cx, |h, _| h.view.ordered());
        let (from, to) = selection.expect("the drag selected text");
        assert!(from.entry < to.entry && to.entry < 500);
        cx.update_window(window, |_, window, cx| {
            window.key_down("ctrl-c", false, cx);
        })
        .unwrap();
        let copied = cx.read_from_clipboard().and_then(|item| item.text());
        let expected = harness.read_with(cx, |h, _| h.log.selected_text(from, to));
        assert_eq!(copied, Some(expected));
    }
}
