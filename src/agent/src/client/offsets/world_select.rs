use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldSelect {
    pub switcher: u32,
    pub current_world: u32,
    pub master_count: u32,
    pub master_worlds: u32,
    pub open: u32,
    pub list: u32,
    pub state: u32,
    pub layout: u32,
    pub selected: u32,
    pub hovered: u32,
    pub page: u32,
    pub compact: u32,
    pub list_rect: u32,
    pub row_offset: u32,
    pub row_height: u32,
    pub rows: u32,
    pub columns: u32,
    pub compact_columns: u32,
    pub previous: u32,
    pub next: u32,
    pub world: WorldFields,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldFields {
    pub id: u32,
    pub types: u32,
    pub players: u32,
    pub activity: u32,
    pub location: u32,
}

const FLAG_SPRITES: &str = "sl_flags";
const SHORT_FUNCTION: usize = 20;
const CHUNK_SCAN: usize = 32;

fn body(pe: &Pe, begin: u32) -> Option<Vec<Instruction>> {
    let (b, e) = pe.function_containing(begin)?;
    (b == begin).then(|| function_instrs(pe, b, e))
}

fn root_of(pe: &Pe, begin: u32) -> u32 {
    chunk_root(pe, begin).unwrap_or(begin)
}

fn with_chunks(pe: &Pe, root: u32) -> Option<Vec<Instruction>> {
    let start = pe.functions.iter().position(|&(b, _)| b == root)?;
    let mut instrs = Vec::new();
    for &(begin, end) in pe.functions[start..].iter().take(CHUNK_SCAN) {
        if root_of(pe, begin) == root {
            instrs.extend(function_instrs(pe, begin, end));
        }
    }
    Some(instrs)
}

fn immediate_u32(i: &Instruction) -> Option<u32> {
    immediate(i).map(|value| value as u32)
}

fn loads_rcx_from(i: &Instruction, global: u32) -> bool {
    global_load(i).is_some_and(|(reg, g)| reg == Register::RCX && g == global)
}

fn compares_global_with_zero(i: &Instruction, global: u32) -> bool {
    i.mnemonic() == Mnemonic::Cmp
        && i.op0_kind() == OpKind::Memory
        && i.is_ip_rel_memory_operand()
        && i.ip_rel_memory_address() as u32 == global
        && immediate(i) == Some(0)
}

fn memory_base_disp(i: &Instruction) -> Option<(Register, u32)> {
    (0..i.op_count())
        .any(|k| i.op_kind(k) == OpKind::Memory)
        .then(|| (i.memory_base(), i.memory_displacement64() as u32))
        .filter(|(base, _)| {
            !matches!(base, Register::RIP | Register::RSP | Register::None)
                && i.memory_index() == Register::None
        })
}

fn flag_sprite_users(pe: &Pe) -> HashSet<u32> {
    let texts = string_rvas(pe, FLAG_SPRITES);
    pe.functions
        .iter()
        .filter_map(|&(begin, end)| {
            let instrs = function_instrs(pe, begin, end);
            (instrs.len() > SHORT_FUNCTION
                && instrs
                    .iter()
                    .any(|i| rip_lea(i).is_some_and(|t| texts.contains(&t))))
            .then(|| root_of(pe, begin))
        })
        .collect()
}

struct Opener {
    switcher: u32,
    current_world: u32,
}

fn opener(pe: &Pe, client: u32) -> Result<Opener, ScanError> {
    let mut found = Vec::new();
    for user in flag_sprite_users(pe) {
        let loaders: HashSet<u32> = callers(pe, user).map(|c| root_of(pe, c.begin)).collect();
        for loader in loaders {
            for caller in callers(pe, loader) {
                let root = root_of(pe, caller.begin);
                let instrs = function_instrs(pe, root, caller.end);
                let Some(at) = instrs
                    .iter()
                    .position(|i| i.ip() == caller.instrs[caller.at].ip())
                else {
                    continue;
                };
                let Some(switcher) = instrs[at.saturating_sub(4)..at]
                    .iter()
                    .rev()
                    .find_map(|i| global_load(i).filter(|(r, _)| *r == Register::RCX))
                    .map(|(_, g)| g)
                else {
                    continue;
                };
                if !instrs
                    .iter()
                    .any(|i| compares_global_with_zero(i, switcher))
                {
                    continue;
                }
                let client_regs: HashSet<Register> = instrs[..at]
                    .iter()
                    .filter(|i| global_load(i).is_some_and(|(_, g)| g == client))
                    .map(|i| i.op0_register().full_register())
                    .collect();
                let current: Vec<u32> = instrs[at + 1..]
                    .iter()
                    .take(12)
                    .filter(|i| {
                        i.mnemonic() == Mnemonic::Mov
                            && i.op0_register() == Register::EDX
                            && client_regs.contains(&i.memory_base())
                    })
                    .filter_map(memory_base_disp)
                    .map(|(_, disp)| disp)
                    .collect();
                if let Ok(current_world) = one(current) {
                    found.push((switcher, current_world));
                }
            }
        }
    }
    let (switcher, current_world) = one(found)?;
    Ok(Opener {
        switcher,
        current_world,
    })
}

fn switcher_methods(pe: &Pe, switcher: u32) -> Vec<u32> {
    let mut targets: Vec<u32> = pe
        .functions
        .iter()
        .flat_map(|&(begin, end)| {
            let instrs = function_instrs(pe, begin, end);
            instrs
                .iter()
                .enumerate()
                .filter_map(|(n, i)| {
                    let target = call_target(i)?;
                    instrs[n.saturating_sub(12)..n]
                        .iter()
                        .rev()
                        .find(|j| {
                            j.op0_kind() == OpKind::Register
                                && j.op0_register().full_register() == Register::RCX
                        })
                        .is_some_and(|j| loads_rcx_from(j, switcher))
                        .then_some(target)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    targets.sort_unstable();
    targets.dedup();
    targets
}

struct Dispatch {
    open: u32,
    list: u32,
    state: u32,
    layout: u32,
    input: u32,
}

fn dispatch(pe: &Pe, methods: &[u32]) -> Result<Dispatch, ScanError> {
    let mut found = Vec::new();
    for &method in methods {
        let Some(instrs) = body(pe, method) else {
            continue;
        };
        let head = &instrs[..instrs.len().min(40)];
        let open = head.iter().find_map(|i| {
            (i.mnemonic() == Mnemonic::Cmp
                && i.op0_kind() == OpKind::Memory
                && i.memory_size() == iced_x86::MemorySize::UInt8
                && immediate(i) == Some(0))
            .then(|| memory_base_disp(i))
            .flatten()
            .filter(|(base, _)| *base == Register::RCX)
            .map(|(_, disp)| disp)
        });
        let layout = head.iter().find_map(|i| {
            (i.mnemonic() == Mnemonic::Mov
                && i.op0_kind() == OpKind::Register
                && i.op0_register().size() == 8)
                .then(|| load_from(i, Register::RCX))
                .flatten()
        });
        let state = head.iter().find_map(|i| {
            (i.mnemonic() == Mnemonic::Lea && i.memory_base() == Register::RCX)
                .then(|| i.memory_displacement64() as u32)
        });
        let list = head.iter().find_map(|i| {
            (i.mnemonic() == Mnemonic::Add && i.op0_register() == Register::RCX)
                .then(|| immediate_u32(i))
                .flatten()
        });
        let input = head.iter().find_map(call_target);
        if let (Some(open), Some(layout), Some(state), Some(list), Some(input)) =
            (open, layout, state, list, input)
        {
            found.push((method, open, list, state, layout, input));
        }
    }
    let (_, open, list, state, layout, input) = one(found)?;
    Ok(Dispatch {
        open,
        list,
        state,
        layout,
        input,
    })
}

#[derive(Clone, Copy)]
struct HitTest {
    function: u32,
    list_rect: u32,
    row_offset: u32,
    row_height: u32,
    rows: u32,
    columns: u32,
    compact_columns: u32,
}

fn hit_test(pe: &Pe, function: u32) -> Option<HitTest> {
    let instrs = body(pe, function)?;
    let mut layout_regs: HashSet<Register> = HashSet::from([Register::RCX]);
    for i in &instrs {
        if i.mnemonic() == Mnemonic::Mov
            && i.op0_kind() == OpKind::Register
            && i.op1_kind() == OpKind::Register
            && i.op1_register() == Register::RCX
        {
            layout_regs.insert(i.op0_register());
        }
    }
    let field = |i: &Instruction| {
        memory_base_disp(i)
            .filter(|(base, _)| layout_regs.contains(base))
            .map(|(_, disp)| disp)
    };
    let row_height = one(instrs
        .iter()
        .filter(|i| i.mnemonic() == Mnemonic::Idiv)
        .filter_map(field)
        .collect())
    .ok()?;
    let row_offset = one(instrs
        .iter()
        .filter(|i| i.mnemonic() == Mnemonic::Sub)
        .filter_map(field)
        .collect())
    .ok()?;
    let loads: Vec<(Register, u32)> = instrs
        .iter()
        .filter(|i| i.mnemonic() == Mnemonic::Mov && i.op0_kind() == OpKind::Register)
        .filter_map(|i| Some((i.op0_register(), field(i)?)))
        .collect();
    let &(_, list_rect) = loads.first()?;
    let rect = list_rect..list_rect + 16;
    let others: Vec<(Register, u32)> = loads
        .iter()
        .copied()
        .filter(|(_, disp)| !rect.contains(disp))
        .collect();
    let paired: Vec<u32> = others
        .iter()
        .filter(|(reg, _)| others.iter().filter(|(r, _)| r == reg).count() == 2)
        .map(|&(_, disp)| disp)
        .collect();
    let [compact_columns, columns] = paired.as_slice() else {
        return None;
    };
    let rows = one(others
        .iter()
        .map(|&(_, disp)| disp)
        .filter(|disp| !paired.contains(disp))
        .collect())
    .ok()?;
    Some(HitTest {
        function,
        list_rect,
        row_offset,
        row_height,
        rows,
        columns: *columns,
        compact_columns: *compact_columns,
    })
}

struct StateFields {
    hit: HitTest,
    selected: u32,
    hovered: u32,
    page: u32,
    compact: u32,
}

fn state_fields(pe: &Pe, input: u32) -> Result<StateFields, ScanError> {
    let instrs = body(pe, input).ok_or(ScanError::NotFound)?;
    let mut found = Vec::new();
    for (at, call) in instrs.iter().enumerate() {
        let Some(hit) = call_target(call).and_then(|t| hit_test(pe, t)) else {
            continue;
        };
        let before = &instrs[at.saturating_sub(10)..at];
        let page_reg = before.iter().rev().find_map(|i| {
            (i.mnemonic() == Mnemonic::Mov
                && i.op0_register() == Register::R9D
                && i.op1_kind() == OpKind::Register)
                .then(|| i.op1_register())
        });
        let page = page_reg.and_then(|reg| {
            before
                .iter()
                .rev()
                .find(|i| i.mnemonic() == Mnemonic::Mov && i.op0_register() == reg)
                .and_then(memory_base_disp)
        });
        let stores: Vec<(Register, u32)> = instrs[at + 1..]
            .iter()
            .take(16)
            .filter(|i| {
                i.mnemonic() == Mnemonic::Mov
                    && i.op0_kind() == OpKind::Memory
                    && i.op1_kind() == OpKind::Register
                    && i.op1_register() == Register::EAX
            })
            .filter_map(memory_base_disp)
            .collect();
        if let (Some((state_reg, page)), [(hb, hovered), (sb, selected), ..]) =
            (page, stores.as_slice())
        {
            if *hb == state_reg && *sb == state_reg {
                found.push((hit.function, page, *hovered, *selected));
            }
        }
    }
    let (function, page, hovered, selected) = one(found)?;
    let hit = hit_test(pe, function).ok_or(ScanError::NotFound)?;
    let compact = one(instrs
        .iter()
        .filter_map(call_target)
        .filter_map(|t| {
            decode_at(pe, t, 4).iter().find_map(|i| {
                (i.mnemonic() == Mnemonic::Cmp
                    && i.memory_size() == iced_x86::MemorySize::UInt8
                    && immediate(i) == Some(0))
                .then(|| memory_base_disp(i))
                .flatten()
                .filter(|(base, _)| *base == Register::RDX)
                .map(|(_, disp)| disp)
            })
        })
        .collect())?;
    Ok(StateFields {
        hit,
        selected,
        hovered,
        page,
        compact,
    })
}

fn page_count(pe: &Pe, function: u32, hit: &HitTest) -> bool {
    let instrs = decode_at(pe, function, 40);
    let instrs: Vec<&Instruction> = instrs
        .iter()
        .take_while(|i| i.mnemonic() != Mnemonic::Ret)
        .collect();
    let reads = |disp: u32| {
        instrs
            .iter()
            .any(|i| memory_base_disp(i) == Some((Register::RCX, disp)))
    };
    instrs.iter().any(|i| i.mnemonic() == Mnemonic::Idiv)
        && reads(hit.rows)
        && reads(hit.columns)
        && reads(hit.compact_columns)
}

fn render(pe: &Pe, methods: &[u32], layout: u32) -> Result<u32, ScanError> {
    one(methods
        .iter()
        .filter_map(|&method| {
            let instrs = body(pe, method)?;
            let checks_layout = instrs.iter().any(|i| {
                i.mnemonic() == Mnemonic::Cmp
                    && memory_base_disp(i) == Some((Register::RCX, layout))
                    && immediate(i) == Some(0)
            });
            let jump = instrs
                .iter()
                .rev()
                .find(|i| i.mnemonic() == Mnemonic::Jmp)?;
            (checks_layout && jump.op0_kind() == OpKind::NearBranch64)
                .then(|| jump.near_branch_target() as u32)
        })
        .collect())
}

fn arrows(pe: &Pe, parts: &[u32], hit: &HitTest) -> Result<(u32, u32), ScanError> {
    let mut found = Vec::new();
    for &part in parts {
        let Some(instrs) = body(pe, part) else {
            continue;
        };
        for (at, call) in instrs.iter().enumerate() {
            if !call_target(call).is_some_and(|t| page_count(pe, t, hit)) {
                continue;
            }
            let before = &instrs[at.saturating_sub(30)..at];
            let xs: Vec<u32> = before
                .iter()
                .filter(|i| i.mnemonic() == Mnemonic::Mov && i.op0_register() == Register::R8D)
                .filter_map(memory_base_disp)
                .filter(|&(base, disp)| {
                    before.iter().any(|j| {
                        j.mnemonic() == Mnemonic::Mov
                            && j.op0_register() == Register::R9D
                            && memory_base_disp(j) == Some((base, disp + 4))
                    })
                })
                .map(|(_, disp)| disp)
                .collect();
            if let [previous, next] = xs.as_slice() {
                found.push((*previous, *next));
            }
        }
    }
    one(found)
}

struct Header {
    master_count: u32,
    master_worlds: u32,
    world: WorldFields,
}

fn header_at(pe: &Pe, instrs: &[Instruction], n: usize) -> Option<(u32, WorldFields)> {
    let cmp = &instrs[n];
    let count = (cmp.mnemonic() == Mnemonic::Cmp
        && cmp.op1_kind() == OpKind::Memory
        && cmp.is_ip_rel_memory_operand())
    .then(|| cmp.ip_rel_memory_address() as u32)?;
    let rest = &instrs[n + 1..instrs.len().min(n + 12)];
    let array = rest
        .iter()
        .find_map(|j| global_load(j).filter(|(_, g)| *g == count + 8))
        .map(|(reg, _)| reg)?;
    let world = rest.iter().find_map(|j| {
        (j.mnemonic() == Mnemonic::Mov
            && j.memory_base() == array
            && j.memory_index() != Register::None)
            .then(|| j.op0_register())
    })?;
    let tail = &instrs[n + 1..];
    let passes_world = |k: &Instruction| {
        k.mnemonic() == Mnemonic::Mov
            && k.op0_register() == Register::RCX
            && k.op1_kind() == OpKind::Register
            && k.op1_register() == world
    };
    let call_at = (0..tail.len())
        .find(|&c| call_target(&tail[c]).is_some() && tail[..c].iter().any(passes_world))?;
    let fields = world_box(pe, call_target(&tail[call_at])?)?;
    let activity = tail[call_at + 1..].iter().take(8).find_map(|j| {
        (j.mnemonic() == Mnemonic::Lea && j.memory_base() == world)
            .then(|| j.memory_displacement64() as u32)
    })?;
    Some((count, WorldFields { activity, ..fields }))
}

fn header(pe: &Pe, parts: &[u32]) -> Result<Header, ScanError> {
    let mut found = Vec::new();
    for &part in parts {
        let Some(instrs) = body(pe, part) else {
            continue;
        };
        found.extend((0..instrs.len()).filter_map(|n| header_at(pe, &instrs, n)));
    }
    found.sort_unstable_by_key(|(count, w)| {
        (*count, w.id, w.types, w.players, w.activity, w.location)
    });
    found.dedup();
    let (count, world) = single(&found)?;
    Ok(Header {
        master_count: count,
        master_worlds: count + 8,
        world,
    })
}

fn world_box(pe: &Pe, function: u32) -> Option<WorldFields> {
    let instrs = with_chunks(pe, function)?;
    let mut world_regs: HashSet<Register> = HashSet::from([Register::RCX]);
    for i in &instrs {
        if i.mnemonic() == Mnemonic::Mov
            && i.op0_kind() == OpKind::Register
            && i.op1_kind() == OpKind::Register
            && i.op1_register() == Register::RCX
        {
            world_regs.insert(i.op0_register());
        }
    }
    let field = |i: &Instruction| {
        (i.op1_kind() == OpKind::Memory)
            .then(|| memory_base_disp(i))
            .flatten()
            .filter(|(base, _)| world_regs.contains(base))
            .map(|(_, disp)| disp)
    };
    let players = one(instrs
        .iter()
        .enumerate()
        .filter(|(_, i)| i.mnemonic() == Mnemonic::Mov && i.op0_kind() == OpKind::Register)
        .filter_map(|(n, i)| {
            let disp = field(i)?;
            instrs[n + 1..]
                .iter()
                .take(4)
                .any(|c| {
                    c.mnemonic() == Mnemonic::Cmp
                        && c.op0_kind() == OpKind::Register
                        && c.op0_register() == i.op0_register()
                        && immediate_u32(c) == Some(u32::MAX)
                })
                .then_some(disp)
        })
        .collect())
    .ok()?;
    let location = one(instrs
        .iter()
        .enumerate()
        .filter(|(_, i)| i.mnemonic() == Mnemonic::Mov && i.op0_register().size() == 4)
        .filter_map(|(n, i)| {
            let disp = field(i)?;
            instrs[n + 1..]
                .iter()
                .take(12)
                .any(|j| {
                    j.mnemonic() == Mnemonic::Movsxd
                        && j.op1_kind() == OpKind::Register
                        && j.op1_register() == i.op0_register()
                })
                .then_some(disp)
        })
        .collect())
    .ok()?;
    let id = one(instrs
        .iter()
        .enumerate()
        .filter(|(_, i)| i.mnemonic() == Mnemonic::Mov && i.op0_register() == Register::R9D)
        .filter_map(|(n, i)| {
            let disp = field(i).filter(|&disp| disp != players)?;
            instrs[n + 1..]
                .iter()
                .take(12)
                .any(|j| call_target(j).is_some())
                .then_some(disp)
        })
        .collect())
    .ok()?;
    let accessor = instrs.iter().find_map(call_target)?;
    let types = decode_at(pe, accessor, 2).first().and_then(|load| {
        (load.mnemonic() == Mnemonic::Mov && load.op0_register().size() == 4)
            .then(|| load_from(load, Register::RCX))
            .flatten()
    })?;
    Some(WorldFields {
        id,
        types,
        players,
        activity: 0,
        location,
    })
}

pub fn find_world_select(pe: &Pe, client: u32) -> Result<WorldSelect, ScanError> {
    let opener = opener(pe, client)?;
    let methods = switcher_methods(pe, opener.switcher);
    let dispatch = dispatch(pe, &methods)?;
    let state = state_fields(pe, dispatch.input)?;
    let render = render(pe, &methods, dispatch.layout)?;
    let render_body = body(pe, render).ok_or(ScanError::NotFound)?;
    let mut parts: Vec<u32> = render_body.iter().filter_map(call_target).collect();
    parts.extend(
        render_body
            .iter()
            .filter(|i| i.mnemonic() == Mnemonic::Jmp && i.op0_kind() == OpKind::NearBranch64)
            .map(|i| i.near_branch_target() as u32),
    );
    let (previous, next) = arrows(pe, &parts, &state.hit)?;
    let header = header(pe, &parts)?;
    Ok(WorldSelect {
        switcher: opener.switcher,
        current_world: opener.current_world,
        master_count: header.master_count,
        master_worlds: header.master_worlds,
        open: dispatch.open,
        list: dispatch.list,
        state: dispatch.state,
        layout: dispatch.layout,
        selected: state.selected,
        hovered: state.hovered,
        page: state.page,
        compact: state.compact,
        list_rect: state.hit.list_rect,
        row_offset: state.hit.row_offset,
        row_height: state.hit.row_height,
        rows: state.hit.rows,
        columns: state.hit.columns,
        compact_columns: state.hit.compact_columns,
        previous,
        next,
        world: header.world,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::offsets::tests::for_each_archived_build;

    #[test]
    fn finds_world_select_in_e470c533() {
        for_each_archived_build(|build, pe| {
            if !build.to_string_lossy().contains("e470c533") {
                return;
            }
            let client = find_local_player(pe).unwrap().client;
            let found = find_world_select(pe, client);
            assert_eq!(
                found,
                Ok(WorldSelect {
                    switcher: 0x1c21a68,
                    current_world: 0x32fc,
                    master_count: 0x15a7a38,
                    master_worlds: 0x15a7a40,
                    open: 0,
                    list: 8,
                    state: 0x78,
                    layout: 0x180,
                    selected: 0x58,
                    hovered: 0x5c,
                    page: 0x60,
                    compact: 0x64,
                    list_rect: 0x10,
                    row_offset: 0xd0,
                    row_height: 0xd4,
                    rows: 0xe8,
                    columns: 0xec,
                    compact_columns: 0xf0,
                    previous: 0x40,
                    next: 0x50,
                    world: WorldFields {
                        id: 0,
                        types: 4,
                        players: 8,
                        activity: 0x28,
                        location: 0x40,
                    },
                })
            );
        });
    }
}
