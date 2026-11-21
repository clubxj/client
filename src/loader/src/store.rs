use std::collections::BTreeMap;

use gpui_kit::{Div, FontWeight, ParentElement, Pixels, Rgba, Styled, div, px};
use serde::{Deserialize, Serialize};

use crate::api::{
    StoreScript,
    models::{credits_label, normalize_share_code, rate_label},
};
use crate::ui::{theme, widgets};

const MAX_CONFIRMED: usize = 500;

#[derive(Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Confirmed(BTreeMap<String, i64>);

impl Confirmed {
    pub fn has(&self, script_id: &str, revision: i64) -> bool {
        self.0.get(script_id) == Some(&revision)
    }

    pub fn confirm(&mut self, script_id: &str, revision: i64) {
        if self.0.len() >= MAX_CONFIRMED
            && !self.0.contains_key(script_id)
            && let Some(first) = self.0.keys().next().cloned()
        {
            self.0.remove(&first);
        }
        self.0.insert(script_id.to_owned(), revision);
    }

    pub fn forget(&mut self, script_id: &str) {
        self.0.remove(script_id);
    }
}

#[derive(Default)]
pub struct Store {
    pub scripts: Vec<StoreScript>,
    pub loading: bool,
    pub loaded: bool,
    pub error: Option<String>,
    pub search: String,
    pub category: String,
    pub community: bool,
    pub code: String,
    pub redeeming: bool,
    pub redeem_result: Option<(bool, String)>,
    pub removing: Option<String>,
}

pub enum Action {
    Refresh,
    Run(Box<StoreScript>),
    Redeem(String),
    RemoveGrant(Box<StoreScript>),
    OpenSite(String),
}

const MIN_DETAILS_WIDTH: Pixels = px(240.0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Placement {
    Beside,
    Stacked,
    Below,
}

impl Placement {
    pub fn of(room: Pixels, buttons: usize, button_width: Pixels) -> Self {
        let row = group_width(buttons, button_width);
        let column = group_width(1, button_width);
        if room - row >= MIN_DETAILS_WIDTH {
            Self::Beside
        } else if buttons > 1 && room - column >= MIN_DETAILS_WIDTH {
            Self::Stacked
        } else {
            Self::Below
        }
    }
}

pub fn group_width(buttons: usize, button_width: Pixels) -> Pixels {
    (button_width + px(1.0)) * buttons as f32 + px(1.0)
}

pub const ICON_WEBSITE: &str = "\u{f0ac}";
pub const ICON_ADD_FUNDS: &str = "\u{f09d}";
pub const COMMUNITY_COLOR: Rgba = Rgba {
    r: 0xff as f32 / 255.0,
    g: 0x9e as f32 / 255.0,
    b: 0x64 as f32 / 255.0,
    a: 1.0,
};

pub fn origin_label(script: &StoreScript) -> &'static str {
    if script.community {
        "community"
    } else {
        "official"
    }
}

pub fn origin_color(script: &StoreScript) -> Rgba {
    if script.community {
        COMMUNITY_COLOR
    } else {
        theme::BLUE
    }
}

pub fn origin_hint(script: &StoreScript) -> &'static str {
    if script.community {
        "Written by a community member, not reviewed by staff. You're warned before it runs."
    } else {
        "Written by a scripter."
    }
}

pub fn price_texts(script: &StoreScript) -> Option<(String, String)> {
    (!script.is_free()).then(|| {
        (
            rate_label(script.price as f64),
            credits_label(script.price as f64 / 60.0),
        )
    })
}

pub fn price(script: &StoreScript) -> Div {
    match price_texts(script) {
        Some((hour, minute)) => div()
            .flex()
            .flex_col()
            .items_start()
            .gap(theme::LINE_GAP)
            .child(
                widgets::credits(hour, "/h", false)
                    .text_size(theme::HEADING)
                    .font_weight(FontWeight::BOLD),
            )
            .child(widgets::credits(minute, "/min", false).text_size(theme::SMALL)),
        None => widgets::strong("Free").text_color(theme::GREEN),
    }
}

const INVALID_SHARE_CODE: &str =
    "A share code is 64 characters: the digits 0-9 and the letters a-f.";

impl Store {
    pub fn categories(&self) -> Vec<String> {
        let mut all: Vec<String> = self
            .scripts
            .iter()
            .flat_map(|s| s.categories.iter().cloned())
            .collect();
        all.sort();
        all.dedup();
        all
    }

    pub fn visible(&self) -> impl Iterator<Item = &StoreScript> {
        let search = self.search.trim().to_lowercase();
        self.scripts.iter().filter(move |s| {
            (self.category.is_empty() || s.categories.contains(&self.category))
                && (search.is_empty()
                    || s.title.to_lowercase().contains(&search)
                    || s.description.to_lowercase().contains(&search))
        })
    }

    pub fn can_redeem(&self) -> bool {
        !self.redeeming && !self.code.trim().is_empty()
    }

    pub fn redeem(&mut self) -> Option<Action> {
        if self.redeeming {
            return None;
        }
        let code = normalize_share_code(&self.code);
        if code.is_none() {
            self.redeem_result = Some((false, INVALID_SHARE_CODE.into()));
        }
        code.map(Action::Redeem)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(title: &str, price: i64, categories: &[&str]) -> StoreScript {
        StoreScript {
            id: title.into(),
            url: title.into(),
            title: title.into(),
            description: String::new(),
            categories: categories.iter().map(|c| (*c).into()).collect(),
            stage: "stable".into(),
            price,
            revision: Some(1),
            cover_url: None,
            can_access: true,
            author: "a".into(),
            online: 0,
            community: false,
            granted: false,
            queueable: false,
        }
    }

    #[test]
    fn scripts_are_tagged_by_where_they_come_from() {
        let mut community = script("c", 0, &[]);
        community.community = true;
        assert_eq!(origin_label(&community), "community");
        assert_eq!(origin_label(&script("o", 0, &[])), "official");
    }

    #[test]
    fn a_warning_holds_for_one_revision_of_one_script() {
        let mut confirmed = Confirmed::default();
        assert!(!confirmed.has("a", 1));
        confirmed.confirm("a", 1);
        assert!(confirmed.has("a", 1));
        assert!(!confirmed.has("a", 2), "a new revision warns again");
        assert!(!confirmed.has("b", 1), "so does a different script");
        confirmed.confirm("a", 2);
        assert!(
            confirmed.has("a", 2) && !confirmed.has("a", 1),
            "only the latest confirmed revision is kept"
        );
        confirmed.forget("a");
        assert!(!confirmed.has("a", 2));
    }

    #[test]
    fn confirmations_survive_storage_and_stay_bounded() {
        let mut confirmed = Confirmed::default();
        for i in 0..(MAX_CONFIRMED + 20) {
            confirmed.confirm(&format!("script-{i:04}"), 1);
        }
        assert_eq!(confirmed.0.len(), MAX_CONFIRMED);
        let again: Confirmed =
            serde_json::from_str(&serde_json::to_string(&confirmed).unwrap()).unwrap();
        assert_eq!(again, confirmed);
        let before = confirmed.0.len();
        confirmed.confirm("script-0019", 2);
        assert_eq!(confirmed.0.len(), before);
    }

    #[test]
    fn prices_read_as_credit_amounts() {
        assert_eq!(price_texts(&script("a", 0, &[])), None);
        assert_eq!(
            price_texts(&script("a", 180_000, &[])),
            Some(("0.18\u{a0}CREDIT".into(), "0.003\u{a0}CREDIT".into()))
        );
        assert_eq!(
            price_texts(&script("a", 70_000, &[])),
            Some(("0.07\u{a0}CREDIT".into(), "0.001167\u{a0}CREDIT".into()))
        );
    }

    #[test]
    fn card_buttons_move_under_the_details_as_the_card_narrows() {
        let width = px(80.0);
        assert_eq!(Placement::of(px(500.0), 2, width), Placement::Beside);
        assert_eq!(Placement::of(px(330.0), 2, width), Placement::Stacked);
        assert_eq!(Placement::of(px(330.0), 1, width), Placement::Beside);
        assert_eq!(Placement::of(px(300.0), 2, width), Placement::Below);
    }

    #[test]
    fn only_a_well_formed_share_code_is_redeemed() {
        let mut store = Store {
            code: "not a code".into(),
            ..Store::default()
        };
        assert!(store.redeem().is_none());
        assert_eq!(
            store.redeem_result,
            Some((false, INVALID_SHARE_CODE.into()))
        );
        store.code = "ab".repeat(32);
        assert!(matches!(store.redeem(), Some(Action::Redeem(code)) if code == "ab".repeat(32)));
        store.redeeming = true;
        assert!(store.redeem().is_none() && !store.can_redeem());
    }

    #[test]
    fn search_and_category_filter_the_list() {
        let mut store = Store {
            scripts: vec![
                script("Tempoross", 0, &["fishing"]),
                script("Miner", 5, &["mining"]),
            ],
            ..Store::default()
        };
        assert_eq!(store.visible().count(), 2);
        store.search = "temp".into();
        assert_eq!(
            store
                .visible()
                .map(|s| s.title.as_str())
                .collect::<Vec<_>>(),
            ["Tempoross"]
        );
        store.search.clear();
        store.category = "mining".into();
        assert_eq!(
            store
                .visible()
                .map(|s| s.title.as_str())
                .collect::<Vec<_>>(),
            ["Miner"]
        );
        assert_eq!(store.categories(), ["fishing", "mining"]);
    }
}
