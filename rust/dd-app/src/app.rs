//! The DupliDetect interface.

use crate::scanner::{self, Phase, ScanOptions, ScanResult};
use crate::{platform, preview::Preview};
use dd_core::keep::KeepRule;
use dd_core::matcher::MatchLevel;
use dd_core::model::{AudioFile, MatchKind};
use egui::{Color32, RichText};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};

pub struct App {
    folders: Vec<PathBuf>,
    options: ScanOptions,
    keep_rule: KeepRule,

    scanning: bool,
    phase: Arc<Mutex<Phase>>,
    incoming: Option<Receiver<ScanResult>>,
    result: Option<ScanResult>,

    marked: HashSet<PathBuf>,
    expanded: HashSet<usize>,
    preview: Preview,
    error: Option<String>,
    trashed_count: usize,
    trashed_bytes: u64,
}

impl Default for App {
    fn default() -> Self {
        App {
            folders: Vec::new(),
            options: ScanOptions::default(),
            keep_rule: KeepRule::BestQuality,
            scanning: false,
            phase: Arc::new(Mutex::new(Phase::Collecting)),
            incoming: None,
            result: None,
            marked: HashSet::new(),
            expanded: HashSet::new(),
            preview: Preview::default(),
            error: None,
            trashed_count: 0,
            trashed_bytes: 0,
        }
    }
}

impl App {
    pub fn with_folders(folders: Vec<PathBuf>) -> Self {
        let mut app = App::default();
        app.folders = folders;
        if !app.folders.is_empty() {
            app.start_scan();
        }
        app
    }

    fn start_scan(&mut self) {
        if self.folders.is_empty() || self.scanning {
            return;
        }
        self.scanning = true;
        self.result = None;
        self.marked.clear();
        self.expanded.clear();
        self.trashed_count = 0;
        self.trashed_bytes = 0;
        self.error = None;

        let (tx, rx) = channel();
        self.incoming = Some(rx);
        let roots = self.folders.clone();
        let options = self.options.clone();
        let phase = Arc::clone(&self.phase);
        std::thread::spawn(move || {
            let result = scanner::run(&roots, &options, |p| {
                if let Ok(mut slot) = phase.lock() {
                    *slot = p;
                }
            });
            let _ = tx.send(result);
        });
    }

    fn apply_keep_rule(&mut self) {
        let Some(result) = &self.result else { return };
        let mut next = HashSet::new();
        for group in &result.groups {
            if let Some(keeper) = self.keep_rule.keeper(&group.files) {
                for file in &group.files {
                    if file.path != keeper.path {
                        next.insert(file.path.clone());
                    }
                }
            }
        }
        self.marked = next;
    }

    fn marked_files(&self) -> Vec<AudioFile> {
        let Some(result) = &self.result else { return Vec::new() };
        result
            .groups
            .iter()
            .flat_map(|g| g.files.iter())
            .filter(|f| self.marked.contains(&f.path))
            .cloned()
            .collect()
    }

    /// Toggling is refused when it would leave a group with nothing kept.
    fn toggle_mark(&mut self, file: &AudioFile, group_index: usize) {
        if self.marked.contains(&file.path) {
            self.marked.remove(&file.path);
            return;
        }
        let Some(result) = &self.result else { return };
        let group = &result.groups[group_index];
        let remaining = group
            .files
            .iter()
            .filter(|f| f.path != file.path && !self.marked.contains(&f.path))
            .count();
        if remaining > 0 {
            self.marked.insert(file.path.clone());
        }
    }

    fn trash_marked(&mut self) {
        let targets = self.marked_files();
        if targets.is_empty() {
            return;
        }
        let mut failures = Vec::new();
        let mut removed = HashSet::new();
        let mut freed = 0u64;

        for file in &targets {
            match platform::move_to_trash(&file.path) {
                Ok(()) => {
                    removed.insert(file.path.clone());
                    freed += file.byte_size;
                }
                Err(e) => failures.push(format!("{}: {e}", file.display_name())),
            }
        }

        if let Some(result) = &mut self.result {
            for group in &mut result.groups {
                group.files.retain(|f| !removed.contains(&f.path));
            }
            result.groups.retain(|g| g.files.len() >= 2);
        }
        for path in &removed {
            self.marked.remove(path);
        }
        self.trashed_count += removed.len();
        self.trashed_bytes += freed;

        self.error = if failures.is_empty() {
            None
        } else {
            Some(format!(
                "Could not move {} file(s) to the Trash:\n{}",
                failures.len(),
                failures.join("\n")
            ))
        };
    }

    fn choose_folders(&mut self) {
        if let Some(picked) = rfd::FileDialog::new().set_title("Add Folders to Search").pick_folders() {
            for folder in picked {
                if !self.folders.contains(&folder) {
                    self.folders.push(folder);
                }
            }
        }
    }
}

fn format_bytes(value: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = value as f64;
    let mut unit = 0;
    while v >= 1000.0 && unit < UNITS.len() - 1 {
        v /= 1000.0;
        unit += 1;
    }
    if unit == 0 { format!("{} B", value) } else { format!("{:.1} {}", v, UNITS[unit]) }
}

fn format_duration(seconds: Option<f64>) -> String {
    match seconds {
        Some(s) if s.is_finite() && s >= 0.0 => {
            let total = s.round() as u64;
            let (h, m, sec) = (total / 3600, (total % 3600) / 60, total % 60);
            if h > 0 { format!("{h}:{m:02}:{sec:02}") } else { format!("{m}:{sec:02}") }
        }
        _ => "—".into(),
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Folders dropped anywhere on the window are added to the search list.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect()
        });
        for path in dropped {
            let folder = if path.is_dir() { path } else { path.parent().map(|p| p.to_path_buf()).unwrap_or(path) };
            if !self.folders.contains(&folder) {
                self.folders.push(folder);
            }
        }

        if self.scanning {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            if let Some(rx) = &self.incoming {
                if let Ok(result) = rx.try_recv() {
                    self.result = Some(result);
                    self.scanning = false;
                    self.incoming = None;
                    if let Some(r) = &self.result {
                        // Open everything when the result is small enough to
                        // take in at a glance; only fall back to the first few
                        // when a large library would unfold into thousands of rows.
                        self.expanded = if r.groups.len() <= 20 {
                            (0..r.groups.len()).collect()
                        } else {
                            (0..3).collect()
                        };
                    }
                    self.apply_keep_rule();
                }
            }
        }

        self.draw(ctx);
    }
}

impl App {
    /// Builds one frame of the interface. Separated from `eframe::App::update`
    /// so it can be exercised headlessly in tests, on any platform, without a
    /// window server.
    pub fn draw(&mut self, ctx: &egui::Context) {
        self.sidebar(ctx);
        self.results(ctx);
        self.error_dialog(ctx);
    }

    /// Waits for a scan started by `with_folders` to finish. Test helper.
    pub fn wait_for_scan(&mut self, ctx: &egui::Context, timeout: std::time::Duration) {
        let start = std::time::Instant::now();
        while self.scanning && start.elapsed() < timeout {
            if let Some(rx) = &self.incoming {
                if let Ok(result) = rx.recv_timeout(std::time::Duration::from_millis(200)) {
                    self.result = Some(result);
                    self.scanning = false;
                    self.incoming = None;
                    if let Some(r) = &self.result {
                        self.expanded = (0..r.groups.len()).collect();
                    }
                    self.apply_keep_rule();
                }
            }
            let _ = ctx;
        }
    }

    pub fn marked_paths(&self) -> &HashSet<PathBuf> {
        &self.marked
    }

    pub fn group_count(&self) -> usize {
        self.result.as_ref().map(|r| r.groups.len()).unwrap_or(0)
    }
}

impl App {
    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("sidebar").exact_width(320.0).resizable(false).show(ctx, |ui| {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("♪").size(22.0).color(ui.visuals().hyperlink_color));
                ui.vertical(|ui| {
                    ui.label(RichText::new("DupliDetect").strong().size(16.0));
                    ui.label(RichText::new("Find duplicate audio").weak().size(11.0));
                });
            });
            ui.add_space(10.0);
            ui.separator();

            ui.horizontal(|ui| {
                ui.label(RichText::new("FOLDERS").strong().size(10.0).weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("+").on_hover_text("Add a folder to search").clicked() {
                        self.choose_folders();
                    }
                    if !self.folders.is_empty() && ui.small_button("Remove All").clicked() {
                        self.folders.clear();
                    }
                });
            });

            if self.folders.is_empty() {
                ui.add_space(14.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Drag folders here").weak());
                    ui.label(RichText::new("or").weak().size(10.0));
                    if ui.button("Choose Folder…").clicked() {
                        self.choose_folders();
                    }
                });
                ui.add_space(14.0);
            } else {
                let mut remove = None;
                egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                    for (i, folder) in self.folders.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.small_button("✕").on_hover_text("Stop searching this folder").clicked() {
                                remove = Some(i);
                            }
                            ui.label(
                                folder.file_name().map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_else(|| folder.display().to_string()),
                            )
                            .on_hover_text(folder.display().to_string());
                        });
                    }
                });
                if let Some(i) = remove {
                    self.folders.remove(i);
                }
            }

            ui.separator();
            egui::ScrollArea::vertical().id_salt("options").show(ui, |ui| {
                ui.label(RichText::new("WHAT TO LOOK FOR").strong().size(10.0).weak());
                ui.checkbox(&mut self.options.find_exact_duplicates, "Identical files");
                ui.checkbox(&mut self.options.find_similar_audio, "Same audio in any format");
                if self.options.find_similar_audio {
                    ui.add_space(4.0);
                    ui.label("Match strictness");
                    egui::ComboBox::from_id_salt("level")
                        .selected_text(self.options.level.label())
                        .width(280.0)
                        .show_ui(ui, |ui| {
                            for level in MatchLevel::ALL {
                                ui.selectable_value(&mut self.options.level, level, level.label());
                            }
                        });
                    ui.label(RichText::new(self.options.level.explanation()).weak().size(10.0));
                }

                ui.add_space(8.0);
                ui.label(RichText::new("WHERE TO LOOK").strong().size(10.0).weak());
                ui.checkbox(&mut self.options.include_subfolders, "Include subfolders");
                ui.checkbox(&mut self.options.skip_hidden_files, "Skip hidden files");
                ui.horizontal(|ui| {
                    ui.label("Ignore clips under");
                    ui.add(egui::DragValue::new(&mut self.options.minimum_duration)
                        .range(0.0..=60.0).suffix(" s").speed(1.0));
                });

                ui.add_space(8.0);
                ui.label(RichText::new("WHICH COPY TO KEEP").strong().size(10.0).weak());
                let mut changed = false;
                egui::ComboBox::from_id_salt("keep")
                    .selected_text(self.keep_rule.label())
                    .width(280.0)
                    .show_ui(ui, |ui| {
                        for rule in KeepRule::ALL {
                            if ui.selectable_value(&mut self.keep_rule, rule, rule.label()).clicked() {
                                changed = true;
                            }
                        }
                    });
                ui.label(RichText::new(self.keep_rule.explanation()).weak().size(10.0));
                if changed {
                    self.apply_keep_rule();
                }
            });

            egui::TopBottomPanel::bottom("scan").show_inside(ui, |ui| {
                ui.add_space(6.0);
                if self.scanning {
                    let phase = *self.phase.lock().unwrap();
                    if let Some(f) = phase_fraction(phase) {
                        ui.add(egui::ProgressBar::new(f as f32).desired_width(280.0));
                    } else {
                        ui.spinner();
                    }
                    ui.label(RichText::new(phase_text(phase)).weak().size(11.0));
                } else {
                    let enabled = !self.folders.is_empty();
                    if ui.add_enabled(enabled, egui::Button::new("  Scan for Duplicates  "))
                        .clicked()
                    {
                        self.start_scan();
                    }
                    if !enabled {
                        ui.label(RichText::new("Add at least one folder to begin.").weak().size(10.0));
                    }
                }
                ui.add_space(6.0);
            });
        });
    }

    fn results(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("actions").show(ctx, |ui| {
            if self.result.as_ref().map(|r| r.groups.is_empty()).unwrap_or(true) {
                return;
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let mut changed = false;
                egui::ComboBox::from_id_salt("keep2")
                    .selected_text(format!("Keep {}", self.keep_rule.label().to_lowercase()))
                    .show_ui(ui, |ui| {
                        for rule in KeepRule::ALL {
                            if ui.selectable_value(&mut self.keep_rule, rule, rule.label()).clicked() {
                                changed = true;
                            }
                        }
                    });
                if changed {
                    self.apply_keep_rule();
                }
                if ui.add_enabled(!self.marked.is_empty(), egui::Button::new("Clear Selection")).clicked() {
                    self.marked.clear();
                }
                let total = self.result.as_ref().map(|r| r.groups.len()).unwrap_or(0);
                let all_open = self.expanded.len() >= total && total > 0;
                if ui.button(if all_open { "Collapse All" } else { "Expand All" }).clicked() {
                    if all_open {
                        self.expanded.clear();
                    } else {
                        self.expanded = (0..total).collect();
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let marked = self.marked_files();
                    let bytes: u64 = marked.iter().map(|f| f.byte_size).sum();
                    if ui.add_enabled(!marked.is_empty(),
                                      egui::Button::new(RichText::new("🗑 Move to Trash")))
                        .clicked()
                    {
                        self.trash_marked();
                    }
                    if marked.is_empty() {
                        ui.label(RichText::new("Nothing selected").weak());
                    } else {
                        ui.label(RichText::new(format!("{} selected · {}", marked.len(), format_bytes(bytes))).weak());
                    }
                });
            });
            ui.add_space(6.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(result) = self.result.take() else {
                self.empty_state(ui);
                return;
            };
            if result.groups.is_empty() {
                self.result = Some(result);
                self.empty_state(ui);
                return;
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let files: usize = result.groups.iter().map(|g| g.files.len()).sum();
                ui.label(RichText::new(result.groups.len().to_string()).strong().size(17.0));
                ui.label(RichText::new(if result.groups.len() == 1 { "group" } else { "groups" }).weak());
                ui.add_space(10.0);
                ui.label(RichText::new(files.to_string()).strong().size(17.0));
                ui.label(RichText::new("files").weak());
                ui.add_space(10.0);
                ui.label(RichText::new(format_bytes(result.reclaimable_bytes())).strong().size(17.0)
                    .color(ui.visuals().hyperlink_color));
                ui.label(RichText::new("recoverable").weak());
                if !result.files_skipped.is_empty() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let tip = result.files_skipped.iter()
                            .map(|(p, r)| format!("{}: {r}", p.file_name().unwrap().to_string_lossy()))
                            .collect::<Vec<_>>().join("\n");
                        ui.label(RichText::new(format!("⚠ {} skipped", result.files_skipped.len())).weak())
                            .on_hover_text(tip);
                    });
                }
            });
            ui.separator();

            let mut actions: Vec<(usize, AudioFile, Action)> = Vec::new();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (gi, group) in result.groups.iter().enumerate() {
                    let open = self.expanded.contains(&gi);
                    let header = ui.horizontal(|ui| {
                        let arrow = if open { "▾" } else { "▸" };
                        let badge = match group.kind {
                            MatchKind::Exact => RichText::new(" Identical ")
                                .color(Color32::from_rgb(40, 140, 70)).size(11.0),
                            MatchKind::Similar => RichText::new(format!(" {}% match ",
                                (group.confidence * 100.0).round() as i64))
                                .color(Color32::from_rgb(190, 120, 30)).size(11.0),
                        };
                        let clicked = ui.selectable_label(false, format!("{arrow}  ")).clicked();
                        ui.label(badge);
                        let name = group.files.first().map(|f| f.display_name()).unwrap_or_default();
                        ui.label(RichText::new(name).strong());
                        ui.label(RichText::new(format!("+{} more", group.files.len() - 1)).weak().size(11.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new("recoverable").weak().size(11.0));
                            ui.label(RichText::new(format_bytes(group.reclaimable_bytes()))
                                .color(ui.visuals().hyperlink_color));
                        });
                        clicked
                    });
                    if header.inner || header.response.clicked() {
                        if open { self.expanded.remove(&gi); } else { self.expanded.insert(gi); }
                    }

                    if open {
                        for file in &group.files {
                            let marked = self.marked.contains(&file.path);
                            ui.horizontal(|ui| {
                                ui.add_space(14.0);
                                let mark_label = if marked {
                                    RichText::new("🗑").color(Color32::from_rgb(190, 60, 60))
                                } else {
                                    RichText::new("✔").color(Color32::from_rgb(40, 140, 70))
                                };
                                if ui.button(mark_label)
                                    .on_hover_text(if marked { "Marked for the Trash — click to keep" }
                                                   else { "Being kept — click to mark for the Trash" })
                                    .clicked()
                                {
                                    actions.push((gi, file.clone(), Action::ToggleMark));
                                }
                                let playing = self.preview.playing() == Some(file.path.as_path());
                                if ui.button(if playing { "■" } else { "▶" })
                                    .on_hover_text("Preview the first 30 seconds").clicked()
                                {
                                    actions.push((gi, file.clone(), Action::Preview));
                                }
                                let name = if marked {
                                    RichText::new(file.display_name()).weak()
                                } else {
                                    RichText::new(file.display_name())
                                };
                                ui.label(name).on_hover_text(file.path.display().to_string());
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("→").on_hover_text("Reveal in file manager").clicked() {
                                        actions.push((gi, file.clone(), Action::Reveal));
                                    }
                                    ui.label(RichText::new(format_bytes(file.byte_size)).weak().size(11.0));
                                    ui.label(RichText::new(format_duration(file.duration)).weak().size(11.0));
                                    ui.label(RichText::new(file.quality_summary()).weak().size(11.0));
                                    ui.label(RichText::new(file.format_name.clone().unwrap_or_default())
                                        .weak().size(11.0));
                                });
                            });
                        }
                        ui.separator();
                    }
                }
            });
            self.result = Some(result);

            for (gi, file, action) in actions {
                match action {
                    Action::ToggleMark => self.toggle_mark(&file, gi),
                    Action::Preview => self.preview.toggle(&file.path),
                    Action::Reveal => platform::reveal(&file.path),
                }
            }
        });
    }

    fn empty_state(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(120.0);
            if self.scanning {
                ui.heading("Scanning…");
                ui.label(RichText::new(phase_text(*self.phase.lock().unwrap())).weak());
            } else if self.trashed_count > 0 {
                ui.heading("All cleaned up");
                ui.label(RichText::new(format!(
                    "Moved {} file(s) to the Trash, freeing {}. Nothing was deleted permanently — \
                     you can put anything back from the Trash.",
                    self.trashed_count, format_bytes(self.trashed_bytes)
                )).weak());
            } else if let Some(result) = &self.result {
                if result.files_scanned == 0 {
                    ui.heading("No audio files found");
                    ui.label(RichText::new("Nothing in those folders looked like audio.").weak());
                } else {
                    ui.heading("No duplicates found");
                    ui.label(RichText::new(format!(
                        "Checked {} audio files and every one of them is unique.",
                        result.files_scanned
                    )).weak());
                }
            } else {
                ui.heading("No scan yet");
                ui.label(RichText::new(
                    "Add the folders you want to search, then click Scan for Duplicates.").weak());
            }
        });
    }

    fn error_dialog(&mut self, ctx: &egui::Context) {
        let message = self.error.clone().or_else(|| self.preview.last_error.clone());
        let Some(message) = message else { return };
        let mut open = true;
        egui::Window::new("Something went wrong")
            .collapsible(false).resizable(false).open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(message);
                if ui.button("OK").clicked() {
                    self.error = None;
                    self.preview.last_error = None;
                }
            });
        if !open {
            self.error = None;
            self.preview.last_error = None;
        }
    }
}

enum Action {
    ToggleMark,
    Preview,
    Reveal,
}



fn phase_text(phase: Phase) -> String {
    match phase {
        Phase::Collecting => "Looking for audio files…".into(),
        Phase::Hashing { done, total } => format!("Checking for identical files… {done} of {total}"),
        Phase::Fingerprinting { done, total } => format!("Listening to audio… {done} of {total}"),
        Phase::Matching => "Comparing fingerprints…".into(),
        Phase::Finished => "Done".into(),
    }
}

fn phase_fraction(phase: Phase) -> Option<f64> {
    match phase {
        Phase::Hashing { done, total } | Phase::Fingerprinting { done, total } if total > 0 => {
            Some(done as f64 / total as f64)
        }
        _ => None,
    }
}
