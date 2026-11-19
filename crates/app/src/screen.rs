//! Screen text (spec §7.2). The shell captures the window and runs OCR (ScreenCaptureKit and
//! Vision are Swift-native); the core only says which window and field, and keeps the result.

use std::sync::Mutex;

use nextype_core::{FieldSnapshot, Rect};

#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub pid: i32,
    pub window_title: String,
    /// The focused field, global top-left points: only the window part above it is read.
    pub field: Rect,
}

/// The field to read around; None outside allowed fields, so excluded apps are never captured.
pub static TARGET: Mutex<Option<Target>> = Mutex::new(None);
/// Last OCR result and the window it came from.
static TEXT: Mutex<Option<(i32, String, String)>> = Mutex::new(None);

pub fn set_target(s: Option<&FieldSnapshot>) {
    let t = s.map(|s| Target { pid: s.pid, window_title: s.window_title.clone(), field: s.frame });
    if let Ok(mut l) = TARGET.lock() {
        *l = t;
    }
}

pub fn set_text(pid: i32, window_title: String, text: String) {
    eprintln!("screen text: {} lines, {} chars from pid {pid}", text.lines().count(), text.chars().count());
    if let Ok(mut l) = TEXT.lock() {
        *l = Some((pid, window_title, text));
    }
}

/// OCR text of the field's window, if the last capture was of that window.
pub fn text_for(s: &FieldSnapshot) -> String {
    TEXT.lock()
        .ok()
        .and_then(|l| l.as_ref().filter(|(pid, title, _)| *pid == s.pid && *title == s.window_title).map(|t| t.2.clone()))
        .unwrap_or_default()
}
