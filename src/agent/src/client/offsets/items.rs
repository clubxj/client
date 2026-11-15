use super::*;

pub(super) fn usertype_member(pe: &Pe, usertype: &str, member: &str) -> Result<u32, ScanError> {
    let usertypes = string_rvas(pe, usertype);
    let members = string_rvas(pe, member);

    let mut found = Vec::new();
    for &(begin, end) in &pe.functions {
        let instrs = function_instrs(pe, begin, end);
        if !instrs
            .iter()
            .filter_map(rip_lea)
            .any(|rva| usertypes.contains(&rva))
        {
            continue;
        }
        for (at, instr) in instrs.iter().enumerate() {
            if !rip_lea(instr).is_some_and(|rva| members.contains(&rva)) {
                continue;
            }
            let code = instrs[at.saturating_sub(8)..at]
                .iter()
                .rev()
                .filter_map(rip_lea)
                .find(|&rva| pe.in_code(rva as usize));
            found.extend(code);
        }
    }
    one(found)
}
