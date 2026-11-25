# HuntsmansKit

The huntsman's kit (item 29309) and its interface (group 871, "View" on the kit): stored items on the right, Fill and Empty buttons, and the inventory left in place beside it. The kit holds up to 32 kinds of hunter gear (traps, nets, jars, the guild hunter outfit and more), never whistles, coins, pouches or chinchompas. Items come out stacked: 50 box traps take one inventory slot.

The contents are item container 855, which the client only fills while the interface is open and clears when it closes, so `items` needs the kit open. `last_seen` keeps the last contents read in this script for deciding what to take without opening it again.

```luau
local HuntsmansKit = require("@volcren/huntsmans_kit")
```

## Constants

Definition:
```luau
HuntsmansKit.ID = 29309
HuntsmansKit.GROUP = 871
HuntsmansKit.FRAME = 1
HuntsmansKit.FILL = 8
HuntsmansKit.EMPTY = 9
HuntsmansKit.ITEMS = 11
HuntsmansKit.CONTAINER = 855
HuntsmansKit.EMPTY_SLOT = 6512
HuntsmansKit.OPTION = "View"
HuntsmansKit.CLOSE_OPTION = "Close"
HuntsmansKit.QUERY = "Enter amount"
HuntsmansKit.TIMEOUT_MS = 2500
HuntsmansKit.POLL_MS = 50
```
The kit's item id, widget ids (frame with the close button, the Fill and Empty buttons, the stored items), the contents container, the filler item drawn in unused cells, options, the Withdraw-X/Deposit-X question, and the wait/poll times.

## HuntsmansKit.STORABLE

Definition:
```luau
HuntsmansKit.STORABLE = { "Bird snare", "Noose wand", "Insect net", ... }
```
Every item name the kit accepts, from the game's list: traps and nets, jars, torches, rope, magic boxes, teasing sticks and hunter's spears, the camo, larupia, graahk, kyatt and guild hunter outfits, hunter hoods and capes, the ring of pursuit, impling jars, the Horn of Plenty and gryphon feathers. Whistles, pouches, coins, knives and axes aren't on it.

## HuntsmansKit.can_store

Definition:
```luau
function HuntsmansKit.can_store(name: string): boolean
```
Whether the kit accepts the item, by name, case-insensitive.

Example:
```luau
print(HuntsmansKit.can_store("Box trap"), HuntsmansKit.can_store("Knife")) --> true false
```

## Types

Definition:
```luau
export type Stored = { slot: number, id: number, quantity: number, name: string? }
export type Amount = number | "All"
```

## HuntsmansKit.held

Definition:
```luau
function HuntsmansKit.held(): boolean
```
Whether the kit is in the inventory.

## HuntsmansKit.is_open

Definition:
```luau
function HuntsmansKit.is_open(): boolean
```
Whether the interface is open.

## HuntsmansKit.items

Definition:
```luau
function HuntsmansKit.items(): { Stored }?
```
The kit's contents with names, in kit order. Nil while the interface is closed.

## HuntsmansKit.last_seen

Definition:
```luau
function HuntsmansKit.last_seen(): { Stored }?
```
`items` when the kit is open, otherwise the contents read the last time it was open in this script. Nil if it hasn't been opened yet.

## HuntsmansKit.quantity

Definition:
```luau
function HuntsmansKit.quantity(item: Item): number?
```
How many of an item (id or name) the kit holds, from `last_seen`; 0 when none. Nil if the kit hasn't been opened yet.

Example:
```luau
if HuntsmansKit.open() then
	print(HuntsmansKit.quantity("Box trap"))
	HuntsmansKit.close()
end
```

## HuntsmansKit.open

Definition:
```luau
function HuntsmansKit.open(): boolean
```
Uses "View" on the kit in the inventory and waits for its contents. True once open, also if it already was.

## HuntsmansKit.close

Definition:
```luau
function HuntsmansKit.close(): boolean
```
Reads the contents one last time for `last_seen`, then closes the interface via `Toplevel.close`. True once closed, also if it wasn't open.

## HuntsmansKit.withdraw

Definition:
```luau
function HuntsmansKit.withdraw(item: Item, amount: Amount): boolean
```
With the interface open, takes 1, 5, `"All"` or any other amount (typed into Withdraw-X) of an item. True once the kit holds less and the inventory more.

## HuntsmansKit.deposit

Definition:
```luau
function HuntsmansKit.deposit(item: Item, amount: Amount): boolean
```
With the interface open, stores 1, 5, `"All"` or any other amount (Deposit-X) of an inventory item. True once the kit holds more and the inventory less.

Example:
```luau
if HuntsmansKit.open() then
	HuntsmansKit.withdraw("Box trap", 50)
	HuntsmansKit.deposit("Bird snare", "All")
	HuntsmansKit.close()
end
```

## HuntsmansKit.fill

Definition:
```luau
function HuntsmansKit.fill(): boolean
```
With the interface open, presses Fill, which stores every storable item in the inventory. True once the inventory uses fewer slots; false when there was nothing to store.

## HuntsmansKit.empty

Definition:
```luau
function HuntsmansKit.empty(): boolean
```
With the interface open, presses Empty, which takes out as much as fits, emptying from the kit's last slot. True once the inventory uses more slots.
