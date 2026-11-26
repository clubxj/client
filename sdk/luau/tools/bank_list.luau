--!strict
local Host = require("@volcren/host")

local BLOCK = 512
local SAME_BANK = 12
local MAP = { x1 = 0, y1 = 0, x2 = 6655, y2 = 13823 }

type Found = { x: number, y: number, plane: number, name: string }

local found: { Found } = {}
for x = MAP.x1, MAP.x2, BLOCK do
	for y = MAP.y1, MAP.y2, BLOCK do
		local area = { x1 = x, y1 = y, x2 = x + BLOCK - 1, y2 = y + BLOCK - 1 }
		for _, loc in ipairs(Host.cache_locs(area, { actions = { "Bank", "Collect" } }) or {}) do
			local has: { [string]: boolean } = {}
			for _, action in ipairs(loc.actions) do
				has[action] = true
			end
			if has.Bank or (has.Use and has.Collect) then
				table.insert(found, { x = loc.x, y = loc.y, plane = loc.plane, name = loc.name })
			end
		end
	end
end

local banks: { { Found } } = {}
for _, f in ipairs(found) do
	local joined = false
	for _, bank in ipairs(banks) do
		for _, other in ipairs(bank) do
			if other.plane == f.plane and math.max(math.abs(other.x - f.x), math.abs(other.y - f.y)) <= SAME_BANK then
				table.insert(bank, f)
				joined = true
				break
			end
		end
		if joined then
			break
		end
	end
	if not joined then
		table.insert(banks, { f })
	end
end

local lines = { "--!strict", "return {" }
for _, bank in ipairs(banks) do
	local pick = bank[1]
	for _, f in ipairs(bank) do
		if pick.name == "" and f.name ~= "" then
			pick = f
		end
	end
	local name = string.gsub(pick.name, "<[^>]*>", "")
	table.insert(lines, string.format("\t{ x = %d, y = %d, plane = %d, name = %q },", pick.x, pick.y, pick.plane, name))
end
table.insert(lines, "}")
print(table.concat(lines, "\n"))
