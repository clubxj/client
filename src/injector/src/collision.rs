use decode::definitions::{LocDef, Reader};

pub const REGION_SIZE: i32 = 64;
pub const PLANES: usize = 4;
pub(crate) const TILES: usize = (REGION_SIZE * REGION_SIZE) as usize;
pub(crate) const SQUARES: usize = 1 << 16;

pub mod flag {
    pub const WALL_NORTH_WEST: u32 = 0x1;
    pub const WALL_NORTH: u32 = 0x2;
    pub const WALL_NORTH_EAST: u32 = 0x4;
    pub const WALL_EAST: u32 = 0x8;
    pub const WALL_SOUTH_EAST: u32 = 0x10;
    pub const WALL_SOUTH: u32 = 0x20;
    pub const WALL_SOUTH_WEST: u32 = 0x40;
    pub const WALL_WEST: u32 = 0x80;
    pub const OBJECT: u32 = 0x100;
    pub const PROJECTILE_SHIFT: u32 = 9;
    pub const OBJECT_PROJECTILE: u32 = 0x20000;
    pub const FLOOR_DECORATION: u32 = 0x40000;
    pub const FLOOR: u32 = 0x200000;
    pub const BLOCKED: u32 = OBJECT | FLOOR_DECORATION | FLOOR;
    pub const DOOR_NORTH: u32 = 1 << 24;
    pub const DOOR_EAST: u32 = 1 << 25;
    pub const DOOR_SOUTH: u32 = 1 << 26;
    pub const DOOR_WEST: u32 = 1 << 27;
    pub const DOOR_OBJECT: u32 = 1 << 28;
    pub const DOORS: u32 = 0x1f << 24;
    pub const NO_FLOOR: u32 = 1 << 29;

    const DOOR_WALLS: [(u32, u32); 4] = [
        (DOOR_NORTH, WALL_NORTH),
        (DOOR_EAST, WALL_EAST),
        (DOOR_SOUTH, WALL_SOUTH),
        (DOOR_WEST, WALL_WEST),
    ];

    pub(super) fn doors_of(walls: u32) -> u32 {
        DOOR_WALLS
            .iter()
            .filter(|&&(_, wall)| walls & wall != 0)
            .fold(0, |doors, &(door, _)| doors | door)
    }

    pub fn door_blocks(flags: u32) -> u32 {
        let walls = DOOR_WALLS
            .iter()
            .filter(|&&(door, _)| flags & door != 0)
            .fold(0, |walls, &(_, wall)| walls | wall);
        walls | if flags & DOOR_OBJECT != 0 { OBJECT } else { 0 }
    }
}

const SETTING_BLOCKED: u8 = 1;
const SETTING_BRIDGE: u8 = 2;
const SETTING_HAS_FLOOR: u8 = 0x80;

pub fn turned(size_x: u8, size_y: u8, rotation: u8) -> (u8, u8) {
    if rotation & 1 == 1 {
        (size_y, size_x)
    } else {
        (size_x, size_y)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub id: u32,
    pub x: u8,
    pub y: u8,
    pub plane: u8,
    pub shape: u8,
    pub rotation: u8,
}

pub type Settings = [[[u8; REGION_SIZE as usize]; REGION_SIZE as usize]; PLANES];

pub fn decode_terrain(data: &[u8]) -> Option<Box<Settings>> {
    let mut settings = Box::new([[[0u8; REGION_SIZE as usize]; REGION_SIZE as usize]; PLANES]);
    let mut r = Reader::new(data);
    for plane in settings.iter_mut() {
        for column in plane.iter_mut() {
            for tile in column.iter_mut() {
                loop {
                    match r.u16()? {
                        0 => break,
                        1 => {
                            r.u8()?;
                            break;
                        }
                        2..=49 => {
                            r.skip(2)?;
                            *tile |= SETTING_HAS_FLOOR;
                        }
                        op @ 50..=81 => *tile = *tile & SETTING_HAS_FLOOR | (op - 49) as u8,
                        _ => *tile |= SETTING_HAS_FLOOR,
                    }
                }
            }
        }
    }
    Some(settings)
}

pub fn decode_locs(data: &[u8]) -> Option<Vec<Placement>> {
    let mut r = Reader::new(data);
    let mut locs = Vec::new();
    let mut id: i64 = -1;
    loop {
        let delta = r.uint_smart_compat()?;
        if delta == 0 {
            return Some(locs);
        }
        id += delta as i64;
        let mut position = 0u32;
        loop {
            let delta = r.ushort_smart()?;
            if delta == 0 {
                break;
            }
            position = position.wrapping_add(delta - 1);
            let attributes = r.u8()?;
            locs.push(Placement {
                id: id as u32,
                x: ((position >> 6) & 63) as u8,
                y: (position & 63) as u8,
                plane: ((position >> 12) & 3) as u8,
                shape: attributes >> 2,
                rotation: attributes & 3,
            });
        }
    }
}

pub fn region_id(region_x: i32, region_y: i32) -> u32 {
    ((region_x as u32) << 8) | region_y as u32
}

pub trait Cell: Copy + Default + PartialEq {
    fn with(self, flags: u32) -> Self;
}

impl Cell for u32 {
    fn with(self, flags: u32) -> Self {
        self | flags
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Movement(pub u16);

impl Movement {
    pub const BLOCKS: u16 = 0x100;
    pub const NO_FLOOR: u16 = 0x4000;
    const DOOR_SHIFT: u32 = 24 - 9;

    pub fn opened(self) -> Movement {
        let doors = (self.0 as u32) << Self::DOOR_SHIFT & flag::DOORS;
        let blocks = flag::door_blocks(doors);
        let object = if blocks & flag::OBJECT != 0 {
            Self::BLOCKS
        } else {
            0
        };
        Movement(self.0 & !(blocks as u16 & 0xff) & !object)
    }
}

impl Cell for Movement {
    fn with(self, flags: u32) -> Self {
        let bit = |set: bool, bit: u16| if set { bit } else { 0 };
        Movement(
            self.0
                | (flags & 0xff) as u16
                | ((flags & flag::DOORS) >> Self::DOOR_SHIFT) as u16
                | bit(flags & flag::BLOCKED != 0, Self::BLOCKS)
                | bit(flags & flag::NO_FLOOR != 0, Self::NO_FLOOR),
        )
    }
}

#[derive(Clone)]
pub enum Square<T> {
    Missing,
    Open,
    Same(T),
    Tiles(Box<[T; TILES]>),
}

impl<T: Cell> Square<T> {
    pub fn tile(&self, index: usize) -> Option<T> {
        match self {
            Square::Missing => None,
            Square::Open => Some(T::default()),
            Square::Same(value) => Some(*value),
            Square::Tiles(tiles) => Some(tiles[index]),
        }
    }
}

pub struct WorldCollision<T: Cell = u32> {
    squares: Vec<Square<T>>,
}

impl<T: Cell> Default for WorldCollision<T> {
    fn default() -> Self {
        Self {
            squares: (0..PLANES * SQUARES).map(|_| Square::Missing).collect(),
        }
    }
}

pub(crate) fn plane_index(plane: i32) -> Option<usize> {
    usize::try_from(plane).ok().filter(|&p| p < PLANES)
}

pub(crate) fn square_index(x: i32, y: i32) -> Option<(usize, usize)> {
    if !(0..REGION_SIZE * 256).contains(&x) || !(0..REGION_SIZE * 256).contains(&y) {
        return None;
    }
    Some((
        region_id(x / REGION_SIZE, y / REGION_SIZE) as usize,
        ((x % REGION_SIZE) * REGION_SIZE + y % REGION_SIZE) as usize,
    ))
}

fn same<T: Cell>(value: T) -> Square<T> {
    if value == T::default() {
        Square::Open
    } else {
        Square::Same(value)
    }
}

impl<T: Cell> WorldCollision<T> {
    pub fn flags(&self, plane: i32, x: i32, y: i32) -> Option<T> {
        let plane = plane_index(plane)?;
        let (square, tile) = square_index(x, y)?;
        self.square(plane, square).tile(tile)
    }

    pub fn square(&self, plane: usize, region: usize) -> &Square<T> {
        &self.squares[plane * SQUARES + region]
    }

    pub fn add_square(&mut self, region: u32) {
        for squares in self.squares.as_chunks_mut::<SQUARES>().0 {
            if let Some(square @ Square::Missing) = squares.get_mut(region as usize) {
                *square = Square::Open;
            }
        }
    }

    pub fn remove_square(&mut self, region: u32) {
        for squares in self.squares.as_chunks_mut::<SQUARES>().0 {
            if let Some(square) = squares.get_mut(region as usize) {
                *square = Square::Missing;
            }
        }
    }

    pub(crate) fn add(&mut self, plane: i32, x: i32, y: i32, flags: u32) {
        let (Some(plane), Some((square, tile))) = (plane_index(plane), square_index(x, y)) else {
            return;
        };
        let square = &mut self.squares[plane * SQUARES + square];
        match square {
            Square::Open => *square = Square::Tiles(Box::new([T::default(); TILES])),
            Square::Same(value) => *square = Square::Tiles(Box::new([*value; TILES])),
            _ => {}
        }
        if let Square::Tiles(tiles) = square {
            tiles[tile] = tiles[tile].with(flags);
        }
    }

    pub fn shrink(&mut self) {
        for square in &mut self.squares {
            if let Square::Tiles(tiles) = square
                && tiles.iter().all(|t| *t == tiles[0])
            {
                *square = same(tiles[0]);
            }
        }
    }

    pub fn add_terrain_and_locs(
        &mut self,
        region: u32,
        settings: &Settings,
        locs: &[Placement],
        defs: &impl Fn(u32) -> Option<LocDef>,
    ) {
        let base_x = (region >> 8) as i32 * REGION_SIZE;
        let base_y = (region & 0xff) as i32 * REGION_SIZE;
        let bridge = |x: usize, y: usize| settings[1][x][y] & SETTING_BRIDGE != 0;
        for (plane, columns) in settings.iter().enumerate() {
            for (x, column) in columns.iter().enumerate() {
                for (y, &setting) in column.iter().enumerate() {
                    let (wx, wy) = (base_x + x as i32, base_y + y as i32);
                    let bridged = bridge(x, y);
                    if setting & SETTING_BLOCKED != 0 {
                        self.add(plane as i32 - bridged as i32, wx, wy, flag::FLOOR);
                    }
                    let floored = settings
                        .get(plane + bridged as usize)
                        .is_some_and(|p| p[x][y] & SETTING_HAS_FLOOR != 0);
                    if plane > 0 && !floored {
                        self.add(plane as i32, wx, wy, flag::NO_FLOOR);
                    }
                }
            }
        }
        for loc in locs {
            let walked = loc.plane as i32 - bridge(loc.x as usize, loc.y as usize) as i32;
            if walked < 0 {
                continue;
            }
            let Some(def) = defs(loc.id) else { continue };
            self.add_loc(
                walked,
                base_x + loc.x as i32,
                base_y + loc.y as i32,
                loc.shape,
                loc.rotation,
                &def,
            );
        }
    }

    fn add_loc(&mut self, plane: i32, x: i32, y: i32, shape: u8, rotation: u8, def: &LocDef) {
        match shape {
            22 => {
                if def.interact_type == 1 {
                    self.add(plane, x, y, flag::FLOOR_DECORATION);
                }
            }
            0..=3 if def.interact_type != 0 => self.add_wall(plane, x, y, shape, rotation, def),
            9..=21 if def.interact_type != 0 => {
                let (w, h) = turned(def.size_x, def.size_y, rotation);
                let door = if shape == 9 && def.door {
                    flag::DOOR_OBJECT
                } else {
                    0
                };
                let projectile = if def.blocks_projectile {
                    flag::OBJECT_PROJECTILE
                } else {
                    0
                };
                for dx in 0..w as i32 {
                    for dy in 0..h as i32 {
                        self.add(plane, x + dx, y + dy, flag::OBJECT | door | projectile);
                    }
                }
            }
            _ => {}
        }
    }

    fn add_wall(&mut self, plane: i32, x: i32, y: i32, shape: u8, rotation: u8, def: &LocDef) {
        for &(dx, dy, wall) in wall_marks(shape, rotation) {
            let doors = if def.door { flag::doors_of(wall) } else { 0 };
            let projectile = if def.blocks_projectile {
                wall << flag::PROJECTILE_SHIFT
            } else {
                0
            };
            self.add(plane, x + dx, y + dy, wall | projectile | doors);
        }
    }
}

pub fn wall_marks(shape: u8, rotation: u8) -> &'static [(i32, i32, u32)] {
    use flag::*;
    match (shape, rotation & 3) {
        (0, 0) => &[(0, 0, WALL_WEST), (-1, 0, WALL_EAST)],
        (0, 1) => &[(0, 0, WALL_NORTH), (0, 1, WALL_SOUTH)],
        (0, 2) => &[(0, 0, WALL_EAST), (1, 0, WALL_WEST)],
        (0, _) => &[(0, 0, WALL_SOUTH), (0, -1, WALL_NORTH)],
        (1 | 3, 0) => &[(0, 0, WALL_NORTH_WEST), (-1, 1, WALL_SOUTH_EAST)],
        (1 | 3, 1) => &[(0, 0, WALL_NORTH_EAST), (1, 1, WALL_SOUTH_WEST)],
        (1 | 3, 2) => &[(0, 0, WALL_SOUTH_EAST), (1, -1, WALL_NORTH_WEST)],
        (1 | 3, _) => &[(0, 0, WALL_SOUTH_WEST), (-1, -1, WALL_NORTH_EAST)],
        (2, 0) => &[
            (0, 0, WALL_WEST | WALL_NORTH),
            (-1, 0, WALL_EAST),
            (0, 1, WALL_SOUTH),
        ],
        (2, 1) => &[
            (0, 0, WALL_NORTH | WALL_EAST),
            (0, 1, WALL_SOUTH),
            (1, 0, WALL_WEST),
        ],
        (2, 2) => &[
            (0, 0, WALL_EAST | WALL_SOUTH),
            (1, 0, WALL_WEST),
            (0, -1, WALL_NORTH),
        ],
        (2, _) => &[
            (0, 0, WALL_SOUTH | WALL_WEST),
            (0, -1, WALL_NORTH),
            (-1, 0, WALL_EAST),
        ],
        _ => &[],
    }
}

const MAGIC: &[u8; 8] = b"BOTMOV01";
const OPEN: u8 = 1;
const TILES_RLE: u8 = 2;
const END: u8 = 0xff;

impl WorldCollision<Movement> {
    pub fn to_bytes(&self, key: u64) -> Vec<u8> {
        let push_run = |out: &mut Vec<u8>, run: usize, value: Movement| {
            out.extend_from_slice(&(run as u16).to_le_bytes());
            out.extend_from_slice(&value.0.to_le_bytes());
        };
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&key.to_le_bytes());
        for (plane, squares) in self.squares.as_chunks::<SQUARES>().0.iter().enumerate() {
            for (region, square) in squares.iter().enumerate() {
                let kind = match square {
                    Square::Missing => continue,
                    Square::Open => OPEN,
                    Square::Same(_) | Square::Tiles(_) => TILES_RLE,
                };
                out.push(plane as u8);
                out.extend_from_slice(&(region as u16).to_le_bytes());
                out.push(kind);
                match square {
                    Square::Same(value) => push_run(&mut out, TILES, *value),
                    Square::Tiles(tiles) => {
                        for run in tiles.chunk_by(|a, b| a == b) {
                            push_run(&mut out, run.len(), run[0]);
                        }
                    }
                    Square::Missing | Square::Open => {}
                }
            }
        }
        out.push(END);
        out
    }

    pub fn from_bytes(bytes: &[u8], key: u64) -> Option<Self> {
        if bytes.get(..8)? != MAGIC || bytes.get(8..16)? != key.to_le_bytes() {
            return None;
        }
        let mut r = Reader::new(bytes.get(16..)?);
        let mut world = Self::default();
        loop {
            let plane = r.u8()?;
            if plane == END {
                return Some(world);
            }
            let region = r.u16_le()? as usize;
            let slot = world
                .squares
                .as_chunks_mut::<SQUARES>()
                .0
                .get_mut(plane as usize)?
                .get_mut(region)?;
            *slot = match r.u8()? {
                OPEN => Square::Open,
                TILES_RLE => {
                    let run = r.u16_le()? as usize;
                    let value = Movement(r.u16_le()?);
                    if run == TILES {
                        same(value)
                    } else {
                        let mut tiles = Box::new([Movement::default(); TILES]);
                        tiles.get_mut(..run)?.fill(value);
                        let mut at = run.max(1);
                        while at < TILES {
                            let run = r.u16_le()? as usize;
                            let value = Movement(r.u16_le()?);
                            tiles.get_mut(at..at + run)?.fill(value);
                            at += run.max(1);
                        }
                        Square::Tiles(tiles)
                    }
                }
                _ => return None,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TileOps<'a> = ((usize, usize, usize), &'a [u16]);

    fn terrain_with(tiles: &[TileOps]) -> Vec<u8> {
        let mut out = Vec::new();
        for plane in 0..PLANES {
            for x in 0..64 {
                for y in 0..64 {
                    if let Some((_, ops)) = tiles.iter().find(|(at, _)| *at == (plane, x, y)) {
                        for op in *ops {
                            out.extend_from_slice(&op.to_be_bytes());
                        }
                    }
                    out.extend_from_slice(&0u16.to_be_bytes());
                }
            }
        }
        out.push(0);
        out
    }

    #[test]
    fn decodes_terrain_settings() {
        let data = terrain_with(&[((0, 3, 4), &[2, 0x0005, 50]), ((1, 3, 4), &[51])]);
        let settings = decode_terrain(&data).unwrap();
        assert_eq!(settings[0][3][4], SETTING_BLOCKED | SETTING_HAS_FLOOR);
        assert_eq!(settings[1][3][4], SETTING_BRIDGE);
        assert_eq!(settings[0][3][5], 0);
    }

    #[test]
    fn decodes_locs() {
        let position = (1 << 12) | (2 << 6) | 3;
        let mut data = vec![6];
        data.extend_from_slice(&((position + 1) as u16 | 0x8000).to_be_bytes());
        data.push((10 << 2) | 1);
        data.push(0);
        data.push(3);
        data.push(1);
        data.push(2);
        data.push(0);
        data.push(0);
        let locs = decode_locs(&data).unwrap();
        assert_eq!(
            locs,
            vec![
                Placement {
                    id: 5,
                    x: 2,
                    y: 3,
                    plane: 1,
                    shape: 10,
                    rotation: 1
                },
                Placement {
                    id: 8,
                    x: 0,
                    y: 0,
                    plane: 0,
                    shape: 0,
                    rotation: 2
                },
            ]
        );
    }

    #[test]
    fn marks_doors_and_opens_them_for_paths() {
        let region = region_id(50, 50);
        let (bx, by) = (50 * 64, 50 * 64);
        let settings = Box::new([[[0u8; 64]; 64]; PLANES]);
        let locs = [
            Placement {
                id: 1,
                x: 5,
                y: 5,
                plane: 0,
                shape: 0,
                rotation: 2,
            },
            Placement {
                id: 2,
                x: 5,
                y: 6,
                plane: 0,
                shape: 0,
                rotation: 2,
            },
        ];
        let defs = |id: u32| {
            Some(LocDef {
                door: id == 1,
                ..LocDef::default()
            })
        };
        let mut world = WorldCollision::<Movement>::default();
        world.add_square(region);
        world.add_terrain_and_locs(region, &settings, &locs, &defs);
        let door = world.flags(0, bx + 5, by + 5).unwrap();
        assert_eq!(door.opened().0 & flag::WALL_EAST as u16, 0);
        assert_ne!(door.0 & flag::WALL_EAST as u16, 0);
        let wall = world.flags(0, bx + 5, by + 6).unwrap();
        assert_eq!(wall.opened(), wall);
        let mut u32_world = WorldCollision::<u32>::default();
        u32_world.add_square(region);
        u32_world.add_terrain_and_locs(region, &settings, &locs, &defs);
        let flags = u32_world.flags(0, bx + 6, by + 5).unwrap();
        assert_eq!(flag::door_blocks(flags), flag::WALL_WEST);
    }

    #[test]
    fn places_walls_footprints_and_bridges() {
        let region = region_id(50, 50);
        let (bx, by) = (50 * 64, 50 * 64);
        let mut settings = Box::new([[[0u8; 64]; 64]; PLANES]);
        settings[1][10][10] = SETTING_BRIDGE;
        let locs = [
            Placement {
                id: 1,
                x: 5,
                y: 5,
                plane: 0,
                shape: 0,
                rotation: 1,
            },
            Placement {
                id: 2,
                x: 20,
                y: 20,
                plane: 0,
                shape: 10,
                rotation: 1,
            },
            Placement {
                id: 3,
                x: 10,
                y: 10,
                plane: 1,
                shape: 10,
                rotation: 0,
            },
        ];
        let defs = |id: u32| match id {
            2 => Some(LocDef {
                size_x: 2,
                size_y: 1,
                interact_type: 2,
                blocks_projectile: false,
                ..LocDef::default()
            }),
            _ => Some(LocDef::default()),
        };
        let mut world = WorldCollision::<u32>::default();
        world.add_square(region);
        world.add_terrain_and_locs(region, &settings, &locs, &defs);
        assert_eq!(
            world.flags(0, bx + 5, by + 5),
            Some(flag::WALL_NORTH | flag::WALL_NORTH << flag::PROJECTILE_SHIFT)
        );
        assert_eq!(
            world.flags(0, bx + 5, by + 6),
            Some(flag::WALL_SOUTH | flag::WALL_SOUTH << flag::PROJECTILE_SHIFT)
        );
        assert_eq!(world.flags(0, bx + 20, by + 20), Some(flag::OBJECT));
        assert_eq!(world.flags(0, bx + 20, by + 21), Some(flag::OBJECT));
        assert_eq!(world.flags(0, bx + 21, by + 20), Some(0));
        assert_eq!(
            world.flags(0, bx + 10, by + 10),
            Some(flag::OBJECT | flag::OBJECT_PROJECTILE)
        );
        assert_eq!(world.flags(1, bx + 10, by + 10), Some(flag::NO_FLOOR));
        assert_eq!(world.flags(0, bx - 1, by), None);
    }

    #[test]
    fn movement_round_trips_through_bytes() {
        let region = region_id(50, 50);
        let mut world = WorldCollision::<Movement>::default();
        world.add_square(region);
        world.add_square(region_id(51, 50));
        world.add(0, 50 * 64 + 3, 50 * 64 + 4, flag::FLOOR | flag::WALL_EAST);
        world.shrink();
        let bytes = world.to_bytes(7);
        assert!(WorldCollision::from_bytes(&bytes, 8).is_none());
        let back = WorldCollision::from_bytes(&bytes, 7).unwrap();
        assert_eq!(
            back.flags(0, 50 * 64 + 3, 50 * 64 + 4),
            Some(Movement(Movement::BLOCKS | flag::WALL_EAST as u16))
        );
        assert_eq!(back.flags(0, 51 * 64, 50 * 64), Some(Movement(0)));
        assert_eq!(back.flags(0, 52 * 64, 50 * 64), None);
    }

    #[test]
    fn a_square_of_one_value_is_kept_as_that_value() {
        let region = region_id(50, 50);
        let mut world = WorldCollision::<Movement>::default();
        world.add_square(region);
        for x in 0..64 {
            for y in 0..64 {
                world.add(1, 50 * 64 + x, 50 * 64 + y, flag::NO_FLOOR);
            }
        }
        world.shrink();
        let no_floor = Movement::default().with(flag::NO_FLOOR);
        assert!(matches!(world.square(1, region as usize), Square::Same(v) if *v == no_floor));
        assert_eq!(world.flags(1, 50 * 64 + 9, 50 * 64 + 9), Some(no_floor));

        let back = WorldCollision::from_bytes(&world.to_bytes(7), 7).unwrap();
        assert!(matches!(back.square(1, region as usize), Square::Same(v) if *v == no_floor));
        assert!(matches!(back.square(0, region as usize), Square::Open));

        world.add(1, 50 * 64, 50 * 64, flag::WALL_EAST);
        assert!(matches!(world.square(1, region as usize), Square::Tiles(_)));
        assert_eq!(world.flags(1, 50 * 64 + 9, 50 * 64 + 9), Some(no_floor));
    }
}
