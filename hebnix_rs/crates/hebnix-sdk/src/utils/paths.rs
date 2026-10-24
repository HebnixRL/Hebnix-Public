//! Shared Hebnix runtime and data paths.

use std::path::PathBuf;

/// Windows' Documents known folder includes OneDrive and other folder redirects.
/// Use the traditional location only when the known folder is unavailable.
pub fn documents_dir() -> Option<PathBuf> {
    dirs::document_dir()
        .filter(|path| path.is_absolute())
        .or_else(|| dirs::home_dir().map(|home| home.join("Documents")))
        .filter(|path| path.is_absolute())
}

/// The root for all Hebnix-owned files.
///
/// On Windows this is `%AppData%\Hebnix`. `HEBNIX_BASE_DIR` remains available
/// for isolated development and test runs.
pub fn base_dir() -> PathBuf {
    let dir = std::env::var_os("HEBNIX_BASE_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(portable_base_dir)
        .or_else(|| dirs::data_dir().map(|dir| dir.join("Hebnix")))
        .unwrap_or_else(|| std::env::temp_dir().join("Hebnix"));

    if let Err(error) = std::fs::create_dir_all(&dir) {
        eprintln!(
            "failed to create Hebnix data directory {}: {error}",
            dir.display()
        );
    }
    dir
}

/// portable.txt next to the exe keeps Hebnix data in the exe folder, unless
/// it has a `portable=false` line (then it only holds options).
fn portable_base_dir() -> Option<PathBuf> {
    let path = portable_txt()?;
    if !path.exists() || portable_value("portable").is_some_and(|value| !truthy(&value)) {
        return None;
    }
    path.parent().map(PathBuf::from)
}

fn portable_txt() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join("portable.txt"))
}

/// the value of `name` in portable.txt: `name=value`, or "true" for a bare
/// `name` line (case-insensitive). Other lines are ignored, so the file can
/// keep its note.
fn portable_value(name: &str) -> Option<String> {
    let contents = std::fs::read_to_string(portable_txt()?).ok()?;
    contents.lines().find_map(|line| {
        let line = line.trim().trim_start_matches('\u{feff}');
        match line.split_once('=') {
            Some((key, value)) if key.trim().eq_ignore_ascii_case(name) => {
                Some(value.trim().to_string())
            }
            None if line.eq_ignore_ascii_case(name) => Some("true".to_string()),
            _ => None,
        }
    })
}

fn truthy(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "true" | "1" | "yes")
}

/// true if portable.txt next to the exe has a line `name` or `name=true`.
pub fn portable_option(name: &str) -> bool {
    portable_value(name).is_some_and(|value| truthy(&value))
}
