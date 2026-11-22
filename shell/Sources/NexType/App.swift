import AppKit
import SwiftUI

/// Where the model lives (M4 download target). Dev: a symlink to the checkout's `models/` file.
let modelURL = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
    .appendingPathComponent("NexType/models/gemma-4-E2B-it-Q4_0.gguf")

/// Pinned model (spec §16): ggml-org/gemma-4-E2B-it-GGUF at a fixed commit, Q4_0.
let modelDownload = Download(
    url: URL(string: "https://huggingface.co/ggml-org/gemma-4-E2B-it-GGUF/resolve/2a8538c2e2d4687953a86c422c7c623cfd022880/gemma-4-E2B-it-Q4_0.gguf")!,
    dest: modelURL,
    sha256: "8e30dff3ac4c8434c49a7036fa15564bdbb6044e42bf04550bf1a096ad7e6a52",
    size: 2_841_481_184
)

/// "Downloading 42%" style line for menus; nil when there is nothing to say.
func downloadLine(_ s: Download.State) -> String? {
    switch s {
    case .running(let f): "Downloading model… \(Int(f * 100))%"
    case .verifying: "Verifying download…"
    case .failed(let why): why
    default: nil
    }
}

/// One app's overrides (spec §14.2 subset), stored as JSON under `appOverrides`.
struct AppSetting: Codable, Identifiable, Equatable {
    var bundleID: String
    var completionsOff = false
    var tabOff = false
    var instructions = ""
    var id: String { bundleID }
}

/// Display name for an override key: the app's name, or the host for `domain:<host>`.
func appName(_ bundleID: String) -> String {
    if bundleID.hasPrefix("domain:") { return String(bundleID.dropFirst(7)) }
    guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleID) else { return bundleID }
    // displayName keeps ".app" when Finder shows extensions.
    return url.deletingPathExtension().lastPathComponent
}

enum Status {
    case needsAccessibility, modelMissing, running

    var text: String {
        switch self {
        case .needsAccessibility: "Needs Accessibility permission"
        case .modelMissing: "Model not downloaded"
        case .running: "Ready"
        }
    }
}

/// UI-visible state; pushes the persisted settings into the Rust core.
@MainActor
final class Core: ObservableObject {
    static let shared = Core()
    @Published var status: Status = .needsAccessibility
    /// Per-app overrides (the core also has its own built-in exclusion list).
    @Published var apps: [AppSetting] = Core.loadApps() {
        didSet {
            UserDefaults.standard.set(try? JSONEncoder().encode(apps), forKey: "appOverrides")
            pushApps()
        }
    }
    /// The last app the user worked in (not NexType), for the menu's per-app items.
    @Published var frontApp: NSRunningApplication? = NSWorkspace.shared.frontmostApplication
    @Published var pausedUntil: Date?
    @Published var today = (completions: 0, words: 0)
    /// Website of the field NexType last read ("" outside browsers).
    @Published var domain = ""
    private var statsTimer: Timer?

    private init() {
        modelDownload.onDone = { [weak self] in self?.launch() }
        NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didActivateApplicationNotification, object: nil, queue: .main
        ) { note in
            let app = note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication
            MainActor.assumeIsolated {
                if let app, app.bundleIdentifier != Bundle.main.bundleIdentifier { Core.shared.frontApp = app }
            }
        }
        refreshStats()
        statsTimer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { _ in
            MainActor.assumeIsolated {
                Core.shared.refreshStats()
                ScreenText.shared.tick()
            }
        }
    }

    private static func loadApps() -> [AppSetting] {
        let d = UserDefaults.standard
        if let data = d.data(forKey: "appOverrides"), let apps = try? JSONDecoder().decode([AppSetting].self, from: data) {
            return apps
        }
        // Earlier builds stored a plain list of turned-off apps.
        return (d.stringArray(forKey: "excludedApps") ?? []).map { AppSetting(bundleID: $0, completionsOff: true) }
    }

    private func pushApps() {
        setAppOverrides(overrides: apps.map {
            AppOverride(bundleId: $0.bundleID, completionsOff: $0.completionsOff, tabOff: $0.tabOff, instructions: $0.instructions)
        })
    }

    func setting(_ bundleID: String) -> AppSetting {
        apps.first { $0.bundleID == bundleID } ?? AppSetting(bundleID: bundleID, completionsOff: isOffByDefault(bundleId: bundleID))
    }

    /// Change one app's overrides, adding its record if needed.
    func update(_ bundleID: String, _ change: (inout AppSetting) -> Void) {
        var a = setting(bundleID)
        change(&a)
        if let i = apps.firstIndex(where: { $0.bundleID == bundleID }) { apps[i] = a } else { apps.append(a) }
    }

    func refreshStats() {
        let d = currentDomain()
        if d != domain { domain = d }
        let s = statsToday()
        if (Int(s.completions), Int(s.words)) != today { today = (Int(s.completions), Int(s.words)) }
    }

    func pause(minutes: Int) {
        let until = Date().addingTimeInterval(Double(minutes) * 60)
        pausedUntil = until
        apply()
        DispatchQueue.main.asyncAfter(deadline: .now() + Double(minutes) * 60) {
            MainActor.assumeIsolated {
                if Core.shared.pausedUntil == until { Core.shared.resume() }
            }
        }
    }

    func resume() {
        pausedUntil = nil
        apply()
    }

    func launch() {
        apply()
        guard accessibilityTrusted(prompt: false) else { return status = .needsAccessibility }
        guard FileManager.default.fileExists(atPath: modelURL.path) else { return status = .modelMissing }
        status = start(modelPath: modelURL.path) ? .running : .needsAccessibility
    }

    /// Push every persisted setting to the core.
    func apply() {
        let d = UserDefaults.standard
        setEnabled(on: (d.object(forKey: "enabled") as? Bool ?? true) && pausedUntil == nil)
        setLength(length: Self.length(d.object(forKey: "length") as? Int ?? 1))
        setInstructions(text: d.string(forKey: "instructions") ?? "")
        setClipboardContext(on: d.bool(forKey: "clipboardContext"))
        setScreenContext(on: d.bool(forKey: "screenContext"))
        setHistory(on: d.bool(forKey: "history"))
        pushApps()
    }

    static func length(_ i: Int) -> Length {
        switch i {
        case 0: .short
        case 2: .long
        default: .medium
        }
    }

    /// Ask for the permission; a grant is only seen after a relaunch (macOS caches the answer).
    func requestAccessibility() {
        _ = accessibilityTrusted(prompt: true)
    }

    func relaunch() {
        let config = NSWorkspace.OpenConfiguration()
        config.createsNewApplicationInstance = true
        NSWorkspace.shared.openApplication(at: Bundle.main.bundleURL, configuration: config) { _, _ in
            DispatchQueue.main.async { NSApp.terminate(nil) }
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        MainActor.assumeIsolated {
            Core.shared.launch()
            Onboarding.showIfNeeded()
        }
    }

    /// Opening the app again (Spotlight, Finder) shows Settings, since the menu bar icon can be hidden.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        MainActor.assumeIsolated { SettingsWindowController.shared.show() }
        return false
    }

    func applicationWillTerminate(_ notification: Notification) {
        stop()
    }
}

@main
struct NexTypeApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @AppStorage("enabled") private var enabled = true
    @ObservedObject private var core = Core.shared
    @ObservedObject private var download = modelDownload

    var body: some Scene {
        MenuBarExtra("NexType", systemImage: enabled ? "text.cursor" : "pause.circle") {
            if core.status == .running {
                Text("\(core.today.completions) completions · \(core.today.words) words today")
            } else {
                Text(core.status.text)
            }
            Toggle("Completions", isOn: $enabled)
                .onChange(of: enabled) { _, _ in core.apply() }
            if let until = core.pausedUntil {
                Button("Resume (paused until \(until.formatted(date: .omitted, time: .shortened)))") { core.resume() }
            } else if enabled {
                Button("Pause for 1 Hour") { core.pause(minutes: 60) }
            }
            if let id = core.frontApp?.bundleIdentifier {
                let name = core.frontApp?.localizedName ?? id
                let a = core.setting(id)
                Divider()
                Button(a.completionsOff ? "Turn On in \(name)" : "Turn Off in \(name)") {
                    core.update(id) { $0.completionsOff.toggle() }
                }
                Button(a.tabOff ? "Turn On Tab Shortcut in \(name)" : "Turn Off Tab Shortcut in \(name)") {
                    core.update(id) { $0.tabOff.toggle() }
                }
            }
            if !core.domain.isEmpty {
                let key = "domain:" + core.domain
                Button(core.setting(key).completionsOff ? "Turn On on \(core.domain)" : "Turn Off on \(core.domain)") {
                    core.update(key) { $0.completionsOff.toggle() }
                }
            }
            if core.status == .modelMissing {
                if let line = downloadLine(download.state) { Text(line) }
                if !download.isBusy { Button("Download model (2.8 GB)") { download.start() } }
                if case .running = download.state { Button("Cancel download") { download.cancel() } }
            }
            if core.status == .needsAccessibility {
                Button("Grant Accessibility…") { core.requestAccessibility() }
                Button("Relaunch NexType") { core.relaunch() }
            }
            Divider()
            Button("Settings…") { SettingsWindowController.shared.show() }
                .keyboardShortcut(",")
            Button("Quit NexType") { NSApp.terminate(nil) }
                .keyboardShortcut("q")
        }
    }
}
