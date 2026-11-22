# Releases, signing and the installer

## What a push to `main` does

`.github/workflows/release.yml`: `build` + `build-loader` -> `package` (Linux tar) + `msi` -> `sign` -> `publish` -> `release`. Pull requests run build, package and the MSI test only.

- Version `YYYY.MM.DD-<7-char commit>` (commit's UTC date) and build = commit Unix time, identical in every job. Manifest `build` orders releases, so an older signed manifest is never offered.
- The injector and the loader both build `sdk/luau` in ([sdk.md](sdk.md)), so a release's SDK is exactly the one in its commit.
- Rust (`RUST_VERSION`) is pinned so the build cache survives. The cache job backdates tracked files and touches only those changed since `target/.built-commit`; don't remove that or every crate rebuilds.
- The Windows loader is built on a `windows-latest` runner (`build-loader`, MSVC target, static CRT) because GPUI compiles its HLSL shaders with the Windows SDK's `fxc.exe`, and only when the build host is Windows. A release Windows loader therefore can't be built on Linux; agent and injector still cross-compile with MinGW in `build`.
- MinGW comes from cached `.deb`s keyed on the runner's OS release and `MINGW_PACKAGES`, because the Ubuntu mirror sometimes serves them at under 100 kB/s and stalls the build for half an hour. Only the `-win32` variants are listed: the `gcc-mingw-w64-x86-64` metapackages also pull the unused `-posix` ones.
- A failing `installer/test-msi.ps1` stops the release before signing.

## luau-analyze

Both packages ship `luau-analyze` (`.exe` in the MSI) next to the loader, which runs it from its own folder for the MCP `check_luau` tool ([mcp.md](mcp.md)); script authors don't have Luau installed. It is the official luau-lang/luau release build, pinned to the version in `sdk/luau/mise.toml` so the loader checks scripts the way the SDK's lint does. The Windows build needs the MSVC 2015+ runtime (`VCRUNTIME140.dll`, `MSVCP140.dll`), which the MSI doesn't install.

To bump: set `LUAU_VERSION` in `release.yml` to the new tag, then set `LUAU_UBUNTU_SHA256` and `LUAU_WINDOWS_SHA256` to the hashes of that tag's `luau-ubuntu.zip` and `luau-windows.zip` (`gh release download <tag> -R luau-lang/luau -p 'luau-*.zip'`, then `sha256sum`, and compare with the digests `gh release view <tag> -R luau-lang/luau --json assets` reports). Bump `sdk/luau/mise.toml` with it.

## Bucket layout (public `client`)

- `<version>/volcren-windows-<version>.msi`, `<version>/volcren-linux-<version>.tar.gz` (folder `volcren/` with `loader`, `injector.exe`, `luau-analyze`); cached a year.
- `<version>/injector.exe` and `<version>/injector.exe.minisig`: the release's injector on its own, signed with the update key and the version as trusted comment. Script revisions are pinned to a release, and a loader of any later release downloads this file to run them (`injectors.rs`, [loader-gui.md](loader-gui.md)), so never delete a release's folder while the database lists it unretired in `scripts.injectors`.
- `<version>/sdk-<version>.zip`, listed as `sdk` in the manifests: the SDK the API type-checks revisions against ([sdk.md](sdk.md)).
- The tarball must hold exactly the files in `FILES` (`src/loader/src/update/archive.rs`): a Linux loader rejects an update with a file it doesn't know or without one it expects, so adding or removing one strands older installs on a manual install.
- `latest.signed.json` `{manifest, signature}`: what the loader reads. One object so a signature can't pair with another manifest. Uploaded after the packages.
- `latest.json`: same manifest unsigned, for the webapp's install page; nothing installs from it. Uploaded last.
- Manifest: `version`, `build`, `api_version` (from `API_VERSION` in `src/loader/src/api/models.rs`), and per-OS `name`, `path`, `sha256`. One signature covers all packages through the hashes.
- `publish` verifies the signature against the keys in `update.rs` before uploading, and checks the published manifests match the uploaded bytes.

## Secrets and keys

- `UPDATE_SIGNING_KEY` lives only on the `release-signing` Environment (should allow only `main`). `SUPABASE_SERVICE_KEY` is only in `publish`'s upload step. `release` uses only the job token. Keeping them apart means a leaked Supabase key can't ship code.
- The loader trusts the two minisign keys in `PUBLIC_KEYS` (`src/loader/src/update.rs`); verification can't be disabled. The second ("next") private key stays offline.
- Never put private keys in code, docs or `.env*`.

Rotation (by hand, when the current key is retired or leaked):

1. Set `UPDATE_SIGNING_KEY` to the next private key.
2. `rsign generate -W -p next.pub -s next.key`.
3. `PUBLIC_KEYS` = [old next, new next]; release.
4. Re-sign `src/loader/src/update/signed_fixture.json` (its `build` 0 can never be offered).

A loader trusting neither key needs a manual install.

## MSI (`installer/volcren.wxs`)

- `perUserOrMachine`: per-user `%LOCALAPPDATA%\Programs\VOLCREN` (no UAC, default unattended), per-machine `%ProgramFiles%\VOLCREN`. Windows Installer won't upgrade across modes; the loader's update passes the existing mode and folder.
- `UpgradeCode` is fixed forever and must match `UPGRADE_CODE` in `src/loader/src/update/msi.rs`.
- MSI version is `<year-2000>.<month>.<minute of month, UTC>` (limit `255.255.65535`); `AllowSameVersionUpgrades` covers re-runs.
- The loader detects its install mode via `ARPINSTALLLOCATION`.
- Not code-signed: SmartScreen warns, UAC shows unknown publisher, Smart App Control blocks it.
- WiX pinned to 5.0.2 (MS-RL, no fee). Don't move to v6+ without deciding on its Open Source Maintenance Fee for revenue-generating use.
- CI can't test the dialog, the UAC prompt, or injecting from an installed copy; check those on Windows by hand after changing them.
