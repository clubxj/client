use std::{
    sync::{
        atomic::{AtomicU32, Ordering},
        Mutex, OnceLock,
    },
    time::Instant,
};

use decode::menu::{sorted, Entry, Geometry, MenuObject, Opened, Submenu, MAX_ENTRIES};
use ipc::{read::Memory, state::MenuEntryInfo};

use super::{
    detour::function,
    offsets::MenuLayout,
    own::{on_other_thread, OwnMemory, LAYOUT},
};

const ROW_SIZE: u64 = 24;
const ROW_INDEX: u64 = 8;
const SUBMENUS: usize = 4;
const SUBMENU_ENTRIES: u64 = 64;

static LIST_ENTRIES: AtomicU32 = AtomicU32::new(u32::MAX);
static CAPTURED: Mutex<Option<(Instant, decode::menu::Menu)>> = Mutex::new(None);
static LAYOUT_RULE: OnceLock<MenuLayout> = OnceLock::new();

pub fn set_menu_layout(rule: MenuLayout) {
    let _ = LAYOUT_RULE.set(rule);
}

fn current(memory: &OwnMemory) -> Option<MenuObject> {
    MenuObject::current(memory, LAYOUT.get()?)
}

#[repr(C)]
#[derive(Default)]
struct LayoutOut {
    menu: [i32; 4],
    header: [i32; 4],
    rows: [i32; 4],
    header_height: i32,
    row_height: i32,
    rows_total: i32,
    scroll: i32,
    rest: [i32; 10],
}

const _: () = assert!(std::mem::size_of::<LayoutOut>() == 0x68);

fn read_geometry(menu: MenuObject, minimenu: u64, path: &[u64]) -> Option<Geometry> {
    let layout = LAYOUT_RULE.get()?;
    let function: unsafe extern "C" fn(u64, *mut LayoutOut, *const [u64; 3]) -> *mut LayoutOut =
        unsafe { function(menu.module_base + layout.function as u64) };
    let empty = 0u64;
    let begin = if path.is_empty() {
        &empty as *const u64 as u64
    } else {
        path.as_ptr() as u64
    };
    let end = begin + 8 * path.len() as u64;
    let vector = [begin, end, end];
    let mut out = LayoutOut::default();
    unsafe { function(minimenu + layout.view as u64, &mut out, &vector) };
    let [x, y, width, height] = out.menu;
    let [rows_x, rows_y, rows_width, rows_height] = out.rows;
    Some(Geometry {
        x,
        y,
        width,
        height,
        rows_x,
        rows_y,
        rows_width,
        rows_height,
        header_height: out.header_height,
        row_height: out.row_height,
        scroll: out.scroll,
        margin: layout.margin,
    })
}

fn vector(memory: &impl Memory, at: u64, size: u64, max: u64) -> Option<(u64, u64)> {
    let (begin, end) = (memory.u64(at)?, memory.u64(at + 8)?);
    (end >= begin && (end - begin) % size == 0 && (end - begin) / size <= max)
        .then_some((begin, (end - begin) / size))
}

fn read_rows(
    memory: &impl Memory,
    menu: MenuObject,
    rows: u64,
    max: u64,
) -> Option<Vec<MenuEntryInfo>> {
    let layout = LAYOUT_RULE.get()?;
    let size = menu.rules.entry_size as u64;
    let (begin, count) = vector(memory, rows, ROW_SIZE, max)?;
    (0..count)
        .map(|k| {
            let row = begin + k * ROW_SIZE;
            let list = memory.u64(row)?;
            let known = LIST_ENTRIES.load(Ordering::Relaxed);
            let entries = if known != u32::MAX {
                known as u64
            } else {
                let vector = menu.entries_offset(memory)?;
                let offset = list.checked_sub(menu.object).filter(|&o| o <= vector)?;
                LIST_ENTRIES.store((vector - offset) as u32, Ordering::Relaxed);
                vector - offset
            };
            let (first, n) = vector(memory, list + entries, size, MAX_ENTRIES)?;
            let index = memory.u64(row + ROW_INDEX)?;
            let entry = first + index * size;
            if index >= n {
                return None;
            }
            let mut info = menu.entry(memory, entry)?;
            info.submenu = (memory.u64(row + layout.submenu as u64)? != 0) as u8;
            Some(info)
        })
        .collect()
}

fn open_path(memory: &impl Memory, minimenu: u64) -> Option<Vec<u64>> {
    let layout = LAYOUT_RULE.get()?;
    let (begin, count) = vector(memory, minimenu + layout.path as u64, 8, SUBMENUS as u64)?;
    let mut path = (0..count)
        .map(|k| memory.u64(begin + 8 * k))
        .collect::<Option<Vec<_>>>()?;
    let open = memory.u64(minimenu + layout.open_row as u64)?;
    if open != u64::MAX {
        path.push(open);
    }
    path.truncate(SUBMENUS);
    Some(path)
}

fn entries(rows: &[MenuEntryInfo]) -> Vec<Entry> {
    rows.iter().map(Entry::from).collect()
}

fn read_submenus(memory: &OwnMemory, menu: MenuObject, minimenu: u64) -> Vec<Submenu> {
    let Some(layout) = LAYOUT_RULE.get() else {
        return Vec::new();
    };
    let path = open_path(memory, minimenu).unwrap_or_default();
    let mut rows = minimenu + layout.rows as u64;
    let mut submenus = Vec::new();
    for (depth, &row) in path.iter().enumerate() {
        let Some((begin, count)) = vector(memory, rows, ROW_SIZE, MAX_ENTRIES) else {
            break;
        };
        let Some(sub) = (row < count)
            .then(|| memory.u64(begin + row * ROW_SIZE + layout.submenu as u64))
            .flatten()
            .filter(|&s| s != 0)
        else {
            break;
        };
        let (Some(entries), Some(geometry)) = (
            read_rows(memory, menu, sub, SUBMENU_ENTRIES),
            read_geometry(menu, minimenu, &path[..=depth]),
        ) else {
            break;
        };
        submenus.push(Submenu {
            row: row as u32,
            entries: self::entries(&entries),
            geometry,
        });
        rows = sub;
    }
    submenus
}

fn opened(memory: &OwnMemory) -> Option<Opened> {
    let menu = current(memory)?;
    let raw = menu.raw_entries(memory)?;
    let rows = LAYOUT_RULE.get().zip(menu.minimenu(memory));
    let Some((layout, minimenu)) = rows else {
        return Some(Opened {
            entries: entries(&sorted(raw)),
            geometry: None,
            submenus: Vec::new(),
        });
    };
    let rows = read_rows(memory, menu, minimenu + layout.rows as u64, MAX_ENTRIES)
        .unwrap_or_else(|| sorted(raw));
    let geometry = read_geometry(menu, minimenu, &[]);
    Some(Opened {
        entries: entries(&rows),
        geometry,
        submenus: geometry
            .map(|_| read_submenus(memory, menu, minimenu))
            .unwrap_or_default(),
    })
}

pub fn capture() {
    if on_other_thread() {
        return;
    }
    let Some(layout) = LAYOUT.get() else { return };
    let memory = OwnMemory;
    let mut menu = decode::menu::menu(&memory, layout);
    if menu.open == Some(true) {
        menu.opened = opened(&memory);
    }
    if let Ok(mut captured) = CAPTURED.lock() {
        *captured = Some((Instant::now(), menu));
    }
}

pub fn captured() -> Option<decode::menu::Menu> {
    let captured = CAPTURED.lock().ok()?;
    let (at, menu) = captured.as_ref()?;
    (at.elapsed() <= super::drawn::MAX_AGE).then(|| menu.clone())
}

pub enum Snapshot {
    Closed(Vec<MenuEntryInfo>),
    Open,
}

pub fn snapshot() -> Option<Snapshot> {
    if on_other_thread() {
        return None;
    }
    let memory = OwnMemory;
    let menu = current(&memory)?;
    if menu.open(&memory, LAYOUT.get()?) == Some(true) {
        return Some(Snapshot::Open);
    }
    menu.raw_entries(&memory)
        .map(|raw| Snapshot::Closed(sorted(raw)))
}
