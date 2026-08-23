// No console window on Windows; ddcli is the console-subsystem counterpart.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    // Folders given on the command line are scanned on launch.
    let folders: Vec<PathBuf> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 640.0])
            .with_min_inner_size([860.0, 520.0])
            .with_title("DupliDetect")
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        "DupliDetect",
        options,
        Box::new(move |_cc| Ok(Box::new(dd_app::app::App::with_folders(folders)))),
    )
}
