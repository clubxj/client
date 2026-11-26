# Mounted

Teleports from house-mounted pieces, named by `Kind`: `"glory"`, `"cape"` (mythical cape), `"talisman"` (Xeric's) and `"digsite"` (pendant). Trips are free. The glory offers places as options and the cape's "Teleport" has one place; the talisman and pendant list theirs under "Teleport menu". The mounted pieces open group 187 (the generic numbered `menu`), while 947 (`menu_new`) is kept for items that may still open it; `Mounted.LISTS` holds both, each with its row layer and close button. Configurable first options and "Remove" are never used. `Transports.house` builds on this, and `Transports.use_item` reads the same list when a teleport item opens it (Xeric's talisman's "Rub", the Construction cape's "Teleport").

```luau
local Mounted = require("@volcren/mounted")
```

## Mounted.is_open

Definition:
```luau
function Mounted.is_open(): boolean
```
Whether either list in `LISTS` is open. Other menus share them, so this is only meaningful after opening one of these.

Example:
```luau
print(Mounted.is_open())
```

## Mounted.destinations

Definition:
```luau
function Mounted.destinations(): { Destination }?
```
The open list's places top to bottom, each with `name`, `available` (false when locked) and `row`. Nil when not open.

Example:
```luau
for _, d in ipairs(Mounted.destinations() or {}) do
	print(d.name, d.available)
end
```

## Mounted.find

Definition:
```luau
function Mounted.find(name: string): Destination?
```
The single place whose name contains `name`, case-insensitive. Nil when none or several match.

Example:
```luau
print(Mounted.find("Glade") ~= nil)
```

## Mounted.select

Definition:
```luau
function Mounted.select(name: string): boolean
```
Clicks the place and waits for the list to close. False when not found, locked, or still open after `CLOSE_TIMEOUT_MS`. The trip isn't waited for.

Example:
```luau
print(Mounted.select("Glade"))
```

## Mounted.close

Definition:
```luau
function Mounted.close(): boolean
```
Closes the list via `Toplevel.close`. True once closed.

Example:
```luau
Mounted.close()
```

## Mounted.teleport

Definition:
```luau
function Mounted.teleport(kind: Kind, place: string): (boolean, string?, boolean?)
```
Reaches the piece on any floor of the current house and starts the teleport to `place` (not waited for). On failure returns false and a reason, with a third `true` when retrying is pointless (the piece is missing, or the place is missing or locked).

Example:
```luau
print(Mounted.teleport("talisman", "Glade"))
```
