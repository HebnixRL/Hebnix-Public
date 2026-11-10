//! Surgical ProductAsset overrides for decals that share one cooked package.
use super::{
    cosmetic_upk,
    upk_package::{UpkPackage, strip},
};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fs, path::Path};

fn carrier_roots(package: &UpkPackage, index: usize) -> Result<HashSet<String>, String> {
    let data = payload(package, index)?;
    let mut roots = HashSet::new();
    for offset in data.objects {
        let reference = int(&data.bytes, offset)?;
        let path = package.asset_path(reference);
        let root = path.split('.').next().unwrap_or("");
        if let Some(tail) = root.strip_prefix("hebnix_skin_") {
            let parts: Vec<_> = tail.split('_').collect();
            if parts.len() == 2
                && parts
                    .iter()
                    .all(|part| part.len() == 16 && part.bytes().all(|c| c.is_ascii_hexdigit()))
            {
                roots.insert(root.into());
            }
        }
    }
    Ok(roots)
}

fn retire_carriers(
    previous: &HashSet<String>,
    package: &UpkPackage,
    game_dir: &Path,
    backups: &Path,
) {
    let mut used = HashSet::new();
    for export in &package.exports {
        if strip(&package.class_of(export)) != "ProductAsset_Skin_TA" {
            continue;
        }
        let Ok(roots) = carrier_roots(package, export.table_index) else {
            return;
        };
        used.extend(roots);
    }
    for root in previous.difference(&used) {
        let _ = fs::remove_file(game_dir.join(format!("{root}_SF.upk")));
        let _ = fs::remove_file(backups.join(format!("{root}_SF.upk.bak")));
    }
}

pub fn legacy_shared_package(live: &Path, backup: &Path) -> Result<bool, String> {
    if !backup.is_file() {
        return Ok(false);
    }
    let original = UpkPackage::load(backup)?;
    let current = UpkPackage::load(live)?;
    let originals: Vec<_> = original
        .exports
        .iter()
        .filter(|export| strip(&original.class_of(export)) == "ProductAsset_Skin_TA")
        .map(|export| original.asset_path(export.table_index as i32 + 1))
        .collect();
    Ok(originals.len() > 1
        && originals
            .iter()
            .any(|path| skin_index(&current, path).is_err()))
}

fn skin_index(package: &UpkPackage, path: &str) -> Result<usize, String> {
    let matches: Vec<_> = package
        .exports
        .iter()
        .filter(|export| {
            strip(&package.class_of(export)) == "ProductAsset_Skin_TA"
                && package
                    .asset_path(export.table_index as i32 + 1)
                    .eq_ignore_ascii_case(path)
        })
        .map(|export| export.table_index)
        .collect();
    if matches.len() != 1 {
        return Err(format!("Package does not uniquely export decal '{path}'"));
    }
    Ok(matches[0])
}

pub fn carrier_identity(target_asset: &str) -> String {
    let hash = format!(
        "{:x}",
        Sha256::digest(target_asset.to_ascii_lowercase().as_bytes())
    );
    // A failed reapply must never overwrite a carrier used by an existing edit.
    format!("hebnix_skin_{}_{:016x}", &hash[..16], rand::random::<u64>())
}

pub fn make_carrier(
    source: &Path,
    source_asset: &str,
    target_asset: &str,
    game_dir: &Path,
    base_dir: &Path,
) -> Result<(String, String), String> {
    let name = carrier_identity(target_asset);
    let filename = format!("{name}_SF.upk");
    let path = format!(
        "{name}.{}",
        source_asset
            .rsplit('.')
            .next()
            .ok_or("Missing decal asset name")?
    );
    cosmetic_upk::create_decal_carrier(
        source,
        &game_dir.join(&filename),
        base_dir,
        source_asset,
        &path,
    )?;
    Ok((filename, path))
}

struct Payload {
    bytes: Vec<u8>,
    names: Vec<usize>,
    objects: Vec<usize>,
}

fn payload(package: &UpkPackage, index: usize) -> Result<Payload, String> {
    let export = &package.exports[index];
    let (props, end) = package.serialized_props(export)?;
    if end != export.serial_size {
        return Err("Unsupported native decal product payload".into());
    }
    let mut result = Payload {
        bytes: package.image[export.serial_offset..export.serial_offset + export.serial_size]
            .to_vec(),
        names: vec![end - 8],
        objects: Vec::new(),
    };
    collect_fields(
        package,
        &props,
        export.serial_offset,
        export.serial_offset,
        &mut result,
    )?;
    Ok(result)
}

fn collect_fields(
    package: &UpkPackage,
    props: &[super::upk_package::Prop],
    base: usize,
    origin: usize,
    result: &mut Payload,
) -> Result<(), String> {
    for prop in props {
        let tag = base + prop.tag_offset;
        let value = base + prop.value_offset;
        result.names.extend([tag - origin, tag + 8 - origin]);
        match prop.tag_type.as_str() {
            "ObjectProperty" | "ComponentProperty" | "ClassProperty" if prop.size == 4 => {
                result.objects.push(value - origin)
            }
            "NameProperty" if prop.size == 8 => result.names.push(value - origin),
            "ByteProperty" => {
                result.names.push(tag + 24 - origin);
                if prop.size == 8 {
                    result.names.push(value - origin);
                } else if prop.size != 1 {
                    return Err("Unsupported decal enum width".into());
                }
            }
            "ArrayProperty" => {
                let count = package.read_int(value)?;
                if !(0..=4096).contains(&count) {
                    return Err("Unsupported decal array count".into());
                }
                let end = value.checked_add(prop.size).ok_or("Decal array overflow")?;
                let mut at = value + 4;
                for _ in 0..count {
                    let (fields, next) = package.nested_props(at, end)?;
                    collect_fields(package, &fields, 0, origin, result)?;
                    result.names.push(next - 8 - origin);
                    at = next;
                }
                if at != end {
                    return Err("Unsupported decal array layout".into());
                }
            }
            "StructProperty" => {
                result.names.push(tag + 24 - origin);
                if !matches!(
                    prop.struct_name.as_str(),
                    "Color" | "LinearColor" | "Vector" | "Vector2D" | "Rotator" | "Guid"
                ) {
                    let (fields, end) = package.nested_props(value, value + prop.size)?;
                    if end != value + prop.size {
                        return Err(format!("Unsupported decal struct '{}'", prop.struct_name));
                    }
                    collect_fields(package, &fields, 0, origin, result)?;
                    result.names.push(end - 8 - origin);
                }
            }
            "IntProperty" | "FloatProperty" | "BoolProperty" | "QWordProperty" | "StrProperty" => {}
            other => {
                return Err(format!(
                    "Unsupported decal property '{other}'; original package left unchanged"
                ));
            }
        }
    }
    Ok(())
}
fn int(bytes: &[u8], offset: usize) -> Result<i32, String> {
    Ok(i32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or("Truncated decal property")?
            .try_into()
            .unwrap(),
    ))
}

/// Link only the selected product to the isolated carrier. All sibling product
/// exports and material objects remain intact, including previous overrides.
pub fn link_product(
    source: &Path,
    source_asset: &str,
    target_live: &Path,
    target_backup: &Path,
    target_asset: &str,
    base_dir: &Path,
) -> Result<(), String> {
    let donor = UpkPackage::load(source)?;
    let data = payload(&donor, skin_index(&donor, source_asset)?)?;
    let mut objects = Vec::new();
    for offset in &data.objects {
        let reference = int(&data.bytes, *offset)?;
        if reference == 0 {
            continue;
        }
        let (class_package, class_name) = donor.reference_class(reference)?;
        objects.push((donor.asset_path(reference), class_package, class_name));
    }
    if !target_backup.is_file() {
        if let Some(parent) = target_backup.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::copy(target_live, target_backup).map_err(|e| e.to_string())?;
    }
    let staged = target_live.with_extension("variant.tmp");
    let before = UpkPackage::load(target_live)?;
    let previous = skin_index(&before, target_asset)
        .and_then(|index| carrier_roots(&before, index))
        .unwrap_or_default();
    let result = (|| {
        let baseline = if legacy_shared_package(target_live, target_backup)? {
            target_backup
        } else {
            target_live
        };
        cosmetic_upk::extend_asset_imports(baseline, &staged, base_dir, &donor.names, &objects)?;
        let mut target = UpkPackage::load(&staged)?;
        let index = skin_index(&target, target_asset)?;
        let mut bytes = data.bytes.clone();
        // Keep the original net index; the exported item identity is unchanged.
        bytes[..4].copy_from_slice(
            &target.image
                [target.exports[index].serial_offset..target.exports[index].serial_offset + 4],
        );
        for offset in data.names {
            let old = usize::try_from(int(&bytes, offset)?).map_err(|_| "Invalid property name")?;
            let name = donor.names.get(old).ok_or("Invalid donor property name")?;
            let new = target
                .names
                .iter()
                .position(|candidate| candidate == name)
                .ok_or("Missing imported property name")?;
            bytes[offset..offset + 4].copy_from_slice(&(new as i32).to_le_bytes());
        }
        for offset in data.objects {
            let old = int(&bytes, offset)?;
            let new = if old == 0 {
                0
            } else {
                target
                    .find_object(&donor.asset_path(old))
                    .ok_or("Missing imported decal material")?
            };
            bytes[offset..offset + 4].copy_from_slice(&new.to_le_bytes());
        }
        let peers = peer_payloads(&target, index);
        target.replace_export_payload(index, &bytes)?;
        target.save(&staged)?;
        let checked = UpkPackage::load(&staged)?;
        let checked_index = skin_index(&checked, target_asset)?;
        payload(&checked, checked_index)?;
        if peer_payloads(&checked, checked_index) != peers {
            return Err("Decal override changed a sibling product".into());
        }
        let game = target_live.parent().ok_or("Missing decal package directory")?;
        super::upk_history::install_linked(game, target_live, &fs::read(&staged).map_err(|e| e.to_string())?, "Decal variant")?;
        if let (Some(game), Some(backups)) = (target_live.parent(), target_backup.parent()) {
            retire_carriers(&previous, &checked, game, backups);
        }
        Ok(())
    })();
    let _ = fs::remove_file(staged);
    result
}

fn peer_payloads(package: &UpkPackage, except: usize) -> Vec<(usize, Vec<u8>)> {
    package
        .exports
        .iter()
        .filter(|export| {
            export.table_index != except
                && strip(&package.class_of(export)) == "ProductAsset_Skin_TA"
        })
        .map(|export| {
            (
                export.table_index,
                package.image[export.serial_offset..export.serial_offset + export.serial_size]
                    .to_vec(),
            )
        })
        .collect()
}

pub fn restore_product(live: &Path, backup: &Path, asset: &str) -> Result<(), String> {
    if legacy_shared_package(live, backup)? {
        let game = live.parent().ok_or("Missing decal package directory")?;
        return super::upk_history::install_copy(game, backup, live, "Restore decal variant");
    }
    let original = UpkPackage::load(backup)?;
    let original_index = skin_index(&original, asset)?;
    let mut current = UpkPackage::load(live)?;
    let index = skin_index(&current, asset)?;
    let previous = carrier_roots(&current, index).unwrap_or_default();
    // Import/name extension preserves original indices as a prefix.
    if !current.names.starts_with(&original.names) {
        return Err("Decal package identity changed; cannot safely restore one variant".into());
    }
    let export = &original.exports[original_index];
    let peers = peer_payloads(&current, index);
    current.replace_export_payload(
        index,
        &original.image[export.serial_offset..export.serial_offset + export.serial_size],
    )?;
    let staged = live.with_extension("variant-restore.tmp");
    let result = (|| {
        current.save(&staged)?;
        let checked = UpkPackage::load(&staged)?;
        if peer_payloads(&checked, index) != peers {
            return Err("Restoring one decal changed a sibling product".into());
        }
        let game = live.parent().ok_or("Missing decal package directory")?;
        super::upk_history::install_linked(game, live, &fs::read(&staged).map_err(|e| e.to_string())?, "Restore decal variant")?;
        if let (Some(game), Some(backups)) = (live.parent(), backup.parent()) {
            retire_carriers(&previous, &checked, game, backups);
        }
        Ok(())
    })();
    let _ = fs::remove_file(staged);
    result
}
