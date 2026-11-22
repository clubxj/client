use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{api::Api, session, update};

const FILE: &str = "injector.exe";
const SIGNATURE: &str = "injector.exe.minisig";
const MAX_INJECTOR_BYTES: u64 = 256 * 1024 * 1024;
const MAX_SIGNATURE_BYTES: u64 = 4 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pinned {
    pub version: String,
    pub path: PathBuf,
}

impl Pinned {
    pub fn is_bundled(&self) -> bool {
        self.version == update::VERSION
    }
}

fn valid_version(version: &str) -> bool {
    (1..=64).contains(&version.len())
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

pub fn pinned(api: &Api, bundled: &Path, version: &str) -> Result<Pinned, String> {
    if !valid_version(version) {
        return Err(format!("The server named an injector that can't be used: {version:?}."));
    }
    if version == update::VERSION {
        return Ok(Pinned {
            version: version.to_owned(),
            path: bundled.to_path_buf(),
        });
    }
    let root = session::cache_root().ok_or("Can't find a cache folder on this system.")?;
    let path = fetch(api, &root.join("injectors"), version, update::public_keys())?;
    Ok(Pinned {
        version: version.to_owned(),
        path,
    })
}

fn fetch(api: &Api, root: &Path, version: &str, keys: &[&str]) -> Result<PathBuf, String> {
    let dir = root.join(version);
    let path = dir.join(FILE);
    if path.is_file() {
        return Ok(path);
    }
    let what = format!("injector {version}");
    let url = |file: &str| update::bucket_url(api, &format!("{version}/{file}"));
    let bytes = session::get_bytes(api.agent(), &url(FILE), &what, MAX_INJECTOR_BYTES)?;
    let signature =
        session::get_bytes(api.agent(), &url(SIGNATURE), &what, MAX_SIGNATURE_BYTES)?;
    verify(&bytes, &signature, version, keys)?;
    fs::create_dir_all(&dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    let staging = dir.join(format!("{FILE}.{}.new", std::process::id()));
    fs::write(&staging, &bytes)
        .and_then(|()| fs::rename(&staging, &path))
        .map_err(|e| {
            let _ = fs::remove_file(&staging);
            format!("Couldn't save {what}: {e}")
        })?;
    Ok(path)
}

fn verify(bytes: &[u8], signature: &[u8], version: &str, keys: &[&str]) -> Result<(), String> {
    let refused = || format!("Injector {version} isn't signed for this version, so it wasn't used.");
    let signature = std::str::from_utf8(signature).map_err(|_| refused())?;
    let signature = minisign_verify::Signature::decode(signature).map_err(|_| refused())?;
    let signed = keys.iter().any(|key| {
        minisign_verify::PublicKey::from_base64(key)
            .and_then(|key| key.verify(bytes, &signature, false))
            .is_ok()
    });
    if signed && signature.trusted_comment() == version {
        Ok(())
    } else {
        Err(refused())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::fake_api::{FakeServer, Reply};

    fn sign(pair: &minisign::KeyPair, bytes: &[u8], comment: &str) -> Vec<u8> {
        minisign::sign(None, &pair.sk, Cursor::new(bytes), Some(comment), None)
            .unwrap()
            .into_string()
            .into_bytes()
    }

    #[test]
    fn only_a_signature_for_this_version_is_accepted() {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let other = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let key = pair.pk.to_base64();
        let keys = [key.as_str()];
        let exe = b"MZ injector";
        assert!(verify(exe, &sign(&pair, exe, "2026.11.22-abc1234"), "2026.11.22-abc1234", &keys).is_ok());
        assert!(
            verify(exe, &sign(&pair, exe, "2026.11.21-old0000"), "2026.11.22-abc1234", &keys).is_err(),
            "another release's signature can't be swapped in"
        );
        assert!(verify(b"MZ changed", &sign(&pair, exe, "2026.11.22-abc1234"), "2026.11.22-abc1234", &keys).is_err());
        assert!(verify(exe, &sign(&other, exe, "2026.11.22-abc1234"), "2026.11.22-abc1234", &keys).is_err());
        assert!(verify(exe, b"not a signature", "2026.11.22-abc1234", &keys).is_err());
    }

    #[test]
    fn a_pinned_injector_is_downloaded_once_and_checked() {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let key = pair.pk.to_base64();
        let exe = b"MZ injector".to_vec();
        let signature = sign(&pair, &exe, "2026.11.20-abc1234");
        let fake = FakeServer::start(move |_, request| match request.path.as_str() {
            "/storage/v1/object/public/client/2026.11.20-abc1234/injector.exe" => {
                Reply::bytes(exe.clone())
            }
            "/storage/v1/object/public/client/2026.11.20-abc1234/injector.exe.minisig" => {
                Reply::bytes(signature.clone())
            }
            _ => Reply::status(404),
        });
        let api = crate::update::tests::api(&fake.url);
        let root = crate::update::tests::temp_dir("injectors");

        let path = fetch(&api, &root, "2026.11.20-abc1234", &[key.as_str()]).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"MZ injector");
        fetch(&api, &root, "2026.11.20-abc1234", &[key.as_str()]).unwrap();
        assert_eq!(fake.count("GET", "/storage/"), 2, "the second call reads the cache");

        assert!(fetch(&api, &root, "2026.11.19-missing", &[key.as_str()]).is_err());
        assert!(!root.join("2026.11.19-missing").join(FILE).exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn versions_are_plain() {
        assert!(valid_version("2026.11.22-abc1234"));
        assert!(!valid_version("../evil"));
        assert!(!valid_version(""));
        assert!(!valid_version("2026/11"));
    }
}
