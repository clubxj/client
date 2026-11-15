use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, AtomicI32, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread::sleep,
    time::{Duration, Instant},
};

use decode::{camera::Camera, model::Geometry};
use ipc::read::Memory;
use retour::GenericDetour;

use super::{
    detour::{self, function},
    items::release,
    model::geometry,
    offsets::Render,
    own::{OwnMemory, LAYOUT},
    unload::HookCall,
};

type Draw = unsafe extern "system" fn(
    u64,
    u64,
    i32,
    f32,
    f32,
    f32,
    f32,
    i32,
    i32,
    i32,
    u64,
    i32,
    u64,
    u64,
    u8,
) -> u64;
type Pass = unsafe extern "system" fn(u64, u64, u64, u64) -> u8;
type AnimationHeight = unsafe extern "system" fn(u64) -> i32;

const CONTROL: u64 = 0x20;
const USES: u64 = 8;
const MAX_DRAWN: usize = 8192;
pub const MAX_AGE: Duration = Duration::from_secs(1);
const STOP_TIMEOUT: Duration = Duration::from_secs(1);

static DRAW: OnceLock<GenericDetour<Draw>> = OnceLock::new();
static PASS: OnceLock<GenericDetour<Pass>> = OnceLock::new();
static ANIMATION_SLOT: OnceLock<u32> = OnceLock::new();
static TRANSIENT: OnceLock<u32> = OnceLock::new();
static PENDING: Mutex<Option<Pending>> = Mutex::new(None);
static BUILDING: Mutex<Vec<Drawn>> = Mutex::new(Vec::new());
static PUBLISHED: Mutex<Option<Arc<Frame>>> = Mutex::new(None);
static RETIRED: Mutex<Vec<Arc<Frame>>> = Mutex::new(Vec::new());
static STOPPING: AtomicBool = AtomicBool::new(false);
static STOPPED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
struct Pending {
    renderable: u64,
    orientation: i32,
    at: [i32; 3],
}

pub enum Shape {
    Kept(u64),
    Copied(Geometry),
}

pub struct Drawn {
    pub renderable: u64,
    pub shape: Shape,
    pub orientation: i32,
    pub at: [i32; 3],
}

pub struct Frame {
    at: Instant,
    pub camera: Option<Camera>,
    pub drawn: Vec<Drawn>,
}

impl Frame {
    pub fn find(&self, renderable: u64) -> Option<&Drawn> {
        self.drawn.iter().find(|d| d.renderable == renderable)
    }
}

pub fn latest() -> Option<Arc<Frame>> {
    PUBLISHED
        .lock()
        .ok()?
        .clone()
        .filter(|frame| frame.at.elapsed() <= MAX_AGE)
}

fn set_pending(pending: Option<Pending>) {
    if let Ok(mut slot) = PENDING.lock() {
        *slot = pending;
    }
}

unsafe fn retain(model: u64) -> bool {
    let Some(control) = model.checked_sub(CONTROL) else {
        return false;
    };
    let uses = &*((control + USES) as *const AtomicI32);
    uses.try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
        (n > 0).then_some(n + 1)
    })
    .is_ok()
}

unsafe fn release_all(drawn: &[Drawn]) {
    for d in drawn {
        if let Shape::Kept(model) = d.shape {
            release(model - CONTROL);
        }
    }
}

unsafe fn shape(model: u64) -> Option<Shape> {
    let transient = TRANSIENT
        .get()
        .and_then(|&flag| OwnMemory.u64(model + flag as u64))?;
    if transient as u8 != 0 {
        geometry(model).map(Shape::Copied)
    } else {
        retain(model).then_some(Shape::Kept(model))
    }
}

unsafe fn lift(renderable: u64) -> i32 {
    let memory = OwnMemory;
    let height = ANIMATION_SLOT.get().and_then(|&slot| {
        let vtable = memory.ptr(renderable)?;
        memory.ptr(vtable + slot as u64)
    });
    height.map_or(0, |height| function::<AnimationHeight>(height)(renderable))
}

unsafe fn record(renderable: u64, model: u64) {
    let Some(pending) = PENDING
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
        .filter(|p| p.renderable == renderable && model != 0)
    else {
        return;
    };
    let Ok(mut building) = BUILDING.lock() else {
        return;
    };
    if building.len() >= MAX_DRAWN {
        return;
    }
    let Some(shape) = shape(model) else {
        return;
    };
    let [x, height, y] = pending.at;
    building.push(Drawn {
        renderable,
        shape,
        orientation: pending.orientation,
        at: [x, height - lift(renderable), y],
    });
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn draw(
    renderer: u64,
    renderable: u64,
    orientation: i32,
    pitch_sin: f32,
    pitch_cos: f32,
    yaw_sin: f32,
    yaw_cos: f32,
    x: i32,
    height: i32,
    y: i32,
    tag: u64,
    cycle: i32,
    extra: u64,
    target: u64,
    flag: u8,
) -> u64 {
    let _call = HookCall::enter();
    let recording = !STOPPING.load(Ordering::Acquire);
    if recording {
        set_pending(Some(Pending {
            renderable,
            orientation,
            at: [x, height, y],
        }));
    }
    let result = DRAW.get().unwrap().call(
        renderer,
        renderable,
        orientation,
        pitch_sin,
        pitch_cos,
        yaw_sin,
        yaw_cos,
        x,
        height,
        y,
        tag,
        cycle,
        extra,
        target,
        flag,
    );
    if recording {
        set_pending(None);
    }
    result
}

unsafe extern "system" fn pass(renderer: u64, renderable: u64, model: u64, tag: u64) -> u8 {
    let _call = HookCall::enter();
    let _ = catch_unwind(AssertUnwindSafe(|| record(renderable, model)));
    PASS.get().unwrap().call(renderer, renderable, model, tag)
}

unsafe fn release_retired(retired: &mut Vec<Arc<Frame>>) {
    retired.retain(|frame| {
        if Arc::strong_count(frame) > 1 {
            return true;
        }
        release_all(&frame.drawn);
        false
    });
}

pub fn on_frame() {
    let drawn = BUILDING
        .lock()
        .map(|mut building| std::mem::take(&mut *building))
        .unwrap_or_default();
    let (Ok(mut published), Ok(mut retired)) = (PUBLISHED.lock(), RETIRED.lock()) else {
        return;
    };
    if STOPPING.load(Ordering::Acquire) {
        unsafe { release_all(&drawn) };
        retired.extend(published.take());
        unsafe { release_retired(&mut retired) };
        STOPPED.store(retired.is_empty(), Ordering::Release);
        return;
    }
    let camera = LAYOUT
        .get()
        .and_then(|layout| Camera::current(&OwnMemory, layout));
    let frame = Arc::new(Frame {
        at: Instant::now(),
        camera,
        drawn,
    });
    retired.extend(published.replace(frame));
    unsafe { release_retired(&mut retired) };
}

pub unsafe fn hook(module_base: u64, rule: Render, animation_slot: u32) {
    if DRAW.get().is_some() {
        return;
    }
    let _ = ANIMATION_SLOT.set(animation_slot);
    let _ = TRANSIENT.set(rule.transient);
    let (Some(draw), Some(pass)) = (
        detour::create::<Draw>(module_base + rule.draw as u64, draw, "model draw"),
        detour::create::<Pass>(module_base + rule.pass as u64, pass, "model pass"),
    ) else {
        return;
    };
    let (draw, pass) = (DRAW.get_or_init(|| draw), PASS.get_or_init(|| pass));
    detour::enable_both(pass, draw, "model draw", "Model draws hooked");
}

pub fn stop() {
    if DRAW.get().is_none_or(|detour| !detour.is_enabled()) {
        return;
    }
    STOPPING.store(true, Ordering::Release);
    let start = Instant::now();
    while !STOPPED.load(Ordering::Acquire) {
        if start.elapsed() > STOP_TIMEOUT {
            println!("[Agent]: No frame was drawn to release the drawn models, leaving them.\r\n");
            return;
        }
        sleep(Duration::from_millis(10));
    }
}

pub unsafe fn unhook() {
    detour::disable(DRAW.get(), "model draw hook", None);
    detour::disable(PASS.get(), "model pass hook", Some("Model draws unhooked"));
}
