import ServiceManagement
import SwiftUI
import UniformTypeIdentifiers

struct SettingsView: View {
    var body: some View {
        TabView {
            GeneralSettings().tabItem { Label("General", systemImage: "gearshape") }
            AppSettings().tabItem { Label("Apps", systemImage: "square.grid.2x2") }
        }
        .frame(width: 520)
    }
}

private struct GeneralSettings: View {
    @AppStorage("length") private var length = 1
    @AppStorage("instructions") private var instructions = ""
    @AppStorage("clipboardContext") private var clipboardContext = false
    @AppStorage("screenContext") private var screenContext = false
    @State private var screenAllowed = CGPreflightScreenCaptureAccess()
    @AppStorage("history") private var history = false
    @State private var saved: UInt64 = 0
    @State private var confirmDelete = false
    @ObservedObject private var core = Core.shared
    @ObservedObject private var download = modelDownload
    @State private var atLogin = SMAppService.mainApp.status == .enabled
    @State private var loginError: String?

    var body: some View {
        Form {
            Toggle("Open at login", isOn: $atLogin)
                .onChange(of: atLogin) { _, on in
                    do {
                        try on ? SMAppService.mainApp.register() : SMAppService.mainApp.unregister()
                        loginError = nil
                    } catch {
                        loginError = error.localizedDescription
                        atLogin = SMAppService.mainApp.status == .enabled
                    }
                }
            if let loginError { Text(loginError).foregroundStyle(.red) }
            LabeledContent("Accept") {
                Text("Tab: next word · key above Tab: whole suggestion · Esc: dismiss")
                    .foregroundStyle(.secondary)
            }

            Picker("Suggestion length", selection: $length) {
                Text("Short").tag(0)
                Text("Medium").tag(1)
                Text("Long").tag(2)
            }
            .pickerStyle(.segmented)
            .onChange(of: length) { _, v in setLength(length: Core.length(v)) }

            VStack(alignment: .leading, spacing: 4) {
                Text("Custom instructions")
                TextEditor(text: $instructions)
                    .font(.body)
                    .frame(minHeight: 120)
                    .border(.quaternary)
                    .onChange(of: instructions) { _, v in setInstructions(text: v) }
                Text("Added to every prompt, e.g. “Write in a formal tone.”")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Toggle(isOn: $clipboardContext) {
                Text("Use copied text")
                Text("Text you copied in the last 2 minutes helps the suggestions. Passwords are never used.")
            }
            .onChange(of: clipboardContext) { _, on in setClipboardContext(on: on) }

            Toggle(isOn: $screenContext) {
                Text("Use text on screen")
                Text("Reads the window above where you type, such as the email you are replying to. Stays on your Mac.")
            }
            .onChange(of: screenContext) { _, on in
                setScreenContext(on: on)
                if on && !CGPreflightScreenCaptureAccess() { screenAllowed = CGRequestScreenCaptureAccess() }
            }
            if screenContext && !screenAllowed {
                Text("Allow NexType in System Settings → Privacy & Security → Screen & System Audio Recording, then relaunch NexType.")
                    .font(.caption)
                    .foregroundStyle(.red)
            }

            Toggle(isOn: $history) {
                Text("Learn from my writing")
                Text("Saves what you write, encrypted on this Mac, and shows the AI a few earlier texts so suggestions match your style. Never in password fields.")
            }
            .onChange(of: history) { _, on in setHistory(on: on) }
            LabeledContent("Saved texts") {
                HStack {
                    Text("\(saved)")
                    Button("Delete All…") { confirmDelete = true }.disabled(saved == 0)
                }
            }
            .onAppear { saved = historyCount() }
            .confirmationDialog("Delete all saved writing?", isPresented: $confirmDelete) {
                Button("Delete All", role: .destructive) {
                    deleteHistory()
                    saved = historyCount()
                }
            } message: {
                Text("This can't be undone.")
            }

            LabeledContent("Status", value: core.status.text)
            LabeledContent("Model", value: modelURL.path)
                .textSelection(.enabled)
            if core.status == .modelMissing {
                if case .running(let fraction) = download.state {
                    ProgressView(value: fraction) { Text(downloadLine(download.state) ?? "") }
                } else if let line = downloadLine(download.state) {
                    Text(line).foregroundStyle(download.state == .verifying ? Color.secondary : Color.red)
                }
                if !download.isBusy { Button("Download model (2.8 GB)") { download.start() } }
                if case .running = download.state { Button("Cancel") { download.cancel() } }
            }
        }
        .formStyle(.grouped)
        .padding()
    }
}

/// Per-app overrides (spec §14.2 subset).
private struct AppSettings: View {
    @ObservedObject private var core = Core.shared
    @State private var newSite = ""

    var body: some View {
        Form {
            if core.apps.isEmpty {
                Text("No app overrides. Add an app to turn completions or the Tab shortcut off in it, or to give it its own instructions.")
                    .foregroundStyle(.secondary)
            }
            ForEach($core.apps) { $app in
                Section(appName(app.bundleID)) {
                    Toggle("Completions", isOn: Binding(get: { !app.completionsOff }, set: { app.completionsOff = !$0 }))
                    if !app.bundleID.hasPrefix("domain:") {
                        Toggle("Tab accepts the next word", isOn: Binding(get: { !app.tabOff }, set: { app.tabOff = !$0 }))
                    }
                    TextField("Instructions", text: $app.instructions, prompt: Text("Instructions for this app or site"), axis: .vertical)
                        .labelsHidden()
                        .lineLimit(1...4)
                    Button("Remove Overrides", role: .destructive) {
                        core.apps.removeAll { $0.bundleID == app.bundleID }
                    }
                }
            }
            Button("Add App…") { pickApp() }
            HStack {
                TextField("Website", text: $newSite, prompt: Text("Website, e.g. mail.google.com"))
                    .labelsHidden()
                    .textFieldStyle(.roundedBorder)
                    .onSubmit(addSite)
                Button("Add Website", action: addSite).disabled(siteKey == nil)
            }
        }
        .formStyle(.grouped)
        .padding()
        .frame(minHeight: 360)
    }

    /// "https://Mail.Google.com/x" or "mail.google.com" → "domain:mail.google.com".
    private var siteKey: String? {
        var s = newSite.trimmingCharacters(in: .whitespaces).lowercased()
        if let host = URL(string: s)?.host { s = host } else if let i = s.firstIndex(of: "/") { s = String(s[..<i]) }
        return s.contains(".") && !s.contains(" ") ? "domain:" + s : nil
    }

    private func addSite() {
        guard let key = siteKey else { return }
        if !core.apps.contains(where: { $0.bundleID == key }) { core.apps.append(AppSetting(bundleID: key)) }
        newSite = ""
    }

    private func pickApp() {
        let panel = NSOpenPanel()
        panel.directoryURL = URL(fileURLWithPath: "/Applications")
        panel.allowedContentTypes = [.application]
        panel.allowsMultipleSelection = true
        guard panel.runModal() == .OK else { return }
        for url in panel.urls {
            if let id = Bundle(url: url)?.bundleIdentifier, !core.apps.contains(where: { $0.bundleID == id }) {
                core.apps.append(core.setting(id))
            }
        }
    }
}
