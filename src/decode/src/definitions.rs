use serde::{Deserialize, Serialize};

const DOOR_OPTION: &str = "open";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocDef {
    pub size_x: u8,
    pub size_y: u8,
    pub interact_type: u8,
    pub blocks_projectile: bool,
    pub door: bool,
    pub decoration_height: u8,
}

impl Default for LocDef {
    fn default() -> Self {
        Self {
            size_x: 1,
            size_y: 1,
            interact_type: 2,
            blocks_projectile: true,
            door: false,
            decoration_height: 0,
        }
    }
}

pub struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }

    pub fn u8(&mut self) -> Option<u8> {
        let v = *self.data.get(self.at)?;
        self.at += 1;
        Some(v)
    }

    pub fn u16(&mut self) -> Option<u16> {
        Some(((self.u8()? as u16) << 8) | self.u8()? as u16)
    }

    pub fn u16_le(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes([self.u8()?, self.u8()?]))
    }

    pub fn skip(&mut self, n: usize) -> Option<()> {
        (self.at + n <= self.data.len()).then(|| self.at += n)
    }

    pub fn until_nul(&mut self) -> Option<&'a [u8]> {
        let rest = self.data.get(self.at..)?;
        let end = rest.iter().position(|&b| b == 0)?;
        self.at += end + 1;
        Some(&rest[..end])
    }

    pub fn string(&mut self) -> Option<()> {
        self.until_nul().map(drop)
    }

    pub fn text(&mut self) -> Option<String> {
        self.until_nul()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
    }

    pub fn ushort_smart(&mut self) -> Option<u32> {
        if *self.data.get(self.at)? < 128 {
            self.u8().map(u32::from)
        } else {
            self.u16().map(|v| v as u32 - 0x8000)
        }
    }

    pub fn uint_smart_compat(&mut self) -> Option<u32> {
        let mut total = 0u32;
        loop {
            let v = self.ushort_smart()?;
            if v != 32767 {
                return Some(total.wrapping_add(v));
            }
            total = total.wrapping_add(32767);
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocText {
    pub name: String,
    pub actions: [String; 5],
    pub transforms: Option<Transforms>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transforms {
    pub varbit: Option<u16>,
    pub varp: Option<u16>,
    pub ids: Vec<Option<u32>>,
}

impl Transforms {
    pub fn pick(&self, value: Option<i32>) -> Option<u32> {
        let last = self.ids.len().checked_sub(1)?;
        match value {
            Some(v) if v >= 0 && (v as usize) < last => self.ids[v as usize],
            _ => self.ids[last],
        }
    }
}

pub fn decode_loc(data: &[u8]) -> Option<LocDef> {
    decode_loc_with(data, None)
}

pub fn decode_loc_text(data: &[u8]) -> Option<(LocDef, LocText)> {
    let mut text = LocText::default();
    let def = decode_loc_with(data, Some(&mut text))?;
    Some((def, text))
}

fn decode_loc_with(data: &[u8], mut text: Option<&mut LocText>) -> Option<LocDef> {
    let mut def = LocDef::default();
    let mut r = Reader::new(data);
    let mut int1: Option<u8> = None;
    let mut models: Option<Option<u8>> = None;
    let mut options = [false; 5];
    let mut opens = false;
    loop {
        match r.u8()? {
            0 => {
                let interactive = match int1 {
                    Some(v) => v != 0,
                    None => {
                        models.is_some_and(|shape| shape.is_none_or(|s| s == 10))
                            || options.iter().any(|&o| o)
                    }
                };
                def.door = interactive && opens;
                return Some(def);
            }
            1 => {
                let n = r.u8()? as usize;
                if n > 0 {
                    r.skip(2)?;
                    models = Some(Some(r.u8()?));
                    r.skip((n - 1) * 3)?;
                }
            }
            2 => match text.as_deref_mut() {
                Some(text) => text.name = r.text()?,
                None => r.string()?,
            },
            5 => {
                let n = r.u8()? as usize;
                if n > 0 {
                    models = Some(None);
                }
                r.skip(n * 2)?;
            }
            6 => {
                let n = r.u8()? as usize;
                r.skip(n * 5)?;
            }
            7 | 40 | 41 => {
                let n = r.u8()? as usize;
                r.skip(n * 4)?;
            }
            14 => def.size_x = r.u8()?,
            15 => def.size_y = r.u8()?,
            17 => {
                def.interact_type = 0;
                def.blocks_projectile = false;
            }
            18 => def.blocks_projectile = false,
            19 => int1 = Some(r.u8()?),
            28 | 29 | 39 | 69 | 75 | 81 | 91 | 95 => r.skip(1)?,
            96 => def.decoration_height = r.u8()?,
            21 | 22 | 23 | 62 | 64 | 73 | 74 | 89 | 90 | 94 => {}
            24 | 61 | 65 | 66 | 67 | 68 | 70 | 71 | 72 | 82 => r.skip(2)?,
            27 => def.interact_type = 1,
            op @ 30..=34 => {
                let action = r.text()?;
                let shown = !action.eq_ignore_ascii_case("hidden");
                options[(op - 30) as usize] = shown;
                opens |= shown && action.eq_ignore_ascii_case(DOOR_OPTION);
                if let Some(text) = text.as_deref_mut() {
                    text.actions[(op - 30) as usize] = if shown { action } else { String::new() };
                }
            }
            op @ (77 | 92 | 97 | 98) => {
                let id = |v: u16| (v != 0xffff).then_some(v);
                let varbit = id(r.u16()?);
                let varp = id(r.u16()?);
                let long = op >= 97;
                let shown = |r: &mut Reader| -> Option<Option<u32>> {
                    Some(if long {
                        let v = (r.u8()? as u32) << 16 | r.u16()? as u32;
                        (v != 0xff_ffff).then_some(v)
                    } else {
                        id(r.u16()?).map(u32::from)
                    })
                };
                let default = if op == 92 || op == 98 {
                    shown(&mut r)?
                } else {
                    None
                };
                let n = r.u8()? as usize;
                let mut ids = Vec::with_capacity(n + 2);
                for _ in 0..=n {
                    ids.push(shown(&mut r)?);
                }
                ids.push(default);
                if let Some(text) = text.as_deref_mut() {
                    text.transforms = Some(Transforms { varbit, varp, ids });
                }
            }
            78 => r.skip(4)?,
            79 => {
                r.skip(6)?;
                let n = r.u8()? as usize;
                r.skip(n * 2)?;
            }
            93 => r.skip(6)?,
            100 => {
                r.skip(2)?;
                r.string()?;
            }
            101 => {
                r.skip(13)?;
                r.string()?;
            }
            102 => {
                r.skip(15)?;
                r.string()?;
            }
            249 => {
                let n = r.u8()? as usize;
                for _ in 0..n {
                    let is_string = r.u8()? == 1;
                    r.skip(3)?;
                    if is_string { r.string()? } else { r.skip(4)? }
                }
            }
            _ => return None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VarbitDef {
    pub varp: u16,
    pub lsb: u8,
    pub msb: u8,
}

impl VarbitDef {
    pub fn value(&self, varp: i32) -> i32 {
        let bits = (u32::from(self.msb) + 1).saturating_sub(u32::from(self.lsb));
        let mask = if bits >= 32 {
            u32::MAX
        } else {
            (1u32 << bits) - 1
        };
        ((varp as u32).checked_shr(u32::from(self.lsb)).unwrap_or(0) & mask) as i32
    }
}

pub fn decode_varbit(data: &[u8]) -> Option<VarbitDef> {
    let mut r = Reader::new(data);
    let mut def = None;
    loop {
        match r.u8()? {
            0 => return def,
            1 => {
                def = Some(VarbitDef {
                    varp: r.u16()?,
                    lsb: r.u8()?,
                    msb: r.u8()?,
                })
            }
            _ => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn decodes_loc_collision_fields() {
        let mut data = vec![2];
        data.extend_from_slice(b"Door\0");
        data.extend_from_slice(&[14, 2, 15, 3, 30]);
        data.extend_from_slice(b"Open\0");
        data.extend_from_slice(&[18, 249, 1, 0, 0, 0, 1, 0, 0, 0, 7, 0]);
        assert_eq!(
            decode_loc(&data),
            Some(LocDef {
                size_x: 2,
                size_y: 3,
                interact_type: 2,
                blocks_projectile: false,
                door: true,
                decoration_height: 0
            })
        );
        let (_, text) = decode_loc_text(&data).unwrap();
        assert_eq!(
            text,
            LocText {
                name: "Door".into(),
                actions: [
                    "Open".into(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new()
                ],
                transforms: None
            }
        );
        assert_eq!(
            decode_loc(&[17, 0]),
            Some(LocDef {
                size_x: 1,
                size_y: 1,
                interact_type: 0,
                blocks_projectile: false,
                door: false,
                decoration_height: 0
            })
        );
        assert_eq!(decode_loc(&[200, 0]), None);
    }

    #[test]
    fn a_door_is_an_interactive_loc_that_opens() {
        let door_of = |data: &[u8]| decode_loc(data).map(|d| d.door);
        assert_eq!(door_of(&[1, 1, 0, 5, 0, 0]), Some(false));
        let mut door = vec![1, 1, 0, 5, 0, 30];
        door.extend_from_slice(b"Open\0");
        door.push(0);
        assert_eq!(door_of(&door), Some(true));
        let mut search = vec![1, 1, 0, 5, 0, 30];
        search.extend_from_slice(b"Search\0");
        search.push(0);
        assert_eq!(door_of(&search), Some(false));
        let mut not = vec![19, 0, 30];
        not.extend_from_slice(b"Open\0");
        not.push(0);
        assert_eq!(door_of(&not), Some(false));
        let mut shape_ten = vec![1, 1, 0, 5, 10, 31];
        shape_ten.extend_from_slice(b"Open\0");
        shape_ten.push(0);
        assert_eq!(door_of(&shape_ten), Some(true));
    }

    #[test]
    fn decodes_transforms_with_long_ids() {
        let data = [
            97, 0xff, 0xff, 0, 116, 2, 0, 0x71, 2, 0xff, 0xff, 0xff, 0, 0x71, 3, 0,
        ];
        let (_, text) = decode_loc_text(&data).unwrap();
        let t = text.transforms.unwrap();
        assert_eq!((t.varbit, t.varp), (None, Some(116)));
        assert_eq!(t.pick(Some(0)), Some(28930));
        assert_eq!(t.pick(Some(1)), None);
        assert_eq!(t.pick(Some(2)), Some(28931));
    }

    #[test]
    fn decodes_transforms_with_long_ids_and_a_default() {
        let data = [
            98, 0x25, 0x6a, 0xff, 0xff, 0, 0x38, 0xb1, 1, 0, 0x38, 0xb1, 0, 0x88, 0x0a, 0,
        ];
        let (_, text) = decode_loc_text(&data).unwrap();
        let t = text.transforms.unwrap();
        assert_eq!((t.varbit, t.varp), (Some(0x256a), None));
        assert_eq!(t.pick(Some(1)), Some(34826));
        assert_eq!(t.pick(Some(7)), Some(14513));
    }

    #[test]
    fn decodes_and_picks_transforms() {
        let data = [92, 0, 7, 0xff, 0xff, 0, 30, 1, 0, 10, 0xff, 0xff, 0];
        let (_, text) = decode_loc_text(&data).unwrap();
        let t = text.transforms.unwrap();
        assert_eq!((t.varbit, t.varp), (Some(7), None));
        assert_eq!(t.pick(Some(0)), Some(10));
        assert_eq!(t.pick(Some(1)), None);
        assert_eq!(t.pick(Some(5)), Some(30));
        assert_eq!(t.pick(None), Some(30));
    }

    #[test]
    fn decodes_the_spellbook_varbit() {
        let def = decode_varbit(&hex("0101b7000100")).unwrap();
        assert_eq!(
            def,
            VarbitDef {
                varp: 439,
                lsb: 0,
                msb: 1
            }
        );
        assert_eq!(def.value(0b1110), 2);
    }

    #[test]
    fn varbit_masks_high_bits() {
        let def = VarbitDef {
            varp: 0,
            lsb: 4,
            msb: 31,
        };
        assert_eq!(def.value(-16), 0x0fff_ffff);
    }

    #[test]
    fn malformed_varbits_read_without_overflow() {
        let reversed = VarbitDef {
            varp: 0,
            lsb: 9,
            msb: 2,
        };
        assert_eq!(reversed.value(-1), 0);
        let past_the_word = VarbitDef {
            varp: 0,
            lsb: 40,
            msb: 255,
        };
        assert_eq!(past_the_word.value(-1), 0);
    }

    #[test]
    fn reads_the_decoration_height() {
        assert_eq!(
            decode_loc(&[96, 12, 0]).map(|d| d.decoration_height),
            Some(12)
        );
    }
}
