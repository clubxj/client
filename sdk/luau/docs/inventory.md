# Inventory

The 28-slot inventory: every [Inv](inv.md) function bound to it (on-screen slots are the dynamic children of `149:0`), plus selecting, combining and dropping.

```luau
local Inventory = require("@volcren/inventory")
```

## Constants

Definition:
```luau
Inventory.CONTAINER = 93
Inventory.SIZE = 28
Inventory.GROUP = 149
Inventory.LOCKED_SLOTS_VARBIT = 5422
Inventory.IGNORE_LOCKS_VARBIT = 5450
Inventory.shift_enabled = true
```
Container, size and interface. `LOCKED_SLOTS_VARBIT` holds one bit per slot locked in the bank or deposit box (slot 0 is bit 0); `IGNORE_LOCKS_VARBIT` is the bank setting that makes "Deposit inventory" ignore them. `shift_enabled` is cleared by `shift_drop_slots` once it finds Shift doesn't make the drop option the left-click option.

## Inventory.PATTERN_HABIT

Definition:
```luau
Inventory.PATTERN_HABIT = {
	turn = { 1, 2.5 },
	sloppy = { 0.12, 0.37 },
	finish = { 1.5, 3 },
	stick = { 0.85, 0.95 },
	corner_left = 0.7,
	corner_top = 0.7,
	along = 0.8,
	across = 1.3,
}
```
Ranges each account's `pattern` habit is drawn from: `turn`, `sloppy`, `finish`, `stick` are ranges; `corner_left`/`corner_top` are chances for the fallback start corner; `along`/`across` are per-slot move costs on and off the preferred axis.

## Inventory functions from Inv

`slots`, `items`, `item`, `indices_of`, `index_of`, `find`, `contains`, `contains_any`, `contains_all`, `find_different`, `contains_different`, `count`, `count_ex`, `wait_count`, `quantity`, `slot_quantity`, `used`, `empty`, `is_used`, `is_empty`, `count_used`, `count_empty`, `wait_used`, `open`, `boxes`, `box`, `slot_at`, `near`, `random_near`, `hover_slot`, `click_slot`, `move_slot`, `interact_slot`, `hover`, `click`, `move`, `interact`: as in [Inv](inv.md). Mouse functions open the tab first.

Example:
```luau
local Inventory = require("@volcren/inventory")

Inventory.interact("Bones", "Bury")
```

## Inventory.is_open

Definition:
```luau
function Inventory.is_open(): boolean
```
Whether the inventory tab is the open side panel.

Example:
```luau
local Inventory = require("@volcren/inventory")

print(Inventory.is_open())
```

## Inventory.free_slots

Definition:
```luau
function Inventory.free_slots(): number?
```
Empty slot count (`count_empty`).

Example:
```luau
local Inventory = require("@volcren/inventory")

print(Inventory.free_slots())
```

## Inventory.locked_slots

Definition:
```luau
function Inventory.locked_slots(): { [number]: boolean }?
```
Slots locked from the bank or deposit box side panel (`Lock-slot`), by slot index; `nil` when unreadable. A lock belongs to the slot, not the item: the "Deposit inventory" buttons skip locked slots unless `locks_ignored`, while depositing a locked slot's item from its own menu still works. Lock and unlock with `Bankmain.lock_slot` or `DepositBox.lock_slot`.

Example:
```luau
local Inventory = require("@volcren/inventory")

for slot in pairs(Inventory.locked_slots() or {}) do
	print("locked", slot)
end
```

## Inventory.is_full

Definition:
```luau
function Inventory.is_full(): boolean
```
Whether no slot is empty; false when unreadable.

Example:
```luau
local Inventory = require("@volcren/inventory")

print(Inventory.is_full())
```

## Inventory.selected

Definition:
```luau
function Inventory.selected(): number?
```
The slot of the item selected with "Use", read from the client; nil when none.

Example:
```luau
local Inventory = require("@volcren/inventory")

print(Inventory.selected())
```

## Inventory.is_selected

Definition:
```luau
function Inventory.is_selected(slot: number): boolean
```
Whether `slot` holds the selected item.

Example:
```luau
local Inventory = require("@volcren/inventory")

print(Inventory.is_selected(0))
```

## Inventory.deselect

Definition:
```luau
function Inventory.deselect(): boolean
```
Cancels any pending selection and waits for it to clear: a selected inventory item is used on itself, anything else (a spell, another interface's item) gets "Cancel" from the menu. True when nothing was selected.

Example:
```luau
local Inventory = require("@volcren/inventory")

Inventory.deselect()
```

## Inventory.select_slot

Definition:
```luau
function Inventory.select_slot(slot: number): boolean
```
Picks "Use" on `slot` and waits until selected, cancelling another selection first. True at once if already selected.

Example:
```luau
local Inventory = require("@volcren/inventory")

Inventory.select_slot(0)
```

## Inventory.select

Definition:
```luau
function Inventory.select(items: Item | { Item }): boolean
```
Selects a random matching slot, or keeps a selection that already matches. False when none match.

Example:
```luau
local Inventory = require("@volcren/inventory")

Inventory.select("Knife")
```

## Inventory.combine_slots

Definition:
```luau
function Inventory.combine_slots(slot_a: number, slot_b: number): boolean
```
Uses `slot_a` on `slot_b` after a short random pause and waits for the selection to clear.

Example:
```luau
local Inventory = require("@volcren/inventory")

Inventory.combine_slots(0, 1)
```

## Inventory.combine

Definition:
```luau
function Inventory.combine(items_a: Item | { Item }, items_b: Item | { Item }): boolean
```
Uses one of `items_a` on one of `items_b`, choosing the closest pair of distinct slots. False when either is missing.

Example:
```luau
local Inventory = require("@volcren/inventory")

Inventory.combine("Knife", "Logs")
```

## Inventory.pattern

Definition:
```luau
function Inventory.pattern(slots: { number }?, from: Point?): ({ number }, { boolean })
```
A fresh human-like order to click `slots` (all 28 by default), plus each slot's mode (`true` snake, `false` spiral). Starts at the slot nearest `from` (default the mouse), else the account's corner. Each step is a weighted random pick favouring near slots, the preferred axis, keeping direction and, in snake mode, finishing the current line; the mode starts on a coin flip and flips with chance `1 - stick`.

The habit (rows vs columns from `Biometrics.PATTERN`, turn, sloppy, finish, stick, corner) comes from the Biometrics hash within `PATTERN_HABIT`, so it is stable per account and random per run without `Biometrics.setup`. `examples/inventory_patterns.luau` draws the orders.

Example:
```luau
local Inventory = require("@volcren/inventory")

print(table.concat((Inventory.pattern({ 3, 9, 14, 20 })), ","))
```

## Inventory.DropOptions

Definition:
```luau
export type DropOptions = {
	attempts: number?,
	shift_ms: { number }?,
	gap_ms: { number }?,
	confirm: boolean?,
	option: string?,
}
Inventory.DROP_ATTEMPTS = 5
Inventory.DROP_OPTION = "Drop"
Inventory.DROP_SHIFT_MS = { 95, 170 }
Inventory.DROP_GAP_MS = { 0, 250 }
```
Options for `drop_slots` and `shift_drop_slots`; each falls back to the constant below it. `attempts` is the number of passes, `shift_ms` the `{ lo, hi }` pause after pressing Shift and `gap_ms` the pause after each drop click. Shorter pauses drop faster but look less like a person. `confirm = false` makes one pass and returns true once the clicks are made, without waiting for the server to empty the slots, so for about a tick the inventory still shows the dropped items; a caller that reads the inventory right after must allow for that. `option` is the item's drop option when the game names it otherwise, such as "Release" for caught creatures; Shift-click uses it the same way.

## Inventory.drop_slots

Definition:
```luau
function Inventory.drop_slots(slots: { number }, opts: DropOptions?): boolean
```
Drops `slots` in order with the drop option (`opts.option`, default "Drop"), cancelling a selection first, then waits and retries slots still holding their item, up to `opts.attempts` passes. True when all are gone.

Example:
```luau
local Inventory = require("@volcren/inventory")

Inventory.drop_slots({ 0, 1, 2 })
```

## Inventory.drop

Definition:
```luau
function Inventory.drop(items: Item | { Item }, pattern: { number }?): boolean
```
Drops every matching item in `pattern` order (default a new `Inventory.pattern` of their slots). False when none match.

Example:
```luau
local Inventory = require("@volcren/inventory")

Inventory.drop({ "Logs", "Oak logs" })
```

## Inventory.shift_drop_slots

Definition:
```luau
function Inventory.shift_drop_slots(slots: { number }, opts: DropOptions?): boolean
```
Drops `slots` in order by Shift-clicking, checking before each click that the drop option (`opts.option`, default "Drop") is the left-click option. If it isn't (the game's shift-drop setting is off), clears `shift_enabled` and finishes with `drop_slots` and the same `opts`, as do later calls. Shift is always released.

Example:
```luau
local Inventory = require("@volcren/inventory")

Inventory.shift_drop_slots(Inventory.indices_of("Logs") or {})
```

## Inventory.shift_drop

Definition:
```luau
function Inventory.shift_drop(items: Item | { Item }, pattern: { number }?): boolean
```
`drop` using `shift_drop_slots`.

Example:
```luau
local Inventory = require("@volcren/inventory")

Inventory.shift_drop("Raw shrimps")
```

## Inventory.locks_ignored

Definition:
```luau
function Inventory.locks_ignored(): boolean
```
Whether the bank setting that makes "Deposit inventory" ignore locked slots is on.

Example:
```luau
local Inventory = require("@volcren/inventory")

print(Inventory.locks_ignored())
```

## Inventory.wait_slot_locked

Definition:
```luau
function Inventory.wait_slot_locked(slot: number, locked: boolean, timeout_ms: number?): boolean
```
Waits up to `timeout_ms` (1200 by default) for `slot` to be locked (`locked` true) or unlocked; `true` once it is.

Example:
```luau
local Inventory = require("@volcren/inventory")

print(Inventory.wait_slot_locked(0, true))
```
