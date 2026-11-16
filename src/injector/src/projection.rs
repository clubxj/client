use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use decode::camera::{Camera, NEAR_PLANE, Window, truncate};
use ipc::{
    read::{self, Memory},
    state::{Layout, part},
};

pub const WALL: u8 = 0;
pub const WALL_DECORATION: u8 = 1;
pub const GAME_OBJECT: u8 = 2;
pub const GROUND_DECORATION: u8 = 3;

const NEEDS: u64 = part::PROJECTION | part::CAMERA | part::INTERFACES;

const TILE_SHIFT: i32 = 7;
const TILE: i32 = 1 << TILE_SHIFT;
const PLANES: i32 = crate::collision::PLANES as i32;
const BRIDGE: u8 = 2;
const TILE_ENTRY_SIZE: usize = 16;
const DECORATION_RETRY: Duration = Duration::from_millis(600);
const TILE_ENTRY_OBJECT: usize = 8;
const VERTEX_SIZE: usize = 12;
const MAX_TRIANGLES: usize = 4096;
const PAINT_CORNERS: [[i32; 2]; 4] = [[0, 0], [1, 0], [1, 1], [0, 1]];
const PAINT_TRIANGLES: [[usize; 3]; 2] = [[1, 2, 3], [0, 1, 3]];

type Vertex = [i32; 3];
type Triangle = [Vertex; 3];

fn i32s<const N: usize>(memory: &impl Memory, address: u64) -> Option<[i32; N]> {
    let mut bytes = [[0u8; 4]; N];
    memory
        .read(address, bytes.as_flattened_mut())
        .then(|| bytes.map(i32::from_le_bytes))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneKey {
    scene: u64,
    pub plane: i32,
    base: [i32; 2],
    heights: u64,
    tiles: u64,
}

fn scene_and_trig(
    memory: &impl Memory,
    layout: &Layout,
    client: u64,
) -> Option<(SceneKey, [f32; 4])> {
    let p = &layout.projection;
    let world_view = memory.ptr(client + p.top_world_view as u64)?;
    let has_player = layout.has(part::PLAYER);
    let fields = [
        (p.scene as u64, 8),
        (p.plane as u64, 4),
        (layout.player.base_x as u64, 8),
    ];
    let view = Window::new(
        memory,
        world_view,
        &fields[..if has_player { 3 } else { 2 }],
    );
    let scene = view.ptr(p.scene as u64)?;
    let base = if has_player {
        view.i32s(layout.player.base_x as u64)?
    } else {
        [0; 2]
    };
    let plane = view.i32(p.plane as u64)?;
    let fields = Window::new(
        memory,
        scene,
        &[
            (p.heights as u64, 8),
            (p.tiles as u64, 8),
            (p.trig as u64, 16),
        ],
    );
    let key = SceneKey {
        scene,
        plane,
        base,
        heights: fields.ptr(p.heights as u64)?,
        tiles: fields.ptr(p.tiles as u64)?,
    };
    let trig: [[u8; 4]; 4] = fields.get(p.trig as u64)?;
    Some((key, trig.map(f32::from_le_bytes)))
}

pub struct Terrain {
    key: SceneKey,
    size: [i32; 2],
    heights: Vec<i32>,
    settings: Vec<u8>,
    tiles: Vec<u64>,
    meshes: Vec<Option<Option<Vec<Triangle>>>>,
    decorations: HashMap<(i32, i32, i32), i32>,
    decorated: bool,
    decorations_failed: Option<Instant>,
}

impl Terrain {
    fn read(memory: &impl Memory, layout: &Layout, key: SceneKey) -> Option<Self> {
        let p = &layout.projection;
        let [size_x, size_y] = [
            memory.i32(key.scene + p.size_x as u64)?,
            memory.i32(key.scene + p.size_y as u64)?,
        ];
        if !(1..=1024).contains(&size_x) || !(1..=1024).contains(&size_y) {
            return None;
        }
        let count = (PLANES * size_x * size_y) as usize;
        let heights = memory.i32s(key.heights, (PLANES * (size_x + 1) * (size_y + 1)) as usize)?;
        let mut settings = vec![0u8; count];
        let settings_at = memory.ptr(key.scene + p.settings as u64)?;
        let mut entries = vec![0u8; count * TILE_ENTRY_SIZE];
        if !memory.read(settings_at, &mut settings) || !memory.read(key.tiles, &mut entries) {
            return None;
        }
        let tiles = entries
            .as_chunks::<TILE_ENTRY_SIZE>()
            .0
            .iter()
            .map(|e| u64::from_le_bytes(e.as_chunks::<8>().0[TILE_ENTRY_OBJECT / 8]))
            .collect();
        Some(Self {
            key,
            size: [size_x, size_y],
            heights,
            settings,
            tiles,
            meshes: vec![None; count],
            decorations: HashMap::new(),
            decorated: false,
            decorations_failed: None,
        })
    }

    pub fn key(&self) -> SceneKey {
        self.key
    }

    pub fn setting(&self, plane: i32, x: i32, y: i32) -> Option<u8> {
        self.tile_index(plane, x, y).map(|i| self.settings[i])
    }

    pub fn size(&self) -> [i32; 2] {
        self.size
    }

    fn set_decorations(&mut self, decorations: impl IntoIterator<Item = ((i32, i32, i32), i32)>) {
        self.decorations = decorations
            .into_iter()
            .filter(|&(_, height)| height != 0)
            .collect();
        self.decorated = true;
    }

    fn decorate(&mut self, decorations: impl FnOnce() -> Option<Vec<Decoration>>) {
        if self.decorated
            || self
                .decorations_failed
                .is_some_and(|at| at.elapsed() < DECORATION_RETRY)
        {
            return;
        }
        match decorations() {
            Some(decorations) => self.set_decorations(decorations),
            None => self.decorations_failed = Some(Instant::now()),
        }
    }

    fn in_scene(&self, x: i32, y: i32) -> bool {
        (0..self.size[0]).contains(&x) && (0..self.size[1]).contains(&y)
    }

    fn tile_index(&self, plane: i32, x: i32, y: i32) -> Option<usize> {
        ((0..PLANES).contains(&plane) && self.in_scene(x, y))
            .then(|| ((plane * self.size[0] + x) * self.size[1] + y) as usize)
    }

    fn height(&self, plane: i32, x: i32, y: i32) -> i32 {
        let [size_x, size_y] = self.size;
        let index = (plane.wrapping_mul(size_x + 1).wrapping_add(x))
            .wrapping_mul(size_y + 1)
            .wrapping_add(y);
        usize::try_from(index)
            .ok()
            .and_then(|i| self.heights.get(i))
            .copied()
            .unwrap_or(0)
    }

    fn bilinear(&self, x: i32, y: i32, plane: i32, (tx, ty): (i32, i32)) -> i32 {
        let bridge = plane < 3
            && self
                .tile_index(1, tx, ty)
                .is_some_and(|i| self.settings[i] & BRIDGE != 0);
        let plane = if bridge { plane + 1 } else { plane };
        let (ax, ay) = (
            x.wrapping_sub(tx << TILE_SHIFT),
            y.wrapping_sub(ty << TILE_SHIFT),
        );
        let h = |dx: i32, dy: i32| self.height(plane, tx + dx, ty + dy);
        let north = h(0, 1)
            .wrapping_mul(TILE.wrapping_sub(ax))
            .wrapping_add(h(1, 1).wrapping_mul(ax))
            >> TILE_SHIFT;
        let south = h(1, 0)
            .wrapping_mul(ax)
            .wrapping_add(h(0, 0).wrapping_mul(TILE.wrapping_sub(ax)))
            >> TILE_SHIFT;
        south
            .wrapping_mul(TILE.wrapping_sub(ay))
            .wrapping_add(north.wrapping_mul(ay))
            >> TILE_SHIFT
    }

    fn mesh(&mut self, memory: &impl Memory, layout: &Layout, index: usize) -> Option<&[Triangle]> {
        if self.meshes[index].is_none() {
            self.meshes[index] = Some(self.read_mesh(memory, layout, self.tiles[index]));
        }
        self.meshes[index].as_ref().and_then(|mesh| mesh.as_deref())
    }

    fn read_mesh(&self, memory: &impl Memory, layout: &Layout, tile: u64) -> Option<Vec<Triangle>> {
        let p = &layout.projection;
        if let Some(model) = memory.ptr(tile + p.tile_model as u64) {
            let [begin, end] = [
                memory.ptr(model + p.model_triangles as u64)?,
                memory.ptr(model + p.model_triangles as u64 + 8)?,
            ];
            let count = (end.checked_sub(begin)? as usize / VERTEX_SIZE).min(MAX_TRIANGLES);
            let indices = memory.i32s(begin, count * 3)?;
            let vertex_count = usize::try_from(*indices.iter().max()?).ok()? + 1;
            if vertex_count > MAX_TRIANGLES * 3 {
                return None;
            }
            let vertices = memory.i32s(
                memory.ptr(model + p.model_vertices as u64)?,
                vertex_count * 3,
            )?;
            let vertex = |i: i32| -> Option<Vertex> {
                let at = usize::try_from(i).ok()? * 3;
                vertices.get(at..at + 3)?.try_into().ok()
            };
            return indices
                .as_chunks::<3>()
                .0
                .iter()
                .map(|t| Some([vertex(t[0])?, vertex(t[1])?, vertex(t[2])?]))
                .collect();
        }
        memory.ptr(tile + p.tile_paint as u64)?;
        let [x, y, plane] = i32s(memory, tile + p.tile_x as u64)?;
        let corners = PAINT_CORNERS.map(|[dx, dy]| {
            let (cx, cy) = (x.wrapping_add(dx), y.wrapping_add(dy));
            [
                cx << TILE_SHIFT,
                self.height(plane, cx, cy),
                cy << TILE_SHIFT,
            ]
        });
        Some(
            PAINT_TRIANGLES
                .iter()
                .map(|t| t.map(|i| corners[i]))
                .collect(),
        )
    }

    pub fn heightmap(&self, x: i32, y: i32, plane: i32) -> i32 {
        self.bilinear(x, y, plane, (x >> TILE_SHIFT, y >> TILE_SHIFT))
    }

    fn flat_ground(&self, x: i32, y: i32, plane: i32) -> i32 {
        if self.in_scene(x >> TILE_SHIFT, y >> TILE_SHIFT) {
            self.heightmap(x, y, plane)
        } else {
            0
        }
    }

    fn ground_on(
        &mut self,
        memory: &impl Memory,
        layout: &Layout,
        x: i32,
        y: i32,
        plane: i32,
        on: Option<[i32; 2]>,
    ) -> i32 {
        let (tx, ty) = match on {
            Some([tx, ty]) if self.in_scene(tx, ty) => (tx, ty),
            _ => (x >> TILE_SHIFT, y >> TILE_SHIFT),
        };
        if !self.in_scene(tx, ty) {
            return 0;
        }
        let Some(index) = self
            .tile_index(plane, tx, ty)
            .filter(|&i| self.tiles[i] != 0)
        else {
            return self.bilinear(x, y, plane, (tx, ty));
        };
        let on_mesh = self.mesh(memory, layout, index).and_then(|triangles| {
            triangles
                .iter()
                .find(|t| inside(x, y, t))
                .map(|t| barycentric(x, y, t))
        });
        let height = on_mesh.unwrap_or_else(|| self.bilinear(x, y, plane, (tx, ty)));
        height.wrapping_sub(self.decorations.get(&(plane, tx, ty)).copied().unwrap_or(0))
    }
}

fn inside(px: i32, py: i32, [a, b, c]: &Triangle) -> bool {
    let (xs, ys) = ([a[0], b[0], c[0]], [a[2], b[2], c[2]]);
    if ys.iter().all(|&y| py < y)
        || ys.iter().all(|&y| py > y)
        || xs.iter().all(|&x| px < x)
        || xs.iter().all(|&x| px > x)
    {
        return false;
    }
    let side = |p: &Vertex, q: &Vertex| {
        (py.wrapping_sub(p[2]))
            .wrapping_mul(q[0].wrapping_sub(p[0]))
            .wrapping_sub((q[2].wrapping_sub(p[2])).wrapping_mul(px.wrapping_sub(p[0])))
    };
    let (d1, d2, d3) = (side(a, b), side(b, c), side(c, a));
    match d1.signum() {
        0 if d2 == 0 => true,
        0 if d2 < 0 => d3 <= 0,
        0 => d3 >= 0,
        -1 => d2 <= 0 && d3 <= 0,
        _ => d2 >= 0 && d3 >= 0,
    }
}

fn barycentric(px: i32, py: i32, [a, b, c]: &Triangle) -> i32 {
    let [(ax, ah, ay), (bx, bh, by), (cx, ch, cy)] = [a, b, c].map(|v| (v[0], v[1], v[2]));
    let (dx, dy) = (px.wrapping_sub(cx), py.wrapping_sub(cy));
    let (ay_cy, by_cy, cx_bx, ax_cx) = (
        ay.wrapping_sub(cy),
        by.wrapping_sub(cy),
        cx.wrapping_sub(bx),
        ax.wrapping_sub(cx),
    );
    let den = ay_cy
        .wrapping_mul(cx_bx)
        .wrapping_add(by_cy.wrapping_mul(ax_cx)) as f32;
    let wa = by_cy.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(cx_bx)) as f32 / den;
    let wb = ay_cy
        .wrapping_neg()
        .wrapping_mul(dx)
        .wrapping_add(dy.wrapping_mul(ax_cx)) as f32
        / den;
    let wc = 1.0 - wa - wb;
    truncate(bh as f32 * wb + ah as f32 * wa + wc * ch as f32)
}

pub type Decoration = ((i32, i32, i32), i32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub tile: Option<[i32; 2]>,
    pub heightmap: bool,
}

impl Point {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self {
            x,
            y,
            z,
            tile: None,
            heightmap: false,
        }
    }
}

pub fn ensure_terrain<'a>(
    memory: &impl Memory,
    layout: &Layout,
    terrain: &'a mut Option<Terrain>,
    decorations: impl FnOnce() -> Option<Vec<Decoration>>,
) -> Result<&'a mut Terrain, String> {
    let client = client(memory, layout)?;
    scene_terrain(memory, layout, client, terrain, decorations).map(|(_, _, terrain)| terrain)
}

fn unavailable(what: &str) -> String {
    format!("couldn't read the {what}")
}

fn client(memory: &impl Memory, layout: &Layout) -> Result<u64, String> {
    if !layout.has(NEEDS) {
        return Err("agent didn't find what local projection reads".into());
    }
    read::client(memory, layout).ok_or_else(|| unavailable("client"))
}

fn scene_terrain<'a>(
    memory: &impl Memory,
    layout: &Layout,
    client: u64,
    terrain: &'a mut Option<Terrain>,
    decorations: impl FnOnce() -> Option<Vec<Decoration>>,
) -> Result<(SceneKey, [f32; 4], &'a mut Terrain), String> {
    let (key, trig) = scene_and_trig(memory, layout, client).ok_or_else(|| unavailable("scene"))?;
    let mut current = match terrain.take() {
        Some(kept) if kept.key == key => kept,
        stale => match Terrain::read(memory, layout, key) {
            Some(fresh) => fresh,
            None => {
                *terrain = stale;
                return Err(unavailable("terrain"));
            }
        },
    };
    current.decorate(decorations);
    Ok((key, trig, terrain.insert(current)))
}

pub struct Projector {
    camera: Camera,
    at: [i32; 3],
}

impl Projector {
    pub fn point(&self, [dx, dh, dy]: [i32; 3]) -> Option<[f32; 2]> {
        let [x, height, y] = self.at;
        let (screen, distance) = self.camera.project(
            x.wrapping_add(dx),
            y.wrapping_add(dy),
            height.wrapping_add(dh),
        );
        (distance >= NEAR_PLANE && screen.iter().all(|v| v.is_finite())).then_some(screen)
    }
}

pub fn projector(
    memory: &impl Memory,
    layout: &Layout,
    terrain: &mut Option<Terrain>,
    [x, y]: [i32; 2],
    decorations: impl FnOnce() -> Option<Vec<Decoration>>,
) -> Result<Projector, String> {
    let client = client(memory, layout)?;
    let (key, trig, terrain) = scene_terrain(memory, layout, client, terrain, decorations)?;
    let camera = Camera::read(memory, layout, client, trig).ok_or_else(|| unavailable("camera"))?;
    let ground = terrain.ground_on(memory, layout, x, y, key.plane, None);
    Ok(Projector {
        camera,
        at: [x, ground, y],
    })
}

const YAW_UNITS: i32 = 16384;

pub fn yaw_turn(from: i32, to: i32) -> f32 {
    let units = to.wrapping_sub(from).rem_euclid(YAW_UNITS);
    let units = if units >= YAW_UNITS / 2 {
        units - YAW_UNITS
    } else {
        units
    };
    units as f32 * std::f32::consts::TAU / YAW_UNITS as f32
}

const ZOOM_DOUBLING: f32 = 256.0;

pub fn zoom_scale(from: i32, to: i32) -> f32 {
    ((to - from) as f32 / ZOOM_DOUBLING).exp2()
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Viewpoint {
    pub shift: Option<[i32; 3]>,
    pub turn: Option<f32>,
    pub zoom_scale: Option<f32>,
}

pub fn project(
    memory: &impl Memory,
    layout: &Layout,
    terrain: &mut Option<Terrain>,
    points: &[Point],
    viewpoint: Viewpoint,
    clip: bool,
    decorations: impl FnOnce() -> Option<Vec<Decoration>>,
) -> Result<Vec<[f32; 2]>, String> {
    let client = client(memory, layout)?;
    let (key, trig, terrain) = scene_terrain(memory, layout, client, terrain, decorations)?;
    let mut camera =
        Camera::read(memory, layout, client, trig).ok_or_else(|| unavailable("camera"))?;
    let focal = if viewpoint.shift.is_some() || viewpoint.turn.is_some() {
        let [fx, _, fy] = i32s::<3>(memory, client + layout.camera.focal as u64)
            .ok_or_else(|| unavailable("camera's focal point"))?;
        [fx, fy]
    } else {
        [0, 0]
    };
    if let Some(radians) = viewpoint.turn {
        camera = camera.turned(focal, radians);
    }
    if let Some(scale) = viewpoint.zoom_scale {
        camera = camera.zoomed(scale);
    }
    if let Some([dx, dy, plane]) = viewpoint.shift {
        let [fx, fy] = focal;
        let from = terrain.ground_on(memory, layout, fx, fy, key.plane, None);
        let to = terrain.ground_on(
            memory,
            layout,
            fx.wrapping_add(dx),
            fy.wrapping_add(dy),
            if plane < 0 { key.plane } else { plane },
            None,
        );
        let [x, z, y] = camera.position;
        camera.position = [
            x.wrapping_add(dx),
            z.wrapping_add(to.wrapping_sub(from)),
            y.wrapping_add(dy),
        ];
    }
    Ok(points
        .iter()
        .map(|p| {
            let ground = if p.heightmap {
                terrain.flat_ground(p.x, p.y, key.plane)
            } else {
                terrain.ground_on(memory, layout, p.x, p.y, key.plane, p.tile)
            };
            let (screen, distance) = camera.project(p.x, p.y, ground.wrapping_add(p.z));
            if !clip || distance >= NEAR_PLANE {
                screen
            } else {
                [f32::NAN; 2]
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Fake;
    use ipc::state::ProjectionLayout;

    const CLIENT: u64 = 0x1000;
    const WORLD_VIEW: u64 = 0x2000;
    const SCENE: u64 = 0x3000;
    const HEIGHTS: u64 = 0x10_0000;
    const SETTINGS: u64 = 0x20_0000;
    const TILES: u64 = 0x30_0000;
    const TILE: u64 = 0x40_0000;
    const MODEL: u64 = 0x50_0000;
    const CANVAS: u64 = 0x6000;
    const SIZE: i32 = 4;

    fn layout() -> Layout {
        let mut layout = Layout {
            found: NEEDS | part::GAME_STATE,
            module_base: 0x10,
            client: 0x20,
            game_state: 0x8,
            ..Layout::default()
        };
        layout.camera.position = 0x100;
        layout.interfaces.canvas = 0x200;
        layout.interfaces.ui_scale = 0x68;
        layout.projection = ProjectionLayout {
            top_world_view: 0x300,
            plane: 0x18,
            scene: 0x10,
            size_x: 0x948,
            size_y: 0x94c,
            tiles: 0x968,
            zoom: 0x400,
            view_holder: 0x1b0,
            view: 0x8,
            view_center: 0x14,
            view_offset_x: 0x58,
            view_offset_y: 0x50,
            trig: 0x418,
            canvas_size: 0x30,
            ui_size: 0x6c,
            heights: 0x3d0,
            settings: 0x400,
            tile_x: 0x20,
            tile_model: 0x130,
            tile_paint: 0x128,
            ground_decoration: 0x118,
            model_vertices: 0x8,
            model_triangles: 0x50,
            decoration_height: 0x134,
        };
        layout
    }

    fn put_f32(memory: &mut Fake, address: u64, value: f32) {
        memory.put(address, &value.to_le_bytes());
    }

    fn world() -> (Fake, Layout) {
        let layout = layout();
        let p = layout.projection;
        let mut m = Fake::default();
        m.put_u64(0x10 + 0x20, CLIENT);
        m.put_u64(CLIENT + p.top_world_view as u64, WORLD_VIEW);
        m.put_u64(WORLD_VIEW + p.scene as u64, SCENE);
        m.put_i32(WORLD_VIEW + p.plane as u64, 0);
        m.put_i32(SCENE + p.size_x as u64, SIZE);
        m.put_i32(SCENE + p.size_y as u64, SIZE);
        m.put_u64(SCENE + p.heights as u64, HEIGHTS);
        m.put_u64(SCENE + p.settings as u64, SETTINGS);
        m.put_u64(SCENE + p.tiles as u64, TILES);
        for plane in 0..4 {
            for x in 0..=SIZE {
                for y in 0..=SIZE {
                    let index = (plane * (SIZE + 1) + x) * (SIZE + 1) + y;
                    m.put_i32(HEIGHTS + index as u64 * 4, -(plane * 1000 + x * 10 + y));
                }
            }
        }
        m.put(SETTINGS, &[0; (4 * SIZE * SIZE) as usize]);
        m.put(TILES, &[0; (4 * SIZE * SIZE) as usize * 16]);
        for (i, v) in [1.0f32, 0.0, 0.0, 1.0].into_iter().enumerate() {
            put_f32(&mut m, SCENE + p.trig as u64 + i as u64 * 4, v);
        }
        for (i, v) in [0, -1000, 0].into_iter().enumerate() {
            m.put_i32(CLIENT + 0x100 + i as u64 * 4, v);
        }
        m.put_i32(CLIENT + p.zoom as u64, 500);
        m.put_u64(CLIENT + 0x200, CANVAS);
        m.put_u64(CANVAS + 0x1b0 + 0x8, CANVAS + 0xf8);
        m.put_i32(CANVAS + 0xf8 + 0x14, 400);
        m.put_i32(CANVAS + 0xf8 + 0x18, 300);
        m.put_i32(CANVAS + 0xf8 + 0x58, 4);
        m.put_i32(CANVAS + 0xf8 + 0x50, 2);
        m.put_i32(CANVAS + 0x30, 800);
        m.put_i32(CANVAS + 0x34, 600);
        m.put_i32(CANVAS + 0x6c, 400);
        m.put_i32(CANVAS + 0x70, 300);
        put_f32(&mut m, CANVAS + 0x68, 2.0);
        (m, layout)
    }

    fn put_tile(m: &mut Fake, layout: &Layout, plane: i32, x: i32, y: i32, tile: u64) {
        let index = ((plane * SIZE + x) * SIZE + y) as u64;
        m.put_u64(TILES + index * 16 + 8, tile);
        let p = layout.projection;
        for (i, v) in [x, y, plane].into_iter().enumerate() {
            m.put_i32(tile + p.tile_x as u64 + i as u64 * 4, v);
        }
    }

    fn key(m: &Fake, layout: &Layout) -> SceneKey {
        scene_and_trig(m, layout, CLIENT).unwrap().0
    }

    fn camera(m: &Fake, layout: &Layout) -> Camera {
        let (_, trig) = scene_and_trig(m, layout, CLIENT).unwrap();
        Camera::read(m, layout, CLIENT, trig).unwrap()
    }

    fn ground(m: &Fake, layout: &Layout, x: i32, y: i32) -> i32 {
        Terrain::read(m, layout, key(m, layout))
            .unwrap()
            .ground_on(m, layout, x, y, 0, None)
    }

    #[test]
    fn heightmap_between_corners() {
        let (m, layout) = world();
        assert_eq!(ground(&m, &layout, 128, 256), -12);
        assert_eq!(ground(&m, &layout, 192, 256), -17);
        assert_eq!(
            ground(&m, &layout, 192, 320),
            (((-17 * 64) + (-18 * 64)) >> 7)
        );
        assert_eq!(ground(&m, &layout, -1, 0), 0);
        assert_eq!(ground(&m, &layout, SIZE * 128, 0), 0);
    }

    #[test]
    fn bridges_use_the_plane_above() {
        let (mut m, layout) = world();
        let bridge_setting = (SIZE + 1) * SIZE + 2;
        m.put(SETTINGS + bridge_setting as u64, &[BRIDGE]);
        assert_eq!(ground(&m, &layout, 128, 256), -1012);
    }

    #[test]
    fn an_edge_takes_the_ground_of_the_tile_it_outlines() {
        let (mut m, layout) = world();
        m.put(SETTINGS + ((SIZE + 2) * SIZE + 2) as u64, &[BRIDGE]);
        let mut terrain = Terrain::read(&m, &layout, key(&m, &layout)).unwrap();
        assert_ne!(terrain.ground_on(&m, &layout, 256, 320, 0, None), -23);
        assert_eq!(
            terrain.ground_on(&m, &layout, 256, 320, 0, Some([1, 2])),
            -23
        );
    }

    #[test]
    fn paint_follows_its_own_plane() {
        let (mut m, layout) = world();
        put_tile(&mut m, &layout, 0, 1, 2, TILE);
        m.put_u64(TILE + layout.projection.tile_paint as u64, 0x1);
        m.put_i32(TILE + layout.projection.tile_x as u64 + 8, 1);
        assert_eq!(ground(&m, &layout, 128, 256), -1012);
        assert_eq!(ground(&m, &layout, 255, 383), -1022);
    }

    #[test]
    fn models_pick_the_first_triangle_holding_the_point() {
        let (mut m, layout) = world();
        let p = layout.projection;
        put_tile(&mut m, &layout, 0, 0, 0, TILE);
        m.put_u64(TILE + p.tile_model as u64, MODEL);
        let vertices: [Vertex; 4] = [
            [0, -100, 0],
            [128, -200, 0],
            [128, -300, 128],
            [0, -400, 128],
        ];
        for (i, v) in vertices.iter().flatten().enumerate() {
            m.put_i32(MODEL + 0x1000 + i as u64 * 4, *v);
        }
        m.put_u64(MODEL + p.model_vertices as u64, MODEL + 0x1000);
        for (i, v) in [0, 1, 2, 0, 2, 3].into_iter().enumerate() {
            m.put_i32(MODEL + 0x2000 + i as u64 * 4, v);
        }
        m.put_u64(MODEL + p.model_triangles as u64, MODEL + 0x2000);
        m.put_u64(MODEL + p.model_triangles as u64 + 8, MODEL + 0x2000 + 24);
        let [first, second] = [
            [vertices[0], vertices[1], vertices[2]],
            [vertices[0], vertices[2], vertices[3]],
        ];
        assert_eq!(ground(&m, &layout, 0, 0), -100);
        assert_eq!(ground(&m, &layout, 64, 64), -200);
        assert_eq!(ground(&m, &layout, 96, 32), barycentric(96, 32, &first));
        assert_eq!(ground(&m, &layout, 0, 127), barycentric(0, 127, &second));
        assert_ne!(barycentric(0, 127, &second), barycentric(0, 127, &first));
    }

    #[test]
    fn decorations_lower_their_tile() {
        let (mut m, layout) = world();
        put_tile(&mut m, &layout, 0, 1, 2, TILE);
        let mut terrain = Terrain::read(&m, &layout, key(&m, &layout)).unwrap();
        terrain.set_decorations([((0, 1, 2), 30), ((0, 2, 2), 0)]);
        assert_eq!(terrain.ground_on(&m, &layout, 128, 256, 0, None), -42);
        assert_eq!(terrain.ground_on(&m, &layout, 256, 256, 0, None), -22);
    }

    #[test]
    fn decorations_that_failed_are_asked_again_later() {
        let (mut m, layout) = world();
        put_tile(&mut m, &layout, 0, 1, 2, TILE);
        let mut terrain = Terrain::read(&m, &layout, key(&m, &layout)).unwrap();
        terrain.decorate(|| None);
        terrain.decorate(|| panic!("asked again before the retry"));
        assert_eq!(terrain.ground_on(&m, &layout, 128, 256, 0, None), -12);
        terrain.decorations_failed = terrain.decorations_failed.map(|at| at - DECORATION_RETRY);
        terrain.decorate(|| Some(vec![((0, 1, 2), 30)]));
        assert_eq!(terrain.ground_on(&m, &layout, 128, 256, 0, None), -42);
        terrain.decorate(|| panic!("asked again once decorated"));
    }

    #[test]
    fn edges_count_as_inside() {
        let t: Triangle = [[0, 0, 0], [128, 0, 0], [0, 0, 128]];
        assert!(inside(0, 0, &t));
        assert!(inside(64, 64, &t));
        assert!(inside(64, 0, &t));
        assert!(!inside(65, 64, &t));
        assert!(!inside(-1, 0, &t));
        assert!(!inside(0, 129, &t));
        let reversed: Triangle = [t[0], t[2], t[1]];
        assert!(inside(64, 64, &reversed));
        assert!(!inside(65, 64, &reversed));
    }

    #[test]
    fn projects_like_the_client() {
        let (m, layout) = world();
        let mut terrain = None;
        let mut asked = 0;
        let points = [[0, 0, 0], [100, 0, 0], [0, 200, 0], [128, 256, -88]]
            .map(|[x, y, z]| Point::new(x, y, z));
        let screen = project(&m, &layout, &mut terrain, &points, Viewpoint::default(), false, || {
            asked += 1;
            Some(Vec::new())
        })
        .unwrap();
        assert_eq!(screen[0], [404.0, 302.0]);
        assert_eq!(screen[1], [454.0, 302.0]);
        assert_eq!(screen[2], [404.0, 202.0]);
        let x = 500 * 128 / 900 + 404;
        let y = 2 + 300 - 500 * 256 / 900;
        assert_eq!(
            screen[3],
            [
                ((x as f32 / 2.0 + 0.5) as i32 * 2) as f32,
                ((y as f32 / 2.0 + 0.5) as i32 * 2) as f32
            ]
        );
        project(&m, &layout, &mut terrain, &points, Viewpoint::default(), false, || {
            asked += 1;
            Some(Vec::new())
        })
        .unwrap();
        assert_eq!(
            asked, 1,
            "terrain and decorations are kept while the scene stays"
        );
    }

    #[test]
    fn a_shift_moves_the_camera_with_the_ground_under_its_focal_point() {
        let (mut m, mut layout) = world();
        layout.camera.focal = 0x140;
        for (i, v) in [64, -5, 64].into_iter().enumerate() {
            m.put_i32(CLIENT + 0x140 + i as u64 * 4, v);
        }
        let mut terrain = None;
        let points = [Point::new(300, 200, -40)];
        let shifted = project(
            &m,
            &layout,
            &mut terrain,
            &points,
            Viewpoint {
                shift: Some([128, 0, -1]),
                ..Viewpoint::default()
            },
            false,
            || Some(Vec::new()),
        )
        .unwrap();
        let t = terrain.as_mut().unwrap();
        let rise =
            t.ground_on(&m, &layout, 192, 64, 0, None) - t.ground_on(&m, &layout, 64, 64, 0, None);
        assert_eq!(rise, -10);
        let ground = t.ground_on(&m, &layout, 300, 200, 0, None);
        let mut camera = camera(&m, &layout);
        camera.position = [
            camera.position[0] + 128,
            camera.position[1] + rise,
            camera.position[2],
        ];
        assert_eq!(shifted[0], camera.project(300, 200, ground - 40).0);
        assert_ne!(
            shifted,
            project(&m, &layout, &mut terrain, &points, Viewpoint::default(), false, || Some(
                Vec::new()
            ))
            .unwrap()
        );
    }

    #[test]
    fn behind_the_camera_and_degenerate_values_match_cvttss2si() {
        assert_eq!(truncate(f32::NAN), i32::MIN);
        assert_eq!(truncate(3e9), i32::MIN);
        assert_eq!(truncate(-3e9), i32::MIN);
        assert_eq!(truncate(-2.7), -2);
        let (m, layout) = world();
        let camera = camera(&m, &layout);
        let [x, y] = camera.project(0, 0, -1000).0;
        assert!(x < -1e9 && y < -1e9);
    }

    #[test]
    fn a_yaw_turn_takes_the_short_way_round() {
        let quarter = std::f32::consts::FRAC_PI_2;
        assert!((yaw_turn(0, 4096) - quarter).abs() < 1e-6);
        assert!((yaw_turn(1000, 16000) + 1384.0 * quarter / 4096.0).abs() < 1e-6);
        assert!((yaw_turn(16000, 1000) - 1384.0 * quarter / 4096.0).abs() < 1e-6);
        assert_eq!(yaw_turn(5, 5), 0.0);
    }

    #[test]
    fn zoom_doubles_the_view_scale_every_256() {
        assert_eq!(zoom_scale(300, 556), 2.0);
        assert_eq!(zoom_scale(556, 300), 0.5);
        assert!((zoom_scale(204, 499) - 2.2227).abs() < 1e-3);
    }

    #[test]
    fn a_view_turns_and_zooms_the_camera_about_its_focal_point() {
        let (mut m, mut layout) = world();
        layout.camera.focal = 0x140;
        for (i, v) in [64, -5, 64].into_iter().enumerate() {
            m.put_i32(CLIENT + 0x140 + i as u64 * 4, v);
        }
        let mut terrain = None;
        let points = [Point::new(300, 200, -40)];
        let viewed = project(
            &m,
            &layout,
            &mut terrain,
            &points,
            Viewpoint {
                turn: Some(0.5),
                zoom_scale: Some(0.75),
                ..Viewpoint::default()
            },
            false,
            || Some(Vec::new()),
        )
        .unwrap();
        let ground = terrain
            .as_mut()
            .unwrap()
            .ground_on(&m, &layout, 300, 200, 0, None);
        let camera = camera(&m, &layout).turned([64, 64], 0.5).zoomed(0.75);
        assert_eq!(viewed[0], camera.project(300, 200, ground - 40).0);
    }

    #[test]
    fn needs_the_parts() {
        let (m, mut layout) = world();
        layout.found &= !part::PROJECTION;
        assert_eq!(
            project(
                &m,
                &layout,
                &mut None,
                &[Point::new(0, 0, 0)],
                Viewpoint::default(),
                false,
                || Some(Vec::new())
            ),
            Err("agent didn't find what local projection reads".to_string())
        );
    }
}
