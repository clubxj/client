use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    thread::JoinHandle,
    time::{Duration, Instant},
};

use decode::{
    definitions::{decode_loc_text, decode_varbit, Transforms, VarbitDef},
    js5::{Store, DATA_FILE},
    locs::Unavailable,
};
use windows::Win32::{
    Foundation::HANDLE,
    Storage::FileSystem::{
        GetFileType, GetFinalPathNameByHandleW, FILE_NAME_NORMALIZED, FILE_TYPE_DISK,
    },
};

const CONFIG_INDEX: u8 = 2;
const LOC_GROUP: u32 = 6;
const VARBIT_GROUP: u32 = 14;
const HANDLE_SCAN_LIMIT: usize = 0x4000;
const NULL_NAME: &str = "null";
const RETRY: Duration = Duration::from_secs(30);

pub struct Loc {
    pub name: Option<String>,
    pub decoration_height: u8,
    pub transforms: Option<Transforms>,
}

pub struct Definitions {
    pub locs: HashMap<u32, Loc>,
    pub varbits: HashMap<u32, VarbitDef>,
}

struct Loader {
    thread: Option<JoinHandle<()>>,
    stopped: bool,
}

static DEFINITIONS: OnceLock<Definitions> = OnceLock::new();
static LOADER: Mutex<Loader> = Mutex::new(Loader {
    thread: None,
    stopped: false,
});
static FAILED_AT: Mutex<Option<Instant>> = Mutex::new(None);

pub fn get() -> Result<&'static Definitions, Unavailable> {
    if let Some(definitions) = DEFINITIONS.get() {
        return Ok(definitions);
    }
    match FAILED_AT.lock().ok().and_then(|failed| *failed) {
        Some(at) if at.elapsed() < RETRY => Err(Unavailable::DefinitionsFailed),
        Some(_) => {
            start();
            Err(Unavailable::DefinitionsLoading)
        }
        None => Err(Unavailable::DefinitionsLoading),
    }
}

fn set_failed(at: Option<Instant>) {
    if let Ok(mut failed) = FAILED_AT.lock() {
        *failed = at;
    }
}

fn cache_dir() -> Option<PathBuf> {
    let mut buffer = [0u16; 1024];
    (4..HANDLE_SCAN_LIMIT).step_by(4).find_map(|value| {
        let handle = HANDLE(value as *mut _);
        if unsafe { GetFileType(handle) } != FILE_TYPE_DISK {
            return None;
        }
        let len = unsafe { GetFinalPathNameByHandleW(handle, &mut buffer, FILE_NAME_NORMALIZED) }
            as usize;
        if len == 0 || len >= buffer.len() {
            return None;
        }
        let path = String::from_utf16_lossy(&buffer[..len]);
        let path = PathBuf::from(path.strip_prefix(r"\\?\").unwrap_or(&path));
        (path.file_name()? == DATA_FILE)
            .then(|| path.parent().map(PathBuf::from))
            .flatten()
    })
}

fn load() -> Option<Definitions> {
    let dir = cache_dir()?;
    let store = Store::open(&dir)?;
    let locs = store
        .files(CONFIG_INDEX, LOC_GROUP)?
        .into_iter()
        .filter_map(|(id, data)| {
            let (def, text) = decode_loc_text(&data)?;
            let name = Some(text.name).filter(|n| !n.is_empty() && n != NULL_NAME);
            Some((
                id,
                Loc {
                    name,
                    decoration_height: def.decoration_height,
                    transforms: text.transforms,
                },
            ))
        })
        .collect();
    let varbits = store
        .files(CONFIG_INDEX, VARBIT_GROUP)?
        .into_iter()
        .filter_map(|(id, data)| Some((id, decode_varbit(&data)?)))
        .collect();
    Some(Definitions { locs, varbits })
}

pub fn start() {
    let Ok(mut loader) = LOADER.lock() else {
        return;
    };
    if loader.stopped || loader.thread.as_ref().is_some_and(|t| !t.is_finished()) {
        return;
    }
    if let Some(finished) = loader.thread.take() {
        let _ = finished.join();
    }
    set_failed(None);
    loader.thread = Some(std::thread::spawn(|| {
        let started = Instant::now();
        match std::panic::catch_unwind(load).ok().flatten() {
            Some(definitions) => {
                println!(
                    "[Agent]: Read {} loc and {} varbit definitions from the cache in {:?}.\r\n",
                    definitions.locs.len(),
                    definitions.varbits.len(),
                    started.elapsed()
                );
                let _ = DEFINITIONS.set(definitions);
            }
            None => {
                println!("[Agent]: Cannot read loc definitions from the cache.\r\n");
                set_failed(Some(Instant::now()));
            }
        }
    }));
}

pub fn stop() {
    let thread = LOADER.lock().ok().and_then(|mut loader| {
        loader.stopped = true;
        loader.thread.take()
    });
    if let Some(thread) = thread {
        let _ = thread.join();
    }
}
