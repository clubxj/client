use super::*;

const NPC_ON: &str = "highlightNpcOn";
const NPC_OFF: &str = "highlightNpcOff";
const THUNK_REACH: u32 = 0x60;
const THUNK_LIMIT: usize = 40;
const LEA_RAX: [u8; 3] = [0x48, 0x8d, 0x05];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Highlights {
    pub client: u32,
    pub manager: u32,
    pub npc_on: u32,
    pub npc_off: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Binding {
    client: u32,
    manager: u32,
    function: u32,
}

fn thunk(pe: &Pe, site: u32) -> Option<u32> {
    let from = site.checked_sub(THUNK_REACH)?;
    (from..site).rev().find_map(|at| {
        let bytes = pe.image.get(at as usize..at as usize + 7)?;
        (bytes[..3] == LEA_RAX).then(|| {
            (at as i64 + 7 + i32::from_le_bytes(bytes[3..7].try_into().unwrap()) as i64) as u32
        })
    })
}

fn binding_at(pe: &Pe, thunk: u32) -> Option<Binding> {
    let instrs = decode_at(pe, thunk, THUNK_LIMIT);
    let load = instrs.iter().position(|i| {
        i.mnemonic() == Mnemonic::Mov
            && i.op0_kind() == OpKind::Register
            && i.op0_register() == Register::RCX
            && i.is_ip_rel_memory_operand()
    })?;
    let client = instrs[load].ip_rel_memory_address() as u32;
    let rest = &instrs[load + 1..];
    let add = rest.iter().position(|i| {
        i.mnemonic() == Mnemonic::Add
            && i.op0_kind() == OpKind::Register
            && i.op0_register() == Register::RCX
            && matches!(i.op1_kind(), OpKind::Immediate32to64 | OpKind::Immediate8to64)
    })?;
    let manager = rest[add].immediate(1) as u32;
    let function = rest[add + 1..].iter().find_map(call_target)?;
    Some(Binding {
        client,
        manager,
        function,
    })
}

fn binding(pe: &Pe, name: &str) -> Result<Binding, ScanError> {
    let strings = string_rvas(pe, name);
    let sites = rdx_leas(pe)
        .iter()
        .filter(|(_, target)| strings.contains(target))
        .filter_map(|&(site, _)| binding_at(pe, thunk(pe, site)?))
        .collect();
    one(sites)
}

pub fn find_highlights(pe: &Pe) -> Result<Highlights, ScanError> {
    let on = binding(pe, NPC_ON)?;
    let off = binding(pe, NPC_OFF)?;
    if (on.client, on.manager) != (off.client, off.manager) || on.function == off.function {
        return Err(ScanError::NotFound);
    }
    Ok(Highlights {
        client: on.client,
        manager: on.manager,
        npc_on: on.function,
        npc_off: off.function,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::{for_each_archived_build, found};
    use super::*;

    #[test]
    fn finds_highlights_in_archived_builds() {
        for_each_archived_build(|build, pe| {
            let highlights = found("highlights", build, find_highlights(pe));
            if build.to_string_lossy().contains("c65749a2") {
                assert_eq!(
                    highlights,
                    Highlights {
                        client: 0xe932a0,
                        manager: 0x896c0,
                        npc_on: 0x329640,
                        npc_off: 0x32b540,
                    },
                    "{}",
                    build.display()
                );
            }
        });
    }
}
