//! Settings pushed in by the shell (it owns persistence); read from any thread.

use std::sync::Mutex;

use nextype_core::{FieldSnapshot, domain_matches};
use std::sync::atomic::{AtomicBool, AtomicUsize};

pub static ENABLED: AtomicBool = AtomicBool::new(true);
/// Recent clipboard text goes into the prompt (spec §7.3; off unless the user turns it on).
pub static CLIPBOARD: AtomicBool = AtomicBool::new(false);
/// Text read off the screen above the field goes into the prompt (spec §7.2; off by default).
pub static SCREEN: AtomicBool = AtomicBool::new(false);
/// Save what the user writes and show the model examples of it (spec §7.4; off by default).
pub static HISTORY: AtomicBool = AtomicBool::new(false);
/// Longest ghost in tokens: short / medium / long (spec §9.4; the values are our guesses).
pub static MAX_TOKENS: AtomicUsize = AtomicUsize::new(12);
/// Global custom instructions (spec §8.4).
pub static INSTRUCTIONS: Mutex<String> = Mutex::new(String::new());
/// Per-app settings the user changed (spec §14.2, a subset of its fields).
#[derive(Clone, Debug, Default)]
pub struct AppOverride {
    /// A bundle id, or `domain:<host>` for a website (matches its subdomains too).
    pub bundle_id: String,
    pub completions_off: bool,
    pub tab_off: bool,
    /// Added after the global custom instructions.
    pub instructions: String,
}

pub static OVERRIDES: Mutex<Vec<AppOverride>> = Mutex::new(Vec::new());

/// The field's website override if there is one, else its app's (spec §14.2 resolution order).
pub fn override_for(s: &FieldSnapshot) -> Option<AppOverride> {
    let list = OVERRIDES.lock().ok()?;
    let site = list.iter().find(|o| o.bundle_id.strip_prefix("domain:").is_some_and(|k| domain_matches(k, &s.domain)));
    site.or_else(|| list.iter().find(|o| o.bundle_id == s.bundle_id)).cloned()
}

/// Website of the last field read, for the menu ("Turn Off on <domain>").
pub static CURRENT_DOMAIN: Mutex<String> = Mutex::new(String::new());
