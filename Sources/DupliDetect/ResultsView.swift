import SwiftUI

struct ResultsView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var player: PreviewPlayer

    var body: some View {
        VStack(spacing: 0) {
            if let result = model.result, !result.groups.isEmpty {
                summaryBar(result)
                Divider()
                groupList(result)
                Divider()
                actionBar
            } else {
                EmptyStateView(model: model)
            }
        }
        .background(Color(nsColor: .textBackgroundColor))
        .alert("Cannot play this file",
               isPresented: Binding(get: { player.lastError != nil },
                                    set: { if !$0 { player.clearError() } })) {
            Button("OK", role: .cancel) { player.clearError() }
        } message: {
            Text(player.lastError ?? "")
        }
    }

    private func summaryBar(_ result: ScanResult) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 20) {
            statistic(value: "\(result.groups.count)", label: result.groups.count == 1 ? "group" : "groups")
            statistic(value: "\(result.groups.reduce(0) { $0 + $1.files.count })", label: "files")
            statistic(value: Format.bytes(result.reclaimableBytes), label: "recoverable", highlight: true)
            Spacer()
            if !result.filesSkipped.isEmpty {
                SkippedFilesButton(skipped: result.filesSkipped)
            }
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 12)
        .background(Color(nsColor: .controlBackgroundColor))
    }

    private func statistic(value: String, label: String, highlight: Bool = false) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 5) {
            Text(value)
                .font(.title3.weight(.semibold).monospacedDigit())
                .foregroundColor(highlight ? .accentColor : .primary)
            Text(label).font(.caption).foregroundColor(.secondary)
        }
    }

    private func groupList(_ result: ScanResult) -> some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 10) {
                ForEach(result.groups) { group in
                    GroupCard(group: group, level: result.matchLevel, model: model, player: player)
                }
            }
            .padding(16)
        }
    }

    private var actionBar: some View {
        HStack(spacing: 12) {
            Menu {
                ForEach(KeepRule.allCases) { rule in
                    Button(rule.rawValue) {
                        model.keepRule = rule
                        model.applyKeepRule()
                    }
                }
            } label: {
                Label("Keep \(model.keepRule.rawValue.lowercased())", systemImage: "wand.and.stars")
            }
            .menuStyle(.borderlessButton)
            .fixedSize()

            Button("Clear Selection") { model.clearMarks() }
                .disabled(model.marked.isEmpty)
                .accessibilityLabel("Clear Selection")

            Spacer()

            if model.marked.isEmpty {
                Text("Nothing selected").font(.callout).foregroundColor(.secondary)
            } else {
                Text("\(model.marked.count) selected · \(Format.bytes(model.markedBytes))")
                    .font(.callout)
                    .foregroundColor(.secondary)
            }

            Button {
                model.trashMarkedFiles()
            } label: {
                Label("Move to Trash", systemImage: "trash")
            }
            .keyboardShortcut(.delete, modifiers: .command)
            .disabled(model.marked.isEmpty)
            .accessibilityLabel("Move to Trash")
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 12)
        .background(Color(nsColor: .controlBackgroundColor))
    }
}

// MARK: - One duplicate group

struct GroupCard: View {
    let group: DuplicateGroup
    let level: FingerprintMatcher.Level
    @ObservedObject var model: AppModel
    @ObservedObject var player: PreviewPlayer

    private var isExpanded: Bool { model.expandedGroups.contains(group.id) }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            if isExpanded {
                Divider()
                VStack(spacing: 0) {
                    ForEach(Array(group.files.enumerated()), id: \.element.id) { index, file in
                        if index > 0 { Divider().padding(.leading, 44) }
                        FileRow(file: file, group: group, model: model, player: player)
                    }
                }
            }
        }
        .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .controlBackgroundColor)))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Color(nsColor: .separatorColor)))
    }

    private var header: some View {
        Button {
            withAnimation(.easeInOut(duration: 0.15)) {
                if isExpanded { model.expandedGroups.remove(group.id) }
                else { model.expandedGroups.insert(group.id) }
            }
        } label: {
            HStack(spacing: 10) {
                Image(systemName: "chevron.right")
                    .font(.caption.weight(.semibold))
                    .foregroundColor(.secondary)
                    .rotationEffect(.degrees(isExpanded ? 90 : 0))
                    .frame(width: 12)

                badge

                Text(group.files.first?.displayName ?? "")
                    .fontWeight(.medium)
                    .lineLimit(1)
                    .truncationMode(.middle)

                Text("+\(group.files.count - 1) more")
                    .font(.caption)
                    .foregroundColor(.secondary)

                Spacer(minLength: 8)

                Text(Format.bytes(group.reclaimableBytes))
                    .font(.callout.monospacedDigit())
                    .foregroundColor(.accentColor)
                Text("recoverable").font(.caption).foregroundColor(.secondary)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    /// At Perfect match the app has asserted that these are the same recording
    /// end to end, so it says that. A percentage there would undercut a
    /// conclusion it is certain of: the figure is the weakest pair in the
    /// group, which for a cross-format group is always two different lossy
    /// codecs compared against each other, and so measures codec aggressiveness
    /// rather than whether the recording is the same one.
    private var asserted: Bool {
        group.kind == .exact || level == .perfect
    }

    private var badgeText: String {
        if group.kind == .exact { return "Identical" }
        if level == .perfect { return "Same recording" }
        return "\(Format.percent(group.confidence)) match"
    }

    private var badge: some View {
        HStack(spacing: 4) {
            Image(systemName: group.kind == .exact ? "equal.circle.fill" : "waveform")
                .font(.caption)
            Text(badgeText).font(.caption.weight(.medium))
        }
        .padding(.horizontal, 7)
        .padding(.vertical, 3)
        .background(Capsule().fill((asserted ? Color.green : Color.orange).opacity(0.15)))
        .foregroundColor(asserted ? .green : .orange)
    }
}

// MARK: - One file inside a group

struct FileRow: View {
    let file: AudioFile
    let group: DuplicateGroup
    @ObservedObject var model: AppModel
    @ObservedObject var player: PreviewPlayer

    private var isMarked: Bool { model.marked.contains(file.id) }
    private var isPlaying: Bool { player.playingFile == file.id }

    var body: some View {
        HStack(spacing: 10) {
            Button {
                model.toggleMark(file, in: group)
            } label: {
                Image(systemName: isMarked ? "trash.circle.fill" : "checkmark.circle")
                    .font(.system(size: 17))
                    .foregroundColor(isMarked ? .red : .green)
            }
            .buttonStyle(.plain)
            .help(isMarked ? "Marked for the Trash — click to keep" : "Being kept — click to mark for the Trash")
            .accessibilityLabel(isMarked ? "Marked for Trash: \(file.displayName)" : "Keeping: \(file.displayName)")

            Button {
                player.toggle(file: file)
            } label: {
                Image(systemName: isPlaying ? "stop.circle.fill" : "play.circle")
                    .font(.system(size: 16))
                    .foregroundColor(isPlaying ? .accentColor : .secondary)
            }
            .buttonStyle(.plain)
            .help("Preview the first 30 seconds")
            .accessibilityLabel(isPlaying ? "Stop preview" : "Preview \(file.displayName)")

            VStack(alignment: .leading, spacing: 2) {
                Text(file.displayName)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .foregroundColor(isMarked ? .secondary : .primary)
                Text(file.folderPath)
                    .font(.caption2)
                    .foregroundColor(.secondary)
                    .lineLimit(1)
                    .truncationMode(.head)
            }

            Spacer(minLength: 8)

            HStack(spacing: 14) {
                detail(file.formatName ?? file.url.pathExtension.uppercased(), width: 78)
                detail(file.qualitySummary, width: 96)
                detail(Format.duration(file.duration), width: 52)
                detail(Format.bytes(file.byteSize), width: 68)
            }
            .font(.caption.monospacedDigit())
            .foregroundColor(.secondary)

            Button {
                model.reveal(file)
            } label: {
                Image(systemName: "arrow.forward.circle").font(.system(size: 15))
            }
            .buttonStyle(.plain)
            .foregroundColor(.secondary)
            .help("Reveal in Finder")
            .accessibilityLabel("Reveal \(file.displayName) in Finder")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(isMarked ? Color.red.opacity(0.06) : Color.clear)
        .contextMenu {
            Button(isMarked ? "Keep This One" : "Mark for Trash") { model.toggleMark(file, in: group) }
            Button("Reveal in Finder") { model.reveal(file) }
        }
    }

    private func detail(_ text: String, width: CGFloat) -> some View {
        Text(text)
            .lineLimit(1)
            .frame(width: width, alignment: .trailing)
    }
}

// MARK: - Empty states

struct EmptyStateView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        VStack(spacing: 14) {
            Spacer()
            Image(systemName: icon)
                .font(.system(size: 46))
                .foregroundColor(.secondary.opacity(0.55))
            Text(title).font(.title3.weight(.medium))
            Text(subtitle)
                .font(.callout)
                .foregroundColor(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 380)
            if let result = model.result, !result.filesSkipped.isEmpty {
                SkippedFilesButton(skipped: result.filesSkipped)
            }
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var icon: String {
        if model.isScanning { return "waveform" }
        if model.trashedCount > 0 { return "sparkles" }
        return model.result != nil ? "checkmark.circle" : "music.note.list"
    }

    private var title: String {
        if model.isScanning { return "Scanning…" }
        if model.trashedCount > 0 { return "All cleaned up" }
        guard let result = model.result else { return "No scan yet" }
        return result.filesScanned == 0 ? "No audio files found" : "No duplicates found"
    }

    private var subtitle: String {
        if model.isScanning { return model.phase.describes }
        if model.trashedCount > 0 {
            let files = model.trashedCount == 1 ? "1 file" : "\(model.trashedCount) files"
            return "Moved \(files) to the Trash, freeing \(Format.bytes(model.trashedBytes)). "
                 + "Nothing was deleted permanently — you can put anything back from the Trash."
        }
        guard let result = model.result else {
            return "Add the folders you want to search, then click Scan for Duplicates."
        }
        if result.filesScanned == 0 {
            return "Nothing in those folders looked like audio. Check that subfolders are included."
        }
        return "Checked \(result.filesScanned) audio files and every one of them is unique."
    }
}

struct SkippedFilesButton: View {
    let skipped: [(url: URL, reason: String)]
    @State private var showing = false

    var body: some View {
        Button {
            showing = true
        } label: {
            Label("\(skipped.count) skipped", systemImage: "exclamationmark.triangle")
                .font(.caption)
        }
        .buttonStyle(.borderless)
        .foregroundColor(.secondary)
        .popover(isPresented: $showing) {
            VStack(alignment: .leading, spacing: 8) {
                Text("Skipped files").font(.headline)
                Text("These could not be compared by their audio.")
                    .font(.caption).foregroundColor(.secondary)
                Divider()
                ScrollView {
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(Array(skipped.prefix(200).enumerated()), id: \.offset) { _, item in
                            VStack(alignment: .leading, spacing: 1) {
                                Text(item.url.lastPathComponent).font(.callout).lineLimit(1)
                                Text(item.reason).font(.caption2).foregroundColor(.secondary)
                            }
                        }
                    }
                }
                .frame(maxHeight: 260)
            }
            .padding(16)
            .frame(width: 420)
        }
    }
}
