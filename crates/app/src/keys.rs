//! Keyboard plumbing: an active session event tap that swallows the accept keys only while a
//! ghost is visible (spec §12.1, §20 alternative to Carbon hot keys), and Unicode insertion (§12.2).
//! The tap callback runs on the main run loop and must stay trivial: no AX calls in it.

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardTypeString, NSPasteboardWriting};
use objc2_foundation::{NSArray, NSData, NSString};
use objc2_core_foundation::{CFMachPort, CFRetained, CFRunLoop, kCFRunLoopCommonModes};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventTapProxy, CGEventType,
};

/// Accept-the-whole-ghost key: the key above Tab on ANSI layouts (spec §6.3; ISO layouts differ).
pub const KEY_ABOVE_TAB: i64 = 50;
pub const TAB: i64 = 48;
pub const ESCAPE: i64 = 53;
/// Insert at most this many UTF-16 units per key event (spec §12.2).
const CHUNK_UTF16: usize = 20;

/// Set by the main thread while a ghost is on screen; any other key-down clears it at once so a
/// fast Tab after a mismatching character never accepts a stale ghost.
pub static GHOST_VISIBLE: AtomicBool = AtomicBool::new(false);
/// False in apps where the user turned the Tab shortcut off (Tab then reaches the app).
pub static TAB_ENABLED: AtomicBool = AtomicBool::new(true);
/// ANSI V, for ⌘V. shortcut: layouts that move V (Dvorak) need a `UCKeyTranslate` lookup (§6.3).
const KEY_V: u16 = 9;
/// How long the pasted text stays on the clipboard before the user's contents come back.
const CLIPBOARD_RESTORE: std::time::Duration = std::time::Duration::from_millis(500);

/// Key code whose key-up we swallow because we swallowed its key-down (-1 = none).
static SWALLOWED_UP: AtomicI64 = AtomicI64::new(-1);

/// The user's clipboard while a paste is in flight: the change count after we wrote, and each
/// item's (type, data) pairs.
type SavedClipboard = (isize, Vec<Vec<(Retained<NSString>, Retained<NSData>)>>);

thread_local! {
    static SAVED_CLIPBOARD: RefCell<Option<SavedClipboard>> = const { RefCell::new(None) };
    static TAP: RefCell<Option<CFRetained<CFMachPort>>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug)]
pub enum Key {
    AcceptAll,
    AcceptWord,
    Escape,
    /// Any other key while the ghost showed: re-check the field so the ghost (and Tab) come back.
    Other,
}

/// Install the tap on the main run loop; `on_key` runs later on the main queue. Needs Accessibility.
pub fn install_tap(on_key: fn(Key)) -> bool {
    let mask = (1u64 << CGEventType::KeyDown.0) | (1u64 << CGEventType::KeyUp.0);
    let Some(port) = (unsafe {
        CGEvent::tap_create(
            CGEventTapLocation::SessionEventTap,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            mask,
            Some(tap_cb),
            on_key as *mut c_void,
        )
    }) else {
        return false;
    };
    let Some(source) = CFMachPort::new_run_loop_source(None, Some(&port), 0) else { return false };
    let Some(rl) = CFRunLoop::main() else { return false };
    rl.add_source(Some(&source), unsafe { kCFRunLoopCommonModes });
    CGEvent::tap_enable(&port, true);
    TAP.with(|t| *t.borrow_mut() = Some(port));
    true
}

unsafe extern "C-unwind" fn tap_cb(
    _proxy: CGEventTapProxy,
    ty: CGEventType,
    event: NonNull<CGEvent>,
    info: *mut c_void,
) -> *mut CGEvent {
    let pass = event.as_ptr();
    if ty == CGEventType::TapDisabledByTimeout || ty == CGEventType::TapDisabledByUserInput {
        TAP.with(|t| t.borrow().as_ref().map(|p| CGEvent::tap_enable(p, true)));
        return pass;
    }
    let e = unsafe { event.as_ref() };
    let code = CGEvent::integer_value_field(Some(e), CGEventField::KeyboardEventKeycode);
    if ty == CGEventType::KeyUp {
        return if SWALLOWED_UP.compare_exchange(code, -1, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
            ptr::null_mut()
        } else {
            pass
        };
    }
    if !GHOST_VISIBLE.swap(false, Ordering::Relaxed) {
        return pass;
    }
    // SAFETY: `info` is the `fn(Key)` given to `install_tap`.
    let on_key: fn(Key) = unsafe { std::mem::transmute(info) };
    let mods = CGEvent::flags(Some(e))
        & (CGEventFlags::MaskCommand | CGEventFlags::MaskShift | CGEventFlags::MaskControl | CGEventFlags::MaskAlternate);
    let key = match code {
        KEY_ABOVE_TAB if mods.is_empty() => Key::AcceptAll,
        TAB if mods.is_empty() && TAB_ENABLED.load(Ordering::Relaxed) => Key::AcceptWord,
        // Escape only hides the ghost; the app still gets it (spec §5.3 option 1).
        ESCAPE => Key::Escape,
        _ => Key::Other,
    };
    DispatchQueue::main().exec_async(move || on_key(key));
    if matches!(key, Key::Escape | Key::Other) {
        return pass;
    }
    SWALLOWED_UP.store(code, Ordering::Relaxed);
    ptr::null_mut()
}

/// Type `text` into app `pid` as Unicode key events, at most 20 UTF-16 units per event,
/// never splitting a surrogate pair.
pub fn insert(pid: i32, text: &str) {
    let mut chunk: Vec<u16> = Vec::with_capacity(CHUNK_UTF16);
    let mut buf = [0u16; 2];
    for c in text.chars() {
        if chunk.len() + c.len_utf16() > CHUNK_UTF16 {
            post_chunk(pid, &chunk);
            chunk.clear();
        }
        chunk.extend_from_slice(c.encode_utf16(&mut buf));
    }
    post_chunk(pid, &chunk);
}

fn post_chunk(pid: i32, chunk: &[u16]) {
    if chunk.is_empty() {
        return;
    }
    for down in [true, false] {
        let Some(e) = CGEvent::new_keyboard_event(None, 0, down) else { return };
        // Don't let held modifiers turn the text into shortcuts.
        CGEvent::set_flags(Some(&e), CGEventFlags::empty());
        unsafe { CGEvent::keyboard_set_unicode_string(Some(&e), chunk.len() as _, chunk.as_ptr()) };
        CGEvent::post_to_pid(pid, Some(&e));
    }
}

/// Paste `text` into app `pid` with ⌘V (spec §12.2 fallback), then put the user's clipboard back.
pub fn paste(pid: i32, text: &str) {
    let pb = NSPasteboard::generalPasteboard();
    let saved = pb
        .pasteboardItems()
        .map(|items| {
            items
                .iter()
                .map(|item| item.types().iter().filter_map(|t| item.dataForType(&t).map(|d| (t, d))).collect())
                .collect()
        })
        .unwrap_or_default();
    pb.clearContents();
    pb.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString });
    // Clipboard managers skip transient content (nspasteboard.org convention).
    pb.setData_forType(Some(&NSData::new()), &NSString::from_str("org.nspasteboard.TransientType"));
    SAVED_CLIPBOARD.with(|s| *s.borrow_mut() = Some((pb.changeCount(), saved)));
    for down in [true, false] {
        let Some(e) = CGEvent::new_keyboard_event(None, KEY_V, down) else { return };
        CGEvent::set_flags(Some(&e), CGEventFlags::MaskCommand);
        CGEvent::post_to_pid(pid, Some(&e));
    }
    let when = CLIPBOARD_RESTORE.try_into().expect("dispatch time");
    let _ = DispatchQueue::main().after(when, restore_clipboard);
}

fn restore_clipboard() {
    let Some((count, saved)) = SAVED_CLIPBOARD.with(|s| s.borrow_mut().take()) else { return };
    let pb = NSPasteboard::generalPasteboard();
    if pb.changeCount() != count {
        return; // something else wrote to the clipboard meanwhile: keep that
    }
    pb.clearContents();
    let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = saved
        .into_iter()
        .map(|pairs| {
            let item = NSPasteboardItem::new();
            for (t, d) in pairs {
                item.setData_forType(&d, &t);
            }
            ProtocolObject::from_retained(item)
        })
        .collect();
    if !items.is_empty() {
        pb.writeObjects(&NSArray::from_retained_slice(&items));
    }
    crate::clipboard::RESTORED.store(pb.changeCount(), Ordering::Relaxed);
}
