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
    pub ordered: bool,
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
            ordered: layout.has(part::MENU_ORDER),
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
        info.kind = byte(memory, e + r.kind as u64).unwrap_or(0);
        info.category = byte(memory, e + r.category as u64).unwrap_or(0);
        info.cancel = byte(memory, e + r.cancel as u64).unwrap_or(0);
        Some(info)
    }

    pub fn sorted_entries(
        &self,
        memory: &impl Memory,
        layout: &Layout,
    ) -> Option<Vec<MenuEntryInfo>> {
        let settings = self.settings(memory, layout);
        let entries = self.raw_entries(memory, layout, settings.is_some())?;
        Some(sorted(entries, settings.as_ref()))
    }

    fn settings(&self, memory: &impl Memory, layout: &Layout) -> Option<Settings> {
        if !layout.has(part::MENU_SETTINGS) {
            return None;
        }
        let r = &self.rules;
        let minimenu = self.minimenu(memory)?;
        let client = ipc::read::client(memory, layout)?;
        let (_, me) = ipc::read::local_player(memory, layout)?;
        Some(Settings {
            player_attack: byte(memory, minimenu + r.player_attack as u64)?,
            npc_attack: byte(memory, minimenu + r.npc_attack as u64)?,
            follower: byte(memory, minimenu + r.follower as u64)?,
            local_index: memory.i32(client + layout.player.player_index as u64)?,
            local_level: memory.i32(me + r.player_level as u64)?,
            local_team: memory.i32(me + r.player_team as u64)?,
        })
    }

    fn player_level(&self, memory: &impl Memory, layout: &Layout, index: i32) -> Option<i32> {
        let (view, _) = ipc::read::local_player(memory, layout)?;
        let player = ipc::read::table_get(memory, view + layout.player.players as u64, index)?;
        memory.i32(player + self.rules.player_level as u64)
    }

    fn npc_type(&self, memory: &impl Memory, layout: &Layout, index: i32) -> Option<(i32, bool)> {
        let w = &layout.world;
        let npc = crate::world::actor_at(memory, w, w.npc_table, index)?;
        let definition = memory.ptr(npc + w.npc_definition as u64)?;
        Some((
            memory.i32(definition + self.rules.npc_level as u64)?,
            byte(memory, definition + self.rules.npc_follower as u64)? != 0,
        ))
    }

    fn extra(
        &self,
        memory: &impl Memory,
        layout: &Layout,
        e: u64,
        info: &MenuEntryInfo,
        settings: bool,
    ) -> Option<Extra> {
        let r = &self.rules;
        let key = if self.ordered {
            memory.u64(e + r.order_key as u64)? as i64
        } else {
            0
        };
        if !settings {
            return Some(Extra {
                key,
                ..Extra::default()
            });
        }
        let (level, follower) = match info.kind {
            menu_kind::PLAYER => (self.player_level(memory, layout, info.index), false),
            menu_kind::NPC => match self.npc_type(memory, layout, info.index) {
                Some((level, follower)) => (Some(level), follower),
                None => (None, false),
            },
            _ => (None, false),
        };
        Some(Extra {
            key,
            team: byte(memory, e + r.team as u64)?,
            flag: byte(memory, e + r.flag as u64)?,
            level,
            follower,
        })
    }

    fn raw_entries(
        &self,
        memory: &impl Memory,
        layout: &Layout,
        settings: bool,
    ) -> Option<Vec<(MenuEntryInfo, Extra)>> {
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
            .map(|k| {
                let e = begin + k * size;
                let info = self.entry(memory, e)?;
                let extra = self.extra(memory, layout, e, &info, settings)?;
                Some((info, extra))
            })
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

const PLAYER_INDEX_LIMIT: u32 = 0x7ff;

#[derive(Debug, Clone, Copy, Default)]
struct Extra {
    key: i64,
    team: u8,
    flag: u8,
    level: Option<i32>,
    follower: bool,
}

#[derive(Debug, Clone, Copy)]
struct Settings {
    player_attack: u8,
    npc_attack: u8,
    follower: u8,
    local_index: i32,
    local_level: i32,
    local_team: i32,
}

fn attack_choice(setting: u8, extra: &Extra, settings: &Settings) -> u8 {
    match setting {
        0 if extra.level.unwrap_or(0) <= settings.local_level => 2,
        0 | 1 => 1,
        2 => 2,
        4 if extra.flag != 0 => 1,
        4 => 2,
        _ => 0,
    }
}

fn player_attack(entry: &MenuEntryInfo, extra: &Extra, settings: &Settings) -> u8 {
    if entry.kind != menu_kind::PLAYER
        || settings.player_attack == 3
        || entry.index as u32 > PLAYER_INDEX_LIMIT
    {
        0
    } else if extra.team != 0 && settings.local_team != 0 {
        if i32::from(extra.team) != settings.local_team {
            2
        } else {
            1
        }
    } else {
        attack_choice(settings.player_attack, extra, settings)
    }
}

fn npc_attack(entry: &MenuEntryInfo, extra: &Extra, settings: &Settings) -> u8 {
    if entry.kind != menu_kind::NPC {
        0
    } else {
        attack_choice(settings.npc_attack, extra, settings)
    }
}

fn rank(entry: &MenuEntryInfo, extra: &Extra, settings: Option<&Settings>) -> i8 {
    if entry.cancel != 0 {
        return -4;
    }
    if entry.priority == EXAMINE_PRIORITY {
        return -3;
    }
    if settings.is_some_and(|s| s.follower != 0 && entry.kind == menu_kind::NPC && extra.follower) {
        return -2;
    }
    if entry.kind == menu_kind::WIDGET && entry.category <= 1 {
        return 1;
    }
    let deprioritised = |choice: fn(&MenuEntryInfo, &Extra, &Settings) -> u8| {
        if settings.is_some_and(|s| choice(entry, extra, s) == 1) {
            -2
        } else {
            0
        }
    };
    match entry.category {
        1 => -1,
        2 => -2,
        3 => deprioritised(player_attack),
        4 => deprioritised(npc_attack),
        _ => 0,
    }
}

fn sorted(entries: Vec<(MenuEntryInfo, Extra)>, settings: Option<&Settings>) -> Vec<MenuEntryInfo> {
    let own = |e: &MenuEntryInfo| {
        settings.is_some_and(|s| e.kind == menu_kind::PLAYER && e.index == s.local_index)
    };
    let mut entries: Vec<_> = entries.into_iter().enumerate().collect();
    entries.sort_by(|(ia, (a, xa)), (ib, (b, xb))| {
        rank(b, xb, settings)
            .cmp(&rank(a, xa, settings))
            .then(own(b).cmp(&own(a)))
            .then(a.action.cmp(&b.action))
            .then(xb.key.cmp(&xa.key))
            .then(a.index.cmp(&b.index))
            .then(a.priority.cmp(&b.priority))
            .then(ib.cmp(ia))
    });
    entries.into_iter().map(|(_, (e, _))| e).collect()
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
        .then(|| menu.sorted_entries(memory, layout))
        .flatten()
        .map(|entries| entries.iter().map(Entry::from).collect());
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
            category: 0x65,
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
    fn a_closed_menu_follows_the_clients_order() {
        let mut memory = menu(&[
            ("Cancel", 0, 0, 1),
            ("Walk here", menu_kind::TILE, 0x7fff_ffff, 0),
            ("Follow", 6, 3, 0),
            ("Trade with", 6, 4, 0),
            ("Follow", 6, 3, 0),
            ("Trade with", 6, 4, 0),
            ("Examine", 4, EXAMINE_PRIORITY, 0),
            ("Use", menu_kind::WIDGET, 0, 0),
        ]);
        let mut layout = layout();
        layout.found |= part::MENU_ORDER;
        layout.menu.order_key = 0x70;
        for (i, (index, category)) in [
            (0, 0),
            (0, 0),
            (9, 1),
            (9, 1),
            (4, 1),
            (4, 1),
            (0, 0),
            (0, 0),
        ]
        .into_iter()
        .enumerate()
        {
            let e = ENTRIES + SIZE * i as u64;
            memory.put_i32(e + 0x44, index);
            memory.put(e + 0x65, &[category]);
            memory.put_u64(e + 0x70, 0);
        }
        let read = super::menu(&memory, &layout);
        let rows: Vec<(String, i32)> = read
            .entries
            .unwrap()
            .iter()
            .map(|e| (String::from_utf8_lossy(&e.option).into_owned(), e.index))
            .collect();
        let expected = [
            ("Use", 0),
            ("Walk here", 0),
            ("Follow", 4),
            ("Trade with", 4),
            ("Follow", 9),
            ("Trade with", 9),
            ("Examine", 0),
            ("Cancel", 0),
        ];
        assert_eq!(rows, expected.map(|(o, i)| (o.to_string(), i)).to_vec());
    }

    fn entry(option: &str, kind: u8, category: u8, index: i32, priority: i32) -> MenuEntryInfo {
        let mut e = MenuEntryInfo {
            kind,
            category,
            index,
            priority,
            ..MenuEntryInfo::default()
        };
        e.option[..option.len()].copy_from_slice(option.as_bytes());
        e
    }

    fn level(level: i32) -> Extra {
        Extra {
            level: Some(level),
            ..Extra::default()
        }
    }

    fn settings(player_attack: u8, npc_attack: u8, local_team: i32) -> Settings {
        Settings {
            player_attack,
            npc_attack,
            follower: 1,
            local_index: 7,
            local_level: 50,
            local_team,
        }
    }

    #[test]
    fn attack_options_follow_the_attack_option_settings() {
        let by_level = settings(1, 0, 0);
        let s = Some(&by_level);
        let npc = entry("Attack", menu_kind::NPC, 4, 30, 1);
        assert_eq!(rank(&npc, &level(100), s), -2);
        assert_eq!(rank(&npc, &level(50), s), 0);
        let player = entry("Attack", menu_kind::PLAYER, 3, 9, 1);
        assert_eq!(rank(&player, &level(3), s), -2);
        let left = settings(2, 1, 0);
        assert_eq!(rank(&player, &level(126), Some(&left)), 0);
        assert_eq!(rank(&npc, &level(1), Some(&left)), -2);
        let flagged = Extra {
            flag: 1,
            ..level(1)
        };
        let unreachable = settings(4, 4, 0);
        assert_eq!(rank(&npc, &flagged, Some(&unreachable)), -2);
        assert_eq!(rank(&npc, &level(1), Some(&unreachable)), 0);
        assert_eq!(rank(&npc, &level(100), None), 0);
    }

    #[test]
    fn a_team_mate_is_right_click_and_an_enemy_left_click() {
        let s = settings(0, 0, 5);
        let player = entry("Attack", menu_kind::PLAYER, 3, 2, 1);
        let team = |team: u8| Extra {
            team,
            level: Some(126),
            ..Extra::default()
        };
        assert_eq!(rank(&player, &team(5), Some(&s)), -2);
        assert_eq!(rank(&player, &team(6), Some(&s)), 0);
    }

    #[test]
    fn followers_and_player_options_rank_below_walk_here() {
        let s = settings(0, 0, 0);
        let pet = Extra {
            follower: true,
            ..Extra::default()
        };
        let walk = entry("Walk here", menu_kind::TILE, 0, -1, WALK);
        assert_eq!(rank(&walk, &Extra::default(), Some(&s)), 0);
        assert_eq!(
            rank(&entry("Pick-up", menu_kind::NPC, 0, 40, 1), &pet, Some(&s)),
            -2
        );
        assert_eq!(
            rank(
                &entry("Follow", menu_kind::PLAYER, 1, 9, 3),
                &Extra::default(),
                Some(&s)
            ),
            -1
        );
        assert_eq!(
            rank(
                &entry("Use", menu_kind::WIDGET, 0, 0, 0),
                &Extra::default(),
                Some(&s)
            ),
            1
        );
    }

    #[test]
    fn your_own_player_comes_first_among_equals() {
        let s = settings(0, 0, 0);
        let read: Vec<i32> = sorted(
            vec![
                (
                    entry("Follow", menu_kind::PLAYER, 1, 9, 3),
                    Extra::default(),
                ),
                (
                    entry("Follow", menu_kind::PLAYER, 1, 7, 3),
                    Extra::default(),
                ),
            ],
            Some(&s),
        )
        .iter()
        .map(|e| e.index)
        .collect();
        assert_eq!(read, [7, 9]);
    }

    const WALK: i32 = 0x7fff_ffff;

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
