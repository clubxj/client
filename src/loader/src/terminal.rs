use std::{
    cell::RefCell,
    io::{Read, Write},
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use alacritty_terminal::{
    event::{Event, EventListener},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionType},
    term::{Config, Term, TermMode, cell::Flags, color::Colors},
    vte::ansi::{Color, CursorShape, NamedColor, Processor, Rgb},
};
use gpui_kit::{
    AnyElement, App, BorderStyle, Bounds, ClipboardItem, ContentMask, Context, DispatchPhase,
    FocusHandle, Font, FontFeatures, FontWeight, Hsla, InputHandler, InteractiveElement,
    IntoElement, KeyDownEvent, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PaintQuad, ParentElement, Pixels, Rgba, ScrollEvent, ShapedLine,
    Styled, TextAlign, TextRun, UTF16Selection, UnderlineStyle,
    Window, canvas, div, fill, font, outline, point, prelude::FluentBuilder, px, size,
};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::ui::{theme, widgets};
use crate::wake::Wake;

const HISTORY: usize = 10_000;
const FONT_SIZE: Pixels = px(14.0);
const EXITED: &str = "[the shell exited: press Enter to start a new one]";

const ANSI: [[u8; 3]; 16] = [
    [0x15, 0x16, 0x1e],
    [0xf7, 0x76, 0x8e],
    [0x9e, 0xce, 0x6a],
    [0xe0, 0xaf, 0x68],
    [0x7a, 0xa2, 0xf7],
    [0xbb, 0x9a, 0xf7],
    [0x7d, 0xcf, 0xff],
    [0xa9, 0xb1, 0xd6],
    [0x41, 0x48, 0x68],
    [0xf7, 0x76, 0x8e],
    [0x9e, 0xce, 0x6a],
    [0xe0, 0xaf, 0x68],
    [0x7a, 0xa2, 0xf7],
    [0xbb, 0x9a, 0xf7],
    [0x7d, 0xcf, 0xff],
    [0xc0, 0xca, 0xf5],
];
const FG: [u8; 3] = rgb_of(theme::FG);
const BG: [u8; 3] = rgb_of(theme::BG_DARK);

const fn rgb_of(color: Rgba) -> [u8; 3] {
    [
        (color.r * 255.0 + 0.5) as u8,
        (color.g * 255.0 + 0.5) as u8,
        (color.b * 255.0 + 0.5) as u8,
    ]
}

fn default_rgb(index: usize) -> Rgb {
    let [r, g, b] = match index {
        0..=15 => ANSI[index],
        16..=231 => {
            let n = (index - 16) as u8;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            [level(n / 36), level(n / 6 % 6), level(n % 6)]
        }
        232..=255 => {
            let v = 8 + (index - 232) as u8 * 10;
            [v, v, v]
        }
        257 => BG,
        259..=266 => ANSI[index - 259].map(|v| (f32::from(v) * 0.66) as u8),
        268 => FG.map(|v| (f32::from(v) * 0.66) as u8),
        _ => FG,
    };
    Rgb { r, g, b }
}

fn pty_size(size: Size) -> PtySize {
    PtySize {
        rows: size.rows as u16,
        cols: size.cols as u16,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn hsla(Rgb { r, g, b }: Rgb) -> Hsla {
    Rgba {
        r: f32::from(r) / 255.0,
        g: f32::from(g) / 255.0,
        b: f32::from(b) / 255.0,
        a: 1.0,
    }
    .into()
}

fn rgb([r, g, b]: [u8; 3]) -> Hsla {
    hsla(Rgb { r, g, b })
}

fn resolve(color: Color, colors: &Colors) -> Hsla {
    let index = match color {
        Color::Spec(rgb) => return hsla(rgb),
        Color::Indexed(i) => usize::from(i),
        Color::Named(named) => named as usize,
    };
    hsla(colors[index].unwrap_or_else(|| default_rgb(index)))
}

type Writer = Arc<Mutex<Box<dyn Write + Send>>>;
type Screen = Arc<Mutex<Term<Proxy>>>;

fn send(writer: &Writer, bytes: &[u8]) {
    if let Ok(mut writer) = writer.lock() {
        let _ = writer.write_all(bytes).and_then(|()| writer.flush());
    }
}

fn type_into(term: &Screen, writer: &Writer, bytes: &[u8]) {
    if let Ok(mut term) = term.lock() {
        term.scroll_display(Scroll::Bottom);
        term.selection = None;
    }
    send(writer, bytes);
}

#[derive(Clone)]
struct Proxy {
    writer: Writer,
    clipboard: Arc<Mutex<Option<String>>>,
    wake: Wake,
}

impl EventListener for Proxy {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(text) => send(&self.writer, text.as_bytes()),
            Event::ColorRequest(index, format) => {
                send(&self.writer, format(default_rgb(index)).as_bytes())
            }
            Event::ClipboardStore(_, text) => {
                if let Ok(mut clipboard) = self.clipboard.lock() {
                    *clipboard = Some(text);
                }
                self.wake.wake();
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

struct Session {
    term: Screen,
    writer: Writer,
    clipboard: Arc<Mutex<Option<String>>>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    exited: Arc<AtomicBool>,
    size: Size,
}

impl Session {
    fn spawn(wake: &Wake, program: &str, size: Size, dir: Option<&Path>) -> Result<Self, String> {
        let pair = native_pty_system()
            .openpty(pty_size(size))
            .map_err(|e| e.to_string())?;
        let mut command = CommandBuilder::new(program);
        if let Some(dir) = dir
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
        {
            command.cwd(dir);
        }
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| format!("{program}: {e}"))?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer: Writer = Arc::new(Mutex::new(
            pair.master.take_writer().map_err(|e| e.to_string())?,
        ));
        let clipboard = Arc::new(Mutex::new(None));
        let term = Self::screen(&writer, &clipboard, wake, size);
        let exited = Arc::new(AtomicBool::new(false));
        {
            let (term, exited, wake) = (term.clone(), exited.clone(), wake.clone());
            thread::spawn(move || {
                let mut processor: Processor = Processor::new();
                let mut buf = [0u8; 8192];
                while let Ok(n @ 1..) = reader.read(&mut buf) {
                    if let Ok(mut term) = term.lock() {
                        processor.advance(&mut *term, &buf[..n]);
                    }
                    wake.wake();
                }
                exited.store(true, Ordering::Relaxed);
                wake.wake();
            });
        }
        Ok(Self {
            term,
            writer,
            clipboard,
            master: pair.master,
            child,
            exited,
            size,
        })
    }

    fn screen(
        writer: &Writer,
        clipboard: &Arc<Mutex<Option<String>>>,
        wake: &Wake,
        size: Size,
    ) -> Screen {
        let proxy = Proxy {
            writer: writer.clone(),
            clipboard: clipboard.clone(),
            wake: wake.clone(),
        };
        Arc::new(Mutex::new(Term::new(
            Config {
                scrolling_history: HISTORY,
                ..Config::default()
            },
            &size,
            proxy,
        )))
    }

    fn write(&self, bytes: &[u8]) {
        send(&self.writer, bytes);
    }

    fn type_in(&self, bytes: &[u8]) {
        type_into(&self.term, &self.writer, bytes);
    }

    fn mode(&self) -> TermMode {
        self.term
            .lock()
            .map(|t| *t.mode())
            .unwrap_or(TermMode::NONE)
    }

    fn exited(&self) -> bool {
        self.exited.load(Ordering::Relaxed)
    }

    fn resize(&mut self, size: Size) {
        if size == self.size {
            return;
        }
        self.size = size;
        if let Ok(mut term) = self.term.lock() {
            term.resize(size);
        }
        let _ = self.master.resize(pty_size(size));
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[derive(Clone)]
struct Metrics {
    regular: Font,
    bold: Font,
    cell: gpui_kit::Size<Pixels>,
}

impl Metrics {
    fn new(window: &Window) -> Self {
        let regular = Font {
            features: FontFeatures::disable_ligatures(),
            ..font(theme::MONO)
        };
        let bold = Font {
            weight: FontWeight::BOLD,
            ..regular.clone()
        };
        let text = window.text_system();
        let id = text.resolve_font(&regular);
        let width = text
            .advance(id, FONT_SIZE, 'M')
            .map_or(FONT_SIZE * 0.6, |advance| advance.width)
            .max(px(1.0));
        let height = (text.ascent(id, FONT_SIZE) + text.descent(id, FONT_SIZE).abs())
            .ceil()
            .max(px(1.0));
        Self {
            regular,
            bold,
            cell: size(width, height),
        }
    }

    fn grid_size(&self, bounds: Bounds<Pixels>) -> Size {
        Size {
            cols: ((bounds.size.width / self.cell.width) as usize).max(2),
            rows: ((bounds.size.height / self.cell.height) as usize).max(2),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Grid {
    bounds: Bounds<Pixels>,
    cell: gpui_kit::Size<Pixels>,
}

impl Grid {
    fn cell_at(&self, position: gpui_kit::Point<Pixels>, size: Size) -> (usize, usize, Side) {
        let x = (position.x - self.bounds.origin.x) / self.cell.width;
        let y = (position.y - self.bounds.origin.y) / self.cell.height;
        let col = (x.max(0.0) as usize).min(size.cols - 1);
        let row = (y.max(0.0) as usize).min(size.rows - 1);
        let side = if x - col as f32 > 0.5 {
            Side::Right
        } else {
            Side::Left
        };
        (row, col, side)
    }
}

pub type Lens<V> = fn(&mut V) -> Option<&mut Terminal>;

pub struct Terminal {
    session: Result<Session, String>,
    dir: Option<PathBuf>,
    wake: Wake,
    focus: Option<FocusHandle>,
    focus_pending: bool,
    pressed: bool,
    selecting: bool,
    scroll: f32,
    grid: Grid,
    marked: Rc<RefCell<String>>,
}

impl Terminal {
    pub fn new(wake: Wake, dir: Option<PathBuf>) -> Self {
        Self::with_session(Self::start(&wake, dir.as_deref()), dir, wake)
    }

    fn with_session(session: Result<Session, String>, dir: Option<PathBuf>, wake: Wake) -> Self {
        Self {
            session,
            dir,
            wake,
            focus: None,
            focus_pending: true,
            pressed: false,
            selecting: false,
            scroll: 0.0,
            grid: Grid::default(),
            marked: Rc::default(),
        }
    }

    fn start(wake: &Wake, dir: Option<&Path>) -> Result<Session, String> {
        Session::spawn(wake, &shell(), Size { cols: 80, rows: 24 }, dir)
    }

    pub fn close(&mut self) {
        self.session = Err("closed".to_owned());
    }

    pub fn render<V: 'static>(
        &mut self,
        lens: Lens<V>,
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> AnyElement {
        let focus = self.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        let session = match &self.session {
            Ok(session) => session,
            Err(error) => return failure(error, lens, cx),
        };
        if let Some(text) = session.clipboard.lock().ok().and_then(|mut c| c.take()) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
        if std::mem::take(&mut self.focus_pending) {
            window.focus(&focus, cx);
        }
        let focused = focus.is_focused(window);
        let reporting = reports_mouse(session.mode(), window.modifiers());
        let metrics = Metrics::new(window);
        let term = session.term.clone();
        let writer = session.writer.clone();
        let exited = session.exited();
        let marked = self.marked.clone();
        let owner = cx.weak_entity();
        let tracked = focus.clone();

        let prepaint = {
            let (term, marked, owner, metrics) =
                (term.clone(), marked.clone(), owner.clone(), metrics.clone());
            move |bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App| {
                let grid_size = metrics.grid_size(bounds);
                owner
                    .update(cx, |owner, _| {
                        if let Some(terminal) = lens(owner) {
                            terminal.grid = Grid {
                                bounds,
                                cell: metrics.cell,
                            };
                            if let Ok(session) = &mut terminal.session {
                                session.resize(grid_size);
                            }
                        }
                    })
                    .ok();
                let mut frame = Frame::new(bounds, &metrics);
                if let Ok(term) = term.lock() {
                    frame.lay_out(&term, focused, window);
                }
                frame.notes(exited, &marked.borrow(), window);
                frame
            }
        };
        let paint =
            move |bounds: Bounds<Pixels>, frame: Frame, window: &mut Window, cx: &mut App| {
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    frame.paint(window, cx);
                });
                window.handle_input(
                    &focus,
                    Typing {
                        term,
                        writer,
                        marked,
                        cursor: frame.cursor,
                    },
                    cx,
                );
                let moved = owner.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble {
                        moved
                            .update(cx, |owner, cx| {
                                if let Some(terminal) = lens(owner) {
                                    terminal.mouse_moved(event, cx);
                                }
                            })
                            .ok();
                    }
                });
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                        owner
                            .update(cx, |owner, cx| {
                                if let Some(terminal) = lens(owner) {
                                    terminal.mouse_released(event, cx);
                                }
                            })
                            .ok();
                    }
                });
            };

        div()
            .id("terminal")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .bg(rgb(BG))
            .track_focus(&tracked)
            .when(!reporting, |this| this.cursor_text())
            .on_key_down(cx.listener(move |owner, event: &KeyDownEvent, _, cx| {
                if let Some(terminal) = lens(owner) {
                    terminal.key_down(event, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |owner, event: &MouseDownEvent, _, cx| {
                    if let Some(terminal) = lens(owner) {
                        terminal.mouse_pressed(event, cx);
                    }
                }),
            )
            .on_scroll(cx.listener(move |owner, event: &ScrollEvent, _, cx| {
                if let Some(terminal) = lens(owner) {
                    terminal.wheel(event, cx);
                }
            }))
            .child(canvas(prepaint, paint).flex_1().min_h_0().w_full())
            .into_any_element()
    }

    fn key_down<V: 'static>(&mut self, event: &KeyDownEvent, cx: &mut Context<V>) {
        let Ok(session) = &self.session else {
            return;
        };
        let keystroke = &event.keystroke;
        let (key, m) = (keystroke.key.as_str(), keystroke.modifiers);
        let mode = session.mode();
        if session.exited() && key == "enter" {
            let size = session.size;
            self.session = Session::spawn(&self.wake, &shell(), size, self.dir.as_deref());
        } else if m.control && m.shift && key == "c" {
            if let Some(text) = session
                .term
                .lock()
                .ok()
                .and_then(|t| t.selection_to_string())
            {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        } else if is_paste(key, m) {
            let text = cx
                .read_from_clipboard()
                .and_then(|item| item.text())
                .map(|text| text.replace("\r\n", "\n"))
                .filter(|text| !text.is_empty());
            if let Some(text) = text {
                session.type_in(&paste_bytes(&text, mode));
            }
        } else if let Some(bytes) = key_bytes(key, m, mode).or_else(|| alt_text(keystroke)) {
            session.type_in(&bytes);
        } else {
            return;
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn mouse_pressed<V: 'static>(&mut self, event: &MouseDownEvent, cx: &mut Context<V>) {
        let Ok(session) = &self.session else {
            return;
        };
        let mode = session.mode();
        let (row, col, side) = self.grid.cell_at(event.position, session.size);
        self.pressed = true;
        if reports_mouse(mode, event.modifiers) {
            session.write(&mouse_report(mode, 0, event.modifiers, row, col, false));
        } else if let Ok(mut term) = session.term.lock() {
            let offset = term.grid().display_offset() as i32;
            let point = Point::new(Line(row as i32 - offset), Column(col));
            let kind = match event.click_count {
                0 | 1 => SelectionType::Simple,
                2 => SelectionType::Semantic,
                _ => SelectionType::Lines,
            };
            term.selection = Some(Selection::new(kind, point, side));
            self.selecting = true;
        }
        cx.notify();
    }

    fn mouse_moved<V: 'static>(&mut self, event: &MouseMoveEvent, cx: &mut Context<V>) {
        if !self.pressed {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.pressed = false;
            self.selecting = false;
            return;
        }
        let Ok(session) = &self.session else {
            return;
        };
        let mode = session.mode();
        let (row, col, side) = self.grid.cell_at(event.position, session.size);
        if reports_mouse(mode, event.modifiers) {
            if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) {
                session.write(&mouse_report(mode, 32, event.modifiers, row, col, false));
            }
        } else if self.selecting
            && let Ok(mut term) = session.term.lock()
        {
            let offset = term.grid().display_offset() as i32;
            let point = Point::new(Line(row as i32 - offset), Column(col));
            if let Some(selection) = term.selection.as_mut() {
                selection.update(point, side);
                cx.notify();
            }
        }
    }

    fn mouse_released<V: 'static>(&mut self, event: &MouseUpEvent, cx: &mut Context<V>) {
        let pressed = std::mem::take(&mut self.pressed);
        self.selecting = false;
        let Ok(session) = &self.session else {
            return;
        };
        let mode = session.mode();
        let over = self.grid.bounds.contains(&event.position);
        if reports_mouse(mode, event.modifiers) && (pressed || over) {
            let (row, col, _) = self.grid.cell_at(event.position, session.size);
            session.write(&mouse_report(mode, 0, event.modifiers, row, col, true));
            cx.notify();
        }
    }

    fn wheel<V: 'static>(&mut self, event: &ScrollEvent, cx: &mut Context<V>) {
        let Ok(session) = &self.session else {
            return;
        };
        let cell_h = self.grid.cell.height;
        if cell_h <= px(0.0) {
            return;
        }
        self.scroll += event.delta.pixel_delta(cell_h).y / cell_h;
        let lines = self.scroll.trunc() as i32;
        self.scroll -= lines as f32;
        if lines == 0 {
            return;
        }
        let mode = session.mode();
        if reports_mouse(mode, event.modifiers) {
            let (row, col, _) = self.grid.cell_at(event.position, session.size);
            let button = if lines > 0 { 64 } else { 65 };
            for _ in 0..lines.unsigned_abs() {
                session.write(&mouse_report(
                    mode,
                    button,
                    Modifiers::none(),
                    row,
                    col,
                    false,
                ));
            }
        } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            let arrow = if lines > 0 { "up" } else { "down" };
            if let Some(bytes) = key_bytes(arrow, Modifiers::none(), mode) {
                for _ in 0..lines.unsigned_abs() {
                    session.write(&bytes);
                }
            }
        } else if let Ok(mut term) = session.term.lock() {
            term.scroll_display(Scroll::Delta(lines));
        }
        cx.stop_propagation();
        cx.notify();
    }
}

fn failure<V: 'static>(error: &str, lens: Lens<V>, cx: &mut Context<V>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .w_full()
        .items_start()
        .gap(theme::LINE_GAP)
        .p(px(8.0))
        .bg(rgb(BG))
        .child(widgets::error(format!("Couldn't start a shell: {error}")))
        .child(widgets::button(
            "terminal-retry",
            "Try again",
            true,
            cx.listener(move |owner, _, _, cx| {
                if let Some(terminal) = lens(owner) {
                    terminal.session = Terminal::start(&terminal.wake, terminal.dir.as_deref());
                    terminal.focus_pending = true;
                }
                cx.notify();
            }),
        ))
        .into_any_element()
}

fn reports_mouse(mode: TermMode, modifiers: Modifiers) -> bool {
    mode.intersects(TermMode::MOUSE_MODE) && !modifiers.shift
}

fn is_paste(key: &str, m: Modifiers) -> bool {
    key == "paste" || (m.control && key == "v") || (cfg!(windows) && m.shift && key == "insert")
}

fn paste_bytes(text: &str, mode: TermMode) -> Vec<u8> {
    if mode.contains(TermMode::BRACKETED_PASTE) {
        format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', ""))
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r")
    }
    .into_bytes()
}

fn alt_text(keystroke: &gpui_kit::Keystroke) -> Option<Vec<u8>> {
    let m = keystroke.modifiers;
    if !m.alt || m.control || m.platform {
        return None;
    }
    let text = keystroke
        .key_char
        .clone()
        .or_else(|| (keystroke.key.chars().count() == 1).then(|| keystroke.key.clone()))?;
    Some([b"\x1b".as_slice(), text.as_bytes()].concat())
}

struct Typing {
    term: Screen,
    writer: Writer,
    marked: Rc<RefCell<String>>,
    cursor: Bounds<Pixels>,
}

impl InputHandler for Typing {
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut App,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(&mut self, _: &mut Window, _: &mut App) -> Option<Range<usize>> {
        let marked = self.marked.borrow();
        (!marked.is_empty()).then(|| 0..marked.encode_utf16().count())
    }

    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<String> {
        None
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        _: &mut App,
    ) {
        self.marked.borrow_mut().clear();
        type_into(&self.term, &self.writer, text.as_bytes());
        window.refresh();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        new_text: &str,
        _: Option<Range<usize>>,
        window: &mut Window,
        _: &mut App,
    ) {
        *self.marked.borrow_mut() = new_text.to_owned();
        window.refresh();
    }

    fn unmark_text(&mut self, window: &mut Window, _: &mut App) {
        self.marked.borrow_mut().clear();
        window.refresh();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<Bounds<Pixels>> {
        let mut bounds = self.cursor;
        bounds.origin.x += bounds.size.width * range.start as f32;
        Some(bounds)
    }

    fn character_index_for_point(
        &mut self,
        _: gpui_kit::Point<Pixels>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<usize> {
        None
    }
}

struct Frame {
    origin: gpui_kit::Point<Pixels>,
    bottom: Pixels,
    metrics: Metrics,
    fills: Vec<(i32, usize, usize, Hsla)>,
    lines: Vec<(gpui_kit::Point<Pixels>, ShapedLine)>,
    marks: Vec<PaintQuad>,
    notes: Vec<(Option<PaintQuad>, gpui_kit::Point<Pixels>, ShapedLine)>,
    cursor: Bounds<Pixels>,
    segment: Segment,
}

#[derive(Default)]
struct Segment {
    row: i32,
    col: usize,
    end: usize,
    text: String,
    runs: Vec<(usize, Hsla, bool)>,
}

impl Frame {
    fn new(bounds: Bounds<Pixels>, metrics: &Metrics) -> Self {
        Self {
            origin: bounds.origin,
            bottom: bounds.bottom(),
            metrics: metrics.clone(),
            fills: Vec::new(),
            lines: Vec::new(),
            marks: Vec::new(),
            notes: Vec::new(),
            cursor: Bounds::new(bounds.origin, metrics.cell),
            segment: Segment::default(),
        }
    }

    fn cell(&self, row: i32, col: usize, cells: usize) -> Bounds<Pixels> {
        let cell = self.metrics.cell;
        Bounds::new(
            self.origin + point(cell.width * col as f32, cell.height * row as f32),
            size(cell.width * cells as f32, cell.height),
        )
    }

    fn fill(&mut self, row: i32, col: usize, cells: usize, color: Hsla) {
        if let Some(last) = self.fills.last_mut()
            && last.0 == row
            && last.2 == col
            && last.3 == color
        {
            last.2 = col + cells;
            return;
        }
        self.fills.push((row, col, col + cells, color));
    }

    fn run(&self, runs: &[(usize, Hsla, bool)], underline: bool) -> Vec<TextRun> {
        runs.iter()
            .map(|&(len, color, bold)| TextRun {
                len,
                font: if bold {
                    self.metrics.bold.clone()
                } else {
                    self.metrics.regular.clone()
                },
                color,
                background_color: None,
                underline: underline.then_some(UnderlineStyle {
                    thickness: px(1.0),
                    color: Some(color),
                    wavy: false,
                }),
                strikethrough: None,
            })
            .collect()
    }

    fn shape(&self, text: String, runs: &[TextRun], window: &Window, forced: bool) -> ShapedLine {
        window.text_system().shape_line(
            text.into(),
            FONT_SIZE,
            runs,
            forced.then_some(self.metrics.cell.width),
        )
    }

    fn push_char(
        &mut self,
        row: i32,
        col: usize,
        c: char,
        color: Hsla,
        bold: bool,
        window: &Window,
    ) {
        if self.segment.row != row || self.segment.end != col || self.segment.text.is_empty() {
            self.flush(window);
            self.segment.row = row;
            self.segment.col = col;
            self.segment.end = col;
        }
        let segment = &mut self.segment;
        segment.text.push(c);
        segment.end += 1;
        match segment.runs.last_mut() {
            Some(last) if last.1 == color && last.2 == bold => last.0 += c.len_utf8(),
            _ => segment.runs.push((c.len_utf8(), color, bold)),
        }
    }

    fn flush(&mut self, window: &Window) {
        let mut segment = std::mem::take(&mut self.segment);
        while segment.text.ends_with(' ') {
            segment.text.pop();
            if let Some(last) = segment.runs.last_mut() {
                last.0 -= 1;
                if last.0 == 0 {
                    segment.runs.pop();
                }
            }
        }
        if segment.text.is_empty() {
            return;
        }
        let runs = self.run(&segment.runs, false);
        let origin = self.cell(segment.row, segment.col, 1).origin;
        let line = self.shape(segment.text, &runs, window, true);
        self.lines.push((origin, line));
    }

    fn wide_char(
        &mut self,
        row: i32,
        col: usize,
        c: char,
        color: Hsla,
        bold: bool,
        window: &Window,
    ) {
        self.flush(window);
        let text = c.to_string();
        let runs = self.run(&[(text.len(), color, bold)], false);
        let origin = self.cell(row, col, 1).origin;
        let line = self.shape(text, &runs, window, false);
        self.lines.push((origin, line));
    }

    fn lay_out(&mut self, term: &Term<Proxy>, focused: bool, window: &Window) {
        let content = term.renderable_content();
        let offset = content.display_offset as i32;
        let cursor = (content.mode.contains(TermMode::SHOW_CURSOR)
            && content.cursor.shape != CursorShape::Hidden
            && offset == 0)
            .then_some(content.cursor);
        self.cursor = self.cell(
            content.cursor.point.line.0 + offset,
            content.cursor.point.column.0,
            1,
        );
        let cell_h = self.metrics.cell.height;

        for cell in content.display_iter {
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let (row, col) = (cell.point.line.0 + offset, cell.point.column.0);
            let wide = cell.flags.contains(Flags::WIDE_CHAR);
            let cells = if wide { 2 } else { 1 };
            let area = self.cell(row, col, cells);
            let inverse = cell.flags.contains(Flags::INVERSE);
            let (mut fg, mut bg) = (
                resolve(cell.fg, content.colors),
                resolve(cell.bg, content.colors),
            );
            if inverse {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.flags.intersects(Flags::DIM) {
                fg.a *= 0.66;
            }
            let selected = content.selection.is_some_and(|s| s.contains(cell.point));
            let on_cursor = focused
                && cursor.is_some_and(|c| c.shape == CursorShape::Block && c.point == cell.point);
            if on_cursor {
                self.fill(row, col, cells, rgb(FG));
                fg = rgb(BG);
            } else if selected {
                self.fill(row, col, cells, theme::SEL.into());
            } else if inverse || cell.bg != Color::Named(NamedColor::Background) {
                self.fill(row, col, cells, bg);
            }
            let c = if cell.flags.contains(Flags::HIDDEN) {
                ' '
            } else {
                cell.c
            };
            let bold = cell.flags.contains(Flags::BOLD);
            if wide {
                self.wide_char(row, col, c, fg, bold, window);
            } else {
                self.push_char(row, col, c, fg, bold, window);
            }
            for (drawn, y) in [
                (
                    cell.flags.intersects(Flags::ALL_UNDERLINES),
                    area.bottom() - px(2.0),
                ),
                (
                    cell.flags.contains(Flags::STRIKEOUT),
                    area.top() + (cell_h * 0.5).floor(),
                ),
            ] {
                if drawn {
                    self.marks.push(fill(
                        Bounds::new(point(area.left(), y), size(area.size.width, px(1.0))),
                        fg,
                    ));
                }
            }
        }
        self.flush(window);

        if let Some(cursor) = cursor {
            let area = self.cell(cursor.point.line.0, cursor.point.column.0, 1);
            let color = rgb(FG);
            match (cursor.shape, focused) {
                (CursorShape::Block, true) | (CursorShape::Hidden, _) => {}
                (CursorShape::Block | CursorShape::HollowBlock, _) => {
                    self.marks.push(outline(area, color, BorderStyle::Solid));
                }
                (CursorShape::Underline, _) => {
                    self.marks.push(fill(
                        Bounds::new(
                            point(area.left(), area.bottom() - px(2.0)),
                            size(area.size.width, px(2.0)),
                        ),
                        color,
                    ));
                }
                (CursorShape::Beam, _) => {
                    self.marks
                        .push(fill(Bounds::new(area.origin, size(px(2.0), cell_h)), color));
                }
            }
        }
    }

    fn notes(&mut self, exited: bool, marked: &str, window: &Window) {
        if exited {
            let runs = self.run(&[(EXITED.len(), theme::YELLOW.into(), false)], false);
            let origin = point(
                self.origin.x + px(6.0),
                self.bottom - px(4.0) - self.metrics.cell.height,
            );
            let line = self.shape(EXITED.to_owned(), &runs, window, false);
            self.notes.push((None, origin, line));
        }
        if !marked.is_empty() {
            let runs = self.run(&[(marked.len(), rgb(FG), false)], true);
            let line = self.shape(marked.to_owned(), &runs, window, false);
            let backdrop = fill(
                Bounds::new(
                    self.cursor.origin,
                    size(line.width, self.metrics.cell.height),
                ),
                rgb(BG),
            );
            self.notes.push((Some(backdrop), self.cursor.origin, line));
        }
    }

    fn paint(&self, window: &mut Window, cx: &mut App) {
        let cell_h = self.metrics.cell.height;
        for &(row, start, end, color) in &self.fills {
            window.paint_quad(fill(self.cell(row, start, end - start), color));
        }
        for (origin, line) in &self.lines {
            line.paint(*origin, cell_h, TextAlign::Left, None, window, cx)
                .ok();
        }
        for mark in &self.marks {
            window.paint_quad(mark.clone());
        }
        for (backdrop, origin, line) in &self.notes {
            if let Some(backdrop) = backdrop {
                window.paint_quad(backdrop.clone());
            }
            line.paint(*origin, cell_h, TextAlign::Left, None, window, cx)
                .ok();
        }
    }
}

#[cfg(test)]
impl Terminal {
    pub fn showing(bytes: &[u8], exited: bool) -> Self {
        let size = Size { cols: 80, rows: 24 };
        let pair = native_pty_system()
            .openpty(pty_size(size))
            .expect("a pty opens");
        let writer: Writer = Arc::new(Mutex::new(Box::new(std::io::sink())));
        let clipboard = Arc::new(Mutex::new(None));
        let wake = Wake::default();
        let term = Session::screen(&writer, &clipboard, &wake, size);
        if let Ok(mut term) = term.lock() {
            let mut processor: Processor = Processor::new();
            processor.advance(&mut *term, bytes);
        }
        let session = Session {
            term,
            writer,
            clipboard,
            master: pair.master,
            child: Box::new(Idle),
            exited: Arc::new(AtomicBool::new(exited)),
            size,
        };
        Self::with_session(Ok(session), None, wake)
    }

    pub fn failed(error: &str) -> Self {
        Self::with_session(Err(error.to_owned()), None, Wake::default())
    }
}

#[cfg(test)]
#[derive(Debug)]
struct Idle;

#[cfg(test)]
impl portable_pty::ChildKiller for Idle {
    fn kill(&mut self) -> std::io::Result<()> {
        Ok(())
    }

    fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
        Box::new(Idle)
    }
}

#[cfg(test)]
impl Child for Idle {
    fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> {
        Ok(None)
    }

    fn wait(&mut self) -> std::io::Result<portable_pty::ExitStatus> {
        Ok(portable_pty::ExitStatus::with_exit_code(0))
    }

    fn process_id(&self) -> Option<u32> {
        None
    }

    #[cfg(windows)]
    fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
        None
    }
}

fn shell() -> String {
    #[cfg(windows)]
    return std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_owned());
    #[cfg(not(windows))]
    return std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_owned());
}

fn modifier_param(m: Modifiers) -> u8 {
    1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.control)
}

fn key_bytes(key: &str, m: Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    let param = modifier_param(m);
    let plain = param == 1;
    let csi = |number: Option<u8>, final_byte: char| -> Vec<u8> {
        match (number, plain) {
            (None, true) => format!("\x1b[{final_byte}"),
            (None, false) => format!("\x1b[1;{param}{final_byte}"),
            (Some(n), true) => format!("\x1b[{n}{final_byte}"),
            (Some(n), false) => format!("\x1b[{n};{param}{final_byte}"),
        }
        .into_bytes()
    };
    let ss3 = |final_byte: char| -> Vec<u8> {
        if plain {
            format!("\x1bO{final_byte}").into_bytes()
        } else {
            csi(None, final_byte)
        }
    };
    let arrow = |final_byte: char| {
        if mode.contains(TermMode::APP_CURSOR) {
            ss3(final_byte)
        } else {
            csi(None, final_byte)
        }
    };
    let alt = |bytes: &[u8]| {
        if m.alt {
            [b"\x1b".as_slice(), bytes].concat()
        } else {
            bytes.to_vec()
        }
    };

    Some(match key {
        "enter" if m.shift => b"\n".to_vec(),
        "enter" => alt(b"\r"),
        "tab" if m.shift => b"\x1b[Z".to_vec(),
        "tab" => b"\t".to_vec(),
        "backspace" if m.control => b"\x17".to_vec(),
        "backspace" => alt(b"\x7f"),
        "escape" => b"\x1b".to_vec(),
        "up" => arrow('A'),
        "down" => arrow('B'),
        "right" => arrow('C'),
        "left" => arrow('D'),
        "home" => arrow('H'),
        "end" => arrow('F'),
        "insert" => csi(Some(2), '~'),
        "delete" => csi(Some(3), '~'),
        "pageup" => csi(Some(5), '~'),
        "pagedown" => csi(Some(6), '~'),
        "f1" => ss3('P'),
        "f2" => ss3('Q'),
        "f3" => ss3('R'),
        "f4" => ss3('S'),
        "f5" => csi(Some(15), '~'),
        "f6" => csi(Some(17), '~'),
        "f7" => csi(Some(18), '~'),
        "f8" => csi(Some(19), '~'),
        "f9" => csi(Some(20), '~'),
        "f10" => csi(Some(21), '~'),
        "f11" => csi(Some(23), '~'),
        "f12" => csi(Some(24), '~'),
        "space" if m.control => b"\0".to_vec(),
        "v" if m.control => return None,
        "c" if m.control && m.shift => return None,
        _ if m.control && !m.alt => {
            let byte = match key {
                "[" => 0x1b,
                "\\" => 0x1c,
                "]" => 0x1d,
                n if n.len() == 1 && n.as_bytes()[0].is_ascii_alphabetic() => {
                    n.as_bytes()[0].to_ascii_uppercase() - b'A' + 1
                }
                _ => return None,
            };
            vec![byte]
        }
        _ => return None,
    })
}

fn mouse_report(
    mode: TermMode,
    button: u8,
    m: Modifiers,
    row: usize,
    col: usize,
    release: bool,
) -> Vec<u8> {
    let code = button + 4 * u8::from(m.shift) + 8 * u8::from(m.alt) + 16 * u8::from(m.control);
    if mode.contains(TermMode::SGR_MOUSE) {
        format!(
            "\x1b[<{code};{};{}{}",
            col + 1,
            row + 1,
            if release { 'm' } else { 'M' }
        )
        .into_bytes()
    } else {
        let code = if release { 3 + (code & !3) } else { code };
        vec![
            0x1b,
            b'[',
            b'M',
            32 + code,
            32 + (col + 1).min(223) as u8,
            32 + (row + 1).min(223) as u8,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(key: &str, modifiers: Modifiers, mode: TermMode) -> Option<String> {
        key_bytes(key, modifiers, mode).map(|b| String::from_utf8(b).unwrap())
    }

    #[test]
    fn arrows_follow_the_cursor_key_mode() {
        let none = Modifiers::none();
        assert_eq!(bytes("up", none, TermMode::NONE).unwrap(), "\x1b[A");
        assert_eq!(bytes("up", none, TermMode::APP_CURSOR).unwrap(), "\x1bOA");
        assert_eq!(
            bytes("left", Modifiers::control(), TermMode::APP_CURSOR).unwrap(),
            "\x1b[1;5D"
        );
    }

    #[test]
    fn control_letters_and_editing_keys() {
        let ctrl = Modifiers::control();
        assert_eq!(bytes("c", ctrl, TermMode::NONE).unwrap(), "\x03");
        assert_eq!(bytes("d", ctrl, TermMode::NONE).unwrap(), "\x04");
        assert_eq!(bytes("[", ctrl, TermMode::NONE).unwrap(), "\x1b");
        assert_eq!(bytes("space", ctrl, TermMode::NONE).unwrap(), "\0");
        assert_eq!(
            bytes("delete", Modifiers::none(), TermMode::NONE).unwrap(),
            "\x1b[3~"
        );
        assert_eq!(
            bytes("tab", Modifiers::shift(), TermMode::NONE).unwrap(),
            "\x1b[Z"
        );
        assert_eq!(
            bytes("enter", Modifiers::shift(), TermMode::NONE).unwrap(),
            "\n"
        );
        assert_eq!(
            bytes("enter", Modifiers::alt(), TermMode::NONE).unwrap(),
            "\x1b\r"
        );
        assert!(bytes("a", Modifiers::none(), TermMode::NONE).is_none());
        assert!(bytes("space", Modifiers::none(), TermMode::NONE).is_none());
        assert!(bytes("v", ctrl, TermMode::NONE).is_none());
        assert!(bytes("c", Modifiers::control_shift(), TermMode::NONE).is_none());
    }

    #[test]
    fn alt_prefixes_typed_text_with_escape() {
        let keystroke = gpui_kit::Keystroke {
            modifiers: Modifiers::alt(),
            key: "b".into(),
            key_char: Some("b".into()),
        };
        assert_eq!(alt_text(&keystroke).unwrap(), b"\x1bb");
        let plain = gpui_kit::Keystroke {
            modifiers: Modifiers::none(),
            ..keystroke
        };
        assert!(alt_text(&plain).is_none());
    }

    #[test]
    fn paste_brackets_or_turns_newlines_into_returns() {
        assert_eq!(paste_bytes("a\nb", TermMode::NONE), b"a\rb");
        assert_eq!(
            paste_bytes("a\x1b\nb", TermMode::BRACKETED_PASTE),
            b"\x1b[200~a\nb\x1b[201~"
        );
    }

    #[test]
    fn mouse_reports() {
        let none = Modifiers::none();
        let sgr = mouse_report(TermMode::SGR_MOUSE, 0, none, 4, 9, false);
        assert_eq!(sgr, b"\x1b[<0;10;5M");
        let release = mouse_report(TermMode::SGR_MOUSE, 0, none, 4, 9, true);
        assert_eq!(release, b"\x1b[<0;10;5m");
        let legacy = mouse_report(TermMode::NONE, 0, none, 0, 0, false);
        assert_eq!(legacy, [0x1b, b'[', b'M', 32, 33, 33]);
    }

    #[test]
    fn color_table() {
        assert_eq!(default_rgb(1).r, 0xf7);
        assert_eq!((default_rgb(16).r, default_rgb(231).r), (0, 255));
        assert_eq!(default_rgb(232).r, 8);
        assert_eq!(default_rgb(257).r, 0x16);
        assert_eq!(default_rgb(256).r, 0xc0);
    }

    #[cfg(unix)]
    #[test]
    fn shell_output_reaches_the_screen() {
        let session = Session::spawn(
            &Wake::default(),
            "/bin/sh",
            Size { cols: 40, rows: 5 },
            None,
        )
        .expect("spawn");
        session.write(b"echo volcren-$((6*7))\r");
        let found = (0..100).any(|_| {
            thread::sleep(std::time::Duration::from_millis(50));
            let term = session.term.lock().unwrap();
            let grid = term.grid();
            (0..5).any(|line| {
                let text: String = (0..40).map(|col| grid[Line(line)][Column(col)].c).collect();
                text.starts_with("volcren-42")
            })
        });
        assert!(found, "the echoed line never showed up on the screen");
        session.write(b"exit\r");
        assert!((0..100).any(|_| {
            thread::sleep(std::time::Duration::from_millis(50));
            session.exited()
        }));
    }

    struct Recorder(Arc<Mutex<Vec<u8>>>);

    impl Write for Recorder {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct Host {
        terminal: Terminal,
    }

    impl gpui_kit::Render for Host {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().flex().flex_col().child(self.terminal.render(
                |host: &mut Host| Some(&mut host.terminal),
                window,
                cx,
            ))
        }
    }

    #[gpui_kit::test]
    fn keys_text_selection_and_clipboard_reach_the_shell(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext, test::TestWindowExt};

        let terminal = Terminal::showing(b"$ ls\r\nlib  scripts  CLAUDE.md\r\n$ ", false);
        let typed = Arc::new(Mutex::new(Vec::new()));
        if let Ok(session) = &terminal.session {
            *session.writer.lock().unwrap() = Box::new(Recorder(typed.clone()));
        }
        let handle = cx.add_window(|_, _| Host { terminal });
        let take = || String::from_utf8(std::mem::take(&mut *typed.lock().unwrap())).unwrap();

        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.input("ls", cx);
            window.press("enter", cx);
        })
        .unwrap();
        assert_eq!(take(), "ls\r");

        for (key, sent) in [
            ("ctrl-c", "\x03"),
            ("up", "\x1b[A"),
            ("ctrl-left", "\x1b[1;5D"),
            ("alt-b", "\x1bb"),
            ("escape", "\x1b"),
            ("tab", "\t"),
            ("backspace", "\x7f"),
        ] {
            cx.update_window(handle.into(), |_, window, cx| window.press(key, cx))
                .unwrap();
            assert_eq!(take(), sent, "{key}");
        }

        let grid = handle.update(cx, |host, _, _| host.terminal.grid).unwrap();
        let at = |row: usize, col: f32| {
            grid.bounds.origin
                + point(
                    grid.cell.width * col,
                    grid.cell.height * (row as f32 + 0.5),
                )
        };
        cx.update_window(handle.into(), |_, window, cx| {
            window.drag(at(1, 14.25), at(1, 22.75), cx);
            window.press("ctrl-shift-c", cx);
        })
        .unwrap();
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()).as_deref(),
            Some("CLAUDE.md")
        );
        assert_eq!(take(), "");

        cx.write_to_clipboard(ClipboardItem::new_string("echo hi\r\n".into()));
        cx.update_window(handle.into(), |_, window, cx| window.press("ctrl-v", cx))
            .unwrap();
        assert_eq!(take(), "echo hi\r");
    }
}
