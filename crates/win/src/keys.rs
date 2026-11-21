//! Keyboard plumbing: a low-level hook that swallows the accept keys only while a ghost shows,
//! and `SendInput` typing of accepted text.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::WM_KEY;

pub static GHOST_VISIBLE: AtomicBool = AtomicBool::new(false);
pub static TAB_ENABLED: AtomicBool = AtomicBool::new(true);
/// Thread whose message loop receives `WM_KEY`.
static UI_THREAD: AtomicU32 = AtomicU32::new(0);
/// Virtual key whose key-up we swallow because we swallowed its key-down (0 = none).
static SWALLOW_UP: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Key {
    AcceptAll,
    AcceptWord,
    Escape,
    Other,
}

impl Key {
    pub fn to_wparam(self) -> usize {
        self as usize
    }
    pub fn from_wparam(w: usize) -> Self {
        [Key::AcceptAll, Key::AcceptWord, Key::Escape, Key::Other].get(w).copied().unwrap_or(Key::Other)
    }
}

/// Test hook: treat injected keys (from a test script) like real ones. Our own `insert` only
/// injects Unicode events and Enter, never an accept key.
static ALLOW_INJECTED: AtomicBool = AtomicBool::new(false);

pub fn install_hook(ui_thread: u32) -> bool {
    UI_THREAD.store(ui_thread, Ordering::Relaxed);
    ALLOW_INJECTED.store(std::env::var_os("NEXTYPE_TEST_INJECTED").is_some(), Ordering::Relaxed);
    unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), None, 0).is_ok() }
}

fn post(key: Key) {
    unsafe {
        let _ = PostThreadMessageW(UI_THREAD.load(Ordering::Relaxed), WM_KEY, WPARAM(key.to_wparam()), LPARAM(0));
    }
}

fn modifier_down() -> bool {
    [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN].iter().any(|k| unsafe { GetAsyncKeyState(k.0 as i32) } < 0)
}

unsafe extern "system" fn hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let k = unsafe { &*(l.0 as *const KBDLLHOOKSTRUCT) };
        let msg = w.0 as u32;
        let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let injected = k.flags.0 & LLKHF_INJECTED.0 != 0 && !ALLOW_INJECTED.load(Ordering::Relaxed);
        if !injected {
            let vk = k.vkCode;
            if !down && SWALLOW_UP.load(Ordering::Relaxed) == vk {
                SWALLOW_UP.store(0, Ordering::Relaxed);
                return LRESULT(1);
            }
            if down {
                let accept = if !GHOST_VISIBLE.load(Ordering::Relaxed) || modifier_down() {
                    None
                } else if vk == VK_TAB.0 as u32 && TAB_ENABLED.load(Ordering::Relaxed) {
                    Some(Key::AcceptWord)
                } else if vk == VK_OEM_3.0 as u32 {
                    Some(Key::AcceptAll)
                } else {
                    None
                };
                if let Some(key) = accept {
                    SWALLOW_UP.store(vk, Ordering::Relaxed);
                    post(key);
                    return LRESULT(1);
                }
                // Escape only hides the ghost; the app still gets it.
                post(if vk == VK_ESCAPE.0 as u32 { Key::Escape } else { Key::Other });
            }
        }
    }
    unsafe { CallNextHookEx(None, code, w, l) }
}

/// Gap between typed characters when we fall back to typing.
const CHAR_GAP: std::time::Duration = std::time::Duration::from_millis(4);
/// One insertion at a time, so two quick accepts don't interleave.
static TYPING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Insert `text` into the focused field on a background thread: paste it (see `clip`), or
/// type it if the clipboard is unavailable. Off the UI thread so the keyboard hook stays fast.
pub fn insert(text: &str) {
    let text = text.to_owned();
    std::thread::spawn(move || {
        let _one = TYPING.lock();
        if !SPLIT_SPACES.load(Ordering::Relaxed) {
            if !crate::clip::paste(&text) {
                eprintln!("clipboard busy: typing instead");
                type_text(&text);
            }
            return;
        }
        // Rich editors (Day One, many web editors) trim spaces at the edges of pasted plain
        // text, so " like" lands as "like". Type edge spaces as real Space keys, paste the rest.
        let core = text.trim_matches(' ');
        let lead = text.len() - text.trim_start_matches(' ').len();
        let trail = text.len() - text.trim_end_matches(' ').len();
        let spaces = |n: usize| {
            for _ in 0..n {
                send_char(' ');
                std::thread::sleep(CHAR_GAP);
            }
        };
        if core.is_empty() {
            spaces(text.len());
            return;
        }
        spaces(lead);
        if crate::clip::paste(core) {
            spaces(trail);
            return;
        }
        eprintln!("clipboard busy: typing instead");
        type_text(&text[lead..]);
    });
}

/// Type spaces at the edges of an accepted word as Space keys instead of pasting them (tray
/// menu). On: works in editors that trim pasted spaces (Day One), but the space shows a moment
/// before the word. Off: one paste, word and space appear together.
pub static SPLIT_SPACES: AtomicBool = AtomicBool::new(true);

fn type_text(text: &str) {
    for c in text.chars() {
        send_char(c);
        std::thread::sleep(CHAR_GAP);
    }
}

/// One character as Unicode key events; a newline becomes Enter.
fn send_char(c: char) {
    let mut inputs = Vec::new();
    let key = |vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    };
    if c == '\n' || c == ' ' {
        // Real keys: editors that handle key-down themselves see an ordinary Space / Enter.
        let vk = if c == '\n' { VK_RETURN } else { VK_SPACE };
        inputs.push(key(vk, 0, KEYBD_EVENT_FLAGS(0)));
        inputs.push(key(vk, 0, KEYEVENTF_KEYUP));
    } else {
        // A character outside the BMP is a surrogate pair: both halves down, then both up.
        let mut buf = [0u16; 2];
        let units = c.encode_utf16(&mut buf);
        for &u in units.iter() {
            inputs.push(key(VIRTUAL_KEY(0), u, KEYEVENTF_UNICODE));
        }
        for &u in units.iter() {
            inputs.push(key(VIRTUAL_KEY(0), u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
        }
    }
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        eprintln!("SendInput: {sent} of {} events (blocked by a higher-integrity window?)", inputs.len());
    }
}
