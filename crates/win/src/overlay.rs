//! Ghost text window: layered, click-through, topmost, never activated. Text is drawn with
//! grayscale antialiasing into a 32-bit DIB and turned into per-pixel alpha, so it blends over
//! light and dark fields alike.

use std::ffi::c_void;

use nextype_core::Rect;

use crate::font::GhostFont;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;


/// Gray of the ghost and how opaque it is (0–255).
const GRAY: u32 = 128;
const OPACITY: u32 = 200;

pub struct Overlay {
    hwnd: HWND,
}

impl Overlay {
    pub fn new() -> windows::core::Result<Self> {
        unsafe {
            let inst = GetModuleHandleW(None)?;
            let class = w!("NexTypeGhost");
            let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: class, ..Default::default() };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                class,
                w!(""),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(inst.into()),
                None,
            )?;
            Ok(Self { hwnd })
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    /// Draw `text` from the caret on, in `font` (see `font::resolve`). With `edges` (left, right)
    /// the ghost wraps between them like the field's own text; otherwise it stays on one line,
    /// cut after the last whole word that fits before `clip_right`. Returns how many bytes of
    /// `text` are visible (0 = nothing drawn, the window is hidden), so accepting takes only
    /// what the user can see.
    pub fn show(&self, text: &str, caret: Rect, font: &GhostFont, edges: Option<(f64, f64)>, clip_right: f64) -> usize {
        let caret_x = caret.x + caret.w;
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let font = crate::font::create(font);
            let old_font = SelectObject(mem, font.into());
            let measure = |s: &str| {
                let wide: Vec<u16> = s.encode_utf16().collect();
                let mut size = SIZE::default();
                let _ = GetTextExtentPoint32W(mem, &wide, &mut size);
                size.cx as f64
            };
            let mut tm = TEXTMETRICW::default();
            let _ = GetTextMetricsW(mem, &mut tm);
            let line_h = caret.h.max(tm.tmHeight as f64).round() as i32;
            let (ascent, descent) = (tm.tmAscent as f64, tm.tmDescent as f64);
            // Baseline below the caret box top, and room above it for tall glyphs.
            let base = baseline_in_box(caret.h, ascent, descent);
            let pad = (ascent - base).max(0.0).ceil();
            let first_top = (pad + base - ascent).round() as i32;

            let (x0, right) = match edges {
                Some((left, right)) => (left, right),
                None => (caret_x, clip_right.max(caret_x + 1.0)),
            };
            let (lines, visible) = layout(text, right - caret_x, right - x0, edges.is_some(), &measure);
            if visible == 0 || lines.iter().all(|l| l.trim().is_empty()) {
                SelectObject(mem, old_font);
                let _ = DeleteObject(font.into());
                let _ = DeleteDC(mem);
                ReleaseDC(None, screen);
                self.hide();
                return 0;
            }
            let w = (right - x0).round().max(1.0) as i32;
            let h = first_top + tm.tmHeight + line_h * (lines.len().max(1) as i32 - 1);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut c_void = std::ptr::null_mut();
            let Ok(dib) = CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
                SelectObject(mem, old_font);
                let _ = DeleteObject(font.into());
                let _ = DeleteDC(mem);
                ReleaseDC(None, screen);
                return 0;
            };
            let old_bmp = SelectObject(mem, dib.into());
            // White text on black; coverage becomes alpha below.
            let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (w * h) as usize);
            pixels.fill(0);
            SetBkMode(mem, TRANSPARENT);
            SetTextColor(mem, COLORREF(0x00FF_FFFF));
            for (i, line) in lines.iter().enumerate() {
                let left = if i == 0 { (caret_x - x0).round() as i32 } else { 0 };
                let top = first_top + line_h * i as i32;
                let mut rc = RECT { left, top, right: w, bottom: top + tm.tmHeight };
                // Layout measured lines without their trailing space; draw them the same way.
                let mut wide: Vec<u16> = line.trim_end().encode_utf16().collect();
                DrawTextW(mem, &mut wide, &mut rc, DT_SINGLELINE | DT_TOP | DT_NOPREFIX | DT_LEFT);
            }
            for p in pixels.iter_mut() {
                let a = (*p & 0xFF) * OPACITY / 255;
                let c = GRAY * a / 255; // premultiplied
                *p = (a << 24) | (c << 16) | (c << 8) | c;
            }

            let dst = POINT { x: x0.round() as i32, y: (caret.y - pad).round() as i32 };
            let src = POINT::default();
            let sz = SIZE { cx: w, cy: h };
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            let _ = UpdateLayeredWindow(self.hwnd, Some(screen), Some(&dst), Some(&sz), Some(mem), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA);
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);

            SelectObject(mem, old_bmp);
            SelectObject(mem, old_font);
            let _ = DeleteObject(dib.into());
            let _ = DeleteObject(font.into());
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
            visible
        }
    }
}

/// Where the field's baseline is, measured down from the top of a caret box `box_h` tall, for a
/// ghost font with these ascent and descent. A box about one font tall (Chromium, WebView2) is the
/// text's own box: the baseline sits at the same fraction of it as in the font, whichever font the
/// field uses. A box clearly taller holds extra line spacing, which apps like Notepad's RichEdit
/// put above the text: the baseline is one descent above the bottom.
fn baseline_in_box(box_h: f64, ascent: f64, descent: f64) -> f64 {
    let glyph = ascent + descent;
    if glyph <= 0.0 {
        return box_h;
    }
    if box_h >= glyph * 1.15 { box_h - descent } else { box_h * ascent / glyph }
}

/// Most lines a wrapped ghost may take.
const MAX_LINES: usize = 6;

/// Split `text` into lines: the first `first` px wide (it starts at the caret), the rest `rest`
/// px. Breaks at spaces and at newlines; a word wider than a wrapped line is left to be cut.
/// Without `wrap` there is one line, ending at the first newline or after the last whole word
/// that fits. Returns the lines and how many bytes of `text` they show.
fn layout(text: &str, first: f64, rest: f64, wrap: bool, measure: &dyn Fn(&str) -> f64) -> (Vec<String>, usize) {
    if !wrap {
        let line = text.split('\n').next().unwrap_or("");
        let mut shown = 0;
        for word in line.split_inclusive(' ') {
            if measure(line[..shown + word.len()].trim_end()) > first {
                break;
            }
            shown += word.len();
        }
        return (vec![line[..shown].to_owned()], shown);
    }
    let mut lines = vec![String::new()];
    // Byte offset in `text` where each line starts.
    let mut starts = vec![0];
    let mut at = 0;
    for (p, para) in text.split('\n').enumerate() {
        if p > 0 {
            at += 1;
            lines.push(String::new());
            starts.push(at);
        }
        for word in para.split_inclusive(' ') {
            let n = lines.len();
            let avail = if n == 1 { first } else { rest };
            let cur = lines.last_mut().expect("one line");
            let fits = measure(format!("{cur}{word}").trim_end()) <= avail;
            // A word too wide for an empty wrapped line stays there (drawing cuts it).
            if fits || cur.trim().is_empty() && n > 1 {
                cur.push_str(word);
            } else {
                lines.push(word.to_owned());
                starts.push(at);
            }
            at += word.len();
        }
    }
    let shown = starts.get(MAX_LINES).copied().unwrap_or(text.len());
    lines.truncate(MAX_LINES);
    (lines, shown)
}

#[cfg(test)]
mod tests {
    use super::{baseline_in_box, layout};

    #[test]
    fn baseline_follows_the_box_the_field_reports() {
        // Day One: 27 px box (Inter's own height); Segoe UI ghost 25 px ascent + 7 px descent.
        let b = baseline_in_box(27.0, 25.0, 7.0);
        assert!((b - 21.1).abs() < 0.1, "{b}");
        // Notepad: 30 px box around a 19 px font, spacing above the text.
        assert_eq!(baseline_in_box(30.0, 15.0, 4.0), 26.0);
    }

    /// 10 px per char.
    fn m(s: &str) -> f64 {
        s.chars().count() as f64 * 10.0
    }

    #[test]
    fn wraps_first_line_at_caret_room() {
        let (l, shown) = layout("aa bb cc dd", 50.0, 80.0, true, &m);
        assert_eq!(l, ["aa bb ", "cc dd"]);
        assert_eq!(shown, 11);
    }

    #[test]
    fn no_room_after_caret_starts_on_next_line() {
        let (l, shown) = layout("hello world", 20.0, 200.0, true, &m);
        assert_eq!(l, ["", "hello world"]);
        assert_eq!(shown, 11);
    }

    #[test]
    fn wrapped_lines_past_the_limit_are_not_shown() {
        let text = "a b c d e f g h";
        let (l, shown) = layout(text, 10.0, 10.0, true, &m);
        assert_eq!(l.len(), super::MAX_LINES);
        assert_eq!(&text[..shown], "a b c d e f ");
    }

    #[test]
    fn single_line_cuts_at_last_whole_word() {
        let (l, shown) = layout(" it is ready", 60.0, 0.0, false, &m);
        assert_eq!(l, [" it is "]);
        assert_eq!(shown, 7);
        assert_eq!(layout("toolong word", 50.0, 0.0, false, &m).1, 0);
    }

    #[test]
    fn single_line_stops_at_newline() {
        let (l, shown) = layout("a\nb", 100.0, 0.0, false, &m);
        assert_eq!(l, ["a"]);
        assert_eq!(shown, 1);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => unsafe { DefWindowProcW(hwnd, msg, w, l) },
    }
}
