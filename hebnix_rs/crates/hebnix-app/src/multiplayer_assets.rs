//! Multiplayer support programs embedded only in the full Hebnix binary.

use std::io::Read;
use std::path::Path;

// Generated recursively by build.rs from assets/multiplayer: (path, raw length,
// deflated bytes) plus a checksum of the whole bundle.
include!(concat!(env!("OUT_DIR"), "/multiplayer_assets.rs"));

/// Extracts the embedded bundle to `%AppData%\Hebnix\multiplayer-lan`.
/// Existing files are updated only when their bundled contents changed.
pub fn ensure_present(base_dir: &Path) -> std::io::Result<()> {
    let multiplayer_dir = base_dir.join("multiplayer-lan");
    let stamp = multiplayer_dir.join(".bundle");
    // same bundle and every file the right size: skip inflating ~27MB at startup
    let intact = std::fs::read_to_string(&stamp).is_ok_and(|saved| saved == MULTIPLAYER_STAMP)
        && MULTIPLAYER_ASSETS.iter().all(|&(relative_path, length, _)| {
            std::fs::metadata(multiplayer_dir.join(relative_path)).is_ok_and(|meta| meta.len() == length)
        });
    if intact {
        return Ok(());
    }
    for &(relative_path, length, packed) in MULTIPLAYER_ASSETS {
        let mut bytes = Vec::with_capacity(length as usize);
        flate2::read::DeflateDecoder::new(packed).read_to_end(&mut bytes)?;
        crate::runtime_assets::write_if_changed(&multiplayer_dir.join(relative_path), &bytes)?;
    }
    std::fs::write(stamp, MULTIPLAYER_STAMP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_assets_inflate_to_their_recorded_length() {
        for &(relative_path, length, packed) in MULTIPLAYER_ASSETS {
            let mut bytes = Vec::new();
            flate2::read::DeflateDecoder::new(packed).read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes.len() as u64, length, "{relative_path}");
        }
    }
}
