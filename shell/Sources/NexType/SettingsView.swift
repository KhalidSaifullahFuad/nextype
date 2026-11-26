import AppKit
import ServiceManagement
import SwiftUI
import UniformTypeIdentifiers

// Look and feel matches the Launchpad project's settings window (orange accent, custom sidebar, cards).

@MainActor
final class SettingsWindowController {
    static let shared = SettingsWindowController()
    private var window: NSWindow?

    func show() {
        if window == nil {
            let host = NSHostingController(rootView: SettingsView())
            host.sceneBridgingOptions = [.toolbars, .title]
            let w = SettingsWindow(contentViewController: host)
            w.styleMask = [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView]
            w.toolbarStyle = .unified
            w.title = "NexType Settings"
            w.isReleasedWhenClosed = false
            w.setContentSize(NSSize(width: 820, height: 660))
            w.center()
            window = w
        }
        NSApp.activate(ignoringOtherApps: true)
        window?.makeKeyAndOrderFront(nil)
    }
}

/// Keeps the sidebar search field from taking focus by itself (on open, on switching back to the
/// window); a click or key press in this window still focuses it.
private final class SettingsWindow: NSWindow {
    override func makeFirstResponder(_ responder: NSResponder?) -> Bool {
        if responder is NSSearchField, NSApp.currentEvent?.window !== self { return false }
        return super.makeFirstResponder(responder)
    }
}

// MARK: Navigation

enum SettingsPane: String, CaseIterable, Identifiable {
    case general, suggestions, keyboard
    case sources, learning
    case apps
    case permissions, model, about
    var id: String { rawValue }

    var title: String {
        switch self {
        case .general: "General"
        case .suggestions: "Suggestions"
        case .keyboard: "Keyboard"
        case .sources: "Context sources"
        case .learning: "Learn my style"
        case .apps: "Apps & websites"
        case .permissions: "Permissions"
        case .model: "Model"
        case .about: "About"
        }
    }

    var symbol: String {
        switch self {
        case .general: "gearshape"
        case .suggestions: "text.cursor"
        case .keyboard: "keyboard"
        case .sources: "doc.on.clipboard"
        case .learning: "brain"
        case .apps: "square.grid.2x2"
        case .permissions: "lock.shield"
        case .model: "cpu"
        case .about: "info.circle"
        }
    }

    /// Extra words the sidebar search matches, so a query finds the setting and not only the page title.
    var keywords: String {
        switch self {
        case .general: "login startup pause enable completions today stats status"
        case .suggestions: "length short medium long instructions tone prompt"
        case .keyboard: "tab accept dismiss esc shortcut key"
        case .sources: "clipboard copied screen window context"
        case .learning: "history writing style saved delete encrypted"
        case .apps: "override exclude disable website domain browser"
        case .permissions: "accessibility screen recording privacy"
        case .model: "download gemma status location"
        case .about: "version"
        }
    }

    func matches(_ query: String) -> Bool {
        let q = query.trimmingCharacters(in: .whitespaces)
        return q.isEmpty || title.localizedCaseInsensitiveContains(q) || keywords.localizedCaseInsensitiveContains(q)
    }

    static let groups: [(title: String, panes: [SettingsPane])] = [
        ("Essentials", [.general, .suggestions, .keyboard]),
        ("Context", [.sources, .learning]),
        ("Apps", [.apps]),
        ("App", [.permissions, .model, .about]),
    ]
}

/// Current pane plus browser-style back/forward history.
@MainActor
final class SettingsNavigator: ObservableObject {
    @Published private(set) var current: SettingsPane = .general
    @Published private var backStack: [SettingsPane] = []
    @Published private var forwardStack: [SettingsPane] = []

    var canGoBack: Bool { !backStack.isEmpty }
    var canGoForward: Bool { !forwardStack.isEmpty }

    func go(_ pane: SettingsPane) {
        guard pane != current else { return }
        backStack.append(current)
        forwardStack.removeAll()
        current = pane
    }

    func back() {
        guard let pane = backStack.popLast() else { return }
        forwardStack.append(current)
        current = pane
    }

    func forward() {
        guard let pane = forwardStack.popLast() else { return }
        backStack.append(current)
        current = pane
    }
}

struct SettingsView: View {
    @StateObject private var nav = SettingsNavigator()
    @State private var query = ""
    @State private var columns = NavigationSplitViewVisibility.all

    var body: some View {
        NavigationSplitView(columnVisibility: $columns) {
            SettingsSidebar(nav: nav, query: query)
                .navigationSplitViewColumnWidth(min: 210, ideal: 230, max: 300)
        } detail: {
            detail.id(nav.current)
        }
        .searchable(text: $query, placement: .sidebar, prompt: "Search settings")
        .navigationTitle("NexType Settings")
        .toolbar {
            ToolbarItemGroup(placement: .navigation) {
                Button {
                    withAnimation { columns = columns == .detailOnly ? .all : .detailOnly }
                } label: { Image(systemName: "sidebar.left") }
                    .help("Show or hide the sidebar")
            }
            ToolbarItemGroup(placement: .navigation) {
                Button { nav.back() } label: { Image(systemName: "chevron.left") }
                    .disabled(!nav.canGoBack).help("Back")
                Button { nav.forward() } label: { Image(systemName: "chevron.right") }
                    .disabled(!nav.canGoForward).help("Forward")
            }
        }
        .frame(minWidth: 720, minHeight: 500)
    }

    @ViewBuilder private var detail: some View {
        switch nav.current {
        case .general: GeneralPane()
        case .suggestions: SuggestionsPane()
        case .keyboard: KeyboardPane()
        case .sources: SourcesPane()
        case .learning: LearningPane()
        case .apps: AppsPane()
        case .permissions: PermissionsPane()
        case .model: ModelPane()
        case .about: AboutPane()
        }
    }
}

// MARK: Sidebar

struct SettingsSidebar: View {
    @ObservedObject var nav: SettingsNavigator
    let query: String
    @State private var collapsed: Set<String> = []

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(SettingsPane.groups, id: \.title) { group in
                    let panes = group.panes.filter { $0.matches(query) }
                    if !panes.isEmpty {
                        groupHeader(group.title)
                        if !collapsed.contains(group.title) || !query.isEmpty {
                            ForEach(panes) { pane in
                                SidebarRow(pane: pane, selected: nav.current == pane) { nav.go(pane) }
                            }
                        }
                    }
                }
                if !SettingsPane.allCases.contains(where: { $0.matches(query) }) {
                    Text("No results")
                        .font(.system(size: 13))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 8)
                        .padding(.top, 14)
                }
            }
            .padding(.horizontal, 10)
            .padding(.bottom, 12)
        }
    }

    private func groupHeader(_ title: String) -> some View {
        let isCollapsed = collapsed.contains(title) && query.isEmpty
        return Button {
            withAnimation(.easeInOut(duration: 0.15)) {
                if collapsed.contains(title) { collapsed.remove(title) } else { collapsed.insert(title) }
            }
        } label: {
            HStack(spacing: 6) {
                Image(systemName: "chevron.down")
                    .font(.system(size: 9, weight: .bold))
                    .rotationEffect(.degrees(isCollapsed ? -90 : 0))
                Text(title).font(.system(size: 11, weight: .semibold))
                Spacer(minLength: 0)
            }
            .foregroundStyle(.secondary)
            .padding(.horizontal, 8)
            .padding(.top, 14)
            .padding(.bottom, 4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

struct SidebarRow: View {
    let pane: SettingsPane
    let selected: Bool
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Image(systemName: pane.symbol)
                    .font(.system(size: 13))
                    .foregroundStyle(.orange)
                    .frame(width: 20)
                Text(pane.title).font(.system(size: 13))
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 8)
            .frame(height: 30)
            .background(RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(Color.primary.opacity(selected ? 0.1 : hovering ? 0.05 : 0)))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }
}

// MARK: Building blocks

/// Scrolling page with a large title above a stack of cards.
struct SettingsPage<Content: View>: View {
    let title: String
    @ViewBuilder var content: Content

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Text(title).font(.system(size: 22, weight: .bold))
                content
            }
            .padding(.horizontal, 24)
            .padding(.vertical, 20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// Rounded group of rows, optionally headed by a title.
struct SettingsCard<Content: View>: View {
    var title: String?
    @ViewBuilder var content: Content

    init(_ title: String? = nil, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let title {
                Text(title).font(.system(size: 14, weight: .semibold)).padding(.top, 12).padding(.bottom, 8)
            }
            content
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 16, style: .continuous).fill(Color.primary.opacity(0.045)))
    }
}

struct IconTile: View {
    let symbol: String

    var body: some View {
        Image(systemName: symbol)
            .font(.system(size: 14, weight: .medium))
            .foregroundStyle(.orange)
            .frame(width: 30, height: 30)
            .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(Color.orange.opacity(0.15)))
    }
}

/// Icon tile, title and subtitle on the left, a control on the right. `indented` lines a row up
/// under the text of the row above it, for options that belong to that row.
struct SettingsRow<Trailing: View>: View {
    var symbol: String?
    let title: String
    var subtitle: String?
    var indented = false
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(spacing: 12) {
            if let symbol { IconTile(symbol: symbol) }
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.system(size: 13))
                if let subtitle {
                    Text(subtitle).font(.system(size: 11)).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 16)
            trailing
        }
        .padding(.leading, indented ? 42 : 0)
        .padding(.vertical, indented ? 6 : 10)
    }
}

extension SettingsRow where Trailing == EmptyView {
    init(symbol: String? = nil, title: String, subtitle: String? = nil, indented: Bool = false) {
        self.init(symbol: symbol, title: title, subtitle: subtitle, indented: indented) { EmptyView() }
    }
}

struct ToggleRow: View {
    var symbol: String?
    let title: String
    var subtitle: String?
    var indented = false
    @Binding var isOn: Bool

    var body: some View {
        SettingsRow(symbol: symbol, title: title, subtitle: subtitle, indented: indented) {
            Toggle(title, isOn: $isOn).toggleStyle(.switch).labelsHidden().tint(.orange)
        }
    }
}

struct RowDivider: View {
    var body: some View { Divider().opacity(0.7) }
}

struct Note: View {
    let text: String
    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text).font(.system(size: 11)).foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
            .padding(.vertical, 6)
    }
}

struct Warning: View {
    let text: String
    var color: Color = .orange
    init(_ text: String, color: Color = .orange) { self.text = text; self.color = color }

    var body: some View {
        Label(text, systemImage: "exclamationmark.triangle").font(.system(size: 11)).foregroundStyle(color)
            .fixedSize(horizontal: false, vertical: true)
            .padding(.leading, 42)
            .padding(.bottom, 8)
    }
}

private struct Granted: View {
    var text = "Granted"
    var body: some View { Label(text, systemImage: "checkmark.circle.fill").foregroundStyle(.green) }
}

private struct KeyCap: View {
    let key: String
    var body: some View {
        Text(key)
            .font(.system(size: 12, weight: .medium))
            .padding(.horizontal, 8)
            .frame(minWidth: 28, minHeight: 22)
            .background(RoundedRectangle(cornerRadius: 6, style: .continuous).fill(Color.primary.opacity(0.08)))
            .overlay(RoundedRectangle(cornerRadius: 6, style: .continuous).strokeBorder(Color.primary.opacity(0.15)))
    }
}

// MARK: Suggestion length picker

enum SuggestionLength: Int, CaseIterable, Identifiable {
    case short, medium, long
    var id: Int { rawValue }

    var title: String {
        switch self {
        case .short: "Short"
        case .medium: "Medium"
        case .long: "Long"
        }
    }

    var subtitle: String {
        switch self {
        case .short: "Up to about 6 words"
        case .medium: "Up to about 10 words"
        case .long: "Up to about 20 words"
        }
    }

    var sample: String {
        switch self {
        case .short: "the update"
        case .medium: "the update, I'll take a look"
        case .long: "the update, I'll take a look today and get back to you."
        }
    }
}

/// Row of large selectable previews, like System Settings' appearance picker.
private struct LengthPicker: View {
    @Binding var selection: SuggestionLength

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            ForEach(SuggestionLength.allCases) { option in
                let on = option == selection
                Button { selection = option } label: {
                    VStack(spacing: 8) {
                        LengthThumbnail(length: option)
                            .frame(height: 84)
                            .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous)
                                .strokeBorder(on ? Color.orange : Color.primary.opacity(0.12), lineWidth: on ? 2.5 : 1))
                        Text(option.title).font(.system(size: 12, weight: .semibold))
                            .foregroundStyle(on ? Color.orange : Color.primary)
                        Text(option.subtitle).font(.system(size: 11)).foregroundStyle(.secondary)
                            .multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true)
                    }
                    .padding(8)
                    .frame(maxWidth: .infinity)
                    .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Color.orange.opacity(on ? 0.12 : 0)))
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.vertical, 4)
    }
}

/// Dusk wallpaper with a small window holding typed text and grey ghost text.
private struct LengthThumbnail: View {
    let length: SuggestionLength

    var body: some View {
        ZStack {
            LinearGradient(colors: [Color(red: 0.16, green: 0.24, blue: 0.48), Color(red: 0.3, green: 0.46, blue: 0.7)],
                           startPoint: .top, endPoint: .bottom)
            Ellipse().fill(Color(red: 0.95, green: 0.62, blue: 0.42).opacity(0.55))
                .frame(maxWidth: 180, maxHeight: 60).offset(y: 46).blur(radius: 18)
            VStack(alignment: .leading, spacing: 5) {
                HStack(spacing: 3) {
                    Circle().fill(.red).frame(width: 4, height: 4)
                    Circle().fill(.yellow).frame(width: 4, height: 4)
                    Circle().fill(.green).frame(width: 4, height: 4)
                }
                (Text("Thanks for ").foregroundColor(.white) + Text(length.sample).foregroundColor(.white.opacity(0.4)))
                    .font(.system(size: 9))
                    .lineLimit(3)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
            }
            .padding(7)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(RoundedRectangle(cornerRadius: 7, style: .continuous).fill(Color(white: 0.12).opacity(0.78)))
            .overlay(RoundedRectangle(cornerRadius: 7, style: .continuous).strokeBorder(.white.opacity(0.18), lineWidth: 0.5))
            .shadow(color: .black.opacity(0.35), radius: 6, y: 3)
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
        }
    }
}

// MARK: Essentials

struct GeneralPane: View {
    @AppStorage("enabled") private var enabled = true
    @ObservedObject private var core = Core.shared
    @State private var startAtLogin = SMAppService.mainApp.status == .enabled
    @State private var loginError: String?

    var body: some View {
        SettingsPage(title: "General") {
            SettingsCard {
                ToggleRow(symbol: "power", title: "Start at login", subtitle: "Opens by itself every time you log in to your Mac.", isOn: $startAtLogin)
                    .onChange(of: startAtLogin) { _, on in
                        do {
                            try on ? SMAppService.mainApp.register() : SMAppService.mainApp.unregister()
                            loginError = nil
                        } catch {
                            loginError = error.localizedDescription
                            startAtLogin = SMAppService.mainApp.status == .enabled
                        }
                    }
                if let loginError { Warning(loginError, color: .red) }
                RowDivider()
                ToggleRow(symbol: "text.cursor", title: "Completions", subtitle: "Show suggestions as you type in any app.", isOn: $enabled)
                    .onChange(of: enabled) { _, _ in core.apply() }
                RowDivider()
                SettingsRow(symbol: "pause.circle", title: "Pause",
                            subtitle: core.pausedUntil.map { "Paused until \($0.formatted(date: .omitted, time: .shortened))." }
                                ?? "Stop suggestions for an hour.") {
                    if core.pausedUntil != nil {
                        Button("Resume") { core.resume() }
                    } else {
                        Button("Pause 1 Hour") { core.pause(minutes: 60) }.disabled(!enabled)
                    }
                }
            }
            SettingsCard("Today") {
                SettingsRow(symbol: "checkmark.circle", title: "Completions accepted") {
                    Text("\(core.today.completions)").font(.system(size: 13, weight: .semibold)).monospacedDigit()
                }
                RowDivider()
                SettingsRow(symbol: "text.word.spacing", title: "Words written for you") {
                    Text("\(core.today.words)").font(.system(size: 13, weight: .semibold)).monospacedDigit()
                }
            }
            SettingsCard {
                SettingsRow(symbol: "circle.dotted", title: "Status") {
                    if core.status == .running { Granted(text: core.status.text) } else {
                        Label(core.status.text, systemImage: "exclamationmark.triangle").foregroundStyle(.orange)
                    }
                }
            }
        }
    }
}

struct SuggestionsPane: View {
    @AppStorage("length") private var length = 1
    @AppStorage("instructions") private var instructions = ""

    var body: some View {
        SettingsPage(title: "Suggestions") {
            SettingsCard("Length") {
                LengthPicker(selection: Binding(
                    get: { SuggestionLength(rawValue: length) ?? .medium },
                    set: { length = $0.rawValue }))
                    .onChange(of: length) { _, v in setLength(length: Core.length(v)) }
                    .padding(.bottom, 8)
            }
            SettingsCard("Custom instructions") {
                TextEditor(text: $instructions)
                    .font(.system(size: 13))
                    .scrollContentBackground(.hidden)
                    .padding(6)
                    .frame(minHeight: 100)
                    .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(Color.primary.opacity(0.06)))
                    .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(Color.primary.opacity(0.12)))
                    .onChange(of: instructions) { _, v in setInstructions(text: v) }
                Note("Added to every prompt, e.g. “Write in a formal tone.”")
                    .padding(.bottom, 6)
            }
        }
    }
}

struct KeyboardPane: View {
    var body: some View {
        SettingsPage(title: "Keyboard") {
            SettingsCard {
                SettingsRow(symbol: "arrow.right.to.line", title: "Accept next word", subtitle: "Takes one word of the suggestion.") {
                    KeyCap(key: "⇥ Tab")
                }
                RowDivider()
                SettingsRow(symbol: "text.append", title: "Accept whole suggestion", subtitle: "The key above Tab (` or §, depending on your keyboard).") {
                    KeyCap(key: "`")
                }
                RowDivider()
                SettingsRow(symbol: "xmark.circle", title: "Dismiss", subtitle: "Hides the suggestion until you type again.") {
                    KeyCap(key: "esc")
                }
            }
            Note("Turn the Tab shortcut off for a single app in Apps & websites.")
        }
    }
}

// MARK: Context

struct SourcesPane: View {
    @AppStorage("clipboardContext") private var clipboardContext = false
    @AppStorage("screenContext") private var screenContext = false
    @State private var screenAllowed = CGPreflightScreenCaptureAccess()

    var body: some View {
        SettingsPage(title: "Context sources") {
            SettingsCard {
                ToggleRow(symbol: "doc.on.clipboard", title: "Use copied text",
                          subtitle: "Text you copied in the last 2 minutes helps the suggestions. Passwords are never used.",
                          isOn: $clipboardContext)
                    .onChange(of: clipboardContext) { _, on in setClipboardContext(on: on) }
                RowDivider()
                ToggleRow(symbol: "macwindow", title: "Use text on screen",
                          subtitle: "Reads the window above where you type, such as the email you are replying to. Stays on your Mac.",
                          isOn: $screenContext)
                    .onChange(of: screenContext) { _, on in
                        setScreenContext(on: on)
                        if on && !CGPreflightScreenCaptureAccess() { screenAllowed = CGRequestScreenCaptureAccess() }
                    }
                if screenContext && !screenAllowed {
                    Warning("Allow NexType in System Settings → Privacy & Security → Screen & System Audio Recording, then relaunch NexType.")
                }
            }
        }
    }
}

struct LearningPane: View {
    @AppStorage("history") private var history = false
    @State private var saved: UInt64 = 0
    @State private var confirmDelete = false

    var body: some View {
        SettingsPage(title: "Learn my style") {
            SettingsCard {
                ToggleRow(symbol: "brain", title: "Learn from my writing",
                          subtitle: "Saves what you write, encrypted on this Mac, and shows the AI a few earlier texts so suggestions match your style. Never in password fields.",
                          isOn: $history)
                    .onChange(of: history) { _, on in setHistory(on: on) }
                RowDivider()
                SettingsRow(symbol: "tray.full", title: "Saved texts", subtitle: "\(saved) saved on this Mac.") {
                    Button("Delete All…", role: .destructive) { confirmDelete = true }.disabled(saved == 0)
                }
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
    }
}

// MARK: Apps

/// Per-app overrides (spec §14.2 subset).
struct AppsPane: View {
    @ObservedObject private var core = Core.shared
    @State private var newSite = ""

    var body: some View {
        SettingsPage(title: "Apps & websites") {
            SettingsCard {
                SettingsRow(symbol: "plus.app", title: "Add an app", subtitle: "Turn completions or Tab off in it, or give it its own instructions.") {
                    Button("Add App…") { pickApp() }
                }
                RowDivider()
                SettingsRow(symbol: "globe", title: "Add a website", subtitle: "Safari, Chrome, Arc and other browsers.") {
                    HStack {
                        TextField("Website", text: $newSite, prompt: Text("mail.google.com"))
                            .labelsHidden()
                            .textFieldStyle(.roundedBorder)
                            .frame(width: 170)
                            .onSubmit(addSite)
                        Button("Add", action: addSite).disabled(siteKey == nil)
                    }
                }
            }
            ForEach($core.apps) { $app in
                SettingsCard {
                    HStack(spacing: 12) {
                        appIcon(app.bundleID)
                        Text(appName(app.bundleID)).font(.system(size: 13, weight: .semibold))
                        Spacer()
                        Button("Remove") { core.apps.removeAll { $0.bundleID == app.bundleID } }
                    }
                    .padding(.vertical, 10)
                    RowDivider()
                    ToggleRow(title: "Completions", indented: true,
                              isOn: Binding(get: { !app.completionsOff }, set: { app.completionsOff = !$0 }))
                    if !app.bundleID.hasPrefix("domain:") {
                        ToggleRow(title: "Tab accepts the next word", indented: true,
                                  isOn: Binding(get: { !app.tabOff }, set: { app.tabOff = !$0 }))
                    }
                    TextField("Instructions", text: $app.instructions, prompt: Text("Instructions for this app or site"), axis: .vertical)
                        .labelsHidden()
                        .textFieldStyle(.roundedBorder)
                        .lineLimit(1...4)
                        .padding(.leading, 42)
                        .padding(.vertical, 8)
                }
            }
        }
    }

    @ViewBuilder private func appIcon(_ id: String) -> some View {
        if let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: id) {
            Image(nsImage: NSWorkspace.shared.icon(forFile: url.path)).resizable().frame(width: 30, height: 30)
        } else {
            IconTile(symbol: id.hasPrefix("domain:") ? "globe" : "app.dashed")
        }
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

// MARK: App

struct PermissionsPane: View {
    @ObservedObject private var core = Core.shared
    private let axOK = accessibilityTrusted(prompt: false)
    private let screenOK = CGPreflightScreenCaptureAccess()

    var body: some View {
        SettingsPage(title: "Permissions") {
            SettingsCard {
                SettingsRow(symbol: "accessibility", title: "Accessibility", subtitle: "Needed to read the field you type in and show suggestions.") {
                    if axOK { Granted() } else { Button("Grant Permission") { core.requestAccessibility() } }
                }
                RowDivider()
                SettingsRow(symbol: "rectangle.dashed.badge.record", title: "Screen Recording", subtitle: "Needed only for “Use text on screen”.") {
                    if screenOK { Granted() } else {
                        Button("Grant Access") {
                            NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")!)
                        }
                    }
                }
                RowDivider()
                SettingsRow(symbol: "arrow.clockwise", title: "Relaunch NexType", subtitle: "macOS applies new permissions after a relaunch.") {
                    Button("Relaunch") { core.relaunch() }
                }
            }
        }
    }
}

struct ModelPane: View {
    @ObservedObject private var core = Core.shared
    @ObservedObject private var download = modelDownload

    var body: some View {
        SettingsPage(title: "Model") {
            SettingsCard {
                SettingsRow(symbol: "cpu", title: "Gemma 4 E2B (Q4_0)", subtitle: "Runs entirely on your Mac. 2.8 GB.") {
                    if core.status == .modelMissing {
                        Label("Not downloaded", systemImage: "exclamationmark.triangle").foregroundStyle(.orange)
                    } else {
                        Granted(text: "Installed")
                    }
                }
                RowDivider()
                SettingsRow(symbol: "folder", title: "Model file", subtitle: (modelURL.path as NSString).abbreviatingWithTildeInPath) {
                    Button("Show in Finder") { NSWorkspace.shared.activateFileViewerSelecting([modelURL]) }
                        .disabled(core.status == .modelMissing)
                }
                if core.status == .modelMissing {
                    RowDivider()
                    SettingsRow(symbol: "arrow.down.circle", title: "Download model",
                                subtitle: downloadLine(download.state) ?? "Downloads once from Hugging Face and is checked before use.") {
                        if case .running = download.state {
                            Button("Cancel") { download.cancel() }
                        } else if !download.isBusy {
                            Button("Download") { download.start() }
                        }
                    }
                    if case .running(let fraction) = download.state {
                        ProgressView(value: fraction).tint(.orange).padding(.leading, 42).padding(.bottom, 10)
                    }
                }
            }
        }
    }
}

struct AboutPane: View {
    var body: some View {
        SettingsPage(title: "About") {
            SettingsCard {
                VStack(spacing: 10) {
                    // shortcut: no app icon yet, so a large tile stands in; use NSApp.applicationIconImage once one ships.
                    Image(systemName: "text.cursor")
                        .font(.system(size: 40, weight: .medium))
                        .foregroundStyle(.orange)
                        .frame(width: 88, height: 88)
                        .background(RoundedRectangle(cornerRadius: 20, style: .continuous).fill(Color.orange.opacity(0.15)))
                    Text("NexType").font(.system(size: 22, weight: .semibold))
                    Text("Version \(Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev")")
                        .foregroundStyle(.secondary)
                    Text("AI autocomplete everywhere you type")
                        .multilineTextAlignment(.center)
                    Text("Private and on-device. Nothing you type leaves your Mac.")
                        .font(.system(size: 11)).foregroundStyle(.secondary).multilineTextAlignment(.center)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 20)
            }
        }
    }
}
