use serde::{Deserialize, Serialize};

pub mod request {
    pub const PLAYER_PLANE: i32 = 0xff;
    pub const ALL_PLANES: i32 = 4;
    pub const CLICKABLE_ONLY: i32 = 0x100;
    pub const PLANES: i32 = 0xff;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loc {
    pub id: i32,
    pub x: u8,
    pub y: u8,
    pub size_x: u8,
    pub size_y: u8,
    pub kind: u8,
    pub shape: Option<u8>,
    pub rotation: Option<u8>,
    pub plane: u8,
    pub clickable: bool,
    pub name: Option<u16>,
    pub decoration_height: i16,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Locs {
    pub plane: i32,
    pub base_x: i32,
    pub base_y: i32,
    pub locs: Vec<Loc>,
    pub names: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unavailable {
    NotFound,
    NotInGame,
    DefinitionsLoading,
    DefinitionsFailed,
}
