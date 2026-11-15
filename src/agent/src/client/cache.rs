use ipc::{
    read::Memory,
    state::{
        cache_status, CacheReply, CacheRequest, CACHE_BATCH, CACHE_DATA_SIZE, CACHE_FILE_MISSING,
        CACHE_FILE_NO_ROOM, ITEM_ICON_HEIGHT, ITEM_ICON_ID_MASK, ITEM_ICON_SHOW_COUNT,
        ITEM_ICON_WIDTH,
    },
};
use std::sync::OnceLock;

use super::{
    detour::function,
    items::release,
    own::{on_other_thread, OwnMemory},
    state::mapping,
};

type IconBuilder = unsafe extern "system" fn(
    *mut [u64; 2],
    i32,
    i32,
    i32,
    i32,
    i32,
    u8,
    i32,
    i32,
    *const u8,
) -> *mut [u64; 2];

static ICON_BUILDER: OnceLock<IconBuilder> = OnceLock::new();

const ICON_BORDER: i32 = 1;
const ICON_SHADOW: i32 = 0x30_2020;
const ICON_TEXT_OFF: i32 = 0;
const ICON_TEXT_COUNT: i32 = 1;
const EMPTY_TEXT: [u8; 0x18] = {
    let mut text = [0u8; 0x18];
    text[0x17] = 0x17;
    text
};
const SPRITE_COUNT: u64 = 0x0;
const SPRITE_PIXELS: u64 = 0x10;
const ICON_BYTES: usize = (ITEM_ICON_WIDTH * ITEM_ICON_HEIGHT * 4) as usize;
const MAX_ITEM_ID: i32 = 1 << 16;

pub unsafe fn set_item_icon(module_base: u64, rva: u32) {
    let _ = ICON_BUILDER.set(function(module_base + rva as u64));
}

pub fn on_request() {
    let Some(mapping) = mapping() else { return };
    let shared = mapping.shared();
    let Some((sequence, request)) = shared.read_cache_request() else {
        return;
    };
    shared.write_cache_reply_with(|reply| {
        reply.sequence = sequence;
        reply.count = 0;
        reply.used = 0;
        reply.status = fill(reply, &request);
    });
}

fn fill(reply: &mut CacheReply, request: &CacheRequest) -> i32 {
    if request.count as usize > CACHE_BATCH {
        return cache_status::TOO_MANY;
    }
    if on_other_thread() {
        return cache_status::OTHER_THREAD;
    }
    match ICON_BUILDER.get() {
        Some(&builder) => unsafe { draw_icons(&OwnMemory, builder, request, reply) },
        None => cache_status::NOT_FOUND,
    }
}

enum Copied {
    Bytes(usize),
    Missing,
    NoRoom,
}

fn copy_each(
    reply: &mut CacheReply,
    request: &CacheRequest,
    mut copy: impl FnMut([i32; 2], &mut [u8]) -> Copied,
) -> i32 {
    let count = request.count as usize;
    reply.count = count as u32;
    let mut full = false;
    for (slot, &pair) in reply.files[..count].iter_mut().zip(&request.files[..count]) {
        slot.offset = reply.used;
        slot.length = CACHE_FILE_NO_ROOM;
        if full {
            continue;
        }
        match copy(pair, &mut reply.data[reply.used as usize..CACHE_DATA_SIZE]) {
            Copied::Bytes(length) => {
                slot.length = length as i32;
                reply.used += length as u32;
            }
            Copied::Missing => slot.length = CACHE_FILE_MISSING,
            Copied::NoRoom => full = true,
        }
    }
    cache_status::OK
}

unsafe fn draw_icons(
    memory: &OwnMemory,
    builder: IconBuilder,
    request: &CacheRequest,
    reply: &mut CacheReply,
) -> i32 {
    copy_each(reply, request, |[id, quantity], room| {
        if ICON_BYTES > room.len() {
            return Copied::NoRoom;
        }
        let text = if id & ITEM_ICON_SHOW_COUNT != 0 {
            ICON_TEXT_COUNT
        } else {
            ICON_TEXT_OFF
        };
        let id = id & ITEM_ICON_ID_MASK;
        if !(0..MAX_ITEM_ID).contains(&id) {
            return Copied::Missing;
        }
        let mut out = [0u64; 2];
        builder(
            &mut out,
            id,
            quantity,
            ICON_BORDER,
            ICON_SHADOW,
            text,
            0,
            ITEM_ICON_WIDTH as i32,
            ITEM_ICON_HEIGHT as i32,
            EMPTY_TEXT.as_ptr(),
        );
        let [control, sprite] = out;
        let copied = (sprite != 0)
            .then(|| memory.u64(sprite + SPRITE_COUNT))
            .flatten()
            .filter(|&count| count == (ITEM_ICON_WIDTH * ITEM_ICON_HEIGHT) as u64)
            .and_then(|_| memory.ptr(sprite + SPRITE_PIXELS))
            .is_some_and(|pixels| memory.read(pixels, &mut room[..ICON_BYTES]));
        release(control);
        if copied {
            Copied::Bytes(ICON_BYTES)
        } else {
            Copied::Missing
        }
    })
}
