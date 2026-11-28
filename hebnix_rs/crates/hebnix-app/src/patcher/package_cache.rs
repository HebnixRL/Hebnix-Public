//! Optional cache of previously validated, exact-input package transformations.
//! Recovery receipts and game-closed checks remain owned by each caller.
use sha2::{Digest, Sha256};
use std::{fs, path::{Path, PathBuf}};
#[derive(serde::Serialize, serde::Deserialize)]
struct Entry { output: String }
fn digest(bytes: &[u8]) -> String { hex::encode(Sha256::digest(bytes)) }
fn valid(value: &str) -> bool { value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()) }
fn directory() -> PathBuf { crate::config::base_dir().join("state/package-cache") }
fn key(scope: &str, input: &str) -> String { digest(format!("v1\n{scope}\n{input}").as_bytes()) }
pub(super) fn get(scope: &str, input: &str) -> Option<Vec<u8>> {
    if !valid(input) { return None; }
    read_cached(&directory(), &key(scope, input))
}
fn read_cached(folder: &Path, key: &str) -> Option<Vec<u8>> {
    let entry: Entry = serde_json::from_slice(&fs::read(folder.join(format!("{key}.json"))).ok()?).ok()?;
    if !valid(&entry.output) { return None; }
    let path = folder.join(format!("{}.upk", entry.output));
    if fs::metadata(&path).ok()?.len() > 512 * 1024 * 1024 { return None; }
    let bytes = fs::read(path).ok()?;
    (digest(&bytes) == entry.output).then_some(bytes)
}
pub(super) fn remember(scope: &str, input: &str, bytes: &[u8]) {
    let save = || -> Result<(), String> {
        if !valid(input) { return Err("Invalid package cache input digest".into()); }
        let folder = directory();
        fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let output = digest(bytes);
        let payload = folder.join(format!("{output}.upk"));
        // Replacing the payload also repairs a previously truncated cache file.
        super::upk_history::replace(&payload, bytes)?;
        let manifest = serde_json::to_vec(&Entry { output }).map_err(|e| e.to_string())?;
        super::upk_history::replace(&folder.join(format!("{}.json", key(scope, input))), &manifest)
    };
    if let Err(error) = save() { tracing::debug!(%error, "optional package cache was not saved"); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corrupt_or_unrelated_cached_package_never_becomes_a_fast_path() {
        let temp = std::env::temp_dir();
        let folder = temp.join(format!("hebnix-cache-check-{}", rand::random::<u64>()));
        fs::create_dir_all(&folder).unwrap();
        let input = digest(b"input package");
        let key = key("avatar-v1", &input);
        let bytes = b"verified output package";
        let output = digest(bytes);
        let payload = folder.join(format!("{output}.upk"));
        fs::write(&payload, bytes).unwrap();
        fs::write(folder.join(format!("{key}.json")), serde_json::to_vec(&Entry { output }).unwrap()).unwrap();
        assert_eq!(read_cached(&folder, &key), Some(bytes.to_vec()));
        assert!(read_cached(&folder, &super::key("menu-v1", &input)).is_none());
        assert!(read_cached(&folder, &super::key("avatar-v1", &digest(b"other package"))).is_none());
        fs::write(&payload, b"truncated").unwrap();
        assert!(read_cached(&folder, &key).is_none());
        assert!(folder.canonicalize().unwrap().starts_with(temp.canonicalize().unwrap()));
        fs::remove_dir_all(folder).unwrap();
    }
}
