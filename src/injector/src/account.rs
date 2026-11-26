use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};

pub const PREFIX: &str = "\u{1e}ACCOUNTS ";

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub off: Vec<String>,
    pub house: Vec<String>,
}

pub type Accounts = BTreeMap<String, Settings>;

static ACCOUNTS: Mutex<Accounts> = Mutex::new(BTreeMap::new());

pub fn line(accounts: &Accounts) -> String {
    format!(
        "{PREFIX}{}",
        serde_json::to_string(accounts).expect("accounts serialize")
    )
}

pub fn parse(payload: &str) -> Option<Accounts> {
    serde_json::from_str(payload.trim()).ok()
}

pub fn set(accounts: Accounts) {
    *ACCOUNTS.lock().unwrap_or_else(PoisonError::into_inner) = accounts;
}

pub fn settings_for(account: &str) -> Option<Settings> {
    let hash = crate::bank_pin::hash_name(account);
    ACCOUNTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&hash)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_found_by_the_hash_of_the_account_name() {
        let saved = Settings {
            off: vec!["items".into()],
            house: vec!["glory".into()],
        };
        let accounts = Accounts::from([(crate::bank_pin::hash_name("some_name"), saved.clone())]);
        let line = line(&accounts);
        set(parse(line.strip_prefix(PREFIX).unwrap()).unwrap());
        assert_eq!(settings_for("Some Name"), Some(saved));
        assert_eq!(settings_for("other"), None);
    }
}
