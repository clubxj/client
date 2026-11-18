//! The game thread's frame (see `offsets::frame`). The per-frame work that reads client memory
//! (drawn models, act, menu) runs right after the client's draw step, on the game thread, in
//! every renderer mode. The swap hook keeps only the OpenGL work, because some modes present
//! from a render thread of their own.

use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering},
        OnceLock,
    },
};

use ipc::read::Memory;
use retour::GenericDetour;

use super::{
    detour,
    offsets::Frame,
    own::{note_game_thread, OwnMemory},
    unload::HookCall,
};

type Step = unsafe extern "system" fn(u64, f32);

static TICK: OnceLock<GenericDetour<Step>> = OnceLock::new();
static DRAW: OnceLock<GenericDetour<Step>> = OnceLock::new();
static DRAW_SLOT: AtomicU32 = AtomicU32::new(0);
static IMAGE: OnceLock<(u64, u64)> = OnceLock::new();
static DRAW_TRIED: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static OFFSET_X: AtomicI32 = AtomicI32::new(0);
static OFFSET_Y: AtomicI32 = AtomicI32::new(0);

/// Whether the draw step hook runs the per-frame work; until then the swap hook does.
pub fn active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

/// The viewport's offset in the window, top-down, as the swap hook last read it.
pub fn set_viewport_offset(x: i32, y: i32) {
    OFFSET_X.store(x, Ordering::Relaxed);
    OFFSET_Y.store(y, Ordering::Relaxed);
}

/// The frame's work on client memory: publish the drawn models, advance the act job, copy the
/// finished menu for the worker.
pub fn on_frame(offset: (i32, i32)) {
    let _ = catch_unwind(super::drawn::on_frame);
    let _ = catch_unwind(|| super::act::on_frame(offset));
    let _ = catch_unwind(super::menu::capture);
}

unsafe extern "system" fn draw(app: u64, delta: f32) {
    let _call = HookCall::enter();
    DRAW.get().unwrap().call(app, delta);
    note_game_thread();
    let offset = (
        OFFSET_X.load(Ordering::Relaxed),
        OFFSET_Y.load(Ordering::Relaxed),
    );
    on_frame(offset);
}

unsafe fn hook_draw(app: u64) {
    let memory = OwnMemory;
    let slot = DRAW_SLOT.load(Ordering::Relaxed) as u64;
    let Some(&(base, end)) = IMAGE.get() else {
        return;
    };
    let Some(address) = memory
        .ptr(app)
        .and_then(|vtable| memory.ptr(vtable + slot))
        .filter(|&address| (base..end).contains(&address))
    else {
        println!("[Agent]: The frame's draw step isn't in the client image.\r\n");
        return;
    };
    let Some(detour) = detour::create::<Step>(address, draw, "frame draw step") else {
        return;
    };
    let detour = DRAW.get_or_init(|| detour);
    match detour.enable() {
        Ok(()) => {
            ACTIVE.store(true, Ordering::Release);
            println!(
                "[Agent]: Frame draw step hooked at +{:#x}.\r\n",
                address - base
            );
        }
        Err(error) => println!("[Agent]: Failed to enable the frame draw step hook: {error}.\r\n"),
    }
}

/// Hooks the draw step from the first tick, where the app object is at hand, so the draw
/// step's address comes from its vtable rather than being guessed.
unsafe extern "system" fn tick(app: u64, delta: f32) {
    let _call = HookCall::enter();
    if !DRAW_TRIED.swap(true, Ordering::AcqRel) {
        let _ = catch_unwind(AssertUnwindSafe(|| hook_draw(app)));
    }
    TICK.get().unwrap().call(app, delta)
}

pub unsafe fn hook(module_base: u64, image_size: u64, rule: Frame) {
    if TICK.get().is_some() {
        return;
    }
    let _ = IMAGE.set((module_base, module_base + image_size));
    DRAW_SLOT.store(rule.draw_slot, Ordering::Relaxed);
    let Some(detour) = detour::create::<Step>(module_base + rule.tick as u64, tick, "frame tick")
    else {
        return;
    };
    detour::enable(
        TICK.get_or_init(|| detour),
        "frame tick hook",
        "Frame tick hooked",
    );
}

/// Leaves `active` set: once the draw step took the per-frame work, the swap hook must not take
/// it back on a render thread while the agent unloads.
pub unsafe fn unhook() {
    detour::disable(
        DRAW.get(),
        "frame draw step hook",
        Some("Frame draw step unhooked"),
    );
    detour::disable(TICK.get(), "frame tick hook", Some("Frame tick unhooked"));
}
