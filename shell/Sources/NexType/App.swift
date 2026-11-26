import AppKit
import Sparkle
import SwiftUI

/// Auto-update: checks the feed in Info.plist (SUFeedURL) about once a day, then downloads,
/// verifies (SUPublicEDKey) and installs on request. Started at launch.
let updater = SPUStandardUpdaterController(startingUpdater: true, updaterDelegate: nil, userDriverDelegate: nil)

/// A model NexType can run: a pinned GGUF file on Hugging Face (spec §16).
struct ModelInfo: Identifiable, Equatable {
    let name: String
    let repo: String
    let commit: String
    /// File name in the repo, also the local file name.
    let id: String
    let sha256: String
    let size: Int64
    /// Runs best alongside the user's apps on Macs with more memory (GB) than this. The whole
    /// file stays in memory while NexType runs, so it should take about a sixth of it at most.
    let bestAboveGB: Int

    /// Dev: a symlink to the checkout's `models/` file works too.
    var url: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("NexType/models/\(id)")
    }
    var sizeText: String { ByteCountFormatter.string(fromByteCount: size, countStyle: .file) }
    var installed: Bool { FileManager.default.fileExists(atPath: url.path) }
    var fitsThisMac: Bool { macMemoryGB > bestAboveGB }
}

/// Best first. The engine reads the chat format from the file, so only these families work:
/// Gemma 4, Gemma 3, Qwen 3.
let models = [
    ModelInfo(name: "Gemma 4 E4B", repo: "ggml-org/gemma-4-E4B-it-GGUF", commit: "8903cb408c7df39cde2ed15fac397e6d7be92951",
              id: "gemma-4-E4B-it-Q4_0.gguf", sha256: "a555b900214b477d8880e7832e0b8925e139b0159640036b09fe472b6f2097f2",
              size: 4_590_807_392, bestAboveGB: 16),
    ModelInfo(name: "Gemma 4 E2B", repo: "ggml-org/gemma-4-E2B-it-GGUF", commit: "2a8538c2e2d4687953a86c422c7c623cfd022880",
              id: "gemma-4-E2B-it-Q4_0.gguf", sha256: "8e30dff3ac4c8434c49a7036fa15564bdbb6044e42bf04550bf1a096ad7e6a52",
              size: 2_841_481_184, bestAboveGB: 8),
    ModelInfo(name: "Gemma 3 4B", repo: "ggml-org/gemma-3-4b-it-GGUF", commit: "d0976223747697cb51e056d85c532013931fe52e",
              id: "gemma-3-4b-it-Q4_K_M.gguf", sha256: "882e8d2db44dc554fb0ea5077cb7e4bc49e7342a1f0da57901c0802ea21a0863",
              size: 2_489_757_856, bestAboveGB: 8),
    ModelInfo(name: "Qwen 3 1.7B", repo: "unsloth/Qwen3-1.7B-GGUF", commit: "d7f544eead698dbd1f15126ef60b45a1e1933222",
              id: "Qwen3-1.7B-Q4_K_M.gguf", sha256: "b139949c5bd74937ad8ed8c8cf3d9ffb1e99c866c823204dc42c0d91fa181897",
              size: 1_107_409_472, bestAboveGB: 4),
    ModelInfo(name: "Gemma 3 1B", repo: "ggml-org/gemma-3-1b-it-GGUF", commit: "f9c28bcd85737ffc5aef028638d3341d49869c27",
              id: "gemma-3-1b-it-Q4_K_M.gguf", sha256: "8ccc5cd1f1b3602548715ae25a66ed73fd5dc68a210412eea643eb20eb75a135",
              size: 806_058_240, bestAboveGB: 0),
]

let macMemoryGB = Int(ProcessInfo.processInfo.physicalMemory >> 30)

/// The best model this Mac's memory allows. shortcut: memory only, not chip speed; an M1 with
/// 16 GB may find E4B slow, so benchmark first-token time if users report lag.
let recommendedModel = models.first { $0.fitsThisMac } ?? models[models.count - 1]

/// One download per model, so each keeps its own progress.
let downloads: [String: Download] = Dictionary(uniqueKeysWithValues: models.map { m in
    let d = Download(url: URL(string: "https://huggingface.co/\(m.repo)/resolve/\(m.commit)/\(m.id)")!,
                     dest: m.url, sha256: m.sha256, size: m.size)
    d.onDone = { MainActor.assumeIsolated { Core.shared.downloaded(m) } }
    return (m.id, d)
})

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
    /// The model to run. Existing installs keep E2B, the only model earlier builds had.
    @Published private(set) var model: ModelInfo = models.first { $0.id == UserDefaults.standard.string(forKey: "model") }
        ?? (models[1].installed ? models[1] : recommendedModel)
    private var statsTimer: Timer?

    private init() {
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
        guard model.installed else { return status = .modelMissing }
        status = start(modelPath: model.url.path) ? .running : .needsAccessibility
    }

    var download: Download { downloads[model.id]! }

    /// Use `m` from now on. Not on disk yet: download it, and keep the current one running meanwhile.
    func select(_ m: ModelInfo) {
        model = m
        UserDefaults.standard.set(m.id, forKey: "model")
        guard m.installed else { return downloads[m.id]!.start() }
        setModel(modelPath: m.url.path)  // no-op until started; start is a no-op after
        launch()
    }

    func downloaded(_ m: ModelInfo) {
        if m == model { select(m) } else { objectWillChange.send() }
    }

    /// Frees disk space; the model in use can't be deleted.
    func delete(_ m: ModelInfo) {
        guard m != model else { return }
        try? FileManager.default.removeItem(at: m.url)
        objectWillChange.send()
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
            _ = updater
            Core.shared.launch()
            Onboarding.showIfNeeded()
            if UserDefaults.standard.string(forKey: "settingsPane") != nil { SettingsWindowController.shared.show() }
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
            if core.status == .modelMissing { MenuDownload(model: core.model, download: core.download) }
            if core.status == .needsAccessibility {
                Button("Grant Accessibility…") { core.requestAccessibility() }
                Button("Relaunch NexType") { core.relaunch() }
            }
            Divider()
            Button("Check for Updates…") { updater.checkForUpdates(nil) }
            Button("Settings…") { SettingsWindowController.shared.show() }
                .keyboardShortcut(",")
            Button("Quit NexType") { NSApp.terminate(nil) }
                .keyboardShortcut("q")
        }
    }
}

/// Menu items for the selected model's download.
private struct MenuDownload: View {
    let model: ModelInfo
    @ObservedObject var download: Download

    var body: some View {
        if let line = downloadLine(download.state) { Text(line) }
        if !download.isBusy { Button("Download \(model.name) (\(model.sizeText))") { download.start() } }
        if case .running = download.state { Button("Cancel download") { download.cancel() } }
    }
}
