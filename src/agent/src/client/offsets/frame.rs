use super::*;

/// The game thread's frame. `tick(app, f32)` runs the update steps, then the draw step
/// `app->vtable[draw_slot](app, f32)` when the renderer takes a frame, then presents when the
/// client draws on its own thread. When the client hands frames to a render thread, the
/// present (and so the swap) runs there instead, but `tick` and the draw step stay on the
/// game thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Frame {
    pub tick: u32,
    pub draw_slot: u32,
}

const IMPORT_DIRECTORY: usize = 112 + 8;
const IMPORT_DESCRIPTOR: usize = 20;

fn c_str(image: &[u8], at: usize) -> Option<&[u8]> {
    let bytes = image.get(at..)?;
    Some(&bytes[..bytes.iter().position(|&b| b == 0)?])
}

fn import_slot(pe: &Pe, dll: &str, function: &str) -> Option<u32> {
    let image = pe.image;
    let nt = u32_at(image, 0x3c)? as usize;
    let mut descriptor = u32_at(image, nt + 24 + IMPORT_DIRECTORY)? as usize;
    if descriptor == 0 {
        return None;
    }
    loop {
        let names = u32_at(image, descriptor)? as usize;
        let name = u32_at(image, descriptor + 12)? as usize;
        let slots = u32_at(image, descriptor + 16)? as usize;
        if name == 0 {
            return None;
        }
        if names != 0 && c_str(image, name)?.eq_ignore_ascii_case(dll.as_bytes()) {
            for i in 0.. {
                let thunk = u64_at(image, names + i * 8)?;
                if thunk == 0 {
                    break;
                }
                if thunk >> 63 == 0 && c_str(image, thunk as usize + 2)? == function.as_bytes() {
                    return Some((slots + i * 8) as u32);
                }
            }
        }
        descriptor += IMPORT_DESCRIPTOR;
    }
}

fn import_calls(pe: &Pe, slot: u32) -> Vec<u32> {
    pe.code_sections()
        .flat_map(|(rva, data)| {
            data.windows(6)
                .enumerate()
                .filter(|(_, w)| w[0] == 0xff && w[1] == 0x15)
                .filter_map(move |(offset, w)| {
                    let site = (rva + offset) as i64;
                    let target = site + 6 + i32::from_le_bytes(w[2..6].try_into().ok()?) as i64;
                    (target == slot as i64).then_some(site as u32)
                })
        })
        .collect()
}

fn virtual_slot(i: &Instruction) -> Option<u32> {
    (i.mnemonic() == Mnemonic::Call
        && i.op0_kind() == OpKind::Memory
        && i.memory_index() == Register::None
        && !matches!(i.memory_base(), Register::None | Register::RIP))
    .then(|| i.memory_displacement64() as u32)
}

const FLOAT_ARGUMENT_WINDOW: usize = 12;

fn passes_float(instrs: &[Instruction], call: usize) -> bool {
    instrs[call.saturating_sub(FLOAT_ARGUMENT_WINDOW)..call]
        .iter()
        .any(|i| i.op0_kind() == OpKind::Register && i.op0_register() == Register::XMM1)
}

/// The update call is `vtable[d]` and the draw call after it is `vtable[d + 8]`, which passes
/// the frame's float in `xmm1`.
fn draw_slot(instrs: &[Instruction]) -> Option<u32> {
    let slots: Vec<(usize, u32)> = instrs
        .iter()
        .enumerate()
        .filter_map(|(n, i)| Some((n, virtual_slot(i)?)))
        .collect();
    let mut pairs: Vec<u32> = slots
        .iter()
        .enumerate()
        .filter_map(|(k, &(_, update))| {
            let draw = update.checked_add(8)?;
            slots[k + 1..]
                .iter()
                .any(|&(n, slot)| slot == draw && passes_float(instrs, n))
                .then_some(draw)
        })
        .collect();
    pairs.sort_unstable();
    pairs.dedup();
    (pairs.len() == 1).then(|| pairs[0])
}

pub fn find_frame(pe: &Pe) -> Result<Frame, ScanError> {
    let slot = import_slot(pe, "GDI32.dll", "SwapBuffers")
        .or_else(|| import_slot(pe, "OPENGL32.dll", "wglSwapBuffers"))
        .ok_or(ScanError::NotFound)?;
    let present = one(import_calls(pe, slot)
        .into_iter()
        .filter_map(|site| pe.function_containing(site))
        .filter_map(|(begin, _)| chunk_root(pe, begin))
        .collect())?;
    let (tick, draw_slot) = one(callers(pe, present)
        .filter_map(|caller| Some((chunk_root(pe, caller.begin)?, draw_slot(&caller.instrs)?)))
        .collect())?;
    Ok(Frame { tick, draw_slot })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_the_update_and_draw_slots() {
        // call [rax+0F0h]; call [rbx+18h]; movaps xmm1,xmm6; call [rax+0F8h]
        let code = [
            0xff, 0x90, 0xf0, 0x00, 0x00, 0x00, 0xff, 0x53, 0x18, 0x0f, 0x28, 0xce, 0xff, 0x90,
            0xf8, 0x00, 0x00, 0x00,
        ];
        assert_eq!(draw_slot(&decode(&code, 0x1000)), Some(0xf8));
        assert_eq!(draw_slot(&decode(&code[..9], 0x1000)), None);
        // call [rax+10h]; call [rax+18h]: no float passed
        let unrelated = [0xff, 0x50, 0x10, 0xff, 0x50, 0x18];
        assert_eq!(draw_slot(&decode(&unrelated, 0x1000)), None);
    }

    #[test]
    fn finds_frame_in_installed_client() {
        let Some(path) = std::env::var_os("GAME_CLIENT_EXE") else {
            eprintln!("skipped, GAME_CLIENT_EXE isn't set");
            return;
        };
        let Ok(file) = std::fs::read(&path) else {
            eprintln!("skipped, cannot read {}", path.to_string_lossy());
            return;
        };
        let image = super::super::tests::map_image(&file);
        let pe = Pe::parse(&image).unwrap();
        let frame = find_frame(&pe).expect("frame not found");
        println!("{frame:x?}");
        assert_eq!(calls_to(&pe, frame.tick).count(), 1);
    }
}
