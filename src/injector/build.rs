use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const AGENT_INPUTS: [&str; 6] = [
    "../agent",
    "../ipc",
    "../decode",
    "../../Cargo.toml",
    "../../Cargo.lock",
    "../../.cargo/config.toml",
];

const SDK_MODULES: &str = "../../sdk/luau/modules";

fn main() {
    embed_sdk();
    if env::var_os("CARGO_FEATURE_AGENT").is_none()
        || env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
    {
        return;
    }
    for input in AGENT_INPUTS {
        println!("cargo::rerun-if-changed={input}");
    }

    let target = env::var("TARGET").expect("cargo sets TARGET");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let profile_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("OUT_DIR is <target dir>/[<triple>/]<profile>/build/<crate>/out");
    let profile_dir_name = profile_dir
        .file_name()
        .and_then(|name| name.to_str())
        .expect("the profile folder has a UTF-8 name");
    let profile = match profile_dir_name {
        "debug" => "dev",
        name => name,
    };
    let target_dir = profile_dir
        .parent()
        .filter(|parent| !parent.ends_with(&target))
        .or_else(|| profile_dir.parent().and_then(Path::parent))
        .expect("the profile folder sits in a target dir")
        .join("embedded-agent");

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(cargo)
        .current_dir(manifest_dir.join("../.."))
        .args([
            "build",
            "-p",
            "agent",
            "--target",
            &target,
            "--profile",
            profile,
        ])
        .arg("--target-dir")
        .arg(&target_dir)
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("CARGO_TARGET_DIR")
        .stdout(Stdio::from(io::stderr()))
        .status()
        .expect("cargo runs");
    assert!(status.success(), "building the agent failed");

    let built = target_dir
        .join(&target)
        .join(profile_dir_name)
        .join("agent.dll");
    fs::copy(&built, out_dir.join("agent.dll"))
        .unwrap_or_else(|e| panic!("Cannot copy {}: {e}", built.display()));
}

fn embed_sdk() {
    println!("cargo::rerun-if-changed={SDK_MODULES}");
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let dir = manifest_dir.join(SDK_MODULES);
    let mut modules: Vec<(String, PathBuf)> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("Cannot read {}: {e}", dir.display()))
        .map(|entry| entry.expect("the SDK folder lists").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "luau"))
        .map(|path| {
            let name = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .expect("SDK module names are UTF-8")
                .to_owned();
            let path = path.canonicalize().expect("the SDK module exists");
            (name, path)
        })
        .collect();
    modules.sort();
    let entries: String = modules
        .iter()
        .map(|(name, path)| format!("    ({name:?}, include_str!({:?})),\n", path.display().to_string()))
        .collect();
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    fs::write(
        out_dir.join("sdk_modules.rs"),
        format!("const SDK_MODULES: &[(&str, &str)] = &[\n{entries}];\n"),
    )
    .expect("OUT_DIR is writable");
}
