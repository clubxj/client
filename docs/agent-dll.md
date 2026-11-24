# agent (`src/agent`)

`agent.dll`, a `cdylib` injected into the game client; the only component that touches client memory directly. Windows x64 only. Offsets and client internals: [reverse-engineering.md](reverse-engineering.md).

## Lifecycle

- **Load:** the DLL ships only inside `injector.exe` (see [injector-and-sandbox.md](injector-and-sandbox.md)). `injector load` writes it to `%TEMP%\agent\agent-<version>.dll`, so a new build can replace a loaded one, then `LoadLibraryW`s it. Only `DllMain` is exported; everything else goes through shared memory, named events and window messages.
- **Start** (`hooks::start`, own thread): find the `JagRenderView` window, run every offset rule, install `retour` detours, patch `GetFocus` (below), then decode the loc and varbit definitions from the client's disk cache on a thread of its own (`definitions::start`; unload joins it). The cache folder is found once, through the client's open handle to `main_file_cache.dat2`.
- **Per frame, game thread** (`client/frame.rs`, after the client's draw step): publish the models drawn this frame (below); advance the act job (below); copy the finished menu for the worker (`menu::capture`, see [ipc-protocol.md](ipc-protocol.md)).
  - Some renderer modes (1 and 3 of the client's 0 to 3) present from a render thread of their own, so the swap hook isn't on the game thread there. The frame rule (`offsets/frame.rs`, see [reverse-engineering.md](reverse-engineering.md)) finds the game thread's tick. Its detour reads the draw step from the app object's vtable on the first tick and hooks it, so the work runs on the game thread in every mode.
  - Until the draw step is hooked (or if the rule doesn't match a new build), the swap hook does this work as before. Once hooked, the swap hook never takes it back, even while unloading.
- **Per frame, swap hook** (whichever thread presents): store the viewport offset the act job uses; publish state; capture pixels only when a reader set `capture_enabled` (only `injector screenshot`; Luau never reads pixels); draw the overlay, re-uploaded only when its serial or the viewport's size changes. The overlay is drawn 1:1 from the viewport's top-left at the size it was painted, cropped to the viewport, so a resize never rescales it out of line with the game's pixels. The overlay shader discards world-layer pixels (alpha low bit 0) inside the interface mask, passed as uniforms every frame, so world drawings hide under interfaces without a repaint; texels are fetched exactly (`texelFetch`), since filtering would blend the layer bit. Capture is skipped while the viewport exceeds `IMAGE_DATA_SIZE`. Rule results go to the header's `offsets_state`, shown by the loader.
- **Interface mask** (`client/interface_mask.rs`, rule in `decode::interface_mask`): while the overlay is on, a thread reads the whole widget tree in-process every 100 ms (about 7,400 widgets in resizable mode, too many to read from the injector at that rate) and keeps up to `MASK_RECTS` rectangles. Only what the client paints opaquely counts: visible sprites and filled rectangles with opacity below 255, from every open interface including the root. Mount widgets can't be used: resizable mode mounts large transparent containers (groups 122 and 163, over half the viewport), while the side panel's and minimap's backgrounds belong to the root (161), which isn't a sub-interface. Rects covering 90% of the viewport are dropped, so do rects inside another; overlapping ones aren't merged into a bounding box, which would mask world between panels, and past the limit the pair whose union adds the least area merges.
- **Commands:** custom `ipc::input::*_MESSAGE` window messages handled by the WndProc hook. Worker requests (polled decoders) are signalled on `AGENT_WORK_<pid>` and answered by `client/worker.rs`.
- **Unload:** named event `AGENT_UNLOAD_<pid>` (works with the WndProc hook off). Start creates it before anything else and waits on it once start ends, failed or not, so an unload sent while starting runs right after and a failed start can still unload: unhook, drain in-flight hook calls, restore `GetFocus`, close mappings, `FreeLibraryAndExitThread` (`client/unload.rs`).

## Input

- Real mouse input is dropped for `REAL_MOUSE_BLOCK_MS` after synthetic input (`SYNTHETIC` flag).
- The client drops keys unless `GetFocus` returns its window, so `client/focus.rs` patches the main module's `USER32!GetFocus` import slot to return the render view. Only the client's own calls are affected. The WndProc hook also swallows `WM_KILLFOCUS`.

## Drawn models (`client/drawn.rs`)

The client already poses and places every model it draws, so the agent keeps what it drew instead of building models itself. Building one calls client code, which only the game thread may do.

- Two detours, both found by `find_render` (see [reverse-engineering.md](reverse-engineering.md)): the per-renderable draw stashes the renderable, orientation and camera-relative placement; the pass classifier, which the draw calls only for models that survived culling, records that placement with the posed model.
- Posed models (the transient flag, `Render::transient`) are copied when recorded, because the client poses every animated model into one reused scratch model, so a later draw in the same frame overwrites it. Cached models are never changed, so the agent keeps them instead: one more shared_ptr use (the control block is `model - 0x20`), so they outlive the client's own release and the worker reads their vertices off the game thread.
- Every use the agent adds is released on the game thread: after the draw step the building frame is published, the previous one is retired, and a retired frame is released once no worker read holds it.
- The animation lift (renderable vtable slot `model::animation_slot`) is subtracted from the height, as the draw does before it submits.
- The camera for the frame is read when it's published (`decode::camera::Camera::current`), so a projection never mixes two frames.
- Identical static locs (same id and rotation) share one cached model, so one renderable is drawn once per copy. Each draw stores the draw's tag, which encodes the loc's tile, and a loc lookup matches the renderable and the scene object's tag together; otherwise every copy would get the first copy's model.
- Unload first asks the next frame to release everything (`drawn::stop`); if no frame comes within a second, the uses are leaked rather than released off the game thread.

## Act (`client/act.rs`)

Clicking a moving thing from outside the client is always a frame or more late, so the agent makes the click decision itself on the frame the client drew.

- `ACT_MESSAGE` arms or cancels one job from `act_request`; a new arm replaces the old job.
- Each frame, after the draw step on the game thread: project the job's model from the frame just published (`model::faces`), keep a spot on one triangle (barycentric, re-picked only when that triangle shrinks or leaves the visible area), read the menu the frame just built (`menu::snapshot`), then write `act_state` (spot, mouse, status).
- It presses only when an entry matches the target (kind, id, index and the entry's tile, each optional) and the mouse is within `near_px` of the spot plus the spot's speed times `FOLLOW_LAG_MS`: left when the left-click entry is a wanted option, else right if allowed. The press is posted as a move to the mouse position this frame's menu was built for, then the button down, so moves still queued from the injector can't shift it.
- The agent only presses; the injector moves the mouse (WindMouse after `act_state.spot`) and releases the button.
- `GONE` means no model for `GONE_FRAMES`; a model that exists but is covered keeps tracking until the timeout.

## Event logs

`packets.rs`, `script_runs.rs`, `varc_log.rs`, `item_log.rs` detour client functions (rules in `offsets/`) and append to shared rings `AGENT_PACKETS_`/`SCRIPTS_`/`VARCS_`/`ITEMS_<pid>`; `log.rs` holds the shared parts. Format and reader wake-ups: [ipc-protocol.md](ipc-protocol.md).

- Hooks run on the game thread (packet send: network thread), copy only what they need and return fast.
- `packets.rs` checks the local player's server tile before each packet receive and publishes changes, so no tick's move is missed however slow a script is.
- Script runs log the tick read where the game thread writes it; scripts the client starts without an event aren't logged.
- Varc and item writes that leave the value unchanged aren't logged.

## Layout

- `shared/`: DLL entry, shared-memory manager, unload/worker events, window lookup.
- `client/hooks.rs`: hooks, per-frame work, messages. `client/offsets/`: one rule module per feature. `client/own.rs`: in-process `Memory`, global `Layout`, game-thread tracking. `client/graphics.rs`: capture and overlay. Other `client/*.rs`: readers over rule results.

## Gotchas

- A panic in a hook kills the client: wrap hook bodies in `catch_unwind`.
- Save and restore OpenGL state around overlay drawing (`graphics::restore_state`).
- Anything in shared memory is protocol: change it in `src/ipc` and bump `PROTOCOL_VERSION`.
