use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{HANDLE, WAIT_OBJECT_0},
        System::Threading::{CreateEventW, WaitForMultipleObjects, INFINITE},
    },
};

use ipc::{unload_event_name, work_event_name, worker_reply_event_name};

use super::windows::wide;
use crate::client::unload::unload;

pub struct Events {
    unload: HANDLE,
    work: HANDLE,
    reply: HANDLE,
}

impl Events {
    pub fn create(pid: u32) -> Option<Self> {
        let create = |name: String| {
            let name = wide(&name);
            unsafe { CreateEventW(None, false, false, PCWSTR(name.as_ptr())) }.ok()
        };
        Some(Self {
            unload: create(unload_event_name(pid))?,
            work: create(work_event_name(pid))?,
            reply: create(worker_reply_event_name(pid))?,
        })
    }

    pub fn listen(self) {
        let events = [self.unload, self.work];
        loop {
            let signaled = unsafe { WaitForMultipleObjects(&events, false, INFINITE) };
            if signaled == WAIT_OBJECT_0 {
                unsafe { unload(&[self.unload, self.work, self.reply]) };
            } else if signaled.0 == WAIT_OBJECT_0.0 + 1 {
                let _ = std::panic::catch_unwind(|| crate::client::worker::on_work(self.reply));
            } else {
                println!("[Agent]: Waiting for events failed, stopping the listener.\r\n");
                return;
            }
        }
    }
}
