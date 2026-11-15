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
