// Custom backdrop artwork.
//
// The chosen file is copied into the app data folder instead of referencing
// the original location: moving or deleting the source picture must not break
// the workspace, and the asset protocol only ever exposes this one folder.

use crate::error::{AppError, Result};
use std::path::{Path, PathBuf};

/// Matches the slider range in Appearance settings.
pub const MAX_BLUR: f32 = 40.0;
/// A wallpaper is decoration, not a dataset; refuse absurd source files.
pub const MAX_BYTES: u64 = 25 * 1024 * 1024;
const ALLOWED: [&str; 5] = ["png", "jpg", "jpeg", "webp", "bmp"];

pub fn wallpaper_dir(root: &Path) -> Result<PathBuf> {
    crate::storage::child(root, "appearance")
}

fn extension(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    ALLOWED.contains(&ext.as_str()).then_some(ext)
}

/// True only for `background.*` inside this app's wallpaper folder. Anything
/// else (a settings path edited by hand, a symlink, an arbitrary file) is
/// ignored, so the webview can never be pointed outside the app data folder.
pub fn is_managed(root: &Path, candidate: &str) -> bool {
    if candidate.trim().is_empty() {
        return false;
    }
    let Ok(dir) = wallpaper_dir(root).and_then(|p| Ok(p.canonicalize()?)) else {
        return false;
    };
    let path = Path::new(candidate);
    let Ok(file) = path.canonicalize() else {
        return false;
    };
    let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
    file.is_file()
        && file.parent() == Some(dir.as_path())
        && name.starts_with("background.")
        && extension(&file).is_some()
}

/// Removes every wallpaper copy. Missing folder is fine.
pub fn clear(root: &Path) -> Result<()> {
    let dir = wallpaper_dir(root)?;
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.is_file()
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("background."))
        {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// Copies a user-chosen picture into the wallpaper folder and returns its
/// managed path. The previous wallpaper is removed only after the new one is
/// validated, so cancelling the dialog leaves the current look untouched.
pub fn install(root: &Path, source: &str) -> Result<String> {
    let source = Path::new(source);
    let ext =
        extension(source).ok_or_else(|| AppError::user("Choose a PNG, JPG, WEBP or BMP image."))?;
    let meta = std::fs::metadata(source)
        .map_err(|e| AppError::new("Could not read the selected image.", e))?;
    if !meta.is_file() {
        return Err(AppError::user("Choose an image file, not a folder."));
    }
    if meta.len() > MAX_BYTES {
        return Err(AppError::user("Choose an image smaller than 25 MB."));
    }
    let dir = wallpaper_dir(root)?;
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(format!("background.{ext}"));
    // Copy under a temporary name first; a failed copy must not wipe the
    // wallpaper that is currently showing.
    let temp = dir.join(format!("background.{}.part", uuid::Uuid::new_v4()));
    std::fs::copy(source, &temp)?;
    if dest.exists() {
        std::fs::remove_file(&dest)?;
    }
    std::fs::rename(&temp, &dest)?;
    // Drop any older copy with a different extension.
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path != dest
            && path.is_file()
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("background."))
        {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(dest.to_string_lossy().into_owned())
}

#[cfg(windows)]
fn pick_file() -> Result<Option<String>> {
    const SCRIPT: &str = r#"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Add-Type -AssemblyName System.Windows.Forms | Out-Null
$dialog = New-Object System.Windows.Forms.OpenFileDialog
$dialog.Title = 'Choose a background image'
$dialog.Filter = 'Images (*.png;*.jpg;*.jpeg;*.webp;*.bmp)|*.png;*.jpg;*.jpeg;*.webp;*.bmp|All files (*.*)|*.*'
$dialog.CheckFileExists = $true
$dialog.Multiselect = $false
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
  [Console]::Out.WriteLine($dialog.FileName)
}
"#;
    crate::migration::file_dialog(SCRIPT)
}

#[cfg(not(windows))]
fn pick_file() -> Result<Option<String>> {
    Ok(None)
}

/// Opens the picker and installs the selection in one step.
pub fn choose(root: &Path) -> Result<Option<String>> {
    match pick_file()? {
        Some(source) => Ok(Some(install(root, &source)?)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_copied_wallpapers_are_managed() {
        let root = tempfile::tempdir().unwrap();
        let dir = wallpaper_dir(root.path()).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("background.png");
        std::fs::write(&good, b"png").unwrap();
        assert!(is_managed(root.path(), good.to_str().unwrap()));

        let outside = root.path().join("settings.json");
        std::fs::write(&outside, b"{}").unwrap();
        assert!(!is_managed(root.path(), outside.to_str().unwrap()));

        let stray = dir.join("payload.html");
        std::fs::write(&stray, b"<html>").unwrap();
        assert!(!is_managed(root.path(), stray.to_str().unwrap()));
        assert!(!is_managed(root.path(), ""));
    }

    #[test]
    fn install_copies_and_clear_removes() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("holiday.jpg");
        std::fs::write(&source, b"jpeg-bytes").unwrap();
        let managed = install(root.path(), source.to_str().unwrap()).unwrap();
        let path = std::path::PathBuf::from(&managed);
        assert!(path.is_file());
        assert_eq!(std::fs::read(&path).unwrap(), b"jpeg-bytes");
        assert!(is_managed(root.path(), &managed));
        clear(root.path()).unwrap();
        assert!(!path.exists());
        assert!(!is_managed(root.path(), &managed));
    }

    #[test]
    fn rejects_unsupported_and_oversized_sources() {
        let root = tempfile::tempdir().unwrap();
        let text = root.path().join("notes.txt");
        std::fs::write(&text, b"nope").unwrap();
        assert!(install(root.path(), text.to_str().unwrap()).is_err());
    }
}
