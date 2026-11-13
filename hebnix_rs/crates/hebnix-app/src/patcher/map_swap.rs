//! Arena look swap: a host arena shows another arena's scenery and keeps its own gameplay.
//! `stage` writes the patched files to a scratch folder and never touches the game folder.

use super::background_merger::{
    Patch, apply_patches, disable_old_atmosphere, donor_copy_label, is_gameplay_actor_class,
    make_copy_name, visible_sublevel, world_visual_patches,
};
use super::upk_package::{Prop, UpkPackage, strip};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct Staged {
    /// Name of the donor copy the host's streaming slot points at
    pub copy_name: String,
    pub files: Vec<PathBuf>,
    /// Host scenery references emptied
    pub hidden: usize,
    /// Donor gameplay and collision edits
    pub stripped: usize,
}

fn collidable(props: &[Prop]) -> bool {
    props.iter().any(|p| {
        matches!(p.name.as_str(), "BlockRigidBody" | "CollideActors") && p.bool_value == Some(true)
    })
}

/// Empties a host's scenery: meshes, particles, lights and fog. The invisible collision
/// meshes and the components of gameplay actors (boost pads, goals) stay as they are.
fn hide_scenery(package: &UpkPackage) -> Result<Vec<Patch>, String> {
    let mut patches = Vec::new();
    for e in &package.exports {
        let class = strip(&package.class_of(e)).to_string();
        let property = match class.as_str() {
            "StaticMeshComponent" | "InstancedStaticMeshComponent" => "StaticMesh",
            "SkeletalMeshComponent" => "SkeletalMesh",
            "ParticleSystemComponent" => "Template",
            _ => continue,
        };
        if package.owner_classes(e).iter().any(|owner| is_gameplay_actor_class(owner)) {
            continue;
        }
        let props = package.parse_props(e);
        if collidable(&props) {
            continue;
        }
        if let Some(p) = props.iter().find(|p| p.name == property && p.tag_type == "ObjectProperty") {
            let offset = e.serial_offset + p.value_offset;
            if package.read_int(offset)? != 0 {
                patches.push((offset, vec![0; 4]));
            }
        }
    }
    patches.extend(disable_old_atmosphere(package)?);
    Ok(patches)
}

/// Keeps a donor arena's scenery and removes what would play: gameplay actors, collision
/// and physics. The host supplies those, so the match stays the host map's.
fn strip_gameplay(package: &UpkPackage) -> Result<Vec<Patch>, String> {
    let mut patches = Vec::new();
    for e in &package.exports {
        let class = strip(&package.class_of(e)).to_string();
        let props = package.parse_props(e);
        let gameplay = is_gameplay_actor_class(&class);
        let brush = class == "BrushComponent" || class == "ModelComponent";
        for p in &props {
            let offset = e.serial_offset + p.value_offset;
            let drop_reference = p.tag_type == "ObjectProperty"
                && (gameplay || (brush && matches!(p.name.as_str(), "Brush" | "Model" | "BrushAggGeom")));
            if drop_reference && package.read_int(offset)? != 0 {
                patches.push((offset, vec![0; 4]));
            }
        }
        if class.ends_with("MeshComponent") || class.contains("Collision") || brush || gameplay {
            for p in &props {
                if p.tag_type == "BoolProperty"
                    && (p.name.contains("Collide") || p.name.contains("Block") || p.name.contains("RigidBody"))
                {
                    patches.push((e.serial_offset + p.value_offset - 1, vec![0]));
                }
            }
        }
    }
    Ok(patches)
}

pub const ARENA_DONOR_PREFIX: &str = "arena:";
const BACKUP_SUFFIX: &str = ".hbnx_mapbak";

/// Where `build` writes. Staging never touches the game folder. Installing backs each
/// file up beside itself first, which is how the Background Changer restores them.
enum Target<'a> {
    Stage(&'a Path),
    Install,
}

fn build(
    cooked: &Path,
    host: &str,
    donor_source: &Path,
    donor: &str,
    target: Target,
) -> Result<(Staged, Vec<String>), String> {
    let package = |name: &str| cooked.join(format!("{name}.upk"));
    let destination = |name: &str| match &target {
        Target::Stage(out) => out.join(format!("{name}.upk")),
        Target::Install => package(name),
    };
    if let Target::Stage(out) = &target {
        std::fs::create_dir_all(out).map_err(|e| format!("Could not create {}: {e}", out.display()))?;
    }
    let mut host_package = UpkPackage::load(&package(host))?;
    let donor_package = UpkPackage::load(donor_source)?;
    // The donor copy takes over a scenery layer, so the arena keeps its own audio layer.
    // Maps with no scenery layer fall back to the slot the Background Changer uses.
    let layers = host_package.find_all_stream_name_indices();
    let scenery = |index: &usize| visible_sublevel(&host_package.names[*index]);
    let slot = layers
        .iter()
        .copied()
        .find(|i| scenery(i) && host_package.names[*i].to_ascii_lowercase().contains("oob"))
        .or_else(|| layers.iter().copied().find(scenery))
        .or_else(|| host_package.find_stream_name_index())
        .ok_or("This map has no streaming slot and cannot host another arena's look")?;
    let copy_name = make_copy_name(&donor_copy_label(donor), host_package.names[slot].len())?;
    let mut staged = Staged { copy_name: copy_name.clone(), ..Staged::default() };
    let mut patched_layers = Vec::new();

    let copy_path = destination(&copy_name);
    std::fs::copy(donor_source, &copy_path).map_err(|e| format!("Could not copy {donor}: {e}"))?;
    let mut copy = UpkPackage::load(&copy_path)?;
    let patches = strip_gameplay(&copy)?;
    staged.stripped = patches.len();
    apply_patches(&mut copy, patches)?;
    copy.save(&copy_path)?;
    staged.files.push(copy_path);

    for index in layers.into_iter().filter(|i| *i != slot) {
        let name = host_package.names[index].clone();
        let live = package(&name);
        if !visible_sublevel(&name) || !live.is_file() {
            continue;
        }
        // A layer shared with another arena may already be backed up, keep that backup.
        let source = if matches!(target, Target::Install) {
            let backup = PathBuf::from(format!("{}{BACKUP_SUFFIX}", live.display()));
            if !backup.exists() {
                std::fs::copy(&live, &backup)
                    .map_err(|e| format!("Could not back up sub-level {name}: {e}"))?;
            }
            backup
        } else {
            live
        };
        let mut layer = UpkPackage::load(&source)?;
        let patches = hide_scenery(&layer)?;
        staged.hidden += patches.len();
        apply_patches(&mut layer, patches)?;
        let path = destination(&name);
        layer.save(&path)?;
        staged.files.push(path);
        patched_layers.push(name);
    }

    host_package.rename_name(slot, &copy_name)?;
    let mut patches = hide_scenery(&host_package)?;
    patches.extend(world_visual_patches(&host_package, &donor_package)?);
    staged.hidden += patches.len();
    apply_patches(&mut host_package, patches)?;
    let path = destination(host);
    host_package.save(&path)?;
    staged.files.push(path);

    for file in &staged.files {
        UpkPackage::load(file).map_err(|e| format!("{} does not load again: {e}", file.display()))?;
    }
    Ok((staged, patched_layers))
}

/// Writes everything a swap needs into `out`: the donor copy, the host arena with its
/// streaming slot pointing at that copy, and the host's other scenery layers emptied.
#[cfg(test)]
pub fn stage(cooked: &Path, host: &str, donor: &str, out: &Path) -> Result<Staged, String> {
    let donor_source = cooked.join(format!("{donor}.upk"));
    build(cooked, host, &donor_source, donor, Target::Stage(out)).map(|(staged, _)| staged)
}

/// Installs the swap into the game folder. The caller has already backed up the host
/// arena and records the result so the Background Changer can restore it.
/// Returns the donor copy's name and the host layers that were emptied.
pub fn apply_arena(
    cooked: &Path,
    host: &str,
    donor_source: &Path,
    donor: &str,
) -> Result<(String, Vec<String>), String> {
    build(cooked, host, donor_source, donor, Target::Install)
        .map(|(staged, layers)| (staged.copy_name, layers))
}

#[cfg(test)]
mod layout_report {
    use super::super::background_merger::is_gameplay_actor_class;
    use super::super::upk_package::{UpkPackage, strip};
    use std::collections::BTreeMap;
    use std::path::Path;

    /// One package: size, class counts, gameplay exports and its streaming layers.
    fn report(cooked: &Path, name: &str, indent: &str, recurse: bool) {
        let path = cooked.join(format!("{name}.upk"));
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let package = match UpkPackage::load(&path) {
            Ok(package) => package,
            Err(error) => {
                println!("{indent}{name}: cannot load ({error})");
                return;
            }
        };
        let mut classes: BTreeMap<String, usize> = BTreeMap::new();
        let mut gameplay = 0;
        let mut collidable = 0;
        let mut collidable_meshes: Vec<String> = Vec::new();
        for export in &package.exports {
            let class = package.class_of(export);
            let class = strip(&class).to_string();
            if is_gameplay_actor_class(&class) {
                gameplay += 1;
            }
            if class.ends_with("MeshComponent") {
                let props = package.parse_props(export);
                if props.iter().any(|p| {
                    matches!(p.name.as_str(), "BlockRigidBody" | "CollideActors") && p.bool_value == Some(true)
                }) {
                    collidable += 1;
                    let mesh = props
                        .iter()
                        .find(|p| p.name == "StaticMesh" && p.tag_type == "ObjectProperty")
                        .and_then(|p| package.read_int(export.serial_offset + p.value_offset).ok())
                        .map(|reference| package.obj_name(reference))
                        .unwrap_or_default();
                    let hidden = props.iter().any(|p| p.name == "HiddenGame" && p.bool_value == Some(true));
                    collidable_meshes.push(format!("{mesh}{}", if hidden { " [hidden]" } else { "" }));
                }
            }
            *classes.entry(class).or_default() += 1;
        }
        let mut top: Vec<_> = classes.into_iter().collect();
        top.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        let top: Vec<String> = top.iter().take(8).map(|(c, n)| format!("{c} {n}")).collect();
        let mut layers: Vec<String> = Vec::new();
        for index in package.find_all_stream_name_indices() {
            let layer = package.names[index].clone();
            if !layers.iter().any(|known| known.eq_ignore_ascii_case(&layer)) {
                layers.push(layer);
            }
        }
        println!(
            "{indent}{name}: {} KB, {} exports, {gameplay} gameplay, {collidable} collidable mesh components",
            size / 1024,
            package.exports.len()
        );
        println!("{indent}  top classes: {}", top.join(", "));
        if !collidable_meshes.is_empty() {
            println!("{indent}  collidable meshes: {}", collidable_meshes.join(", "));
        }
        println!("{indent}  streaming slots ({}): {}", layers.len(), layers.join(", "));
        if recurse {
            for layer in &layers {
                if cooked.join(format!("{layer}.upk")).is_file() {
                    report(cooked, layer, &format!("{indent}    "), false);
                } else {
                    println!("{indent}    {layer}: file not found");
                }
            }
        }
    }

    /// HBNX_COOKED=<CookedPCConsole> HBNX_HOST=Underwater_P HBNX_DONOR=Park_P
    /// cargo test -p hebnix-app layout_report -- --ignored --nocapture
    #[test]
    #[ignore = "reads a Rocket League install"]
    fn host_and_donor_layout() {
        let (Ok(cooked), Ok(host), Ok(donor)) = (
            std::env::var("HBNX_COOKED"),
            std::env::var("HBNX_HOST"),
            std::env::var("HBNX_DONOR"),
        ) else {
            panic!("set HBNX_COOKED, HBNX_HOST and HBNX_DONOR");
        };
        for name in [host, donor] {
            report(Path::new(&cooked), &name, "", true);
        }
    }

    /// HBNX_COOKED, HBNX_HOST, HBNX_DONOR and HBNX_OUT (a scratch folder, not the game's).
    #[test]
    #[ignore = "reads a Rocket League install"]
    fn stage_a_swap() {
        let get = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("set {name}"));
        let cooked = get("HBNX_COOKED");
        let out = std::path::PathBuf::from(get("HBNX_OUT"));
        assert!(!out.starts_with(&cooked), "HBNX_OUT must not be inside the game folder");
        let staged = super::stage(Path::new(&cooked), &get("HBNX_HOST"), &get("HBNX_DONOR"), &out).unwrap();
        println!("copy {} | hidden {} | stripped {}", staged.copy_name, staged.hidden, staged.stripped);
        for file in &staged.files {
            println!("  {} ({} KB)", file.display(), std::fs::metadata(file).unwrap().len() / 1024);
        }
    }
}
