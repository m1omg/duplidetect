//! The handful of operations that genuinely differ per operating system.

use std::path::Path;

/// Moves a file to the platform's trash.
///
/// The app's central promise is that nothing is ever deleted permanently, so a
/// failure here must surface as an error and leave the file alone. There is
/// deliberately no fallback to an ordinary delete.
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|e| e.to_string())
}

/// Shows a file in the system file manager.
pub fn reveal(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // The file-manager D-Bus interface selects the file itself; falling
        // back to opening the folder is better than doing nothing.
        let uri = format!("file://{}", path.display());
        let dbus = std::process::Command::new("dbus-send")
            .args([
                "--session",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                &format!("array:string:{uri}"),
                "string:",
            ])
            .status();
        if !matches!(dbus, Ok(s) if s.success()) {
            if let Some(parent) = path.parent() {
                let _ = std::process::Command::new("xdg-open").arg(parent).spawn();
            }
        }
    }
}
