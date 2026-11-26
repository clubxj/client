--!strict
local ChatOptions = require("@volcren/chat_options")
local Host = require("@volcren/host")
local Npcs = require("@volcren/npcs")
local Players = require("@volcren/players")

local IDLE_MS = 6000
local CONTINUE_PAUSE_MS = 300
local PICK_PAUSE_MS = 600
local POLL_MS = 100

local args: { [string]: any } = Host.settings() or {}
local wanted: string? = args.npc
local choose: { string } = args.choose or {}

local function matches(npc: Host.Npc, me: Host.PlayerInfo?): boolean
	if wanted == nil or wanted == "" then
		return me ~= nil and npc.interacting_player == me.index
	end
	local id = tonumber(wanted)
	if id then
		return npc.id == id
	end
	return string.lower(npc.name or "") == string.lower(wanted)
end

local function target(): Host.Npc?
	local me = Players.me()
	local at = Host.game_state().position
	if at == nil then
		return nil
	end
	local best: Host.Npc? = nil
	local best_d = math.huge
	for _, npc in Host.npcs() or {} do
		if matches(npc, me) then
			local d = math.max(math.abs(npc.x - at.x), math.abs(npc.y - at.y))
			if d < best_d then
				best, best_d = npc, d
			end
		end
	end
	return best
end

local npc = target()
if npc == nil then
	print(if wanted then `No NPC matching "{wanted}" nearby.` else "No NPC is interacting with you.")
	return
end
print(`Talking to {npc.name} (id {npc.id}) at {npc.x}, {npc.y}`)
if not Npcs.interact(npc, "Talk-to") then
	print("Talk-to didn't go through.")
	return
end

local next_pick = 1
local last_line = ""
local active_at = Host.time()
while Host.time() - active_at < IDLE_MS do
	local options = ChatOptions.get()
	if options and #options > 0 then
		local title = ChatOptions.title()
		print(`Options{if title then ` ({title})` else ""}: {ChatOptions.to_string(options)}`)
		local pick = choose[next_pick]
		if pick == nil then
			return
		end
		if not ChatOptions.select(pick) then
			print(`Couldn't pick "{pick}".`)
			return
		end
		print(`Picked "{pick}"`)
		next_pick += 1
		active_at = Host.time()
		Host.sleep(PICK_PAUSE_MS)
	elseif ChatOptions.contains_continue() then
		local speech = ChatOptions.speech()
		local line = if speech then `{speech.name}: {speech.text}` else "(a message to continue)"
		if line ~= last_line then
			print(line)
			last_line = line
		end
		ChatOptions.continue_chat()
		active_at = Host.time()
		Host.sleep(CONTINUE_PAUSE_MS)
	else
		Host.sleep(POLL_MS)
	end
end
local at = Host.game_state().position
if at then
	print(`The dialogue ended. You are at {at.x}, {at.y}, plane {at.plane}.`)
end
