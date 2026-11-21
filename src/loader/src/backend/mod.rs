use std::path::{Path, PathBuf};

use injector::Client;
use serde::{Deserialize, Serialize};

#[cfg(windows)]
#[path = "native.rs"]
mod imp;
#[cfg(not(windows))]
#[path = "wine.rs"]
mod imp;
pub use imp::*;

fn run_args(
    command: &mut std::process::Command,
    script: Option<impl AsRef<std::ffi::OsStr>>,
    lib: Option<impl AsRef<std::ffi::OsStr>>,
) {
    command.arg("run");
    if let Some(script) = script {
        command.arg("--script").arg(script);
        if let Some(lib) = lib {
            command.arg("--lib").arg(lib);
        }
    }
}

pub const LOG_LINES: std::ops::RangeInclusive<usize> = 2000..=10_000;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub dll: String,
    pub helper: String,
    pub lib_dir: String,
    pub antiban: injector::antiban::Config,
    pub transports: injector::transports::Settings,
    pub include_system_info: bool,
    pub log_lines: usize,
    pub script_debugging: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_default();
        Self {
            dll: detect(&exe_dir, "agent.dll"),
            helper: detect(&exe_dir, "injector.exe"),
            lib_dir: String::new(),
            antiban: Default::default(),
            transports: Default::default(),
            include_system_info: true,
            log_lines: *LOG_LINES.start(),
            script_debugging: false,
        }
    }
}

fn detect(exe_dir: &Path, name: &str) -> String {
    let beside = exe_dir.join(name);
    let found = if beside.is_file() {
        Some(beside)
    } else {
        let cross = exe_dir.join("../x86_64-pc-windows-gnu");
        let profile = exe_dir.file_name().map(PathBuf::from).unwrap_or_default();
        [profile.as_path(), Path::new("fast"), Path::new("release"), Path::new("debug")]
            .iter()
            .map(|dir| cross.join(dir).join(name))
            .filter_map(|path| Some((path.metadata().ok()?.modified().ok()?, path)))
            .max_by_key(|(modified, _)| *modified)
            .map(|(_, path)| path)
    };
    found
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

impl Settings {
    pub fn fill_missing(&mut self) {
        self.resolve_paths(Self::default());
        self.log_lines = self.log_lines.clamp(*LOG_LINES.start(), *LOG_LINES.end());
    }

    fn resolve_paths(&mut self, detected: Self) {
        if !detected.dll.is_empty() {
            self.dll = detected.dll;
        }
        if !detected.helper.is_empty() {
            self.helper = detected.helper;
        }
    }

    pub fn dll_version(&self) -> Result<String, String> {
        std::fs::read(&self.dll)
            .map(|dll| injector::dll_version(&dll))
            .map_err(|e| format!("Cannot read {}: {e}", self.dll))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Target {
    pub client: Client,
    pub location: Location,
}

impl Target {
    pub fn key(&self) -> String {
        format!(
            "{}:{}",
            self.location.label().unwrap_or_default(),
            self.client.pid
        )
    }
}

/// Starts a console program without a console window. On Windows the loader is a GUI
/// program (outside debug builds), so each console child would otherwise open its own
/// terminal. Its pipes are unaffected.
pub fn no_window(command: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    command
}

#[derive(Default)]
pub struct Scan {
    pub targets: Vec<Target>,
    pub errors: Vec<String>,
    pub dll_version: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detected_files_replace_saved_paths_which_stay_only_when_nothing_is_found() {
        let with = |dll: &str| Settings {
            dll: dll.into(),
            helper: String::new(),
            ..Default::default()
        };
        let mut saved = with("saved.dll");
        saved.resolve_paths(with("detected.dll"));
        assert_eq!(saved.dll, "detected.dll");
        let mut saved = with("saved.dll");
        saved.resolve_paths(with(""));
        assert_eq!(saved.dll, "saved.dll");
    }

    #[test]
    fn a_local_build_uses_the_newest_cross_build_in_its_target_folder() {
        let root = std::env::temp_dir().join(format!("volcren-target-{}", std::process::id()));
        let exe_dir = root.join("debug");
        let cross = root.join("x86_64-pc-windows-gnu");
        for dir in [&exe_dir, &cross.join("release"), &cross.join("fast")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(cross.join("release/agent.dll"), b"").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(cross.join("fast/agent.dll"), b"").unwrap();
        assert!(detect(&exe_dir, "agent.dll").ends_with("fast/agent.dll"));
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(cross.join("release/agent.dll"), b"").unwrap();
        assert!(detect(&exe_dir, "agent.dll").ends_with("release/agent.dll"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_installed_release_finds_its_files_next_to_the_executable() {
        let dir = std::env::temp_dir().join(format!("volcren-detect-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("agent.dll"), b"").unwrap();
        std::fs::write(dir.join("injector.exe"), b"").unwrap();
        assert!(detect(&dir, "agent.dll").ends_with("agent.dll"));
        assert!(detect(&dir, "injector.exe").ends_with("injector.exe"));
        assert_eq!(detect(&dir, "missing.dll"), "");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
