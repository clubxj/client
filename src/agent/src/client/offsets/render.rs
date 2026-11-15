use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Render {
    pub draw: u32,
    pub pass: u32,
    pub transient: u32,
}

fn returns_on(instrs: &[Instruction], tag: i64, pass: u64) -> bool {
    instrs.windows(3).any(|w| {
        w[0].mnemonic() == Mnemonic::Cmp
            && w[0].op0_kind() == OpKind::Register
            && w[0].op0_register() == Register::R9
            && immediate(&w[0]).is_some_and(|value| value as i64 == tag)
            && w[1].mnemonic() == Mnemonic::Jne
            && w[2].mnemonic() == Mnemonic::Mov
            && w[2].op0_kind() == OpKind::Register
            && w[2].op0_register() == Register::AL
            && immediate(&w[2]) == Some(pass)
    })
}

fn is_pass(pe: &Pe, begin: u32, end: u32) -> bool {
    let head = function_head(pe, begin, end, 16);
    returns_on(&head, -2, 4) && returns_on(&head, -3, 5)
}

fn stores_orientation(pe: &Pe, root: u32) -> bool {
    let Some(&(begin, end)) = pe.functions.iter().find(|&&(b, _)| b == root) else {
        return false;
    };
    let head = function_head(pe, begin, end, 2);
    head.len() == 2
        && head[0].mnemonic() == Mnemonic::Mov
        && head[0].op0_kind() == OpKind::Register
        && head[0].op0_register() == Register::RAX
        && head[0].op1_kind() == OpKind::Register
        && head[0].op1_register() == Register::RSP
        && head[1].mnemonic() == Mnemonic::Mov
        && head[1].op0_kind() == OpKind::Memory
        && head[1].memory_base() == Register::RAX
        && head[1].memory_displacement64() == 0x18
        && head[1].op1_kind() == OpKind::Register
        && head[1].op1_register() == Register::R8D
}

fn transient_flag(pe: &Pe, draw: u32) -> Option<u32> {
    whole_function(pe, draw, 0x1000).windows(2).find_map(|w| {
        (w[0].mnemonic() == Mnemonic::Call
            && w[0].op0_kind() == OpKind::Register
            && w[1].mnemonic() == Mnemonic::Cmp
            && w[1].op0_kind() == OpKind::Memory
            && w[1].memory_size() == iced_x86::MemorySize::UInt8
            && w[1].memory_index() == Register::None
            && immediate(&w[1]) == Some(0))
        .then(|| w[1].memory_displacement64() as u32)
    })
}

pub fn find_render(pe: &Pe) -> Result<Render, ScanError> {
    let pass = one(pe
        .functions
        .iter()
        .filter(|&&(begin, end)| chunk_root(pe, begin) == Some(begin) && is_pass(pe, begin, end))
        .map(|&(begin, _)| begin)
        .collect())?;
    let draw = one(caller_functions(pe, pass)
        .filter_map(|(begin, _)| chunk_root(pe, begin))
        .filter(|&root| stores_orientation(pe, root))
        .collect())?;
    let transient = transient_flag(pe, draw).ok_or(ScanError::NotFound)?;
    Ok(Render {
        draw,
        pass,
        transient,
    })
}
