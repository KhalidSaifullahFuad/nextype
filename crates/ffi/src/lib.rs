//! The Rust core as seen by the Swift shell (uniffi proc-macro bindings).
//! Every call except `accessibility_trusted` must come from the main thread.

use std::sync::atomic::Ordering;

use nextype::settings;

uniffi::setup_scaffolding!();

#[derive(uniffi::Enum)]
pub enum Length {
    Short,
    Medium,
    Long,
}

/// Whether Accessibility is granted; `prompt` shows the system dialog if not.
#[uniffi::export]
pub fn accessibility_trusted(prompt: bool) -> bool {
    nextype::accessibility_trusted(prompt)
}

/// Start completing. False if not on the main thread or Accessibility is missing.
#[uniffi::export]
pub fn start(model_path: String) -> bool {
    nextype::start(model_path)
}

/// Undo changes made to other apps (Chrome's accessibility mode); call before quitting.
#[uniffi::export]
pub fn stop() {
    nextype::stop();
}

#[uniffi::export]
pub fn set_enabled(on: bool) {
    nextype::set_enabled(on);
}

#[uniffi::export]
pub fn set_length(length: Length) {
    let tokens = match length {
        Length::Short => 8,
        // ~0.85 English words per token (Gemma 4 E2B); generation also stops at a sentence end.
        Length::Medium => 12,
        Length::Long => 24,
    };
    settings::MAX_TOKENS.store(tokens, Ordering::Relaxed);
}

/// Completions start off in this app until the user turns them on.
#[uniffi::export]
pub fn is_off_by_default(bundle_id: String) -> bool {
    nextype::off_by_default(&bundle_id)
}

/// Save what the user writes and use it as style examples (spec §7.4).
#[uniffi::export]
pub fn set_history(on: bool) {
    settings::HISTORY.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Number of saved texts. Main thread.
#[uniffi::export]
pub fn history_count() -> u64 {
    nextype::history::count()
}

/// Delete all saved writing. Main thread.
#[uniffi::export]
pub fn delete_history() {
    nextype::history::delete_all();
}

/// Use text read off the screen as context (spec §7.2).
#[uniffi::export]
pub fn set_screen_context(on: bool) {
    settings::SCREEN.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Where to read screen text: the focused field's app, window and frame (global top-left points).
#[derive(uniffi::Record)]
pub struct ScreenTarget {
    pub pid: i32,
    pub window_title: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// The field to read around, or None when no allowed field is focused.
#[uniffi::export]
pub fn screen_target() -> Option<ScreenTarget> {
    let t = nextype::screen::TARGET.lock().ok()?.clone()?;
    Some(ScreenTarget { pid: t.pid, window_title: t.window_title, x: t.field.x, y: t.field.y, w: t.field.w, h: t.field.h })
}

/// OCR result for a window, from the shell.
#[uniffi::export]
pub fn set_screen_text(pid: i32, window_title: String, text: String) {
    nextype::screen::set_text(pid, window_title, text);
}

/// Use recently copied text as context (spec §7.3).
#[uniffi::export]
pub fn set_clipboard_context(on: bool) {
    settings::CLIPBOARD.store(on, std::sync::atomic::Ordering::Relaxed);
}

#[uniffi::export]
pub fn set_instructions(text: String) {
    if let Ok(mut s) = settings::INSTRUCTIONS.lock() {
        *s = text;
    }
}

#[derive(uniffi::Record)]
pub struct AppOverride {
    pub bundle_id: String,
    pub completions_off: bool,
    pub tab_off: bool,
    pub instructions: String,
}

/// Replace all per-app overrides.
#[uniffi::export]
pub fn set_app_overrides(overrides: Vec<AppOverride>) {
    if let Ok(mut l) = settings::OVERRIDES.lock() {
        *l = overrides
            .into_iter()
            .map(|o| settings::AppOverride {
                bundle_id: o.bundle_id,
                completions_off: o.completions_off,
                tab_off: o.tab_off,
                instructions: o.instructions,
            })
            .collect();
    }
}

#[derive(uniffi::Record)]
pub struct TodayStats {
    pub completions: u64,
    pub words: u64,
}

/// Accepted completions and words today (from the stats file).
#[uniffi::export]
pub fn stats_today() -> TodayStats {
    let (completions, words) = nextype::stats::today();
    TodayStats { completions, words }
}

/// Host of the web page in the focused field, or "" outside web views.
#[uniffi::export]
pub fn current_domain() -> String {
    settings::CURRENT_DOMAIN.lock().map(|d| d.clone()).unwrap_or_default()
}
