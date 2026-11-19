//! macOS Accessibility access for NexType (spec §4): read the focused text field and
//! observe the frontmost app for focus / value / selection changes.
//! Everything here must run on the main thread (the observer source lives on the main run loop).

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::{self, NonNull};

use nextype_core::{FieldSnapshot, NEEDS_ENHANCED_UI, Rect, WidthSample, host_of, utf16_to_byte};
use objc2_app_kit::{NSRunningApplication, NSWorkspace};
use objc2_application_services::{
    AXError, AXIsProcessTrustedWithOptions, AXObserver, AXUIElement, AXValue, AXValueType,
    kAXTrustedCheckOptionPrompt,
};
use objc2_core_foundation::{
    CFAttributedString, CFBoolean, CFDictionary, CFNumber, CFRange, CFRetained, CFRunLoop,
    CFRunLoopSource, CFString, CFType, CFURL, CGPoint, CGRect, CGSize, kCFBooleanFalse, kCFBooleanTrue, kCFRunLoopCommonModes,
};

/// Seconds an AX call may block before giving up (Cotypist sets a timeout too; value unknown).
const MESSAGING_TIMEOUT: f32 = 0.25;

/// Notifications that can change the completion state (spec §4.1).
const NOTIFICATIONS: &[&str] = &[
    "AXFocusedUIElementChanged",
    "AXFocusedWindowChanged",
    "AXValueChanged",
    "AXSelectedTextChanged",
];

/// Whether we have the Accessibility permission; `prompt` shows the system dialog if not.
pub fn is_trusted(prompt: bool) -> bool {
    let value: &CFBoolean = if prompt {
        unsafe { kCFBooleanTrue }.expect("kCFBooleanTrue")
    } else {
        return unsafe { objc2_application_services::AXIsProcessTrusted() };
    };
    let key: &CFString = unsafe { kAXTrustedCheckOptionPrompt };
    let opts = CFDictionary::<CFString, CFBoolean>::from_slices(&[key], &[value]);
    unsafe { AXIsProcessTrustedWithOptions(Some(opts.as_opaque())) }
}

fn system_wide() -> CFRetained<AXUIElement> {
    let sys = unsafe { AXUIElement::new_system_wide() };
    unsafe { sys.set_messaging_timeout(MESSAGING_TIMEOUT) };
    sys
}

fn attr(el: &AXUIElement, name: &str) -> Option<CFRetained<CFType>> {
    let mut out: *const CFType = ptr::null();
    let err = unsafe { el.copy_attribute_value(&CFString::from_str(name), NonNull::from(&mut out)) };
    if err != AXError::Success {
        return None;
    }
    // SAFETY: a successful Copy call hands us a +1 reference.
    NonNull::new(out.cast_mut()).map(|p| unsafe { CFRetained::from_raw(p) })
}

fn string_attr(el: &AXUIElement, name: &str) -> Option<String> {
    attr(el, name)?.downcast::<CFString>().ok().map(|s| s.to_string())
}

fn element_attr(el: &AXUIElement, name: &str) -> Option<CFRetained<AXUIElement>> {
    attr(el, name)?.downcast::<AXUIElement>().ok()
}

/// `T` must be the plain C struct matching `ty` (CGPoint, CGSize, CGRect, CFRange).
fn ax_value<T: Copy>(v: &CFType, ty: AXValueType) -> Option<T> {
    let v = v.downcast_ref::<AXValue>()?;
    // SAFETY: all-zero bytes are a valid value for these C structs.
    let mut out: T = unsafe { std::mem::zeroed() };
    unsafe { v.value(ty, NonNull::from(&mut out).cast()) }.then_some(out)
}

fn to_rect(r: CGRect) -> Rect {
    Rect { x: r.origin.x, y: r.origin.y, w: r.size.width, h: r.size.height }
}

/// A parameterized attribute that takes a character range (`AXBoundsForRange`, …).
fn range_attr(el: &AXUIElement, name: &str, location: usize, length: usize) -> Option<CFRetained<CFType>> {
    let mut range = CFRange { location: location as isize, length: length as isize };
    let param = unsafe { AXValue::new(AXValueType::CFRange, NonNull::from(&mut range).cast()) }?;
    param_attr(el, name, &param)
}

fn param_attr(el: &AXUIElement, name: &str, param: &CFType) -> Option<CFRetained<CFType>> {
    let mut out: *const CFType = ptr::null();
    let err = unsafe {
        el.copy_parameterized_attribute_value(&CFString::from_str(name), param, NonNull::from(&mut out))
    };
    if err != AXError::Success {
        return None;
    }
    NonNull::new(out.cast_mut()).map(|p| unsafe { CFRetained::from_raw(p) })
}

fn bounds_for_range(el: &AXUIElement, location: usize, length: usize) -> Option<Rect> {
    let v = range_attr(el, "AXBoundsForRange", location, length)?;
    let r = to_rect(ax_value::<CGRect>(&v, AXValueType::CGRect)?);
    (r.h > 0.0).then_some(r)
}

/// Font of the character before the caret (spec §11.2): the `AXFont` attribute of
/// `AXAttributedStringForRange`, a dictionary with `AXFontName` and `AXFontSize`.
fn caret_font(el: &AXUIElement, caret: usize) -> Option<(String, f64)> {
    let s = range_attr(el, "AXAttributedStringForRange", caret.checked_sub(1)?, 1)?;
    let s = s.downcast::<CFAttributedString>().ok()?;
    let font = unsafe { s.attribute(0, Some(&CFString::from_str("AXFont")), ptr::null_mut()) }?;
    let font = font.downcast::<CFDictionary>().ok()?;
    // SAFETY: AX font dictionaries have CFString keys.
    let font: &CFDictionary<CFString, CFType> = unsafe { font.cast_unchecked() };
    let name = font.get(&CFString::from_str("AXFontName"))?.downcast::<CFString>().ok()?;
    let size = font.get(&CFString::from_str("AXFontSize"))?.downcast::<CFNumber>().ok()?.as_f64()?;
    Some((name.to_string(), size))
}

/// Caret rectangle: the right edge of the character before the caret. Zero-length ranges are
/// unreliable (TextEdit on macOS 27 reports them one line too high, many apps don't answer),
/// so they are used only at a line start, where the previous character is on the line above.
fn caret_rect(el: &AXUIElement, caret: usize, line_start: bool) -> Option<Rect> {
    let prev = caret.checked_sub(1).filter(|_| !line_start).and_then(|p| bounds_for_range(el, p, 1));
    prev.map(|r| Rect { x: r.x + r.w, w: 0.0, ..r })
        .or_else(|| bounds_for_range(el, caret, 0))
        .or_else(|| marker_caret_rect(el))
}

/// Web views (WebKit, Chromium) often don't answer `AXBoundsForRange` in editable content but
/// do answer the text-marker API (spec §4, M5): the bounds of the selected marker range.
fn marker_caret_rect(el: &AXUIElement) -> Option<Rect> {
    let range = attr(el, "AXSelectedTextMarkerRange")?;
    let v = param_attr(el, "AXBoundsForTextMarkerRange", &range)?;
    let r = to_rect(ax_value::<CGRect>(&v, AXValueType::CGRect)?);
    // A collapsed caret is a thin line; some web views answer with a whole block instead.
    (r.h > 0.0 && r.h < 100.0 && r.w < 50.0).then_some(Rect { x: r.x + r.w, w: 0.0, ..r })
}

/// Widths of up to the last 24 characters of the caret's line, and of their first half.
fn width_sample(el: &AXUIElement, line: &str, caret: usize) -> Option<WidthSample> {
    let chars: Vec<char> = line.chars().collect();
    let n = chars.len().min(24);
    if n < 8 {
        return None;
    }
    let text: String = chars[chars.len() - n..].iter().collect();
    let head_chars = n / 2;
    let head16: usize = chars[chars.len() - n..][..head_chars].iter().map(|c| c.len_utf16()).sum();
    let start = caret.checked_sub(text.encode_utf16().count())?;
    // Widths from single-character bounds: Chromium (VS Code) doesn't answer multi-character ranges.
    let first = bounds_for_range(el, start, 1)?;
    let head_end = right_edge(el, start + head16, chars[chars.len() - n + head_chars - 1])?;
    let full_end = right_edge(el, caret, chars[chars.len() - 1])?;
    let (width, head_width) = (full_end.x + full_end.w - first.x, head_end.x + head_end.w - first.x);
    // One line only: a wrapped sample has its last character on another line.
    ((full_end.y - first.y).abs() < first.h * 0.5 && head_width > 0.0 && width > head_width)
        .then_some(WidthSample { text, width, head_chars, head_width })
}

/// Bounds of the character `c` that ends at UTF-16 offset `end`.
fn right_edge(el: &AXUIElement, end: usize, c: char) -> Option<Rect> {
    bounds_for_range(el, end.checked_sub(c.len_utf16())?, c.len_utf16())
}

fn pid_of(el: &AXUIElement) -> Option<i32> {
    let mut pid = 0;
    (unsafe { el.pid(NonNull::from(&mut pid)) } == AXError::Success).then_some(pid)
}

fn bundle_id(pid: i32) -> String {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .and_then(|app| app.bundleIdentifier())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

fn app_name(pid: i32) -> String {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .and_then(|app| app.localizedName())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Title of the window that holds `el`.
fn window_title(el: &AXUIElement) -> String {
    element_attr(el, "AXWindow").and_then(|w| string_attr(&w, "AXTitle")).unwrap_or_default()
}

thread_local! {
    /// (pid, window title) → domain, so the parent walk runs once per page, not per keystroke.
    static DOMAIN_CACHE: RefCell<(i32, String, String)> = const { RefCell::new((0, String::new(), String::new())) };
}

/// Host of the top-level page around `el` (spec §4.3): the outermost `AXWebArea` ancestor's `AXURL`.
/// shortcut: cached by window title; a page that changes URL without changing its title keeps
/// the old domain until the title changes.
fn web_domain(el: &AXUIElement, pid: i32, title: &str) -> String {
    if let Some(d) = DOMAIN_CACHE.with(|c| {
        let c = c.borrow();
        (c.0 == pid && c.1 == title).then(|| c.2.clone())
    }) {
        return d;
    }
    let mut domain = String::new();
    // SAFETY: `el` is a live element; retaining it gives us an owned handle to walk from.
    let mut cur: CFRetained<AXUIElement> = unsafe { CFRetained::retain(NonNull::from(el)) };
    for _ in 0..60 {
        if string_attr(&cur, "AXRole").as_deref() == Some("AXWebArea") {
            if let Some(url) = attr(&cur, "AXURL").and_then(|u| u.downcast::<CFURL>().ok()) {
                domain = host_of(&url.string().to_string());
            }
        }
        match element_attr(&cur, "AXParent") {
            Some(p) => cur = p,
            None => break,
        }
    }
    DOMAIN_CACHE.with(|c| *c.borrow_mut() = (pid, title.to_owned(), domain.clone()));
    domain
}

/// pid of the app that owns keyboard focus.
pub fn focused_app_pid() -> Option<i32> {
    // The system-wide query can fail (-25204) for a while after launch; ask NSWorkspace then.
    element_attr(&system_wide(), "AXFocusedApplication")
        .and_then(|app| pid_of(&app))
        .or_else(|| Some(NSWorkspace::sharedWorkspace().frontmostApplication()?.processIdentifier()))
}

/// Read the focused element. `None` if nothing is focused or it has no string value.
pub fn snapshot() -> Option<FieldSnapshot> {
    // Some apps (Ghostex) fail the system-wide query (-25204) but answer on their own element.
    let el = element_attr(&system_wide(), "AXFocusedUIElement").or_else(|| {
        let pid = NSWorkspace::sharedWorkspace().frontmostApplication()?.processIdentifier();
        let app = unsafe { AXUIElement::new_application(pid) };
        element_attr(&app, "AXFocusedUIElement")
    })?;
    // Web editors turn a typed trailing space into U+00A0; it would no longer match the ghost.
    // Same UTF-16 length, so caret offsets stay valid.
    let text = string_attr(&el, "AXValue")?.replace('\u{a0}', " ");
    let sel = attr(&el, "AXSelectedTextRange")
        .and_then(|v| ax_value::<CFRange>(&v, AXValueType::CFRange))
        .unwrap_or(CFRange { location: 0, length: 0 });
    let caret = sel.location.max(0) as usize;
    let origin = attr(&el, "AXPosition").and_then(|v| ax_value::<CGPoint>(&v, AXValueType::CGPoint));
    let size = attr(&el, "AXSize").and_then(|v| ax_value::<CGSize>(&v, AXValueType::CGSize));
    let frame = match (origin, size) {
        (Some(o), Some(s)) => Rect { x: o.x, y: o.y, w: s.width, h: s.height },
        _ => Rect::default(),
    };
    let pid = pid_of(&el).unwrap_or(0);
    let title = window_title(&el);
    let before = &text[..utf16_to_byte(&text, caret)];
    let para = before.rfind(['\n', '\r']).map_or(0, |i| i + 1);
    let line_start = para == before.len();
    let caret_rect = caret_rect(&el, caret, line_start);
    let font = caret_font(&el, caret);
    let width_sample = if font.is_none() { width_sample(&el, &before[para..], caret) } else { None };
    let text_left = if line_start {
        caret_rect.map(|r| r.x)
    } else {
        bounds_for_range(&el, before[..para].encode_utf16().count(), 1).map(|r| r.x)
    };
    Some(FieldSnapshot {
        pid,
        bundle_id: bundle_id(pid),
        app_name: app_name(pid),
        domain: web_domain(&el, pid, &title),
        window_title: title,
        role: string_attr(&el, "AXRole").unwrap_or_default(),
        subrole: string_attr(&el, "AXSubrole").unwrap_or_default(),
        caret_rect,
        font,
        width_sample,
        text_left,
        text,
        caret_utf16: caret,
        selection_len: sel.length.max(0) as usize,
        frame,
    })
}

/// AX notifications for one app, delivered on the main run loop to `on_event(notification)`.
pub struct AppObserver {
    pub pid: i32,
    observer: CFRetained<AXObserver>,
    source: CFRetained<CFRunLoopSource>,
}

unsafe extern "C-unwind" fn trampoline(
    _observer: NonNull<AXObserver>,
    _element: NonNull<AXUIElement>,
    notification: NonNull<CFString>,
    refcon: *mut c_void,
) {
    // SAFETY: refcon is the `fn(&str)` passed to `AppObserver::new`.
    let on_event: fn(&str) = unsafe { std::mem::transmute(refcon) };
    on_event(&unsafe { notification.as_ref() }.to_string());
}

impl AppObserver {
    pub fn new(pid: i32, on_event: fn(&str)) -> Option<Self> {
        let mut raw: *mut AXObserver = ptr::null_mut();
        if unsafe { AXObserver::create(pid, Some(trampoline), NonNull::from(&mut raw)) } != AXError::Success {
            return None;
        }
        let observer = unsafe { CFRetained::from_raw(NonNull::new(raw)?) };
        let app = unsafe { AXUIElement::new_application(pid) };
        // Electron and Chromium build their AX tree only when asked (spec §4.2); others ignore it.
        // Errors are ignored: Chrome reports one for an attribute it does apply.
        let on = unsafe { kCFBooleanTrue }.expect("kCFBooleanTrue");
        let attr = if NEEDS_ENHANCED_UI.contains(&bundle_id(pid).as_str()) { "AXEnhancedUserInterface" } else { "AXManualAccessibility" };
        let _ = unsafe { app.set_attribute_value(&CFString::from_str(attr), on) };
        if attr == "AXEnhancedUserInterface" {
            let mut pids = ENHANCED_PIDS.lock().unwrap();
            if !pids.contains(&pid) {
                pids.push(pid);
            }
        }
        for name in NOTIFICATIONS {
            // Some apps reject individual notifications; the others still work.
            let _ = unsafe { observer.add_notification(&app, &CFString::from_str(name), on_event as *mut c_void) };
        }
        let source = unsafe { observer.run_loop_source() };
        CFRunLoop::main()?.add_source(Some(&source), unsafe { kCFRunLoopCommonModes });
        Some(Self { pid, observer, source })
    }
}

/// Browsers we switched `AXEnhancedUserInterface` on in; it makes Chromium slower, so it is undone on quit.
static ENHANCED_PIDS: std::sync::Mutex<Vec<i32>> = std::sync::Mutex::new(Vec::new());

/// Switch `AXEnhancedUserInterface` back off in every app we turned it on in.
pub fn reset_enhanced_ui() {
    let off = unsafe { kCFBooleanFalse }.expect("kCFBooleanFalse");
    for pid in ENHANCED_PIDS.lock().unwrap().drain(..) {
        let app = unsafe { AXUIElement::new_application(pid) };
        let err = unsafe { app.set_attribute_value(&CFString::from_str("AXEnhancedUserInterface"), off) };
        eprintln!("enhanced UI off in pid {pid}: {err:?}");
    }
}

impl Drop for AppObserver {
    fn drop(&mut self) {
        if let Some(rl) = CFRunLoop::main() {
            rl.remove_source(Some(&self.source), unsafe { kCFRunLoopCommonModes });
        }
        let _ = &self.observer;
    }
}
