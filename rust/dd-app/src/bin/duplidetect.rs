// No console window on Windows; ddcli is the console-subsystem counterpart.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

/// Writes panics to stderr and to a file, so a crash on a machine the
/// developer cannot reach still leaves something to read. A GUI app launched
/// from a desktop icon has nowhere to print, which is how a crash becomes
/// "it just disappeared".
fn install_panic_reporter() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let report = format!(
            "DupliDetect {} panicked\n{}\n",
            env!("CARGO_PKG_VERSION"),
            info
        );
        eprint!("{report}");
        let path = std::env::temp_dir().join("duplidetect-crash.txt");
        let _ = std::fs::write(&path, &report);
        eprintln!("(also written to {})", path.display());
        previous(info);
    }));
}

fn main() -> eframe::Result<()> {
    install_panic_reporter();

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
