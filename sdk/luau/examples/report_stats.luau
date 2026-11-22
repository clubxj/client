--!strict
local Host = require("@volcren/host")
local StatsReport = require("@volcren/stats_report")

local tracker = StatsReport.new(15000)
local sent = 0
while true do
	local state = Host.game_state()
	local before = tracker.xp
	if StatsReport.update(tracker, state.skills) then
		sent += 1
		print(`report {sent}: {before} xp`)
	end
	Host.sleep(600)
end
