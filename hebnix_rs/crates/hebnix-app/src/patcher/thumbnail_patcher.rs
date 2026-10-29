//! Persistent, reversible image replacements in thumbnail UPKs. Shared packages
//! are rebuilt from one snapshot so restoring a decal retains its neighbours.
use super::{backup_guard, cosmetic_thumbnail, upk_package::UpkPackage};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{DynamicImage, RgbaImage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

const EDGE: u32 = 512;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Patch {
    pub export: usize,
    pub name: String,
    pub kind: String,
    png: String,
}

impl Patch {
    pub fn png(&self) -> Result<Vec<u8>, String> {
        STANDARD.decode(&self.png).map_err(|e| e.to_string())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Package {
    pub file: String,
    original_hash: String,
    patched_hash: String,
    pub patches: Vec<Patch>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub packages: Vec<Package>,
}

impl Manifest {
    pub fn count(&self) -> usize {
        self.packages.iter().map(|p| p.patches.len()).sum()
    }
}

pub(crate) fn directory(backups: &Path) -> std::path::PathBuf {
    backups.join("ThumbnailPatcher")
}

/// Experimental GFx menu textures keep their own manifest and snapshots, so
/// restoring thumbnails never touches menu packages and vice versa.
pub(crate) fn menu_directory(backups: &Path) -> std::path::PathBuf {
    backups.join("MenuTexturePatcher")
}

fn valid_file(file: &str) -> bool {
    !file.contains(['/', '\\', ':']) && file.to_ascii_lowercase().ends_with(".upk")
}

pub(crate) fn load(backups: &Path) -> Result<Manifest, String> {
    load_from(&directory(backups))
}

pub(crate) fn menu_load(backups: &Path) -> Result<Manifest, String> {
    load_from(&menu_directory(backups))
}

fn load_from(store: &Path) -> Result<Manifest, String> {
    let path = store.join("patches.json");
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Manifest::default()),
        Err(e) => return Err(format!("Could not read thumbnail patches: {e}")),
    };
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Could not read thumbnail patches: {e}"))?;
    if manifest.packages.iter().any(|p| !valid_file(&p.file)) {
        return Err("Invalid thumbnail package filename".into());
    }
    Ok(manifest)
}

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Fit without stretching or cropping; centre the result on a transparent square.
pub(crate) fn square(image: DynamicImage) -> RgbaImage {
    let fitted = image.thumbnail(EDGE, EDGE).to_rgba8();
    let mut output = RgbaImage::new(EDGE, EDGE);
    image::imageops::overlay(
        &mut output,
        &fitted,
        ((EDGE - fitted.width()) / 2) as i64,
        ((EDGE - fitted.height()) / 2) as i64,
    );
    output
}

pub(crate) fn import_image(path: &Path) -> Result<Vec<u8>, String> {
    let mut reader = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    cosmetic_thumbnail::preview_png(square(reader.decode().map_err(|e| e.to_string())?))
}

pub(crate) struct MenuTexture {
    pub export: usize,
    pub name: String,
    pub size: (u32, u32),
    pub png: Result<Vec<u8>, String>,
}

/// Every Texture2D in a GFx package, with a decoded preview where possible.
pub(crate) fn menu_textures(path: &Path) -> Result<Vec<MenuTexture>, String> {
    let upk = UpkPackage::load(path)?;
    let directory = path.parent().ok_or("Menu package has no parent directory")?;
    Ok(upk
        .exports
        .iter()
        .enumerate()
        .filter(|(_, e)| upk.class_of(e).starts_with("Texture2D"))
        .map(|(export, e)| {
            let pixels = cosmetic_thumbnail::texture(&upk, e, directory);
            MenuTexture {
                export,
                name: upk.name_of(e.object_name),
                size: pixels.as_ref().map(|p| p.dimensions()).unwrap_or((0, 0)),
                png: pixels.and_then(cosmetic_thumbnail::preview_png),
            }
        })
        .collect())
}

fn replace_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension("thumbnail-pending");
    fs::write(&temporary, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Err(e) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("{}: {e}", path.display()));
    }
    Ok(())
}

fn snapshot(store: &Path, package: &Package) -> Result<Vec<u8>, String> {
    let bytes = fs::read(store.join(format!("{}.bak", package.file)))
        .map_err(|e| format!("Original thumbnail backup is unavailable: {e}"))?;
    if hash(&bytes) != package.original_hash {
        return Err("Original thumbnail backup has changed; restore was cancelled.".into());
    }
    Ok(bytes)
}

fn check_live(bytes: &[u8], package: &Package) -> Result<(), String> {
    if hash(bytes) != package.patched_hash {
        return Err(format!(
            "{} was changed by another patch. Undo that change before editing or restoring its thumbnails.",
            package.file
        ));
    }
    Ok(())
}

fn rebuilt(store: &Path, package: &Package) -> Result<Vec<u8>, String> {
    let original = snapshot(store, package)?;
    if package.patches.is_empty() {
        return Ok(original);
    }
    let path = store.join(format!("{}.bak", package.file));
    let mut upk = UpkPackage::load(&path)?;
    for patch in &package.patches {
        let pixels = image::load_from_memory(&patch.png()?)
            .map_err(|e| e.to_string())?
            .to_rgba8();
        let export = upk
            .exports
            .get(patch.export)
            .ok_or("Thumbnail export is missing")?;
        if !upk.class_of(export).starts_with("Texture2D") {
            return Err("Thumbnail export is not a texture".into());
        }
        let payload = cosmetic_thumbnail::bake_thumbnail(&upk, export, &pixels)?;
        upk.replace_export_payload(patch.export, &payload)?;
    }
    upk.repacked()
}

/// Commit a package and its manifest together, rolling back if state cannot save.
fn commit(
    cooked: &Path,
    store: &Path,
    file: &str,
    before: &[u8],
    after: &[u8],
    manifest: &Manifest,
) -> Result<(), String> {
    let saved = serde_json::to_vec(manifest).map_err(|e| e.to_string())?;
    replace_file(&cooked.join(file), after)?;
    if let Err(error) = replace_file(&store.join("patches.json"), &saved) {
        if let Err(rollback) = replace_file(&cooked.join(file), before) {
            return Err(format!(
                "{error}; rollback failed: {rollback}. The original backup has been retained."
            ));
        }
        return Err(error);
    }
    Ok(())
}

pub(crate) fn apply(
    cooked: &Path,
    backups: &Path,
    file: &str,
    kind: &str,
    name: &str,
    png: &[u8],
) -> Result<Manifest, String> {
    if !valid_file(file) {
        return Err("Invalid thumbnail package filename".into());
    }
    let upk = UpkPackage::load(&cooked.join(file))?;
    let (index, _) = cosmetic_thumbnail::readable_texture(&upk, &cooked.join(file), kind)?;
    let pixels = square(image::load_from_memory(png).map_err(|e| e.to_string())?);
    let change = (index, kind.to_string(), name.to_string(), pixels);
    apply_at(cooked, backups, &directory(backups), &upk, file, vec![change])
}

/// Replace Texture2D exports in a GFx menu package as (export, name, image).
/// Images are stretched over the texture's UV space, as Scaleform samples it.
pub(crate) fn menu_apply(
    cooked: &Path,
    backups: &Path,
    file: &str,
    textures: Vec<(usize, String, RgbaImage)>,
) -> Result<Manifest, String> {
    if !valid_file(file) {
        return Err("Invalid menu package filename".into());
    }
    let upk = UpkPackage::load(&cooked.join(file))?;
    let mut changes = Vec::new();
    for (export, name, pixels) in textures {
        if !upk
            .exports
            .get(export)
            .is_some_and(|e| upk.class_of(e).starts_with("Texture2D"))
        {
            return Err(format!("Menu export {export} is not a texture"));
        }
        changes.push((export, format!("menu|{export}"), name, pixels));
    }
    apply_at(cooked, backups, &menu_directory(backups), &upk, file, changes)
}

/// Applies (export, kind, name, image) changes to one package with a single rebuild.
fn apply_at(
    cooked: &Path,
    backups: &Path,
    store: &Path,
    upk: &UpkPackage,
    file: &str,
    changes: Vec<(usize, String, String, RgbaImage)>,
) -> Result<Manifest, String> {
    backup_guard::check_install(cooked, backups)?;
    let mut manifest = load_from(store)?;
    let live = fs::read(cooked.join(file)).map_err(|e| e.to_string())?;
    let mut package = if let Some(existing) = manifest
        .packages
        .iter()
        .find(|p| p.file.eq_ignore_ascii_case(file))
    {
        check_live(&live, existing)?;
        existing.clone()
    } else {
        Package {
            file: file.into(),
            original_hash: hash(&live),
            patched_hash: hash(&live),
            patches: Vec::new(),
        }
    };
    let mut patches = Vec::new();
    for (index, kind, name, pixels) in changes {
        let png = cosmetic_thumbnail::preview_png(pixels.clone())?;
        // Validate the texture before creating a backup or modifying the live file.
        cosmetic_thumbnail::bake_thumbnail(upk, &upk.exports[index], &pixels)?;
        patches.push(Patch {
            export: index,
            name,
            kind,
            png: STANDARD.encode(png),
        });
    }
    fs::create_dir_all(store).map_err(|e| e.to_string())?;
    if package.patches.is_empty() {
        replace_file(&store.join(format!("{}.bak", package.file)), &live)?;
    }
    package
        .patches
        .retain(|p| !patches.iter().any(|new| new.export == p.export));
    package.patches.extend(patches);
    let after = rebuilt(store, &package)?;
    package.patched_hash = hash(&after);
    manifest
        .packages
        .retain(|p| !p.file.eq_ignore_ascii_case(file));
    manifest.packages.push(package);
    commit(cooked, store, file, &live, &after, &manifest)?;
    Ok(manifest)
}

pub(crate) fn restore(
    cooked: &Path,
    backups: &Path,
    file: &str,
    export: usize,
) -> Result<Manifest, String> {
    restore_in(cooked, backups, &directory(backups), file, &[export])
}

pub(crate) fn menu_restore(
    cooked: &Path,
    backups: &Path,
    file: &str,
    exports: &[usize],
) -> Result<Manifest, String> {
    restore_in(cooked, backups, &menu_directory(backups), file, exports)
}

fn restore_in(
    cooked: &Path,
    backups: &Path,
    store: &Path,
    file: &str,
    exports: &[usize],
) -> Result<Manifest, String> {
    backup_guard::check_install(cooked, backups)?;
    let mut manifest = load_from(store)?;
    let index = manifest
        .packages
        .iter()
        .position(|p| p.file.eq_ignore_ascii_case(file))
        .ok_or("Thumbnail patch is no longer active")?;
    let package = &mut manifest.packages[index];
    let live = fs::read(cooked.join(&package.file)).map_err(|e| e.to_string())?;
    check_live(&live, package)?;
    if !package.patches.iter().any(|p| exports.contains(&p.export)) {
        return Err("Thumbnail patch is no longer active".into());
    }
    package.patches.retain(|p| !exports.contains(&p.export));
    let after = rebuilt(store, package)?;
    package.patched_hash = hash(&after);
    if package.patches.is_empty() {
        manifest.packages.remove(index);
    }
    commit(cooked, store, file, &live, &after, &manifest)?;
    Ok(manifest)
}

pub(crate) fn restore_all(cooked: &Path, backups: &Path) -> Result<Manifest, String> {
    restore_all_in(cooked, backups, &directory(backups))
}

pub(crate) fn menu_restore_all(cooked: &Path, backups: &Path) -> Result<Manifest, String> {
    restore_all_in(cooked, backups, &menu_directory(backups))
}

fn restore_all_in(cooked: &Path, backups: &Path, store: &Path) -> Result<Manifest, String> {
    backup_guard::check_install(cooked, backups)?;
    let mut manifest = load_from(store)?;
    for package in manifest.packages.clone() {
        let live = fs::read(cooked.join(&package.file)).map_err(|e| e.to_string())?;
        check_live(&live, &package)?;
        let original = snapshot(store, &package)?;
        manifest.packages.retain(|p| p.file != package.file);
        commit(cooked, store, &package.file, &live, &original, &manifest)?;
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_landscape_portrait_and_retains_alpha() {
        for (width, height) in [(400, 100), (100, 400), (128, 128)] {
            let pixels = RgbaImage::from_pixel(width, height, image::Rgba([90, 120, 200, 128]));
            let fitted = square(DynamicImage::ImageRgba8(pixels));
            assert_eq!(fitted.dimensions(), (EDGE, EDGE));
            assert_eq!(fitted.get_pixel(EDGE / 2, EDGE / 2)[3], 128);
            if width != height {
                assert_eq!(fitted.get_pixel(0, 0)[3], 0);
            }
        }
    }

    #[test]
    fn refuses_external_changes_and_invalid_paths() {
        let package = Package {
            file: "test.upk".into(),
            original_hash: hash(b"original"),
            patched_hash: hash(b"patched"),
            patches: vec![],
        };
        assert!(check_live(b"patched", &package).is_ok());
        assert!(check_live(b"other patch", &package).is_err());
        for file in ["../test.upk", "C:\\test.upk", "test.upk/evil.upk"] {
            assert!(!valid_file(file));
        }
    }

    /// Reads an installed UPK, but all writes and the mock game build stay in a
    /// temporary install. Run explicitly with HEBNIX_THUMBNAIL_TEST_SOURCE set.
    #[test]
    #[ignore = "requires HEBNIX_THUMBNAIL_TEST_SOURCE pointing to a thumbnail UPK"]
    fn real_package_roundtrip_and_shared_restore() {
        let source =
            std::path::PathBuf::from(std::env::var_os("HEBNIX_THUMBNAIL_TEST_SOURCE").unwrap());
        let root =
            std::env::temp_dir().join(format!("hebnix-thumbnail-test-{}", rand::random::<u64>()));
        let cooked = root.join("TAGame/CookedPCConsole");
        let backups = cooked.join("Backups");
        fs::create_dir_all(&cooked).unwrap();
        fs::create_dir_all(root.join("Binaries/Win64")).unwrap();
        fs::write(
            root.join("Binaries/Win64/RocketLeague.exe"),
            b"fixture build",
        )
        .unwrap();
        let file = source.file_name().unwrap().to_str().unwrap();
        let original = fs::read(&source).unwrap();
        fs::write(cooked.join(file), &original).unwrap();
        let upk = UpkPackage::load(&source).unwrap();
        let mut kinds = vec!["bodies".to_owned()];
        let mut seen = std::collections::HashSet::new();
        for export in &upk.exports {
            if upk.class_of(export).starts_with("ProductThumbnailAsset") {
                let name = upk.name_of(export.object_name);
                let leaf = super::super::upk_package::strip(&name);
                let leaf = leaf
                    .strip_suffix("_TThumbnail")
                    .or_else(|| leaf.strip_suffix("_T"))
                    .unwrap_or(leaf);
                kinds.push(format!("skins|fixture.{leaf}"));
            }
        }
        let choices: Vec<_> = kinds
            .into_iter()
            .filter_map(|kind| {
                let (index, _) = cosmetic_thumbnail::readable_texture(&upk, &source, &kind).ok()?;
                seen.insert(index).then_some((kind, index))
            })
            .take(2)
            .collect();
        assert!(!choices.is_empty(), "No readable texture in fixture");
        let red = cosmetic_thumbnail::preview_png(square(DynamicImage::ImageRgba8(
            RgbaImage::from_pixel(200, 100, image::Rgba([255, 0, 0, 255])),
        )))
        .unwrap();
        let blue = cosmetic_thumbnail::preview_png(square(DynamicImage::ImageRgba8(
            RgbaImage::from_pixel(100, 200, image::Rgba([0, 0, 255, 128])),
        )))
        .unwrap();
        for (kind, _) in &choices {
            apply(&cooked, &backups, file, kind, "Fixture", &red).unwrap();
        }
        // Reapplying must retain the original snapshot, and survive a reload.
        apply(
            &cooked,
            &backups,
            file,
            &choices[0].0,
            "Fixture replacement",
            &blue,
        )
        .unwrap();
        assert_eq!(load(&backups).unwrap().count(), choices.len());
        assert_eq!(
            fs::read(directory(&backups).join(format!("{file}.bak"))).unwrap(),
            original
        );
        let patched = UpkPackage::load(&cooked.join(file)).unwrap();
        let pixels =
            cosmetic_thumbnail::texture(&patched, &patched.exports[choices[0].1], &cooked).unwrap();
        assert_eq!(pixels.dimensions(), (EDGE, EDGE));
        assert_eq!(pixels.get_pixel(0, 0)[3], 0);
        let one_bit_alpha = !upk
            .names
            .iter()
            .any(|name| name == "PF_DXT5" || name == "PF_A8R8G8B8");
        let expected_alpha = if one_bit_alpha { 255 } else { 128 };
        assert!((pixels.get_pixel(EDGE / 2, EDGE / 2)[3] as i32 - expected_alpha).abs() <= 1);
        // Every unrelated export must retain exactly the same serialized bytes.
        for (index, export) in upk.exports.iter().enumerate() {
            if choices.iter().any(|(_, chosen)| *chosen == index) {
                continue;
            }
            let after = &patched.exports[index];
            assert_eq!(
                &upk.image[export.serial_offset..export.serial_offset + export.serial_size],
                &patched.image[after.serial_offset..after.serial_offset + after.serial_size],
                "Export {index} changed"
            );
        }
        restore(&cooked, &backups, file, choices[0].1).unwrap();
        assert_eq!(load(&backups).unwrap().count(), choices.len() - 1);
        if choices.len() > 1 {
            let patched = UpkPackage::load(&cooked.join(file)).unwrap();
            let pixels =
                cosmetic_thumbnail::texture(&patched, &patched.exports[choices[1].1], &cooked)
                    .unwrap();
            assert!(pixels.get_pixel(EDGE / 2, EDGE / 2)[0] > 240);
        }
        restore_all(&cooked, &backups).unwrap();
        assert_eq!(fs::read(cooked.join(file)).unwrap(), original);
        apply(&cooked, &backups, file, &choices[0].0, "Fixture", &red).unwrap();
        let conflict = b"another patch changed the file";
        fs::write(cooked.join(file), conflict).unwrap();
        assert!(restore_all(&cooked, &backups).is_err());
        assert_eq!(fs::read(cooked.join(file)).unwrap(), conflict);
        fs::remove_dir_all(root).unwrap();
    }

    /// Bakes into every readable texture of a real GFx package in a temporary
    /// install. Run explicitly with HEBNIX_MENU_TEST_SOURCE set.
    #[test]
    #[ignore = "requires HEBNIX_MENU_TEST_SOURCE pointing to a GFx UPK"]
    fn real_menu_package_roundtrip() {
        let source = std::path::PathBuf::from(std::env::var_os("HEBNIX_MENU_TEST_SOURCE").unwrap());
        let root = std::env::temp_dir().join(format!("hebnix-menu-test-{}", rand::random::<u64>()));
        let cooked = root.join("TAGame/CookedPCConsole");
        let backups = cooked.join("Backups");
        fs::create_dir_all(&cooked).unwrap();
        fs::create_dir_all(root.join("Binaries/Win64")).unwrap();
        fs::write(root.join("Binaries/Win64/RocketLeague.exe"), b"fixture build").unwrap();
        let file = source.file_name().unwrap().to_str().unwrap();
        let original = fs::read(&source).unwrap();
        fs::write(cooked.join(file), &original).unwrap();
        // External TFC mips must resolve against the real cooked folder.
        let real = menu_textures(&source).unwrap();
        let readable: Vec<_> = real.iter().filter(|t| t.png.is_ok()).collect();
        assert!(!readable.is_empty(), "No readable menu texture");
        let green = cosmetic_thumbnail::preview_png(RgbaImage::from_pixel(
            16,
            16,
            image::Rgba([0, 255, 0, 255]),
        ))
        .unwrap();
        let green = image::load_from_memory(&green).unwrap().to_rgba8();
        let all = readable
            .iter()
            .map(|t| (t.export, t.name.clone(), green.clone()))
            .collect();
        menu_apply(&cooked, &backups, file, all).unwrap();
        // A partial restore keeps the other replacements.
        menu_restore(&cooked, &backups, file, &[readable[0].export]).unwrap();
        assert_eq!(menu_load(&backups).unwrap().count(), readable.len() - 1);
        let patched = menu_textures(&cooked.join(file)).unwrap();
        let applied = &menu_load(&backups).unwrap().packages[0].patches;
        for patch in applied {
            let pixels = patched.iter().find(|t| t.export == patch.export).unwrap();
            let image = image::load_from_memory(pixels.png.as_ref().unwrap()).unwrap().to_rgba8();
            assert!(image.get_pixel(0, 0)[1] > 240, "{} not green", patch.name);
        }
        menu_restore_all(&cooked, &backups).unwrap();
        assert_eq!(fs::read(cooked.join(file)).unwrap(), original);
        fs::remove_dir_all(root).unwrap();
    }
}
