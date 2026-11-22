use std::{
    fs,
    path::{Path, PathBuf},
};

use loader::backend::Settings;

include!(concat!(env!("OUT_DIR"), "/sdk_files.rs"));

const COMPLETE: &str = ".complete";
const STAGING: &str = ".new";

pub fn custom(settings: &Settings) -> Result<Option<PathBuf>, String> {
    let custom = settings.sdk_dir.trim();
    if custom.is_empty() {
        return Ok(None);
    }
    let dir = PathBuf::from(custom);
    if dir.join("modules").is_dir() {
        Ok(Some(dir))
    } else {
        Err(format!(
            "The SDK folder {} set in the loader's settings has no modules folder.",
            dir.display()
        ))
    }
}

pub fn dir(settings: &Settings) -> Result<PathBuf, String> {
    match custom(settings)? {
        Some(dir) => Ok(dir),
        None => {
            let root = crate::session::cache_root()
                .ok_or("Can't find a cache folder on this system.")?
                .join("sdk");
            unpack(&root)
        }
    }
}

fn unpack(root: &Path) -> Result<PathBuf, String> {
    let dir = root.join(SDK_HASH);
    if dir.join(COMPLETE).is_file() {
        return Ok(dir);
    }
    let staging = root.join(format!("{SDK_HASH}.{}{STAGING}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    let written = SDK_FILES.iter().try_for_each(|(path, bytes)| {
        let file = staging.join(path);
        fs::create_dir_all(file.parent().unwrap_or(&staging))?;
        fs::write(file, bytes)
    });
    let unpacked = written
        .and_then(|()| fs::write(staging.join(COMPLETE), b""))
        .and_then(|()| {
            let _ = fs::remove_dir_all(&dir);
            fs::rename(&staging, &dir)
        });
    if let Err(error) = unpacked {
        let _ = fs::remove_dir_all(&staging);
        if !dir.join(COMPLETE).is_file() {
            return Err(format!("Couldn't unpack the SDK into {}: {error}", dir.display()));
        }
    }
    prune(root);
    Ok(dir)
}

fn prune(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name != SDK_HASH && !name.ends_with(STAGING) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpacks_the_built_in_sdk_once_and_drops_older_ones() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-scratch/sdk");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("old")).unwrap();

        let dir = unpack(&root).unwrap();
        assert!(dir.join("modules/host.luau").is_file());
        assert!(dir.join("docs/host.md").is_file());
        assert!(dir.join(".luaurc").is_file());
        assert!(!root.join("old").exists());

        fs::write(dir.join("modules/host.luau"), "edited").unwrap();
        assert_eq!(unpack(&root).unwrap(), dir);
        assert_eq!(
            fs::read_to_string(dir.join("modules/host.luau")).unwrap(),
            "edited",
            "a complete unpack is reused"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_custom_folder_needs_modules() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-scratch/sdk-custom");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let settings = |dir: &Path| Settings {
            sdk_dir: dir.display().to_string(),
            ..Settings::default()
        };
        assert!(custom(&settings(&root)).is_err());
        fs::create_dir_all(root.join("modules")).unwrap();
        assert_eq!(custom(&settings(&root)).unwrap(), Some(root.clone()));
        assert_eq!(custom(&Settings::default()).unwrap(), None);
        let _ = fs::remove_dir_all(root);
    }
}
