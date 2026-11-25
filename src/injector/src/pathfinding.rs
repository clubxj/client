use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, VecDeque, hash_map::Entry},
    hash::{BuildHasherDefault, Hasher},
};

use crate::collision::{Movement, WorldCollision, flag};

const BLOCKS: u16 = Movement::BLOCKS;
const STRAIGHT: u32 = 10;
const DIAGONAL: u32 = 11;
const DOOR_COST: u32 = 30;

pub(crate) const DIRECTIONS: [(i32, i32); 8] = [
    (-1, 0),
    (1, 0),
    (0, -1),
    (0, 1),
    (-1, -1),
    (1, -1),
    (-1, 1),
    (1, 1),
];

pub(crate) fn step_cost(dx: i32, dy: i32) -> u32 {
    if dx != 0 && dy != 0 {
        DIAGONAL
    } else {
        STRAIGHT
    }
}

#[derive(Default)]
struct TileHasher(u64);

impl Hasher for TileHasher {
    fn finish(&self) -> u64 {
        let mut z = self.0;
        z ^= z >> 33;
        z = z.wrapping_mul(0xff51_afd7_ed55_8ccd);
        z ^ (z >> 33)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u32(b as u32);
        }
    }

    fn write_u32(&mut self, i: u32) {
        self.0 = (self.0.rotate_left(5) ^ i as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

type TileMap<V> = HashMap<u32, V, BuildHasherDefault<TileHasher>>;

pub(crate) fn straight_blocks(dx: i32, dy: i32) -> u16 {
    let wall = match (dx, dy) {
        (dx, 0) if dx < 0 => flag::WALL_EAST,
        (_, 0) => flag::WALL_WEST,
        (_, dy) if dy < 0 => flag::WALL_NORTH,
        _ => flag::WALL_SOUTH,
    };
    BLOCKS | wall as u16
}

pub type Tile = (i32, i32);

fn walkable(
    world: &WorldCollision<Movement>,
    plane: i32,
    x: i32,
    y: i32,
    mask: u16,
    doors: bool,
) -> bool {
    world
        .flags(plane, x, y)
        .is_some_and(|m| if doors { m.opened() } else { m }.0 & mask == 0)
}

pub fn can_step(
    world: &WorldCollision<Movement>,
    plane: i32,
    x: i32,
    y: i32,
    dx: i32,
    dy: i32,
) -> bool {
    can_step_with(world, plane, x, y, dx, dy, false)
}

fn can_step_with(
    world: &WorldCollision<Movement>,
    plane: i32,
    x: i32,
    y: i32,
    dx: i32,
    dy: i32,
    doors: bool,
) -> bool {
    use flag::*;
    let walkable = |x: i32, y: i32, mask: u16| walkable(world, plane, x, y, mask, doors);
    let straight_x = |dx: i32| walkable(x + dx, y, straight_blocks(dx, 0));
    let straight_y = |dy: i32| walkable(x, y + dy, straight_blocks(0, dy));
    match (dx, dy) {
        (0, 0) => true,
        (dx, 0) => straight_x(dx),
        (0, dy) => straight_y(dy),
        (dx, dy) => {
            let corner = match (dx < 0, dy < 0) {
                (true, true) => WALL_NORTH | WALL_NORTH_EAST | WALL_EAST,
                (false, true) => WALL_NORTH | WALL_NORTH_WEST | WALL_WEST,
                (true, false) => WALL_SOUTH | WALL_SOUTH_EAST | WALL_EAST,
                (false, false) => WALL_SOUTH | WALL_SOUTH_WEST | WALL_WEST,
            };
            walkable(x + dx, y + dy, BLOCKS | corner as u16) && straight_x(dx) && straight_y(dy)
        }
    }
}

fn key((x, y): Tile) -> u32 {
    ((x as u32) << 16) | (y as u32 & 0xffff)
}

fn tile(key: u32) -> Tile {
    ((key >> 16) as i32, (key & 0xffff) as i32)
}

fn estimate((x, y): Tile, (tx, ty): Tile) -> u32 {
    let (dx, dy) = ((x - tx).unsigned_abs(), (y - ty).unsigned_abs());
    STRAIGHT * dx.max(dy) + (DIAGONAL - STRAIGHT) * dx.min(dy)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    Blocked(Tile),
    NoPath,
    TooFar(usize),
}

pub fn find_path(
    world: &WorldCollision<Movement>,
    plane: i32,
    from: Tile,
    to: Tile,
    max_tiles: usize,
) -> Result<Vec<Tile>, PathError> {
    find_path_with(
        world,
        plane,
        from,
        to,
        max_tiles,
        false,
        &Default::default(),
    )
}

pub fn find_path_with(
    world: &WorldCollision<Movement>,
    plane: i32,
    from: Tile,
    to: Tile,
    max_tiles: usize,
    doors: bool,
    closed: &crate::nav::Closed,
) -> Result<Vec<Tile>, PathError> {
    for t in [from, to] {
        if !walkable(world, plane, t.0, t.1, BLOCKS, false) {
            return Err(PathError::Blocked(t));
        }
    }
    let (start, goal) = (key(from), key(to));
    let mut seen: TileMap<(u32, u32)> = TileMap::default();
    let mut open = BinaryHeap::new();
    seen.insert(start, (0, start));
    open.push((Reverse(estimate(from, to)), Reverse(0u32), start));
    let mut expanded = 0;
    while let Some((_, Reverse(cost), at)) = open.pop() {
        if at == goal {
            let mut path = vec![tile(at)];
            let mut step = at;
            while step != start {
                step = seen[&step].1;
                path.push(tile(step));
            }
            path.reverse();
            return Ok(path);
        }
        if seen.get(&at).is_some_and(|&(best, _)| best < cost) {
            continue;
        }
        expanded += 1;
        if expanded > max_tiles {
            return Err(PathError::TooFar(max_tiles));
        }
        let (x, y) = tile(at);
        for (dx, dy) in DIRECTIONS {
            let door = if can_step(world, plane, x, y, dx, dy) {
                0
            } else if doors
                && can_step_with(world, plane, x, y, dx, dy, true)
                && !crate::nav::step_closed(closed, (x, y, plane), dx, dy)
            {
                DOOR_COST
            } else {
                continue;
            };
            let next = key((x + dx, y + dy));
            let cost = cost + door + step_cost(dx, dy);
            match seen.entry(next) {
                Entry::Occupied(mut e) if e.get().0 > cost => {
                    e.insert((cost, at));
                }
                Entry::Vacant(e) => {
                    e.insert((cost, at));
                }
                _ => continue,
            }
            open.push((
                Reverse(cost + estimate((x + dx, y + dy), to)),
                Reverse(cost),
                next,
            ));
        }
    }
    Err(PathError::NoPath)
}

mod route_mask {
    pub const WEST: u32 = 0x4024_0108;
    pub const EAST: u32 = 0x4024_0180;
    pub const SOUTH: u32 = 0x4024_0102;
    pub const NORTH: u32 = 0x4024_0120;
    pub const SOUTH_WEST: u32 = 0x4024_010e;
    pub const SOUTH_EAST: u32 = 0x4024_0183;
    pub const NORTH_WEST: u32 = 0x4024_0138;
    pub const NORTH_EAST: u32 = 0x4024_01e0;
    pub const STAND: u32 = 0x4024_0100;
}

pub fn step_open(
    flags: &impl Fn(i32, i32) -> Option<u32>,
    x: i32,
    y: i32,
    dx: i32,
    dy: i32,
) -> bool {
    use route_mask::*;
    let free = |x: i32, y: i32, mask: u32| flags(x, y).is_some_and(|f| f & mask == 0);
    let side = if dx < 0 { WEST } else { EAST };
    let end = if dy < 0 { SOUTH } else { NORTH };
    match (dx, dy) {
        (0, 0) => false,
        (_, 0) => free(x + dx, y, side),
        (0, _) => free(x, y + dy, end),
        _ => {
            let diagonal = match (dx < 0, dy < 0) {
                (true, true) => SOUTH_WEST,
                (false, true) => SOUTH_EAST,
                (true, false) => NORTH_WEST,
                (false, false) => NORTH_EAST,
            };
            free(x + dx, y + dy, diagonal) && free(x + dx, y, side) && free(x, y + dy, end)
        }
    }
}

const GRAPH: i32 = 128;
const ALTERNATIVE_RADIUS: i32 = 10;
const ALTERNATIVE_MAX_STEPS: u32 = 100;
const START: u8 = 99;
const BACK_NORTH: u8 = 1;
const BACK_EAST: u8 = 2;
const BACK_SOUTH: u8 = 4;
const BACK_WEST: u8 = 8;
const BACK: [u8; 8] = [
    BACK_EAST,
    BACK_WEST,
    BACK_NORTH,
    BACK_SOUTH,
    BACK_EAST | BACK_NORTH,
    BACK_WEST | BACK_NORTH,
    BACK_EAST | BACK_SOUTH,
    BACK_WEST | BACK_SOUTH,
];

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Route {
    pub tiles: Vec<Tile>,
    pub reached: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Tile,
    Beside,
    Rectangle,
    Wall { shape: u8, rotation: u8 },
    WallDecoration { shape: u8, rotation: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub length: i32,
    pub reach: Reach,
}

impl Target {
    pub fn tile((x, y): Tile) -> Self {
        Target {
            x,
            y,
            width: 1,
            length: 1,
            reach: Reach::Tile,
        }
    }

    pub fn npc((x, y): Tile, size: i32) -> Self {
        Target {
            x,
            y,
            width: size.max(1),
            length: size.max(1),
            reach: Reach::Beside,
        }
    }

    pub fn loc((x, y): Tile, (width, length): (i32, i32), shape: Option<u8>, rotation: u8) -> Self {
        let rotation = rotation & 3;
        let reach = match shape {
            Some(shape @ (0..=3 | 9)) => Reach::Wall { shape, rotation },
            Some(shape @ 4..=8) => Reach::WallDecoration { shape, rotation },
            Some(10 | 11 | 22) | None => Reach::Rectangle,
            Some(_) => Reach::Tile,
        };
        Target {
            x,
            y,
            width: width.max(1),
            length: length.max(1),
            reach,
        }
    }

    fn covers(&self, x: i32, y: i32) -> bool {
        (self.x..self.x + self.width).contains(&x) && (self.y..self.y + self.length).contains(&y)
    }

    fn distance_squared(&self, x: i32, y: i32) -> i32 {
        let gap = |v: i32, low: i32, size: i32| (low - v).max(v - (low + size - 1)).max(0);
        let (dx, dy) = (gap(x, self.x, self.width), gap(y, self.y, self.length));
        dx * dx + dy * dy
    }

    fn beside(&self, flags: &impl Fn(i32, i32) -> Option<u32>, x: i32, y: i32) -> bool {
        let (east, north) = (self.x + self.width - 1, self.y + self.length - 1);
        let open = |wall: u32| flags(x, y).is_some_and(|f| f & wall == 0);
        let along_x = (self.x..=east).contains(&x);
        let along_y = (self.y..=north).contains(&y);
        (x == self.x - 1 && along_y && open(flag::WALL_EAST))
            || (x == east + 1 && along_y && open(flag::WALL_WEST))
            || (y == self.y - 1 && along_x && open(flag::WALL_NORTH))
            || (y == north + 1 && along_x && open(flag::WALL_SOUTH))
    }

    pub fn reached(&self, flags: &impl Fn(i32, i32) -> Option<u32>, (x, y): Tile) -> bool {
        if self.reach != Reach::Beside && (x, y) == (self.x, self.y) {
            return true;
        }
        let free = |mask: u32| flags(x, y).is_some_and(|f| f & mask == 0);
        let west = x == self.x - 1 && y == self.y;
        let east = x == self.x + 1 && y == self.y;
        let south = x == self.x && y == self.y - 1;
        let north = x == self.x && y == self.y + 1;
        use route_mask::{EAST, NORTH, SOUTH, WEST};
        match self.reach {
            Reach::Tile => false,
            Reach::Beside => !self.covers(x, y) && self.beside(flags, x, y),
            Reach::Rectangle => self.covers(x, y) || self.beside(flags, x, y),
            Reach::Wall { shape, rotation } => match (shape, rotation) {
                (0, 0) => west || (north && free(NORTH)) || (south && free(SOUTH)),
                (0, 1) => north || (west && free(WEST)) || (east && free(EAST)),
                (0, 2) => east || (north && free(NORTH)) || (south && free(SOUTH)),
                (0, 3) => south || (west && free(WEST)) || (east && free(EAST)),
                (2, 0) => west || north || (east && free(EAST)) || (south && free(SOUTH)),
                (2, 1) => (west && free(WEST)) || north || east || (south && free(SOUTH)),
                (2, 2) => (west && free(WEST)) || (north && free(NORTH)) || east || south,
                (2, 3) => west || (north && free(NORTH)) || (east && free(EAST)) || south,
                (9, _) => any_side_open(&free, west, east, south, north),
                _ => false,
            },
            Reach::WallDecoration { shape, rotation } => {
                let facing = if shape == 7 {
                    (rotation + 2) & 3
                } else {
                    rotation
                };
                let open = |w: bool, e: bool, s: bool, n: bool| any_side_open(&free, w, e, s, n);
                match (shape, facing) {
                    (6 | 7, 0) => open(false, east, south, false),
                    (6 | 7, 1) => open(west, false, south, false),
                    (6 | 7, 2) => open(west, false, false, north),
                    (6 | 7, 3) => open(false, east, false, north),
                    (8, _) => open(west, east, south, north),
                    _ => false,
                }
            }
        }
    }
}

fn any_side_open(
    free: &impl Fn(u32) -> bool,
    west: bool,
    east: bool,
    south: bool,
    north: bool,
) -> bool {
    (north && free(flag::WALL_SOUTH))
        || (south && free(flag::WALL_NORTH))
        || (west && free(flag::WALL_EAST))
        || (east && free(flag::WALL_WEST))
}

pub fn game_route(flags: impl Fn(i32, i32) -> Option<u32>, from: Tile, to: Tile) -> Option<Route> {
    target_route(flags, from, &Target::tile(to))
}

pub fn target_route(
    flags: impl Fn(i32, i32) -> Option<u32>,
    from: Tile,
    target: &Target,
) -> Option<Route> {
    let (ox, oy) = (from.0 - GRAPH / 2, from.1 - GRAPH / 2);
    let on_graph =
        |x: i32, y: i32| (0..GRAPH).contains(&(x - ox)) && (0..GRAPH).contains(&(y - oy));
    let index = |x: i32, y: i32| ((x - ox) * GRAPH + (y - oy)) as usize;
    let size = (GRAPH * GRAPH) as usize;
    let mut via = vec![0u8; size];
    let mut steps = vec![u32::MAX; size];
    via[index(from.0, from.1)] = START;
    steps[index(from.0, from.1)] = 0;
    let mut queue = VecDeque::from([from]);
    let mut end = None;
    while let Some((x, y)) = queue.pop_front() {
        if target.reached(&flags, (x, y)) {
            end = Some((x, y));
            break;
        }
        let next = steps[index(x, y)] + 1;
        for ((dx, dy), back) in DIRECTIONS.into_iter().zip(BACK) {
            let (nx, ny) = (x + dx, y + dy);
            if !on_graph(nx, ny) {
                continue;
            }
            let i = index(nx, ny);
            if via[i] == 0 && step_open(&flags, x, y, dx, dy) {
                via[i] = back;
                steps[i] = next;
                queue.push_back((nx, ny));
            }
        }
    }
    let reached = end.is_some();
    let end = match end {
        Some(end) => end,
        None => {
            let mut best: Option<((i32, u32), Tile)> = None;
            for x in target.x - ALTERNATIVE_RADIUS..=target.x + ALTERNATIVE_RADIUS {
                for y in target.y - ALTERNATIVE_RADIUS..=target.y + ALTERNATIVE_RADIUS {
                    if !on_graph(x, y) || steps[index(x, y)] >= ALTERNATIVE_MAX_STEPS {
                        continue;
                    }
                    let score = (target.distance_squared(x, y), steps[index(x, y)]);
                    if best.is_none_or(|(b, _)| score < b) {
                        best = Some((score, (x, y)));
                    }
                }
            }
            best?.1
        }
    };
    let mut tiles = vec![end];
    let (mut x, mut y) = end;
    while (x, y) != from {
        let back = via[index(x, y)];
        if back & BACK_EAST != 0 {
            x += 1;
        } else if back & BACK_WEST != 0 {
            x -= 1;
        }
        if back & BACK_NORTH != 0 {
            y += 1;
        } else if back & BACK_SOUTH != 0 {
            y -= 1;
        }
        tiles.push((x, y));
    }
    tiles.reverse();
    Some(Route { tiles, reached })
}

pub fn stand_tiles(flags: impl Fn(i32, i32) -> Option<u32>, target: &Target) -> Vec<Tile> {
    let standable = |x: i32, y: i32| flags(x, y).is_some_and(|f| f & route_mask::STAND == 0);
    let mut tiles = Vec::new();
    for x in target.x - 1..=target.x + target.width {
        for y in target.y - 1..=target.y + target.length {
            if standable(x, y) && target.reached(&flags, (x, y)) {
                tiles.push((x, y));
            }
        }
    }
    tiles
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::region_id;

    const BASE: i32 = 50 * 64;

    fn empty() -> WorldCollision<Movement> {
        let mut world = WorldCollision::<Movement>::default();
        world.add_square(region_id(50, 50));
        world
    }

    fn square(rows: &[&str]) -> (WorldCollision<Movement>, i32, i32) {
        let mut world = empty();
        for (row, line) in rows.iter().enumerate() {
            let y = (rows.len() - 1 - row) as i32;
            for (x, c) in line.chars().enumerate() {
                if c == '#' {
                    world.add(0, BASE + x as i32, BASE + y, flag::OBJECT);
                }
            }
        }
        (world, BASE, BASE)
    }

    #[test]
    fn walks_around_a_wall() {
        let (world, bx, by) = square(&[".....", ".###.", ".#...", ".#..."]);
        let path = find_path(&world, 0, (bx, by), (bx + 2, by), 10_000).unwrap();
        assert_eq!(path.first(), Some(&(bx, by)));
        assert_eq!(path.last(), Some(&(bx + 2, by)));
        for w in path.windows(2) {
            let (dx, dy) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
            assert!(
                can_step(&world, 0, w[0].0, w[0].1, dx, dy),
                "illegal step {w:?}"
            );
        }
        assert_eq!(path.len(), 12, "{path:?}");
    }

    #[test]
    fn no_diagonal_through_a_corner() {
        let (world, bx, by) = square(&["#.", ".."]);
        assert!(!can_step(&world, 0, bx, by, 1, 1));
        assert!(can_step(&world, 0, bx, by, 1, 0));
        let (world, bx, by) = square(&["#.", ".#"]);
        assert!(!can_step(&world, 0, bx, by, 1, 1));
    }

    #[test]
    fn missing_squares_block() {
        let (world, bx, by) = square(&["."]);
        assert!(!can_step(&world, 0, bx, by, -1, 0));
        assert_eq!(
            find_path(&world, 0, (bx, by), (bx - 5, by), 1000),
            Err(PathError::Blocked((bx - 5, by)))
        );
    }

    fn route(rows: &[&str], from: Tile, to: Tile) -> Option<Route> {
        let scene = crate::scene_collision::from_rows(0, 0, rows);
        game_route(|x, y| scene.flags(x, y), from, to)
    }

    #[test]
    fn game_route_prefers_west_east_south_north_then_diagonals() {
        let open = [".....", ".....", ".....", ".....", "....."];
        assert_eq!(
            route(&open, (0, 2), (3, 2)).unwrap().tiles,
            vec![(0, 2), (1, 2), (2, 2), (3, 2)]
        );
        let r = route(&open, (0, 0), (3, 2)).unwrap();
        assert!(r.reached);
        assert_eq!(r.tiles, vec![(0, 0), (1, 0), (2, 1), (3, 2)]);
    }

    #[test]
    fn game_route_goes_around_and_ends_near_unreachable_tiles() {
        let rows = ["..#..", "..#..", "..#..", "....."];
        let r = route(&rows, (0, 3), (4, 3)).unwrap();
        assert!(r.reached);
        assert_eq!(
            r.tiles,
            vec![
                (0, 3),
                (0, 2),
                (0, 1),
                (1, 0),
                (2, 0),
                (3, 0),
                (3, 1),
                (3, 2),
                (4, 3)
            ]
        );
        let rows = [".....", ".###.", ".#.#.", ".###.", "....."];
        let r = route(&rows, (0, 0), (2, 2)).unwrap();
        assert!(!r.reached);
        assert_eq!(r.tiles.last(), Some(&(0, 2)));
        let r = route(&rows, (0, 2), (1, 2)).unwrap();
        assert_eq!(r.tiles, vec![(0, 2)]);
    }

    fn walled(rows: &[&str], walls: &[(Tile, u32)]) -> impl Fn(i32, i32) -> Option<u32> + use<> {
        let scene = crate::scene_collision::from_rows(0, 0, rows);
        let walls = walls.to_vec();
        move |x, y| {
            let extra = walls
                .iter()
                .filter(|(at, _)| *at == (x, y))
                .fold(0, |f, (_, wall)| f | wall);
            scene.flags(x, y).map(|f| f | extra)
        }
    }

    fn wall_across(y: i32, width: i32, gap: Option<i32>) -> Vec<(Tile, u32)> {
        (0..width)
            .filter(|&x| Some(x) != gap)
            .flat_map(|x| [((x, y), flag::WALL_NORTH), ((x, y + 1), flag::WALL_SOUTH)])
            .collect()
    }

    #[test]
    fn an_npc_behind_a_wall_is_not_reached_from_across_it() {
        let open = [".....", ".....", ".....", ".....", "....."];
        let banker = Target::npc((2, 3), 1);
        let flags = walled(&open, &wall_across(1, 5, None));
        let r = target_route(&flags, (2, 1), &banker).unwrap();
        assert!(!r.reached);
        assert_eq!(r.tiles, vec![(2, 1)]);
        let flags = walled(&open, &wall_across(1, 5, Some(4)));
        let r = target_route(&flags, (2, 1), &banker).unwrap();
        assert!(r.reached);
        assert!(banker.reached(&flags, *r.tiles.last().unwrap()));
    }

    #[test]
    fn an_npc_is_used_from_beside_it_never_under_it() {
        let open = [".....", ".....", ".....", ".....", "....."];
        let flags = walled(&open, &[]);
        let npc = Target::npc((2, 2), 1);
        let r = target_route(&flags, (2, 2), &npc).unwrap();
        assert!(r.reached);
        assert_eq!(r.tiles, vec![(2, 2), (1, 2)]);
        assert!(!npc.reached(&flags, (1, 1)));
        let big = Target::npc((1, 1), 2);
        assert_eq!(
            stand_tiles(&flags, &big),
            vec![
                (0, 1),
                (0, 2),
                (1, 0),
                (1, 3),
                (2, 0),
                (2, 3),
                (3, 1),
                (3, 2)
            ]
        );
    }

    #[test]
    fn an_item_on_a_table_is_never_reached() {
        let rows = [".....", ".....", "..#..", ".....", "....."];
        let flags = walled(&rows, &[]);
        let item = Target::tile((2, 2));
        let r = target_route(&flags, (0, 2), &item).unwrap();
        assert!(!r.reached);
        assert_eq!(r.tiles.last(), Some(&(1, 2)));
        assert!(stand_tiles(&flags, &item).is_empty());
    }

    #[test]
    fn an_object_is_used_from_its_sides_but_not_through_a_wall() {
        let rows = [".....", ".##..", ".##..", ".....", "....."];
        let walls = [((0, 2), flag::WALL_EAST), ((1, 2), flag::WALL_WEST)];
        let flags = walled(&rows, &walls);
        let table = Target::loc((1, 2), (2, 2), Some(10), 0);
        assert_eq!(
            stand_tiles(&flags, &table),
            vec![(0, 3), (1, 1), (1, 4), (2, 1), (2, 4), (3, 2), (3, 3)]
        );
    }

    #[test]
    fn walls_and_wall_decorations_are_used_from_their_own_sides() {
        let open = [".....", ".....", ".....", ".....", "....."];
        let walls = [((2, 2), flag::WALL_WEST), ((1, 2), flag::WALL_EAST)];
        let flags = walled(&open, &walls);
        let door = Target::loc((2, 2), (1, 1), Some(0), 0);
        assert_eq!(
            stand_tiles(&flags, &door),
            vec![(1, 2), (2, 1), (2, 2), (2, 3)]
        );
        let lever = Target::loc((2, 2), (1, 1), Some(4), 0);
        assert_eq!(stand_tiles(&flags, &lever), vec![(2, 2)]);
        let corner = Target::loc((2, 2), (1, 1), Some(6), 1);
        assert_eq!(stand_tiles(&flags, &corner), vec![(2, 1), (2, 2)]);
        let flags = walled(&open, &[]);
        assert_eq!(stand_tiles(&flags, &corner), vec![(1, 2), (2, 1), (2, 2)]);
    }

    #[test]
    fn goes_through_doors_when_asked() {
        let mut world = empty();
        for y in 0..64 {
            let door = |bit: u32| if y == 0 { bit } else { 0 };
            world.add(0, BASE, BASE + y, flag::WALL_EAST | door(flag::DOOR_EAST));
            world.add(
                0,
                BASE + 1,
                BASE + y,
                flag::WALL_WEST | door(flag::DOOR_WEST),
            );
        }
        let (bx, by) = (BASE, BASE);
        assert!(find_path(&world, 0, (bx, by + 3), (bx + 1, by + 3), 100_000).is_err());
        let path = find_path_with(
            &world,
            0,
            (bx, by + 3),
            (bx + 1, by + 3),
            100_000,
            true,
            &Default::default(),
        )
        .unwrap();
        assert!(
            path.contains(&(bx, by)) && path.contains(&(bx + 1, by)),
            "{path:?}"
        );
    }

    #[test]
    fn walls_block_both_sides() {
        let mut world = empty();
        world.add(0, BASE, BASE, flag::WALL_EAST);
        world.add(0, BASE + 1, BASE, flag::WALL_WEST);
        let (bx, by) = (BASE, BASE);
        assert!(!can_step(&world, 0, bx, by, 1, 0));
        assert!(!can_step(&world, 0, bx + 1, by, -1, 0));
        assert!(can_step(&world, 0, bx, by, 0, 1));
        let path = find_path(&world, 0, (bx, by), (bx + 1, by), 1000).unwrap();
        assert_eq!(path.len(), 4, "{path:?}");
    }
}
