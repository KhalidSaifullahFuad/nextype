import AppKit

@MainActor
enum Onboarding {
    /// First run: Settings opens on Setup until everything is set up.
    /// `open NexType.app --args -showOnboarding YES` forces it.
    static func showIfNeeded() {
        guard Core.shared.status != .running || UserDefaults.standard.bool(forKey: "showOnboarding") else { return }
        SettingsWindowController.shared.show(.setup)
    }
}
