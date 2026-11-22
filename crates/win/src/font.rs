//! The ghost's font: the field's own font where Windows can draw it, sized so the ghost is as
//! wide as the field's text really is on screen.
//!
//! What UI Automation reports is not always drawable as-is. Chromium and WebView2 apps report the
//! CSS family list ("Inter, -apple-system, \"Segoe UI\", sans-serif"), which may start with a web
//! font that isn't installed, and their point size ignores page zoom. So: take the first family
//! in the list that is installed (CSS generic names mapped to what Chrome draws on Windows), then
//! measure the last few characters before the caret on screen and scale the size until the same
//! text in that font has the same width.

use std::cell::RefCell;
use std::collections::HashMap;

use nextype_core::WidthSample;
use windows::Win32::Foundation::LPARAM;
use windows::Win32::Graphics::Gdi::*;

/// What the field reports about the text at the caret.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reported {
    /// UIA font name: one face, or a CSS family list.
    pub families: String,
    /// Size in physical pixels.
    pub px: Option<f64>,
    pub weight: Option<i32>,
    pub italic: bool,
    /// On-screen width of the text just before the caret.
    pub sample: Option<WidthSample>,
}

/// The font the overlay draws with.
#[derive(Debug, Clone, PartialEq)]
pub struct GhostFont {
    pub face: String,
    /// Height of the em box in physical pixels (`CreateFontW` with a negative height).
    pub px: f64,
    pub weight: i32,
    pub italic: bool,
}

const SANS: &str = "Segoe UI";
const MONO: &str = "Consolas";
/// Size used for measuring; large so rounding of the measured width doesn't matter.
const PROBE_PX: f64 = 100.0;
/// A measured size this close to the reported one keeps the reported one.
const SAME_SIZE: f64 = 0.04;
/// A measured size this far off means the sample is unreliable (mixed fonts, an image inline).
const SIZE_RANGE: (f64, f64) = (0.6, 1.7);

/// Pick the ghost font. `caret_h` sizes the fallback when the field reports nothing.
pub fn resolve(r: &Reported, caret_h: f64) -> GhostFont {
    let weight = r.weight.filter(|w| (100..=950).contains(w)).unwrap_or(400);
    let italic = r.italic;
    let families = families(&r.families);
    let named = families.iter().find(|f| installed(f)).cloned();
    let face = match (&named, &r.sample) {
        (Some(f), _) => f.clone(),
        // No usable name: tell fixed-width from proportional text by how the measured width
        // splits between the sample's two halves.
        (None, Some(s)) => guess_face(s, weight, italic),
        (None, None) => SANS.to_owned(),
    };
    let reported = r.px.filter(|p| *p > 0.0);
    let measured = r.sample.as_ref().and_then(|s| fit_size(&face, weight, italic, s));
    let px = match (reported, measured) {
        (Some(rep), Some(m)) => {
            let ratio = m / rep;
            if (ratio - 1.0).abs() <= SAME_SIZE || !(SIZE_RANGE.0..=SIZE_RANGE.1).contains(&ratio) { rep } else { m }
        }
        (Some(rep), None) => rep,
        // Without a reported size the measurement is all we have; still keep it sane.
        (None, Some(m)) if m >= 6.0 && m <= caret_h * 1.5 => m,
        _ => caret_h / 1.3,
    };
    GhostFont { face, px: px.clamp(6.0, 200.0), weight, italic }
}

/// The families of a font name or CSS family list, in order, with CSS generic names replaced
/// by the font Chrome uses for them on Windows.
pub fn families(css: &str) -> Vec<String> {
    css.split(',')
        .map(|f| f.trim().trim_matches(['"', '\'']).trim())
        .filter(|f| !f.is_empty())
        .map(|f| match f.to_ascii_lowercase().as_str() {
            "sans-serif" | "system-ui" | "ui-sans-serif" | "-apple-system" | "blinkmacsystemfont" | "ui-rounded" => SANS.to_owned(),
            "serif" | "ui-serif" => "Times New Roman".to_owned(),
            "monospace" | "ui-monospace" => MONO.to_owned(),
            "cursive" => "Comic Sans MS".to_owned(),
            "fantasy" => "Impact".to_owned(),
            _ => f.to_owned(),
        })
        .collect()
}

fn guess_face(s: &WidthSample, weight: i32, italic: bool) -> String {
    let head: String = s.text.chars().take(s.head_chars).collect();
    let target = s.head_width / s.width;
    [SANS, MONO]
        .into_iter()
        .filter_map(|face| {
            let full = text_width(face, PROBE_PX, weight, italic, &s.text)?;
            let part = text_width(face, PROBE_PX, weight, italic, &head)?;
            (full > 0.0).then(|| ((part / full - target).abs(), face))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map_or(SANS, |(_, face)| face)
        .to_owned()
}

/// Size at which `face` draws the sample's text as wide as it is on screen.
fn fit_size(face: &str, weight: i32, italic: bool, s: &WidthSample) -> Option<f64> {
    let w = text_width(face, PROBE_PX, weight, italic, &s.text)?;
    (w > 0.0 && s.width > 0.0).then(|| PROBE_PX * s.width / w)
}

/// Width of `text` in pixels, drawn in `face` at `px`.
pub fn text_width(face: &str, px: f64, weight: i32, italic: bool, text: &str) -> Option<f64> {
    with_font(&GhostFont { face: face.to_owned(), px, weight, italic }, |dc| {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let mut size = windows::Win32::Foundation::SIZE::default();
        unsafe { GetTextExtentPoint32W(dc, &wide, &mut size) }.as_bool().then_some(size.cx as f64)
    })
    .flatten()
}

/// `CreateFontW` for `f`. The caller owns (and deletes) the font.
pub fn create(f: &GhostFont) -> HFONT {
    let face: Vec<u16> = f.face.encode_utf16().chain([0]).collect();
    unsafe {
        CreateFontW(
            -(f.px.round().max(1.0) as i32), 0, 0, 0, f.weight, f.italic as u32, 0, 0,
            DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY,
            (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32, windows::core::PCWSTR(face.as_ptr()),
        )
    }
}

/// Run `f` with a memory DC that has the font selected.
fn with_font<T>(font: &GhostFont, f: impl FnOnce(HDC) -> T) -> Option<T> {
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return None;
        }
        let hf = create(font);
        let old = SelectObject(dc, hf.into());
        let out = f(dc);
        SelectObject(dc, old);
        let _ = DeleteObject(hf.into());
        let _ = DeleteDC(dc);
        Some(out)
    }
}

thread_local! {
    static INSTALLED: RefCell<HashMap<String, bool>> = RefCell::new(HashMap::new());
}

/// Whether a font family of this name is installed (GDI would otherwise substitute another).
pub fn installed(face: &str) -> bool {
    // LOGFONTW holds 31 characters and a NUL.
    if face.is_empty() || face.encode_utf16().count() > 31 {
        return false;
    }
    let key = face.to_lowercase();
    if let Some(&known) = INSTALLED.with(|c| c.borrow().get(&key).copied()).as_ref() {
        return known;
    }
    let mut lf = LOGFONTW { lfCharSet: DEFAULT_CHARSET, ..Default::default() };
    for (d, s) in lf.lfFaceName.iter_mut().zip(face.encode_utf16()) {
        *d = s;
    }
    let mut found = false;
    unsafe extern "system" fn on_font(_: *const LOGFONTW, _: *const TEXTMETRICW, _: u32, found: LPARAM) -> i32 {
        unsafe { *(found.0 as *mut bool) = true };
        0 // one match is enough
    }
    unsafe {
        let dc = CreateCompatibleDC(None);
        if !dc.is_invalid() {
            EnumFontFamiliesExW(dc, &lf, Some(on_font), LPARAM(&mut found as *mut bool as isize), 0);
            let _ = DeleteDC(dc);
        }
    }
    INSTALLED.with(|c| c.borrow_mut().insert(key, found));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_of(face: &str, px: f64, text: &str) -> WidthSample {
        let head_chars = text.chars().count() / 2;
        let head: String = text.chars().take(head_chars).collect();
        WidthSample {
            text: text.into(),
            width: text_width(face, px, 400, false, text).unwrap(),
            head_chars,
            head_width: text_width(face, px, 400, false, &head).unwrap(),
        }
    }

    #[test]
    fn css_lists_become_installed_families() {
        assert_eq!(families("Inter, -apple-system, \"Segoe UI\", sans-serif"), ["Inter", "Segoe UI", "Segoe UI", "Segoe UI"]);
        assert_eq!(families("'JetBrains Mono', monospace"), ["JetBrains Mono", "Consolas"]);
        assert_eq!(families("Calibri"), ["Calibri"]);
        assert!(families("  ,  ").is_empty());
        assert!(installed("Segoe UI") && installed("segoe ui"));
        assert!(!installed("No Such Font NexType"));
    }

    #[test]
    fn web_font_falls_to_the_next_installed_family_and_takes_the_screen_size() {
        // Page zoomed to 125%: the app says 16 px, the screen shows 20 px Segoe UI.
        let r = Reported {
            families: "NotInstalledWebFont, \"Segoe UI\", sans-serif".into(),
            px: Some(16.0),
            sample: Some(sample_of(SANS, 20.0, "and that is how it went")),
            ..Default::default()
        };
        let f = resolve(&r, 26.0);
        assert_eq!(f.face, SANS);
        assert!((f.px - 20.0).abs() < 0.6, "{f:?}");
    }

    #[test]
    fn a_matching_report_is_kept_and_an_odd_sample_is_ignored() {
        let close = Reported { families: SANS.into(), px: Some(15.0), sample: Some(sample_of(SANS, 15.2, "the quick brown fox")), ..Default::default() };
        assert_eq!(resolve(&close, 20.0).px, 15.0);
        let odd = Reported { families: SANS.into(), px: Some(15.0), sample: Some(sample_of(SANS, 40.0, "the quick brown fox")), ..Default::default() };
        assert_eq!(resolve(&odd, 20.0).px, 15.0);
    }

    #[test]
    fn no_name_tells_fixed_width_from_proportional() {
        let mono = Reported { sample: Some(sample_of(MONO, 14.0, "let total = items.len();")), ..Default::default() };
        assert_eq!(resolve(&mono, 19.0).face, MONO);
        let prop = Reported { sample: Some(sample_of(SANS, 14.0, "Will you be there at five")), ..Default::default() };
        assert_eq!(resolve(&prop, 19.0).face, SANS);
    }
}
