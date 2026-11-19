use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use decode::interface_mask::{interface_rects, Rect};

use super::own::{OwnMemory, LAYOUT};

const REFRESH: Duration = Duration::from_millis(100);

pub struct Mask {
    pub count: usize,
    pub rects: [Rect; ipc::MASK_RECTS],
}

static MASK: Mutex<Mask> = Mutex::new(Mask {
    count: 0,
    rects: [[0; 4]; ipc::MASK_RECTS],
});
static WANTED: AtomicU64 = AtomicU64::new(0);
static STOPPED: AtomicBool = AtomicBool::new(false);
static THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

pub fn want(viewport: Option<[i32; 2]>) {
    let packed = viewport.map_or(0, |[width, height]| {
        u64::from(width as u32) << 32 | u64::from(height as u32)
    });
    WANTED.store(packed, Ordering::Relaxed);
    if packed != 0 {
        start();
    }
}

pub fn with_current<T>(read: impl FnOnce(&[Rect]) -> T) -> T {
    let mask = MASK.lock().unwrap_or_else(|e| e.into_inner());
    read(&mask.rects[..mask.count])
}

fn wanted() -> Option<[i32; 2]> {
    let packed = WANTED.load(Ordering::Relaxed);
    (packed != 0).then_some([(packed >> 32) as u32 as i32, packed as u32 as i32])
}

fn start() {
    let Ok(mut thread) = THREAD.lock() else {
        return;
    };
    if STOPPED.load(Ordering::Relaxed) || thread.is_some() {
        return;
    }
    *thread = thread::Builder::new()
        .name("interface-mask".into())
        .spawn(refresh_loop)
        .ok();
}

fn refresh_loop() {
    while !STOPPED.load(Ordering::Relaxed) {
        let rects = match (wanted(), LAYOUT.get()) {
            (Some(viewport), Some(layout)) => std::panic::catch_unwind(|| {
                decode::worker::widget_tree(&OwnMemory, layout)
                    .map(|tree| interface_rects(&tree, viewport))
                    .unwrap_or_default()
            })
            .unwrap_or_default(),
            _ => Vec::new(),
        };
        if let Ok(mut mask) = MASK.lock() {
            let count = rects.len().min(ipc::MASK_RECTS);
            mask.rects[..count].copy_from_slice(&rects[..count]);
            mask.count = count;
        }
        thread::park_timeout(REFRESH);
    }
}

pub fn stop() {
    STOPPED.store(true, Ordering::Relaxed);
    let thread = THREAD.lock().ok().and_then(|mut thread| thread.take());
    if let Some(thread) = thread {
        thread.thread().unpark();
        let _ = thread.join();
    }
}
