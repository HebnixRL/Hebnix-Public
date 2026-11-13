use super::service;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub(super) const LIMIT: usize = 9;

pub(super) fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}
pub(super) fn path(root: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("Invalid saved picture.".into());
    }
    Ok(root.join("gallery").join(format!("{id}.jpg")))
}
pub(super) fn import(root: &Path, source: &Path) -> Result<String, String> {
    let mut bytes = Vec::new();
    fs::File::open(source)
        .map_err(|e| e.to_string())?
        .take(6 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let jpeg = service::normalize_image(&bytes)?;
    store_jpeg(root, &jpeg)
}
pub(super) fn store_jpeg(root: &Path, jpeg: &[u8]) -> Result<String, String> {
    let id = hex::encode(Sha256::digest(jpeg));
    let target = path(root, &id)?;
    fs::create_dir_all(target.parent().ok_or("Invalid gallery path")?)
        .map_err(|e| e.to_string())?;
    fs::write(target, jpeg).map_err(|e| e.to_string())?;
    Ok(id)
}
pub(super) fn read(root: &Path, id: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    fs::File::open(path(root, id)?)
        .map_err(|e| e.to_string())?
        .take(6 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 6 * 1024 * 1024 || hex::encode(Sha256::digest(&bytes)) != id {
        return Err("This saved picture changed. Please import it again.".into());
    }
    Ok(bytes)
}
pub(super) fn remember(history: &mut Vec<String>, id: String) {
    history.retain(|old| old != &id && valid_id(old));
    history.insert(0, id);
    history.truncate(LIMIT);
}
pub(super) fn prune(root: &Path, keep: &[String]) {
    if let Ok(entries) = fs::read_dir(root.join("gallery")) {
        for entry in entries.flatten() {
            let file = entry.path();
            let Some(id) = file.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if file.extension().is_some_and(|s| s == "jpg")
                && valid_id(id)
                && !keep.iter().any(|s| s == id)
            {
                let _ = fs::remove_file(file);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gallery_is_bounded_deduplicated_and_keeps_250px_files() {
        let root = std::env::temp_dir().join(format!("hebnix-gallery-{}", rand::random::<u64>()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.png");
        let mut history = Vec::new();
        for color in 0..10 {
            image::RgbImage::from_pixel(40, 20, image::Rgb([color * 20, 128, 0]))
                .save(&source)
                .unwrap();
            remember(&mut history, import(&root, &source).unwrap());
        }
        assert_eq!(history.len(), 9);
        let last = history[2].clone();
        remember(&mut history, last.clone());
        assert_eq!(history[0], last);
        assert_eq!(history.len(), 9);
        prune(&root, &history);
        assert_eq!(fs::read_dir(root.join("gallery")).unwrap().count(), 9);
        for id in history {
            let picture = image::open(path(&root, &id).unwrap()).unwrap().to_rgb8();
            assert_eq!(picture.dimensions(), (250, 250));
            assert!(picture.get_pixel(0, 0).0.iter().all(|v| *v < 10));
        }
        assert!(path(&root, "../secret").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
