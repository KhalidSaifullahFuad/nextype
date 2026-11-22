//! Reads the focused text field through UI Automation (the Windows twin of `crates/ax`).
//! Coordinates are physical screen pixels; the process is per-monitor DPI aware.

use std::ffi::c_void;

use nextype_core::{FieldSnapshot, Rect, WidthSample};

use crate::font::Reported;
use windows::Win32::Foundation::{CloseHandle, HWND, MAX_PATH, POINT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, SAFEARRAY};
use windows::Win32::System::Ole::{SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetUBound, SafeArrayUnaccessData};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::{
    GUITHREADINFO, GetForegroundWindow, GetGUIThreadInfo, GetWindowTextW, GetWindowThreadProcessId,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::System::Variant::VariantToString;
use windows::core::{PWSTR, Result};

pub struct Uia {
    auto: IUIAutomation,
}

impl Uia {
    /// Needs COM initialised on this thread.
    pub fn new() -> Result<Self> {
        Ok(Self { auto: unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)? } })
    }

    /// Process id of the foreground window.
    pub fn foreground_pid(&self) -> Option<i32> {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_invalid() {
            return None;
        }
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        (pid != 0).then_some(pid as i32)
    }

    /// The focused field, or `None` if focus isn't in a text control we can read.
    pub fn snapshot(&self) -> Option<(FieldSnapshot, Reported)> {
        match unsafe { self.read() } {
            Ok(s) => s,
            Err(_) => None,
        }
    }

    unsafe fn read(&self) -> Result<Option<(FieldSnapshot, Reported)>> {
        unsafe {
            let el = self.auto.GetFocusedElement()?;
            let pid = el.CurrentProcessId()?;
            if pid as u32 == GetCurrentProcessId() {
                return Ok(None);
            }
            let ct = el.CurrentControlType()?;
            let mut role = if ct == UIA_EditControlTypeId {
                "AXTextField"
            } else if ct == UIA_DocumentControlTypeId {
                "AXTextArea"
            } else if ct == UIA_ComboBoxControlTypeId {
                "AXComboBox"
            } else {
                return Ok(None);
            };
            let subrole = if el.CurrentIsPassword()?.as_bool() {
                "AXSecureTextField"
            } else if el.CurrentLocalizedControlType().map(|t| t.to_string().to_lowercase().contains("search")).unwrap_or(false)
                // Browser address bars ("Address and search bar" in Chrome/Edge) take queries too.
                || el.CurrentName().map(|n| { let n = n.to_string().to_lowercase(); n.contains("address") || n.contains("search") }).unwrap_or(false)
            {
                "AXSearchField"
            } else {
                ""
            };
            let r = el.CurrentBoundingRectangle()?;
            let frame = Rect { x: r.left as f64, y: r.top as f64, w: (r.right - r.left) as f64, h: (r.bottom - r.top) as f64 };

            let tp: IUIAutomationTextPattern = el.GetCurrentPatternAs(UIA_TextPatternId)?;
            let sel = tp.GetSelection()?;
            if sel.Length()? < 1 {
                return Ok(None);
            }
            let caret = sel.GetElement(0)?;
            let doc = tp.DocumentRange()?;
            let before_r = doc.Clone()?;
            before_r.MoveEndpointByRange(TextPatternRangeEndpoint_End, &caret, TextPatternRangeEndpoint_Start)?;
            let after_r = doc.Clone()?;
            after_r.MoveEndpointByRange(TextPatternRangeEndpoint_Start, &caret, TextPatternRangeEndpoint_End)?;
            let mut before = normalize(&before_r.GetText(-1)?.to_string());
            let mut selected = normalize(&caret.GetText(-1)?.to_string());
            let mut after = normalize(&after_r.GetText(-1)?.to_string());

            let caret_rect = caret_rect(&caret, &before).or_else(gui_caret);
            // An empty field shows its placeholder ("Title", "Describe the view…") and Chromium
            // reports it as the field's text. Read as typed text, the model continues a word the
            // user never wrote, drawn over the placeholder. Treat such a field as empty.
            if is_placeholder(&el, &doc, &before, &format!("{before}{selected}{after}"), caret_rect) {
                (before, selected, after) = (String::new(), String::new(), String::new());
            }
            let reported = field_font(&caret, &before);
            let text_left = line_left(&caret);
            // A multi-line edit is a text area for wrapping purposes.
            if role == "AXTextField" && caret_rect.is_some_and(|c| frame.h > 2.5 * c.h) {
                role = "AXTextArea";
            }
            let text = format!("{before}{selected}{after}");
            let (app_name, bundle_id) = exe_of(pid as u32);
            let font = reported.px.filter(|_| !reported.families.is_empty()).map(|px| (reported.families.clone(), px));
            Ok(Some((FieldSnapshot {
                pid,
                bundle_id,
                role: role.into(),
                subrole: subrole.into(),
                app_name,
                window_title: foreground_title(),
                domain: String::new(),
                caret_utf16: before.encode_utf16().count(),
                selection_len: selected.encode_utf16().count(),
                text,
                frame,
                caret_rect,
                font,
                width_sample: reported.sample.clone(),
                text_left,
            }, reported)))
        }
    }
}

/// UIA text uses `\r\n` (or bare `\r` in rich edits); the core and the ghost logic expect `\n`.
fn normalize(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

/// Caret rectangle from the text pattern. A degenerate range usually has no rectangles, so we
/// measure the character before the caret (its right edge) or, at a line start, the one after.
unsafe fn caret_rect(caret: &IUIAutomationTextRange, before: &str) -> Option<Rect> {
    unsafe {
        if let Some(r) = rects(caret).first().filter(|r| r.h > 0.0) {
            return Some(Rect { w: 1.0, ..*r });
        }
        let at_line_start = before.is_empty() || before.ends_with('\n');
        let probe = caret.Clone().ok()?;
        if !at_line_start {
            probe.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -1).ok()?;
            let r = *rects(&probe).last()?;
            return Some(Rect { x: r.x + r.w, y: r.y, w: 1.0, h: r.h });
        }
        probe.MoveEndpointByUnit(TextPatternRangeEndpoint_End, TextUnit_Character, 1).ok()?;
        let r = *rects(&probe).first()?;
        Some(Rect { x: r.x, y: r.y, w: 1.0, h: r.h })
    }
}

/// Whether the field's text is only its placeholder: it equals the placeholder the element
/// reports (UIA HelpText, or `aria-placeholder`), or, for placeholders drawn with CSS (ProseMirror,
/// Tiptap), there is text before the caret on the first line and yet the caret sits at that
/// line's very start. A soft-wrapped caret at a later line's start is below the first line.
unsafe fn is_placeholder(
    el: &IUIAutomationElement,
    doc: &IUIAutomationTextRange,
    before: &str,
    text: &str,
    caret: Option<Rect>,
) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }
    unsafe {
        let help = el.CurrentHelpText().map(|h| h.to_string()).unwrap_or_default();
        let aria = el.CurrentAriaProperties().map(|a| a.to_string()).unwrap_or_default();
        let aria_hint = aria.split(';').find_map(|p| p.strip_prefix("placeholder=")).unwrap_or("");
        if [help.trim(), aria_hint.trim()].contains(&text) {
            return true;
        }
        let (Some(caret), false) = (caret, before.is_empty() || before.contains('\n')) else { return false };
        let Ok(first) = doc.Clone() else { return false };
        if first.MoveEndpointByRange(TextPatternRangeEndpoint_End, doc, TextPatternRangeEndpoint_Start).is_err()
            || first.MoveEndpointByUnit(TextPatternRangeEndpoint_End, TextUnit_Character, 1).is_err()
        {
            return false;
        }
        let Some(r) = rects(&first).first().copied() else { return false };
        let same_line = (caret.y - r.y).abs() < r.h.max(caret.h) / 2.0;
        same_line && caret.x <= r.x + 1.0
    }
}

/// A one-character range next to the caret: the one before it, or after it at a line start.
unsafe fn char_near(caret: &IUIAutomationTextRange, before: &str) -> Option<IUIAutomationTextRange> {
    unsafe {
        let probe = caret.Clone().ok()?;
        if before.is_empty() || before.ends_with('\n') {
            probe.MoveEndpointByUnit(TextPatternRangeEndpoint_End, TextUnit_Character, 1).ok()?;
        } else {
            probe.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -1).ok()?;
        }
        Some(probe)
    }
}

/// Font of the text at the caret as the app reports it (name or CSS family list, size in
/// physical pixels, weight, italic), plus the on-screen width of the text before the caret.
/// `font::resolve` turns this into the font the ghost is drawn in.
unsafe fn field_font(caret: &IUIAutomationTextRange, before: &str) -> Reported {
    unsafe {
        let sample = width_sample(caret, before);
        let Some(probe) = char_near(caret, before) else { return Reported { sample, ..Default::default() } };
        let attr = |id| probe.GetAttributeValue(id).ok();
        // Large enough for a CSS family list.
        let mut buf = [0u16; 512];
        let families = attr(UIA_FontNameAttributeId)
            .filter(|v| VariantToString(v, &mut buf).is_ok())
            .map(|_| String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(0)]))
            .unwrap_or_default();
        let dpi = GetDpiForWindow(GetForegroundWindow()).max(96) as f64;
        let px = attr(UIA_FontSizeAttributeId).and_then(|v| f64::try_from(&v).ok()).filter(|pt| *pt > 0.0).map(|pt| pt * dpi / 72.0);
        let weight = attr(UIA_FontWeightAttributeId).and_then(|v| i32::try_from(&v).ok());
        let italic = attr(UIA_IsItalicAttributeId).and_then(|v| bool::try_from(&v).ok()).unwrap_or(false);
        Reported { families, px, weight, italic, sample }
    }
}

/// Characters measured for `width_sample`; fewer than `SAMPLE_MIN` say little about the size.
const SAMPLE_MAX: i32 = 32;
const SAMPLE_MIN: usize = 6;

/// The text on the caret's line just before the caret, and how wide it is on screen (and its
/// first half), measured from the text range's bounding rectangles.
unsafe fn width_sample(caret: &IUIAutomationTextRange, before: &str) -> Option<WidthSample> {
    unsafe {
        let line_text = before.rsplit('\n').next().unwrap_or("");
        // Trailing spaces: some apps give them no width at a line end; leave them out.
        let trailing = line_text.chars().rev().take_while(|c| *c == ' ').count() as i32;
        let r = caret.Clone().ok()?;
        // Collapse to the caret, then move the start back first: an end moved before the start
        // would drag the start along.
        r.MoveEndpointByRange(TextPatternRangeEndpoint_End, caret, TextPatternRangeEndpoint_Start).ok()?;
        r.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -(trailing + SAMPLE_MAX)).ok()?;
        r.MoveEndpointByUnit(TextPatternRangeEndpoint_End, TextUnit_Character, -trailing).ok()?;
        // Not past the start of the caret's (visual) line.
        let line = caret.Clone().ok()?;
        line.ExpandToEnclosingUnit(TextUnit_Line).ok()?;
        if r.CompareEndpoints(TextPatternRangeEndpoint_Start, &line, TextPatternRangeEndpoint_Start).ok()? < 0 {
            r.MoveEndpointByRange(TextPatternRangeEndpoint_Start, &line, TextPatternRangeEndpoint_Start).ok()?;
        }
        let text = normalize(&r.GetText(-1).ok()?.to_string());
        let n = text.chars().count();
        if n < SAMPLE_MIN || text.contains('\n') {
            return None;
        }
        let all = rects(&r);
        let last = *all.last()?;
        // All on one line, or the sample crosses a wrap and its width means nothing.
        if all.iter().any(|q| (q.y - last.y).abs() > last.h / 2.0) {
            return None;
        }
        let left = all.iter().map(|q| q.x).fold(f64::INFINITY, f64::min);
        let width = all.iter().map(|q| q.x + q.w).fold(0.0, f64::max) - left;
        let head_chars = n / 2;
        let head = r.Clone().ok()?;
        head.MoveEndpointByRange(TextPatternRangeEndpoint_End, &r, TextPatternRangeEndpoint_Start).ok()?;
        head.MoveEndpointByUnit(TextPatternRangeEndpoint_End, TextUnit_Character, head_chars as i32).ok()?;
        let head_width = rects(&head).iter().map(|q| q.x + q.w).fold(0.0, f64::max) - left;
        (width > 0.0 && head_width > 0.0 && head_width < width).then_some(WidthSample { text, width, head_chars, head_width })
    }
}

/// x where the caret's line starts, so wrapped ghost lines line up with the text.
unsafe fn line_left(caret: &IUIAutomationTextRange) -> Option<f64> {
    unsafe {
        let line = caret.Clone().ok()?;
        line.ExpandToEnclosingUnit(TextUnit_Line).ok()?;
        rects(&line).first().map(|r| r.x)
    }
}

/// `GetBoundingRectangles` as rects (the SAFEARRAY holds left, top, width, height doubles).
unsafe fn rects(range: &IUIAutomationTextRange) -> Vec<Rect> {
    unsafe {
        let Ok(sa) = range.GetBoundingRectangles() else { return Vec::new() };
        if sa.is_null() {
            return Vec::new();
        }
        let mut out = Vec::new();
        if let Ok(ub) = SafeArrayGetUBound(sa, 1) {
            let mut data: *mut c_void = std::ptr::null_mut();
            if SafeArrayAccessData(sa, &mut data).is_ok() {
                let v = std::slice::from_raw_parts(data as *const f64, (ub + 1).max(0) as usize);
                out = v.chunks_exact(4).map(|c| Rect { x: c[0], y: c[1], w: c[2], h: c[3] }).collect();
                let _ = SafeArrayUnaccessData(sa);
            }
        }
        let _ = SafeArrayDestroy(sa as *const SAFEARRAY);
        out
    }
}

/// The Win32 system caret of the foreground thread, for apps whose text pattern has no geometry.
fn gui_caret() -> Option<Rect> {
    unsafe {
        let fg = GetForegroundWindow();
        let tid = GetWindowThreadProcessId(fg, None);
        let mut gi = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        GetGUIThreadInfo(tid, &mut gi).ok()?;
        if gi.hwndCaret.is_invalid() {
            return None;
        }
        let rc = gi.rcCaret;
        let mut p = POINT { x: rc.left, y: rc.top };
        let _ = ClientToScreen(gi.hwndCaret, &mut p);
        let h = (rc.bottom - rc.top) as f64;
        (h > 0.0).then_some(Rect { x: p.x as f64, y: p.y as f64, w: 1.0, h })
    }
}

fn foreground_title() -> String {
    unsafe { window_text(GetForegroundWindow()) }
}

unsafe fn window_text(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// (display name, lowercase exe file name) of a process; the exe name is the app key.
fn exe_of(pid: u32) -> (String, String) {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return (String::new(), String::new()) };
        let mut buf = [0u16; MAX_PATH as usize * 2];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return (String::new(), String::new());
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        let file = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_owned();
        let name = file.strip_suffix(".exe").or_else(|| file.strip_suffix(".EXE")).unwrap_or(&file).to_owned();
        (name, file.to_lowercase())
    }
}
