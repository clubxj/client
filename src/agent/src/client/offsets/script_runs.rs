use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ScriptRuns {
    pub function: u32,
    pub event_type: u32,
    pub args: u32,
    pub arg_count: u32,
}

pub const ARG_SIZE: u32 = 0x28;
pub const ARG_KIND: u32 = 0x20;
pub const KIND_INT: u8 = 1;
const FIRST_ARG_PLACEHOLDER: u64 = 0x8000_0001;
const LAST_ARG_PLACEHOLDER: u64 = 0x8000_000a;
const RANGE_WINDOW: usize = 8;

fn is_event_runner(instrs: &[Instruction]) -> bool {
    let compares = |value: u64| {
        instrs.iter().any(|i| {
            i.mnemonic() == Mnemonic::Cmp
                && i.op0_kind() == OpKind::Register
                && i.op0_register().size() == 4
                && immediate(i) == Some(value)
        })
    };
    let subtracts_ten = |i: &Instruction| match i.mnemonic() {
        Mnemonic::Lea => i.memory_displacement64() as i64 == -0xa,
        Mnemonic::Add => immediate(i).is_some_and(|v| v as i32 == -0xa),
        Mnemonic::Sub => immediate(i) == Some(0xa),
        _ => false,
    };
    let range = instrs.iter().enumerate().any(|(n, i)| {
        i.op0_kind() == OpKind::Register
            && subtracts_ten(i)
            && instrs[n + 1..].iter().take(RANGE_WINDOW).any(|c| {
                c.mnemonic() == Mnemonic::Cmp
                    && c.op0_kind() == OpKind::Register
                    && c.op0_register() == i.op0_register()
                    && immediate(c) == Some(7)
            })
    });
    range && (FIRST_ARG_PLACEHOLDER..=LAST_ARG_PLACEHOLDER).all(compares)
}

fn fields(instrs: &[Instruction]) -> Option<(u32, u32, u32)> {
    let copies = instrs
        .iter()
        .take(24)
        .filter(|i| {
            i.mnemonic() == Mnemonic::Mov
                && i.op0_kind() == OpKind::Register
                && i.op0_register().size() == 8
                && i.op1_register() == Register::RDX
        })
        .map(|i| i.op0_register());
    let event: Vec<Register> = core::iter::once(Register::RDX).chain(copies).collect();
    let loads = |size: usize| {
        instrs
            .iter()
            .take(24)
            .filter(move |i| i.op0_kind() == OpKind::Register && i.op0_register().size() == size)
            .filter_map(|i| event.iter().find_map(|&base| load_from(i, base)))
    };
    let event_type = loads(4).next()?;
    let args = instrs.iter().take(24).find_map(|i| {
        let wide = i.op0_kind() == OpKind::Register && i.op0_register().size() == 8;
        let address = i.mnemonic() == Mnemonic::Lea
            && i.memory_index() == Register::None
            && event.contains(&i.memory_base());
        if !wide {
            return None;
        }
        if address {
            return Some(i.memory_displacement64() as u32);
        }
        event.iter().find_map(|&base| load_from(i, base))
    })?;
    let count = one(instrs
        .iter()
        .filter(|i| {
            i.mnemonic() == Mnemonic::Cmp
                && i.op0_kind() == OpKind::Memory
                && i.memory_size() == iced_x86::MemorySize::UInt32
                && event.contains(&i.memory_base())
                && i.op1_kind() == OpKind::Register
        })
        .map(|i| i.memory_displacement64() as u32)
        .collect())
    .ok()?;
    Some((event_type, args, count))
}

pub fn find_script_runs(pe: &Pe) -> Result<ScriptRuns, ScanError> {
    let mut found = Vec::new();
    for &(begin, end) in &pe.functions {
        if !(0x200..=0x800).contains(&(end - begin)) {
            continue;
        }
        let instrs = whole_function(pe, begin, end - begin);
        if !is_event_runner(&instrs) {
            continue;
        }
        if let Some((event_type, args, arg_count)) = fields(&instrs) {
            found.push(ScriptRuns {
                function: begin,
                event_type,
                args,
                arg_count,
            });
        }
    }
    one(found)
}
