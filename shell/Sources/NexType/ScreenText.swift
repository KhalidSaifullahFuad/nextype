import AppKit
import ScreenCaptureKit
import Vision

/// Screen text (spec §7.2): captures the focused field's window, reads the part above the field
/// with Vision OCR, and hands the text to the core. The core picks the field (never in excluded
/// apps or secure fields); this only captures and reads.
@MainActor
final class ScreenText {
    static let shared = ScreenText()
    /// Re-read the same window and field at most this often.
    private static let refresh: TimeInterval = 30
    /// Long edge of the captured image (spec §7.2).
    private static let maxPixels: CGFloat = 2000
    private var last: (key: String, at: Date)?
    private var busy = false

    /// Called from the shell's 2 s timer.
    func tick() {
        guard UserDefaults.standard.bool(forKey: "screenContext"), !busy,
              CGPreflightScreenCaptureAccess(), let t = screenTarget() else { return }
        let key = "\(t.pid)|\(t.windowTitle)|\(Int(t.y))|\(Int(t.h))"
        if let last, last.key == key, Date().timeIntervalSince(last.at) < Self.refresh { return }
        busy = true
        Task {
            let text = (try? await Self.read(t)) ?? ""
            setScreenText(pid: t.pid, windowTitle: t.windowTitle, text: text)
            last = (key, Date())
            busy = false
        }
    }

    private static func read(_ t: ScreenTarget) async throws -> String {
        let field = CGRect(x: t.x, y: t.y, width: t.w, height: t.h)
        let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true)
        guard let window = content.windows.first(where: {
            $0.owningApplication?.processID == t.pid && $0.windowLayer == 0
                && $0.frame.contains(CGPoint(x: field.midX, y: field.minY))
        }) else { return "" }
        // Only the window above the field: the field's own text is already in the prompt.
        let above = field.minY - window.frame.minY
        guard above > 20 else { return "" }
        let config = SCStreamConfiguration()
        let scale = min(maxPixels / max(window.frame.width, window.frame.height), 2)
        config.width = Int(window.frame.width * scale)
        config.height = Int(window.frame.height * scale)
        config.showsCursor = false
        let image = try await SCScreenshotManager.captureImage(
            contentFilter: SCContentFilter(desktopIndependentWindow: window), configuration: config)
        let s = CGFloat(image.width) / window.frame.width
        guard let top = image.cropping(to: CGRect(x: 0, y: 0, width: CGFloat(image.width), height: above * s)) else { return "" }
        return try await Task.detached(priority: .userInitiated) { try ocr(top) }.value
    }

    /// Lines top to bottom; single words (buttons, menu items) and unsure reads are dropped.
    nonisolated private static func ocr(_ image: CGImage) throws -> String {
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.automaticallyDetectsLanguage = true
        try VNImageRequestHandler(cgImage: image).perform([request])
        // shortcut: one observation per line, no re-joining of wrapped lines (spec §7.2 cleanup).
        return (request.results ?? [])
            .filter { $0.confidence >= 0.3 }
            .sorted { ($0.boundingBox.maxY, -$0.boundingBox.minX) > ($1.boundingBox.maxY, -$1.boundingBox.minX) }
            .compactMap { $0.topCandidates(1).first?.string }
            .filter { $0.split(separator: " ").count >= 2 }
            .joined(separator: "\n")
    }
}
