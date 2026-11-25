use super::*;

const RENDER_3D: &str = "render3D";
const RENDER_2D: &str = "render2D";
const RENDER_PROJECTILES: &str = "renderProjectiles";
const FRIEND: &str = "isOnlineFriend";
const FRIENDS_CHAT: &str = "isFriendChat";
const CLAN: &str = "isGroup";
const IGNORED: &str = "isIgnored";
const FOLLOWER: &str = "isFollower";
const LOCAL_FOLLOWER: &str = "isLocalFollower";
const DEAD: &str = "isDead";
const SETTER_LIMIT: usize = 40;
const HEAD_LIMIT: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hider {
    pub hidden: u32,
    pub shows_2d: u32,
    pub entity_index: u32,
    pub friend: u32,
    pub friends_chat: u32,
    pub clan: u32,
    pub ignored: u32,
    pub follower: u32,
    pub local_follower: u32,
    pub dead: u32,
}

fn byte_store(i: &Instruction) -> Option<(Register, u32, Register)> {
    (i.mnemonic() == Mnemonic::Mov
        && i.op0_kind() == OpKind::Memory
        && i.memory_size() == iced_x86::MemorySize::UInt8
        && i.memory_index() == Register::None
        && i.op1_kind() == OpKind::Register)
        .then(|| {
            (
                i.memory_base().full_register(),
                i.memory_displacement64() as u32,
                i.op1_register().full_register(),
            )
        })
}

fn negated_store(instrs: &[Instruction]) -> Option<u32> {
    let mut negated = HashSet::new();
    for i in instrs {
        if i.mnemonic() == Mnemonic::Xor
            && i.op0_kind() == OpKind::Register
            && immediate(i) == Some(1)
        {
            negated.insert(i.op0_register().full_register());
        }
        if let Some((_, field, _)) = byte_store(i).filter(|(_, _, v)| negated.contains(v)) {
            return Some(field);
        }
    }
    None
}

fn actor_store(instrs: &[Instruction]) -> Option<u32> {
    let mut this = HashSet::from([Register::RCX]);
    let mut value = HashSet::new();
    for i in instrs {
        if i.op0_kind() == OpKind::Register && i.op1_kind() == OpKind::Register {
            let (to, from) = (i.op0_register().full_register(), i.op1_register());
            match i.mnemonic() {
                Mnemonic::Mov if this.contains(&from.full_register()) => {
                    this.insert(to);
                }
                Mnemonic::Movzx if from == Register::DL => {
                    value.insert(to);
                }
                _ => {}
            }
        }
        if let Some((base, field, stored)) = byte_store(i) {
            if value.contains(&stored) && !this.contains(&base) {
                return Some(field);
            }
        }
    }
    None
}

fn entity_index(pe: &Pe, predicate: u32) -> Option<u32> {
    decode_at(pe, predicate, HEAD_LIMIT).iter().find_map(|i| {
        (i.op1_kind() == OpKind::Memory
            && i.memory_base() == Register::RCX
            && i.memory_index() == Register::None
            && i.memory_size().size() == 4)
            .then(|| i.memory_displacement64() as u32)
    })
}

pub fn find_hider(pe: &Pe) -> Result<Hider, ScanError> {
    let render_3d = negated_store(&decode_at(pe, lua_binding(pe, RENDER_3D)?, SETTER_LIMIT));
    let projectiles = negated_store(&decode_at(
        pe,
        lua_binding(pe, RENDER_PROJECTILES)?,
        SETTER_LIMIT,
    ));
    let hidden = match (render_3d, projectiles) {
        (Some(actor), Some(projectile)) if actor == projectile => actor,
        _ => return Err(ScanError::NotFound),
    };
    let shows_2d = actor_store(&decode_at(pe, lua_binding(pe, RENDER_2D)?, SETTER_LIMIT))
        .ok_or(ScanError::NotFound)?;
    let friend = lua_binding(pe, FRIEND)?;
    let friends_chat = lua_binding(pe, FRIENDS_CHAT)?;
    let clan = lua_binding(pe, CLAN)?;
    let ignored = lua_binding(pe, IGNORED)?;
    let follower = lua_binding(pe, FOLLOWER)?;
    let local_follower = lua_binding(pe, LOCAL_FOLLOWER)?;
    let dead = lua_binding(pe, DEAD)?;
    let indices: HashSet<Option<u32>> = [
        friend,
        friends_chat,
        clan,
        ignored,
        follower,
        local_follower,
    ]
    .into_iter()
    .map(|predicate| entity_index(pe, predicate))
    .collect();
    let entity_index = match indices.into_iter().collect::<Vec<_>>().as_slice() {
        [Some(index)] => *index,
        _ => return Err(ScanError::NotFound),
    };
    Ok(Hider {
        hidden,
        shows_2d,
        entity_index,
        friend,
        friends_chat,
        clan,
        ignored,
        follower,
        local_follower,
        dead,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::{for_each_archived_build, found};
    use super::*;

    #[test]
    fn finds_hider_in_archived_builds() {
        for_each_archived_build(|build, pe| {
            let hider = found("hider", build, find_hider(pe));
            if build.to_string_lossy().contains("osclient-243.1") {
                assert_eq!(
                    hider,
                    Hider {
                        hidden: 0x1c,
                        shows_2d: 0x3a0,
                        entity_index: 8,
                        friend: 0x34da70,
                        friends_chat: 0x34dba0,
                        clan: 0x34dad0,
                        ignored: 0x34db30,
                        follower: 0x34ad60,
                        local_follower: 0x34add0,
                        dead: 0x17c180,
                    },
                    "{}",
                    build.display()
                );
            }
        });
    }

}
