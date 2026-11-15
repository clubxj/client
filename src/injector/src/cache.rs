use serde::{Deserialize, Serialize};

pub const CONFIG_INDEX: u32 = 2;
pub const MAPS_INDEX: u32 = 5;
pub const GAME_VALS_INDEX: u32 = 24;
pub const FRAMES_INDEX: u32 = 0;
pub const FRAME_MAPS_INDEX: u32 = 1;
pub const MODELS_INDEX: u32 = 7;
pub const NPC_ARCHIVE: u32 = 9;
pub const SEQUENCE_ARCHIVE: u32 = 12;
pub const LOC_ARCHIVE: i32 = 6;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheIds {
    pub ids: Vec<u32>,
    pub fingerprint: u64,
}

pub fn game_val_name(kind: u32, data: &[u8]) -> &[u8] {
    let data = match kind {
        ipc::state::game_val::TABLE => data.get(1..).unwrap_or_default(),
        ipc::state::game_val::INTERFACE | 13 => data,
        _ => return data,
    };
    &data[..data.iter().position(|&b| b == 0).unwrap_or(data.len())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc::state::game_val;

    #[test]
    fn parses_each_game_val_format() {
        assert_eq!(
            game_val_name(game_val::OBJ, b"bucket_empty"),
            b"bucket_empty"
        );
        assert_eq!(
            game_val_name(game_val::INTERFACE, b"inventory\0\0\0items\0\xff\xff"),
            b"inventory"
        );
        assert_eq!(
            game_val_name(game_val::TABLE, b"\x01quests\0\x01name\0\0"),
            b"quests"
        );
        assert_eq!(game_val_name(game_val::INTERFACE, b""), b"");
    }
}
