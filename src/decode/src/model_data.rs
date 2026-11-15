use std::collections::HashMap;
use std::sync::LazyLock;

const ANGLE_UNIT: f64 = 0.0030679615;

static TRIG: LazyLock<Vec<(i32, i32)>> = LazyLock::new(|| {
    (0..2048)
        .map(|i| {
            let angle = i as f64 * ANGLE_UNIT;
            (
                (65536.0 * angle.sin()) as i32,
                (65536.0 * angle.cos()) as i32,
            )
        })
        .collect()
});

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelData {
    pub vertices: Vec<[i32; 3]>,
    pub faces: Vec<[usize; 3]>,
    pub hidden: Vec<bool>,
    pub labels: Vec<i32>,
    texture_vertices: Vec<[usize; 3]>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameMap {
    pub types: Vec<u8>,
    pub labels: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frame {
    pub transforms: Vec<(usize, [i32; 3])>,
}

struct Stream<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Stream<'a> {
    fn at(data: &'a [u8], at: usize) -> Self {
        Self { data, at }
    }

    fn u8(&mut self) -> Option<u8> {
        let v = *self.data.get(self.at)?;
        self.at += 1;
        Some(v)
    }

    fn i8(&mut self) -> Option<i8> {
        self.u8().map(|v| v as i8)
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from(self.u8()?) << 8 | u16::from(self.u8()?))
    }

    fn smart(&mut self) -> Option<i32> {
        if *self.data.get(self.at)? < 128 {
            self.u8().map(|v| i32::from(v) - 64)
        } else {
            self.u16().map(|v| i32::from(v) - 0xc000)
        }
    }
}

struct Layout {
    vertices: usize,
    faces: usize,
    flags: usize,
    axes: [usize; 3],
    labels: Option<usize>,
    types: usize,
    indices: usize,
    render: Option<usize>,
    legacy: bool,
    colors: usize,
    alpha: Option<usize>,
    textures: Option<usize>,
    texture_kinds: Option<usize>,
    texture_count: usize,
    texture_indices: usize,
}

fn legacy_layout(data: &[u8], rigged: bool) -> Option<Layout> {
    let mut h = Stream::at(data, data.len().checked_sub(if rigged { 23 } else { 18 })?);
    let vertices = h.u16()? as usize;
    let faces = h.u16()? as usize;
    let texture_count = h.u8()? as usize;
    let render = h.u8()? == 1;
    let priority = h.u8()?;
    let alpha = h.u8()? == 1;
    let face_skins = h.u8()? == 1;
    let vertex_skins = h.u8()? == 1;
    if rigged {
        h.u8()?;
    }
    let lengths = [h.u16()?, h.u16()?, h.u16()?].map(usize::from);
    let index_length = h.u16()? as usize;
    let skin_length = if rigged {
        h.u16()? as usize
    } else if vertex_skins {
        vertices
    } else {
        0
    };
    let mut pos = vertices + faces;
    if priority == 255 {
        pos += faces;
    }
    if face_skins {
        pos += faces;
    }
    let render_at = pos;
    if render {
        pos += faces;
    }
    let labels_at = pos;
    pos += skin_length;
    let alpha_at = pos;
    if alpha {
        pos += faces;
    }
    let indices = pos;
    pos += index_length;
    let colors = pos;
    pos += faces * 2;
    let texture_indices = pos;
    pos += texture_count * 6;
    let mut axes = [0; 3];
    for (axis, length) in axes.iter_mut().zip(lengths) {
        *axis = pos;
        pos += length;
    }
    Some(Layout {
        vertices,
        faces,
        flags: 0,
        axes,
        labels: vertex_skins.then_some(labels_at),
        types: vertices,
        indices,
        render: render.then_some(render_at),
        legacy: true,
        colors,
        alpha: alpha.then_some(alpha_at),
        textures: None,
        texture_kinds: None,
        texture_count,
        texture_indices,
    })
}

fn layout(data: &[u8], rigged: bool) -> Option<Layout> {
    let mut h = Stream::at(data, data.len().checked_sub(if rigged { 26 } else { 23 })?);
    let vertices = h.u16()? as usize;
    let faces = h.u16()? as usize;
    let texture_count = h.u8()? as usize;
    let render = h.u8()? == 1;
    let priority = h.u8()?;
    let alpha = h.u8()? == 1;
    let face_skins = h.u8()? == 1;
    let textured = h.u8()? == 1;
    let vertex_skins = h.u8()? == 1;
    if rigged {
        h.u8()?;
    }
    let lengths = [h.u16()?, h.u16()?, h.u16()?].map(usize::from);
    let index_length = h.u16()? as usize;
    let coord_length = h.u16()? as usize;
    let skin_length = if rigged {
        h.u16()? as usize
    } else if vertex_skins {
        vertices
    } else {
        0
    };
    let mut pos = texture_count + vertices;
    let render_at = pos;
    if render {
        pos += faces;
    }
    let types = pos;
    pos += faces;
    if priority == 255 {
        pos += faces;
    }
    if face_skins {
        pos += faces;
    }
    let labels_at = pos;
    pos += skin_length;
    let alpha_at = pos;
    if alpha {
        pos += faces;
    }
    let indices = pos;
    pos += index_length;
    let textures_at = pos;
    if textured {
        pos += faces * 2;
    }
    pos += coord_length;
    let colors = pos;
    pos += faces * 2;
    let mut axes = [0; 3];
    for (axis, length) in axes.iter_mut().zip(lengths) {
        *axis = pos;
        pos += length;
    }
    Some(Layout {
        vertices,
        faces,
        flags: texture_count,
        axes,
        labels: vertex_skins.then_some(labels_at),
        types,
        indices,
        render: render.then_some(render_at),
        legacy: false,
        colors,
        alpha: alpha.then_some(alpha_at),
        textures: textured.then_some(textures_at),
        texture_kinds: Some(0),
        texture_count,
        texture_indices: pos,
    })
}

fn hidden(render: i8, alpha: i8, texture: i16) -> bool {
    let render = match alpha {
        -2 => 3,
        -1 => 2,
        _ => render,
    };
    if texture == -1 {
        !matches!(render, 0 | 1 | 3)
    } else {
        !matches!(render, 0 | 1)
    }
}

pub fn decode_model(data: &[u8]) -> Option<ModelData> {
    let layout = match data.len().checked_sub(2).map(|n| (data[n], data[n + 1])) {
        Some((0xff, 0xfd)) => layout(data, true),
        Some((0xff, 0xfe)) => legacy_layout(data, true),
        Some((0xff, 0xff)) => layout(data, false),
        _ => legacy_layout(data, false),
    }?;
    let mut flags = Stream::at(data, layout.flags);
    let mut axes = layout.axes.map(|at| Stream::at(data, at));
    let mut labels_stream = layout.labels.map(|at| Stream::at(data, at));
    let mut vertices = Vec::with_capacity(layout.vertices);
    let mut labels = Vec::with_capacity(layout.vertices);
    let mut point = [0i32; 3];
    for _ in 0..layout.vertices {
        let flag = flags.u8()?;
        for (axis, (stream, value)) in axes.iter_mut().zip(point.iter_mut()).enumerate() {
            if flag & (1 << axis) != 0 {
                *value = value.wrapping_add(stream.smart()?);
            }
        }
        vertices.push(point);
        labels.push(match labels_stream.as_mut() {
            Some(stream) => i32::from(stream.u8()?),
            None => -1,
        });
    }

    let mut render = layout.render.map(|at| Stream::at(data, at));
    let mut alpha = layout.alpha.map(|at| Stream::at(data, at));
    let mut textures = layout.textures.map(|at| Stream::at(data, at));
    let mut colors = Stream::at(data, layout.colors);
    let mut hidden_faces = Vec::with_capacity(layout.faces);
    for _ in 0..layout.faces {
        let color = colors.u16()? as i16;
        let (render_type, legacy_texture) = match render.as_mut() {
            Some(stream) if layout.legacy => {
                let info = stream.u8()?;
                ((info & 1) as i8, if info & 2 != 0 { color } else { -1 })
            }
            Some(stream) => (stream.i8()?, -1),
            None => (0, -1),
        };
        let face_alpha = match alpha.as_mut() {
            Some(stream) => stream.i8()?,
            None => 0,
        };
        let texture = match textures.as_mut() {
            Some(stream) => (i32::from(stream.u16()?) - 1) as i16,
            None => legacy_texture,
        };
        hidden_faces.push(hidden(render_type, face_alpha, texture));
    }

    let mut types = Stream::at(data, layout.types);
    let mut indices = Stream::at(data, layout.indices);
    let mut faces = Vec::with_capacity(layout.faces);
    let (mut a, mut b, mut c, mut last) = (0i32, 0i32, 0i32, 0i32);
    for _ in 0..layout.faces {
        match types.u8()? {
            1 => {
                a = indices.smart()?.wrapping_add(last);
                b = indices.smart()?.wrapping_add(a);
                c = indices.smart()?.wrapping_add(b);
            }
            2 => {
                b = c;
                c = indices.smart()?.wrapping_add(last);
            }
            3 => {
                a = c;
                c = indices.smart()?.wrapping_add(last);
            }
            4 => {
                std::mem::swap(&mut a, &mut b);
                c = indices.smart()?.wrapping_add(last);
            }
            _ => {
                faces.push([0, 0, 0]);
                continue;
            }
        }
        last = c;
        faces.push([a, b, c]);
    }
    let vertex = |v: i32| usize::try_from(v).ok().filter(|&v| v < layout.vertices);
    let faces = faces
        .into_iter()
        .map(|[a, b, c]| Some([vertex(a)?, vertex(b)?, vertex(c)?]))
        .collect::<Option<Vec<_>>>()?;

    let mut kinds = layout.texture_kinds.map(|at| Stream::at(data, at));
    let mut texture_indices = Stream::at(data, layout.texture_indices);
    let mut texture_vertices = Vec::new();
    for _ in 0..layout.texture_count {
        let kind = match kinds.as_mut() {
            Some(stream) => stream.u8()?,
            None => 0,
        };
        if kind == 0 {
            texture_vertices.push([
                texture_indices.u16()? as usize,
                texture_indices.u16()? as usize,
                texture_indices.u16()? as usize,
            ]);
        }
    }

    Some(ModelData {
        vertices,
        faces,
        hidden: hidden_faces,
        labels,
        texture_vertices,
    })
}

fn copy_vertex(
    out: &mut ModelData,
    seen: &mut HashMap<[i32; 3], usize>,
    model: &ModelData,
    vertex: usize,
) -> usize {
    let point = model.vertices[vertex];
    *seen.entry(point).or_insert_with(|| {
        out.vertices.push(point);
        out.labels.push(model.labels[vertex].max(0));
        out.vertices.len() - 1
    })
}

pub fn merge(models: &[ModelData]) -> ModelData {
    let mut out = ModelData::default();
    let mut seen = HashMap::new();
    for model in models {
        for (face, &hidden) in model.faces.iter().zip(&model.hidden) {
            let mapped = face.map(|v| copy_vertex(&mut out, &mut seen, model, v));
            out.faces.push(mapped);
            out.hidden.push(hidden);
        }
        for triangle in &model.texture_vertices {
            if triangle.iter().all(|&v| v < model.vertices.len()) {
                let mapped = triangle.map(|v| copy_vertex(&mut out, &mut seen, model, v));
                out.texture_vertices.push(mapped);
            }
        }
    }
    out
}

pub fn decode_frame_map(data: &[u8]) -> Option<FrameMap> {
    let mut r = Stream::at(data, 0);
    let count = r.u8()? as usize;
    let types = (0..count).map(|_| r.u8()).collect::<Option<Vec<_>>>()?;
    let lengths = (0..count).map(|_| r.u8()).collect::<Option<Vec<_>>>()?;
    let labels = lengths
        .into_iter()
        .map(|n| (0..n).map(|_| r.u8()).collect::<Option<Vec<_>>>())
        .collect::<Option<Vec<_>>>()?;
    Some(FrameMap { types, labels })
}

pub fn frame_map_id(frame: &[u8]) -> Option<u32> {
    Stream::at(frame, 0).u16().map(u32::from)
}

pub fn decode_frame(data: &[u8], map: &FrameMap) -> Option<Frame> {
    let count = *data.get(2)? as usize;
    let mut flags = Stream::at(data, 3);
    let mut values = Stream::at(data, 3 + count);
    let mut transforms = Vec::new();
    let mut first_free = 0;
    for transform in 0..count {
        let flag = flags.u8()?;
        if flag == 0 {
            continue;
        }
        let kind = *map.types.get(transform)?;
        if kind != 0
            && let Some(origin) = (first_free..transform).rev().find(|&t| map.types[t] == 0)
        {
            transforms.push((origin, [0; 3]));
        }
        let default = if kind == 3 { 128 } else { 0 };
        let mut delta = [default; 3];
        for (axis, value) in delta.iter_mut().enumerate() {
            if flag & (1 << axis) != 0 {
                *value = values.smart()?;
            }
        }
        transforms.push((transform, delta));
        first_free = transform + 1;
    }
    (values.at == data.len()).then_some(Frame { transforms })
}

impl ModelData {
    fn groups(&self) -> Vec<Vec<usize>> {
        let size = self.labels.iter().copied().max().map_or(0, |max| max + 1);
        let mut groups = vec![Vec::new(); size.max(0) as usize];
        for (vertex, &label) in self.labels.iter().enumerate() {
            if let Some(group) = usize::try_from(label).ok().and_then(|l| groups.get_mut(l)) {
                group.push(vertex);
            }
        }
        groups
    }

    pub fn animate(&mut self, map: &FrameMap, frame: &Frame) {
        if !self.labels.iter().any(|&label| label >= 0) {
            return;
        }
        let groups = self.groups();
        let mut origin = [0i32; 3];
        for &(transform, delta) in &frame.transforms {
            if let (Some(&kind), Some(labels)) =
                (map.types.get(transform), map.labels.get(transform))
            {
                let members = labels
                    .iter()
                    .filter_map(|&label| groups.get(label as usize))
                    .flatten()
                    .copied();
                self.transform(kind, members, delta, &mut origin);
            }
        }
    }

    fn transform(
        &mut self,
        kind: u8,
        members: impl Iterator<Item = usize>,
        delta: [i32; 3],
        origin: &mut [i32; 3],
    ) {
        match kind {
            0 => {
                let mut sum = [0i32; 3];
                let mut count = 0i32;
                for vertex in members {
                    for (total, value) in sum.iter_mut().zip(self.vertices[vertex]) {
                        *total = total.wrapping_add(value);
                    }
                    count += 1;
                }
                for axis in 0..3 {
                    let center = if count > 0 { sum[axis] / count } else { 0 };
                    origin[axis] = center.wrapping_add(delta[axis]);
                }
            }
            1 => {
                for vertex in members {
                    let point = &mut self.vertices[vertex];
                    for (value, d) in point.iter_mut().zip(delta) {
                        *value = value.wrapping_add(d);
                    }
                }
            }
            2 => {
                let [pitch, yaw, roll] = delta.map(|d| ((d & 0xff) * 8) as usize);
                for vertex in members {
                    let point = &mut self.vertices[vertex];
                    let [mut x, mut y, mut z] = [0, 1, 2].map(|a| point[a].wrapping_sub(origin[a]));
                    if roll != 0 {
                        let (sin, cos) = TRIG[roll];
                        let rotated = y.wrapping_mul(sin).wrapping_add(x.wrapping_mul(cos)) >> 16;
                        y = y.wrapping_mul(cos).wrapping_sub(x.wrapping_mul(sin)) >> 16;
                        x = rotated;
                    }
                    if pitch != 0 {
                        let (sin, cos) = TRIG[pitch];
                        let rotated = y.wrapping_mul(cos).wrapping_sub(z.wrapping_mul(sin)) >> 16;
                        z = y.wrapping_mul(sin).wrapping_add(z.wrapping_mul(cos)) >> 16;
                        y = rotated;
                    }
                    if yaw != 0 {
                        let (sin, cos) = TRIG[yaw];
                        let rotated = z.wrapping_mul(sin).wrapping_add(x.wrapping_mul(cos)) >> 16;
                        z = z.wrapping_mul(cos).wrapping_sub(x.wrapping_mul(sin)) >> 16;
                        x = rotated;
                    }
                    *point = [x, y, z];
                    for (value, o) in point.iter_mut().zip(*origin) {
                        *value = value.wrapping_add(o);
                    }
                }
            }
            3 => {
                for vertex in members {
                    let point = &mut self.vertices[vertex];
                    for axis in 0..3 {
                        let local = point[axis].wrapping_sub(origin[axis]);
                        point[axis] =
                            (delta[axis].wrapping_mul(local) / 128).wrapping_add(origin[axis]);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn scale(&mut self, x: i32, y: i32, z: i32) {
        for point in &mut self.vertices {
            for (value, s) in point.iter_mut().zip([x, y, z]) {
                *value = value.wrapping_mul(s) / 128;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry() -> [Vec<u8>; 6] {
        [
            vec![0, 1, 7],
            vec![1, 2],
            vec![64, 65, 65, 62],
            vec![0xc0, 0x64, 0xbf, 0x9c],
            vec![14],
            vec![84],
        ]
    }

    fn triangle() -> Vec<[i32; 3]> {
        vec![[0, 0, 0], [100, 0, 0], [0, -50, 20]]
    }

    #[test]
    fn decodes_the_original_format() {
        let [flags, types, indices, x, y, z] = geometry();
        let mut data = [flags, types, vec![1, 2, 2], vec![0, 0xff], indices].concat();
        data.extend_from_slice(&[0x12, 0x34, 0x56, 0x78]);
        data.extend([x, y, z].concat());
        data.extend_from_slice(&[0, 3, 0, 2, 0, 0, 0, 1, 0, 1, 0, 4, 0, 1, 0, 1, 0, 4]);
        let model = decode_model(&data).unwrap();
        assert_eq!(model.vertices, triangle());
        assert_eq!(model.faces, vec![[0, 1, 2], [0, 2, 0]]);
        assert_eq!(model.hidden, vec![false, true]);
        assert_eq!(model.labels, vec![1, 2, 2]);
    }

    #[test]
    fn decodes_the_textured_format() {
        let [flags, types, indices, x, y, z] = geometry();
        let mut data = [flags, vec![0, 2], types, indices, vec![0, 5, 0, 0]].concat();
        data.extend_from_slice(&[0x12, 0x34, 0x56, 0x78]);
        data.extend([x, y, z].concat());
        data.extend_from_slice(&[0, 3, 0, 2, 0, 1, 0, 0, 0, 1, 0]);
        data.extend_from_slice(&[0, 4, 0, 1, 0, 1, 0, 4, 0, 0, 0xff, 0xff]);
        let model = decode_model(&data).unwrap();
        assert_eq!(model.vertices, triangle());
        assert_eq!(model.faces, vec![[0, 1, 2], [0, 2, 0]]);
        assert_eq!(model.hidden, vec![false, true]);
        assert_eq!(model.labels, vec![-1, -1, -1]);
        assert_eq!(decode_model(&data[..data.len() - 3]), None);
    }

    #[test]
    fn hides_faces_the_client_never_draws() {
        assert!(!hidden(0, 0, -1));
        assert!(!hidden(1, 0, -1));
        assert!(!hidden(3, 0, -1));
        assert!(hidden(2, 0, -1));
        assert!(hidden(0, -1, -1));
        assert!(!hidden(0, -2, -1));
        assert!(hidden(0, -2, 4));
        assert!(hidden(3, 0, 4));
        assert!(!hidden(1, 0, 4));
    }

    fn model() -> ModelData {
        ModelData {
            vertices: triangle(),
            faces: vec![[0, 1, 2]],
            hidden: vec![false],
            labels: vec![1, 2, 2],
            texture_vertices: Vec::new(),
        }
    }

    fn map() -> FrameMap {
        decode_frame_map(&[4, 0, 1, 2, 3, 1, 1, 1, 1, 2, 2, 2, 2]).unwrap()
    }

    #[test]
    fn translates_a_label_group() {
        let frame_data = [0, 7, 2, 0, 3, 74, 59];
        assert_eq!(frame_map_id(&frame_data), Some(7));
        let frame = decode_frame(&frame_data, &map()).unwrap();
        assert_eq!(frame.transforms, vec![(0, [0, 0, 0]), (1, [10, -5, 0])]);
        let mut model = model();
        model.animate(&map(), &frame);
        assert_eq!(model.vertices, vec![[0, 0, 0], [110, -5, 0], [10, -55, 20]]);
        assert_eq!(decode_frame(&frame_data[..6], &map()), None);
        assert_eq!(
            decode_frame(&[0, 7, 9, 1, 1, 1, 1, 1, 1, 1, 1, 1], &map()),
            None
        );
    }

    #[test]
    fn rotates_around_the_group_origin() {
        let frame = decode_frame(&[0, 7, 3, 0, 0, 4, 0xc0, 0x40], &map()).unwrap();
        assert_eq!(frame.transforms, vec![(0, [0, 0, 0]), (2, [0, 0, 64])]);
        let mut model = model();
        model.animate(&map(), &frame);
        assert_eq!(model.vertices, vec![[0, 0, 0], [74, -75, 0], [25, 24, 20]]);
    }

    #[test]
    fn scales_around_the_group_origin_with_default_factors() {
        let frame = decode_frame(&[0, 7, 4, 0, 0, 0, 1, 0xc1, 0x00], &map()).unwrap();
        assert_eq!(frame.transforms, vec![(0, [0, 0, 0]), (3, [256, 128, 128])]);
        let mut model = model();
        model.animate(&map(), &frame);
        assert_eq!(model.vertices, vec![[0, 0, 0], [150, 0, 0], [-50, -50, 20]]);
    }

    #[test]
    fn a_model_without_labels_does_not_animate() {
        let frame = decode_frame(&[0, 7, 2, 0, 3, 74, 59], &map()).unwrap();
        let mut model = model();
        model.labels = vec![-1; 3];
        model.animate(&map(), &frame);
        assert_eq!(model.vertices, triangle());
    }

    #[test]
    fn scales_toward_zero() {
        let mut model = model();
        model.vertices = vec![[100, -50, -3]];
        model.scale(64, 192, 64);
        assert_eq!(model.vertices, vec![[50, -75, -1]]);
    }

    #[test]
    fn merges_shared_vertices_like_the_client() {
        let mut other = model();
        other.vertices = vec![[5, 5, 5], [100, 0, 0], [0, 0, 0], [9, 9, 9]];
        other.faces = vec![[1, 0, 2]];
        other.labels = vec![-1; 4];
        other.hidden = vec![true];
        let merged = merge(&[model(), other]);
        assert_eq!(
            merged.vertices,
            vec![[0, 0, 0], [100, 0, 0], [0, -50, 20], [5, 5, 5]]
        );
        assert_eq!(merged.labels, vec![1, 2, 2, 0]);
        assert_eq!(merged.faces, vec![[0, 1, 2], [1, 3, 0]]);
        assert_eq!(merged.hidden, vec![false, true]);
    }

    #[test]
    fn decodes_every_installed_model() {
        let Some(dir) = std::env::var_os("VOLCREN_CACHE_DIR") else {
            eprintln!("skipped, VOLCREN_CACHE_DIR is not set");
            return;
        };
        let store = crate::js5::Store::open(std::path::Path::new(&dir)).unwrap();
        let reference = store.reference(7).unwrap();
        let mut stored = 0;
        let mut failed = Vec::new();
        for &(id, _) in &reference.groups {
            let Some(data) = store.group(7, id) else {
                continue;
            };
            stored += 1;
            if decode_model(&data).is_none() {
                failed.push(id);
            }
        }
        println!(
            "{} models listed, {stored} stored, {} decoded, failed {:?}",
            reference.groups.len(),
            stored - failed.len(),
            &failed[..failed.len().min(20)]
        );
        assert!(failed.is_empty());
    }
}
