use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use decode::locs::{request as locs_request, Loc, Locs, Unavailable};
use ipc::{
    read::{self, Memory},
    state::{game_state, Layout},
};

use super::{
    definitions::Definitions,
    hooks::client_image,
    offsets::{loc_kind, render_slots, GameObjectLayout, LocConfigs, Scene},
    own::{OwnMemory, LAYOUT},
};

const TAG_TYPE_SHIFT: u64 = 16;
const TAG_LOC: u64 = 2;
const TAG_NOT_CLICKABLE: u64 = 1 << 19;
const ENTRY_SIZE: u64 = 0x10;
const ENTRY_OBJECT: u64 = 8;
const MAX_GAME_OBJECTS: i32 = 64;
const MAX_SCENE_SIZE: i32 = 1024;
const SHAPE_MASK: i32 = 0x3f;
const ROTATION_SHIFT: i32 = 6;

#[derive(Clone, Copy)]
pub struct Rules {
    pub module_base: u64,
    pub scene: Scene,
    pub game: GameObjectLayout,
    pub configs: Option<LocConfigs>,
    pub plane: u32,
    pub item_pile: Option<u32>,
}

static RULES: OnceLock<Rules> = OnceLock::new();
static SLOTS: Mutex<Option<HashMap<u64, Vec<u32>>>> = Mutex::new(None);

pub fn set_scene(rules: Rules) {
    let _ = RULES.set(rules);
}

struct View {
    scene: u64,
    plane: i32,
    base_x: i32,
    base_y: i32,
}

fn view(memory: &OwnMemory, rules: &Rules) -> Option<View> {
    let layout = LAYOUT.get()?;
    let (world_view, _) = read::local_player(memory, layout)?;
    let base = layout.player.base_x as u64;
    Some(View {
        scene: memory.ptr(world_view + rules.scene.scene as u64)?,
        plane: memory.i32(world_view + rules.plane as u64)?,
        base_x: memory.i32(world_view + base)?,
        base_y: memory.i32(world_view + base + 4)?,
    })
}

struct Tiles {
    plane: i32,
    size_x: i32,
    size_y: i32,
    base: u64,
}

impl Tiles {
    fn new(memory: &OwnMemory, rules: &Rules, scene: u64, plane: i32) -> Option<Self> {
        let s = &rules.scene;
        let (size_x, size_y) = (
            memory.i32(scene + s.size_x as u64)?,
            memory.i32(scene + s.size_y as u64)?,
        );
        let base = memory.ptr(scene + s.tiles as u64)?;
        Some(Self {
            plane,
            size_x,
            size_y,
            base,
        })
    }

    fn at(&self, memory: &OwnMemory, x: i32, y: i32) -> Option<u64> {
        if !(0..4).contains(&self.plane)
            || !(0..self.size_x).contains(&x)
            || !(0..self.size_y).contains(&y)
        {
            return None;
        }
        let index = ((self.plane * self.size_x + x) * self.size_y + y) as u64;
        memory.ptr(self.base + index * ENTRY_SIZE + ENTRY_OBJECT)
    }

    fn objects(&self, memory: &OwnMemory) -> Option<Vec<u64>> {
        if !(0..4).contains(&self.plane)
            || !(0..=MAX_SCENE_SIZE).contains(&self.size_x)
            || !(0..=MAX_SCENE_SIZE).contains(&self.size_y)
        {
            return None;
        }
        let count = (self.size_x * self.size_y) as usize;
        let mut entries = vec![0u8; count * ENTRY_SIZE as usize];
        let first = self.base + self.plane as u64 * count as u64 * ENTRY_SIZE;
        if count > 0 && !memory.read(first, &mut entries) {
            return None;
        }
        Some(
            entries
                .as_chunks::<{ ENTRY_SIZE as usize }>()
                .0
                .iter()
                .map(|entry| Fields(entry).u64(ENTRY_OBJECT as u32).unwrap_or(0))
                .collect(),
        )
    }
}

struct Fields<'a>(&'a [u8]);

impl Fields<'_> {
    fn bytes<const N: usize>(&self, at: u32) -> Option<[u8; N]> {
        self.0.get(at as usize..at as usize + N)?.try_into().ok()
    }

    fn u64(&self, at: u32) -> Option<u64> {
        self.bytes(at).map(u64::from_le_bytes)
    }

    fn ptr(&self, at: u32) -> Option<u64> {
        self.u64(at).filter(|&p| p != 0)
    }

    fn i32(&self, at: u32) -> Option<i32> {
        self.bytes(at).map(i32::from_le_bytes)
    }

    fn u8(&self, at: u32) -> Option<u8> {
        self.bytes(at).map(|[b]| b)
    }
}

fn read_fields<'a>(memory: &OwnMemory, at: u64, block: &'a mut [u8]) -> Option<Fields<'a>> {
    memory.read(at, block).then_some(Fields(block))
}

struct Blocks {
    tile: Vec<u8>,
    slot: Vec<u8>,
    game: Vec<u8>,
}

impl Blocks {
    fn new(rules: &Rules) -> Self {
        let s = &rules.scene;
        let span = |ends: &[Option<u32>]| {
            vec![0u8; ends.iter().flatten().max().copied().unwrap_or(0) as usize]
        };
        Self {
            tile: span(&[
                Some(s.wall + 8),
                Some(s.wall_decoration + 8),
                Some(s.ground_decoration + 8),
                Some(s.game_count + 4),
                Some(s.game_array + 8),
                rules.item_pile.map(|pile| pile + 8),
            ]),
            slot: span(&single_slots(rules).map(|(_, _, config)| Some(slot_span(s, config)))),
            game: span(&[
                Some(s.game_tag + 8),
                Some(s.game_origin_x + 8),
                Some(s.game_origin_y + 8),
                Some(rules.game.config + 4),
            ]),
        }
    }
}

fn shape_rotation(config: i32) -> (u8, u8) {
    (
        (config & SHAPE_MASK) as u8,
        ((config >> ROTATION_SHIFT) & 3) as u8,
    )
}

fn loc_id(rules: &Rules, tag: u64) -> i32 {
    ((tag >> rules.scene.id_shift) & 0xffff) as i32
}

fn slot_span(scene: &Scene, config: Option<u32>) -> u32 {
    (scene.tag + 8).max(config.map_or(0, |c| c + 1))
}

fn single_slots(rules: &Rules) -> [(u32, u8, Option<u32>); 3] {
    let s = &rules.scene;
    let c = rules.configs;
    [
        (s.wall, loc_kind::WALL, c.map(|c| c.wall)),
        (
            s.wall_decoration,
            loc_kind::WALL_DECORATION,
            c.map(|c| c.wall_decoration),
        ),
        (
            s.ground_decoration,
            loc_kind::GROUND_DECORATION,
            c.map(|c| c.ground_decoration),
        ),
    ]
}

struct GameObject {
    object: u64,
    tag: u64,
    x1: i32,
    x2: i32,
    y1: i32,
    y2: i32,
    config: i32,
}

fn game_objects(
    memory: &OwnMemory,
    rules: &Rules,
    tile: &Fields,
    block: &mut [u8],
) -> Vec<GameObject> {
    let s = &rules.scene;
    let count = tile
        .i32(s.game_count)
        .unwrap_or(0)
        .clamp(0, MAX_GAME_OBJECTS) as usize;
    let Some(array) = tile.ptr(s.game_array) else {
        return Vec::new();
    };
    let mut entries = vec![0u8; count * ENTRY_SIZE as usize];
    if count > 0 && !memory.read(array, &mut entries) {
        return Vec::new();
    }
    entries
        .as_chunks::<{ ENTRY_SIZE as usize }>()
        .0
        .iter()
        .filter_map(|entry| Fields(entry).ptr(ENTRY_OBJECT as u32))
        .filter_map(|object| {
            let fields = read_fields(memory, object, block)?;
            let tag = fields.u64(s.game_tag)?;
            if (tag >> TAG_TYPE_SHIFT) & 7 != TAG_LOC {
                return None;
            }
            Some(GameObject {
                object,
                tag,
                x1: fields.i32(s.game_origin_x)?,
                x2: fields.i32(s.game_origin_x + 4)?,
                y1: fields.i32(s.game_origin_y)?,
                y2: fields.i32(s.game_origin_y + 4)?,
                config: fields.i32(rules.game.config).unwrap_or(0),
            })
        })
        .collect()
}

fn shown(
    memory: &OwnMemory,
    layout: &Layout,
    definitions: &Definitions,
    id: i32,
) -> Option<(Option<String>, i16)> {
    let loc = definitions.locs.get(&u32::try_from(id).ok()?)?;
    let height = loc.decoration_height as i16;
    let Some(transforms) = &loc.transforms else {
        return Some((loc.name.clone(), height));
    };
    let varp = |id: u16| decode::state::varp(memory, layout, id as usize);
    let value = match (transforms.varbit, transforms.varp) {
        (Some(varbit), _) => {
            let def = definitions.varbits.get(&(varbit as u32))?;
            Some(def.value(varp(def.varp)?))
        }
        (None, Some(id)) => Some(varp(id)?),
        (None, None) => None,
    };
    let name = transforms
        .pick(value)
        .and_then(|id| definitions.locs.get(&id))
        .and_then(|shown| shown.name.clone());
    Some((name, height))
}

pub fn locs(request: i32) -> Result<Locs, Unavailable> {
    let memory = OwnMemory;
    let (Some(layout), Some(rules)) = (LAYOUT.get(), RULES.get()) else {
        return Err(Unavailable::NotFound);
    };
    if read::current_game_state(&memory, layout) != Some(game_state::LOGGED_IN) {
        return Err(Unavailable::NotInGame);
    }
    let view = view(&memory, rules).ok_or(Unavailable::NotInGame)?;
    let definitions = super::definitions::get()?;
    let s = &rules.scene;
    let planes = match request & locs_request::PLANES {
        locs_request::ALL_PLANES => 0..4,
        plane @ 0..=3 => plane..plane + 1,
        _ => view.plane..view.plane + 1,
    };
    let clickable_only = request & locs_request::CLICKABLE_ONLY != 0;
    let mut found: Vec<Loc> = Vec::new();
    let mut push = |info: Loc, tag: u64, plane: i32| {
        let clickable = tag & TAG_NOT_CLICKABLE == 0;
        if clickable_only && !clickable {
            return;
        }
        found.push(Loc {
            id: loc_id(rules, tag),
            clickable,
            plane: plane as u8,
            ..info
        });
    };
    let mut blocks = Blocks::new(rules);
    for plane in planes {
        let Some(tiles) = Tiles::new(&memory, rules, view.scene, plane) else {
            continue;
        };
        let Some(objects) = tiles.objects(&memory) else {
            continue;
        };
        for x in 0..tiles.size_x {
            for y in 0..tiles.size_y {
                let tile = objects[(x * tiles.size_y + y) as usize];
                let Some(tile) = (tile != 0)
                    .then(|| read_fields(&memory, tile, &mut blocks.tile))
                    .flatten()
                else {
                    continue;
                };
                let base = Loc {
                    id: 0,
                    x: x as u8,
                    y: y as u8,
                    size_x: 1,
                    size_y: 1,
                    kind: 0,
                    shape: None,
                    rotation: None,
                    plane: 0,
                    clickable: false,
                    name: None,
                    decoration_height: 0,
                };
                for (slot, kind, config) in single_slots(rules) {
                    let Some(object) = tile.ptr(slot) else {
                        continue;
                    };
                    let block = &mut blocks.slot[..slot_span(s, config) as usize];
                    let Some(fields) = read_fields(&memory, object, block) else {
                        continue;
                    };
                    let Some(tag) = fields.u64(s.tag) else {
                        continue;
                    };
                    let config = config.and_then(|c| fields.u8(c));
                    let (shape, rotation) = config.map(|c| shape_rotation(c as i32)).unzip();
                    push(
                        Loc {
                            kind,
                            shape,
                            rotation,
                            ..base
                        },
                        tag,
                        plane,
                    );
                }
                for object in game_objects(&memory, rules, &tile, &mut blocks.game) {
                    if object.x1 != x || object.y1 != y {
                        continue;
                    }
                    let (shape, rotation) = shape_rotation(object.config);
                    push(
                        Loc {
                            kind: loc_kind::GAME,
                            size_x: (object.x2 - object.x1 + 1).clamp(1, 255) as u8,
                            size_y: (object.y2 - object.y1 + 1).clamp(1, 255) as u8,
                            shape: Some(shape),
                            rotation: Some(rotation),
                            ..base
                        },
                        object.tag,
                        plane,
                    );
                }
            }
        }
    }
    let mut ids: Vec<i32> = found.iter().map(|l| l.id).collect();
    ids.sort_unstable();
    ids.dedup();
    let known: HashMap<i32, (Option<String>, i16)> = ids
        .into_iter()
        .filter_map(|id| Some((id, shown(&memory, layout, definitions, id)?)))
        .collect();
    let mut names: Vec<String> = Vec::new();
    let mut slots: HashMap<i32, Option<u16>> = HashMap::new();
    for loc in &mut found {
        let (name, height) = known.get(&loc.id).cloned().unwrap_or((None, 0));
        if loc.kind == loc_kind::GROUND_DECORATION {
            loc.decoration_height = height;
        }
        loc.name = *slots.entry(loc.id).or_insert_with(|| {
            name.map(|name| {
                names.push(name);
                (names.len() - 1) as u16
            })
        });
    }
    Ok(Locs {
        plane: view.plane,
        base_x: view.base_x,
        base_y: view.base_y,
        locs: found,
        names,
    })
}

fn cached_slots<'c>(
    memory: &OwnMemory,
    rules: &Rules,
    cache: &'c mut HashMap<u64, Vec<u32>>,
    object: u64,
) -> &'c [u32] {
    let Some(vtable) = memory.ptr(object) else {
        return &[];
    };
    cache.entry(vtable).or_insert_with(|| {
        let first = memory.ptr(vtable).unwrap_or(0);
        match unsafe { client_image() } {
            Some(image) if first > rules.module_base => {
                render_slots(image, (first - rules.module_base) as u32)
            }
            _ => Vec::new(),
        }
    })
}

fn slots(memory: &OwnMemory, rules: &Rules, object: u64) -> Vec<u32> {
    let mut cache = SLOTS.lock().unwrap();
    let cache = cache.get_or_insert_with(HashMap::new);
    cached_slots(memory, rules, cache, object).to_vec()
}

fn object_renderables(
    memory: &OwnMemory,
    rules: &Rules,
    cache: &mut HashMap<u64, Vec<u32>>,
    object: u64,
    mut visit: impl FnMut(u64),
) {
    for &offset in cached_slots(memory, rules, cache, object) {
        if let Some(renderable) = memory.ptr(object + offset as u64) {
            visit(renderable);
        }
    }
}

pub fn visit_renderables(mut visit: impl FnMut(u8, u64)) -> Option<()> {
    let memory = OwnMemory;
    let rules = RULES.get()?;
    let view = view(&memory, rules)?;
    let mut blocks = Blocks::new(rules);
    let mut cache = SLOTS.lock().ok()?;
    let cache = cache.get_or_insert_with(HashMap::new);
    for plane in 0..4 {
        let Some(tiles) = Tiles::new(&memory, rules, view.scene, plane) else {
            continue;
        };
        let Some(objects) = tiles.objects(&memory) else {
            continue;
        };
        for tile in objects.into_iter().filter(|&tile| tile != 0) {
            let Some(tile) = read_fields(&memory, tile, &mut blocks.tile) else {
                continue;
            };
            for (slot, kind, _) in single_slots(rules) {
                if let Some(object) = tile.ptr(slot) {
                    object_renderables(&memory, rules, cache, object, |r| visit(kind, r));
                }
            }
            if let Some(pile) = rules.item_pile.and_then(|offset| tile.ptr(offset)) {
                object_renderables(&memory, rules, cache, pile, |r| {
                    visit(loc_kind::ITEM_PILE, r)
                });
            }
            for object in game_objects(&memory, rules, &tile, &mut blocks.game) {
                if let Some(renderable) = memory.ptr(object.object + rules.game.renderable as u64) {
                    visit(loc_kind::GAME, renderable);
                }
            }
        }
    }
    Some(())
}

pub fn loc_renderables(kind: u8, x: i32, y: i32, id: i32) -> Option<(Vec<u64>, Option<u64>)> {
    let memory = OwnMemory;
    let rules = RULES.get()?;
    let view = view(&memory, rules)?;
    let tile = Tiles::new(&memory, rules, view.scene, view.plane)?.at(&memory, x, y)?;
    let mut blocks = Blocks::new(rules);
    let tile = read_fields(&memory, tile, &mut blocks.tile)?;
    let s = &rules.scene;
    if kind == loc_kind::GAME {
        let object = game_objects(&memory, rules, &tile, &mut blocks.game)
            .into_iter()
            .find(|object| loc_id(rules, object.tag) == id)?;
        return Some((
            vec![memory.ptr(object.object + rules.game.renderable as u64)?],
            Some(object.tag),
        ));
    }
    let (object, tag) = if kind == loc_kind::ITEM_PILE {
        (tile.ptr(rules.item_pile?)?, None)
    } else {
        let (slot, _, _) = single_slots(rules)
            .into_iter()
            .find(|&(_, k, _)| k == kind)?;
        let object = tile.ptr(slot)?;
        let tag = memory.u64(object + s.tag as u64)?;
        if loc_id(rules, tag) != id {
            return None;
        }
        (object, Some(tag))
    };
    Some((
        slots(&memory, rules, object)
            .into_iter()
            .filter_map(|offset| memory.ptr(object + offset as u64))
            .collect(),
        tag,
    ))
}
