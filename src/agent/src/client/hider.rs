use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    thread::sleep,
    time::{Duration, Instant},
};

use decode::world::{self, ActorObject};
use ipc::{
    read::{self, Memory},
    state::{game_state, hide, hider_status, part, HiderStatus, Layout},
};

use super::{
    offsets::{loc_kind, Hider},
    own::{OwnMemory, LAYOUT},
    state::mapping,
};

const THRALLS: [i32; 18] = [
    10878, 10879, 10880, 10881, 10882, 10883, 10884, 10885, 10886, 15701, 15702, 15703, 15704,
    15705, 15706, 15707, 15708, 15709,
];
const RANDOM_EVENTS: [i32; 41] = [
    307, 312, 314, 322, 326, 327, 375, 376, 380, 390, 5426, 5429, 5430, 5431, 5432, 5436, 5437,
    5438, 5439, 5440, 5441, 5510, 6738, 6743, 6744, 6746, 6747, 6748, 6749, 6750, 6751, 6752, 6753,
    6754, 6755, 12551, 12552, 13443, 13444, 13445, 13446,
];
const PLAYER_BITS: u64 = hide::OTHERS
    | hide::OTHERS_2D
    | hide::FRIENDS
    | hide::FRIENDS_CHAT
    | hide::CLAN
    | hide::IGNORED
    | hide::LOCAL
    | hide::LOCAL_2D
    | hide::ATTACKERS;
const NPC_BITS: u64 = hide::NPCS
    | hide::NPCS_2D
    | hide::PETS
    | hide::THRALLS
    | hide::DEAD_NPCS
    | hide::RANDOM_EVENTS
    | hide::ATTACKERS;
const SCENE_BITS: u64 = hide::WALLS
    | hide::WALL_DECORATIONS
    | hide::GROUND_DECORATIONS
    | hide::OBJECTS
    | hide::GROUND_ITEMS;
const ENTITY_SIZE: usize = 32;
const STOP_TIMEOUT: Duration = Duration::from_secs(1);

type Predicate = unsafe extern "system" fn(*const u8) -> u8;

struct Rule {
    hider: Hider,
    friend: Predicate,
    friends_chat: Predicate,
    clan: Predicate,
    ignored: Predicate,
    follower: Predicate,
    local_follower: Predicate,
    dead: Predicate,
}

#[derive(Default)]
struct Tracked {
    hidden: HashMap<u64, u64>,
    hidden_2d: HashMap<u64, u64>,
}

static RULE: OnceLock<Rule> = OnceLock::new();
static MASK: AtomicU64 = AtomicU64::new(0);
static ADOPTED: AtomicU32 = AtomicU32::new(0);
static PUBLISHED: Mutex<Option<HiderStatus>> = Mutex::new(None);
static TRACKED: Mutex<Option<Tracked>> = Mutex::new(None);
static STOPPING: AtomicBool = AtomicBool::new(false);
static STOPPED: AtomicBool = AtomicBool::new(false);

pub fn set_rule(module_base: u64, hider: Hider) {
    let at = |rva: u32| unsafe { std::mem::transmute::<u64, Predicate>(module_base + rva as u64) };
    let _ = RULE.set(Rule {
        hider,
        friend: at(hider.friend),
        friends_chat: at(hider.friends_chat),
        clan: at(hider.clan),
        ignored: at(hider.ignored),
        follower: at(hider.follower),
        local_follower: at(hider.local_follower),
        dead: at(hider.dead),
    });
}

fn ask(rule: &Rule, predicate: Predicate, index: i32) -> bool {
    let mut entity = [0u8; ENTITY_SIZE];
    let at = rule.hider.entity_index as usize;
    entity[at..at + 4].copy_from_slice(&index.to_le_bytes());
    unsafe { predicate(entity.as_ptr()) != 0 }
}

fn when(mask: u64, bit: u64) -> u64 {
    mask & bit
}

fn player_reasons(rule: &Rule, mask: u64, local: i32, player: &ActorObject) -> (u64, u64) {
    if mask & PLAYER_BITS == 0 {
        return (0, 0);
    }
    if player.index == local {
        return (when(mask, hide::LOCAL), when(mask, hide::LOCAL_2D));
    }
    if mask & hide::ATTACKERS != 0 && player.interacting_player == local {
        return (hide::ATTACKERS, hide::ATTACKERS);
    }
    let groups = [
        (rule.friend, hide::FRIENDS),
        (rule.friends_chat, hide::FRIENDS_CHAT),
        (rule.clan, hide::CLAN),
        (rule.ignored, hide::IGNORED),
    ];
    for (predicate, bit) in groups {
        if ask(rule, predicate, player.index) {
            return (when(mask, bit), when(mask, bit));
        }
    }
    (when(mask, hide::OTHERS), when(mask, hide::OTHERS_2D))
}

fn npc_reasons(rule: &Rule, mask: u64, local: i32, npc: &ActorObject) -> (u64, u64) {
    if mask & NPC_BITS == 0 {
        return (0, 0);
    }
    if ask(rule, rule.follower, npc.index) && !ask(rule, rule.local_follower, npc.index) {
        return (when(mask, hide::PETS), when(mask, hide::PETS));
    }
    if mask & hide::DEAD_NPCS != 0 && ask(rule, rule.dead, npc.index) {
        return (hide::DEAD_NPCS, hide::DEAD_NPCS);
    }
    if npc.interacting_player == local {
        let attackers = when(mask, hide::ATTACKERS);
        if mask & (hide::NPCS | hide::NPCS_2D) == 0 {
            return (attackers, attackers);
        }
        let only = |bit: u64| if mask & bit != 0 { attackers } else { 0 };
        return (only(hide::NPCS), only(hide::NPCS_2D));
    }
    if THRALLS.contains(&npc.id) {
        return (when(mask, hide::THRALLS), when(mask, hide::THRALLS));
    }
    if RANDOM_EVENTS.contains(&npc.id) {
        return (
            when(mask, hide::RANDOM_EVENTS),
            when(mask, hide::RANDOM_EVENTS),
        );
    }
    (when(mask, hide::NPCS), when(mask, hide::NPCS_2D))
}

fn scene_bit(kind: u8) -> u64 {
    match kind {
        loc_kind::WALL => hide::WALLS,
        loc_kind::WALL_DECORATION => hide::WALL_DECORATIONS,
        loc_kind::GROUND_DECORATION => hide::GROUND_DECORATIONS,
        loc_kind::GAME => hide::OBJECTS,
        _ => hide::GROUND_ITEMS,
    }
}

#[derive(Default)]
struct Wanted {
    hidden: HashMap<u64, u64>,
    hidden_2d: HashMap<u64, u64>,
}

impl Wanted {
    fn add(map: &mut HashMap<u64, u64>, address: u64, reason: u64) {
        let entry = map.entry(address).or_insert(0);
        if *entry == 0 {
            *entry = reason;
        }
    }

    fn hide(&mut self, address: u64, reason: u64) {
        Self::add(&mut self.hidden, address, reason);
    }

    fn hide_2d(&mut self, address: u64, reason: u64) {
        Self::add(&mut self.hidden_2d, address, reason);
    }
}

fn actors(rule: &Rule, layout: &Layout, mask: u64, wanted: &mut Wanted) {
    let memory = OwnMemory;
    let Some(local) = read::client(&memory, layout)
        .and_then(|client| memory.i32(client + layout.player.player_index as u64))
    else {
        return;
    };
    let players = world::player_objects(&memory, &layout.world).unwrap_or_default();
    for player in &players {
        let (reason, reason_2d) = player_reasons(rule, mask, local, player);
        wanted.hide(player.address, reason);
        wanted.hide_2d(player.address, reason_2d);
    }
    let npcs = world::npc_objects(&memory, &layout.world).unwrap_or_default();
    for npc in &npcs {
        let (reason, reason_2d) = npc_reasons(rule, mask, local, npc);
        wanted.hide(npc.address, reason);
        wanted.hide_2d(npc.address, reason_2d);
    }
}

fn walked(mask: u64, tracked: &Tracked) -> u64 {
    let reasons = tracked
        .hidden
        .values()
        .chain(tracked.hidden_2d.values())
        .fold(mask, |all, &reason| all | reason);
    let mut kinds = 0;
    for bits in [
        PLAYER_BITS | NPC_BITS,
        hide::PROJECTILES,
        hide::SPOT_ANIMS,
        SCENE_BITS,
    ] {
        if reasons & bits != 0 {
            kinds |= bits;
        }
    }
    kinds
}

fn collect(rule: &Rule, layout: &Layout, mask: u64, walk: u64) -> Wanted {
    let memory = OwnMemory;
    let mut wanted = Wanted::default();
    if walk & (PLAYER_BITS | NPC_BITS) != 0 && layout.has(part::WORLD) {
        actors(rule, layout, mask, &mut wanted);
    }
    if walk & hide::PROJECTILES != 0 && layout.has(part::PROJECTILES) {
        for projectile in world::projectile_objects(&memory, layout).unwrap_or_default() {
            wanted.hide(projectile, when(mask, hide::PROJECTILES));
        }
    }
    if walk & hide::SPOT_ANIMS != 0 && layout.has(part::SPOT_ANIMS) {
        for spot in world::spot_anim_objects(&memory, layout).unwrap_or_default() {
            wanted.hide(spot, when(mask, hide::SPOT_ANIMS));
        }
    }
    if walk & SCENE_BITS != 0 {
        super::scene::visit_renderables(|kind, renderable| {
            wanted.hide(renderable, when(mask, scene_bit(kind)));
        });
    }
    wanted
}

fn apply(
    tracked: &mut HashMap<u64, u64>,
    wanted: &HashMap<u64, u64>,
    field: u32,
    hidden_value: u8,
    walk: u64,
) {
    let memory = OwnMemory;
    let shown_value = hidden_value ^ 1;
    tracked.retain(|address, reason| *reason & walk == 0 || wanted.contains_key(address));
    for (&address, &reason) in wanted {
        let flag = address + field as u64;
        match (reason != 0, tracked.contains_key(&address)) {
            (true, true) => {
                tracked.insert(address, reason);
            }
            (true, false) => {
                let mut current = [0];
                if memory.read(flag, &mut current)
                    && current[0] == shown_value
                    && memory.write(flag, &[hidden_value])
                {
                    tracked.insert(address, reason);
                }
            }
            (false, true) => {
                memory.write(flag, &[shown_value]);
                tracked.remove(&address);
            }
            (false, false) => {}
        }
    }
}

fn adopt_request() {
    let Some(mapping) = mapping() else {
        return;
    };
    let Some((sequence, request)) = mapping.shared().read_hider_request() else {
        return;
    };
    if sequence != 0 && sequence != ADOPTED.swap(sequence, Ordering::AcqRel) {
        MASK.store(request.mask & hide::ALL, Ordering::Release);
    }
}

fn publish(status: i32, mask: u64) {
    let Some(mapping) = mapping() else {
        return;
    };
    let status = HiderStatus {
        sequence: ADOPTED.load(Ordering::Acquire),
        status,
        mask,
    };
    let Ok(mut published) = PUBLISHED.lock() else {
        return;
    };
    if *published != Some(status) {
        mapping.shared().write_hider_status(&status);
        *published = Some(status);
    }
}

pub fn on_frame() {
    adopt_request();
    let stopping = STOPPING.load(Ordering::Acquire);
    let mask = if stopping {
        0
    } else {
        MASK.load(Ordering::Acquire)
    };
    let Some(rule) = RULE.get() else {
        publish(hider_status::NOT_FOUND, 0);
        return;
    };
    publish(
        if mask == 0 {
            hider_status::OFF
        } else {
            hider_status::ON
        },
        mask,
    );
    let Ok(mut tracked) = TRACKED.lock() else {
        return;
    };
    let tracked = tracked.get_or_insert_with(Tracked::default);
    let memory = OwnMemory;
    let layout = LAYOUT
        .get()
        .filter(|layout| read::current_game_state(&memory, layout) == Some(game_state::LOGGED_IN));
    let walk = walked(mask, tracked);
    match layout {
        Some(layout) if walk != 0 => {
            let wanted = collect(rule, layout, mask, walk);
            apply(&mut tracked.hidden, &wanted.hidden, rule.hider.hidden, 1, walk);
            apply(
                &mut tracked.hidden_2d,
                &wanted.hidden_2d,
                rule.hider.shows_2d,
                0,
                walk,
            );
        }
        Some(_) => {}
        None => {
            tracked.hidden.clear();
            tracked.hidden_2d.clear();
        }
    }
    if stopping {
        STOPPED.store(
            tracked.hidden.is_empty() && tracked.hidden_2d.is_empty(),
            Ordering::Release,
        );
    }
}

pub fn reveal(addresses: &[u64]) {
    let Ok(tracked) = TRACKED.lock() else {
        return;
    };
    let Some(tracked) = tracked.as_ref() else {
        return;
    };
    let reasons = addresses
        .iter()
        .filter_map(|address| tracked.hidden.get(address))
        .fold(0, |all, &reason| all | reason);
    if reasons != 0 {
        MASK.fetch_and(!reasons, Ordering::AcqRel);
    }
}

pub fn stop() {
    let idle = TRACKED
        .lock()
        .map(|tracked| {
            tracked
                .as_ref()
                .is_none_or(|t| t.hidden.is_empty() && t.hidden_2d.is_empty())
        })
        .unwrap_or(true);
    if idle || !super::frame::active() {
        return;
    }
    STOPPING.store(true, Ordering::Release);
    let start = Instant::now();
    while !STOPPED.load(Ordering::Acquire) {
        if start.elapsed() > STOP_TIMEOUT {
            println!("[Agent]: No frame was drawn to show hidden entities again, leaving them hidden.\r\n");
            return;
        }
        sleep(Duration::from_millis(10));
    }
}
