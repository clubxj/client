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

fn main() {
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
