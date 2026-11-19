//! Ghost-text overlay (spec §11): a borderless, click-through panel that never activates,
//! holding one grey label. The label spans the field's text width and indents its first line
//! to the caret, so the ghost wraps the way the field's own text would.

use nextype_core::{Rect, WidthSample};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAttributedStringNSStringDrawing, NSBackingStoreType, NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName,
    NSMutableParagraphStyle, NSPanel, NSParagraphStyleAttributeName, NSScreen, NSStatusWindowLevel,
    NSTextField, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSAttributedString, NSDictionary, NSPoint, NSRect, NSSize, NSString};

/// canJoinAllSpaces | transient | ignoresCycle | fullScreenAuxiliary (spec §11.1).
const COLLECTION_BEHAVIOR: usize = 0x149;
/// NSTextField draws its text this far inside the frame on each side (line-fragment padding).
const LABEL_PAD_X: f64 = 2.0;
/// Width used when there is nothing to wrap against (single-line fields).
const NO_WRAP: f64 = 100_000.0;

pub struct Overlay {
    panel: Retained<NSPanel>,
    label: Retained<NSTextField>,
    mtm: MainThreadMarker,
}

impl Overlay {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            NSRect::ZERO,
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        );
        panel.setFloatingPanel(true);
        // Panels hide when their app is inactive by default, and NexType is never active.
        panel.setHidesOnDeactivate(false);
        panel.setIgnoresMouseEvents(true);
        panel.setCollectionBehavior(NSWindowCollectionBehavior(COLLECTION_BEHAVIOR));
        panel.setLevel(NSStatusWindowLevel);
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(false);
        let label = NSTextField::wrappingLabelWithString(&NSString::new(), mtm);
        panel.setContentView(Some(&label));
        Self { panel, label, mtm }
    }

    /// Show `text` with its first glyph at the caret, wrapping between `edges` (left, right) when
    /// given. All coordinates are AX (top-left origin).
    pub fn show(&self, text: &str, caret: Rect, font: Option<&(String, f64)>, edges: Option<(f64, f64)>) {
        let font = font
            .and_then(|(name, size)| NSFont::fontWithName_size(&NSString::from_str(name), *size))
            .unwrap_or_else(|| NSFont::systemFontOfSize(caret.h * 0.8));
        let (left, width) = edges.map_or((caret.x, NO_WRAP), |(l, r)| (l, r - l));
        let para = NSMutableParagraphStyle::new();
        para.setFirstLineHeadIndent(caret.x - left);
        let color = NSColor::grayColor();
        let values: [&AnyObject; 3] = [&font, &color, &para];
        let keys = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName, NSParagraphStyleAttributeName] };
        let attrs = NSDictionary::from_slices(&keys, &values);
        let s = unsafe {
            NSAttributedString::initWithString_attributes(NSAttributedString::alloc(), &NSString::from_str(text), Some(&attrs))
        };
        self.label.setAttributedStringValue(&s);
        let size = self.label.sizeThatFits(NSSize::new(width + 2.0 * LABEL_PAD_X, NO_WRAP));
        let w = if edges.is_some() { width + 2.0 * LABEL_PAD_X } else { size.width };
        // AppKit's origin is the primary screen's bottom-left; AX's is its top-left.
        let primary_h = NSScreen::screens(self.mtm).firstObject().map_or(0.0, |s| s.frame().size.height);
        // Top-aligned to the caret line: with the field's font, the baselines then match.
        let origin = NSPoint::new(left - LABEL_PAD_X, primary_h - caret.y - size.height);
        self.panel.setFrame_display(NSRect::new(origin, NSSize::new(w, size.height)), true);
        self.panel.orderFrontRegardless();
    }

    pub fn hide(&self) {
        self.panel.orderOut(None);
    }
}

fn text_width(text: &str, font: &NSFont) -> f64 {
    let values: [&AnyObject; 1] = [font];
    let keys = unsafe { [NSFontAttributeName] };
    let attrs = NSDictionary::from_slices(&keys, &values);
    let s = unsafe { NSAttributedString::initWithString_attributes(NSAttributedString::alloc(), &NSString::from_str(text), Some(&attrs)) };
    s.size().width
}

/// For apps that don't report their font (VS Code, Chrome): pick the system font or Menlo by how
/// the measured width splits between the sample's two halves (fixed-width text splits by
/// character count, proportional text doesn't), then size it to the measured width.
pub fn guess_font(w: &WidthSample) -> Option<(String, f64)> {
    const PROBE: f64 = 12.0;
    let head: String = w.text.chars().take(w.head_chars).collect();
    let measured = w.head_width / w.width;
    let system = NSFont::systemFontOfSize(PROBE).fontName().to_string();
    [system.as_str(), "Menlo-Regular"]
        .into_iter()
        .filter_map(|name| {
            let f = NSFont::fontWithName_size(&NSString::from_str(name), PROBE)?;
            let full = text_width(&w.text, &f);
            (full > 0.0).then(|| ((text_width(&head, &f) / full - measured).abs(), name, PROBE * w.width / full))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, name, size)| (name.to_owned(), size.clamp(6.0, 72.0)))
}
