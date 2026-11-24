use super::{scene::DecorationLayout, *};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GroundItems {
    pub items: u32,
    pub level: u32,
    pub node_item: u32,
    pub id: u32,
}

fn tile_lookup(instrs: &[Instruction]) -> Option<(u32, u32)> {
    let first = instrs.first()?;
    if !(first.mnemonic() == Mnemonic::Mov
        && first.op0_register() == Register::RCX
        && first.op1_kind() == OpKind::Memory
        && first.memory_base() == Register::RDX
        && first.memory_index() == Register::None)
    {
        return None;
    }
    let tripled = instrs
        .iter()
        .filter(|i| {
            i.mnemonic() == Mnemonic::Lea
                && i.memory_index_scale() == 2
                && i.memory_base() == i.memory_index()
        })
        .count();
    let levels: Vec<u32> = instrs
        .iter()
        .take_while(|i| i.mnemonic() != Mnemonic::Ret)
        .filter(|i| {
            i.mnemonic() == Mnemonic::Mov
                && i.op1_kind() == OpKind::Memory
                && i.memory_index_scale() == 8
                && i.memory_index() != Register::None
        })
        .map(|i| i.memory_displacement64() as u32)
        .collect();
    (tripled == 3 && levels.len() == 2 && levels[0] == levels[1])
        .then_some((first.memory_displacement64() as u32, levels[0]))
}

fn id_compares(instrs: &[Instruction]) -> Vec<(u32, u32)> {
    instrs
        .windows(2)
        .filter_map(|w| {
            let [load, compare] = w else { return None };
            (load.mnemonic() == Mnemonic::Mov
                && load.op0_kind() == OpKind::Register
                && load.op0_register().size() == 8
                && load.op1_kind() == OpKind::Memory
                && load.memory_index() == Register::None
                && compare.mnemonic() == Mnemonic::Cmp
                && compare.op0_kind() == OpKind::Memory
                && compare.memory_base() == load.op0_register()
                && compare.memory_size() == iced_x86::MemorySize::UInt32
                && compare.op1_kind() == OpKind::Register)
                .then(|| {
                    (
                        load.memory_displacement64() as u32,
                        compare.memory_displacement64() as u32,
                    )
                })
        })
        .collect()
}

pub fn find_ground_items(pe: &Pe) -> Result<GroundItems, ScanError> {
    let implementation = lua_binding(pe, "objFind")?;
    let mut found = Vec::new();
    for (_, find) in calls(&whole_function(pe, implementation, 0x800)) {
        let instrs = whole_function(pe, find, 0x200);
        let compares = id_compares(&instrs);
        for (_, lookup) in calls(&instrs) {
            if let Some((items, level)) = tile_lookup(&whole_function(pe, lookup, 0x80)) {
                found.extend(compares.iter().map(|&(node_item, id)| GroundItems {
                    items,
                    level,
                    node_item,
                    id,
                }));
            }
        }
    }
    one(found)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GroundItemOwnership {
    pub public_tick: u32,
    pub despawn_tick: u32,
    pub ownership: u32,
    pub never_public: u32,
    pub op_flags: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Stack(u64),
    Tick,
    TickPlus(u64),
}

struct Store {
    object: Register,
    field: u32,
    byte: bool,
    source: Source,
}

fn stack_slot(i: &Instruction) -> Option<u64> {
    (i.op1_kind() == OpKind::Memory
        && matches!(i.memory_base(), Register::RBP | Register::RSP)
        && i.memory_index() == Register::None)
        .then(|| i.memory_displacement64())
}

fn object_stores(instrs: &[Instruction], tick_count: u32) -> Vec<Store> {
    let mut sources: HashMap<Register, Source> = HashMap::new();
    let mut stores = Vec::new();
    for i in instrs {
        let reads = |size| {
            i.op0_kind() == OpKind::Register
                && i.op0_register().size() == 4
                && i.memory_size() == size
        };
        let target = (i.op0_kind() == OpKind::Register).then(|| i.op0_register().full_register());
        let source = match i.mnemonic() {
            Mnemonic::Mov if reads(iced_x86::MemorySize::UInt32) => {
                stack_slot(i).map(Source::Stack).or_else(|| {
                    (i.op1_kind() == OpKind::Memory
                        && i.memory_index() == Register::None
                        && i.memory_displacement64() == tick_count as u64)
                        .then_some(Source::Tick)
                })
            }
            Mnemonic::Movzx if reads(iced_x86::MemorySize::UInt8) => {
                stack_slot(i).map(Source::Stack)
            }
            Mnemonic::Add if reads(iced_x86::MemorySize::UInt32) => target
                .and_then(|r| sources.get(&r))
                .filter(|&&s| s == Source::Tick)
                .and(stack_slot(i))
                .map(Source::TickPlus),
            Mnemonic::Mov
                if i.op0_kind() == OpKind::Memory
                    && i.op1_kind() == OpKind::Register
                    && i.memory_index() == Register::None
                    && !matches!(
                        i.memory_base(),
                        Register::RBP | Register::RSP | Register::RIP
                    ) =>
            {
                if let Some(&source) = sources.get(&i.op1_register().full_register()) {
                    stores.push(Store {
                        object: i.memory_base(),
                        field: i.memory_displacement64() as u32,
                        byte: i.memory_size() == iced_x86::MemorySize::UInt8,
                        source,
                    });
                }
                None
            }
            _ => None,
        };
        if let Some(target) = target {
            match source {
                Some(source) => sources.insert(target, source),
                None => sources.remove(&target),
            };
        }
        if i.mnemonic() == Mnemonic::Call {
            sources.clear();
        }
    }
    stores
}

fn byte_setter(pe: &Pe, target: u32) -> Option<u32> {
    let [store, ret, ..] = whole_function(pe, target, 8)[..] else {
        return None;
    };
    (store.mnemonic() == Mnemonic::Mov
        && store.op0_kind() == OpKind::Memory
        && store.memory_base() == Register::RCX
        && store.memory_index() == Register::None
        && store.op1_kind() == OpKind::Register
        && store.op1_register() == Register::DL
        && ret.mnemonic() == Mnemonic::Ret)
        .then(|| store.memory_displacement64() as u32)
}

fn ownership_in(
    pe: &Pe,
    instrs: &[Instruction],
    tick_count: u32,
    id: u32,
) -> Option<GroundItemOwnership> {
    let stores = object_stores(instrs, tick_count);
    let object = stores.iter().find(|s| s.field == id)?.object;
    let on_object: Vec<&Store> = stores.iter().filter(|s| s.object == object).collect();
    let from_stack = |field: u32| {
        on_object
            .iter()
            .any(|s| s.field == field && !s.byte && matches!(s.source, Source::Stack(_)))
    };
    if !(from_stack(id) && from_stack(id + 4)) {
        return None;
    }
    let mut timers: Vec<(u64, u32)> = on_object
        .iter()
        .filter_map(|s| match s.source {
            Source::TickPlus(slot) if !s.byte => Some((slot, s.field)),
            _ => None,
        })
        .collect();
    timers.sort_unstable();
    let [(_, public_tick), (despawn_slot, despawn_tick)] = timers[..] else {
        return None;
    };
    let after_timers = |byte: bool| -> Vec<u32> {
        on_object
            .iter()
            .filter_map(|s| match s.source {
                Source::Stack(slot) if s.byte == byte && slot > despawn_slot => Some(s.field),
                _ => None,
            })
            .collect()
    };
    let ([ownership], [never_public]) = (&after_timers(false)[..], &after_timers(true)[..]) else {
        return None;
    };
    let inlined = on_object
        .iter()
        .filter(|s| s.byte && matches!(s.source, Source::Stack(_)) && s.field != *never_public)
        .map(|s| s.field);
    let setters: HashSet<u32> = calls(instrs)
        .filter_map(|(_, target)| byte_setter(pe, target))
        .chain(inlined)
        .collect();
    let op_flags = single(&setters.into_iter().collect::<Vec<_>>()).ok()?;
    Some(GroundItemOwnership {
        public_tick,
        despawn_tick,
        ownership: *ownership,
        never_public: *never_public,
        op_flags,
    })
}

pub fn find_ground_item_ownership(
    pe: &Pe,
    tick_count: u32,
    items: &GroundItems,
) -> Result<GroundItemOwnership, ScanError> {
    let found = functions_with_dword(pe, tick_count)
        .into_iter()
        .filter_map(|(begin, end)| {
            ownership_in(pe, &function_instrs(pe, begin, end), tick_count, items.id)
        })
        .collect();
    one(found)
}

fn pile_store(instrs: &[Instruction], decoration: &DecorationLayout) -> Option<u32> {
    let mut slots: HashMap<(u32, Register), HashSet<u32>> = HashMap::new();
    for w in instrs.windows(2) {
        if w[0].mnemonic() == Mnemonic::Lea
            && w[0].op0_register() == Register::RCX
            && w[0].memory_index() == Register::None
            && call_target(&w[1]).is_some()
        {
            slots
                .entry((w[1].near_branch_target() as u32, w[0].memory_base()))
                .or_default()
                .insert(w[0].memory_displacement64() as u32);
        }
    }
    let stores = |pile: Register, field: u32| {
        instrs.iter().any(|i| {
            i.mnemonic() == Mnemonic::Mov
                && i.op0_kind() == OpKind::Memory
                && i.memory_base() == pile
                && i.memory_displacement64() == field as u64
        })
    };
    slots
        .iter()
        .filter(|(_, offsets)| offsets.len() == 3)
        .find(|(&(_, base), _)| stores(base, decoration.x) && stores(base, decoration.y))?;
    instrs.iter().enumerate().find_map(|(at, i)| {
        let (true, OpKind::Register, OpKind::Immediate32to64) =
            (i.mnemonic() == Mnemonic::Add, i.op0_kind(), i.op1_kind())
        else {
            return None;
        };
        let tile = i.op0_register();
        instrs[at + 1..(at + 8).min(instrs.len())]
            .iter()
            .any(|s| {
                s.mnemonic() == Mnemonic::Mov
                    && s.op0_kind() == OpKind::Memory
                    && s.memory_base() == tile
                    && s.memory_displacement64() == 0
                    && s.op1_kind() == OpKind::Register
            })
            .then(|| i.immediate(1) as u32)
    })
}

pub fn find_item_pile(pe: &Pe, decoration: &DecorationLayout) -> Result<u32, ScanError> {
    let mut found = Vec::new();
    for &(begin, end) in &pe.functions {
        if end - begin > 0x800 {
            continue;
        }
        let instrs = function_instrs(pe, begin, end);
        found.extend(pile_store(&instrs, decoration));
    }
    one(found)
}

