use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn version() -> String {
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .env("TZ", "UTC")
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    std::env::var("VOLCREN_VERSION")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            let date = git(&[
                "show",
                "-s",
                "--format=%cd",
                "--date=format-local:%Y.%m.%d",
                "HEAD",
            ]);
            let commit = git(&["rev-parse", "--short=7", "HEAD"]);
            date.zip(commit).map_or_else(
                || "dev".to_owned(),
                |(date, commit)| format!("{date}-{commit}"),
            )
        })
}

fn build() -> String {
    std::env::var("VOLCREN_BUILD")
        .ok()
        .filter(|build| !build.is_empty())
        .or_else(|| {
            let output = Command::new("git")
                .args(["show", "-s", "--format=%ct", "HEAD"])
                .output()
                .ok()?;
            output
                .status
                .success()
                .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        })
        .unwrap_or_else(|| "0".to_owned())
}

fn main() {
    println!("cargo:rerun-if-env-changed=VOLCREN_VERSION");
    println!("cargo:rerun-if-env-changed=VOLCREN_BUILD");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rustc-env=VOLCREN_VERSION={}", version());
    println!("cargo:rustc-env=VOLCREN_BUILD={}", build());
    embed_sdk();
}

const SDK: &str = "../../sdk/luau";
const SDK_PARTS: [&str; 4] = ["modules", "docs", "examples", ".luaurc"];

fn sdk_files(root: &Path, path: &Path, found: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let mut entries: Vec<PathBuf> = fs::read_dir(path)
            .unwrap_or_else(|e| panic!("Cannot read {}: {e}", path.display()))
            .map(|entry| entry.expect("the SDK folder lists").path())
            .collect();
        entries.sort();
        for entry in entries {
            sdk_files(root, &entry, found);
        }
    } else if path.is_file() {
        found.push(path.strip_prefix(root).expect("inside the SDK").to_path_buf());
    }
}

fn embed_sdk() {
    let root = Path::new(&std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"))
        .join(SDK)
        .canonicalize()
        .expect("the SDK folder exists");
    let mut files = Vec::new();
    for part in SDK_PARTS {
        println!("cargo:rerun-if-changed={SDK}/{part}");
        sdk_files(&root, &root.join(part), &mut files);
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut entries = String::new();
    for file in &files {
        let name = file.to_string_lossy().replace('\\', "/");
        let bytes = fs::read(root.join(file)).expect("the SDK file reads");
        for byte in name.bytes().chain([0]).chain(bytes.iter().copied()) {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
        entries.push_str(&format!(
            "    ({name:?}, include_bytes!({:?})),\n",
            root.join(file).display().to_string()
        ));
    }
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    fs::write(
        out.join("sdk_files.rs"),
        format!(
            "const SDK_HASH: &str = \"{hash:016x}\";\nconst SDK_FILES: &[(&str, &[u8])] = &[\n{entries}];\n"
        ),
    )
    .expect("OUT_DIR is writable");
}
