use ipc::state::{ModelRequest, model_target};

const MODEL_HULL: usize = 128;
const MODEL_FACES: usize = 8192;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub bounding_box: bool,
    pub orientation: i32,
    pub vertex_count: u32,
    pub corners: Vec<[f32; 2]>,
    pub hull: Vec<[f32; 2]>,
    pub face_count: u32,
    pub faces: Vec<[[f32; 2]; 3]>,
}

const SCENE_COORD: std::ops::Range<i32> = 0..256;

pub fn args(request: &ModelRequest) -> Option<[i32; 4]> {
    match request.target {
        model_target::NPC | model_target::PLAYER => {
            Some([request.target as i32, request.index, 0, 0])
        }
        model_target::LOC
            if request.loc_kind < 256
                && SCENE_COORD.contains(&request.loc_x)
                && SCENE_COORD.contains(&request.loc_y) =>
        {
            Some([
                model_target::LOC as i32,
                (request.loc_kind as i32) << 16 | request.loc_x << 8 | request.loc_y,
                request.loc_id,
                0,
            ])
        }
        _ => None,
    }
}

pub fn request(args: [i32; 4]) -> Option<ModelRequest> {
    match args[0] as u32 {
        target @ (model_target::NPC | model_target::PLAYER) => Some(ModelRequest {
            target,
            index: args[1],
            ..ModelRequest::default()
        }),
        model_target::LOC => Some(ModelRequest {
            target: model_target::LOC,
            loc_kind: (args[1] >> 16 & 0xff) as u32,
            loc_x: args[1] >> 8 & 0xff,
            loc_y: args[1] & 0xff,
            loc_id: args[2],
            ..ModelRequest::default()
        }),
        _ => None,
    }
}

const MIN_EXTREME: i32 = 32;
const BOUNDING_BOX_MARGIN: i32 = 8;
const ANGLE_STEP: f64 = 0.0030679615;

pub fn sin_cos(orientation: i32) -> (i32, i32) {
    let angle = (orientation & 2047) as f64 * ANGLE_STEP;
    (
        (65536.0 * angle.sin()) as i32,
        (65536.0 * angle.cos()) as i32,
    )
}

pub fn rotate([x, y, z]: [i32; 3], sin: i32, cos: i32) -> [i32; 3] {
    let rx = z.wrapping_mul(sin).wrapping_add(x.wrapping_mul(cos)) >> 16;
    let rz = z.wrapping_mul(cos).wrapping_sub(x.wrapping_mul(sin)) >> 16;
    [rx, y, rz]
}

fn bounding_box(vertices: &[[i32; 3]], bounding_box: bool) -> ([i32; 3], [i32; 3]) {
    let mut min = [0i32; 3];
    let mut max = [0i32; 3];
    for v in vertices {
        for axis in 0..3 {
            min[axis] = min[axis].min(v[axis]);
            max[axis] = max[axis].max(v[axis]);
        }
    }
    let center = [0, 1, 2].map(|a| (max[a] + min[a]) / 2);
    let mut extreme = [0, 1, 2].map(|a| (max[a] - min[a] + 1) / 2);
    for axis in [0, 2] {
        extreme[axis] = extreme[axis].max(MIN_EXTREME);
        if bounding_box {
            extreme[axis] += BOUNDING_BOX_MARGIN;
        }
    }
    (center, extreme)
}

fn convex_hull(mut points: Vec<[f32; 2]>) -> Vec<[f32; 2]> {
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let cross = |o: [f32; 2], a: [f32; 2], b: [f32; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut hull: Vec<[f32; 2]> = Vec::with_capacity(points.len() * 2);
    let n = points.len();
    for pass in 0..2 {
        let start = hull.len();
        for i in 0..n {
            let p = if pass == 0 {
                points[i]
            } else {
                points[n - 1 - i]
            };
            while hull.len() >= start + 2
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0
            {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    hull
}

fn thin(hull: Vec<[f32; 2]>, limit: usize) -> Vec<[f32; 2]> {
    if hull.len() <= limit {
        return hull;
    }
    (0..limit).map(|i| hull[i * hull.len() / limit]).collect()
}

pub struct Geometry {
    pub bounding_box: bool,
    pub vertices: Vec<[i32; 3]>,
    pub faces: Vec<[usize; 3]>,
}

pub struct Projected {
    pub bounding_box: bool,
    pub corners: Option<[[f32; 2]; 8]>,
    pub points: Vec<[f32; 2]>,
    pub faces: Vec<[[f32; 2]; 3]>,
    pub vertex_count: usize,
}

pub fn project(
    geometry: &Geometry,
    orientation: i32,
    to_screen: impl Fn([i32; 3]) -> Option<[f32; 2]>,
) -> Projected {
    let (sin, cos) = sin_cos(orientation);
    let turned: Vec<[i32; 3]> = geometry
        .vertices
        .iter()
        .map(|&v| rotate(v, sin, cos))
        .collect();

    let (center, extreme) = bounding_box(&turned, geometry.bounding_box);
    let corners: [Option<[f32; 2]>; 8] = std::array::from_fn(|i| {
        let sign = |bit: i32| if i as i32 & bit == 0 { -1 } else { 1 };
        to_screen([
            center[0] + sign(1) * extreme[0],
            center[1] + sign(2) * extreme[1],
            center[2] + sign(4) * extreme[2],
        ])
    });
    let corners = corners
        .iter()
        .all(Option::is_some)
        .then(|| corners.map(|corner| corner.unwrap_or_default()));

    let screen: Vec<Option<[f32; 2]>> = turned.iter().map(|&v| to_screen(v)).collect();
    let faces = geometry
        .faces
        .iter()
        .filter_map(|&[a, b, c]| Some([screen[a]?, screen[b]?, screen[c]?]))
        .collect();
    Projected {
        bounding_box: geometry.bounding_box,
        corners,
        points: screen.into_iter().flatten().collect(),
        faces,
        vertex_count: geometry.vertices.len(),
    }
}

pub fn summary(models: &[Projected], orientation: i32) -> Option<Model> {
    let first = models.first()?;
    let hull = thin(
        convex_hull(
            models
                .iter()
                .flat_map(|m| m.points.iter().copied())
                .collect(),
        ),
        MODEL_HULL,
    );
    Some(Model {
        bounding_box: first.bounding_box,
        orientation: orientation & 2047,
        vertex_count: models.iter().map(|m| m.vertex_count as u32).sum(),
        corners: first.corners.map(Vec::from).unwrap_or_default(),
        hull,
        face_count: models.iter().map(|m| m.faces.len() as u32).sum(),
        faces: models
            .iter()
            .flat_map(|m| m.faces.iter().copied())
            .take(MODEL_FACES)
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_round_trip() {
        let loc = ModelRequest {
            target: model_target::LOC,
            loc_kind: 4,
            loc_x: 52,
            loc_y: 103,
            loc_id: 5296,
            ..ModelRequest::default()
        };
        let npc = ModelRequest {
            target: model_target::NPC,
            index: 1234,
            ..ModelRequest::default()
        };
        for model in [loc, npc] {
            assert_eq!(request(args(&model).unwrap()), Some(model));
        }
    }

    #[test]
    fn tiles_outside_the_scene_have_no_args() {
        let far = ModelRequest {
            target: model_target::LOC,
            loc_x: 300,
            ..ModelRequest::default()
        };
        assert_eq!(args(&far), None);
    }

    #[test]
    fn hull_drops_inner_points() {
        let hull = convex_hull(vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0],
            [5.0, 5.0],
            [5.0, 0.0],
        ]);
        assert_eq!(hull.len(), 4);
        assert!(!hull.contains(&[5.0, 5.0]));
    }

    #[test]
    fn box_matches_java_rounding_and_minimum() {
        let (center, extreme) = bounding_box(&[[-10, -200, 4], [12, 3, 20]], false);
        assert_eq!(center, [1, -98, 10]);
        assert_eq!(extreme, [32, 102, 32]);
        let (_, boxed) = bounding_box(&[[-10, -200, 4], [12, 3, 20]], true);
        assert_eq!(boxed, [40, 102, 40]);
    }

    #[test]
    fn rotation_is_a_quarter_turn_at_512() {
        let (sin, cos) = sin_cos(512);
        assert_eq!(rotate([100, -5, 0], sin, cos), [0, -5, -100]);
    }
}
