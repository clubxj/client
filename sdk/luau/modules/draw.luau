--!strict
local Host = require("@volcren/host")
local draw_rect = Host.draw_rect
local clear_overlay = Host.clear_overlay
local draw_line = Host.draw_line
local draw_lines = Host.draw_lines
local draw_batch = Host.draw_batch
local Geometry = require("@volcren/geometry")
type Point = Geometry.Point
type Quad = Geometry.Quad
type Area = Host.Area
type Corners = Host.Corners
type Shape = Host.Shape
export type Layer = Host.Layer
local Draw = {}

local batch: { Shape }? = nil
local batch_layer: Layer? = nil

export type Color = {
	r: number,
	g: number,
	b: number,
	a: number?,
}

function Draw.color(r: number, g: number, b: number, a: number?): Color
	return { r = r, g = g, b = b, a = a or 255 }
end

Draw.RED = Draw.color(255, 0, 0)
Draw.GREEN = Draw.color(0, 255, 0)
Draw.BLUE = Draw.color(0, 0, 255)
Draw.YELLOW = Draw.color(255, 255, 0)
Draw.CYAN = Draw.color(0, 255, 255)
Draw.MAGENTA = Draw.color(255, 0, 255)
Draw.WHITE = Draw.color(255, 255, 255)
Draw.BLACK = Draw.color(0, 0, 0)

local function shape(
	shapes: { Shape },
	kind: "rect" | "line",
	x1: number,
	y1: number,
	x2: number,
	y2: number,
	c: Color,
	layer: Layer?
)
	table.insert(
		shapes,
		{ kind = kind, x1 = x1, y1 = y1, x2 = x2, y2 = y2, r = c.r, g = c.g, b = c.b, a = c.a, layer = layer }
	)
end

local function layer_of(layer: Layer?): Layer?
	return layer or batch_layer
end

function Draw.rect(x1: number, y1: number, x2: number, y2: number, color: Color?, layer: Layer?)
	local c = color or Draw.RED
	x1, y1, x2, y2 = math.round(x1), math.round(y1), math.round(x2), math.round(y2)
	if batch then
		shape(batch, "rect", x1, y1, x2, y2, c, layer_of(layer))
	else
		draw_rect(x1, y1, x2, y2, c.r, c.g, c.b, c.a or 255, layer)
	end
end

function Draw.box(box: Area, color: Color?, layer: Layer?)
	Draw.rect(box.x, box.y, box.x + box.width - 1, box.y + box.height - 1, color, layer)
end

function Draw.button(button: Corners, color: Color?, layer: Layer?)
	Draw.rect(button.x1, button.y1, button.x2, button.y2, color, layer)
end

function Draw.line(a: Point, b: Point, color: Color?, layer: Layer?)
	local c = color or Draw.RED
	if batch then
		shape(batch, "line", a.x, a.y, b.x, b.y, c, layer_of(layer))
	else
		draw_line(math.round(a.x), math.round(a.y), math.round(b.x), math.round(b.y), c.r, c.g, c.b, c.a or 255, layer)
	end
end

local function widened(segments: { { Point } }, width: number): { { Point } }
	local reach = math.floor((width - 1) / 2)
	local out = table.create(#segments * width) :: { { Point } }
	for _, s in ipairs(segments) do
		local a, b = s[1], s[2]
		local flat = math.abs(b.x - a.x) >= math.abs(b.y - a.y)
		for d = -reach, width - 1 - reach do
			local dx, dy = if flat then 0 else d, if flat then d else 0
			table.insert(out, { { x = a.x + dx, y = a.y + dy }, { x = b.x + dx, y = b.y + dy } })
		end
	end
	return out
end

function Draw.lines(segments: { { Point } }, color: Color?, width: number?, layer: Layer?)
	if width and width > 1 then
		segments = widened(segments, width)
	end
	local c = color or Draw.RED
	if batch then
		local shape_layer = layer_of(layer)
		for _, s in ipairs(segments) do
			shape(batch, "line", s[1].x, s[1].y, s[2].x, s[2].y, c, shape_layer)
		end
		return
	end
	local lines = table.create(#segments) :: { { number } }
	for i, s in ipairs(segments) do
		lines[i] = { s[1].x, s[1].y, s[2].x, s[2].y }
	end
	draw_lines(lines, c.r, c.g, c.b, c.a or 255, layer)
end

function Draw.polygon(points: { Point }, color: Color?, layer: Layer?)
	local segments = table.create(#points) :: { { Point } }
	for i, p in ipairs(points) do
		segments[i] = { p, points[i % #points + 1] }
	end
	Draw.lines(segments, color, nil, layer)
end

function Draw.quad(quad: Quad, color: Color?, layer: Layer?)
	Draw.polygon(Geometry.Quad.corners(quad), color, layer)
end

function Draw.cuboid(cuboid: { bottom: Quad, top: Quad }, color: Color?, layer: Layer?)
	local bottom, top = Geometry.Quad.corners(cuboid.bottom), Geometry.Quad.corners(cuboid.top)
	local segments = {}
	for i = 1, 4 do
		local j = i % 4 + 1
		table.insert(segments, { bottom[i], bottom[j] })
		table.insert(segments, { top[i], top[j] })
		table.insert(segments, { bottom[i], top[i] })
	end
	Draw.lines(segments, color, nil, layer)
end

local SEGMENTS: { [string]: string } = {
	["0"] = "a b c d e f",
	["1"] = "b c",
	["2"] = "a b g1 g2 e d",
	["3"] = "a b g1 g2 c d",
	["4"] = "f g1 g2 b c",
	["5"] = "a f g1 g2 c d",
	["6"] = "a f g1 g2 e d c",
	["7"] = "a b c",
	["8"] = "a b c d e f g1 g2",
	["9"] = "a b c d f g1 g2",
	["-"] = "g1 g2",
	["#"] = "b c g1 g2 f",
	["/"] = "j k",
	A = "a b c e f g1 g2",
	B = "a b c d i l g2",
	C = "a d e f",
	D = "a b c d i l",
	E = "a d e f g1",
	F = "a e f g1",
	G = "a c d e f g2",
	H = "b c e f g1 g2",
	I = "a d i l",
	J = "b c d e",
	K = "e f g1 j m",
	L = "d e f",
	M = "b c e f h j",
	N = "b c e f h m",
	O = "a b c d e f",
	P = "a b e f g1 g2",
	Q = "a b c d e f m",
	R = "a b e f g1 g2 m",
	S = "a c d f g1 g2",
	T = "a i l",
	U = "b c d e f",
	V = "e f j k",
	W = "b c e f k m",
	X = "h j k m",
	Y = "h j l",
	Z = "a d j k",
}

local function segment(name: string, x: number, y: number, w: number, h: number): { Point }
	local mx, my = x + w / 2, y + h / 2
	local ends: { [string]: { number } } = {
		a = { x, y, x + w, y },
		b = { x + w, y, x + w, my },
		c = { x + w, my, x + w, y + h },
		d = { x, y + h, x + w, y + h },
		e = { x, my, x, y + h },
		f = { x, y, x, my },
		g1 = { x, my, mx, my },
		g2 = { mx, my, x + w, my },
		h = { x, y, mx, my },
		i = { mx, y, mx, my },
		j = { x + w, y, mx, my },
		k = { mx, my, x, y + h },
		l = { mx, my, mx, y + h },
		m = { mx, my, x + w, y + h },
	}
	local e = ends[name]
	return { { x = e[1], y = e[2] }, { x = e[3], y = e[4] } }
end

function Draw.text(text: string, x: number, y: number, size: number?, color: Color?, layer: Layer?): number
	local h = size or 16
	local w = h * 0.5
	local gap = h * 0.3
	local cx = x
	local segments: { { Point } } = {}
	for i = 1, #text do
		local ch = string.upper(string.sub(text, i, i))
		if ch == "." then
			table.insert(segments, { { x = cx + 1, y = y + h }, { x = cx + 2, y = y + h } })
			cx += gap
		else
			local names = SEGMENTS[ch]
			if names then
				for name in string.gmatch(names, "%S+") do
					table.insert(segments, segment(name, cx, y, w, h))
				end
			end
			cx += w + gap
		end
	end
	if #segments > 0 then
		Draw.lines(segments, color, nil, layer)
	end
	return cx
end

function Draw.clear()
	if batch then
		local clear: Shape = { kind = "clear" }
		table.clear(batch)
		table.insert(batch, clear)
	else
		clear_overlay()
	end
end

function Draw.batch(paint: () -> (), layer: Layer?)
	local outer_layer = batch_layer
	batch_layer = layer or outer_layer
	if batch then
		local ok, err = pcall(paint)
		batch_layer = outer_layer
		if not ok then
			error(err, 0)
		end
		return
	end
	local shapes: { Shape } = {}
	batch = shapes
	local ok, err = pcall(paint)
	batch = nil
	batch_layer = outer_layer
	if not ok then
		error(err, 0)
	end
	draw_batch(shapes)
end

return Draw
