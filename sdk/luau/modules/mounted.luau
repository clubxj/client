--!strict
local Host = require("@volcren/host")
local sleep = Host.sleep
local widget_children = Host.widget_children
type Widget = Host.Widget
local Chat = require("@volcren/chat")
local House = require("@volcren/house")
local Mouse = require("@volcren/mouse")
local Objects = require("@volcren/objects")
local Random = require("@volcren/random")
local Toplevel = require("@volcren/toplevel")
local Widgets = require("@volcren/widgets")

local Mounted = {}

export type List = { group: number, list: number, close: number, close_index: number? }

Mounted.LISTS = {
	{ group = 187, list = 3, close = 4 },
	{ group = 947, list = 9, close = 3, close_index = 4 },
} :: { List }
Mounted.LOCKED_COLOUR = "5f5f5f"
Mounted.MENU_OPTION = "Teleport menu"
Mounted.CAPE_OPTION = "Teleport"
Mounted.OPEN_TIMEOUT_MS = 8000
Mounted.CLOSE_TIMEOUT_MS = 3000
Mounted.POLL_MS = 50
Mounted.SETTLE_MS = { 250, 500 }

export type Kind = "glory" | "cape" | "talisman" | "digsite"

export type Destination = {
	name: string,
	available: boolean,
	row: Widget,
}

local function open_list(): List?
	for _, list in ipairs(Mounted.LISTS) do
		if Toplevel.is_open(list.group) then
			return list
		end
	end
	return nil
end

function Mounted.is_open(): boolean
	return open_list() ~= nil
end

function Mounted.destinations(): { Destination }?
	local list = open_list()
	if list == nil then
		return nil
	end
	local found: { Destination } = {}
	for _, row in ipairs(widget_children(list.group, list.list) or {}) do
		if row.kind == 4 and not row.hidden and row.text ~= nil then
			local name = string.match(Chat.clean(row.text), "^%w+:%s*(.+)$")
			if name ~= nil then
				local locked = string.find(row.text, `<col={Mounted.LOCKED_COLOUR}>`, 1, true) ~= nil
				table.insert(found, { name = name, available = not locked, row = row })
			end
		end
	end
	return found
end

function Mounted.find(name: string): Destination?
	local wanted = string.lower(name)
	local match: Destination? = nil
	for _, d in ipairs(Mounted.destinations() or {}) do
		if string.find(string.lower(d.name), wanted, 1, true) ~= nil then
			if match ~= nil then
				return nil
			end
			match = d
		end
	end
	return match
end

function Mounted.select(name: string): boolean
	local d = Mounted.find(name)
	if d == nil or not d.available then
		return false
	end
	Mouse.click(d.row)
	return Widgets.wait_for(function()
		return not Mounted.is_open()
	end, Mounted.CLOSE_TIMEOUT_MS, Mounted.POLL_MS)
end

function Mounted.close(): boolean
	local list = open_list()
	if list == nil then
		return true
	end
	return Toplevel.close(list.group, function(): Widget?
		if list.close_index == nil then
			local button = Host.widget(list.group, list.close)
			return if button ~= nil and not button.hidden then button else nil
		end
		for _, child in ipairs(widget_children(list.group, list.close) or {}) do
			if child.index == list.close_index and not child.hidden then
				return child
			end
		end
		return nil
	end)
end

local function wait_open(): boolean
	if not Widgets.wait_for(Mounted.is_open, Mounted.OPEN_TIMEOUT_MS, Mounted.POLL_MS) then
		return false
	end
	sleep(Random.int(Mounted.SETTLE_MS[1], Mounted.SETTLE_MS[2]))
	return true
end

function Mounted.teleport(kind: Kind, place: string): (boolean, string?, boolean?)
	local names = House.FURNITURE[kind]
	local loc = if names ~= nil then House.reach(names) else nil
	if loc == nil then
		return false, `no {kind} to reach in this house`, true
	end
	if kind == "glory" or kind == "cape" then
		local option = if kind == "cape" then Mounted.CAPE_OPTION else place
		if not Objects.approach(loc, option) then
			return false, `couldn't click {option}`, nil
		end
		return true, nil, nil
	end
	if not Toplevel.escape_clear() then
		return false, "another interface is open", nil
	end
	if not Objects.approach(loc, Mounted.MENU_OPTION) then
		return false, `couldn't click {Mounted.MENU_OPTION}`, nil
	end
	if not wait_open() then
		return false, "the list didn't open", nil
	end
	local d = Mounted.find(place)
	if d == nil or not d.available then
		Mounted.close()
		return false, `{place} isn't there or isn't unlocked`, true
	end
	if not Mounted.select(place) then
		Mounted.close()
		return false, `{place} couldn't be chosen`, nil
	end
	return true, nil, nil
end

return Mounted
