use std::sync::OnceLock;

use decode::model::{project, summary, Geometry, Model, Projected};

use ipc::{
    read::{self, Memory},
    state::{game_state, model_status, model_target, ModelRequest},
};

use super::{
    drawn::{self, Drawn, Frame, Shape},
    offsets::{u16_at, u32_at, ModelFaces, ModelVertices, NpcModel},
    own::{OwnMemory, LAYOUT},
};

#[derive(Clone, Copy)]
struct Rules {
    npc: NpcModel,
    vertices: ModelVertices,
    faces: ModelFaces,
}

static RULES: OnceLock<Rules> = OnceLock::new();

const MAX_VERTICES: usize = 20_000;
const MAX_FACES: usize = 40_000;
const HIDDEN_FACE: i32 = -2;

fn section(image: &[u8], name: &[u8]) -> Option<std::ops::Range<usize>> {
    let nt = u32_at(image, 0x3c)? as usize;
    let count = u16_at(image, nt + 6)? as usize;
    let table = nt + 24 + u16_at(image, nt + 20)? as usize;
    (0..count).find_map(|i| {
        let header = table + i * 40;
        let section_name = image.get(header..header + 8)?;
        if !section_name.starts_with(name) {
            return None;
        }
        let (size, rva) = (
            u32_at(image, header + 8)? as usize,
            u32_at(image, header + 12)? as usize,
        );
        Some(rva..(rva + size).min(image.len()) / 8 * 8)
    })
}

pub fn animation_slot(
    image: &[u8],
    module_base: u64,
    get_model: u32,
    animation_height: u32,
) -> Option<u32> {
    let (get_model, height) = (
        module_base + get_model as u64,
        module_base + animation_height as u64,
    );
    let rdata = section(image, b".rdata")?;
    let qwords = image.get(rdata)?.as_chunks::<8>().0;
    let is = |qword: &[u8; 8], value: u64| u64::from_le_bytes(*qword) == value;
    qwords
        .iter()
        .enumerate()
        .filter(|&(_, qword)| is(qword, get_model))
        .find_map(|(at, _)| {
            qwords[at..(at + 64).min(qwords.len())]
                .iter()
                .position(|qword| is(qword, height))
                .map(|slot| slot as u32 * 8)
        })
}

pub fn set_npc_model(npc: NpcModel, vertices: ModelVertices, faces: ModelFaces) {
    let _ = RULES.set(Rules {
        npc,
        vertices,
        faces,
    });
}

fn count(memory: &OwnMemory, at: u64, max: usize) -> Option<usize> {
    let count = memory.i32(at)?;
    (count > 0 && count as usize <= max).then_some(count as usize)
}

fn array(memory: &OwnMemory, model: u64, offset: u32, count: usize) -> Option<Vec<i32>> {
    memory
        .ptr(model + offset as u64)
        .and_then(|p| memory.i32s(p, count))
}

fn read_vertices(memory: &OwnMemory, model: u64, layout: &ModelVertices) -> Option<Vec<[i32; 3]>> {
    let count = count(memory, model + layout.count as u64, MAX_VERTICES)?;
    let field = |offset| array(memory, model, offset, count);
    let (xs, ys, zs) = (field(layout.x)?, field(layout.y)?, field(layout.z)?);
    Some((0..count).map(|i| [xs[i], ys[i], zs[i]]).collect())
}

fn read_faces(
    memory: &OwnMemory,
    model: u64,
    layout: &ModelFaces,
    vertex_count: usize,
) -> Option<Vec<[usize; 3]>> {
    let count = count(memory, model + layout.count as u64, MAX_FACES)?;
    let field = |offset| array(memory, model, offset, count);
    let (a, b, c) = (field(layout.a)?, field(layout.b)?, field(layout.c)?);
    let colors = field(layout.color);
    Some(
        (0..count)
            .filter(|&i| {
                colors
                    .as_ref()
                    .is_none_or(|colors| colors[i] != HIDDEN_FACE)
            })
            .map(|i| [a[i] as usize, b[i] as usize, c[i] as usize])
            .filter(|f| f.iter().all(|&v| v < vertex_count))
            .collect(),
    )
}

pub fn geometry(model: u64) -> Option<Geometry> {
    let rules = RULES.get()?;
    let memory = OwnMemory;
    let vertices = read_vertices(&memory, model, &rules.vertices)?;
    let faces = read_faces(&memory, model, &rules.faces, vertices.len()).unwrap_or_default();
    let bounding_box = memory
        .u64(model + rules.npc.bounding_box as u64)
        .is_some_and(|v| v as u8 != 0);
    Some(Geometry {
        bounding_box,
        vertices,
        faces,
    })
}

fn on_camera(camera: &decode::camera::Camera, at: [i32; 3], offset: [i32; 3]) -> Option<[f32; 2]> {
    let (screen, distance) =
        camera.project_relative([0, 1, 2].map(|axis| at[axis].wrapping_add(offset[axis])));
    (distance >= decode::camera::NEAR_PLANE && screen.iter().all(|v| v.is_finite()))
        .then_some(screen)
}

type Models = (Vec<Projected>, i32);

fn actor(target: u32, index: i32) -> Result<u64, i32> {
    let memory = OwnMemory;
    let layout = LAYOUT.get().ok_or(model_status::NOT_FOUND)?;
    let (view, _) = read::local_player(&memory, layout).ok_or(model_status::NOT_IN_GAME)?;
    let table = if target == model_target::PLAYER {
        layout.player.players
    } else {
        layout.world.npc_table
    };
    read::table_get(&memory, view + table as u64, index).ok_or(model_status::NO_NPC)
}

fn renderables(request: &ModelRequest) -> Result<(Vec<u64>, Option<u64>), i32> {
    match request.target {
        model_target::LOC => super::scene::loc_renderables(
            request.loc_kind as u8,
            request.loc_x,
            request.loc_y,
            request.loc_id,
        )
        .ok_or(model_status::NO_NPC),
        target => actor(target, request.index).map(|actor| (vec![actor], None)),
    }
}

fn drawn_models(frame: &Frame, request: &ModelRequest) -> Result<Models, i32> {
    let camera = frame.camera.ok_or(model_status::NOT_FOUND)?;
    let (renderables, tag) = renderables(request)?;
    let drawn: Vec<&Drawn> = renderables
        .into_iter()
        .filter_map(|renderable| frame.find(renderable, tag))
        .collect();
    let models: Vec<Projected> = drawn
        .iter()
        .filter_map(|d| {
            let kept;
            let geometry = match &d.shape {
                Shape::Copied(geometry) => geometry,
                &Shape::Kept(model) => {
                    kept = geometry(model)?;
                    &kept
                }
            };
            Some(project(geometry, d.orientation, |offset| {
                on_camera(&camera, d.at, offset)
            }))
        })
        .collect();
    match drawn.first() {
        Some(first) if !models.is_empty() => Ok((models, first.orientation)),
        _ => Err(model_status::NO_MODEL),
    }
}

fn models(frame: &Frame, request: &ModelRequest) -> Result<Models, i32> {
    let memory = OwnMemory;
    match LAYOUT.get().filter(|_| RULES.get().is_some()) {
        None => Err(model_status::NOT_FOUND),
        Some(layout)
            if read::current_game_state(&memory, layout) != Some(game_state::LOGGED_IN) =>
        {
            Err(model_status::NOT_IN_GAME)
        }
        _ => drawn_models(frame, request),
    }
}

pub fn faces(request: &ModelRequest) -> Result<Vec<[[f32; 2]; 3]>, i32> {
    let frame = drawn::latest().ok_or(model_status::NOT_FOUND)?;
    models(&frame, request).map(|(models, _)| models.into_iter().flat_map(|m| m.faces).collect())
}

pub fn served(args: [i32; 4]) -> Option<Option<Model>> {
    let request = decode::model::request(args)?;
    let frame = drawn::latest()?;
    match models(&frame, &request) {
        Ok((models, orientation)) => Some(summary(&models, orientation)),
        Err(model_status::NO_NPC | model_status::NO_MODEL) => Some(None),
        Err(_) => None,
    }
}
