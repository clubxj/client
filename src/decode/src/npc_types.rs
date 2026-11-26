use crate::definitions::{Reader, Transforms};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpcType {
    pub models: Vec<u32>,
    pub size: u8,
    pub width_scale: i32,
    pub height_scale: i32,
    pub transforms: Option<Transforms>,
    pub actions: Vec<String>,
}

impl Default for NpcType {
    fn default() -> Self {
        Self {
            models: Vec::new(),
            size: 1,
            width_scale: 128,
            height_scale: 128,
            transforms: None,
            actions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sequence {
    pub frame_ids: Vec<u32>,
    pub skeletal: bool,
}

fn u32(r: &mut Reader) -> Option<u32> {
    Some(u32::from(r.u16()?) << 16 | u32::from(r.u16()?))
}

fn params(r: &mut Reader) -> Option<()> {
    let n = r.u8()?;
    for _ in 0..n {
        let kind = r.u8()?;
        r.skip(3)?;
        match kind {
            1 => r.string()?,
            2 => r.skip(8)?,
            _ => r.skip(4)?,
        }
    }
    Some(())
}

pub fn decode_npc(data: &[u8]) -> Option<NpcType> {
    decode_npc_with_idle(data).map(|(npc, _)| npc)
}

fn decode_npc_with_idle(data: &[u8]) -> Option<(NpcType, Option<u16>)> {
    let mut npc = NpcType::default();
    let mut idle = None;
    let mut r = Reader::new(data);
    loop {
        match r.u8()? {
            0 => return Some((npc, idle)),
            2 => r.string()?,
            30..=34 => {
                let action = r.text()?;
                if !action.eq_ignore_ascii_case("hidden") {
                    npc.actions.push(action);
                }
            }
            12 => npc.size = r.u8()?,
            13 => idle = Some(r.u16()?),
            14..=16 | 18 | 42 | 74..=79 | 95 | 103 | 114 | 116 | 124 | 126 | 146 => r.skip(2)?,
            17 | 115 | 117 => r.skip(8)?,
            40 | 41 => {
                let n = r.u8()? as usize;
                r.skip(n * 4)?;
            }
            61 => {
                let n = r.u8()?;
                npc.models = (0..n).map(|_| u32(&mut r)).collect::<Option<_>>()?;
            }
            62 => {
                let n = r.u8()? as usize;
                r.skip(n * 4)?;
            }
            93 | 99 | 107 | 109 | 111 | 122 | 123 | 130 | 145 | 147 => {}
            97 => npc.width_scale = i32::from(r.u16()?),
            98 => npc.height_scale = i32::from(r.u16()?),
            100 | 101 | 149 | 151 => r.skip(1)?,
            102 => {
                let mask = r.u8()?;
                for bit in 0..8 - mask.leading_zeros() {
                    if mask & 1 << bit != 0 {
                        let wide = r.u8()? & 0x80 != 0;
                        r.skip(if wide { 3 } else { 1 })?;
                        r.ushort_smart()?;
                    }
                }
            }
            op @ (106 | 118) => {
                let id = |v: u16| (v != 0xffff).then_some(v);
                let varbit = id(r.u16()?);
                let varp = id(r.u16()?);
                let default = if op == 118 {
                    id(r.u16()?).map(u32::from)
                } else {
                    None
                };
                let n = r.u8()? as usize;
                let mut ids = Vec::with_capacity(n + 2);
                for _ in 0..=n {
                    ids.push(id(r.u16()?).map(u32::from));
                }
                ids.push(default);
                npc.transforms = Some(Transforms { varbit, varp, ids });
            }
            148 => r.skip(4)?,
            150 => r.skip(6)?,
            152 => {
                r.skip(6)?;
                let n = r.u8()? as usize;
                r.skip(n * 2)?;
            }
            249 => params(&mut r)?,
            251 => {
                r.skip(2)?;
                r.string()?;
            }
            252 => {
                r.skip(13)?;
                r.string()?;
            }
            253 => {
                r.skip(15)?;
                r.string()?;
            }
            _ => return None,
        }
    }
}

pub fn decode_sequence(data: &[u8]) -> Option<Sequence> {
    let mut sequence = Sequence::default();
    let mut r = Reader::new(data);
    loop {
        match r.u8()? {
            0 => return Some(sequence),
            1 => {
                let n = r.u16()? as usize;
                r.skip(n * 2)?;
                let low = (0..n).map(|_| r.u16()).collect::<Option<Vec<_>>>()?;
                sequence.frame_ids = low
                    .into_iter()
                    .map(|low| Some(u32::from(low).wrapping_add(u32::from(r.u16()?) << 16)))
                    .collect::<Option<_>>()?;
            }
            2 | 6 | 7 => r.skip(2)?,
            3 | 17 => {
                let n = r.u8()? as usize;
                r.skip(n)?;
            }
            4 | 19 => {}
            5 | 8..=11 | 16 => r.skip(1)?,
            12 => {
                let n = r.u8()? as usize;
                r.skip(n * 4)?;
            }
            13 => sequence.skeletal = (u32(&mut r)? as i32) >= 0,
            14 => {
                let n = r.u16()? as usize;
                r.skip(n * 8)?;
            }
            15 => r.skip(4)?,
            _ => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_data::{self, ModelData};
    use std::collections::HashMap;

    #[test]
    fn options_keep_their_order_and_hidden_ones_are_left_out() {
        let mut data = Vec::new();
        for (opcode, text) in [(30u8, "Talk-to"), (31, "Hidden"), (33, "Charter")] {
            data.push(opcode);
            data.extend_from_slice(text.as_bytes());
            data.push(0);
        }
        data.push(0);
        assert_eq!(decode_npc(&data).unwrap().actions, ["Talk-to", "Charter"]);
    }

    #[test]
    fn npcs_default_like_the_client() {
        assert_eq!(decode_npc(&[0]), Some(NpcType::default()));
        assert_eq!(
            decode_npc(&[0]).map(|n| (n.size, n.width_scale, n.height_scale)),
            Some((1, 128, 128))
        );
        assert_eq!(decode_npc(&[1, 0]), None);
        assert_eq!(decode_npc(&[12]), None);
    }

    #[test]
    fn decodes_npc_models_scales_and_transforms() {
        let mut data = vec![2];
        data.extend_from_slice(b"Guard\0");
        data.extend_from_slice(&[12, 2, 61, 2, 0, 0, 0x1d, 0x4c, 0, 1, 0, 0]);
        data.extend_from_slice(&[13, 0, 0x2f, 30]);
        data.extend_from_slice(b"Attack\0");
        data.extend_from_slice(&[
            97, 0, 64, 98, 0, 200, 102, 0b101, 0x80, 0, 1, 0x2c, 3, 0, 5, 2,
        ]);
        data.extend_from_slice(&[118, 0x01, 0x02, 0xff, 0xff, 0x00, 0x09, 1, 0, 5, 0xff, 0xff]);
        data.extend_from_slice(&[249, 2, 1, 0, 0, 1]);
        data.extend_from_slice(b"x\0");
        data.extend_from_slice(&[2, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 1, 0]);
        let (npc, idle) = decode_npc_with_idle(&data).unwrap();
        assert_eq!(idle, Some(0x2f));
        assert_eq!(npc.models, vec![7500, 65536]);
        assert_eq!((npc.size, npc.width_scale, npc.height_scale), (2, 64, 200));
        let transforms = npc.transforms.unwrap();
        assert_eq!((transforms.varbit, transforms.varp), (Some(0x0102), None));
        assert_eq!(transforms.pick(Some(0)), Some(5));
        assert_eq!(transforms.pick(Some(1)), None);
        assert_eq!(transforms.pick(Some(4)), Some(9));
    }

    #[test]
    fn decodes_sequence_frame_ids() {
        let data = [
            1, 0, 2, 0, 4, 0, 5, 0x00, 0x10, 0x00, 0x11, 0x03, 0x2a, 0x03, 0x2a, 5, 10, 14, 0, 1,
            0, 1, 0, 2, 1, 1, 1, 1, 0,
        ];
        assert_eq!(
            decode_sequence(&data),
            Some(Sequence {
                frame_ids: vec![0x032a_0010, 0x032a_0011],
                skeletal: false
            })
        );
        assert_eq!(
            decode_sequence(&[13, 0, 0, 0, 9, 0]),
            Some(Sequence {
                frame_ids: Vec::new(),
                skeletal: true
            })
        );
        assert_eq!(
            decode_sequence(&[13, 0xff, 0xff, 0xff, 0xff, 0]).map(|s| s.skeletal),
            Some(false)
        );
        assert_eq!(decode_sequence(&[18, 0]), None);
    }

    fn npc_model(store: &crate::js5::Store, npc: &NpcType) -> Option<ModelData> {
        let models = npc
            .models
            .iter()
            .map(|&id| model_data::decode_model(&store.group(7, id)?))
            .collect::<Option<Vec<_>>>()?;
        Some(match models.as_slice() {
            [model] => model.clone(),
            _ => model_data::merge(&models),
        })
    }

    #[test]
    fn decodes_every_installed_npc_and_sequence() {
        let Some(dir) = std::env::var_os("VOLCREN_CACHE_DIR") else {
            eprintln!("skipped, VOLCREN_CACHE_DIR is not set");
            return;
        };
        let store = crate::js5::Store::open(std::path::Path::new(&dir)).unwrap();
        let npc_files = store.files(2, 9).unwrap();
        let npcs: Vec<_> = npc_files
            .iter()
            .map(|(id, data)| (*id, decode_npc_with_idle(data)))
            .collect();
        let npc_failed: Vec<u32> = npcs
            .iter()
            .filter(|(_, npc)| npc.is_none())
            .map(|(id, _)| *id)
            .collect();
        let sequence_files = store.files(2, 12).unwrap();
        let sequences: HashMap<u32, Option<Sequence>> = sequence_files
            .iter()
            .map(|(id, data)| (*id, decode_sequence(data)))
            .collect();
        let mut sequence_failed: Vec<u32> = sequences
            .iter()
            .filter(|(_, s)| s.is_none())
            .map(|(id, _)| *id)
            .collect();
        sequence_failed.sort();
        let skeletal = sequences.values().flatten().filter(|s| s.skeletal).count();
        println!(
            "{} npcs, {} decoded, failed {:?}; {} sequences, {} decoded, {skeletal} skeletal, failed {:?}",
            npc_files.len(),
            npc_files.len() - npc_failed.len(),
            &npc_failed[..npc_failed.len().min(20)],
            sequence_files.len(),
            sequence_files.len() - sequence_failed.len(),
            &sequence_failed[..sequence_failed.len().min(20)]
        );

        let mut users: Vec<(u32, &NpcType)> = Vec::new();
        for (_, npc) in &npcs {
            let Some((npc, Some(idle))) = npc else {
                continue;
            };
            let idle = u32::from(*idle);
            let usable = sequences
                .get(&idle)
                .and_then(Option::as_ref)
                .is_some_and(|s| !s.skeletal && !s.frame_ids.is_empty());
            if usable && !npc.models.is_empty() && !users.iter().any(|(s, _)| *s == idle) {
                users.push((idle, npc));
            }
        }
        users.sort_by_key(|(s, _)| *s);
        let mut groups = HashMap::new();
        let (mut sampled, mut frames, mut decoded, mut moved) = (0, 0, 0, 0);
        for &(sequence, npc) in users.iter().step_by(50) {
            let Some(model) = npc_model(&store, npc) else {
                continue;
            };
            sampled += 1;
            for &frame_id in &sequences[&sequence].as_ref().unwrap().frame_ids {
                frames += 1;
                let files = groups
                    .entry(frame_id >> 16)
                    .or_insert_with(|| store.files(0, frame_id >> 16));
                let Some(data) = files
                    .as_ref()
                    .and_then(|f| f.iter().find(|(id, _)| *id == frame_id & 0xffff))
                    .map(|(_, data)| data)
                else {
                    continue;
                };
                let Some(map) = model_data::frame_map_id(data)
                    .and_then(|id| store.group(1, id))
                    .and_then(|data| model_data::decode_frame_map(&data))
                else {
                    continue;
                };
                let Some(frame) = model_data::decode_frame(data, &map) else {
                    continue;
                };
                decoded += 1;
                let mut animated = model.clone();
                animated.animate(&map, &frame);
                animated.scale(npc.width_scale, npc.height_scale, npc.width_scale);
                assert_eq!(animated.vertices.len(), model.vertices.len());
                if animated.vertices != model.vertices {
                    moved += 1;
                }
            }
        }
        println!(
            "{} idle sequences with npcs, {sampled} sampled, {frames} frames, {decoded} decoded, {moved} moved the model",
            users.len()
        );
        assert!(npc_failed.is_empty() && sequence_failed.is_empty());
        assert!(sampled > 0 && decoded == frames && moved * 10 >= decoded * 9);
    }
}
