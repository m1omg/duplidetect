import SwiftUI

extension Notification.Name {
    static let addFoldersRequested = Notification.Name("DupliDetect.addFolders")
    static let scanRequested = Notification.Name("DupliDetect.scan")
}

@main
struct DupliDetectApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        WindowGroup("DupliDetect") {
            ContentView()
        }
        .windowToolbarStyle(.unified)
        .commands {
            // These post notifications rather than calling AppModel directly:
            // touching the model from inside the commands builder runs during
            // scene construction and stops SwiftUI creating the window at all.
            CommandGroup(replacing: .newItem) {
                Button("Add Folder…") {
                    NotificationCenter.default.post(name: .addFoldersRequested, object: nil)
                }
                .keyboardShortcut("o", modifiers: .command)

                Button("Scan for Duplicates") {
                    NotificationCenter.default.post(name: .scanRequested, object: nil)
                }
                .keyboardShortcut("r", modifiers: .command)
            }
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)

        // Folder paths given on the command line are scanned on launch:
        //   open -a DupliDetect --args ~/Music ~/Downloads
        let paths = CommandLine.arguments.dropFirst().filter { !$0.hasPrefix("-") }
        if !paths.isEmpty {
            openFolders(paths.map { URL(fileURLWithPath: $0) }, scanImmediately: true)
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }

    /// Folders dropped on the app icon, or passed to `open -a DupliDetect`,
    /// are added to the search list and scanned straight away.
    func application(_ application: NSApplication, open urls: [URL]) {
        openFolders(urls, scanImmediately: true)
    }

    private func openFolders(_ urls: [URL], scanImmediately: Bool) {
        // A dropped audio file means "search the folder it lives in".
        let folders = urls.compactMap { url -> URL? in
            var isDirectory: ObjCBool = false
            guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory) else { return nil }
            return isDirectory.boolValue ? url : url.deletingLastPathComponent()
        }
        guard !folders.isEmpty else { return }
        Task { @MainActor in
            let model = AppModel.shared
            model.add(urls: folders)
            if scanImmediately { model.scan() }
        }
    }
}
