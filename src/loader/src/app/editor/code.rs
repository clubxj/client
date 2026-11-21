use std::ops::Range;
use std::time::{Duration, Instant};

use gpui_kit::component::scroll::{ScrollableElement, ScrollbarAxis};
use gpui_kit::{
    App, Bounds, ClipboardItem, Context, CursorStyle, DispatchPhase, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, Font,
    FontFeatures, Global, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId,
    InteractiveElement, IntoElement, KeyBinding, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PaintQuad, ParentElement, PathBuilder, Pixels, Point, Render,
    ScrollHandle, ShapedLine, SharedString, Size, StatefulInteractiveElement, Style, Styled,
    TextAlign, TextRun, UTF16Selection, UnderlineStyle, Window, div, fill, point, px, relative,
    size,
};

use crate::highlight::{self, Highlighter, Kind};
use crate::ui::{theme, widgets};

const MARGIN_X: Pixels = px(4.0);
const MARGIN_Y: Pixels = px(2.0);
const GUTTER_GAP: Pixels = px(8.0);
const TAB_COLUMNS: usize = 4;
const UNDO_LIMIT: usize = 100;
const UNDO_GROUP: Duration = Duration::from_secs(1);
const CONTEXT: &str = "CodeEditor";

gpui_kit::actions!(
    code_editor,
    [
        MoveLeft,
        MoveRight,
        MoveUp,
        MoveDown,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        LineStart,
        LineEnd,
        SelectLineStart,
        SelectLineEnd,
        TextStart,
        TextEnd,
        SelectTextStart,
        SelectTextEnd,
        PageUp,
        PageDown,
        SelectPageUp,
        SelectPageDown,
        Backspace,
        Delete,
        BackspaceWord,
        DeleteWord,
        Newline,
        Indent,
        Outdent,
        SelectAll,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
    ]
);

const WORD_KEYS: [&str; 6] = [
    "ctrl-left",
    "ctrl-right",
    "ctrl-shift-left",
    "ctrl-shift-right",
    "ctrl-backspace",
    "ctrl-delete",
];
const WORD_KEYS_MAC: [&str; 6] = [
    "alt-left",
    "alt-right",
    "alt-shift-left",
    "alt-shift-right",
    "alt-backspace",
    "alt-delete",
];

const COMMAND_KEYS: [&str; 7] = [
    "ctrl-a",
    "ctrl-c",
    "ctrl-x",
    "ctrl-v",
    "ctrl-z",
    "ctrl-shift-z",
    "ctrl-y",
];
const COMMAND_KEYS_MAC: [&str; 7] = [
    "cmd-a",
    "cmd-c",
    "cmd-x",
    "cmd-v",
    "cmd-z",
    "cmd-shift-z",
    "cmd-y",
];

struct KeysBound;

impl Global for KeysBound {}

fn bind_keys(cx: &mut App) {
    if cx.has_global::<KeysBound>() {
        return;
    }
    cx.set_global(KeysBound);
    let context = Some(CONTEXT);
    let [all, copy, cut, paste, undo, redo, redo_y] = if cfg!(target_os = "macos") {
        COMMAND_KEYS_MAC
    } else {
        COMMAND_KEYS
    };
    let [left, right, shift_left, shift_right, backspace, delete] = if cfg!(target_os = "macos") {
        WORD_KEYS_MAC
    } else {
        WORD_KEYS
    };
    let mut bindings = vec![
        KeyBinding::new("left", MoveLeft, context),
        KeyBinding::new("right", MoveRight, context),
        KeyBinding::new("up", MoveUp, context),
        KeyBinding::new("down", MoveDown, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new("shift-up", SelectUp, context),
        KeyBinding::new("shift-down", SelectDown, context),
        KeyBinding::new(left, WordLeft, context),
        KeyBinding::new(right, WordRight, context),
        KeyBinding::new(shift_left, SelectWordLeft, context),
        KeyBinding::new(shift_right, SelectWordRight, context),
        KeyBinding::new("home", LineStart, context),
        KeyBinding::new("end", LineEnd, context),
        KeyBinding::new("shift-home", SelectLineStart, context),
        KeyBinding::new("shift-end", SelectLineEnd, context),
        KeyBinding::new("ctrl-home", TextStart, context),
        KeyBinding::new("ctrl-end", TextEnd, context),
        KeyBinding::new("ctrl-shift-home", SelectTextStart, context),
        KeyBinding::new("ctrl-shift-end", SelectTextEnd, context),
        KeyBinding::new("pageup", PageUp, context),
        KeyBinding::new("pagedown", PageDown, context),
        KeyBinding::new("shift-pageup", SelectPageUp, context),
        KeyBinding::new("shift-pagedown", SelectPageDown, context),
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("shift-backspace", Backspace, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new(backspace, BackspaceWord, context),
        KeyBinding::new(delete, DeleteWord, context),
        KeyBinding::new("enter", Newline, context),
        KeyBinding::new("shift-enter", Newline, context),
        KeyBinding::new("tab", Indent, context),
        KeyBinding::new("shift-tab", Outdent, context),
        KeyBinding::new(all, SelectAll, context),
        KeyBinding::new(copy, Copy, context),
        KeyBinding::new(cut, Cut, context),
        KeyBinding::new(paste, Paste, context),
        KeyBinding::new(undo, Undo, context),
        KeyBinding::new(redo, Redo, context),
        KeyBinding::new(redo_y, Redo, context),
    ];
    if cfg!(target_os = "macos") {
        bindings.extend([
            KeyBinding::new("cmd-left", LineStart, context),
            KeyBinding::new("cmd-right", LineEnd, context),
            KeyBinding::new("cmd-shift-left", SelectLineStart, context),
            KeyBinding::new("cmd-shift-right", SelectLineEnd, context),
            KeyBinding::new("cmd-up", TextStart, context),
            KeyBinding::new("cmd-down", TextEnd, context),
            KeyBinding::new("cmd-shift-up", SelectTextStart, context),
            KeyBinding::new("cmd-shift-down", SelectTextEnd, context),
        ]);
    }
    cx.bind_keys(bindings);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Fold {
    None,
    Open,
    Folded,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Row {
    pub(crate) line: usize,
    pub(crate) fold: Fold,
}

#[derive(Default, PartialEq, Eq)]
pub(crate) struct Shown {
    pub(crate) rows: Vec<Row>,
    pub(crate) headers: Vec<(usize, usize)>,
    pub(crate) lines: usize,
    pub(crate) longest: usize,
}

pub(crate) enum Event {
    Edited,
    Moved(usize),
    Fold(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    Insert,
    Delete,
    Compose,
    Other,
}

struct Snapshot {
    text: String,
    anchor: usize,
    head: usize,
}

#[derive(Clone)]
enum Drag {
    Chars,
    Words(Range<usize>),
    Lines(Range<usize>),
}

#[derive(Clone)]
enum Reveal {
    Cursor,
    Center(Range<usize>),
}

#[derive(Clone)]
struct Metrics {
    font: Font,
    size: Pixels,
    row: Pixels,
    digit: Pixels,
}

#[derive(Clone, Copy)]
struct Geometry {
    origin: Point<Pixels>,
    gutter: Pixels,
    arrow: Pixels,
    text_left: Pixels,
    text_width: Pixels,
    row: Pixels,
}

impl Geometry {
    fn text_x(&self) -> Pixels {
        self.origin.x + self.text_left + MARGIN_X
    }

    fn row_top(&self, row: usize) -> Pixels {
        self.origin.y + MARGIN_Y + self.row * row as f32
    }

    fn row_at(&self, y: Pixels) -> f32 {
        ((y - self.origin.y - MARGIN_Y) / self.row).floor()
    }
}

struct Display {
    text: String,
    tabs: Vec<usize>,
}

impl Display {
    fn new(line: &str) -> Self {
        let tabs: Vec<usize> = line.match_indices('\t').map(|(at, _)| at).collect();
        let text = line.replace('\t', &" ".repeat(TAB_COLUMNS));
        Self { text, tabs }
    }

    fn to_display(&self, at: usize) -> usize {
        at + (TAB_COLUMNS - 1) * self.tabs.partition_point(|&tab| tab < at)
    }

    fn to_buffer(&self, at: usize) -> usize {
        let mut shift = 0;
        for &tab in &self.tabs {
            let start = tab + shift;
            if at <= start {
                break;
            }
            if at < start + TAB_COLUMNS {
                return if at - start < TAB_COLUMNS / 2 {
                    tab
                } else {
                    tab + 1
                };
            }
            shift += TAB_COLUMNS - 1;
        }
        at - shift
    }
}

struct Laid {
    start: usize,
    end: usize,
    display: Display,
    shaped: ShapedLine,
}

impl Laid {
    fn x(&self, at: usize) -> Pixels {
        let at = at.clamp(self.start, self.end) - self.start;
        self.shaped.x_for_index(self.display.to_display(at))
    }

    fn offset_at(&self, x: Pixels) -> usize {
        let index = self.shaped.closest_index_for_x(x);
        (self.start + self.display.to_buffer(index)).min(self.end)
    }
}

pub(crate) struct CodeEditor {
    focus: FocusHandle,
    text: String,
    starts: Vec<usize>,
    anchor: usize,
    head: usize,
    marked: Option<Range<usize>>,
    goal_x: Option<Pixels>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    group: Option<(Group, Instant)>,
    highlighter: Highlighter,
    shown: Shown,
    matches: Vec<Range<usize>>,
    current: Option<usize>,
    reveal: Option<Reveal>,
    scroll: ScrollHandle,
    hover_row: Option<usize>,
    hover_text: bool,
    drag: Option<Drag>,
    metrics: Option<Metrics>,
    geometry: Option<Geometry>,
}

impl EventEmitter<Event> for CodeEditor {}

impl Focusable for CodeEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(at, _)| at + 1))
        .collect()
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn floor_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

impl CodeEditor {
    pub(crate) fn new(text: &str, cx: &mut Context<Self>) -> Self {
        bind_keys(cx);
        Self {
            focus: cx.focus_handle(),
            text: text.to_owned(),
            starts: line_starts(text),
            anchor: 0,
            head: 0,
            marked: None,
            goal_x: None,
            undo: Vec::new(),
            redo: Vec::new(),
            group: None,
            highlighter: Highlighter::default(),
            shown: Shown::default(),
            matches: Vec::new(),
            current: None,
            reveal: None,
            scroll: ScrollHandle::new(),
            hover_row: None,
            hover_text: false,
            drag: None,
            metrics: None,
            geometry: None,
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn load(&mut self, text: &str, cx: &mut Context<Self>) {
        self.text = text.to_owned();
        self.starts = line_starts(text);
        self.anchor = 0;
        self.head = 0;
        self.marked = None;
        self.goal_x = None;
        self.undo.clear();
        self.redo.clear();
        self.group = None;
        self.scroll.set_offset(Point::default());
        cx.notify();
    }

    pub(crate) fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.text == text {
            return;
        }
        self.push_undo();
        self.redo.clear();
        self.group = None;
        self.text = text.to_owned();
        self.starts = line_starts(text);
        self.anchor = floor_boundary(&self.text, self.anchor);
        self.head = floor_boundary(&self.text, self.head);
        self.marked = None;
        self.goal_x = None;
        cx.notify();
    }

    pub(crate) fn set_shown(&mut self, shown: Shown, cx: &mut Context<Self>) {
        if self.shown != shown {
            self.shown = shown;
            cx.notify();
        }
    }

    pub(crate) fn set_matches(
        &mut self,
        matches: Vec<Range<usize>>,
        current: Option<usize>,
        jump: bool,
        cx: &mut Context<Self>,
    ) {
        if jump && let Some(range) = current.and_then(|i| matches.get(i)) {
            self.reveal = Some(Reveal::Center(range.clone()));
        }
        if self.matches != matches || self.current != current {
            self.matches = matches;
            self.current = current;
            cx.notify();
        }
    }

    fn select_line(&mut self, line: usize, window: &mut Window, cx: &mut Context<Self>) {
        let range = self.line_range(line);
        self.anchor = range.start;
        self.head = range.end;
        self.after_move(cx);
        window.focus(&self.focus, cx);
    }

    fn line_of(&self, at: usize) -> usize {
        self.starts.partition_point(|&start| start <= at) - 1
    }

    fn line_range(&self, line: usize) -> Range<usize> {
        let start = self.starts.get(line).copied().unwrap_or(self.text.len());
        let end = self
            .starts
            .get(line + 1)
            .map_or(self.text.len(), |&next| next - 1);
        let end = if self.text[start..end].ends_with('\r') {
            end - 1
        } else {
            end
        };
        start..end
    }

    fn row_of(&self, line: usize) -> usize {
        match self.shown.rows.binary_search_by_key(&line, |row| row.line) {
            Ok(row) => row,
            Err(row) => row.saturating_sub(1),
        }
    }

    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    fn previous_char(&self, at: usize) -> usize {
        self.text[..at]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next_char(&self, at: usize) -> usize {
        self.text[at..]
            .chars()
            .next()
            .map_or(at, |c| at + c.len_utf8())
    }

    fn word_left(&self, at: usize) -> usize {
        let before = &self.text[..at];
        let mut chars = before.char_indices().rev().peekable();
        while chars.next_if(|&(_, c)| !is_word(c)).is_some() {}
        let mut start = chars.peek().map_or(0, |&(i, c)| i + c.len_utf8());
        while let Some((i, _)) = chars.next_if(|&(_, c)| is_word(c)) {
            start = i;
        }
        start
    }

    fn word_right(&self, at: usize) -> usize {
        let after = &self.text[at..];
        let mut chars = after.char_indices().peekable();
        while chars.next_if(|&(_, c)| !is_word(c)).is_some() {}
        while chars.next_if(|&(_, c)| is_word(c)).is_some() {}
        at + chars.peek().map_or(after.len(), |&(i, _)| i)
    }

    fn word_at(&self, at: usize) -> Range<usize> {
        let on_word = self.text[at..].chars().next().is_some_and(is_word);
        if !on_word {
            return at..self.next_char(at);
        }
        let start = self.text[..at]
            .char_indices()
            .rev()
            .take_while(|&(_, c)| is_word(c))
            .last()
            .map_or(at, |(i, _)| i);
        let end = self.text[at..]
            .char_indices()
            .find(|&(_, c)| !is_word(c))
            .map_or(self.text.len(), |(i, _)| at + i);
        start..end
    }

    fn push_undo(&mut self) {
        self.undo.push(Snapshot {
            text: self.text.clone(),
            anchor: self.anchor,
            head: self.head,
        });
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
    }

    fn after_move(&mut self, cx: &mut Context<Self>) {
        self.group = None;
        self.goal_x = None;
        self.reveal = Some(Reveal::Cursor);
        cx.emit(Event::Moved(self.head));
        cx.notify();
    }

    fn move_to(&mut self, at: usize, select: bool, cx: &mut Context<Self>) {
        self.head = at;
        if !select {
            self.anchor = at;
        }
        self.marked = None;
        self.after_move(cx);
    }

    fn edit(&mut self, range: Range<usize>, new: &str, group: Group, cx: &mut Context<Self>) {
        let now = Instant::now();
        let joins = self.group.is_some_and(|(last, at)| {
            last == group
                && (group == Group::Compose
                    || (group != Group::Other && now.duration_since(at) < UNDO_GROUP))
        });
        if !joins {
            self.push_undo();
        }
        self.redo.clear();
        self.text.replace_range(range.clone(), new);
        self.starts = line_starts(&self.text);
        let at = range.start + new.len();
        self.anchor = at;
        self.head = at;
        self.goal_x = None;
        self.reveal = Some(Reveal::Cursor);
        cx.emit(Event::Edited);
        cx.emit(Event::Moved(self.head));
        cx.notify();
        self.group = Some((group, now));
    }

    fn insert(&mut self, new: &str, group: Group, cx: &mut Context<Self>) {
        let range = self.marked.take().unwrap_or_else(|| self.selection());
        self.edit(range, new, group, cx);
    }

    fn restore(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        self.text = snapshot.text;
        self.starts = line_starts(&self.text);
        self.anchor = snapshot.anchor;
        self.head = snapshot.head;
        self.marked = None;
        cx.emit(Event::Edited);
        self.after_move(cx);
    }

    fn metrics(&mut self, window: &Window) -> Metrics {
        if let Some(metrics) = &self.metrics {
            return metrics.clone();
        }
        let mut font = widgets::mono_font();
        font.features = FontFeatures::disable_ligatures();
        let size = theme::BODY;
        let text_system = window.text_system();
        let id = text_system.resolve_font(&font);
        let digit = text_system
            .advance(id, size, '0')
            .map_or(size * 0.6, |advance| advance.width);
        let row = (text_system.ascent(id, size) + text_system.descent(id, size).abs())
            .ceil()
            .max(size);
        let metrics = Metrics {
            font,
            size,
            row,
            digit,
        };
        self.metrics = Some(metrics.clone());
        metrics
    }

    fn gutter_width(&self, metrics: &Metrics) -> (Pixels, Pixels) {
        let digits = self.shown.lines.max(1).to_string().len() as f32;
        let arrow = metrics.row;
        (metrics.digit * digits + MARGIN_X * 3.0 + arrow, arrow)
    }

    fn natural_size(&self, metrics: &Metrics) -> Size<Pixels> {
        let (gutter, _) = self.gutter_width(metrics);
        let text = metrics.digit * self.shown.longest as f32 + MARGIN_X * 2.0 + px(8.0);
        size(
            gutter + GUTTER_GAP + text,
            metrics.row * self.shown.rows.len() as f32 + MARGIN_Y * 2.0,
        )
    }

    fn runs(&mut self, range: Range<usize>, display: &Display, font: &Font) -> Vec<TextRun> {
        let marked = self.marked.clone();
        let tokens = self.highlighter.tokens(&self.text);
        let first = tokens.partition_point(|(token, _)| token.end <= range.start);
        let mut cuts: Vec<(usize, Kind)> = Vec::new();
        for (token, kind) in &tokens[first..] {
            if token.start >= range.end {
                break;
            }
            cuts.push((token.end.min(range.end), *kind));
        }
        let mut runs = Vec::new();
        let mut at = range.start;
        let push =
            |runs: &mut Vec<TextRun>, from: usize, to: usize, kind: Kind, underline: bool| {
                let len =
                    display.to_display(to - range.start) - display.to_display(from - range.start);
                if len == 0 {
                    return;
                }
                let color: Hsla = highlight::color(kind).into();
                runs.push(TextRun {
                    len,
                    font: font.clone(),
                    color,
                    background_color: None,
                    underline: underline.then_some(UnderlineStyle {
                        thickness: px(1.0),
                        color: Some(color),
                        wavy: false,
                    }),
                    strikethrough: None,
                });
            };
        for (end, kind) in cuts {
            let mut from = at;
            let mut stops: Vec<usize> = marked
                .iter()
                .flat_map(|marked| [marked.start, marked.end])
                .filter(|&cut| from < cut && cut < end)
                .collect();
            stops.push(end);
            for to in stops {
                let underline = marked
                    .as_ref()
                    .is_some_and(|marked| marked.start <= from && to <= marked.end);
                push(&mut runs, from, to, kind, underline);
                from = to;
            }
            at = end;
        }
        if at < range.end {
            push(&mut runs, at, range.end, Kind::Plain, false);
        }
        runs
    }

    fn lay(&mut self, line: usize, window: &Window) -> Laid {
        let metrics = self.metrics(window);
        let range = self.line_range(line);
        let display = Display::new(&self.text[range.clone()]);
        let runs = self.runs(range.clone(), &display, &metrics.font);
        let shaped = window.text_system().shape_line(
            SharedString::from(display.text.clone()),
            metrics.size,
            &runs,
            None,
        );
        Laid {
            start: range.start,
            end: range.end,
            display,
            shaped,
        }
    }

    fn offset_at(&mut self, position: Point<Pixels>, window: &Window) -> usize {
        let Some(geometry) = self.geometry else {
            return self.head;
        };
        if self.shown.rows.is_empty() {
            return 0;
        }
        let row = geometry
            .row_at(position.y)
            .clamp(0.0, (self.shown.rows.len() - 1) as f32) as usize;
        let line = self.shown.rows[row].line;
        if line >= self.starts.len() {
            return self.text.len();
        }
        let laid = self.lay(line, window);
        laid.offset_at(position.x - geometry.text_x())
    }

    fn cursor_bounds(&mut self, at: usize, window: &Window) -> Option<Bounds<Pixels>> {
        let geometry = self.geometry?;
        let line = self.line_of(at);
        let row = self
            .shown
            .rows
            .binary_search_by_key(&line, |row| row.line)
            .ok()?;
        let laid = self.lay(line, window);
        Some(Bounds::new(
            point(geometry.text_x() + laid.x(at), geometry.row_top(row)),
            size(px(2.0), geometry.row),
        ))
    }

    fn vertical(&mut self, rows: isize, select: bool, window: &Window, cx: &mut Context<Self>) {
        if self.shown.rows.is_empty() {
            return;
        }
        let line = self.line_of(self.head);
        let row = self.row_of(line);
        let laid = self.lay(line, window);
        let x = self.goal_x.unwrap_or_else(|| laid.x(self.head));
        let target = row as isize + rows;
        let last = self.shown.rows.len() as isize - 1;
        let at = if target < 0 {
            0
        } else if target > last {
            self.text.len()
        } else {
            let line = self.shown.rows[target as usize].line;
            self.lay(line, window).offset_at(x)
        };
        self.move_to(at, select, cx);
        self.goal_x = Some(x);
    }

    fn page_rows(&self) -> isize {
        let height = self.scroll.bounds().size.height;
        let row = self.metrics.as_ref().map_or(px(16.0), |m| m.row);
        ((height / row).floor() as isize).max(1)
    }

    fn horizontal(&mut self, right: bool, select: bool, cx: &mut Context<Self>) {
        let range = self.selection();
        let at = if !select && !range.is_empty() {
            if right { range.end } else { range.start }
        } else if right {
            self.next_char(self.head)
        } else {
            self.previous_char(self.head)
        };
        self.move_to(at, select, cx);
    }

    fn apply_reveal(&mut self, window: &Window) {
        let Some(reveal) = self.reveal.clone() else {
            return;
        };
        let viewport = self.scroll.bounds().size;
        if viewport.height <= px(0.0) || self.shown.rows.is_empty() {
            return;
        }
        self.reveal = None;
        let metrics = self.metrics(window);
        let (range, center) = match reveal {
            Reveal::Cursor => (self.head..self.head, false),
            Reveal::Center(range) => (range, true),
        };
        let range = range.start.min(self.text.len())..range.end.min(self.text.len());
        let line = self.line_of(range.start);
        let row = self.row_of(line);
        let laid = self.lay(line, window);
        let (gutter, _) = self.gutter_width(&metrics);
        let left = gutter + GUTTER_GAP + MARGIN_X;
        let x0 = left + laid.x(range.start);
        let x1 = left + laid.x(range.end) + px(2.0);
        let y0 = MARGIN_Y + metrics.row * row as f32;
        let y1 = y0 + metrics.row;
        let natural = self.natural_size(&metrics);
        let max = point(
            (natural.width.max(viewport.width) - viewport.width).max(px(0.0)),
            (natural.height.max(viewport.height) - viewport.height).max(px(0.0)),
        );
        let mut scroll = -self.scroll.offset();
        let fit = |start: Pixels, low: Pixels, high: Pixels, span: Pixels| {
            if center {
                (low + high - span) / 2.0
            } else if low < start {
                low
            } else if high > start + span {
                high - span
            } else {
                start
            }
        };
        scroll.y = fit(scroll.y, y0, y1, viewport.height).clamp(px(0.0), max.y);
        scroll.x = fit(scroll.x, x0, x1, viewport.width).clamp(px(0.0), max.x);
        self.scroll.set_offset(-scroll);
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.geometry.is_none() {
            return;
        }
        window.focus(&self.focus, cx);
        let at = self.offset_at(event.position, window);
        match event.click_count {
            1 => {
                self.drag = Some(Drag::Chars);
                self.move_to(at, event.modifiers.shift, cx);
            }
            2 => {
                let word = self.word_at(at);
                self.drag = Some(Drag::Words(word.clone()));
                self.anchor = word.start;
                self.head = word.end;
                self.after_move(cx);
            }
            _ => {
                let line = self.line_of(at);
                let range = self.line_range(line);
                let end = self.starts.get(line + 1).copied().unwrap_or(range.end);
                self.drag = Some(Drag::Lines(range.start..end));
                self.anchor = range.start;
                self.head = end;
                self.after_move(cx);
            }
        }
    }

    fn gutter_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(geometry) = self.geometry else {
            return;
        };
        let row = geometry.row_at(event.position.y);
        if row < 0.0 {
            return;
        }
        let Some(&Row { line, fold }) = self.shown.rows.get(row as usize) else {
            return;
        };
        let arrow_left = geometry.origin.x + geometry.gutter - geometry.arrow - MARGIN_X;
        if event.position.x >= arrow_left && fold != Fold::None {
            cx.emit(Event::Fold(line));
        } else {
            self.select_line(line, window, cx);
        }
    }

    fn mouse_drag(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.clone() else {
            return;
        };
        let viewport = self.scroll.bounds();
        let mut offset = self.scroll.offset();
        let row = self.metrics.as_ref().map_or(px(16.0), |m| m.row);
        if event.position.y < viewport.top() {
            offset.y = (offset.y + (viewport.top() - event.position.y).min(row * 2.0)).min(px(0.0));
        } else if event.position.y > viewport.bottom() {
            offset.y -= (event.position.y - viewport.bottom()).min(row * 2.0);
        }
        if event.position.x < viewport.left() {
            offset.x =
                (offset.x + (viewport.left() - event.position.x).min(row * 2.0)).min(px(0.0));
        } else if event.position.x > viewport.right() {
            offset.x -= (event.position.x - viewport.right()).min(row * 2.0);
        }
        if offset != self.scroll.offset() {
            self.scroll.set_offset(offset);
        }
        let at = self.offset_at(event.position, window);
        let (anchor, head) = match drag {
            Drag::Chars => (self.anchor, at),
            Drag::Words(word) => {
                let here = self.word_at(at);
                if at < word.start {
                    (word.end, here.start)
                } else {
                    (word.start, here.end.max(word.end))
                }
            }
            Drag::Lines(lines) => {
                let line = self.line_of(at);
                let start = self.starts[line];
                let end = self
                    .starts
                    .get(line + 1)
                    .copied()
                    .unwrap_or(self.text.len());
                if start < lines.start {
                    (lines.end, start)
                } else {
                    (lines.start, end.max(lines.end))
                }
            }
        };
        if (anchor, head) != (self.anchor, self.head) {
            self.anchor = anchor;
            self.head = head;
            self.group = None;
            self.goal_x = None;
            cx.emit(Event::Moved(self.head));
        }
        cx.notify();
    }

    fn hover(&mut self, row: Option<usize>, text: bool, cx: &mut Context<Self>) {
        if (row, text) != (self.hover_row, self.hover_text) {
            self.hover_row = row;
            self.hover_text = text;
            cx.notify();
        }
    }

    fn badges(&mut self, window: &Window, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let metrics = self.metrics(window);
        let (gutter, _) = self.gutter_width(&metrics);
        let left = gutter + GUTTER_GAP + MARGIN_X;
        let headers = self.shown.headers.clone();
        headers
            .into_iter()
            .filter_map(|(line, header_end)| {
                let row = self
                    .shown
                    .rows
                    .binary_search_by_key(&line, |row| row.line)
                    .ok()?;
                let laid = self.lay(line, window);
                let x = left + laid.x(header_end) + px(6.0);
                let y = MARGIN_Y + metrics.row * row as f32 + px(1.0);
                Some(
                    div()
                        .id(("fold-badge", line))
                        .absolute()
                        .left(x)
                        .top(y)
                        .w(metrics.digit * 2.0 + px(8.0))
                        .h(metrics.row - px(2.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(3.0))
                        .bg(theme::BG_HL)
                        .hover(|style| style.bg(theme::SEL))
                        .cursor_pointer()
                        .occlude()
                        .font_family(theme::MONO)
                        .text_size(metrics.size)
                        .text_color(theme::COMMENT)
                        .child("...")
                        .tooltip(widgets::tooltip("Click to unfold"))
                        .on_mouse_down(MouseButton::Left, |_, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                        })
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(Event::Fold(line));
                        }))
                        .into_any_element(),
                )
            })
            .collect()
    }
}

impl CodeEditor {
    fn move_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.horizontal(false, false, cx);
    }

    fn move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        self.horizontal(true, false, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.horizontal(false, true, cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.horizontal(true, true, cx);
    }

    fn move_up(&mut self, _: &MoveUp, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1, false, window, cx);
    }

    fn move_down(&mut self, _: &MoveDown, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1, false, window, cx);
    }

    fn select_up(&mut self, _: &SelectUp, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1, true, window, cx);
    }

    fn select_down(&mut self, _: &SelectDown, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1, true, window, cx);
    }

    fn page_up(&mut self, _: &PageUp, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-self.page_rows(), false, window, cx);
    }

    fn page_down(&mut self, _: &PageDown, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(self.page_rows(), false, window, cx);
    }

    fn select_page_up(&mut self, _: &SelectPageUp, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-self.page_rows(), true, window, cx);
    }

    fn select_page_down(
        &mut self,
        _: &SelectPageDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.vertical(self.page_rows(), true, window, cx);
    }

    fn word_left_action(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.word_left(self.head), false, cx);
    }

    fn word_right_action(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.word_right(self.head), false, cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.word_left(self.head), true, cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.word_right(self.head), true, cx);
    }

    fn line_start(&mut self, _: &LineStart, _: &mut Window, cx: &mut Context<Self>) {
        let start = self.line_range(self.line_of(self.head)).start;
        self.move_to(start, false, cx);
    }

    fn line_end(&mut self, _: &LineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let end = self.line_range(self.line_of(self.head)).end;
        self.move_to(end, false, cx);
    }

    fn select_line_start(&mut self, _: &SelectLineStart, _: &mut Window, cx: &mut Context<Self>) {
        let start = self.line_range(self.line_of(self.head)).start;
        self.move_to(start, true, cx);
    }

    fn select_line_end(&mut self, _: &SelectLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let end = self.line_range(self.line_of(self.head)).end;
        self.move_to(end, true, cx);
    }

    fn text_start(&mut self, _: &TextStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, false, cx);
    }

    fn text_end(&mut self, _: &TextEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.text.len(), false, cx);
    }

    fn select_text_start(&mut self, _: &SelectTextStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, true, cx);
    }

    fn select_text_end(&mut self, _: &SelectTextEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.text.len(), true, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.anchor = 0;
        self.head = self.text.len();
        self.after_move(cx);
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.selection();
        let range = if range.is_empty() {
            self.previous_char(self.head)..self.head
        } else {
            range
        };
        if !range.is_empty() {
            self.edit(range, "", Group::Delete, cx);
        }
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.selection();
        let range = if range.is_empty() {
            self.head..self.next_char(self.head)
        } else {
            range
        };
        if !range.is_empty() {
            self.edit(range, "", Group::Delete, cx);
        }
    }

    fn backspace_word(&mut self, _: &BackspaceWord, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.selection();
        let range = if range.is_empty() {
            self.word_left(self.head)..self.head
        } else {
            range
        };
        if !range.is_empty() {
            self.edit(range, "", Group::Other, cx);
        }
    }

    fn delete_word(&mut self, _: &DeleteWord, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.selection();
        let range = if range.is_empty() {
            self.head..self.word_right(self.head)
        } else {
            range
        };
        if !range.is_empty() {
            self.edit(range, "", Group::Other, cx);
        }
    }

    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        self.insert("\n", Group::Other, cx);
    }

    fn indent(&mut self, _: &Indent, _: &mut Window, cx: &mut Context<Self>) {
        self.insert("\t", Group::Other, cx);
    }

    fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        let selection = self.selection();
        let mut text = self.text.clone();
        text.replace_range(selection.clone(), "");
        let mut cursor = selection.start;
        let start = text[..cursor].rfind('\n').map_or(0, |at| at + 1);
        let rest = &text[start..];
        let remove = if rest.starts_with('\t') {
            1
        } else if rest.len() >= TAB_COLUMNS && rest[..TAB_COLUMNS].bytes().all(|b| b == b' ') {
            TAB_COLUMNS
        } else {
            0
        };
        if selection.is_empty() && remove == 0 {
            return;
        }
        text.replace_range(start..start + remove, "");
        if cursor != start {
            cursor -= remove;
        }
        self.push_undo();
        self.redo.clear();
        self.text = text;
        self.starts = line_starts(&self.text);
        self.anchor = cursor;
        self.head = cursor;
        self.marked = None;
        cx.emit(Event::Edited);
        self.after_move(cx);
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.selection();
        if !range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.text[range].to_owned()));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.selection();
        if !range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.text[range.clone()].to_owned(),
            ));
            self.edit(range, "", Group::Other, cx);
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.insert(&text, Group::Other, cx);
        }
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        let Some(snapshot) = self.undo.pop() else {
            return;
        };
        self.redo.push(Snapshot {
            text: std::mem::take(&mut self.text),
            anchor: self.anchor,
            head: self.head,
        });
        self.restore(snapshot, cx);
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        let Some(snapshot) = self.redo.pop() else {
            return;
        };
        self.undo.push(Snapshot {
            text: std::mem::take(&mut self.text),
            anchor: self.anchor,
            head: self.head,
        });
        self.restore(snapshot, cx);
    }
}

impl CodeEditor {
    fn utf16_to_offset(&self, utf16: usize) -> usize {
        let mut count = 0;
        for (at, c) in self.text.char_indices() {
            if count >= utf16 {
                return at;
            }
            count += c.len_utf16();
        }
        self.text.len()
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        self.text[..offset.min(self.text.len())]
            .chars()
            .map(char::len_utf16)
            .sum()
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.utf16_to_offset(range.start)..self.utf16_to_offset(range.end)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }
}

impl EntityInputHandler for CodeEditor {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range);
        adjusted_range.replace(self.range_to_utf16(&range));
        Some(self.text[range].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selection()),
            reversed: self.head < self.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        self.group = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|range| self.range_from_utf16(&range))
            .or(self.marked.take())
            .unwrap_or_else(|| self.selection());
        self.marked = None;
        let group = if self.group.is_some_and(|(group, _)| group == Group::Compose) {
            Group::Compose
        } else if !text.is_empty() && !text.contains('\n') && range.is_empty() {
            Group::Insert
        } else {
            Group::Other
        };
        self.edit(range, text, group, cx);
        if group == Group::Compose {
            self.group = None;
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|range| self.range_from_utf16(&range))
            .or(self.marked.clone())
            .unwrap_or_else(|| self.selection());
        self.edit(range.clone(), text, Group::Compose, cx);
        self.marked = (!text.is_empty()).then(|| range.start..range.start + text.len());
        if let Some(selected) = selected {
            let start = self.offset_to_utf16(range.start);
            let len: usize = text.chars().map(char::len_utf16).sum();
            let from = self.utf16_to_offset(start + selected.start.min(len));
            let to = self.utf16_to_offset(start + selected.end.min(len));
            self.anchor = from;
            self.head = to;
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range);
        let start = self.cursor_bounds(range.start, window)?;
        let end = self.cursor_bounds(range.end, window).unwrap_or(start);
        Some(Bounds::from_corners(
            start.origin,
            point(end.origin.x.max(start.origin.x), start.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let at = self.offset_at(point, window);
        Some(self.offset_to_utf16(at))
    }

    fn set_selected_text_range(
        &mut self,
        range: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.range_from_utf16(&range);
        self.anchor = range.start;
        self.head = range.end;
        self.after_move(cx);
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.offset_to_utf16(self.text.len()))
    }
}

impl Render for CodeEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let metrics = self.metrics(window);
        self.apply_reveal(window);
        let natural = self.natural_size(&metrics);
        let badges = self.badges(window, cx);
        div()
            .id("code-editor")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .on_action(cx.listener(Self::move_left))
            .on_action(cx.listener(Self::move_right))
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::word_left_action))
            .on_action(cx.listener(Self::word_right_action))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::line_start))
            .on_action(cx.listener(Self::line_end))
            .on_action(cx.listener(Self::select_line_start))
            .on_action(cx.listener(Self::select_line_end))
            .on_action(cx.listener(Self::text_start))
            .on_action(cx.listener(Self::text_end))
            .on_action(cx.listener(Self::select_text_start))
            .on_action(cx.listener(Self::select_text_end))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::select_page_up))
            .on_action(cx.listener(Self::select_page_down))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::backspace_word))
            .on_action(cx.listener(Self::delete_word))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::indent))
            .on_action(cx.listener(Self::outdent))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .child(
                div()
                    .id("code-scroll")
                    .size_full()
                    .overflow_scroll()
                    .track_scroll(&self.scroll)
                    .child(
                        div()
                            .relative()
                            .flex_none()
                            .w(natural.width)
                            .h(natural.height)
                            .min_w_full()
                            .min_h_full()
                            .child(CodeElement {
                                editor: cx.entity(),
                            })
                            .children(badges),
                    ),
            )
            .scrollbar(&self.scroll, ScrollbarAxis::Both)
    }
}

struct CodeElement {
    editor: Entity<CodeEditor>,
}

impl IntoElement for CodeElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

struct Painted {
    text: Hitbox,
    gutter: Hitbox,
    under: Vec<PaintQuad>,
    numbers: Vec<(ShapedLine, Point<Pixels>)>,
    arrows: Vec<gpui_kit::Path<Pixels>>,
    lines: Vec<(ShapedLine, Point<Pixels>)>,
    over: Vec<PaintQuad>,
    cursor: Option<PaintQuad>,
    row: Pixels,
}

fn arrow(center: Point<Pixels>, half: Pixels, folded: bool) -> Option<gpui_kit::Path<Pixels>> {
    let points = if folded {
        [
            center + point(-half * 0.6, -half),
            center + point(-half * 0.6, half),
            center + point(half * 0.9, px(0.0)),
        ]
    } else {
        [
            center + point(-half, -half * 0.6),
            center + point(half, -half * 0.6),
            center + point(px(0.0), half * 0.9),
        ]
    };
    let mut builder = PathBuilder::fill();
    builder.add_polygon(&points, true);
    builder.build().ok()
}

fn match_color(current: bool) -> Hsla {
    let (r, g, b, a) = if current {
        (255, 140, 0, 150)
    } else {
        (255, 220, 0, 70)
    };
    gpui_kit::Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: a as f32 / 255.0,
    }
    .into()
}

impl Element for CodeElement {
    type RequestLayoutState = ();
    type PrepaintState = Painted;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let focused = self.editor.read(cx).focus.is_focused(window);
        self.editor.update(cx, |editor, _| {
            let metrics = editor.metrics(window);
            let (gutter, arrow_width) = editor.gutter_width(&metrics);
            let text_left = gutter + GUTTER_GAP;
            let geometry = Geometry {
                origin: bounds.origin,
                gutter,
                arrow: arrow_width,
                text_left,
                text_width: bounds.size.width - text_left,
                row: metrics.row,
            };
            editor.geometry = Some(geometry);
            let clip = window.content_mask().bounds.intersect(&bounds);
            let gutter_bounds = Bounds::new(bounds.origin, size(gutter, bounds.size.height));
            let text_bounds = Bounds::new(
                point(bounds.origin.x + text_left, bounds.origin.y),
                size(geometry.text_width, bounds.size.height),
            );
            let gutter_hitbox =
                window.insert_hitbox(gutter_bounds.intersect(&clip), HitboxBehavior::Normal);
            let text_hitbox =
                window.insert_hitbox(text_bounds.intersect(&clip), HitboxBehavior::Normal);
            let rows = editor.shown.rows.len();
            let first = (geometry.row_at(clip.top()).max(0.0) as usize).min(rows);
            let last = ((geometry.row_at(clip.bottom()) + 1.0).max(0.0) as usize).min(rows);
            let mut under = vec![fill(gutter_bounds, theme::BG_FLOAT)];
            let border = if focused || editor.hover_text {
                theme::BLUE.into()
            } else {
                gpui_kit::transparent_black()
            };
            under.push(
                fill(text_bounds, theme::BG_DARK)
                    .border_widths(px(1.0))
                    .border_color(border),
            );
            if let Some(row) = editor.hover_row.filter(|&row| row < rows) {
                let top = geometry.row_top(row);
                under.push(fill(
                    Bounds::new(point(bounds.origin.x, top), size(gutter, metrics.row)),
                    theme::SEL,
                ));
                under.push(fill(
                    Bounds::new(
                        point(text_bounds.origin.x, top),
                        size(geometry.text_width, metrics.row),
                    ),
                    theme::alpha(theme::SEL, 0.4),
                ));
            }
            let number_right = bounds.origin.x + gutter - arrow_width - MARGIN_X * 2.0;
            let selection = editor.selection();
            let mut numbers = Vec::new();
            let mut arrows = Vec::new();
            let mut lines = Vec::new();
            let mut over = Vec::new();
            let mut cursor = None;
            let head = editor.head;
            let matches = editor.matches.clone();
            let current = editor.current;
            for row in first..last {
                let Row { line, fold } = editor.shown.rows[row];
                if line >= editor.starts.len() {
                    continue;
                }
                let top = geometry.row_top(row);
                let color = if editor.hover_row == Some(row) {
                    theme::FG_DARK
                } else {
                    theme::COMMENT
                };
                let number = (line + 1).to_string();
                let shaped = window.text_system().shape_line(
                    number.clone().into(),
                    metrics.size,
                    &[TextRun {
                        len: number.len(),
                        font: metrics.font.clone(),
                        color: color.into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                );
                numbers.push((shaped.clone(), point(number_right - shaped.width(), top)));
                if fold != Fold::None {
                    let center = point(
                        bounds.origin.x + gutter - arrow_width / 2.0,
                        top + metrics.row / 2.0,
                    );
                    arrows.extend(arrow(center, metrics.row * 0.3, fold == Fold::Folded));
                }
                let laid = editor.lay(line, window);
                let origin = point(geometry.text_x(), top);
                let line_end = editor.starts.get(line + 1).copied();
                if !selection.is_empty()
                    && selection.start <= laid.end
                    && selection.end > laid.start
                {
                    let x0 = laid.x(selection.start.max(laid.start));
                    let mut x1 = laid.x(selection.end.min(laid.end));
                    if line_end.is_some_and(|end| selection.end >= end) && fold != Fold::Folded {
                        x1 += metrics.digit;
                    }
                    under.push(fill(
                        Bounds::from_corners(
                            point(origin.x + x0, top),
                            point(origin.x + x1, top + metrics.row),
                        ),
                        theme::SEL,
                    ));
                }
                for (i, found) in matches.iter().enumerate() {
                    if found.start < laid.start || found.end > laid.end {
                        continue;
                    }
                    let x0 = laid.x(found.start);
                    let x1 = laid.x(found.end);
                    over.push(
                        fill(
                            Bounds::from_corners(
                                point(origin.x + x0, top),
                                point(origin.x + x1, top + metrics.row),
                            ),
                            match_color(current == Some(i)),
                        )
                        .corner_radii(px(2.0)),
                    );
                }
                if focused && head >= laid.start && head <= laid.end {
                    cursor = Some(fill(
                        Bounds::new(
                            point(origin.x + laid.x(head), top),
                            size(px(2.0), metrics.row),
                        ),
                        theme::FG,
                    ));
                }
                lines.push((laid.shaped, origin));
            }
            Painted {
                text: text_hitbox,
                gutter: gutter_hitbox,
                under,
                numbers,
                arrows,
                lines,
                over,
                cursor,
                row: metrics.row,
            }
        })
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        painted: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.editor.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );
        window.set_cursor_style(CursorStyle::IBeam, &painted.text);
        window.set_cursor_style(CursorStyle::PointingHand, &painted.gutter);
        for quad in painted.under.drain(..) {
            window.paint_quad(quad);
        }
        for (shaped, origin) in &painted.numbers {
            shaped
                .paint(*origin, painted.row, TextAlign::Left, None, window, cx)
                .ok();
        }
        for path in painted.arrows.drain(..) {
            window.paint_path(path, theme::COMMENT);
        }
        for (shaped, origin) in &painted.lines {
            shaped
                .paint(*origin, painted.row, TextAlign::Left, None, window, cx)
                .ok();
        }
        for quad in painted.over.drain(..) {
            window.paint_quad(quad);
        }
        if let Some(cursor) = painted.cursor.take() {
            window.paint_quad(cursor);
        }

        let (text, gutter) = (painted.text.clone(), painted.gutter.clone());
        let editor = self.editor.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if gutter.is_hovered(window) {
                editor.update(cx, |editor, cx| editor.gutter_down(event, window, cx));
                window.prevent_default();
                cx.stop_propagation();
            } else if text.is_hovered(window) {
                editor.update(cx, |editor, cx| editor.mouse_down(event, window, cx));
                window.prevent_default();
                cx.stop_propagation();
            }
        });
        let (text, gutter) = (painted.text.clone(), painted.gutter.clone());
        let editor = self.editor.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            editor.update(cx, |editor, cx| {
                if editor.drag.is_some() && event.pressed_button == Some(MouseButton::Left) {
                    editor.mouse_drag(event, window, cx);
                }
                let row = gutter
                    .is_hovered(window)
                    .then(|| editor.geometry.map(|g| g.row_at(event.position.y)))
                    .flatten()
                    .filter(|&row| row >= 0.0 && (row as usize) < editor.shown.rows.len())
                    .map(|row| row as usize);
                editor.hover(row, text.is_hovered(window), cx);
            });
        });
        let editor = self.editor.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                editor.update(cx, |editor, _| editor.drag = None);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_map_between_buffer_and_display() {
        let display = Display::new("\tab\tc");
        assert_eq!(display.text, "    ab    c");
        assert_eq!(display.to_display(0), 0);
        assert_eq!(display.to_display(1), 4);
        assert_eq!(display.to_display(4), 10);
        assert_eq!(display.to_buffer(0), 0);
        assert_eq!(display.to_buffer(1), 0);
        assert_eq!(display.to_buffer(3), 1);
        assert_eq!(display.to_buffer(5), 2);
        assert_eq!(display.to_buffer(10), 4);
        assert_eq!(display.to_buffer(11), 5);
    }

    #[test]
    fn line_starts_follow_newlines() {
        assert_eq!(line_starts("a\nbc\n"), [0, 2, 5]);
        assert_eq!(line_starts(""), [0]);
    }

    fn shown(lines: usize, foldable: usize) -> Shown {
        Shown {
            rows: (0..lines)
                .map(|line| Row {
                    line,
                    fold: if line == foldable {
                        Fold::Open
                    } else {
                        Fold::None
                    },
                })
                .collect(),
            headers: Vec::new(),
            lines,
            longest: 20,
        }
    }

    #[test]
    fn edits_undo_and_answers_the_gutter() {
        use gpui_kit::test::{TestEventExt, TestWindowExt};
        use gpui_kit::{AppContext, HeadlessAppContext, WindowBounds, WindowOptions};
        use std::{cell::RefCell, rc::Rc, sync::Arc};

        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(gpui_kit::assets::Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            theme::install(cx, theme::UiFont::default());
        });
        let text = "local x = 1\nfunction f()\n\tx()\nend";
        let (handle, editor) = cx
            .update(|cx| {
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds {
                            origin: Point::default(),
                            size: size(px(600.0), px(400.0)),
                        })),
                        focus: false,
                        show: false,
                        ..Default::default()
                    },
                    cx,
                    |_, cx| cx.new(|cx| CodeEditor::new(text, cx)),
                )
            })
            .unwrap();
        let folds = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let folds = folds.clone();
            cx.subscribe(&editor, move |_, event: &Event, _| {
                if let Event::Fold(line) = event {
                    folds.borrow_mut().push(*line);
                }
            })
            .detach();
            editor.update(cx, |editor, cx| editor.set_shown(shown(4, 1), cx));
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let geometry = editor.read(cx).geometry.expect("painted");
            let row_y = |row: usize| geometry.row_top(row) + geometry.row / 2.0;
            let text_point = point(geometry.text_x() + px(200.0), row_y(0));
            window.pointer_down(text_point, MouseButton::Left, cx);
            window.pointer_up(text_point, MouseButton::Left, cx);
            assert_eq!(editor.read(cx).head, "local x = 1".len());
            window.input("2", cx);
            window.press("enter", cx);
            window.press("tab", cx);
            assert_eq!(editor.read(cx).text(), "local x = 12\n\t\nfunction f()\n\tx()\nend");
            window.press("shift-tab", cx);
            assert_eq!(editor.read(cx).text(), "local x = 12\n\nfunction f()\n\tx()\nend");
            window.press("ctrl-z", cx);
            assert_eq!(editor.read(cx).text(), "local x = 12\n\t\nfunction f()\n\tx()\nend");
            window.press("ctrl-z", cx);
            window.press("ctrl-z", cx);
            window.press("ctrl-z", cx);
            assert_eq!(editor.read(cx).text(), text);
            window.press("ctrl-shift-z", cx);
            assert_eq!(editor.read(cx).text(), "local x = 12\nfunction f()\n\tx()\nend");

            editor.update(cx, |editor, cx| {
                editor.load(text, cx);
                editor.set_shown(shown(4, 1), cx);
            });
            window.render_frame(cx);
            let number = point(geometry.origin.x + MARGIN_X, row_y(2));
            window.pointer_down(number, MouseButton::Left, cx);
            window.pointer_up(number, MouseButton::Left, cx);
            let line = text.find("\tx()").unwrap();
            assert_eq!(editor.read(cx).selection(), line..line + "\tx()".len());
            let arrow = point(
                geometry.origin.x + geometry.gutter - geometry.arrow / 2.0,
                row_y(1),
            );
            window.pointer_down(arrow, MouseButton::Left, cx);
            window.pointer_up(arrow, MouseButton::Left, cx);
            let no_arrow = point(arrow.x, row_y(0));
            window.pointer_down(no_arrow, MouseButton::Left, cx);
            window.pointer_up(no_arrow, MouseButton::Left, cx);
            assert_eq!(editor.read(cx).selection(), 0.."local x = 1".len());
        })
        .unwrap();
        assert_eq!(*folds.borrow(), [1]);
    }
}
