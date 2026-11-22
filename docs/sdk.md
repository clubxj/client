# SDK (`sdk/luau`)

The Luau SDK scripts are written against: `modules/` (the code, with `host.luau` wrapping the sandbox globals), `docs/` (one hand-written page per module) and `examples/`. Scripts require it as `@volcren/<module>`.

## Why it lives here

The SDK is built into `injector.exe`, so a script always runs against the SDK that was built with the globals it calls. Nothing pairs an SDK release with an injector, and a global changes with its wrapper in one commit. A pushed SDK change is a client release.

## Invariants

- **Flat.** Every module is `modules/<name>.luau` and every page `docs/<name>.md`, so the require name, the file and the page agree everywhere: the injector's resolver, `.luaurc` aliases in editors, the MCP's `sdk_docs`. Names are unique, lowercase, digits and `_`. Grouping lives only in `docs/README.md`, which lists every module by category; add a new module there.
- **Modules require each other only as `@volcren/<name>`.** A cycle is broken by requiring inside a function through `(require :: any)(path)`, the `lazy`/`lib` helpers in `walker.luau`, `transports.luau` and `house.luau`.
- **`.luaurc` aliases `volcren` to `./modules`.** `luau-analyze` and luau-lsp resolve requires from the nearest one, here and in the scripts repo, whose alias points at this folder.

## Where it is built in

- **Injector:** `src/injector/build.rs` lists `modules/` into `SDK_MODULES`; `--sdk <dir>` reads `<dir>/modules` instead, for iterating without a rebuild ([injector-and-sandbox.md](injector-and-sandbox.md)).
- **Loader:** `src/loader/build.rs` embeds `modules`, `docs`, `examples` and `.luaurc`; `sdk::dir` unpacks them to `<cache>/sdk/<content hash>/` on first use and deletes older copies. `sdk_docs`, `check_luau` and the workspace `.luaurc` read that copy, or the settings' SDK folder when set ([mcp.md](mcp.md)).
- **Release:** the `sdk` job lints, checks the docs and packages `sdk-<version>.zip` (`.luaurc`, `modules`, `docs`, `examples`). It is published at `<version>/sdk-<version>.zip` and listed as `sdk` in `latest.json` and the signed manifest. The API's type check and the scripts repo's CI read that zip, so revisions are checked against the SDK users run ([releases.md](releases.md)).

## Docs

`tools/docgen/gen_docs.py` adds a stub section for each new function, follows renames when given `--base <commit>`, hides sections of removed functions and prefixes pages of removed modules with `_`. Run it after changing functions and replace each `TODO...` by hand. The release fails when running it would change `docs/`.

## Checks

`tools/lint.sh` (`stylua` and `luau-analyze` over the folder, the versions in `mise.toml`). Runs that need the game client aren't covered by it.
