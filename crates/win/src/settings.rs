//! User settings, saved as JSON in `%LOCALAPPDATA%\NexType\settings.json`. The Windows twin of
//! the Mac shell's UserDefaults plus `crates/app/src/settings.rs`: global options and per-app
//! rules. The settings window edits the whole struct; the runtime asks the `*_for` helpers.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use nextype_core::OFF_BY_DEFAULT;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Length {
    Short,
    #[default]
    Medium,
    Long,
}

impl Length {
    /// Longest ghost in tokens (the Mac's values).
    pub fn tokens(self) -> usize {
        match self {
            Length::Short => 8,
            Length::Medium => 16,
            Length::Long => 32,
        }
    }
}

/// One app's own settings. `None` follows the global setting (or the app's default).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AppRule {
    /// Executable name, lowercase (`notepad.exe`): the app's key, as `FieldSnapshot::bundle_id`.
    pub exe: String,
    /// Display name for the settings list.
    pub name: String,
    pub completions: Option<bool>,
    /// The Tab shortcut; off leaves Tab to the app (the key above Tab still accepts).
    pub tab: Option<bool>,
    pub type_spaces_separately: Option<bool>,
    /// Added to the global instructions in this app.
    pub instructions: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    /// Unix seconds; completions are paused until then.
    pub paused_until: Option<u64>,
    pub open_at_login: bool,
    pub length: Length,
    pub instructions: String,
    pub type_spaces_separately: bool,
    /// Context lines in the prompt (spec §7.1).
    pub context_name: bool,
    pub context_app: bool,
    pub context_date: bool,
    pub apps: Vec<AppRule>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            paused_until: None,
            open_at_login: false,
            length: Length::Medium,
            instructions: String::new(),
            type_spaces_separately: true,
            context_name: true,
            context_app: true,
            context_date: true,
            apps: Vec::new(),
        }
    }
}

impl Settings {
    pub fn rule(&self, exe: &str) -> Option<&AppRule> {
        self.apps.iter().find(|r| r.exe.eq_ignore_ascii_case(exe))
    }

    /// The rule for `exe`, added if missing.
    pub fn rule_mut(&mut self, exe: &str, name: &str) -> &mut AppRule {
        let i = match self.apps.iter().position(|r| r.exe.eq_ignore_ascii_case(exe)) {
            Some(i) => i,
            None => {
                self.apps.push(AppRule { exe: exe.to_lowercase(), name: name.to_owned(), ..Default::default() });
                self.apps.len() - 1
            }
        };
        &mut self.apps[i]
    }

    pub fn paused(&self, now: u64) -> bool {
        self.paused_until.is_some_and(|t| now < t)
    }

    /// Whether NexType may complete in `exe`, or why not.
    pub fn completions_for(&self, exe: &str, now: u64) -> Result<(), &'static str> {
        if !self.enabled {
            return Err("completions off");
        }
        if self.paused(now) {
            return Err("paused");
        }
        let default_on = !OFF_BY_DEFAULT.contains(&exe);
        match self.rule(exe).and_then(|r| r.completions).unwrap_or(default_on) {
            true => Ok(()),
            false if default_on => Err("turned off for this app"),
            false => Err("off by default for this app"),
        }
    }

    pub fn tab_for(&self, exe: &str) -> bool {
        self.rule(exe).and_then(|r| r.tab).unwrap_or(true)
    }

    pub fn spaces_for(&self, exe: &str) -> bool {
        self.rule(exe).and_then(|r| r.type_spaces_separately).unwrap_or(self.type_spaces_separately)
    }

    /// Global instructions, then this app's.
    pub fn instructions_for(&self, exe: &str) -> String {
        let app = self.rule(exe).map_or("", |r| r.instructions.trim());
        [self.instructions.trim(), app].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join("\n")
    }
}

static CURRENT: Mutex<Option<Settings>> = Mutex::new(None);

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("NexType")
}

pub fn path() -> PathBuf {
    dir().join("settings.json")
}

/// The current settings (loaded from disk on first use).
pub fn get() -> Settings {
    let mut cur = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    cur.get_or_insert_with(load).clone()
}

/// Change the settings and save them.
pub fn update(f: impl FnOnce(&mut Settings)) -> Settings {
    let mut cur = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    let s = cur.get_or_insert_with(load);
    f(s);
    save(s);
    s.clone()
}

/// Replace all settings (from the settings window) and save them.
pub fn replace(new: Settings) {
    update(|s| *s = new);
}

fn load() -> Settings {
    if let Ok(bytes) = std::fs::read(path()) {
        match serde_json::from_slice(&bytes) {
            Ok(s) => return s,
            Err(e) => eprintln!("settings: {} is not valid ({e}); using defaults", path().display()),
        }
    }
    let mut s = Settings::default();
    // settings.txt from the first Windows builds held only this one switch.
    if let Ok(old) = std::fs::read_to_string(dir().join("settings.txt")) {
        if let Some(v) = old.lines().filter_map(|l| l.split_once('=')).find(|(k, _)| k.trim() == "type_spaces_separately") {
            s.type_spaces_separately = v.1.trim() == "true";
        }
    }
    s
}

fn save(s: &Settings) {
    let path = path();
    let Ok(body) = serde_json::to_vec_pretty(s) else { return };
    // Write a temp file and rename it, so a crash mid-write can't leave half a file.
    let tmp = path.with_extension("json.tmp");
    let ok = std::fs::create_dir_all(dir()).is_ok() && std::fs::write(&tmp, body).is_ok() && std::fs::rename(&tmp, &path).is_ok();
    if !ok {
        eprintln!("settings: could not save {}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_rules_override_the_defaults() {
        let mut s = Settings::default();
        assert!(s.completions_for("notepad.exe", 0).is_ok());
        assert_eq!(s.completions_for(OFF_BY_DEFAULT[0], 0), Err("off by default for this app"));
        s.rule_mut("Notepad.exe", "Notepad").completions = Some(false);
        assert_eq!(s.completions_for("notepad.exe", 0), Err("turned off for this app"));
        s.rule_mut(OFF_BY_DEFAULT[0], "x").completions = Some(true);
        assert!(s.completions_for(OFF_BY_DEFAULT[0], 0).is_ok());
        assert_eq!(s.apps.len(), 2);

        assert!(s.spaces_for("day one.exe"));
        s.rule_mut("day one.exe", "Day One").type_spaces_separately = Some(false);
        assert!(!s.spaces_for("day one.exe"));
    }

    #[test]
    fn pause_and_global_switch() {
        let mut s = Settings { paused_until: Some(100), ..Default::default() };
        assert_eq!(s.completions_for("notepad.exe", 99), Err("paused"));
        assert!(s.completions_for("notepad.exe", 100).is_ok());
        s.enabled = false;
        assert_eq!(s.completions_for("notepad.exe", 100), Err("completions off"));
    }

    #[test]
    fn instructions_join_global_and_app() {
        let mut s = Settings { instructions: "Be brief.".into(), ..Default::default() };
        s.rule_mut("outlook.exe", "Outlook").instructions = "Formal tone.".into();
        assert_eq!(s.instructions_for("outlook.exe"), "Be brief.\nFormal tone.");
        assert_eq!(s.instructions_for("notepad.exe"), "Be brief.");
    }

    #[test]
    fn old_and_partial_files_load() {
        let s: Settings = serde_json::from_str(r#"{"length":"long","apps":[{"exe":"a.exe"}]}"#).unwrap();
        assert_eq!(s.length, Length::Long);
        assert!(s.enabled && s.type_spaces_separately);
        assert_eq!(s.apps[0].completions, None);
    }
}
