--!strict
local Host = require("@volcren/host")
local item_container = Host.item_container
local widget_children = Host.widget_children
type Widget = Host.Widget
local Chat = require("@volcren/chat")
local Interact = require("@volcren/interact")
local Inventory = require("@volcren/inventory")
local Item = require("@volcren/item")
type Item = Item.Item
local Toplevel = require("@volcren/toplevel")
local Widgets = require("@volcren/widgets")

local HuntsmansKit = {}

export type Stored = { slot: number, id: number, quantity: number, name: string? }
export type Amount = number | "All"

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

HuntsmansKit.STORABLE = table.freeze({
	"Bird snare",
	"Noose wand",
	"Insect net",
	"Magic insect net",
	"Butterfly jar",
	"Box trap",
	"Rabbit snare",
	"Unlit torch",
	"Small fishing net",
	"Rope",
	"Magic box",
	"Imp-in-a-box(1)",
	"Imp-in-a-box(2)",
	"Teasing stick",
	"Hunter's spear",
	"Wood camo top",
	"Wood camo legs",
	"Polar camo top",
	"Polar camo legs",
	"Jungle camo top",
	"Jungle camo legs",
	"Desert camo top",
	"Desert camo legs",
	"Larupia hat",
	"Larupia top",
	"Larupia legs",
	"Graahk headdress",
	"Graahk top",
	"Graahk legs",
	"Kyatt hat",
	"Kyatt top",
	"Kyatt legs",
	"Hunter hood",
	"Hunter cape",
	"Hunter cape(t)",
	"Guild hunter headwear",
	"Guild hunter top",
	"Guild hunter legs",
	"Guild hunter boots",
	"Ring of pursuit",
	"Impling jar",
	"Horn of Plenty",
	"Horn of Plenty (empty)",
	"Gryphon feather",
	"Net trap",
} :: { string })

function HuntsmansKit.can_store(name: string): boolean
	local lower = string.lower(name)
	for _, storable in ipairs(HuntsmansKit.STORABLE) do
		if string.lower(storable) == lower then
			return true
		end
	end
	return false
end

local last_seen: { Stored }? = nil

function HuntsmansKit.held(): boolean
	return Inventory.contains(HuntsmansKit.ID)
end

function HuntsmansKit.is_open(): boolean
	return Toplevel.is_open(HuntsmansKit.GROUP)
end

function HuntsmansKit.items(): { Stored }?
	if not HuntsmansKit.is_open() then
		return nil
	end
	local slots = item_container(HuntsmansKit.CONTAINER)
	if slots == nil then
		return nil
	end
	local stored: { Stored } = {}
	local ids: { number } = {}
	for _, s in ipairs(slots) do
		if s.id > 0 and s.id ~= HuntsmansKit.EMPTY_SLOT and s.quantity > 0 then
			table.insert(stored, { slot = s.slot, id = s.id, quantity = s.quantity })
			table.insert(ids, s.id)
		end
	end
	local names = if #ids > 0 then Item.names(ids) else nil
	if names then
		for i, s in ipairs(stored) do
			s.name = names[i]
		end
	end
	last_seen = stored
	return stored
end

function HuntsmansKit.last_seen(): { Stored }?
	return HuntsmansKit.items() or last_seen
end

function HuntsmansKit.quantity(item: Item): number?
	local stored = HuntsmansKit.last_seen()
	if stored == nil then
		return nil
	end
	local matcher = Item.matcher(item)
	local total = 0
	for _, s in ipairs(stored) do
		if Item.matches(matcher, s.id, s.name) then
			total += s.quantity
		end
	end
	return total
end

function HuntsmansKit.open(): boolean
	if HuntsmansKit.is_open() then
		return true
	end
	if not Inventory.interact(HuntsmansKit.ID, HuntsmansKit.OPTION) then
		return false
	end
	return Widgets.wait_within(function(): boolean
		return HuntsmansKit.items() ~= nil
	end, HuntsmansKit.TIMEOUT_MS, HuntsmansKit.POLL_MS)
end

local function close_button(): Widget?
	for _, child in ipairs(widget_children(HuntsmansKit.GROUP, HuntsmansKit.FRAME) or {}) do
		if child.action == HuntsmansKit.CLOSE_OPTION then
			return child
		end
	end
	return nil
end

function HuntsmansKit.close(): boolean
	if not HuntsmansKit.is_open() then
		return true
	end
	HuntsmansKit.items()
	return Toplevel.close(HuntsmansKit.GROUP, close_button)
end

local function stored_cell(item: Item): Widget?
	local matcher = Item.matcher(item)
	for _, child in ipairs(widget_children(HuntsmansKit.GROUP, HuntsmansKit.ITEMS) or {}) do
		if
			child.item_id > 0
			and child.item_id ~= HuntsmansKit.EMPTY_SLOT
			and Item.matches(matcher, child.item_id, Item.name(child.item_id))
		then
			return child
		end
	end
	return nil
end

local function moved(item: Item, stored: boolean): () -> boolean
	local before = HuntsmansKit.quantity(item) or 0
	local carried = Inventory.count_ex(item) or 0
	return function(): boolean
		local now = HuntsmansKit.quantity(item) or before
		local held = Inventory.count_ex(item) or carried
		return if stored then now > before and held < carried else now < before and held > carried
	end
end

local function pick(verb: string, amount: Amount, choose: (string) -> boolean): boolean
	if amount == 1 or amount == 5 or amount == "All" then
		return choose(`{verb}-{amount}`)
	end
	return choose(`{verb}-X`) and Chat.answer_query(HuntsmansKit.QUERY, tostring(amount), HuntsmansKit.TIMEOUT_MS)
end

function HuntsmansKit.withdraw(item: Item, amount: Amount): boolean
	local cell = stored_cell(item)
	if cell == nil then
		return false
	end
	local done = moved(item, false)
	local w = cell :: Widget
	return pick("Withdraw", amount, function(option: string): boolean
		return Interact.with(w, option, nil, false) == true
	end) and Widgets.wait_within(done, HuntsmansKit.TIMEOUT_MS, HuntsmansKit.POLL_MS)
end

function HuntsmansKit.deposit(item: Item, amount: Amount): boolean
	local slot = Inventory.index_of(item)
	local box = if slot and HuntsmansKit.is_open() then Inventory.box(slot) else nil
	if box == nil then
		return false
	end
	local done = moved(item, true)
	return pick("Deposit", amount, function(option: string): boolean
		return Interact.with(box, option, nil, false) == true
	end) and Widgets.wait_within(done, HuntsmansKit.TIMEOUT_MS, HuntsmansKit.POLL_MS)
end

local function press(child: number, option: string, changed: () -> boolean): boolean
	local button = Widgets.get(HuntsmansKit.GROUP, child)
	if button == nil then
		return false
	end
	return Interact.with(button, option, nil, false) == true
		and Widgets.wait_within(changed, HuntsmansKit.TIMEOUT_MS, HuntsmansKit.POLL_MS)
end

local function used_slots(): number
	return #(Inventory.items() or {})
end

function HuntsmansKit.fill(): boolean
	local before = used_slots()
	return press(HuntsmansKit.FILL, "Fill", function(): boolean
		return used_slots() < before
	end)
end

function HuntsmansKit.empty(): boolean
	local before = used_slots()
	return press(HuntsmansKit.EMPTY, "Empty", function(): boolean
		return used_slots() > before
	end)
end

return HuntsmansKit
