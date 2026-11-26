--!strict
local Host = require("@volcren/host")
local menu = Host.menu
local widget = Host.widget
local widget_children = Host.widget_children
local sleep = Host.sleep
local time = Host.time
type Widget = Host.Widget
local Biometrics = require("@volcren/biometrics")
local Geometry = require("@volcren/geometry")
local Keyboard = require("@volcren/keyboard")
local Log = require("@volcren/log")
local Mouse = require("@volcren/mouse")
local Random = require("@volcren/random")
local Toplevel = require("@volcren/toplevel")
local Widgets = require("@volcren/widgets")
local Box = Geometry.Box
type Box = Geometry.Box
local Point = Geometry.Point
type Point = Geometry.Point
type Tile = { read x: number, read y: number, read plane: number? }

local Chartering = {}
local Tab = {}
local Map = {}
Chartering.Tab = Tab
Chartering.Map = Map

Tab.GROUP = 885
Tab.LIST = 4
Tab.TITLE = 7

Tab.OPEN_TIMEOUT_MS = 8000
Tab.POLL_MS = 150
Tab.MAX_SCROLLS = 30

export type Destination = {
	key: string,
	name: string,
	port: number,
	visible: boolean,
	row: Widget,
}

local function list(): Widget?
	local w = widget(Tab.GROUP, Tab.LIST)
	if w == nil or w.hidden then
		return nil
	end
	return w
end

function Tab.is_open(): boolean
	return list() ~= nil
end

function Tab.wait_open(timeout_ms: number?): boolean
	return Widgets.wait_for(Tab.is_open, timeout_ms or Tab.OPEN_TIMEOUT_MS, Tab.POLL_MS)
end

local function split(text: string): (string?, string?)
	local plain = string.gsub(text, "<[^>]*>", "")
	local key, name = string.match(plain, "^%s*(%w+)%s*:%s*(.-)%s*$")
	return key, name
end

function Tab.destinations(): { Destination }?
	local view = list()
	if view == nil then
		return nil
	end
	local children = widget_children(Tab.GROUP, Tab.LIST)
	if children == nil then
		return nil
	end
	local found: { Destination } = {}
	for _, child in ipairs(children) do
		if child.kind == 4 and not child.hidden and child.text ~= nil then
			local key, name = split(child.text)
			if key ~= nil and name ~= nil and name ~= "" then
				table.insert(found, {
					key = key,
					name = name,
					port = (child.index - 1) // 2,
					visible = child.y >= view.y and child.y + child.height <= view.y + view.height,
					row = child,
				})
			end
		end
	end
	table.sort(found, function(a: Destination, b: Destination)
		return a.row.y < b.row.y
	end)
	return found
end

function Tab.find(name: string): Destination?
	local all = Tab.destinations()
	if all == nil then
		return nil
	end
	local wanted = string.lower(name)
	local loose: Destination? = nil
	local loose_count = 0
	for _, d in ipairs(all) do
		local have = string.lower(d.name)
		if have == wanted then
			return d
		elseif string.find(have, wanted, 1, true) or string.find(wanted, have, 1, true) then
			loose = d
			loose_count += 1
		end
	end
	return if loose_count == 1 then loose else nil
end

function Tab.names(): { string }?
	local all = Tab.destinations()
	if all == nil then
		return nil
	end
	local names: { string } = {}
	for _, d in ipairs(all) do
		table.insert(names, d.name)
	end
	return names
end

local function box_of(w: Widget): Box
	return Box.new(w.x, w.y, w.x + w.width - 1, w.y + w.height - 1)
end

function Tab.scroll_to(name: string): Destination?
	local last: number? = nil
	for _ = 1, Tab.MAX_SCROLLS do
		local d = Tab.find(name)
		local view = list()
		if d == nil or view == nil then
			return nil
		end
		if d.visible then
			return d
		end
		if last == d.row.y then
			return nil
		end
		last = d.row.y
		local down = d.row.y > view.y
		Mouse.scroll(Biometrics.random_mode_integer(2, 1, 3), down, box_of(view))
		Random.sleep(80, 250)
	end
	return nil
end

function Tab.select(name: string): boolean
	local d = Tab.scroll_to(name)
	if d == nil then
		return false
	end
	Mouse.click(d.row)
	return true
end

function Tab.key_code(d: Destination): number?
	local key = string.upper(d.key)
	if #key ~= 1 then
		return nil
	end
	local code = string.byte(key)
	if (code >= 48 and code <= 57) or (code >= 65 and code <= 90) then
		return code
	end
	return nil
end

function Tab.press(name: string): boolean
	local d = Tab.find(name)
	if d == nil then
		return false
	end
	local code = Tab.key_code(d)
	if code == nil then
		return false
	end
	Keyboard.key_press(code)
	return true
end

Map.GROUP = 72
Map.CLOSE = 2
Map.MARKERS = 3

Map.MARKER_SPRITE = 6916
Map.HERE_SPRITE = 6918

Map.OPEN_TIMEOUT_MS = 8000
Map.HOVER_TIMEOUT_MS = 800
Map.POLL_MS = 50
Map.SETTLE_MS = 150
Map.GRID = 3
Map.DECOY_CHANCE = 0.3
Map.SECOND_DECOY_CHANCE = 0.5
Map.DECOY_NEARBY = 3
Map.DECOY_PAUSE_MS = { 150, 450 }

export type Marker = {
	index: number,
	name: string?,
	picture: Widget,
	spot: Point,
	here: boolean,
}

function Map.is_open(): boolean
	local w = widget(Map.GROUP, Map.MARKERS)
	return w ~= nil and not w.hidden
end

function Map.wait_open(timeout_ms: number?): boolean
	return Widgets.wait_for(Map.is_open, timeout_ms or Map.OPEN_TIMEOUT_MS, Map.POLL_MS)
end

local function covers(w: Widget, x: number, y: number): boolean
	return x >= w.x and y >= w.y and x < w.x + w.width and y < w.y + w.height
end

function Map.markers(): { Marker }?
	if not Map.is_open() then
		return nil
	end
	local children = widget_children(Map.GROUP, Map.MARKERS)
	if children == nil then
		return nil
	end
	local by_index: { [number]: Widget } = {}
	for _, child in ipairs(children) do
		by_index[child.index] = child
	end
	local found: { Marker } = {}
	for _, picture in ipairs(children) do
		local square = by_index[picture.index + 1]
		if
			picture.kind == 5
			and picture.index % 2 == 0
			and not picture.hidden
			and square ~= nil
			and not square.hidden
		then
			local above: { Widget } = {}
			for _, other in ipairs(children) do
				if other.kind == 3 and other.index % 2 == 1 and other.index > square.index and not other.hidden then
					table.insert(above, other)
				end
			end
			local near: { Point } = {}
			local rest: { Point } = {}
			for y = square.y + 1, square.y + square.height - 2, Map.GRID do
				for x = square.x + 1, square.x + square.width - 2, Map.GRID do
					local free = true
					for _, other in ipairs(above) do
						if covers(other, x, y) then
							free = false
							break
						end
					end
					if free then
						table.insert(if covers(picture, x, y) then near else rest, Point.new(x, y))
					end
				end
			end
			local pool = if #near > 0 then near else rest
			if #pool > 0 then
				table.insert(found, {
					index = picture.index,
					name = square.action,
					picture = picture,
					spot = pool[Random.int(1, #pool)],
					here = picture.sprite == Map.HERE_SPRITE,
				})
			end
		end
	end
	return found
end

local function menu_name(): string?
	local deadline = time() + Map.HOVER_TIMEOUT_MS
	while true do
		for _, entry in ipairs(menu() or {}) do
			if entry.option ~= "Cancel" and entry.option ~= "Close" then
				return entry.option
			end
		end
		if time() >= deadline then
			return nil
		end
		sleep(Map.POLL_MS)
	end
end

function Map.names(): { [number]: string }?
	local markers = Map.markers()
	if markers == nil then
		return nil
	end
	local names: { [number]: string } = {}
	for _, marker in ipairs(markers) do
		if not marker.here and marker.name ~= nil then
			names[marker.index] = marker.name
		end
	end
	return names
end

function Map.find(name: string): Marker?
	local wanted = string.lower(name)
	local loose: Marker? = nil
	local loose_count = 0
	for _, marker in ipairs(Map.markers() or {}) do
		local have = if marker.here then nil else marker.name
		if have ~= nil then
			have = string.lower(have)
			if have == wanted then
				return marker
			elseif string.find(have, wanted, 1, true) or string.find(wanted, have, 1, true) then
				loose = marker
				loose_count += 1
			end
		end
	end
	return if loose_count == 1 then loose else nil
end

local function decoys(target: Marker): { Marker }
	local nearby: { Marker } = {}
	for _, marker in ipairs(Map.markers() or {}) do
		if not marker.here and marker.index ~= target.index then
			table.insert(nearby, marker)
		end
	end
	local function gap(marker: Marker): number
		return math.abs(marker.spot.x - target.spot.x) + math.abs(marker.spot.y - target.spot.y)
	end
	table.sort(nearby, function(a: Marker, b: Marker)
		return gap(a) < gap(b)
	end)
	local picked: { Marker } = {}
	if #nearby == 0 or not Biometrics.random_boolean(Map.DECOY_CHANCE) then
		return picked
	end
	local pool = math.min(Map.DECOY_NEARBY, #nearby)
	local first = Random.int(1, pool)
	table.insert(picked, nearby[first])
	if pool > 1 and Random.chance(Map.SECOND_DECOY_CHANCE) then
		local second = Random.int(1, pool - 1)
		table.insert(picked, nearby[if second >= first then second + 1 else second])
	end
	return picked
end

function Map.select(name: string): boolean
	local marker = Map.find(name)
	if marker == nil then
		return false
	end
	for _, decoy in ipairs(decoys(marker)) do
		Mouse.move(decoy.spot)
		Random.sleep(Map.DECOY_PAUSE_MS[1], Map.DECOY_PAUSE_MS[2])
	end
	Mouse.move(marker.spot)
	sleep(Map.SETTLE_MS)
	if menu_name() ~= marker.name then
		return false
	end
	Mouse.click()
	return true
end

function Map.close(key_probability: number?): boolean
	return Toplevel.close(Map.GROUP, function()
		local w = widget(Map.GROUP, Map.CLOSE)
		return if w ~= nil and not w.hidden then w else nil
	end, if key_probability ~= nil and key_probability > 0 then key_probability else nil)
end

Chartering.KEY_CHANCE = 0.5
Chartering.MAP_CHANCE = 0.4

Chartering.CONFIRM_TIMEOUT_MS = 4000
Chartering.POLL_MS = 100

Chartering.TRADER_RANGE = 12
Chartering.SETTLE_MS = { 250, 500 }

export type Port =
	"Port Sarim"
	| "Brimhaven"
	| "Catherby"
	| "Mos Le'Harmless"
	| "Musa Point"
	| "Port Khazard"
	| "Port Phasmatys"
	| "Karamja Shipyard"
	| "Port Tyras"
	| "Corsair Cove"
	| "Prifddinas"
	| "Port Piscarilius"
	| "Land's End"
	| "Aldarin"
	| "Sunset Coast"
	| "The Pandemonium"
	| "The Summer Shore"
	| "Deepfin Point"
	| "Port Roberts"
	| "Civitas illa Fortis"
	| "Red Rock"

Chartering.Port = table.freeze({
	PortSarim = "Port Sarim",
	Brimhaven = "Brimhaven",
	Catherby = "Catherby",
	MosLeHarmless = "Mos Le'Harmless",
	MusaPoint = "Musa Point",
	PortKhazard = "Port Khazard",
	PortPhasmatys = "Port Phasmatys",
	KaramjaShipyard = "Karamja Shipyard",
	PortTyras = "Port Tyras",
	CorsairCove = "Corsair Cove",
	Prifddinas = "Prifddinas",
	PortPiscarilius = "Port Piscarilius",
	LandsEnd = "Land's End",
	Aldarin = "Aldarin",
	SunsetCoast = "Sunset Coast",
	ThePandemonium = "The Pandemonium",
	TheSummerShore = "The Summer Shore",
	DeepfinPoint = "Deepfin Point",
	PortRoberts = "Port Roberts",
	CivitasIllaFortis = "Civitas illa Fortis",
	RedRock = "Red Rock",
}) :: {
	PortSarim: "Port Sarim",
	Brimhaven: "Brimhaven",
	Catherby: "Catherby",
	MosLeHarmless: "Mos Le'Harmless",
	MusaPoint: "Musa Point",
	PortKhazard: "Port Khazard",
	PortPhasmatys: "Port Phasmatys",
	KaramjaShipyard: "Karamja Shipyard",
	PortTyras: "Port Tyras",
	CorsairCove: "Corsair Cove",
	Prifddinas: "Prifddinas",
	PortPiscarilius: "Port Piscarilius",
	LandsEnd: "Land's End",
	Aldarin: "Aldarin",
	SunsetCoast: "Sunset Coast",
	ThePandemonium: "The Pandemonium",
	TheSummerShore: "The Summer Shore",
	DeepfinPoint: "Deepfin Point",
	PortRoberts: "Port Roberts",
	CivitasIllaFortis: "Civitas illa Fortis",
	RedRock: "Red Rock",
}

export type Method = "key" | "map" | "list"

function Chartering.is_open(): boolean
	return Tab.is_open() or Map.is_open()
end

function Chartering.wait_open(timeout_ms: number?): boolean
	return Widgets.wait_for(Chartering.is_open, timeout_ms or Tab.OPEN_TIMEOUT_MS, Tab.POLL_MS)
end

local function chance(given: number?, default: number): number
	if given ~= nil and given > 0 then
		return given
	end
	return Biometrics.random_double(default)
end

local function plan(name: string, key_probability: number?, map_probability: number?): { Method }
	local in_list = Tab.is_open() and Tab.find(name) ~= nil
	local on_map = Map.is_open()
	local order: { Method } = {}
	local function add(method: Method)
		if not table.find(order, method) then
			table.insert(order, method)
		end
	end
	if in_list and Random.chance(chance(key_probability, Chartering.KEY_CHANCE)) then
		add("key")
	end
	if on_map and Random.chance(chance(map_probability, Chartering.MAP_CHANCE)) then
		add("map")
	end
	if Tab.is_open() then
		add("list")
	end
	if on_map then
		add("map")
	end
	if in_list then
		add("key")
	end
	return order
end

local function gone(): boolean
	return Widgets.wait_for(function()
		return not Chartering.is_open()
	end, Chartering.CONFIRM_TIMEOUT_MS, Chartering.POLL_MS)
end

function Chartering.select(port: Port, key_probability: number?, map_probability: number?): Method?
	local name: string = port
	local order = plan(name, key_probability, map_probability)
	for _, method in ipairs(order) do
		local sent = if method == "key"
			then Tab.press(name)
			elseif method == "map" then Map.select(name)
			else Tab.select(name)
		if sent then
			if gone() then
				return method
			end
			Log.log("Chartering", `{method} to {name} left the interface open`)
		else
			Log.log("Chartering", `couldn't choose {name} by {method}`)
		end
		if not Chartering.is_open() then
			return if sent then method else nil
		end
	end
	return nil
end

function Chartering.find_trader(option: string, near: Tile, range: number?): Host.Npc?
	local npcs = Host.npcs()
	if npcs == nil then
		return nil
	end
	local reach = range or Chartering.TRADER_RANGE
	type Candidate = { npc: Host.Npc, distance: number }
	local candidates: { Candidate } = {}
	for _, n in ipairs(npcs) do
		local d = math.max(math.abs(n.x - near.x), math.abs(n.y - near.y))
		if d <= reach then
			table.insert(candidates, { npc = n, distance = d })
		end
	end
	table.sort(candidates, function(a: Candidate, b: Candidate)
		return a.distance < b.distance
	end)
	local Npcs = (require :: any)("@volcren/npcs")
	for _, c in ipairs(candidates) do
		if table.find(Npcs.options(c.npc.id) or {}, option) ~= nil then
			return c.npc
		end
	end
	return nil
end

function Chartering.open(option: string?, near: Tile?, range: number?): (boolean, string?)
	if Chartering.is_open() then
		return true, nil
	end
	local say = option or "Charter"
	local Npcs = (require :: any)("@volcren/npcs")
	local at = near
	if at == nil then
		local Player = (require :: any)("@volcren/player")
		at = Player.position()
	end
	if at == nil then
		return false, "no position"
	end
	local npc = Chartering.find_trader(say, at, range)
	if npc == nil then
		return false, "no trader nearby"
	end
	if not Npcs.interact(npc, say) then
		return false, `couldn't {say} the trader`
	end
	if not Chartering.wait_open(Tab.OPEN_TIMEOUT_MS) then
		return false, "the destination list didn't open"
	end
	Random.sleep(Chartering.SETTLE_MS[1], Chartering.SETTLE_MS[2])
	return true, nil
end

return Chartering
