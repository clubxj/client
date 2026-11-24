use super::packet_layouts::{classify_read, PacketLayouts, Read};
use super::*;
use iced_x86::MemorySize;
use ipc::packets::{Field, Name};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PacketAnchors {
    pub tick_count: Option<u32>,
    pub varps: Option<u32>,
    pub chat_messages: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedPacket {
    pub opcode: u16,
    pub order: Vec<u8>,
}

const CASE_MAX: u32 = 0x800;
const CALLEE_MAX: u32 = 0x400;
const SETTER_INSTRS: usize = 24;
const BINDING_MAX: u32 = 0x80;
const SMART_MAX: u32 = 0x40;
const SMART_LIMIT: u64 = 0x80;
const SMART_BIAS: u64 = 0x8000;
const CHAT_DEPTH: usize = 3;
const CHAT_TABLE_SPAN: u32 = 0x40;
const VARP_ID_WIDTH: usize = 2;
const SOUND_WIDTHS: [usize; 3] = [2, 1, 2];
const FLAG_WIDTH: usize = 1;
const NAME_FLAG: u64 = 1;

const VOLATILE: [Register; 7] = [
    Register::RAX,
    Register::RCX,
    Register::RDX,
    Register::R8,
    Register::R9,
    Register::R10,
    Register::R11,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Value {
    Field(usize),
    Call(usize),
}

struct Case<'a> {
    opcode: u16,
    size: i32,
    fields: Option<&'a Vec<Field>>,
    code: Vec<Instruction>,
}

struct Reads<'p> {
    pe: &'p Pe<'p>,
    data_at: u32,
    cursor_at: u32,
    known: HashMap<u32, Option<Read>>,
}

impl Reads<'_> {
    fn width(&mut self, target: u32) -> Option<usize> {
        let (pe, data_at, cursor_at) = (self.pe, self.data_at, self.cursor_at);
        self.known
            .entry(target)
            .or_insert_with(|| classify_read(pe, target, data_at, cursor_at))
            .as_ref()
            .map(|read| read.shifts.len())
    }
}

struct Flow {
    registers: HashMap<Register, Value>,
    fields: usize,
    calls: usize,
}

impl Flow {
    fn new() -> Self {
        Self {
            registers: HashMap::new(),
            fields: 0,
            calls: 0,
        }
    }

    fn get(&self, register: Register) -> Option<Value> {
        self.registers.get(&register).copied()
    }

    fn step(&mut self, i: &Instruction, reads: &mut Reads) {
        if let Some(target) = call_target(i) {
            for register in VOLATILE {
                self.registers.remove(&register);
            }
            let value = match reads.width(target) {
                Some(_) => {
                    self.fields += 1;
                    Value::Field(self.fields - 1)
                }
                None => Value::Call(self.calls),
            };
            self.calls += 1;
            self.registers.insert(Register::RAX, value);
            return;
        }
        if i.op0_kind() != OpKind::Register
            || matches!(
                i.mnemonic(),
                Mnemonic::Cmp | Mnemonic::Test | Mnemonic::Push | Mnemonic::Bt
            )
        {
            return;
        }
        let to = i.op0_register().full_register();
        let copied = matches!(
            i.mnemonic(),
            Mnemonic::Mov | Mnemonic::Movzx | Mnemonic::Movsx | Mnemonic::Movsxd
        ) && i.op1_kind() == OpKind::Register;
        match copied
            .then(|| self.get(i.op1_register().full_register()))
            .flatten()
        {
            Some(value) => self.registers.insert(to, value),
            None => self.registers.remove(&to),
        };
    }
}

fn case_code(pe: &Pe, cases: &[u32], begin: u32) -> Vec<Instruction> {
    let end = cases
        .iter()
        .copied()
        .filter(|&c| c > begin)
        .min()
        .unwrap_or(u32::MAX)
        .min(begin + CASE_MAX);
    function_instrs(pe, begin, end)
}

fn widths(fields: &[Field]) -> Vec<usize> {
    fields.iter().map(Field::width).collect()
}

fn name_one(found: Vec<NamedPacket>) -> Result<NamedPacket, ScanError> {
    match found.as_slice() {
        [] => Err(ScanError::NotFound),
        [only] => Ok(only.clone()),
        many => Err(ScanError::Ambiguous(many.len())),
    }
}

fn plain(opcode: u16) -> NamedPacket {
    NamedPacket {
        opcode,
        order: Vec::new(),
    }
}

fn find_tick(cases: &[Case], tick_count: u32) -> Result<NamedPacket, ScanError> {
    let increments = |i: &Instruction| {
        i.mnemonic() == Mnemonic::Inc
            && i.op0_kind() == OpKind::Memory
            && i.memory_size() == MemorySize::UInt32
            && base_field(i).is_some_and(|(_, disp)| disp == tick_count as i64)
    };
    name_one(
        cases
            .iter()
            .filter(|case| case.size == 0)
            .filter(|case| {
                case.code
                    .iter()
                    .take_while(|i| i.flow_control() == iced_x86::FlowControl::Next)
                    .any(increments)
            })
            .map(|case| plain(case.opcode))
            .collect(),
    )
}

fn loads_global(pe: &Pe, function: u32, global: u32) -> bool {
    decode_at(pe, function, SETTER_INSTRS)
        .iter()
        .any(|i| global_load(i).is_some_and(|(_, at)| at == global))
}

fn find_varps(
    pe: &Pe,
    cases: &[Case],
    varps: u32,
    size: i32,
    value_width: usize,
) -> Result<NamedPacket, ScanError> {
    let data = varps + ipc::state::array::DATA as u32;
    let mut setters: HashMap<u32, bool> = HashMap::new();
    name_one(
        cases
            .iter()
            .filter(|case| case.size == size)
            .filter_map(|case| {
                let fields = widths(case.fields?);
                let id = fields.iter().position(|&w| w == VARP_ID_WIDTH)?;
                let value = fields.iter().position(|&w| w == value_width)?;
                if fields.len() != 2 || id == value {
                    return None;
                }
                let passes_array = case.code.iter().any(|i| rip_lea(i) == Some(varps));
                let sets = passes_array
                    || calls(&case.code).any(|(_, target)| {
                        *setters
                            .entry(target)
                            .or_insert_with(|| loads_global(pe, target, data))
                    });
                sets.then(|| NamedPacket {
                    opcode: case.opcode,
                    order: vec![id as u8, value as u8],
                })
            })
            .collect(),
    )
}

fn binding_target(pe: &Pe, binding: u32) -> Result<u32, ScanError> {
    let targets: Vec<u32> = whole_function(pe, binding, BINDING_MAX)
        .iter()
        .filter(|i| matches!(i.mnemonic(), Mnemonic::Jmp | Mnemonic::Call))
        .filter(|i| i.op0_kind() == OpKind::NearBranch64)
        .map(|i| i.near_branch_target() as u32)
        .filter(|&t| !(binding..binding + BINDING_MAX).contains(&t))
        .collect();
    one(targets)
}

fn find_sound(pe: &Pe, cases: &[Case], reads: &mut Reads) -> Result<NamedPacket, ScanError> {
    let queue = binding_target(pe, lua_binding(pe, "playSound")?)?;
    let mut found = Vec::new();
    for case in cases {
        let Some(fields) = case.fields else { continue };
        if fields.len() != SOUND_WIDTHS.len() {
            continue;
        }
        let mut flow = Flow::new();
        for i in &case.code {
            if call_target(i) == Some(queue) {
                let roles = [Register::RDX, Register::R8, Register::R9].map(|r| flow.get(r));
                if let [Some(Value::Field(id)), Some(Value::Field(loops)), Some(Value::Field(delay))] =
                    roles
                {
                    let order = [id, loops, delay];
                    let fits = order
                        .iter()
                        .zip(SOUND_WIDTHS)
                        .all(|(&f, w)| fields.get(f).map(Field::width) == Some(w));
                    if fits && id != delay {
                        found.push(NamedPacket {
                            opcode: case.opcode,
                            order: order.map(|f| f as u8).to_vec(),
                        });
                    }
                }
                break;
            }
            flow.step(i, reads);
        }
    }
    name_one(found)
}

fn is_smart_reader(pe: &Pe, function: u32) -> bool {
    let code = whole_function(pe, function, SMART_MAX);
    let splits = code.iter().any(|i| {
        i.mnemonic() == Mnemonic::Cmp
            && i.op0_kind() == OpKind::Register
            && i.op0_register().size() == 1
            && immediate(i) == Some(SMART_LIMIT)
    });
    let biases = code
        .iter()
        .any(|i| immediate(i).is_some_and(|v| v & 0xffff == SMART_BIAS));
    splits && biases
}

fn reaches(pe: &Pe, function: u32, table: u32, depth: usize, seen: &mut HashSet<u32>) -> bool {
    if !seen.insert(function) {
        return false;
    }
    let code = whole_function(pe, function, CALLEE_MAX);
    let touches = code.iter().any(|i| {
        i.is_ip_rel_memory_operand()
            && (table..table + CHAT_TABLE_SPAN).contains(&(i.ip_rel_memory_address() as u32))
    });
    if touches {
        return true;
    }
    if depth == 0 {
        return false;
    }
    let end = function + CALLEE_MAX;
    let next: Vec<u32> = code
        .iter()
        .filter(|i| matches!(i.mnemonic(), Mnemonic::Jmp | Mnemonic::Call))
        .filter(|i| i.op0_kind() == OpKind::NearBranch64)
        .map(|i| i.near_branch_target() as u32)
        .filter(|&t| !(function..end).contains(&t))
        .collect();
    next.into_iter()
        .any(|t| reaches(pe, t, table, depth - 1, seen))
}

fn find_message_game(
    pe: &Pe,
    cases: &[Case],
    reads: &mut Reads,
    chat_messages: u32,
) -> Result<NamedPacket, ScanError> {
    let mut found = Vec::new();
    for case in cases.iter().filter(|case| case.size == -1) {
        let targets: Vec<u32> = calls(&case.code).map(|(_, t)| t).collect();
        let [smart, flag, ..] = targets.as_slice() else {
            continue;
        };
        if !is_smart_reader(pe, *smart) || reads.width(*flag) != Some(FLAG_WIDTH) {
            continue;
        }
        let mut flow = Flow::new();
        let mut flag_register = None;
        let mut checks_flag = false;
        let mut adds = false;
        for i in &case.code {
            if i.mnemonic() == Mnemonic::Cmp
                && i.op0_kind() == OpKind::Register
                && Some(i.op0_register().full_register()) == flag_register
                && immediate(i) == Some(NAME_FLAG)
            {
                checks_flag = true;
            }
            if let Some(target) = call_target(i) {
                let type_in_edx = flow.get(Register::RDX) == Some(Value::Call(0));
                if checks_flag
                    && type_in_edx
                    && reaches(pe, target, chat_messages, CHAT_DEPTH, &mut HashSet::new())
                {
                    adds = true;
                    break;
                }
            }
            flow.step(i, reads);
            if flag_register.is_none() && flow.fields == 1 {
                flag_register = flow
                    .registers
                    .iter()
                    .find(|(&r, &v)| v == Value::Field(0) && r != Register::RAX)
                    .map(|(&r, _)| r);
            }
        }
        if adds {
            found.push(plain(case.opcode));
        }
    }
    name_one(found)
}

pub fn find_packet_names(
    pe: &Pe,
    packets: &Packets,
    layouts: &PacketLayouts,
    anchors: PacketAnchors,
) -> Vec<(Name, Result<NamedPacket, ScanError>)> {
    let cases: Vec<Case> = layouts
        .cases
        .iter()
        .enumerate()
        .map(|(opcode, &begin)| Case {
            opcode: opcode as u16,
            size: layouts.sizes[opcode],
            fields: layouts.layouts[opcode].as_ref(),
            code: case_code(pe, &layouts.cases, begin),
        })
        .collect();
    let mut reads = Reads {
        pe,
        data_at: packets.buffer_data,
        cursor_at: packets.buffer_cursor,
        known: HashMap::new(),
    };
    let missing = Err(ScanError::NotFound);
    Name::ALL
        .into_iter()
        .map(|name| {
            let found = match name {
                Name::Tick => anchors
                    .tick_count
                    .map_or(missing.clone(), |t| find_tick(&cases, t)),
                Name::VarpSmall => anchors
                    .varps
                    .map_or(missing.clone(), |v| find_varps(pe, &cases, v, 3, 1)),
                Name::VarpLarge => anchors
                    .varps
                    .map_or(missing.clone(), |v| find_varps(pe, &cases, v, 6, 4)),
                Name::SynthSound => find_sound(pe, &cases, &mut reads),
                Name::MessageGame => anchors.chat_messages.map_or(missing.clone(), |table| {
                    find_message_game(pe, &cases, &mut reads, table)
                }),
            };
            (name, found)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::offsets::tests::for_each_archived_build;

    const OCTOBER: [(Name, u16, &[u8]); 5] = [
        (Name::Tick, 2, &[]),
        (Name::VarpSmall, 21, &[1, 0]),
        (Name::VarpLarge, 40, &[0, 1]),
        (Name::SynthSound, 69, &[0, 1, 2]),
        (Name::MessageGame, 124, &[]),
    ];

    const NOVEMBER: [(Name, u16, &[u8]); 5] = [
        (Name::Tick, 55, &[]),
        (Name::VarpSmall, 54, &[1, 0]),
        (Name::VarpLarge, 66, &[1, 0]),
        (Name::SynthSound, 30, &[0, 1, 2]),
        (Name::MessageGame, 85, &[]),
    ];

    const NOVEMBER_23: [(Name, u16, &[u8]); 5] = [
        (Name::Tick, 23, &[]),
        (Name::VarpSmall, 83, &[1, 0]),
        (Name::VarpLarge, 90, &[1, 0]),
        (Name::SynthSound, 55, &[0, 1, 2]),
        (Name::MessageGame, 119, &[]),
    ];

    #[test]
    fn names_packets_in_archived_builds() {
        for_each_archived_build(|build, pe| {
            let packets = find_packets(pe).unwrap();
            let layouts = find_packet_layouts(pe, &packets).unwrap();
            let client = find_client(pe, find_game_state(pe).unwrap()).unwrap();
            let anchors = PacketAnchors {
                tick_count: find_tick_count(pe, client).ok(),
                varps: super::super::find_varps(pe).ok(),
                chat_messages: find_chat(pe).ok().map(|chat| chat.messages),
            };
            let file = build.file_name().unwrap().to_string_lossy();
            let expected = if file.starts_with("client-2026-10-") {
                OCTOBER
            } else if file.starts_with("client-2026-11-23-") {
                NOVEMBER_23
            } else {
                NOVEMBER
            };
            let names = find_packet_names(pe, &packets, &layouts, anchors);
            for ((name, found), (want, opcode, order)) in names.into_iter().zip(expected) {
                assert_eq!(name, want);
                assert_eq!(
                    found,
                    Ok(NamedPacket {
                        opcode,
                        order: order.to_vec()
                    }),
                    "{} in {file}",
                    name.spelling()
                );
            }
        });
    }
}
