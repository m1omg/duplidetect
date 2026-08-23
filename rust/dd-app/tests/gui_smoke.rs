//! Renders the interface headlessly and reads the text it produced.
//!
//! This is the GUI equivalent of the algorithm parity tests: it proves the
//! widget tree is actually built with real content on whatever platform it runs
//! on, without needing a window server, a screenshot, or a human.

use dd_app::app::App;
use std::path::PathBuf;
use std::time::Duration;

/// Every string egui laid out during one frame.
fn rendered_text(ctx: &egui::Context, app: &mut App) -> Vec<String> {
    let output = ctx.run(Default::default(), |ctx| app.draw(ctx));
    let mut found = Vec::new();
    for shape in output.shapes {
        collect(&shape.shape, &mut found);
    }
    found
}

fn collect(shape: &egui::epaint::Shape, out: &mut Vec<String>) {
    match shape {
        egui::epaint::Shape::Text(t) => out.push(t.galley.text().to_string()),
        egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
        _ => {}
    }
}

fn fixtures() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/fixtures"))
}

/// Guards the Windows bug where every component of an absolute path was tested
/// for hiddenness, and the volume root's hidden attribute excluded every file.
#[test]
fn absolute_paths_are_scanned() {
    let options = dd_app::scanner::ScanOptions::default();
    let found = dd_app::scanner::collect(&[fixtures()], &options);
    assert!(found.len() >= 17, "expected the fixtures, found {}", found.len());
    assert!(found.iter().all(|f| f.path.is_absolute()));
}

#[test]
fn empty_state_renders() {
    let ctx = egui::Context::default();
    let mut app = App::default();
    // Two frames: egui settles layout on the second.
    let _ = rendered_text(&ctx, &mut app);
    let text = rendered_text(&ctx, &mut app).join("\n");

    for expected in ["DupliDetect", "FOLDERS", "Drag folders here", "Choose Folder",
                     "Scan for Duplicates", "No scan yet", "WHAT TO LOOK FOR",
                     "Identical files", "Same audio in any format", "WHICH COPY TO KEEP"] {
        assert!(text.contains(expected), "empty state is missing {expected:?}\n---\n{text}");
    }
}

#[test]
fn results_render_with_groups_and_keepers() {
    let ctx = egui::Context::default();
    let mut app = App::with_folders(vec![fixtures()]);
    app.wait_for_scan(&ctx, Duration::from_secs(120));

    let _ = rendered_text(&ctx, &mut app);
    let text = rendered_text(&ctx, &mut app).join("\n");

    assert_eq!(app.group_count(), 4, "expected the four fixture groups");
    for expected in ["groups", "files", "recoverable", "Move to Trash", "Clear Selection",
                     "musicA.flac", "musicA.wav", "toneA.wav", "44.1 kHz", "kbps"] {
        assert!(text.contains(expected), "results are missing {expected:?}\n---\n{text}");
    }

    // The keep rule must have marked everything except one file per group.
    let total: usize = 4;
    assert_eq!(app.marked_paths().len(), 17 - total,
               "one file per group should be kept and the rest marked");
}
