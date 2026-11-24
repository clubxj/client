use super::menu::method_at;
use super::*;

const STEAM_LABEL: &str = "Sign in with Steam";
const SHORT_STRING_SIZE: u64 = 0x18;
const SLOT_SCAN: u32 = 32;
const METHOD_MAX: u32 = 0x400;
const CONSTRUCTOR_MAX: u32 = 0x80;
const LEAF_MAX: u32 = 0x10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginProvider {
    pub holder: u32,
    pub provider: u32,
    pub steam_vtable: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Chain {
    holder_getter: u32,
    pair_object: u32,
    provider_getter: u32,
    steam_check: u32,
}

fn method_call(i: &Instruction, vtable: Register) -> Option<u32> {
    (i.mnemonic() == Mnemonic::Call
        && i.op0_kind() == OpKind::Memory
        && i.memory_base() == vtable
        && i.memory_index() == Register::None)
        .then(|| i.memory_displacement64() as u32)
}

fn rax_method_call(i: &Instruction) -> Option<u32> {
    method_call(i, Register::RAX)
}

fn references(i: &Instruction, global: u32) -> bool {
    i.is_ip_rel_memory_operand()
        && (global as u64..global as u64 + SHORT_STRING_SIZE).contains(&i.ip_rel_memory_address())
}

fn method_call_on(instrs: &[Instruction], at: usize, object: Register) -> Option<u32> {
    let call = &instrs[at];
    let vtable = (call.op0_kind() == OpKind::Memory).then(|| call.memory_base())?;
    let passes_object = instrs[at.saturating_sub(3)..at]
        .iter()
        .any(|i| register_move(i).is_some_and(|(to, from)| to == Register::RCX && from == object));
    let loads_vtable = instrs[at.saturating_sub(3)..at]
        .iter()
        .any(|i| qword_load(i, vtable, object) == Some(0));
    (passes_object && loads_vtable)
        .then(|| method_call(call, vtable))
        .flatten()
}

fn register_move(i: &Instruction) -> Option<(Register, Register)> {
    (i.mnemonic() == Mnemonic::Mov
        && i.op0_kind() == OpKind::Register
        && i.op1_kind() == OpKind::Register)
        .then(|| (i.op0_register(), i.op1_register()))
}

fn qword_load(i: &Instruction, to: Register, from: Register) -> Option<u32> {
    (i.mnemonic() == Mnemonic::Mov && i.op0_kind() == OpKind::Register && i.op0_register() == to)
        .then(|| memory_field(i, from))
        .flatten()
}

fn writes(i: &Instruction, reg: Register) -> bool {
    i.op0_kind() == OpKind::Register && i.op0_register().full_register() == reg
}

fn label_global(pe: &Pe) -> Result<u32, ScanError> {
    let strings = string_rvas(pe, STEAM_LABEL);
    let found = rdx_leas(pe)
        .iter()
        .filter(|(_, target)| strings.contains(target))
        .filter_map(|&(site, _)| {
            let next = decode_at(pe, site, 2);
            let [_, store] = next.as_slice() else {
                return None;
            };
            (store.op0_register() == Register::RCX)
                .then(|| rip_lea(store))
                .flatten()
        })
        .collect();
    one(found)
}

fn provider_chain(instrs: &[Instruction], client: u32, label: u32) -> Option<Chain> {
    let at = instrs.iter().position(|i| references(i, label))?;
    let (start, holder_getter, pair_object, provider_getter, provider) =
        instrs[..at].windows(5).enumerate().find_map(|(start, w)| {
            let [holder, pair, vtable, getter, keep] = w else {
                return None;
            };
            let holder_getter = rax_method_call(holder)?;
            let pair_object = qword_load(pair, Register::RCX, Register::RAX)?;
            qword_load(vtable, Register::RAX, Register::RCX).filter(|&disp| disp == 0)?;
            let provider_getter = rax_method_call(getter)?;
            let (provider, _) = register_move(keep).filter(|&(_, from)| from == Register::RAX)?;
            let loads_client = instrs[..start]
                .iter()
                .rev()
                .find(|i| writes(i, Register::RCX))
                .and_then(global_load)
                .is_some_and(|(reg, global)| reg == Register::RCX && global == client);
            loads_client.then_some((start, holder_getter, pair_object, provider_getter, provider))
        })?;
    let steam_check = (start + 5..at)
        .rev()
        .find_map(|n| method_call_on(instrs, n, provider))?;
    Some(Chain {
        holder_getter,
        pair_object,
        provider_getter,
        steam_check,
    })
}

fn installed_vtables(instrs: &[Instruction], this: Option<Register>) -> Vec<u32> {
    let mut leas: HashMap<Register, u32> = HashMap::new();
    let mut found = Vec::new();
    for i in instrs {
        if let Some(vtable) = rip_lea(i) {
            leas.insert(i.op0_register(), vtable);
            continue;
        }
        let stores_at_zero = i.mnemonic() == Mnemonic::Mov
            && i.op0_kind() == OpKind::Memory
            && i.memory_index() == Register::None
            && i.memory_displacement64() == 0
            && i.op1_kind() == OpKind::Register
            && this.is_none_or(|base| i.memory_base() == base);
        if stores_at_zero {
            found.extend(leas.get(&i.op1_register()));
        } else if i.op0_kind() == OpKind::Register {
            leas.remove(&i.op0_register());
        }
    }
    found
}

fn global_store_sites(pe: &Pe, global: u32) -> Vec<u32> {
    let mut sites = Vec::new();
    for (rva, data) in pe.code_sections() {
        for (offset, w) in data.windows(7).enumerate() {
            let [rex, 0x89, modrm, d0, d1, d2, d3] = *w else {
                continue;
            };
            if rex & 0xfb != 0x48 || modrm & 0xc7 != 0x05 {
                continue;
            }
            let site = (rva + offset) as i64;
            if site + 7 + i32::from_le_bytes([d0, d1, d2, d3]) as i64 == global as i64 {
                sites.push(site as u32);
            }
        }
    }
    sites
}

fn client_vtable(pe: &Pe, client: u32) -> Result<u32, ScanError> {
    let mut found = Vec::new();
    for (begin, end) in functions_containing(pe, global_store_sites(pe, client)) {
        let instrs = function_instrs(pe, begin, end);
        let Some(this) = instrs.iter().find_map(|i| {
            global_store(i)
                .filter(|&(_, global)| global == client)
                .map(|(reg, _)| reg)
        }) else {
            continue;
        };
        found.extend(installed_vtables(&instrs, Some(this)));
    }
    one(found)
}

fn holder_field(instrs: &[Instruction], pair_object: u32) -> Option<u32> {
    let checked = instrs.iter().find_map(|i| {
        (i.mnemonic() == Mnemonic::Cmp
            && i.memory_size() == iced_x86::MemorySize::UInt64
            && immediate(i) == Some(0))
        .then(|| memory_field_any(i))
        .flatten()
    })?;
    let returned = instrs.windows(2).rev().find_map(|w| {
        let [lea, next] = w else {
            return None;
        };
        let returns = matches!(
            next.mnemonic(),
            Mnemonic::Add | Mnemonic::Pop | Mnemonic::Ret
        );
        (lea.mnemonic() == Mnemonic::Lea && lea.op0_register() == Register::RAX && returns)
            .then(|| base_field(lea))
            .flatten()
            .map(|(_, disp)| disp as u32)
    })?;
    (returned + pair_object == checked).then_some(checked)
}

fn memory_field_any(i: &Instruction) -> Option<u32> {
    (i.op0_kind() == OpKind::Memory)
        .then(|| base_field(i))
        .flatten()
        .map(|(_, disp)| disp as u32)
}

fn leaf_getter(pe: &Pe, method: u32) -> Option<u32> {
    let [load, ret, ..] = image_function(pe.image, method, LEAF_MAX)[..] else {
        return None;
    };
    (ret.mnemonic() == Mnemonic::Ret)
        .then(|| qword_load(&load, Register::RAX, Register::RCX))
        .flatten()
}

fn returns_true(pe: &Pe, method: u32) -> bool {
    let [set, ret, ..] = image_function(pe.image, method, LEAF_MAX)[..] else {
        return false;
    };
    set.mnemonic() == Mnemonic::Mov
        && set.op0_register() == Register::AL
        && immediate(&set) == Some(1)
        && ret.mnemonic() == Mnemonic::Ret
}

fn stores_field(instrs: &[Instruction], field: u32) -> bool {
    instrs.iter().any(|i| {
        i.mnemonic() == Mnemonic::Mov
            && i.op0_kind() == OpKind::Memory
            && i.memory_size() == iced_x86::MemorySize::UInt64
            && i.op1_kind() == OpKind::Register
            && memory_field_any(i) == Some(field)
    })
}

fn method_body(pe: &Pe, method: u32) -> Option<Vec<Instruction>> {
    pe.function_containing(method)
        .is_some_and(|(begin, _)| begin == method)
        .then(|| whole_function(pe, method, METHOD_MAX))
}

pub fn find_login_provider(pe: &Pe, client: u32) -> Result<LoginProvider, ScanError> {
    let nt = u32_at(pe.image, 0x3c).ok_or(ScanError::InvalidImage)? as usize;
    let base = u64_at(pe.image, nt + 48).ok_or(ScanError::InvalidImage)?;
    let method = |vtable: u32, slot: u32| method_at(pe, base, (vtable + slot) as usize);

    let label = label_global(pe)?;
    let draws = functions_containing(
        pe,
        rdx_leas(pe)
            .iter()
            .filter(|&&(_, target)| target == label)
            .map(|&(site, _)| site),
    );
    let chain = one(draws
        .iter()
        .filter_map(|&(begin, end)| provider_chain(&function_instrs(pe, begin, end), client, label))
        .collect())?;

    let holder_getter =
        method(client_vtable(pe, client)?, chain.holder_getter).ok_or(ScanError::NotFound)? as u32;
    let getter = method_body(pe, holder_getter).ok_or(ScanError::NotFound)?;
    let holder = holder_field(&getter, chain.pair_object).ok_or(ScanError::NotFound)?;
    let (holder_vtable, provider) = one(calls(&getter)
        .flat_map(|(_, target)| {
            installed_vtables(
                &image_function(pe.image, target, CONSTRUCTOR_MAX),
                Some(Register::RCX),
            )
        })
        .filter_map(|vtable| {
            let field = leaf_getter(pe, method(vtable, chain.provider_getter)? as u32)?;
            Some((vtable, field))
        })
        .collect())?;

    let mut steam = Vec::new();
    for slot in 0..SLOT_SCAN {
        let Some(factory) = method(holder_vtable, slot * 8) else {
            continue;
        };
        let Some(instrs) = method_body(pe, factory as u32) else {
            continue;
        };
        if !stores_field(&instrs, provider) {
            continue;
        }
        steam.extend(
            installed_vtables(&instrs, None)
                .into_iter()
                .filter(|&vtable| {
                    method(vtable, chain.steam_check)
                        .is_some_and(|check| returns_true(pe, check as u32))
                }),
        );
    }

    Ok(LoginProvider {
        holder,
        provider,
        steam_vtable: one(steam)?,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::{for_each_archived_build, found};
    use super::*;

    #[test]
    fn finds_login_provider_in_archived_builds() {
        for_each_archived_build(|build, pe| {
            let game_state = found("game state", build, find_game_state(pe));
            let client = found("client", build, find_client(pe, game_state));
            let provider = found("login provider", build, find_login_provider(pe, client));
            if build.to_string_lossy().contains("c65749a2") {
                assert_eq!(
                    provider,
                    LoginProvider {
                        holder: 0x70,
                        provider: 0x18,
                        steam_vtable: 0xb92f18,
                    },
                    "{}",
                    build.display()
                );
            }
        });
    }
}
