use serde::{Deserialize, Serialize};

use crate::Loc;

pub const EXIT_PORTAL: i32 = 4525;

pub const PIECES: &[(&str, &str, &[&str])] = &[
    ("box", "Basic jewellery box", &["Jewellery Box"]),
    ("glory", "Mounted glory", &["Amulet of Glory"]),
    ("cape", "Mounted mythical cape", &["Mythical cape"]),
    (
        "talisman",
        "Mounted Xeric's talisman",
        &["Xeric's Talisman"],
    ),
    ("digsite", "Mounted digsite pendant", &["Digsite Pendant"]),
    (
        "fairy_ring",
        "Fairy ring",
        &["Fairy ring", "Spiritual Fairy Tree"],
    ),
    (
        "spirit_tree",
        "Spirit tree",
        &["Spirit tree", "Spiritual Fairy Tree"],
    ),
    ("pool", "Restoration pool", &["Pool of"]),
    ("occult_altar", "Occult altar", &["Altar of the Occult"]),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Survey {
    pub inside: bool,
    pub pieces: Vec<String>,
}

fn named(name: &str, parts: &[&str]) -> bool {
    let name = name.to_lowercase();
    parts.iter().any(|part| name.contains(&part.to_lowercase()))
}

pub fn survey(in_instance: bool, locs: &[Loc]) -> Survey {
    let inside = in_instance && locs.iter().any(|loc| loc.id == EXIT_PORTAL);
    let pieces = if inside {
        PIECES
            .iter()
            .filter(|(_, _, parts)| {
                locs.iter().any(|loc| {
                    loc.clickable && loc.name.as_deref().is_some_and(|name| named(name, parts))
                })
            })
            .map(|(kind, _, _)| (*kind).to_owned())
            .collect()
    } else {
        Vec::new()
    };
    Survey { inside, pieces }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(id: i32, name: &str) -> Loc {
        Loc {
            id,
            name: Some(name.to_owned()),
            kind: 10,
            x: 0,
            y: 0,
            plane: 0,
            scene_x: 0,
            scene_y: 0,
            size_x: 1,
            size_y: 1,
            shape: None,
            rotation: None,
            clickable: true,
        }
    }

    #[test]
    fn only_a_house_with_its_exit_portal_is_read() {
        let locs = [
            loc(EXIT_PORTAL, "Portal"),
            loc(1, "Ornate Jewellery Box"),
            loc(2, "Spiritual Fairy Tree"),
            loc(3, "Ornate pool of Rejuvenation"),
        ];
        let read = survey(true, &locs);
        assert!(read.inside);
        assert_eq!(read.pieces, ["box", "fairy_ring", "spirit_tree", "pool"]);
        assert_eq!(
            survey(false, &locs),
            Survey {
                inside: false,
                pieces: Vec::new()
            }
        );
        assert!(!survey(true, &locs[1..]).inside);
    }
}
