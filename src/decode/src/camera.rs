use ipc::{
    read::{self, Memory},
    state::{Layout, part},
};

pub const NEAR_PLANE: f32 = 50.0;

pub fn truncate(value: f32) -> i32 {
    if value.is_nan() || value >= 2_147_483_648.0 || value < -2_147_483_648.0 {
        i32::MIN
    } else {
        value as i32
    }
}

const WINDOW_SIZE: usize = 512;

pub struct Window<'a, M: Memory> {
    memory: &'a M,
    base: u64,
    low: u64,
    high: u64,
    bytes: [u8; WINDOW_SIZE],
    filled: bool,
}

impl<'a, M: Memory> Window<'a, M> {
    pub fn new(memory: &'a M, base: u64, fields: &[(u64, u64)]) -> Self {
        let low = fields.iter().map(|f| f.0).min().unwrap_or(0);
        let high = fields.iter().map(|f| f.0 + f.1).max().unwrap_or(0);
        let mut window = Self {
            memory,
            base,
            low,
            high,
            bytes: [0; WINDOW_SIZE],
            filled: false,
        };
        if high > low && ((high - low) as usize) <= WINDOW_SIZE {
            window.filled = memory.read(base + low, &mut window.bytes[..(high - low) as usize]);
        }
        window
    }

    pub fn get<const N: usize, const K: usize>(&self, offset: u64) -> Option<[[u8; N]; K]> {
        let mut bytes = [[0; N]; K];
        let flat = bytes.as_flattened_mut();
        let end = offset + flat.len() as u64;
        let read = if self.filled && offset >= self.low && end <= self.high {
            let at = (offset - self.low) as usize;
            flat.copy_from_slice(&self.bytes[at..at + flat.len()]);
            true
        } else {
            self.memory.read(self.base + offset, flat)
        };
        read.then_some(bytes)
    }

    pub fn i32(&self, offset: u64) -> Option<i32> {
        self.i32s::<1>(offset).map(|[v]| v)
    }

    pub fn f32(&self, offset: u64) -> Option<f32> {
        self.get::<4, 1>(offset).map(|[v]| f32::from_le_bytes(v))
    }

    pub fn ptr(&self, offset: u64) -> Option<u64> {
        self.get::<8, 1>(offset)
            .map(|[v]| u64::from_le_bytes(v))
            .filter(|&p| p != 0)
    }

    pub fn i32s<const N: usize>(&self, offset: u64) -> Option<[i32; N]> {
        self.get::<4, N>(offset)
            .map(|values| values.map(i32::from_le_bytes))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub position: [i32; 3],
    pub trig: [f32; 4],
    pub zoom: i32,
    pub center: [i32; 2],
    pub offset: [i32; 2],
    pub scale: f32,
    pub ratio: [f32; 2],
}

impl Camera {
    pub fn read(
        memory: &impl Memory,
        layout: &Layout,
        client: u64,
        trig: [f32; 4],
    ) -> Option<Self> {
        let p = &layout.projection;
        let canvas = memory.ptr(client + layout.interfaces.canvas as u64)?;
        let view = memory.ptr(canvas + p.view_holder as u64 + p.view as u64)?;
        let camera = Window::new(
            memory,
            client,
            &[(layout.camera.position as u64, 12), (p.zoom as u64, 4)],
        );
        let view_fields = Window::new(
            memory,
            view,
            &[
                (p.view_center as u64, 8),
                (p.view_offset_x as u64, 4),
                (p.view_offset_y as u64, 4),
            ],
        );
        let canvas_fields = Window::new(
            memory,
            canvas,
            &[
                (p.canvas_size as u64, 8),
                (p.ui_size as u64, 8),
                (layout.interfaces.ui_scale as u64, 4),
            ],
        );
        let canvas_size = canvas_fields.i32s::<2>(p.canvas_size as u64)?;
        let ui = canvas_fields.i32s::<2>(p.ui_size as u64)?;
        Some(Self {
            position: camera.i32s(layout.camera.position as u64)?,
            trig,
            zoom: camera.i32(p.zoom as u64)?,
            center: view_fields.i32s(p.view_center as u64)?,
            offset: [
                view_fields.i32(p.view_offset_x as u64)?,
                view_fields.i32(p.view_offset_y as u64)?,
            ],
            scale: canvas_fields.f32(layout.interfaces.ui_scale as u64)?,
            ratio: [
                ui[0] as f32 / canvas_size[0] as f32,
                ui[1] as f32 / canvas_size[1] as f32,
            ],
        })
    }

    pub fn current(memory: &impl Memory, layout: &Layout) -> Option<Self> {
        if !layout.has(part::PROJECTION | part::CAMERA | part::INTERFACES) {
            return None;
        }
        let p = &layout.projection;
        let client = read::client(memory, layout)?;
        let world_view = memory.ptr(client + p.top_world_view as u64)?;
        let scene = memory.ptr(world_view + p.scene as u64)?;
        let mut trig = [[0u8; 4]; 4];
        if !memory.read(scene + p.trig as u64, trig.as_flattened_mut()) {
            return None;
        }
        Self::read(memory, layout, client, trig.map(f32::from_le_bytes))
    }

    pub fn turned(mut self, focal: [i32; 2], radians: f32) -> Self {
        let [_, _, sin_yaw, cos_yaw] = self.trig;
        let (sin, cos) = radians.sin_cos();
        let dx = self.position[0].wrapping_sub(focal[0]) as f32;
        let dy = self.position[2].wrapping_sub(focal[1]) as f32;
        self.position[0] = focal[0].wrapping_add((dx * cos - dy * sin).round() as i32);
        self.position[2] = focal[1].wrapping_add((dx * sin + dy * cos).round() as i32);
        self.trig[2] = sin_yaw * cos + cos_yaw * sin;
        self.trig[3] = cos_yaw * cos - sin_yaw * sin;
        self
    }

    pub fn zoomed(mut self, scale: f32) -> Self {
        self.zoom = (self.zoom as f32 * scale).round() as i32;
        self
    }

    pub fn project(&self, x: i32, y: i32, height: i32) -> ([f32; 2], f32) {
        self.project_relative([
            x.wrapping_sub(self.position[0]),
            height.wrapping_sub(self.position[1]),
            y.wrapping_sub(self.position[2]),
        ])
    }

    pub fn project_relative(&self, [dx, dz, dy]: [i32; 3]) -> ([f32; 2], f32) {
        let [sin_pitch, cos_pitch, sin_yaw, cos_yaw] = self.trig;
        let (dx, dz, dy) = (dx as f32, dz as f32, dy as f32);
        let depth = cos_yaw * dy - sin_yaw * dx;
        let across = cos_yaw * dx + sin_yaw * dy;
        let up = cos_pitch * dz - sin_pitch * depth;
        let distance = sin_pitch * dz + cos_pitch * depth;
        let zoom = self.zoom as f32;
        let sx = truncate(zoom * across / distance)
            .wrapping_add(self.offset[0])
            .wrapping_add(self.center[0]);
        let sy = truncate(zoom * up / distance)
            .wrapping_add(self.offset[1])
            .wrapping_add(self.center[1]);
        (
            [self.canvas_units(sx, 0), self.canvas_units(sy, 1)],
            distance,
        )
    }

    fn canvas_units(&self, value: i32, axis: usize) -> f32 {
        let units = self.ratio[axis] * value as f32;
        truncate(if units < 0.0 {
            units - 0.5
        } else {
            units + 0.5
        }) as f32
            * self.scale
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOCAL: [i32; 2] = [6400, 6400];
    const GROUND: i32 = -800;

    fn orbiting(yaw: f32) -> Camera {
        let pitch = 1.17_f32;
        let reach = 2250.0;
        let across = reach * pitch.cos();
        Camera {
            position: [
                FOCAL[0] + (across * yaw.sin()).round() as i32,
                GROUND - (reach * pitch.sin()).round() as i32,
                FOCAL[1] - (across * yaw.cos()).round() as i32,
            ],
            trig: [pitch.sin(), pitch.cos(), yaw.sin(), yaw.cos()],
            zoom: 600,
            center: [400, 300],
            offset: [4, 2],
            scale: 1.0,
            ratio: [1.0, 1.0],
        }
    }

    fn near(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() <= 2.0 && (a[1] - b[1]).abs() <= 2.0
    }

    #[test]
    fn a_turn_matches_the_camera_orbiting_to_that_yaw() {
        let turned = orbiting(0.4).turned(FOCAL, 1.3);
        let there = orbiting(1.7);
        for [x, y, h] in [[6400, 6400, GROUND], [6900, 6100, GROUND - 150], [5800, 7000, GROUND]] {
            assert!(near(turned.project(x, y, h).0, there.project(x, y, h).0));
        }
    }

    #[test]
    fn the_focal_point_stays_on_the_centre_line_after_a_turn() {
        let camera = orbiting(2.9).turned(FOCAL, -2.2);
        let [x, _] = camera.project(FOCAL[0], FOCAL[1], GROUND).0;
        assert!((x - 404.0).abs() <= 1.0);
    }

    #[test]
    fn zoom_scales_around_the_view_centre() {
        let camera = orbiting(0.8);
        let [x, y] = camera.project(6900, 6100, GROUND).0;
        let [zx, zy] = camera.zoomed(0.5).project(6900, 6100, GROUND).0;
        assert!((zx - (404.0 + (x - 404.0) / 2.0)).abs() <= 1.0);
        assert!((zy - (302.0 + (y - 302.0) / 2.0)).abs() <= 1.0);
    }
}
