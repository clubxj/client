use ipc::state::Layout;
use serde::{Deserialize, Serialize};

use crate::memory::{byte, read_string, Memory};

const MAX_WORLDS: i32 = 2048;
const ENTRY_SIZE: u64 = 16;
const ENTRY_WORLD: u64 = 8;
const LIST_COUNT: u64 = 0x10;
const LIST_DATA: u64 = 0x18;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct World {
    pub id: i32,
    pub types: u32,
    pub players: i32,
    pub location: i32,
    pub activity: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Panel {
    pub page: i32,
    pub compact: bool,
    pub selected: i32,
    pub hovered: i32,
    pub rows: i32,
    pub columns: i32,
    pub list: Rect,
    pub row_offset: i32,
    pub row_height: i32,
    pub previous: Rect,
    pub next: Rect,
    pub shown: Vec<World>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldSelect {
    pub open: bool,
    pub current: i32,
    pub worlds: Vec<World>,
    pub panel: Option<Panel>,
}

fn world(memory: &impl Memory, layout: &Layout, address: u64) -> Option<World> {
    let w = &layout.world_select;
    Some(World {
        id: memory.i32(address + w.id as u64)?,
        types: memory.i32(address + w.types as u64)? as u32,
        players: memory.i32(address + w.players as u64)?,
        location: memory.i32(address + w.location as u64)?,
        activity: read_string(memory, address + w.activity as u64).unwrap_or_default(),
    })
}

fn entries(memory: &impl Memory, layout: &Layout, count: i32, data: u64) -> Option<Vec<World>> {
    if !(0..=MAX_WORLDS).contains(&count) || (count > 0 && data == 0) {
        return Some(Vec::new());
    }
    (0..count as u64)
        .map(|i| {
            let address = memory.ptr(data + i * ENTRY_SIZE + ENTRY_WORLD)?;
            world(memory, layout, address)
        })
        .collect()
}

fn rect(memory: &impl Memory, address: u64) -> Option<Rect> {
    Some(Rect {
        x: memory.i32(address)?,
        y: memory.i32(address + 4)?,
        width: memory.i32(address + 8)?,
        height: memory.i32(address + 12)?,
    })
}

fn panel(memory: &impl Memory, layout: &Layout, switcher: u64) -> Option<Panel> {
    let w = &layout.world_select;
    let state = switcher + w.state as u64;
    let sizes = memory.ptr(switcher + w.layout as u64).filter(|&p| p != 0)?;
    let compact = byte(memory, state + w.compact as u64)? != 0;
    let columns = if compact { w.compact_columns } else { w.columns };
    let list = switcher + w.list as u64;
    let shown_count = memory.i32(list + LIST_COUNT)?;
    let shown_data = memory.ptr(list + LIST_DATA).unwrap_or(0);
    Some(Panel {
        page: memory.i32(state + w.page as u64)?,
        compact,
        selected: memory.i32(state + w.selected as u64)?,
        hovered: memory.i32(state + w.hovered as u64)?,
        rows: memory.i32(sizes + w.rows as u64)?,
        columns: memory.i32(sizes + columns as u64)?,
        list: rect(memory, sizes + w.list_rect as u64)?,
        row_offset: memory.i32(sizes + w.row_offset as u64)?,
        row_height: memory.i32(sizes + w.row_height as u64)?,
        previous: rect(memory, sizes + w.previous as u64)?,
        next: rect(memory, sizes + w.next as u64)?,
        shown: entries(memory, layout, shown_count, shown_data)?,
    })
}

pub fn world_select(memory: &impl Memory, layout: &Layout, client: u64) -> Option<WorldSelect> {
    let w = &layout.world_select;
    let switcher = memory.ptr(layout.module_base + w.switcher as u64).unwrap_or(0);
    let open = switcher != 0 && byte(memory, switcher + w.open as u64)? != 0;
    let count = memory.i32(layout.module_base + w.master_count as u64)?;
    let data = memory.ptr(layout.module_base + w.master_worlds as u64).unwrap_or(0);
    Some(WorldSelect {
        open,
        current: memory.i32(client + w.current_world as u64)?,
        worlds: entries(memory, layout, count, data)?,
        panel: if open { panel(memory, layout, switcher) } else { None },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Fake;
    use ipc::state::WorldSelectLayout;

    const BASE: u64 = 0x1_4000_0000;
    const CLIENT: u64 = 0x5000_0000;
    const SWITCHER: u64 = 0x6000_0000;
    const SIZES: u64 = 0x6100_0000;

    fn layout() -> Layout {
        Layout {
            module_base: BASE,
            world_select: WorldSelectLayout {
                switcher: 0x1c21a68,
                current_world: 0x32fc,
                master_count: 0x15a7a38,
                master_worlds: 0x15a7a40,
                open: 0,
                list: 8,
                state: 0x78,
                layout: 0x180,
                selected: 0x58,
                hovered: 0x5c,
                page: 0x60,
                compact: 0x64,
                list_rect: 0x10,
                row_offset: 0xd0,
                row_height: 0xd4,
                rows: 0xe8,
                columns: 0xec,
                compact_columns: 0xf0,
                previous: 0x40,
                next: 0x50,
                id: 0,
                types: 0x4,
                players: 0x8,
                activity: 0x28,
                location: 0x40,
                reserved: 0,
            },
            ..Layout::default()
        }
    }

    fn put_world(m: &mut Fake, address: u64, id: i32, types: u32, players: i32, activity: &str) {
        m.put_i32(address, id);
        m.put(address + 4, &types.to_le_bytes());
        m.put_i32(address + 8, players);
        m.put_short_string(address + 0x28, activity);
        m.put_i32(address + 0x40, 7);
    }

    fn put_rect(m: &mut Fake, address: u64, rect: [i32; 4]) {
        for (k, value) in rect.into_iter().enumerate() {
            m.put_i32(address + 4 * k as u64, value);
        }
    }

    fn master(m: &mut Fake) {
        m.put_i32(CLIENT + 0x32fc, 304);
        m.put_i32(BASE + 0x15a7a38, 2);
        m.put_u64(BASE + 0x15a7a40, 0x7000);
        m.put_u64(0x7008, 0x8000);
        m.put_u64(0x7018, 0x9000);
        put_world(m, 0x8000, 301, 0, 796, "Trade - Free");
        put_world(m, 0x9000, 345, 0x2000_0001, 59, "Deadman");
    }

    #[test]
    fn reads_worlds_while_closed() {
        let mut m = Fake::default();
        master(&mut m);
        let select = world_select(&m, &layout(), CLIENT).unwrap();
        assert!(!select.open);
        assert_eq!(select.current, 304);
        assert_eq!(select.panel, None);
        assert_eq!(
            select.worlds.iter().map(|w| (w.id, w.players, w.location)).collect::<Vec<_>>(),
            [(301, 796, 7), (345, 59, 7)]
        );
        assert_eq!(select.worlds[1].types, 0x2000_0001);
        assert_eq!(select.worlds[0].activity, "Trade - Free");
    }

    #[test]
    fn reads_the_open_panel() {
        let mut m = Fake::default();
        master(&mut m);
        m.put_u64(BASE + 0x1c21a68, SWITCHER);
        m.put(SWITCHER, &[1]);
        m.put_i32(SWITCHER + 8 + LIST_COUNT, 1);
        m.put_u64(SWITCHER + 8 + LIST_DATA, 0xa000);
        m.put_u64(0xa008, 0x9000);
        m.put_i32(SWITCHER + 0x78 + 0x58, 1);
        m.put_i32(SWITCHER + 0x78 + 0x5c, -1);
        m.put_i32(SWITCHER + 0x78 + 0x60, 3);
        m.put(SWITCHER + 0x78 + 0x64, &[1]);
        m.put_u64(SWITCHER + 0x180, SIZES);
        put_rect(&mut m, SIZES + 0x10, [4, 27, 552, 472]);
        put_rect(&mut m, SIZES + 0x40, [9, 458, 45, 36]);
        put_rect(&mut m, SIZES + 0x50, [506, 458, 45, 36]);
        m.put_i32(SIZES + 0xd0, 23);
        m.put_i32(SIZES + 0xd4, 19);
        m.put_i32(SIZES + 0xe8, 21);
        m.put_i32(SIZES + 0xec, 2);
        m.put_i32(SIZES + 0xf0, 6);

        let select = world_select(&m, &layout(), CLIENT).unwrap();
        assert!(select.open);
        let panel = select.panel.unwrap();
        assert_eq!(panel.page, 3);
        assert!(panel.compact);
        assert_eq!(panel.columns, 6);
        assert_eq!(panel.rows, 21);
        assert_eq!((panel.selected, panel.hovered), (1, -1));
        assert_eq!(
            panel.list,
            Rect {
                x: 4,
                y: 27,
                width: 552,
                height: 472
            }
        );
        assert_eq!((panel.row_offset, panel.row_height), (23, 19));
        assert_eq!(panel.next.x, 506);
        assert_eq!(panel.shown.iter().map(|w| w.id).collect::<Vec<_>>(), [345]);
    }
}
