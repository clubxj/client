use std::collections::HashMap;
use std::path::PathBuf;
use std::thread::JoinHandle;

use serde::Serialize;
use serde::de::DeserializeOwned;

pub const SETTINGS_KEY: &str = "app";

pub fn dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", crate::APP_ID).map(|dirs| dirs.data_dir().to_path_buf())
}

pub struct Storage {
    path: Option<PathBuf>,
    values: HashMap<String, String>,
    dirty: bool,
    writing: Option<JoinHandle<()>>,
}

impl Storage {
    pub fn open() -> Self {
        Self::at(dir().map(|dir| dir.join("app.ron")))
    }

    pub fn at(path: Option<PathBuf>) -> Self {
        let values = path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| ron::from_str(&text).ok())
            .unwrap_or_default();
        Self {
            path,
            values,
            dirty: false,
            writing: None,
        }
    }

    pub fn get_string(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn set_string(&mut self, key: &str, text: String) {
        if self.values.get(key) != Some(&text) {
            self.values.insert(key.to_owned(), text);
            self.dirty = true;
        }
    }

    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.get_string(key)
            .and_then(|text| ron::from_str(text).ok())
    }

    pub fn set<T: Serialize>(&mut self, key: &str, value: &T) {
        if let Ok(text) = ron::ser::to_string(value) {
            self.set_string(key, text);
        }
    }

    pub fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(path) = self.path.clone() else {
            return;
        };
        self.dirty = false;
        let values = self.values.clone();
        self.finish();
        self.writing = std::thread::Builder::new()
            .name("storage".into())
            .spawn(move || write(&path, &values))
            .ok();
    }

    pub fn finish(&mut self) {
        if let Some(writing) = self.writing.take() {
            writing.join().ok();
        }
    }
}

impl Drop for Storage {
    fn drop(&mut self) {
        self.finish();
    }
}

fn write(path: &std::path::Path, values: &HashMap<String, String>) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let Ok(text) = ron::ser::to_string_pretty(values, ron::ser::PrettyConfig::default()) else {
        return;
    };
    let partial = path.with_extension("ron.partial");
    if std::fs::write(&partial, text).is_ok() {
        std::fs::rename(&partial, path).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip_and_unknown_keys_survive_a_save() {
        let dir = std::env::temp_dir().join(format!("loader-storage-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app.ron");
        let mut map = HashMap::new();
        map.insert("egui".to_owned(), "(kept: true)".to_owned());
        map.insert("cursor".to_owned(), "\"Arrow\"".to_owned());
        std::fs::write(&path, ron::to_string(&map).unwrap()).unwrap();

        let mut storage = Storage::at(Some(path.clone()));
        assert_eq!(storage.get::<String>("cursor").as_deref(), Some("Arrow"));
        storage.set("forms", &vec![1, 2, 3]);
        storage.flush();
        storage.finish();

        let reopened = Storage::at(Some(path));
        assert_eq!(reopened.get::<Vec<i32>>("forms"), Some(vec![1, 2, 3]));
        assert_eq!(reopened.get_string("egui"), Some("(kept: true)"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_folder_is_the_one_eframe_used() {
        let dir = dir().unwrap();
        assert!(dir.ends_with(if cfg!(windows) {
            "Loader/data"
        } else {
            "loader"
        }));
    }
}
