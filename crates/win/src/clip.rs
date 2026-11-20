//! Insert text by pasting it: put it on the clipboard (kept out of clipboard history), send
//! Ctrl+V, then put back what the user had. Typing Unicode key events is not reliable on
//! Windows: the Windows 11 Notepad reads typed characters late and repeats a later letter
//! ("a few things" came out as "ttttttthings") at any useful typing speed.

use std::time::Duration;

use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::core::w;

const CF_UNICODETEXT: u32 = 13;
/// GDI-handle formats (CF_BITMAP, CF_METAFILEPICT, CF_PALETTE, CF_ENHMETAFILE): not HGLOBAL
/// memory, and Windows synthesizes them from the DIB / text formats we do keep.
const GDI_FORMATS: [u32; 4] = [2, 3, 9, 14];
/// Time the app gets to read the clipboard after Ctrl+V before we restore it.
const PASTE_SETTLE: Duration = Duration::from_millis(400);
/// Pause between setting the clipboard and Ctrl+V, for clipboard monitors to finish.
const PASTE_LEAD: Duration = Duration::from_millis(30);

/// RAII clipboard lock; retries briefly because other apps hold it for short moments.
struct Open;

impl Open {
    fn new() -> Option<Self> {
        for _ in 0..20 {
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Some(Open);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

/// Every memory-backed format on the clipboard, as raw bytes.
fn save(_lock: &Open) -> Vec<(u32, Vec<u8>)> {
    let mut out = Vec::new();
    let mut f = 0;
    loop {
        f = unsafe { EnumClipboardFormats(f) };
        if f == 0 {
            break;
        }
        if GDI_FORMATS.contains(&f) {
            continue;
        }
        let Ok(h) = (unsafe { GetClipboardData(f) }) else { continue };
        let g = HGLOBAL(h.0);
        unsafe {
            let n = GlobalSize(g);
            let p = GlobalLock(g);
            if !p.is_null() {
                out.push((f, std::slice::from_raw_parts(p as *const u8, n).to_vec()));
                let _ = GlobalUnlock(g);
            }
        }
    }
    out
}

fn put(_lock: &Open, format: u32, bytes: &[u8]) {
    unsafe {
        let Ok(g) = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) else { return };
        let p = GlobalLock(g);
        if p.is_null() {
            return;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len());
        let _ = GlobalUnlock(g);
        // On success the clipboard owns the memory.
        let _ = SetClipboardData(format, Some(HANDLE(g.0)));
    }
}

fn ctrl_v() {
    let key = |vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    };
    let up = KEYEVENTF_KEYUP;
    let inputs = [key(VK_CONTROL, KEYBD_EVENT_FLAGS(0)), key(VK_V, KEYBD_EVENT_FLAGS(0)), key(VK_V, up), key(VK_CONTROL, up)];
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
}

/// Paste `text` into the focused field and restore the clipboard. Blocks for about half a
/// second; call it off the UI thread. False if the clipboard couldn't be opened.
pub fn paste(text: &str) -> bool {
    let saved = {
        let Some(lock) = Open::new() else { return false };
        let saved = save(&lock);
        unsafe {
            let _ = EmptyClipboard();
        }
        let wide: Vec<u16> = text.replace('\n', "\r\n").encode_utf16().chain([0]).collect();
        let bytes: Vec<u8> = wide.iter().flat_map(|u| u.to_le_bytes()).collect();
        put(&lock, CF_UNICODETEXT, &bytes);
        // Keep this transient text out of Win+V history, cloud sync and clipboard monitors.
        unsafe {
            put(&lock, RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing")), &[0]);
            put(&lock, RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory")), &0u32.to_le_bytes());
            put(&lock, RegisterClipboardFormatW(w!("CanUploadToCloudClipboard")), &0u32.to_le_bytes());
        }
        saved
    };
    let ours = unsafe { GetClipboardSequenceNumber() };
    // Windows' clipboard history service opens the clipboard right after every change to look at
    // it; an app pasting in that moment can't open it and silently pastes nothing. Wait until the
    // clipboard is free again (open + close at once), plus a moment, before sending Ctrl+V.
    std::thread::sleep(PASTE_LEAD);
    drop(Open::new());
    ctrl_v();
    std::thread::sleep(PASTE_SETTLE);
    // Restore only if nobody copied something else meanwhile.
    if unsafe { GetClipboardSequenceNumber() } == ours {
        if let Some(lock) = Open::new() {
            unsafe {
                let _ = EmptyClipboard();
            }
            for (f, bytes) in &saved {
                put(&lock, *f, bytes);
            }
        }
    }
    true
}
