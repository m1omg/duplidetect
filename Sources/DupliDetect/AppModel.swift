import Foundation
import AppKit
import Combine

/// Which copy to keep when auto-selecting duplicates for removal.
public enum KeepRule: String, CaseIterable, Identifiable {
    case bestQuality = "Best quality"
    case largestFile = "Largest file"
    case smallestFile = "Smallest file"
    case oldest = "Oldest file"
    case newest = "Newest file"
    case shortestPath = "Shallowest folder"

    public var id: String { rawValue }

    public var explanation: String {
        switch self {
        case .bestQuality: return "Lossless first, then sample rate and bitrate — not file size"
        case .largestFile: return "Keeps the copy that takes the most space"
        case .smallestFile: return "Keeps the copy that takes the least space"
        case .oldest: return "Keeps the copy that was modified longest ago"
        case .newest: return "Keeps the most recently modified copy"
        case .shortestPath: return "Keeps the copy nearest the top of your folders"
        }
    }

    /// Picks the single file to keep out of a group.
    func keeper(from files: [AudioFile]) -> AudioFile? {
        switch self {
        case .bestQuality:
            // Never trade away audio for fidelity: a copy missing a chunk of the
            // recording is disqualified no matter how good it sounds. Silent
            // padding does not count, so a file that is merely padded still competes.
            let longest = files.compactMap(\.contentDuration).max() ?? 0
            var contenders = files
            if longest > 0 {
                let complete = files.filter { ($0.contentDuration ?? longest) >= longest * 0.95 }
                if !complete.isEmpty { contenders = complete }
            }
            return contenders.max(by: AudioFile.isLowerQuality)
        case .largestFile: return files.max { $0.byteSize < $1.byteSize }
        case .smallestFile: return files.min { $0.byteSize < $1.byteSize }
        case .oldest: return files.min { $0.modified < $1.modified }
        case .newest: return files.max { $0.modified < $1.modified }
        case .shortestPath:
            return files.min {
                let a = $0.url.pathComponents.count, b = $1.url.pathComponents.count
                if a != b { return a < b }
                if $0.url.path.count != $1.url.path.count {
                    return $0.url.path.count < $1.url.path.count
                }
                // Without this, two equally shallow paths of equal length are a
                // tie, and `min` then returns whichever the directory scan
                // happened to reach first — so the file kept could differ
                // between machines, or between runs on a different filesystem.
                return $0.url.path < $1.url.path
            }
        }
    }
}

@MainActor
public final class AppModel: ObservableObject {

    @Published public var folders: [URL] = []
    @Published public var options = ScanOptions()
    @Published public var keepRule: KeepRule = .bestQuality

    @Published public private(set) var phase: ScanPhase = .idle
    @Published public private(set) var isScanning = false
    @Published public private(set) var result: ScanResult?
    @Published public private(set) var lastError: String?

    /// What the last round of trashing actually removed.
    @Published public private(set) var trashedCount = 0
    @Published public private(set) var trashedBytes: Int64 = 0

    /// Files the user has marked for removal.
    @Published public var marked: Set<UUID> = []
    @Published public var expandedGroups: Set<UUID> = []

    private var scanner: Scanner?

    /// Shared instance so folders opened via the Dock or `open -a` reach the UI.
    public static let shared = AppModel()

    public init() {}

    // MARK: - Sources

    public func addFolders() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = true
        panel.canCreateDirectories = false
        panel.prompt = "Add"
        panel.title = "Add Folders to Search"
        panel.message = "Choose the folders you want searched for duplicate audio."

        // Presented as a sheet on the main window so it is obviously part of
        // DupliDetect, rather than appearing as a detached window.
        if let window = NSApp.keyWindow ?? NSApp.mainWindow ?? NSApp.windows.first(where: \.isVisible) {
            panel.beginSheetModal(for: window) { response in
                guard response == .OK else { return }
                let urls = panel.urls
                Task { @MainActor in self.add(urls: urls) }
            }
        } else if panel.runModal() == .OK {
            add(urls: panel.urls)
        }
    }

    public func add(urls: [URL]) {
        for url in urls where !folders.contains(url) {
            folders.append(url)
        }
    }

    public func removeFolders(_ offsets: IndexSet) {
        folders.remove(atOffsets: offsets)
    }

    public func removeFolder(_ url: URL) {
        folders.removeAll { $0 == url }
    }

    public func removeAllFolders() {
        folders.removeAll()
    }

    // MARK: - Scanning

    public func scan() {
        guard !folders.isEmpty, !isScanning else { return }
        isScanning = true
        lastError = nil
        result = nil
        marked = []
        trashedCount = 0
        trashedBytes = 0
        expandedGroups = []
        phase = .collecting

        let scanner = Scanner()
        self.scanner = scanner
        let roots = folders
        let options = self.options

        Task.detached(priority: .userInitiated) {
            let outcome = scanner.run(roots: roots, options: options) { phase in
                Task { @MainActor in self.phase = phase }
            }
            await MainActor.run {
                self.result = outcome
                self.isScanning = false
                self.phase = .finished
                self.expandedGroups = Set(outcome.groups.prefix(3).map(\.id))
                self.applyKeepRule()
            }
        }
    }

    public func cancelScan() {
        scanner?.cancel()
        isScanning = false
        phase = .idle
    }

    // MARK: - Marking

    /// Marks every file except the one the current keep rule prefers.
    public func applyKeepRule() {
        guard let groups = result?.groups else { return }
        var next: Set<UUID> = []
        for group in groups {
            guard let keeper = keepRule.keeper(from: group.files) else { continue }
            for file in group.files where file.id != keeper.id {
                next.insert(file.id)
            }
        }
        marked = next
    }

    public func clearMarks() { marked = [] }

    /// Toggling is refused when it would leave a group with nothing kept.
    public func toggleMark(_ file: AudioFile, in group: DuplicateGroup) {
        if marked.contains(file.id) {
            marked.remove(file.id)
        } else {
            let remaining = group.files.filter { $0.id != file.id && !marked.contains($0.id) }
            guard !remaining.isEmpty else { return }
            marked.insert(file.id)
        }
    }

    public func isKept(_ file: AudioFile, in group: DuplicateGroup) -> Bool {
        !marked.contains(file.id)
    }

    public var markedFiles: [AudioFile] {
        (result?.groups.flatMap(\.files) ?? []).filter { marked.contains($0.id) }
    }

    public var markedBytes: Int64 {
        markedFiles.reduce(0) { $0 + $1.byteSize }
    }

    // MARK: - Acting on results

    /// Moves every marked file to the Trash, so a mistake stays recoverable.
    public func trashMarkedFiles() {
        let targets = markedFiles
        guard !targets.isEmpty else { return }

        var failures: [String] = []
        var removed: Set<UUID> = []
        var freed: Int64 = 0

        for file in targets {
            do {
                try FileManager.default.trashItem(at: file.url, resultingItemURL: nil)
                removed.insert(file.id)
                freed += file.byteSize
            } catch {
                failures.append("\(file.displayName): \(error.localizedDescription)")
            }
        }

        // Drop trashed files from the results, and any group left without a duplicate.
        if var outcome = result {
            for index in outcome.groups.indices {
                outcome.groups[index].files.removeAll { removed.contains($0.id) }
            }
            outcome.groups.removeAll { $0.files.count < 2 }
            result = outcome
        }
        marked.subtract(removed)
        trashedCount += removed.count
        trashedBytes += freed

        lastError = failures.isEmpty ? nil
            : "Could not move \(failures.count) file(s) to the Trash:\n" + failures.joined(separator: "\n")
    }

    public func clearError() { lastError = nil }

    public func reveal(_ file: AudioFile) {
        NSWorkspace.shared.activateFileViewerSelecting([file.url])
    }
}
