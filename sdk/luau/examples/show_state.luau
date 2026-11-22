--!strict
local Host = require("@volcren/host")
local game_state = Host.game_state
local players = Host.players
local Camera = require("@volcren/camera")
local Chatbox = require("@volcren/chatbox")
local GameVal = require("@volcren/game_val")
local Inventory = require("@volcren/inventory")
local Minimap = require("@volcren/minimap")
local Npcs = require("@volcren/npcs")
local Player = require("@volcren/player")
local SidePanels = require("@volcren/side_panels")
local Toplevel = require("@volcren/toplevel")
local Wornitems = require("@volcren/wornitems")

local state = game_state()
local position = Player.position()
if position then
	print(string.format("position: (%d, %d) plane %d", position.x, position.y, position.plane))
else
	print("position unknown (logged out?)")
end
local run_energy, weight = state.run_energy, state.weight
if run_energy and weight then
	print(string.format("run energy %.2f%%, weight %d kg", run_energy / 100, weight))
end
local minimap = Minimap.state()
if minimap then
	print(`minimap zoom: {minimap.zoom}, compass: {Camera.yaw_degrees()}`)
end

local tab = SidePanels.current()
print("open tab:", tab and SidePanels.name(tab) or "none")
for _, o in ipairs(Toplevel.interfaces() or {}) do
	print(`interface open: {GameVal.interface(o.group) or "?"} ({o.group}, {o.slot})`)
end
for _, group in ipairs(Chatbox.interfaces() or {}) do
	print(`chatbox interface open: {GameVal.interface(group) or "?"} ({group})`)
end

local items = assert(Inventory.items())
print(string.format("inventory: %d items, %d free slots", #items, assert(Inventory.free_slots())))
for _, item in ipairs(items) do
	print(string.format("  slot %d: %s (%d) x%d", item.slot, item.name or "?", item.id, item.quantity))
end
for name, item in pairs(assert(Wornitems.worn())) do
	print(string.format("  wearing %s: item %d", name, item.id))
end

if position then
	local banker = Npcs.nearest("Banker", position)
	if banker then
		print(
			string.format(
				"nearest banker: %d tiles away at (%d, %d)",
				Npcs.distance(banker, position),
				banker.x,
				banker.y
			)
		)
	end
end
print("players around:", #assert(players()))
