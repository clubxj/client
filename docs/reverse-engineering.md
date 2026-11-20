# Reverse engineering the native client

Facts about the client's internals that the code does not state, and the steps for finding offsets again after a game update. Read this before writing or fixing an offset rule. Addresses are image VAs or RVAs from the build named next to them, and they move with every update. Use them only as hints when probing.

## Offset rules (`src/agent/src/client/offsets*`)

- A rule `find_<name>(pe: &Pe, ...)` finds a field offset or a function from structure that survives updates, such as a constant, the order of compares or the shape of a call. `find_offsets` (`client/hooks.rs`) parses the image once and hands every rule the same `Pe`. Tests use `Pe::parse(&image)`.
- Procedure:
  1. Find the logic in `standalone/` and pick something that native code must also have.
  2. Scan the code, map each hit to its function through `.pdata`, decode it with `iced-x86` and track which `[base+disp]` loaded each register. Confirm the hit by matching the surrounding flow.
  3. Require exactly one match. Anything else is a `ScanError`, so a changed shape fails the rule instead of hooking the wrong code.
  4. Store the result in an `ipc::state` layout type.
- Every build in `archive/` is regression data. Each rule must still find exactly one match on all of them.
- Leaf functions such as `mov rax,[rcx+X]; ret` have no `.pdata` entry and are missing from `Pe::functions`. Find them by their bytes, or by scanning `direct_calls` targets up to the first `ret`.
- Some functions are split across chained `.pdata` chunks. `chunk_root` walks back to the entry.
- Hash tables are custom chained tables, not `std::unordered_map`.

## Reference material and tools

- **`standalone/`** is the decompiled Java client with the names from its API applied. Its version is in `standalone/VERSION`, and only about 93 of 730 classes are named. It is read only, private and git-ignored. `MAPPING.md` and `mapping.tsv` hold the renames and the field multipliers.
  - Obfuscated Java int fields store `real * k`. The native client stores plain values.
  - Ignore the junk extra parameter, the `try/catch` wrappers and the injected `getX`/`setX`/`rl$` methods. The code does not compile.
- **`archive/`** holds one exe per update, named `client-<file date>-<first 8 hex of sha256>.exe` (git-ignored). After each update:
  1. Copy the installed exe (under Bolt: `~/.local/share/bolt-launcher/`) into `archive/`.
  2. Hash the exe itself, because the `.sha256` file beside it doesn't match.
  3. Add a row to `archive/README.md`.
- **`tools/memscan`** is a Linux-only crate outside the workspace (`cargo build --release` inside it). It reads `/proc/<pid>/mem` of a client running under Wine. It scans for exact 8-byte values, `[lo,hi)` pointer ranges, aligned `i32`s, `pair`s (x/y within 16 bytes) and `fpair`s. See `src/main.rs`.
  - Workflow: scan for a value you can see in game, change it in game, rescan to narrow the hits, then turn the hit into a rule.
  - Helpers: `dump.py` dumps `i32` fields for diffing, and `watch.py` prints `i32` changes with timestamps. `snapmenu.py` and `probe_region.py` hardcode addresses from one build.
  - Under Wine, a diff that skips stack regions (`0x7ffffe...`) misses objects on the main thread's stack, such as the menu.
- **gdb probing**:
  - Run `gdb -batch -x script -p <linux pid>` under `timeout -s TERM`. On SIGTERM gdb detaches from an attached process. `-batch` can ignore SIGINT while the inferior runs, so end a probe from inside its breakpoint commands: count hits and run `delete`, `detach`, `quit` at the limit.
  - Use `hbreak`, which leaves the code unpatched. Set `handle SIGUSR1 nostop noprint pass` because Wine uses that signal, and put `commands; silent; printf ...; continue; end` on each breakpoint.
  - Finish with `delete; detach; quit`. Never kill gdb while breakpoints are set.
  - Never use `hbreak ... if` on a hot function: gdb stops on every hit to check the condition, and the client froze. Test the condition inside `commands` instead.
- **Injector dumps**: `injector player-words <pid> <index> <count>`, `injector widget-dump <pid> <count>`, `injector cache-file <pid> <index> <group> <file> <out>`.

## Keyboard focus (`client/focus.rs`)

- The client drops posted `WM_KEYDOWN`s while `GetFocus` isn't its own window. Mouse input still works. The agent's `GetFocus` import hook answers with the render view, which fixes this. Swallowing `WM_KILLFOCUS` or posting `WM_SETFOCUS` does not help. Where the client makes the check wasn't traced.

## Login provider (`offsets/login_provider.rs`)

- The welcome screen (login screen 0) asks a sign-in provider which button to draw. With no provider, or one whose checks are all false, it draws New User and Existing User. Otherwise it draws one wide button: Game Center (vtable `+0x70`), Google (`+0x78`) or Steam (`+0x80`), at `y + 58`, 20 px higher than the usual buttons. Game Center and Google also get a "Sign in with Email / Username" link; Steam doesn't.
- Chain: client vtable slot 8 lazily makes a `shared_ptr` at `client+0x68` and returns its address, so the holder object is `[client+0x70]`. Holder slot `+0x78` is the leaf getter `mov rax,[rcx+0x18]`, the provider. The holder's factory (slot 7) builds the Steam provider only when its Steam object (`holder+0x50`) has its initialized byte set: that byte is the result of `SteamAPI_Init`, which the client calls through a delay import after `SteamAPI_RestartAppIfNecessary`. Otherwise it builds a default provider whose checks are all false.
- The kind checks are per-class constants (shared `return true` and `return false` stubs), so the kind is the provider's class. The agent compares its vtable with the Steam provider's.
- Launching the client with Steam's environment variables (`SteamEnv`, `SteamClientLaunch`, `SteamAppId`, all 35 of them) doesn't change the screen; only a working Steam API does. The client never reads those variables itself.
- Anchors: the label is a global short string filled from the literal "Sign in with Steam" by a static initializer. Older builds (2026-10) copy it inline, reading its length byte first, so the rule anchors on the first reference into the string and takes the last provider method call before it as the Steam check.

## Menu (`offsets/menu.rs`)

- The menu object lives on the main thread's stack. The selected spell (Java's `widgetSelected` group) is a sub-object of it, found by `find_selection` through the native deselect and select functions.
- Rows are 22.5 px high with a 16 px header. The Java client's 19/15 is wrong for the native client.
- **Order**: the minimenu registers its sort rules in one function (it takes the rank function's address, `find_menu_order`; build 704f1c08: `0x140373210`) into a list at `minimenu+0x1b0`. The rule registered last decides first. From most to least significant: the rank score (higher first), entries about your own player, `action` (`ids+8`) ascending, the 64-bit key at `ids-8` descending, `index` (`ids+4`) ascending, the left-click swap hook (`minimenu+0x290`), `priority` ascending. The client only stores the sorted rows while the menu is open (built by the open routine into `minimenu+0x140`), so a closed menu is sorted by `decode::menu` with the same chain.
- **Rank** (`ranker`; its callees give `find_menu_settings`): cancel -4, Examine priority -3, an NPC whose type has the follower flag (`+0x15b`) while the follower setting (`minimenu+0x27a`) is on -2, a widget with `category` <= 1 +1, then `category` (`kind+1`): 0 -> 0, 1 -> -1, 2 -> -2, 3 -> -2 when the player attack option (`minimenu+0x278`) puts this player on right click, 4 -> -2 when the NPC attack option (`minimenu+0x279`) does, else 0.
- **Attack options**: 0 compares combat levels (the NPC type's `+0x11c` or the player's `+0x780` against yours: higher is right click), 1 is always right click, 2 left click, 3 hidden, 4 right click when the entry's flag byte (`kind+3`) is set. For players, when the entry's team byte (`kind+2`) and your team (player `+0x790`) are both set, a team mate is right click and anyone else left click. Players with an index above `0x7ff` are never deprioritised. NPC entries hold the NPC type id in `id`.
- The client doesn't store the menu's geometry. It lays the menu out every frame and keeps the result on the stack. The agent calls the layout routine itself (`find_menu_layout`) after the draw step, on the game thread, while the menu is open: `fn(view, out: *mut [i32; 26], path: &std::vector<u64>)`. An empty path means the menu, and row indices mean a submenu.
  - You can recognise the routine because it refuses paths of 5 or more submenus and calls itself for the parent.
  - The view is the minimenu's list view (`minimenu+0x158`, also vtable slot 3).
  - Inputs:
    - The anchor written on the right click: `view+0x10`, holding x, y, area w and h.
    - The style block at `menu+0xac`: caps, scrollbar width, paddings and the header flag.
  - Row width is the font width of `option + " " + target`. The space stays when the target is empty.
  - The open rows are `minimenu+0x140`, which leaves out hidden entries.
- **Stay open**: the menu stays open while the mouse, rounded to interface units, is inside any open level's rect grown by the input handler's margin (`+0x18`, set to 10 in the constructor). The margin is in the caller of the layout routine that grows the rect by a field and its double.
- **Submenus** (Java's `Menu.au[]`) are rows too.
  - A row is 24 bytes: the source entry list (0x150-byte entries), the index in that list, and a pointer to the submenu's rows vector, or null.
- **Entry tile**: a loc entry holds the loc's world origin tile at `ids+0x14` (x) and `ids+0x18` (y), so same-id locs on different tiles are told apart there. `index` (`ids+4`) is -1 for locs.
  - The open path is the minimenu's `std::vector<u64>` of parent rows, then its open row (-1 for none).
  - The stay-open check lays out every prefix of that path. The agent does the same.

## Chat (`offsets/scripts.rs`, `offsets/chat.rs`)

- `Messages` is found through the CS2 interpreter. The range dispatcher sends opcodes 5000-5099 to one switch.
  - Case 4 (`chat_gethistory_byuid`) holds the uid lookup in a global hash table, followed by type, cycle, name, sender and the text at `+0x88`. That text has `[HH:MM] ` prepended when the timestamp setting is on.
  - Case 30 exists only in the native client. It pushes the text as received (`+0xa0`) and calls the stamp chooser: mode 1 is UTC (`+0x28`) and mode 2 is local time (`+0x10`).
- No epoch is stored. The injector converts a message's cycle to a time with the client cycle (`clientclock`, opcode 3300, read from a global), which ticks 50 times a second.
- Messages are `shared_ptr`s. Dialogue lines are logged as type 114 `"Speaker|line"`.

## Scene loader and instances (`ipc::read::map_regions`, `offsets/scene_loader.rs`)

- A `shared_ptr` member of the client object holds the scene loader L. The packet dispatcher calls one of two methods on L, one for normal scenes and one for instances. Each method stores its vtable into a fresh builder (`0xf8` or `0xb0` bytes), stores the builder at `L+0x28` and sets L's instance flag byte to 0 or 1 (it was `+0x54dd`; `find_instance_flag` finds it). These two methods and the dispatcher are the anchors if the rule breaks.
- L layout:
  - `+0x10` is the world view.
  - `+0x38` is the inline template table (`instanceTemplateChunks`): 4x13x13 entries of 16 bytes, each `(chunkX, chunkY)` plus the plane as an `i64`. The index is `(plane*13+col)*13+row`.
  - Entries the packet doesn't fill are not cleared, so the table can hold stale data. Use the builder's vector for the list of source squares.
- Builder layout:
  - `+0x58` holds a copy of baseX and baseY.
  - `+0x68` is a `std::vector` of 0x88-byte records: `i32 x, y` (map square), `x<<8|y` at `+0x10`, then the per-plane sub-records.
  - Outdoors the records are the squares around the player. In an instance they are only the source squares (Tempoross: one record, 47,44).
- Other objects carry stale copies of baseX and baseY that look live but never change. Match by structure, not by value. `map_regions` requires exactly one builder vector of the right shape.
- Instances sit at x >= 6400, so `in_instance` is a coordinate test.

## Projectiles (`offsets/projectiles.rs`)

- The client object holds a circular list. Its head is a sentinel node (next, prev, size at `+0x10`), and each node holds a `shared_ptr` with the projectile at `+0x18`.
- Anchor: the update function (Java `Projectile.am`) is the only code that reads the double `0.02454369`. Its first chunk is chained, so the rule uses `chunk_root`. The function gives every field except the id:
  - start cycle: the first compare
  - target position, then source position: two calls to one helper (`edx` actor, `r8d` x, `r9d` y, level on the stack), each followed by a `sub` of the end or start height
  - the `moving` byte, slope and start position
  - end cycle: `- cycle + 1`
  - x, z and y doubles, in the order they are written back
  - orientation: the first `and 0x7ff` store
- The only caller is the loop, which compares the end cycle with the client cycle global. A client method `mov rdx,rcx; add rcx,list; jmp loop` gives the list offset.
- The constructor ends by reading the id back to look up the spot animation.
- Positions are world tiles. Actor references are NPC index + 1 or -(player index + 1), and 0 means none. The server often sends no source actor, including for your own bolts. `z` is measured from the heightmap's zero, so the height above the ground is the bilinear height minus `z`.

## Spot animations on tiles (`offsets/spot_anims.rs`)

- Java's `GraphicsObject`. An actor's own spot animation is the actor's `graphic` field, not this list.
- The list lives in the world view (`+0x160`) and has the same node shape as the projectile list.
- Anchor: the constructor is a function of at most 0x200 bytes that calls `lookup; mov rdx,[rax+8]; mov edx,[rdx+8]; call init` and stores its parameters in `this`.
  - The id is the register passed to the lookup, and the level is the value stored from `r9d`.
  - The three stack parameters are x, y and z.
  - The start cycle is `add edx,[rsp+n]` (delay plus client cycle).
- The constructor is reached through a `make_shared` thunk from the spot-animation packet handler, either directly or through an allocating wrapper. Each site then appends to the list (`lea rcx,[view+list]` or `add rcx,list`), which gives the list offset.
- Fields on 2026-10-28:

  | Field       | Offset                       |
  | ----------- | ---------------------------- |
  | start cycle | `+0x260`                     |
  | level       | `+0x264`                     |
  | x, y, z     | `+0x268`, `+0x26c`, `+0x270` |
  | id          | `+0x274`                     |
  | view        | `+0x278`                     |
  | animation   | `+0x280`                     |

  z is the ground height minus the height sent, so negative is up.

- Live trigger: Telekinetic Grab on a ground item places id 144 on the item's tile. Teleports and similar spells set the actor's spot animation instead.
- The agent reads the main view, plus the player's view when the player is on a boat. The overlay draws only the main view.

## Actor path and server tile (`offsets/actor_path.rs`)

- `pathX`, `pathY` and `pathLength` hold scene tiles, newest first. Entry 0 is the tile the server last set. The fine position lags it by a tick or two. The world tile is the view base (`view+0x24`, `+0x28`) plus entry 0.
- Anchor: the path reset `(rcx actor, edx x, r8d y)` (Java `Player.ax`). It is a leaf with no `.pdata` entry, and it computes `local = (size + 2*tile) << 6`.
- Fields (2026-10-11 and 2026-10-12): size `+0x278`, length `+0x3ec`, `pathX` `int[10]` at `+0x3f0`, `pathY` at `+0x418`.
- The receive hook compares the server tile at its start (see Packets).

## Movement poses (`decode::world::PLAYER_MOVEMENT`)

- Eight `i32`s at `+0x280`, right after the size: stand, turn left, turn right, walk, walk back, walk left, walk right and run.
- The poses come from the appearance packet, so they follow the weapon:
  - unarmed: 808, 823, 823, 819, 820, 821, 822, 824
  - staff: 813, 1209, 1209, 1205, 1206, 1207, 1208, 1210
- The pose at `+0x4b8` is one of these unless something overrides it, such as a tightrope or a ledge.
- To find them again: `injector player-words <pid> <index> 400` while wielding a known weapon, then look for the run of eight values.

## Packets (`offsets/packets.rs`, `client/packets.rs`, `offsets/packet_layouts.rs`, `offsets/packet_names.rs`)

- **Network thread, receive**:
  1. Reads one byte through a buffered recv wrapper that every socket reader shares.
  2. Subtracts the next ISAAC byte. If the result is `>= 0x80`, a second opcode byte follows.
  3. Looks up the size in an `i32` table of 151 entries. -1 means a length byte follows, and -2 means a length short follows.
  4. Reads the payload and pushes a kind-3 node (opcode at `+0x10`, buffer at `+0x18`) on a lock-free list at `connection+0x20`.
  - ISAAC masks only the opcode. The payload is plain.
- **Game thread, receive**:
  - The dispatcher, which has a 151-case jump table, calls `receive(connection, out) -> bool`. It writes the opcode at `out+0` and moves the buffer into `out+8`.
  - That function is the receive hook: one call per packet, plus at least one call per frame when the queue is empty.
  - The dispatcher calls `receive` only after the previous handler returned, so the start of the hook sees the effects of every packet.
  - Game updates reorder the opcodes and the reads inside a packet (the large varp packet went from `u16leadd,i32:2.3.0.1` at 40 to `i32:1.0.3.2,u16be` at 66), so nothing may key on an opcode number or a field position: use the names below.
- **Buffer object**: `+8` allocated size, `+0x10` data, `+0x18` cursor. No code shape names `BUFFER_SIZE`. The data and cursor offsets come from the opcode writer.
- **Send**:
  - The packet constructor (`js.ac`, 97 call sites) takes `(packet, type, cipher)`. It stores the plain type at `packet+0` and writes the opcode through the opcode writer (`byte = opcode + next cipher byte`).
  - The network thread pops the queue at `connection+0x30` and calls `send(connection, result, packet)`.
  - The hook reads the type, so it never needs the cipher. The first data byte is the masked opcode, and a variable-length packet has its length right after.
- **`find_packets` chain**:
  1. The opcode writer: at most 0x80 bytes, a byte `add`, `mov r32,0x100`, an indexed byte store and one call.
  2. Its only caller, the constructor.
  3. The send function: at most 0x60 bytes, one call, loads at the data and cursor offsets from `r8`.
  - The receive function is found independently.
- **Layouts** (`find_packet_layouts`) are published to the loader live, so this needs no step after an update.
  - Each case's reads go through small buffer read functions. A tiny interpreter classifies each one on probe bytes: byte order, the `x`/`x-128`/`-x`/`128-x` transforms and signedness from the caller's `movsx`.
  - Big-endian reads first test a global against `0x03020100`. The interpreter takes that test as equal.
  - A case's layout is its reads up to the first branch. It is kept only when the widths sum to the size in the table.
  - The size table is found as the one `lea r,[rip+table]` right after a `jae` whose 151 values are all in -2..512, with 10 or more distinct values.
  - Strings, smarts and loops have branches, so they don't classify. 86 of 151 packets get a layout, and none differs from what the Java client reads.
- **Names** (`find_packet_names`) come from what a case's handler does, matched against offsets other rules find. Each name must match exactly one case or it is left out and the agent logs it.
  - `TICK`: the empty case that starts with `inc dword [client + tick count]`.
  - `VARP_SMALL`/`VARP_LARGE`: the 3- or 6-byte case with a `u16` and a 1- or 4-byte field that calls a function loading the varps data pointer (`find_varps`). The `u16` is the id.
  - `SYNTH_SOUND`: the 5-byte case that calls the sound queue the `playSound` Lua binding jumps to. The queue takes `(edx id, r8 loops, r9 delay)` (the binding defaults loops to 1 and delay to 0), so a register trace from each read to those arguments gives the roles.
  - `MESSAGE_GAME`: the variable-length case whose first read is the smart reader (`cmp r8b,0x80`, `+0x8000`), then a byte compared with 1, that calls a function reaching the chat history table (`find_chat`) within 3 calls with the smart value in `edx`. The payload is `smart type, u8 has name, [str name], str text`; strings are CP1252 and zero-terminated.
  - `names_packets_in_archived_builds` pins the opcodes and role order per archived build: the October builds match the Java table, and the November ones were checked live on 2026-11-05.
- **The fallback table**: `tools/packetdefs/gen.py` produces `src/loader/src/packet_defs.tsv` from `standalone/` (`ServerPacket` `jc` sizes plus the reads of the `Client.iy` handlers).
  - `--exe` checks the native size table against `jc`.
  - `names.tsv` names packets and their fields in wire order. Check the order against the Java handler.
  - After an update, rebuild `standalone/` first. This step is optional because layouts don't depend on it.
  - Sent packets aren't covered.
  - Tests: `cd tools/packetdefs && python3 -m unittest test_gen`.
- A packet with no payload has a null data pointer.

## CS2 interpreter (`offsets/script_runs.rs`, `client/script_runs.rs`)

- Found from the Java side, where `StructComposition.az(event, 500000, 475000)` is the event runner: search the native image for the 500000 (`0x7a120`) operation limit.
- The agent hooks only the event runner, which handles nearly all runs. The interpreter, the trigger runner and the script-by-trigger runner are not hooked.
- The interpreter's signature is `(ctx, &{control, Script*}, max_ops, ?)`. Script fields: `+0x30` instructions, `+0x48` int operands, `+0x78` long operands, `+0x80`/`+0x84`/`+0x88` local int/object/long counts.
- Event fields:
  - `+0x8c` type: 76 for ordinary scripts. 10..17, 0x12, 0x55 and 0x56 are trigger types.
  - `+8` argument count.
  - `+0x10` argument vector of 0x28-byte variants (kind byte at `+0x20`: 1 int, 2 string). Argument 0 is the script id.
  - `+0x48`/`+0x4c` mouse x/y. The placeholder ints `0x80000001..a` are filled from the event.
- The Script's id is at `+0x18` for event runs. Scripts found by trigger show `0xfe310000` there, so their id is unknown.
- An idle account runs about 2000 scripts a second, mostly two per frame. The default blacklist from other tools is for other builds, so the loader needs its own blacklist box.

## Varcs (`offsets/varc_setters.rs`, `client/varc_log.rs`)

- Only the interpreter's varc opcodes write varcs: 42/43 int, 51/52 long, 49/50 object. Server varc packets run scripts.
- The opcodes call methods on one `Varcs` object, which holds a map from index to a variant.
- The setters share the getter's map search and end with `mov byte [varcs+0x80],1`, a dirty flag that makes the client save the varcs.
  - `mov r11b,1` marks the int setter.
  - `mov r11b,5` marks the long setter.
  - The setter with neither is the object setter, which takes a `variant*`.
- A variant is 0x28 bytes with its kind at `+0x20` (2 is a string).
- A string is 0x18 bytes: up to 0x17 inline characters, with the room left in byte `+0x17`. When that byte's top bit is set, the string is on the heap with its pointer at `+0` and its length at `+8`. The empty string is all zeros with `0x17` in the last byte.

## Item containers (`offsets/items.rs`, `offsets/item_setter.rs`)

- The containers are in a global hash table (inventory 93, equipment 94, bank 95). A node is `{id +0, items vector +8, quantities vector +0x20, next +0x38}`.
- The slot writer `set_slot(ecx container, edx slot, r8d item, r9d qty)` (Java `setItemTableSlot`) is the one function that:
  - reads both table globals (buckets and count),
  - calls one resize function twice,
  - and makes two indexed 32-bit stores.
- The writer is split in two chunks, so the rule reads from the first chunk on to the padding. The getters read the same globals but neither resize nor store.
- A full bank update is a few hundred calls in one tick.
- Removing a container is a separate function that isn't hooked, so the log only shows a container being recreated (`old_known` false).

## Ground items (`offsets/ground_items.rs`)

- The server's obj add packet carries more than the Java client reads: ticks until public, ticks until despawn, ownership (0 none, 1 you, 2 another player, 3 group ironman), a never-public flag and option flags. It comes standalone with a packed absolute coord (`z | x << 14 | level << 28`, 17 bytes in `client-2026-11-05`) or inside a zone batch with a zone-local coord.
- Both forms call one insert function (`0x140075e70` in `client-2026-11-05`) with the fields as stack arguments in that order. It stores the two timers as `[client+tick_count] + argument`, so the item holds absolute ticks that never change afterwards, and it sets the option flags through a one-store setter (`mov [rcx+X],dl; ret`). `find_ground_item_ownership` matches those stores on the object whose id field `find_ground_items` found.
- Despawn and pickup both arrive as an obj delete from the server, around the despawn tick. Nothing in the item changes when it turns public.

## Item icons (`offsets/item_icon.rs`, `client/cache.rs`)

- Java `getSprite` is native `(out: &mut [control, sprite], id, qty, border, shadow, text mode, noted: u8, width, height, text: &string)`.
  - The client caches the sprite unless the item is noted, so the agent's calls cost the same as the inventory's own draws.
  - Native text mode 1 draws the number. Java's "when stackable" is decided by the caller.
  - The agent passes border 1, shadow `0x302020`, 36x32 and an empty string, which matches what the inventory shows.
- To find the function:
  1. Find the stack-number builder, the one function holding `<col=ffff00>`, `<col=ffffff>` and `<col=00ff80>` that compares with both thresholds.
  2. Its only caller is a chunk of the icon function. Use `chunk_root`, then check that the prologue homes `r8d` and `r9d`.
- The sprite is a shared pointer pair. The object has its pixel count at `+0` and its pixels at `+0x10`, as `0xAARRGGBB` words with alpha already applied. A pixel count other than width x height means there is no icon.
- An out-of-range id draws a default definition, so the agent refuses ids outside `0..65536`.

## Component fields (`decode/src/widgets.rs`)

- Model: the type is at 0xfc (1 means a model id), the id at 0x100, the rotation at 0x11c/0x120 and the zoom at 0x128. They were found on the certer's quiz, whose buttons hold models 8965-8967.
- The `DETAIL_*` offsets were found by correlation, because the native struct doesn't keep the Java field order. The same method works for any field the cache definition states:
  1. Run `injector widget-dump <pid> 1024` and `injector cache-file <pid> 3 <interface> <component> <out>`.
  2. Decode the definitions with Java `Widget.af`, the IF3 decoder.
  3. For each field, count how many components hold its value at each offset. A real field matches about 98-100%. A chance match shows up as a rate equal to the share of zeros.
- Width, height and their modes match only about 70%, because scripts resize most of the resizable interface.
- Listeners are vectors: onLoad at 0x358, mouseover at 0x448.
- Not found: name, target verb, border type, drag settings, the item quantity mode, and the model fields by correlation (no open interface had a model).

## Objects and the scene's edge

- Object definition transform opcodes 77 and 92 use 2-byte ids, and 97 and 98 use 3-byte ids. 92 and 98 end with a default id.
- An opcode the decoder doesn't know fails the whole definition silently, and the object then has no collision or options. Missing 97 and 98 once hid 842 of 63,320 definitions, doors among them.
- The client doesn't fill the scene's outer rows, which read as open ground. The planner takes those tiles from the cache. Planning on them made false routes and back-and-forth walking.

## Definitions in memory

- Every definition lookup calls one cache reader with the config group in `r8d`. A lookup takes `(out: *mut [u64;2], id: edx)` and returns a {control, object} pair that `release` frees. Locs (6), varps (16) and varcs (19) take the id in `ecx` and return a pointer into the cache. A bad id gives a default definition.
- Found but unused (definitions now come from the disk cache): `NpcType::getModel(npc, out, seq, frame, seq2, frame2, overrides)`, which calls itself for an NPC's other forms; the skeletal check, the leaf `mov eax,[rcx+4]; shr eax,31; xor al,1; ret`, and the sequence frame count (`+0x48`) and array (`+0x50`); the model loader `(out, source = client+0x2570, id)`; `ModelData` lighting (vtable `+0x70`); generic animate `(out, 0x48-byte params)`; the spot animation and loc model builders; set-sequence `(client, actor, seq, delay)`, which works on any actor until the server overrides it.

## Frame (`offsets/frame.rs`, `client/frame.rs`; `0x1409211b0` in `client-2026-11-05`)

- The client has four renderer modes (0 to 3). In 0 and 2 the game thread presents its own frames. In 1 and 3 it hands them to a render thread, which calls `SwapBuffers`, so anything the agent does in the swap hook runs off the game thread there.
- The main loop calls `tick(app, f32)` once per iteration on the game thread (called from `0x14003b73a`). In order, it:
  1. Runs the update step `app->vtable[0xf0](app, bool)` zero or more times (a fixed-step loop).
  2. Asks the renderer whether it takes a frame (`0x140a55bb0`). If it does, runs the draw step `app->vtable[0xf8](app, f32)` with the tick's float in `xmm1`. The draw step builds the scene, the interfaces and the menu.
  3. In modes 0 and 2, presents through `present` (`0x140a5de30`), the one function that calls the `GDI32!SwapBuffers` import. In 1 and 3 the render thread calls `present` instead (from `0x140a55af3`; `0x140a54170` is the third caller).
- `find_frame`:
  - Takes the `SwapBuffers` import slot (or `OPENGL32!wglSwapBuffers`), then `present` as the single function that calls it.
  - Takes `tick` as the single caller of `present` with a virtual-call pair `[reg+d]` then `[reg+d+8]`, where the second passes a float in `xmm1`. The draw slot is `d+8`.
  - The float check is needed: the render thread's caller also has a `+0x10`/`+0x18` pair, on a member object.
- The agent detours `tick` only to reach the app object. On the first call it hooks `vtable[draw_slot]`, so the draw step's address is read rather than guessed. The draw step stays on the game thread in every mode, and returns with the frame's menu complete.

## Render loop (`offsets/render.rs`, `client/drawn.rs`; `0x14065d340` in `client-2026-11-05`)

- One scene loop calls a renderer virtual, `draw(renderer, renderable, orientation, xmm3 + 3 floats, x, y, z, tag, cycle, ...)`, for every renderable it draws, about 1500 times a frame. Almost all calls are static locs. NPCs, players and animated locs are a handful.
  - x, y and z are relative to the camera, and the floats are the camera's yaw and pitch sines and cosines.
  - The tag is the 64-bit argument at `entry rsp+0x58`, and the client cycle is at `+0x60`. For locs, `(tag >> 20) & 0xffff` is the loc id.
- Order inside:
  1. Build the model through the renderable's vtable slot 0 with the cycle, then cache its box for the orientation (`0x14064a3b0`).
  2. Cull it against the view, using the projection settings at `[renderer+0x630c60]` and the model's radius.
  3. Mouse picking: a ray against the model's box, then per-triangle tests. A hit adds the tag to the under-mouse list (`0x14065aa80`).
  4. Classify the render pass: `pass(renderer, renderable, model, tag)`, recognised by returning 4 for tag -2 and 5 for tag -3. It is the one call that sees the renderable and the posed model together, and the draw makes it only for models that survived culling.
  5. Subtract the animation height (renderable vtable `+0x50`) from the height, submit to the GPU through renderer vtable `+0x48`, then release the draw's use of the model.
- Posed models (NPCs, players, animated locs) have the transient flag `model+0x21c` set, which the draw tests right after its GPU submit. They are all posed into one reused scratch model, so its vertices belong to whichever renderable was posed last. Cached models (static locs) are pushed onto the vector at `renderer+0x643d98` the first time they are drawn, with `model+0x218` set to that frame's id.
- Models come from `make_shared`: the control block is always `model - 0x20`, with the use count at `+8` and the weak count at `+0xc`.
- The model's vertices stay in model space; the rotated boxes are cached per orientation at `model+0x238`.
- `find_render` finds `pass` by its returns, then `draw` as the caller of `pass` whose entry stores `r8d` (the orientation) at `[rsp+0x18]`, then the transient flag as the first byte compare after a register call in `draw`. A second caller, `0x14065daf0` (a `(renderer, renderable, r8, r9d)` draw), wasn't hit while standing among NPCs and locs and isn't hooked; it is probably for projectiles or spot animations.

## Definitions on disk (`decode/src/js5.rs`, `decode/src/definitions.rs`, `client/definitions.rs`)

- The client keeps the classic disk cache in the following files: `main_file_cache.dat2` (520-byte sectors) and one `main_file_cache.idx<N>` per index, held open while it runs. Configs are index 2: locs are group 6 and varbits group 14, both bzip2 containers split into one file per id. The reference table (index 255, group 2) is protocol 7.
- Loc opcode 96 is the decoration height (one unsigned byte, `def+0x134` in memory).
- Game vals are index 24: one group per `ipc::state::game_val` kind, one file per id, holding the name.
- Models are index 7 (group = model id), animation frames index 0 (group = frame id >> 16, file = frame id & 0xffff) and framemaps index 1; NPCs are config group 9 and sequences group 12. Model files come in four formats, picked by the last two bytes (`FF FD`, `FF FE`, `FF FF`, otherwise the original).
- A face the client never draws (lit `faceColors3 == -2`): untextured unless its render type is 0, 1 or 3, textured unless it is 0 or 1; a transparency of -1 makes the type 2 and -2 makes it 3 (`ModelData.ev` in the Java client).
- An NPC with one model uses it as it is; several are merged, which drops unused vertices and duplicate coordinates and pulls texture-triangle vertices into the label groups, so it moves animation origins.
- Objs are config group 10. Opcodes 97/98, 139/140 and 148/149 are the link and template of noted, bought and placeholder copies; a copy with a template takes its linked obj's name.
- The client's in-memory definition caches (locs, varbits) are LRU hash tables in `.data`, found but unused: a lookup hit reorders the LRU list and a miss decodes and may evict, so only the game thread may call it. A node holds the id at `+0`, the `{control, definition}` pair at `+8` and the next node of its bucket at `+0x40`; a loc definition holds its id at `+8`. The multiloc transform reads the varbit at `def+0x150`, the varp at `def+0x154` and the ids at `def+0x148` (count at `def+0x140`).

## NPC head icons (`offsets/values.rs` `find_npc_head_icons`)

- The NPC holds a pointer at `+0x760` to a 0x90-byte object of two `std::vector<int>` with inline storage for 8: archive ids at `+0` and sprite indices at `+0x48`. Unset slots are -1.
- Archive 440 is the prayer icons: sprite 0 protects from melee, 1 from missiles and 2 from magic.
- The rule matches the clear function: `mov r,[rcx+X]; test; je; mov qword [rcx+X],0; lea rcx,[r+V]; call F; mov rcx,r; call F`.
- Only icons the server sets are read. Default icons from the NPC's definition are not.

## Native highlights (`offsets/highlights.rs`; test-only until something uses it)

- The client draws entity and tile highlights itself in the 3D renderer (render targets `ObjectHighlight_SilhouetteRT`, `_OutlineRT`, `ObjectHighlightOcclusion`), configured per channel: colour (RGBA), `outlineWidth`, model and tile outline and fill, `alwaysOnTop`, `showInsideEdges`, `swOnly`, `snapToGrid`, `ignoreRebuild`, `saveToFile`. Game content drives it from CS2 (`highlight_npc_on`, `highlight_loc_on`, `highlight_tile_on`, `highlight_*_setup(channel, colour, ..., flags)`; the hover highlight setting is `mouseover_entity_highlight`, channel 9 yellow `0xc0c000` for NPCs and 10 cyan `0x00c0c0` for locs), and the client's Lua plugin system (sol2, mod.io or the `::luaplugin` dev command) binds the same engine as `highlightNpcOn`, `highlightTileOn`, `highlightChannelConfigure` and so on.
- Anchor: each plugin binding is registered by `lea rdx` of its name, with the binding's function pointer loaded by the `lea rax` just before. The thunk pops three ints from the Lua stack and calls the engine's function with the manager `[client global] + 0x896c0` (build c65749a2; `0x89680` and `0x896d0` in older ones) and the three ints in reverse order of popping. The rule takes the first direct call after the `add rcx`; NPC on/off hold on every archived build.
- The Obj and Tile thunks first call a shared argument helper (`0x1405b0320` in c65749a2), so their first direct call isn't the engine function; skip that helper before extending the rule to them. `highlightObjTypeOff` and `highlightChannelConfigure` didn't fit the thunk shape in a byte scan and weren't traced.
