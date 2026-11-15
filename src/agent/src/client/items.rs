use std::sync::atomic::{AtomicI32, Ordering};

use ipc::read::Memory;

use super::{detour::function, own::OwnMemory};

const USES: u64 = 0x8;
const WEAKS: u64 = 0xc;
const DISPOSE: u64 = 0x8;
const DELETE: u64 = 0x10;

type Release = unsafe extern "system" fn(u64);

pub(super) unsafe fn release(control: u64) {
    if control == 0 {
        return;
    }
    let memory = OwnMemory;
    let Some(vtable) = memory.ptr(control) else {
        return;
    };
    let drop_one = |count: u64, call: u64| {
        let count = &*((control + count) as *const AtomicI32);
        if count.fetch_sub(1, Ordering::AcqRel) != 1 {
            return false;
        }
        if let Some(call) = memory.ptr(vtable + call) {
            function::<Release>(call)(control);
        }
        true
    };
    if drop_one(USES, DISPOSE) {
        drop_one(WEAKS, DELETE);
    }
}
