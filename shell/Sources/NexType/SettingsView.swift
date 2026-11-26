import AppKit
import Charts
import Combine
import ServiceManagement
import Sparkle
import SwiftUI
import UniformTypeIdentifiers

// Look and feel matches the Launchpad project's settings window (orange accent, custom sidebar, cards).

@MainActor
final class SettingsWindowController {
    static let shared = SettingsWindowController()
    private var window: NSWindow?
    private let nav = SettingsNavigator()
    private var titleSync: AnyCancellable?

    func show(_ pane: SettingsPane? = nil) {
        if let pane { nav.go(pane) }
        if window == nil {
            let host = NSHostingController(rootView: SettingsView(nav: nav))
            host.sceneBridgingOptions = [.toolbars, .title]
            let w = SettingsWindow(contentViewController: host)
            w.styleMask = [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView]
            w.toolbarStyle = .unified
            w.titleVisibility = .hidden
            // The pane's name is the window title, like System Settings.
            titleSync = nav.$current.sink { [weak w] in w?.title = $0.title }
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
    case setup
    case general, statistics, suggestions, keyboard
    case sources, learning
    case apps
    case model, about
    var id: String { rawValue }

    var title: String {
        switch self {
        case .setup: "Setup"
        case .general: "General"
        case .statistics: "Statistics"
        case .suggestions: "Suggestions"
        case .keyboard: "Keyboard"
        case .sources: "Context sources"
        case .learning: "Learn my style"
        case .apps: "Apps & websites"
        case .model: "Model"
        case .about: "About"
        }
    }

    var symbol: String {
        switch self {
        case .setup: "checklist"
        case .general: "gearshape"
        case .statistics: "chart.bar"
        case .suggestions: "text.cursor"
        case .keyboard: "keyboard"
        case .sources: "doc.on.clipboard"
        case .learning: "brain"
        case .apps: "square.grid.2x2"
        case .model: "cpu"
        case .about: "info.circle"
        }
    }

    /// Extra words the sidebar search matches, so a query finds the setting and not only the page title.
    var keywords: String {
        switch self {
        case .setup: "setup start accessibility permissions privacy download model screen recording relaunch ready status"
        case .general: "login startup pause enable completions"
        case .statistics: "stats today daily weekly monthly accepted words written history chart"
        case .suggestions: "length short medium long instructions tone prompt"
        case .keyboard: "tab accept dismiss esc shortcut key"
        case .sources: "clipboard copied screen window context"
        case .learning: "history writing style saved delete encrypted"
        case .apps: "override exclude disable website domain browser"
        case .model: "download gemma qwen recommended memory delete location"
        case .about: "version update check sparkle"
        }
    }

    func matches(_ query: String) -> Bool {
        let q = query.trimmingCharacters(in: .whitespaces)
        return q.isEmpty || title.localizedCaseInsensitiveContains(q) || keywords.localizedCaseInsensitiveContains(q)
    }

    static let groups: [(title: String, panes: [SettingsPane])] = [
        ("", [.setup]),
        ("Writing", [.general, .statistics, .suggestions, .keyboard, .apps]),
        ("Context", [.sources, .learning]),
        ("NexType", [.model, .about]),
    ]
}

/// Current pane plus browser-style back/forward history.
@MainActor
final class SettingsNavigator: ObservableObject {
    /// `open NexType.app --args -settingsPane apps` opens Settings on that pane.
    @Published private(set) var current = SettingsPane(rawValue: UserDefaults.standard.string(forKey: "settingsPane") ?? "") ?? .setup
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
    @ObservedObject var nav: SettingsNavigator
    @State private var query = ""

    var body: some View {
        NavigationSplitView(columnVisibility: .constant(.all)) {
            SettingsSidebar(nav: nav, query: query)
                .navigationSplitViewColumnWidth(min: 210, ideal: 230, max: 300)
        } detail: {
            detail.id(nav.current)
        }
        .searchable(text: $query, placement: .sidebar, prompt: "Search settings")
        .toolbar {
            ToolbarItemGroup(placement: .navigation) {
                Button { nav.back() } label: { Image(systemName: "chevron.left") }
                    .disabled(!nav.canGoBack).help("Back")
                Button { nav.forward() } label: { Image(systemName: "chevron.right") }
                    .disabled(!nav.canGoForward).help("Forward")
            }
            // Drawn here instead of the window title so it can be larger; the window title stays for the Dock.
            if #available(macOS 26, *) {
                ToolbarItem(placement: .navigation) { title }.sharedBackgroundVisibility(.hidden)
            } else {
                ToolbarItem(placement: .navigation) { title }
            }
        }
        .toolbar(removing: .sidebarToggle)  // the sidebar is the navigation; it always fits
        .frame(minWidth: 720, minHeight: 500)
    }

    private var title: some View {
        Text(nav.current.title).font(.system(size: 17, weight: .bold)).padding(.horizontal, 6)
    }

    @ViewBuilder private var detail: some View {
        switch nav.current {
        case .setup: SetupPane(nav: nav)
        case .general: GeneralPane()
        case .statistics: StatisticsPane()
        case .suggestions: SuggestionsPane()
        case .keyboard: KeyboardPane()
        case .sources: SourcesPane()
        case .learning: LearningPane()
        case .apps: AppsPane()
        case .model: ModelPane()
        case .about: AboutPane()
        }
    }
}

// MARK: Sidebar

struct SettingsSidebar: View {
    @ObservedObject var nav: SettingsNavigator
    let query: String

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(SettingsPane.groups, id: \.title) { group in
                    let panes = group.panes.filter { $0.matches(query) }
                    if !panes.isEmpty {
                        if !group.title.isEmpty { groupHeader(group.title) }
                        ForEach(panes) { pane in
                            SidebarRow(pane: pane, selected: nav.current == pane) { nav.go(pane) }
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
        Text(title)
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 8)
            .padding(.top, 14)
            .padding(.bottom, 4)
            .accessibilityAddTraits(.isHeader)
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

/// Scrolling stack of cards. The pane's title shows in the toolbar, like System Settings.
struct SettingsPage<Content: View>: View {
    let title: String
    @ViewBuilder var content: Content

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                content
            }
            .padding(.horizontal, 24)
            .padding(.vertical, 20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// Rounded group of rows, optionally headed by a title above it.
struct SettingsCard<Content: View>: View {
    var title: String?
    @ViewBuilder var content: Content

    init(_ title: String? = nil, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let title {
                Text(title).font(.system(size: 15, weight: .semibold)).padding(.leading, 4).padding(.top, 4)
            }
            VStack(alignment: .leading, spacing: 0) { content }
                .padding(.horizontal, 16)
                .padding(.vertical, 6)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 16, style: .continuous).fill(Color.primary.opacity(0.045)))
        }
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
            Toggle(title, isOn: $isOn).toggleStyle(.switch).controlSize(.small).labelsHidden().tint(.orange)
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

// MARK: Setup

/// Everything NexType needs before it can suggest, in order, each with its fix.
struct SetupPane: View {
    @ObservedObject var nav: SettingsNavigator
    @ObservedObject private var core = Core.shared
    @ObservedObject private var download: Download
    /// Permissions are granted in System Settings; re-read them when the user comes back.
    @State private var axOK = accessibilityTrusted(prompt: false)
    @State private var screenOK = CGPreflightScreenCaptureAccess()

    init(nav: SettingsNavigator) {
        self.nav = nav
        download = Core.shared.download
    }

    var body: some View {
        SettingsPage(title: "Setup") {
            SettingsCard {
                SettingsRow(symbol: core.status == .running ? "checkmark.seal" : "exclamationmark.triangle",
                            title: core.status == .running ? "NexType is ready" : "NexType isn't running yet",
                            subtitle: summary)
            }
            SettingsCard("Required") {
                SettingsRow(symbol: "accessibility", title: "Accessibility",
                            subtitle: "Lets NexType read the text before your cursor and show suggestions.") {
                    if !axOK {
                        Button("Grant Access…") { core.requestAccessibility() }
                    } else if core.status == .needsAccessibility {
                        Button("Relaunch") { core.relaunch() }
                    } else {
                        Granted()
                    }
                }
                if axOK && core.status == .needsAccessibility {
                    Warning("macOS applies the permission after NexType relaunches.")
                }
                RowDivider()
                SettingsRow(symbol: "cpu", title: core.model.name,
                            subtitle: downloadLine(download.state) ?? (core.model.installed
                                ? "\(core.model.sizeText) · Downloaded · Runs on your Mac"
                                : "\(core.model.sizeText) one-time download · Runs on your Mac")) {
                    HStack {
                        Button("Change…") { nav.go(.model) }
                        if core.model.installed {
                            Granted(text: "Ready")
                        } else if case .running = download.state {
                            Button("Cancel") { download.cancel() }
                        } else if !download.isBusy {
                            Button("Download") { core.select(core.model) }
                        }
                    }
                }
                if case .running(let fraction) = download.state {
                    ProgressView(value: fraction).tint(.orange).padding(.leading, 42).padding(.bottom, 8)
                }
            }
            SettingsCard("Optional") {
                SettingsRow(symbol: "rectangle.dashed.badge.record", title: "Screen Recording",
                            subtitle: "Lets suggestions use the text on screen. Turn it on in Context sources.") {
                    if screenOK { Granted() } else {
                        Button("Grant Access…") {
                            NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")!)
                        }
                    }
                }
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            axOK = accessibilityTrusted(prompt: false)
            screenOK = CGPreflightScreenCaptureAccess()
        }
    }

    private var summary: String {
        switch core.status {
        case .running: "Start typing in any app. Press Tab to accept a suggestion."
        case .needsAccessibility: "Allow Accessibility below."
        case .modelMissing: "Download a model below."
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
        }
    }
}

// MARK: Statistics

/// Accepted completions per day, week or month, like Screen Time's day/week switch.
struct StatisticsPane: View {
    enum Period: String, CaseIterable, Identifiable {
        case day = "Day", week = "Week", month = "Month"
        var id: Self { self }
        var unit: Calendar.Component { switch self { case .day: .day; case .week: .weekOfYear; case .month: .month } }
        /// Bars in the chart.
        var count: Int { switch self { case .day: 14; case .week: 12; case .month: 12 } }
        var current: String { switch self { case .day: "Today"; case .week: "This week"; case .month: "This month" } }
        var previous: String { switch self { case .day: "yesterday"; case .week: "last week"; case .month: "last month" } }
    }

    struct Bucket: Identifiable {
        let start: Date
        var completions = 0
        var words = 0
        var id: Date { start }
    }

    @ObservedObject private var core = Core.shared
    @AppStorage("statsPeriod") private var period = Period.day
    @State private var days: [DayStats] = []
    @State private var hovered: Date?

    var body: some View {
        let buckets = buckets(period)
        let now = buckets.last ?? Bucket(start: .now)
        let before = buckets.dropLast().last ?? Bucket(start: .now)
        let shown = hovered.flatMap { h in buckets.last { $0.start <= h } } ?? now
        SettingsPage(title: "Statistics") {
            Picker("Period", selection: $period) {
                ForEach(Period.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .frame(width: 240)
            .frame(maxWidth: .infinity)

            SettingsCard {
                // Screen Time style: period, one hero number, then the detail line.
                VStack(alignment: .leading, spacing: 2) {
                    Text(shown.start == now.start ? period.current : label(shown.start))
                        .font(.system(size: 13, weight: .medium)).foregroundStyle(.secondary)
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text(shown.completions.formatted()).font(.system(size: 30, weight: .semibold)).monospacedDigit()
                        Text(shown.completions == 1 ? "completion" : "completions")
                            .font(.system(size: 15, weight: .medium)).foregroundStyle(.secondary)
                    }
                    detail(shown, now: now, before: before)
                        .font(.system(size: 12)).foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.top, 14)

                Chart(buckets) { b in
                    BarMark(x: .value("Period", b.start, unit: period.unit),
                            y: .value("Completions", b.completions),
                            width: .ratio(0.6))
                        .clipShape(UnevenRoundedRectangle(topLeadingRadius: 4, topTrailingRadius: 4))
                        .foregroundStyle(Color.orange.opacity(hovered == nil || b.start == shown.start ? 1 : 0.35))
                }
                .chartXSelection(value: $hovered)
                .chartXAxis {
                    AxisMarks(values: .stride(by: period.unit, count: period == .day ? 2 : period == .week ? 2 : 1)) { _ in
                        AxisValueLabel(format: axisFormat, centered: true)
                    }
                }
                .chartYAxis {
                    AxisMarks(position: .trailing, values: .automatic(desiredCount: 3)) {
                        AxisGridLine().foregroundStyle(Color.primary.opacity(0.08))
                        AxisValueLabel()
                    }
                }
                .frame(height: 180)
                .padding(.vertical, 14)
                .accessibilityLabel("Completions accepted per \(period.rawValue.lowercased())")
            }

            SettingsCard("All time") {
                SettingsRow(symbol: "checkmark.circle", title: "Completions accepted") {
                    Text("\(days.reduce(0) { $0 + Int($1.completions) })").font(.system(size: 13, weight: .semibold)).monospacedDigit()
                }
                RowDivider()
                SettingsRow(symbol: "text.word.spacing", title: "Words written for you") {
                    Text("\(days.reduce(0) { $0 + Int($1.words) })").font(.system(size: 13, weight: .semibold)).monospacedDigit()
                }
                RowDivider()
                SettingsRow(symbol: "calendar", title: "Days used") {
                    Text("\(days.count)").font(.system(size: 13, weight: .semibold)).monospacedDigit()
                }
            }
        }
        .onAppear { days = statsPerDay() }
        // Core polls today's counts; re-read the history when they move.
        .onChange(of: core.today.completions) { _, _ in days = statsPerDay() }
    }

    /// "↑ 47 from yesterday · 141 words"; the trend only for the current period.
    private func detail(_ shown: Bucket, now: Bucket, before: Bucket) -> some View {
        let words = Text("\(shown.words.formatted()) \(shown.words == 1 ? "word" : "words")")
        guard shown.start == now.start else { return AnyView(words) }
        let diff = now.completions - before.completions
        let trend = diff == 0
            ? Text("Same as \(period.previous)")
            : Text("\(Image(systemName: diff > 0 ? "arrow.up" : "arrow.down")) \(abs(diff).formatted()) from \(period.previous)")
        return AnyView(HStack(spacing: 0) { trend; Text(" · "); words })
    }

    private func label(_ start: Date) -> String {
        switch period {
        case .day: start.formatted(.dateTime.weekday(.wide).month(.abbreviated).day())
        case .week: "Week of " + start.formatted(.dateTime.month(.abbreviated).day())
        case .month: start.formatted(.dateTime.month(.wide).year())
        }
    }

    private var axisFormat: Date.FormatStyle {
        switch period {
        case .day: .dateTime.day()
        case .week: .dateTime.month(.abbreviated).day()
        case .month: .dateTime.month(.abbreviated)
        }
    }

    /// The last `count` periods, oldest first, ending with the current one; empty periods count as 0.
    private func buckets(_ p: Period) -> [Bucket] {
        let cal = Calendar.current
        guard let thisStart = cal.dateInterval(of: p.unit, for: .now)?.start else { return [] }
        var out = (0..<p.count).reversed().compactMap { cal.date(byAdding: p.unit, value: -$0, to: thisStart) }.map { Bucket(start: $0) }
        let parse = Date.ISO8601FormatStyle(timeZone: .current).year().month().day()
        for d in days {
            guard let date = try? parse.parse(d.day),
                  let start = cal.dateInterval(of: p.unit, for: date)?.start,
                  let i = out.firstIndex(where: { $0.start == start }) else { continue }
            out[i].completions += Int(d.completions)
            out[i].words += Int(d.words)
        }
        return out
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

/// Per-app overrides (spec §14.2 subset), laid out like System Settings' Login Items list:
/// apps and websites in separate lists, +/− under each, an info button per row for the details.
struct AppsPane: View {
    @ObservedObject private var core = Core.shared
    @State private var selection: String?
    @State private var editing: String?
    @State private var addingSite = false

    var body: some View {
        SettingsPage(title: "Apps & websites") {
            list(sites: false)
            list(sites: true)
        }
        .sheet(item: Binding(get: { editing.map(SheetID.init) }, set: { editing = $0?.id })) { item in
            if let i = core.apps.firstIndex(where: { $0.id == item.id }) {
                OverrideDetail(app: $core.apps[i]) { editing = nil } remove: {
                    editing = nil
                    remove(item.id)
                }
            }
        }
        .sheet(isPresented: $addingSite) {
            AddWebsiteSheet { key in
                if !core.apps.contains(where: { $0.bundleID == key }) { core.apps.append(AppSetting(bundleID: key)) }
                selection = key
            }
        }
    }

    /// Apps or websites (`domain:` keys): its own card with +/− under it.
    private func list(sites: Bool) -> some View {
        let items = core.apps.filter { $0.bundleID.hasPrefix("domain:") == sites }
        let selectedHere = items.contains { $0.id == selection }
        return SettingsCard(sites ? "Websites" : "Apps") {
            if items.isEmpty {
                Text(sites ? "No websites" : "No apps")
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, minHeight: 52)
            }
            ForEach(items) { app in
                if let i = core.apps.firstIndex(where: { $0.id == app.id }) {
                    OverrideRow(app: $core.apps[i], selected: selection == app.id,
                                select: { selection = app.id }, info: { editing = app.id })
                    if app.id != items.last?.id { RowDivider() }
                }
            }
            Divider().padding(.horizontal, -16)
            HStack(spacing: 0) {
                Button { if sites { addingSite = true } else { pickApp() } } label: {
                    Image(systemName: "plus").frame(width: 24, height: 22)
                }
                .buttonStyle(.borderless)
                .help(sites ? "Add a website" : "Add an app")
                Divider().frame(height: 14).padding(.horizontal, 4)
                Button { remove(selection) } label: { Image(systemName: "minus").frame(width: 24, height: 22) }
                    .buttonStyle(.borderless)
                    .disabled(!selectedHere)
                    .help(sites ? "Remove the selected website" : "Remove the selected app")
                Spacer()
            }
            .padding(.vertical, 4)
        }
    }

    private func remove(_ id: String?) {
        core.apps.removeAll { $0.bundleID == id }
        if selection == id { selection = nil }
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
                selection = id
            }
        }
    }
}

private struct SheetID: Identifiable { let id: String }

/// App icon, or a globe for a website.
private struct OverrideIcon: View {
    let id: String
    var size: CGFloat = 24

    var body: some View {
        if let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: id) {
            Image(nsImage: NSWorkspace.shared.icon(forFile: url.path)).resizable().frame(width: size, height: size)
        } else {
            Image(systemName: id.hasPrefix("domain:") ? "globe" : "app.dashed")
                .font(.system(size: size * 0.55, weight: .medium))
                .foregroundStyle(.white)
                .frame(width: size * 0.84, height: size * 0.84)
                .background(RoundedRectangle(cornerRadius: size * 0.22, style: .continuous).fill(Color.blue.gradient))
                .frame(width: size, height: size)
        }
    }
}

/// One list row: click selects it (for −), the switch turns completions on or off, › opens details.
private struct OverrideRow: View {
    @Binding var app: AppSetting
    let selected: Bool
    let select: () -> Void
    let info: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            OverrideIcon(id: app.bundleID)
            VStack(alignment: .leading, spacing: 1) {
                Text(appName(app.bundleID)).font(.system(size: 13))
                Text(summary).font(.system(size: 11)).foregroundStyle(.secondary)
            }
            Spacer(minLength: 16)
            Toggle("Completions", isOn: Binding(get: { !app.completionsOff }, set: { app.completionsOff = !$0 }))
                .toggleStyle(.switch).controlSize(.small).labelsHidden().tint(.orange)
                .help("Completions in \(appName(app.bundleID))")
            Button(action: info) {
                Image(systemName: "chevron.right").font(.system(size: 12, weight: .semibold)).frame(width: 20, height: 20)
            }
                .buttonStyle(.borderless)
                .foregroundStyle(.tertiary)
                .help("Details")
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 7)
        .background(RoundedRectangle(cornerRadius: 6, style: .continuous)
            .fill(selected ? Color.accentColor.opacity(0.18) : .clear))
        .padding(.horizontal, -8)
        .contentShape(Rectangle())
        .onTapGesture(perform: select)
        .simultaneousGesture(TapGesture(count: 2).onEnded(info))
        .accessibilityElement(children: .contain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private var summary: String {
        if app.completionsOff { return "Completions off" }
        var parts: [String] = []
        if app.tabOff { parts.append("Tab off") }
        if !app.instructions.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { parts.append("Own instructions") }
        return parts.isEmpty ? "Same as everywhere" : parts.joined(separator: " · ")
    }
}

/// Details sheet, like the ones System Settings opens from a row's ›.
private struct OverrideDetail: View {
    @Binding var app: AppSetting
    let done: () -> Void
    let remove: () -> Void

    var body: some View {
        let isSite = app.bundleID.hasPrefix("domain:")
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                OverrideIcon(id: app.bundleID, size: 40)
                VStack(alignment: .leading, spacing: 2) {
                    Text(appName(app.bundleID)).font(.system(size: 15, weight: .semibold))
                    Text(isSite ? "Website" : "App").font(.system(size: 11)).foregroundStyle(.secondary)
                }
                Spacer()
            }
            .padding(.horizontal, 20)
            .padding(.top, 20)

            Form {
                Section {
                    Toggle("Completions", isOn: Binding(get: { !app.completionsOff }, set: { app.completionsOff = !$0 }))
                    if !isSite {
                        Toggle("Tab accepts the next word", isOn: Binding(get: { !app.tabOff }, set: { app.tabOff = !$0 }))
                            .disabled(app.completionsOff)
                    }
                }
                .controlSize(.small)
                Section {
                    TextField("Instructions", text: $app.instructions,
                              prompt: Text(isSite ? "For example: Reply in a friendly, short style." : "For example: Write in British English."),
                              axis: .vertical)
                        .labelsHidden()
                        .lineLimit(3...6)
                } header: {
                    Text("Instructions")
                } footer: {
                    Text("Added to every suggestion here, on top of your general instructions.")
                        .foregroundStyle(.secondary)
                }
                .disabled(app.completionsOff)
            }
            .formStyle(.grouped)
            .scrollDisabled(true)
            .tint(.orange)

            HStack {
                Button("Remove from List", role: .destructive, action: remove)
                Spacer()
                Button("Done", action: done).keyboardShortcut(.defaultAction)
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 20)
        }
        .frame(width: 460)
        .fixedSize(horizontal: false, vertical: true)
    }
}

private struct AddWebsiteSheet: View {
    let add: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var text = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Add Website").font(.system(size: 13, weight: .semibold))
            Text("NexType reads the address in Safari, Chrome, Arc and other browsers.")
                .font(.system(size: 11)).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            TextField("Website", text: $text, prompt: Text("mail.google.com"))
                .labelsHidden()
                .textFieldStyle(.roundedBorder)
                .onSubmit(commit)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Add", action: commit).keyboardShortcut(.defaultAction).disabled(key == nil)
            }
        }
        .padding(20)
        .frame(width: 340)
    }

    /// "https://Mail.Google.com/x" or "mail.google.com" → "domain:mail.google.com".
    private var key: String? {
        var s = text.trimmingCharacters(in: .whitespaces).lowercased()
        if let host = URL(string: s)?.host { s = host } else if let i = s.firstIndex(of: "/") { s = String(s[..<i]) }
        return s.contains(".") && !s.contains(" ") ? "domain:" + s : nil
    }

    private func commit() {
        guard let key else { return }
        add(key)
        dismiss()
    }
}

// MARK: App

struct ModelPane: View {
    @ObservedObject private var core = Core.shared

    var body: some View {
        SettingsPage(title: "Model") {
            SettingsCard { ModelList() }
            Note("Recommended for this Mac's \(macMemoryGB) GB of memory. Bigger models write better but use more memory and can be slower. Every model runs on your Mac; your text never leaves it.")
            SettingsCard {
                SettingsRow(symbol: "folder", title: "Model file", subtitle: (core.model.url.path as NSString).abbreviatingWithTildeInPath) {
                    Button("Show in Finder") { NSWorkspace.shared.activateFileViewerSelecting([core.model.url]) }
                        .disabled(!core.model.installed)
                }
            }
        }
    }
}

/// Every model, with the one this Mac should run marked. Used in Settings and the setup window.
struct ModelList: View {
    var body: some View {
        VStack(spacing: 0) {
            ForEach(models) { m in
                ModelRow(model: m, download: downloads[m.id]!)
                if m != models.last { RowDivider() }
            }
        }
    }
}

/// Click to use a model; one not on disk is downloaded first.
private struct ModelRow: View {
    let model: ModelInfo
    @ObservedObject var download: Download
    @ObservedObject private var core = Core.shared

    var body: some View {
        let selected = core.model == model
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 12) {
                Image(systemName: selected ? "checkmark.circle.fill" : "circle")
                    .font(.system(size: 16))
                    .foregroundStyle(selected ? Color.orange : Color.secondary)
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text(model.name).font(.system(size: 13))
                        if model == recommendedModel {
                            Text("Recommended").font(.system(size: 10, weight: .semibold)).foregroundStyle(.orange)
                                .padding(.horizontal, 6).padding(.vertical, 2)
                                .background(Capsule().fill(Color.orange.opacity(0.15)))
                        }
                    }
                    Text(subtitle).font(.system(size: 11))
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 16)
                if case .running = download.state {
                    Button("Cancel") { download.cancel() }
                } else if download.isBusy {
                    EmptyView()
                } else if !model.installed {
                    Button("Download") { core.select(model) }
                } else if !selected {
                    Button("Delete") { core.delete(model) }
                }
            }
            .padding(.vertical, 8)
            .contentShape(Rectangle())
            .onTapGesture { if !selected { core.select(model) } }
            if case .running(let fraction) = download.state {
                ProgressView(value: fraction).tint(.orange).padding(.leading, 28).padding(.bottom, 8)
            }
        }
    }

    private var subtitle: String {
        if let line = downloadLine(download.state) { return line }
        let base = model.sizeText
        if model.installed { return base + " · Downloaded" }
        if !model.fitsThisMac { return base + " · Best with more than \(model.bestAboveGB) GB of RAM; may slow down other apps" }
        return base
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
            SettingsCard {
                ToggleRow(symbol: "arrow.triangle.2.circlepath", title: "Check for updates automatically",
                          subtitle: "Once a day. You're asked before anything is installed.", isOn: $autoCheck)
                    .onChange(of: autoCheck) { _, on in updater.updater.automaticallyChecksForUpdates = on }
                RowDivider()
                SettingsRow(symbol: "arrow.down.app", title: "Updates", subtitle: "Version \(version) (build \(build))") {
                    Button("Check Now") { updater.checkForUpdates(nil) }
                }
            }
        }
    }

    @State private var autoCheck = updater.updater.automaticallyChecksForUpdates
    private var version: String { Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev" }
    private var build: String { Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "?" }
}
