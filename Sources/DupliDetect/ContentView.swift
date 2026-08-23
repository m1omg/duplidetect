import SwiftUI
import AppKit
import UniformTypeIdentifiers

struct ContentView: View {
    @StateObject private var model = AppModel.shared
    @StateObject private var player = PreviewPlayer()

    var body: some View {
        HSplitView {
            SidebarView(model: model)
                .frame(minWidth: 280, idealWidth: 320, maxWidth: 420)
            ResultsView(model: model, player: player)
                .frame(minWidth: 520)
        }
        .frame(minWidth: 900, minHeight: 560)
        .onReceive(NotificationCenter.default.publisher(for: .addFoldersRequested)) { _ in
            model.addFolders()
        }
        .onReceive(NotificationCenter.default.publisher(for: .scanRequested)) { _ in
            model.scan()
        }
        .alert("Something went wrong",
               isPresented: Binding(get: { model.lastError != nil },
                                    set: { if !$0 { model.clearError() } })) {
            Button("OK", role: .cancel) { model.clearError() }
        } message: {
            Text(model.lastError ?? "")
        }
    }
}

// MARK: - Sidebar

struct SidebarView: View {
    @ObservedObject var model: AppModel
    @State private var isTargeted = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header

            FolderListView(model: model, isTargeted: $isTargeted)

            Divider()

            ScrollView {
                OptionsView(model: model).padding(16)
            }

            Divider()
            scanControls.padding(16)
        }
        .background(Color(nsColor: .controlBackgroundColor))
    }

    private var header: some View {
        HStack(spacing: 10) {
            Image(systemName: "waveform.badge.magnifyingglass")
                .font(.system(size: 22))
                .foregroundColor(.accentColor)
            VStack(alignment: .leading, spacing: 1) {
                Text("DupliDetect").font(.headline)
                Text("Find duplicate audio").font(.caption).foregroundColor(.secondary)
            }
            Spacer()
        }
        .padding(16)
    }

    private var scanControls: some View {
        VStack(spacing: 10) {
            if model.isScanning {
                VStack(spacing: 8) {
                    if let fraction = model.phase.fraction {
                        ProgressView(value: fraction)
                    } else {
                        ProgressView().progressViewStyle(.linear)
                    }
                    Text(model.phase.describes)
                        .font(.caption)
                        .foregroundColor(.secondary)
                        .lineLimit(1)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                Button("Stop Scanning") { model.cancelScan() }
                    .frame(maxWidth: .infinity)
            } else {
                Button {
                    model.scan()
                } label: {
                    Label("Scan for Duplicates", systemImage: "magnifyingglass")
                        .frame(maxWidth: .infinity)
                }
                .keyboardShortcut(.return, modifiers: .command)
                .controlSize(.large)
                .disabled(model.folders.isEmpty)
                .accessibilityLabel("Scan for Duplicates")

                if model.folders.isEmpty {
                    Text("Add at least one folder to begin.")
                        .font(.caption)
                        .foregroundColor(.secondary)
                }
            }
        }
    }
}

struct FolderListView: View {
    @ObservedObject var model: AppModel
    @Binding var isTargeted: Bool
    @State private var hoveredFolder: URL?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            header

            if model.folders.isEmpty {
                emptyDropZone
            } else {
                folderRows
            }
        }
        .padding(.vertical, 8)
        .background(
            RoundedRectangle(cornerRadius: 8)
                .strokeBorder(isTargeted ? Color.accentColor : Color.clear, lineWidth: 2)
                .padding(.horizontal, 10)
        )
        .onDrop(of: [UTType.fileURL], isTargeted: $isTargeted) { providers in
            load(providers: providers)
            return true
        }
    }

    private var header: some View {
        HStack(spacing: 6) {
            Text("FOLDERS").font(.caption.weight(.semibold)).foregroundColor(.secondary)
            Spacer()
            if !model.folders.isEmpty {
                Button("Remove All") { model.removeAllFolders() }
                    .buttonStyle(.borderless)
                    .font(.caption)
                    .accessibilityLabel("Remove All Folders")
            }
            Button {
                model.addFolders()
            } label: {
                Image(systemName: "plus")
            }
            .buttonStyle(.borderless)
            .help("Add a folder to search")
            .accessibilityLabel("Add Folder")
        }
        .padding(.horizontal, 16)
    }

    private var emptyDropZone: some View {
        VStack(spacing: 10) {
            Image(systemName: "folder.badge.plus")
                .font(.system(size: 26))
                .foregroundColor(.secondary)
            Text("Drag folders here")
                .font(.callout)
                .foregroundColor(.secondary)
            Text("or")
                .font(.caption2)
                .foregroundColor(.secondary)
            Button("Choose Folder…") { model.addFolders() }
                .accessibilityLabel("Choose Folder")
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 22)
        .padding(.horizontal, 16)
    }

    private var folderRows: some View {
        ScrollView {
            VStack(spacing: 2) {
                ForEach(model.folders, id: \.self) { folder in
                    folderRow(folder)
                }
            }
            .padding(.horizontal, 12)
        }
        .frame(height: min(CGFloat(model.folders.count) * 44 + 8, 190))
    }

    private func folderRow(_ folder: URL) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "folder.fill").foregroundColor(.accentColor)

            VStack(alignment: .leading, spacing: 1) {
                Text(folder.lastPathComponent).lineLimit(1)
                Text(folder.deletingLastPathComponent().path)
                    .font(.caption2)
                    .foregroundColor(.secondary)
                    .lineLimit(1)
                    .truncationMode(.head)
            }

            Spacer(minLength: 4)

            Button {
                model.removeFolder(folder)
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .font(.system(size: 14))
                    .foregroundColor(hoveredFolder == folder ? .secondary : Color.secondary.opacity(0.45))
            }
            .buttonStyle(.plain)
            .help("Stop searching this folder")
            .accessibilityLabel("Remove \(folder.lastPathComponent)")
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(hoveredFolder == folder ? Color.secondary.opacity(0.12) : Color.clear)
        )
        .onHover { inside in
            hoveredFolder = inside ? folder : (hoveredFolder == folder ? nil : hoveredFolder)
        }
        .contextMenu {
            Button("Remove") { model.removeFolder(folder) }
            Button("Reveal in Finder") { NSWorkspace.shared.activateFileViewerSelecting([folder]) }
        }
    }

    private func load(providers: [NSItemProvider]) {
        for provider in providers {
            _ = provider.loadObject(ofClass: URL.self) { url, _ in
                guard let url else { return }
                var isDirectory: ObjCBool = false
                guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory) else { return }
                let folder = isDirectory.boolValue ? url : url.deletingLastPathComponent()
                Task { @MainActor in model.add(urls: [folder]) }
            }
        }
    }
}

struct OptionsView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            whatSection
            Divider()
            whereSection
            Divider()
            keepSection
        }
    }

    private var whatSection: some View {
        VStack(alignment: .leading, spacing: 14) {
            sectionTitle("WHAT TO LOOK FOR")

            Toggle("Identical files", isOn: $model.options.findExactDuplicates)
                .help("Files that are byte-for-byte the same")
            Toggle("Same audio in any format", isOn: $model.options.findSimilarAudio)
                .help("The same recording saved as MP3, WAV, M4A, FLAC, Ogg Vorbis…")

            if model.options.findSimilarAudio {
                VStack(alignment: .leading, spacing: 4) {
                    HStack {
                        Text("Match strictness").font(.callout)
                        Spacer()
                        Text(strictnessLabel).font(.caption).foregroundColor(.secondary)
                    }
                    Slider(value: $model.options.sensitivity, in: 0...1)
                    Text(strictnessHelp).font(.caption2).foregroundColor(.secondary)
                }
                .padding(.leading, 18)
            }
        }
    }

    private var whereSection: some View {
        VStack(alignment: .leading, spacing: 14) {
            sectionTitle("WHERE TO LOOK")
            Toggle("Include subfolders", isOn: $model.options.includeSubfolders)
            Toggle("Skip hidden files", isOn: $model.options.skipHiddenFiles)

            HStack {
                Text("Ignore clips under").font(.callout)
                Spacer()
                Stepper(value: $model.options.minimumDuration, in: 0...60, step: 1) {
                    Text("\(Int(model.options.minimumDuration))s").monospacedDigit()
                }
            }
        }
    }

    private var keepSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            sectionTitle("WHICH COPY TO KEEP")
            Picker("", selection: $model.keepRule) {
                ForEach(KeepRule.allCases) { rule in
                    Text(rule.rawValue).tag(rule)
                }
            }
            .labelsHidden()
            .onChange(of: model.keepRule) { _ in model.applyKeepRule() }
            Text(model.keepRule.explanation).font(.caption2).foregroundColor(.secondary)
        }
    }

    private func sectionTitle(_ text: String) -> some View {
        Text(text).font(.caption.weight(.semibold)).foregroundColor(.secondary)
    }

    private var strictnessLabel: String {
        switch model.options.sensitivity {
        case ..<0.25: return "Very strict"
        case ..<0.5: return "Strict"
        case ..<0.75: return "Relaxed"
        default: return "Very relaxed"
        }
    }

    private var strictnessHelp: String {
        model.options.sensitivity < 0.5
            ? "Only near-identical recordings are grouped."
            : "Also groups heavily re-encoded or edited copies. Review these before deleting."
    }
}
