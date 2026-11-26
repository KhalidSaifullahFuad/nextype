import AppKit
import SwiftUI

/// First-run window: grant Accessibility (needs a relaunch to take effect), then fetch the model.
struct OnboardingView: View {
    @ObservedObject private var core = Core.shared
    @ObservedObject private var download = modelDownload

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Welcome to NexType").font(.title.bold())
            Text("Inline writing suggestions in any app. Everything runs on your Mac.")
                .foregroundStyle(.secondary)

            step(
                done: core.status != .needsAccessibility,
                title: "1. Allow Accessibility",
                detail: "NexType reads the text before your cursor to suggest what comes next. macOS applies the permission after a relaunch."
            ) {
                Button("Grant Accessibility…") { core.requestAccessibility() }
                Button("Relaunch") { core.relaunch() }
            }

            step(
                done: core.status == .running,
                title: "2. Download the model",
                detail: "One-time 2.8 GB download. Suggestions are generated on your Mac; your text never leaves it."
            ) {
                if case .running(let fraction) = download.state {
                    ProgressView(value: fraction) { Text(downloadLine(download.state) ?? "") }
                } else if let line = downloadLine(download.state) {
                    Text(line).foregroundStyle(download.state == .verifying ? Color.secondary : Color.red)
                }
                if case .running = download.state { Button("Cancel") { download.cancel() } }
                if !download.isBusy && core.status != .running {
                    Button("Download (2.8 GB)") { download.start() }
                        .disabled(core.status == .needsAccessibility)
                }
            }

            if core.status == .running {
                Text("All set. Start typing in any app, then press Tab to accept a suggestion.")
                    .foregroundStyle(.green)
            }
        }
        .padding(28)
        .frame(width: 460)
    }

    private func step<Actions: View>(
        done: Bool, title: String, detail: String, @ViewBuilder actions: () -> Actions
    ) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: done ? "checkmark.circle.fill" : "circle")
                .foregroundStyle(done ? Color.green : Color.secondary)
                .font(.title2)
            VStack(alignment: .leading, spacing: 6) {
                Text(title).font(.headline)
                Text(detail).font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                if !done { HStack { actions() } }
            }
        }
    }
}

@MainActor
enum Onboarding {
    private static var window: NSWindow?

    /// Shown at launch until everything is set up. `open NexType.app --args -showOnboarding YES` forces it.
    static func showIfNeeded() {
        guard Core.shared.status != .running || UserDefaults.standard.bool(forKey: "showOnboarding") else { return }
        let w = NSWindow(contentViewController: NSHostingController(rootView: OnboardingView()))
        w.title = "NexType"
        w.styleMask = [.titled, .closable]
        w.isReleasedWhenClosed = false
        w.center()
        window = w
        NSApp.activate()
        w.makeKeyAndOrderFront(nil)
    }
}
