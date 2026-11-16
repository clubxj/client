use std::sync::atomic::{AtomicU32, Ordering};

use ipc::state::{Layout, MenuEntryInfo, MenuStateLayout, menu_kind, part, text};
use serde::{Deserialize, Serialize};

use crate::memory::{Memory, byte};

const SEARCH: u64 = 0x2000;
pub const MAX_ENTRIES: u64 = 500;
const EXAMINE_PRIORITY: i32 = 0x7fff_fffe;
const TILE_X: u64 = 0x14;
const TILE_Y: u64 = 0x18;

static VECTOR: AtomicU32 = AtomicU32::new(0);
static MINIMENU: AtomicU32 = AtomicU32::new(u32::MAX);

#[derive(Debug, Clone, Copy)]
pub struct MenuObject {
    pub object: u64,
    pub module_base: u64,
    pub entry_vtable: u64,
    pub rules: MenuStateLayout,
}

impl MenuObject {
    pub fn current(memory: &impl Memory, layout: &Layout) -> Option<Self> {
        if !layout.has(part::MENU) {
            return None;
        }
        let client = ipc::read::client(memory, layout)?;
        Some(Self {
            object: memory.ptr(client + layout.menu.object as u64)?,
            module_base: layout.module_base,
            entry_vtable: layout.module_base + layout.menu.entry_vtable as u64,
            rules: layout.menu,
        })
    }

    pub fn entries_offset(&self, memory: &impl Memory) -> Option<u64> {
        let known = VECTOR.load(Ordering::Relaxed);
        if known != 0 {
            return Some(known as u64);
        }
        let size = self.rules.entry_size as u64;
        for offset in (0..SEARCH).step_by(8) {
            let (Some(begin), Some(end), Some(cap)) = (
                memory.u64(self.object + offset),
                memory.u64(self.object + offset + 8),
                memory.u64(self.object + offset + 16),
            ) else {
                continue;
            };
            if begin == 0
                || end <= begin
                || cap < end
                || (end - begin) % size != 0
                || (cap - begin) % size != 0
                || (end - begin) / size > MAX_ENTRIES
            {
                continue;
            }
            if memory.u64(begin) == Some(self.entry_vtable) {
                VECTOR.store(offset as u32, Ordering::Relaxed);
                return Some(offset);
            }
        }
        None
    }

    pub fn minimenu(&self, memory: &impl Memory) -> Option<u64> {
        let known = MINIMENU.load(Ordering::Relaxed);
        if known != u32::MAX {
            return Some(self.object + known as u64);
        }
        let vtable = self.module_base + self.rules.minimenu_vtable as u64;
        let offset = (0..SEARCH)
            .step_by(8)
            .find(|&offset| memory.u64(self.object + offset) == Some(vtable))?;
        MINIMENU.store(offset as u32, Ordering::Relaxed);
        Some(self.object + offset)
    }

    pub fn entry(&self, memory: &impl Memory, e: u64) -> Option<MenuEntryInfo> {
        if memory.u64(e)? != self.entry_vtable {
            return None;
        }
        let r = &self.rules;
        let mut info = MenuEntryInfo::default();
        string(memory, e + r.option as u64, &mut info.option);
        string(memory, e + r.target as u64, &mut info.target);
        let ids = e + r.ids as u64;
        info.id = memory.i32(ids)?;
        info.index = memory.i32(ids + 4)?;
        info.action = memory.i32(ids + 8)?;
        info.x = memory.i32(ids + TILE_X)?;
        info.y = memory.i32(ids + TILE_Y)?;
        info.priority = memory.i32(e + r.priority as u64)?;
        let mut bytes = [0u8; 2];
        memory.read(e + r.kind as u64, &mut bytes);
        info.kind = bytes[0];
        info.category = bytes[1];
        info.cancel = byte(memory, e + r.cancel as u64).unwrap_or(0);
        Some(info)
    }

    pub fn raw_entries(&self, memory: &impl Memory) -> Option<Vec<MenuEntryInfo>> {
        let size = self.rules.entry_size as u64;
        let vector = self.entries_offset(memory)?;
        let (begin, end) = (
            memory.u64(self.object + vector)?,
            memory.u64(self.object + vector + 8)?,
        );
        if end < begin || (end - begin) % size != 0 || (end - begin) / size > MAX_ENTRIES {
            return None;
        }
        (0..(end - begin) / size)
            .map(|k| self.entry(memory, begin + k * size))
            .collect()
    }

    pub fn open(&self, memory: &impl Memory, layout: &Layout) -> Option<bool> {
        layout
            .has(part::MENU_OPEN)
            .then(|| memory.u64(self.object + self.rules.open as u64))
            .flatten()
            .map(|v| v != 0)
    }

    pub fn click(
        &self,
        memory: &impl Memory,
        layout: &Layout,
    ) -> Option<(MenuEntryInfo, i32, i32, i32)> {
        if !layout.has(part::MENU_CLICK) {
            return None;
        }
        let r = &self.rules;
        let minimenu = self.minimenu(memory)?;
        let x = minimenu + r.click_x as u64;
        Some((
            self.entry(memory, minimenu + r.click_entry as u64)?,
            memory.i32(x)?,
            memory.i32(x + 4)?,
            memory.i32(minimenu + r.click_counter as u64)?,
        ))
    }

    pub fn selection(
        &self,
        memory: &impl Memory,
        layout: &Layout,
    ) -> Option<(MenuEntryInfo, bool)> {
        if !layout.has(part::SELECTION) {
            return None;
        }
        let r = &self.rules;
        let s = self.object + r.selection as u64;
        let mut info = MenuEntryInfo::default();
        string(memory, s + r.selection_verb as u64, &mut info.option);
        string(memory, s + r.selection_name as u64, &mut info.target);
        info.id = memory.i32(s + r.selection_id as u64)?;
        info.index = memory.i32(s + r.selection_child as u64)?;
        info.action = memory.i32(s + r.selection_item as u64)?;
        info.kind = byte(memory, s + r.selection_flags as u64).unwrap_or(0);
        let selected = byte(memory, s + r.selection_selected as u64)? != 0;
        Some((info, selected))
    }
}

fn string(memory: &impl Memory, address: u64, out: &mut [u8]) {
    let (Some(ptr), Some(len)) = (memory.u64(address), memory.u64(address + 8)) else {
        return;
    };
    let len = (len as usize).min(out.len() - 1);
    if ptr != 0 && len > 0 {
        memory.read(ptr, &mut out[..len]);
    }
}

fn rank(entry: &MenuEntryInfo) -> u8 {
    if entry.cancel != 0 {
        3
    } else if entry.priority == EXAMINE_PRIORITY {
        2
    } else if entry.kind == menu_kind::TILE {
        1
    } else {
        0
    }
}

pub fn sorted(entries: Vec<MenuEntryInfo>) -> Vec<MenuEntryInfo> {
    let mut entries: Vec<_> = entries.into_iter().enumerate().collect();
    entries.sort_by(|(ia, a), (ib, b)| {
        rank(a)
            .cmp(&rank(b))
            .then_with(|| {
                if rank(a) == 0 {
                    a.priority.cmp(&b.priority)
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .then_with(|| ib.cmp(ia))
    });
    entries.into_iter().map(|(_, e)| e).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub option: Vec<u8>,
    pub target: Vec<u8>,
    pub id: i32,
    pub index: i32,
    pub action: i32,
    pub priority: i32,
    pub x: i32,
    pub y: i32,
    pub kind: u8,
    pub submenu: bool,
}

impl From<&MenuEntryInfo> for Entry {
    fn from(e: &MenuEntryInfo) -> Self {
        Self {
            option: text(&e.option).to_vec(),
            target: text(&e.target).to_vec(),
            id: e.id,
            index: e.index,
            action: e.action,
            priority: e.priority,
            x: e.x,
            y: e.y,
            kind: e.kind,
            submenu: e.submenu != 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Click {
    pub entry: Entry,
    pub x: i32,
    pub y: i32,
    pub age: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub entry: Entry,
    pub selected: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub rows_x: i32,
    pub rows_y: i32,
    pub rows_width: i32,
    pub rows_height: i32,
    pub header_height: i32,
    pub row_height: i32,
    pub scroll: i32,
    pub margin: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Submenu {
    pub row: u32,
    pub entries: Vec<Entry>,
    pub geometry: Geometry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Opened {
    pub entries: Vec<Entry>,
    pub geometry: Option<Geometry>,
    pub submenus: Vec<Submenu>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Menu {
    pub open: Option<bool>,
    pub entries: Option<Vec<Entry>>,
    pub opened: Option<Opened>,
    pub click: Option<Click>,
    pub selection: Option<Selection>,
}

pub fn menu(memory: &impl Memory, layout: &Layout) -> Menu {
    let Some(menu) = MenuObject::current(memory, layout) else {
        return Menu::default();
    };
    let open = menu.open(memory, layout);
    let entries = (open == Some(false))
        .then(|| menu.raw_entries(memory))
        .flatten()
        .map(|raw| sorted(raw).iter().map(Entry::from).collect());
    Menu {
        open,
        entries,
        opened: None,
        click: menu.click(memory, layout).map(|(entry, x, y, age)| Click {
            entry: Entry::from(&entry),
            x,
            y,
            age,
        }),
        selection: menu
            .selection(memory, layout)
            .map(|(entry, selected)| Selection {
                entry: Entry::from(&entry),
                selected,
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Fake;

    const BASE: u64 = 0x1000_0000;
    const CLIENT: u64 = 0x2000_0000;
    const OBJECT: u64 = 0x3000_0000;
    const ENTRIES: u64 = 0x4000_0000;
    const STRINGS: u64 = 0x5000_0000;
    const SIZE: u64 = 0x100;

    fn layout() -> Layout {
        let mut layout = Layout {
            found: part::MENU | part::MENU_OPEN,
            module_base: BASE,
            client: 0x100,
            ..Layout::default()
        };
        layout.menu = MenuStateLayout {
            object: 0x10,
            entry_vtable: 0x50,
            entry_size: SIZE as u32,
            option: 0x08,
            target: 0x20,
            ids: 0x40,
            priority: 0x60,
            kind: 0x64,
            cancel: 0x66,
            open: 0x400,
            ..MenuStateLayout::default()
        };
        layout
    }

    fn string(memory: &mut Fake, at: u64, slot: u64, text: &str) {
        let data = STRINGS + slot * 0x100;
        memory.put(data, text.as_bytes());
        memory.put_u64(at, data);
        memory.put_u64(at + 8, text.len() as u64);
    }

    fn menu(entries: &[(&str, u8, i32, u8)]) -> Fake {
        let mut memory = Fake::default();
        memory.put_u64(BASE + 0x100, CLIENT);
        memory.put_u64(CLIENT + 0x10, OBJECT);
        memory.put_u64(OBJECT + 0x40, ENTRIES);
        memory.put_u64(OBJECT + 0x48, ENTRIES + SIZE * entries.len() as u64);
        memory.put_u64(OBJECT + 0x50, ENTRIES + SIZE * 8);
        memory.put_u64(OBJECT + 0x400, 0);
        for (i, &(option, kind, priority, cancel)) in entries.iter().enumerate() {
            let e = ENTRIES + SIZE * i as u64;
            memory.put_u64(e, BASE + 0x50);
            string(&mut memory, e + 0x08, i as u64 * 2, option);
            string(&mut memory, e + 0x20, i as u64 * 2 + 1, "Seed");
            memory.put_i32(e + 0x40, i as i32);
            memory.put_i32(e + 0x44, -1);
            memory.put_i32(e + 0x48, -1);
            memory.put_i32(e + 0x54, 3188);
            memory.put_i32(e + 0x58, 3426 + i as i32);
            memory.put_i32(e + 0x60, priority);
            memory.put(e + 0x64, &[kind, 0, cancel]);
        }
        memory
    }

    fn options(menu: &Menu) -> Vec<String> {
        menu.entries
            .iter()
            .flatten()
            .map(|e| String::from_utf8_lossy(&e.option).into_owned())
            .collect()
    }

    #[test]
    fn a_closed_menu_reads_with_the_left_click_first() {
        let memory = menu(&[
            ("Cancel", 0, 0, 1),
            ("Walk here", menu_kind::TILE, 0x7fff_ffff, 0),
            ("Examine", 4, EXAMINE_PRIORITY, 0),
            ("Take", 4, 0, 0),
        ]);
        let read = super::menu(&memory, &layout());
        assert_eq!(read.open, Some(false));
        assert_eq!(options(&read), ["Take", "Walk here", "Examine", "Cancel"]);
        let take = &read.entries.unwrap()[0];
        assert_eq!((take.x, take.y), (3188, 3429));
    }

    #[test]
    fn an_entry_that_isnt_one_drops_the_list() {
        let mut memory = menu(&[("Cancel", 0, 0, 1), ("Take", 4, 0, 0)]);
        memory.put_u64(ENTRIES + SIZE, 0xbad);
        assert_eq!(super::menu(&memory, &layout()).entries, None);
    }

    #[test]
    fn an_open_menu_leaves_its_rows_to_the_game_thread() {
        let mut memory = menu(&[("Cancel", 0, 0, 1)]);
        memory.put_u64(OBJECT + 0x400, 1);
        let read = super::menu(&memory, &layout());
        assert_eq!(
            (read.open, read.entries, read.opened),
            (Some(true), None, None)
        );
    }

    #[test]
    fn without_the_menu_rules_nothing_is_read() {
        let memory = menu(&[("Cancel", 0, 0, 1)]);
        assert_eq!(super::menu(&memory, &Layout::default()), Menu::default());
    }
}
