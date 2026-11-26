# Draw

Debug shapes on the game overlay, in screen pixels. Drawings stay until `Draw.clear`; wrap redraws in `Draw.batch` to avoid flicker. A drawing projected from tiles stays where it was painted when the camera moves; use `DebugDraw`'s world drawings for marks that should follow the camera.

```luau
local Draw = require("@volcren/draw")
```

A `Color` is `{ r, g, b, a? }` (0 to 255). Every color argument defaults to red.

A `Layer` is `"world"` (the default) or `"ui"`. World drawings hide under game interfaces (chatbox, minimap, side panel, open windows); use `"ui"` for things that mark interfaces, such as inventory slots or widgets. Every drawing function takes an optional trailing `layer`; inside `Draw.batch` a missing one takes the batch's layer.

## Colors

Definition:
```luau
Draw.RED, Draw.GREEN, Draw.BLUE, Draw.YELLOW, Draw.CYAN, Draw.MAGENTA, Draw.WHITE, Draw.BLACK
```
Ready-made colors.

## Draw.color

Definition:
```luau
function Draw.color(r: number, g: number, b: number, a: number?): Color
```
Makes a color; `a` defaults to 255 (opaque).

Example:
```luau
local Draw = require("@volcren/draw")

local orange = Draw.color(255, 140, 0)
Draw.rect(20, 20, 120, 80, orange)
```

## Draw.rect

Definition:
```luau
function Draw.rect(x1: number, y1: number, x2: number, y2: number, color: Color?, layer: Layer?)
```
Rectangle from top-left to bottom-right.

Example:
```luau
local Draw = require("@volcren/draw")

Draw.rect(50, 50, 200, 120, Draw.GREEN)
```

## Draw.box

Definition:
```luau
function Draw.box(box: Area, color: Color?, layer: Layer?)
```
Rectangle from `{ x, y, width, height }`, such as a widget's bounds.

Example:
```luau
local Draw = require("@volcren/draw")
local Host = require("@volcren/host")

local w = Host.widget(149, 0)
if w then
	Draw.box({ x = w.x, y = w.y, width = w.width, height = w.height }, Draw.YELLOW, "ui")
end
```

## Draw.button

Definition:
```luau
function Draw.button(button: Corners, color: Color?, layer: Layer?)
```
Rectangle from `{ x1, y1, x2, y2 }`.

Example:
```luau
local Draw = require("@volcren/draw")

Draw.button({ x1 = 10, y1 = 10, x2 = 90, y2 = 40 }, Draw.CYAN, "ui")
```

## Draw.line

Definition:
```luau
function Draw.line(a: Point, b: Point, color: Color?, layer: Layer?)
```
Line between two points.

Example:
```luau
local Draw = require("@volcren/draw")

Draw.line({ x = 0, y = 0 }, { x = 200, y = 100 }, Draw.MAGENTA)
```

## Draw.lines

Definition:
```luau
function Draw.lines(segments: { { Point } }, color: Color?, width: number?, layer: Layer?)
```
Many point-pair segments in one call; much faster than looping `Draw.line`. A `width` above 1 draws each segment that many pixels thick, as `width` copies offset across the segment (vertically for flatter segments, horizontally for steeper ones).

Example:
```luau
local Draw = require("@volcren/draw")

Draw.lines({
	{ { x = 0, y = 0 }, { x = 100, y = 0 } },
	{ { x = 100, y = 0 }, { x = 100, y = 100 } },
}, Draw.WHITE)
```

## Draw.polygon

Definition:
```luau
function Draw.polygon(points: { Point }, color: Color?, layer: Layer?)
```
Closed polygon outline.

Example:
```luau
local Draw = require("@volcren/draw")

Draw.polygon({ { x = 50, y = 50 }, { x = 150, y = 60 }, { x = 100, y = 140 } }, Draw.YELLOW)
```

## Draw.quad

Definition:
```luau
function Draw.quad(quad: Quad, color: Color?, layer: Layer?)
```
Quad outline, such as a projected tile.

Example:
```luau
local Draw = require("@volcren/draw")
local Projection = require("@volcren/projection")
local Walker = require("@volcren/walker")

local quad = Projection.tile(assert(Walker.position()))
if quad then
	Draw.quad(quad, Draw.GREEN)
end
```

## Draw.cuboid

Definition:
```luau
function Draw.cuboid(cuboid: { bottom: Quad, top: Quad }, color: Color?, layer: Layer?)
```
3D box outline from its bottom and top quads.

Example:
```luau
local Draw = require("@volcren/draw")
local Host = require("@volcren/host")
local Projection = require("@volcren/projection")

local me = (Host.game_state() or {}).position
local cuboid = me and Projection.cuboid(me.local_x, me.local_y)
if cuboid then
	Draw.cuboid(cuboid, Draw.BLUE)
end
```

## Draw.clear

Definition:
```luau
function Draw.clear()
```
Clears the overlay.

Example:
```luau
local Draw = require("@volcren/draw")

Draw.clear()
```

## Draw.batch

Definition:
```luau
function Draw.batch(paint: () -> (), layer: Layer?)
```
Runs `paint`, collecting its drawing calls (including `Draw.clear`) and sending them as one `Host.draw_batch` update, so the overlay is never shown half drawn. Nested batches join the outer one; if `paint` errors nothing is drawn. `layer` is the default for every drawing inside it that doesn't name one; a nested batch without one keeps the outer batch's.

Example:
```luau
local Draw = require("@volcren/draw")

Draw.batch(function()
	Draw.clear()
	Draw.rect(20, 20, 120, 80)
end)
```

## Draw.record

Definition:
```luau
function Draw.record(paint: () -> (), layer: Layer?): { Host.Shape }
```
Runs `paint` and returns its drawing calls as `Host.draw_batch` shapes without drawing them; batches nested inside it join the list. `layer` works as in `Draw.batch`. If `paint` errors, the error is raised and nothing is returned.

Example:
```luau
local Draw = require("@volcren/draw")

local shapes = Draw.record(function()
	Draw.rect(20, 20, 120, 80)
end)
print(#shapes)
```

## Draw.text

Definition:
```luau
function Draw.text(text: string, x: number, y: number, size: number?, color: Color?, layer: Layer?): number
```
Draws text as 14-segment line glyphs from top-left (`x`, `y`), `size` px tall (default 16): letters (lowercase drawn as capitals), digits, `.`, `-`, `#`, `/` and spaces; any other character leaves a gap. Each character is `0.8 * size` wide. Returns the end x.

Example:
```luau
local Draw = require("@volcren/draw")

Draw.text("#1 0.82", 100, 100, 20, Draw.YELLOW)
```
