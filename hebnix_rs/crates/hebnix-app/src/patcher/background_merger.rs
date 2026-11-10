use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::upk_package::{ExportEntry, Prop, UpkPackage, strip};

const BACKUP_SUFFIX: &str = ".hbnx_mapbak";
const MANIFEST: &str = "background_swaps.json";
const NO_BACKGROUND_DONOR: &str = "__HBNX_NO_BACKGROUND__";
pub const CUSTOM_DONOR_PREFIX: &str = "custom:";
/// `arena:Park_P` borrows a whole arena's look instead of just its sky and background.
pub use super::map_swap::ARENA_DONOR_PREFIX;
const SUNSET_DUNES_SCENERY: &str = "Labs_4v4_Arena15_Retro_OOB";
// Dedicated scenery packages and existing arena donors verified with the exterior filter.
pub(crate) const SCENERY_DONORS: &[(&str, &str)] = &[
    ("ShatterShot_VFX", "Core 707 — scenery"),
    ("BG_Stadium_10A_P", "DFH Stadium (10th Anniversary) — scenery"),
    ("BG_NeoTokyo_Arcade", "Neo Tokyo (Arcade) — scenery"),
    ("BG_NeoTokyo_Hax", "Neo Tokyo (Hacked) — scenery"),
    ("BG_Woods_Day_P", "Drift Woods (Day) — scenery"),
    ("UtopiaStadium_P", "Utopia Coliseum — scenery"),
    ("BG_FNI_Stadium", "Forbidden Temple (Fire & Ice) — scenery"),
    ("BG_EuroStadium_Dusk", "Mannfield (Dusk) — scenery"),
    ("BG_FF_Dusk", "Estadio Vida — scenery"),
    ("BG_Mall_Day_P", "Boostfield Mall — scenery"),
    ("BG_Paname_Dusk", "Parc de Paris — scenery"),
    ("BG_Park_Snowy", "Beckwith Park (Snowy) — scenery"),
    ("BG_UF_Day_P", "Futura Garden (Day) — scenery"),
    ("BG_UF_Night_P", "Futura Garden (Night) — scenery"),
    ("BG_Underwater_GRS", "AquaDome (Salty Shallows) — scenery"),
    ("BG_UtopiaSnow", "Utopia Coliseum (Snowy) — scenery"),
    ("BG_Woods_Night_P", "Drift Woods (Night) — scenery"),
    (SUNSET_DUNES_SCENERY, "Sunset Dunes — scenery"),
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SwapState {
    donor: String,
    copy_name: String,
    patched_sub_levels: Vec<String>,
    extra_copies: Vec<String>,
}

type State = HashMap<String, SwapState>;
pub(super) type Patch = (usize, Vec<u8>);
static BACKGROUND_EDIT: Mutex<()> = Mutex::new(());

enum PreparedDonor {
    Arena { name: String, source: PathBuf },
    Scenery(PreparedScenery),
}

struct PreparedScenery {
    scenery: Vec<u8>,
    world: Option<UpkPackage>,
    generated: HashSet<String>,
}

pub fn run(
    command: &str,
    cooked: &Path,
    state_dir: &Path,
    host: Option<&str>,
    donor: Option<&str>,
) -> Result<String, String> {
    let _lock = BACKGROUND_EDIT.lock().map_err(|e| e.to_string())?;
    if !cooked.is_dir() {
        return Err(
            "The configured Rocket League folder does not contain TAGame\\CookedPCConsole.".into(),
        );
    }
    std::fs::create_dir_all(state_dir)
        .map_err(|e| format!("Could not create background state folder: {e}"))?;
    let discarded_stale = synchronize_build(cooked, state_dir)?;
    if discarded_stale && matches!(command, "undo" | "reset") {
        return Err("Rocket League was updated. Hebnix discarded the old background backups and active entries instead of restoring arenas from the previous build. Apply the background again to start fresh.".into());
    }
    match command {
        "apply" => apply(
            cooked,
            state_dir,
            host.ok_or("Missing host arena")?,
            donor.ok_or("Missing background donor")?,
        ),
        "remove" => remove_background(cooked, state_dir, host.ok_or("Missing host arena")?),
        "undo" => undo(cooked, state_dir, host.ok_or("Missing host arena")?),
        "reset" => reset(cooked, state_dir),
        _ => Err("Invalid background changer command".into()),
    }
}

fn synchronize_build(cooked: &Path, state_dir: &Path) -> Result<bool, String> {
    let backup_root = cooked.join("Backups");
    std::fs::create_dir_all(&backup_root)
        .map_err(|e| format!("Could not create {}: {e}", backup_root.display()))?;
    let marker = backup_root.join("background-build.sha256");
    let current = super::backup_guard::build_id(cooked)?;
    let recorded = std::fs::read_to_string(&marker).unwrap_or_default();
    if recorded.trim() == current {
        return Ok(false);
    }
    let backups = matching_files(cooked, |name| name.ends_with(".upk.hbnx_mapbak"));
    let manifest = state_dir.join(MANIFEST);
    let stale = !backups.is_empty() || manifest.is_file();
    if stale {
        for path in backups {
            std::fs::remove_file(&path).map_err(|e| {
                format!("Could not discard stale map backup {}: {e}", path.display())
            })?;
        }
        for path in matching_files(cooked, |name| {
            name.starts_with("HBNX_") && name.ends_with(".upk")
        }) {
            std::fs::remove_file(&path).map_err(|e| {
                format!(
                    "Could not remove stale generated map {}: {e}",
                    path.display()
                )
            })?;
        }
        if manifest.is_file() {
            std::fs::remove_file(&manifest).map_err(|e| {
                format!(
                    "Could not discard stale background state {}: {e}",
                    manifest.display()
                )
            })?;
        }
    }
    std::fs::write(&marker, current)
        .map_err(|e| format!("Could not write {}: {e}", marker.display()))?;
    Ok(stale)
}

fn remove_background(cooked: &Path, state_dir: &Path, host_name: &str) -> Result<String, String> {
    validate_name(host_name)?;
    ensure_game_closed("removing a background")?;
    let host_path = map_path(cooked, host_name);
    if !host_path.is_file() {
        return Err(format!("Host arena is missing: {host_name}.upk"));
    }
    let donor_name = SCENERY_DONORS
        .iter()
        .map(|(package, _)| *package)
        .find(|name| map_path(cooked, name).is_file())
        .ok_or("No game-safe scenery package is installed to create an empty background.")?;
    let mut state = load_state(state_dir)?;
    undo_core(cooked, &mut state, host_name)?;
    save_state(state_dir, &state)?;
    let old_copies = matching_files(cooked, |name| {
        name.starts_with("HBNX_") && name.ends_with(".upk")
    });
    let old_backups = matching_files(cooked, |name| name.ends_with(".upk.hbnx_mapbak"));
    let backup = backup_path(&host_path);
    if backup.exists() {
        return Err("This arena has a background backup that Hebnix does not own. Restore it before changing this background.".into());
    }
    std::fs::copy(&host_path, &backup).map_err(|e| format!("Could not back up host arena: {e}"))?;
    let result = remove_background_inner(cooked, &host_path, donor_name);
    match result {
        Ok((copy_name, patched_sub_levels)) => {
            state.insert(
                host_name.to_string(),
                SwapState {
                    donor: NO_BACKGROUND_DONOR.to_string(),
                    copy_name,
                    patched_sub_levels,
                    extra_copies: Vec::new(),
                },
            );
            if let Err(e) = save_state(state_dir, &state) {
                let _ = undo_core(cooked, &mut state, host_name);
                return Err(e);
            }
            Ok(format!(
                "Removed {}'s fog, sky and background.",
                friendly(host_name)
            ))
        }
        Err(error) => {
            for new_backup in matching_files(cooked, |name| name.ends_with(".upk.hbnx_mapbak"))
                .difference(&old_backups)
            {
                let live =
                    PathBuf::from(new_backup.to_string_lossy().trim_end_matches(BACKUP_SUFFIX));
                let _ = restore_file(cooked, new_backup, &live);
            }
            for new_copy in matching_files(cooked, |name| {
                name.starts_with("HBNX_") && name.ends_with(".upk")
            })
            .difference(&old_copies)
            {
                let _ = std::fs::remove_file(new_copy);
            }
            Err(error)
        }
    }
}
fn apply(
    cooked: &Path,
    state_dir: &Path,
    host_name: &str,
    donor_name: &str,
) -> Result<String, String> {
    validate_name(host_name)?;
    ensure_game_closed("applying a background")?;
    if host_name.eq_ignore_ascii_case(donor_name.strip_prefix(ARENA_DONOR_PREFIX).unwrap_or(donor_name)) {
        return Err("Pick two different arenas.".into());
    }
    let mut donor = prepare_donor(cooked, donor_name)?;
    ensure_game_closed("applying a background")?;
    apply_prepared(cooked, state_dir, host_name, donor_name, &mut donor, false)?;
    Ok(match donor {
        PreparedDonor::Arena { name, .. } => {
            format!("{} now looks like {}.", friendly(host_name), friendly(&name))
        }
        PreparedDonor::Scenery(_) => format!(
            "{} now uses {}'s background.",
            friendly(host_name),
            friendly(donor_name)
        ),
    })
}

pub fn apply_all(
    cooked: &Path,
    state_dir: &Path,
    hosts: &[String],
    donor_name: &str,
    mut progress: impl FnMut(usize, &str, Result<bool, String>),
) -> Result<(), String> {
    let _lock = BACKGROUND_EDIT.lock().map_err(|e| e.to_string())?;
    ensure_game_closed("applying a background")?;
    synchronize_build(cooked, state_dir)?;
    let mut donor = prepare_donor(cooked, donor_name)?;
    for (index, host) in hosts.iter().enumerate() {
        ensure_game_closed("applying a background")?;
        let result = apply_prepared(cooked, state_dir, host, donor_name, &mut donor, true);
        progress(index, host, result);
    }
    Ok(())
}

fn prepare_donor(cooked: &Path, donor_name: &str) -> Result<PreparedDonor, String> {
    if let Some(name) = donor_name.strip_prefix(ARENA_DONOR_PREFIX) {
        validate_name(name)?;
        return Ok(PreparedDonor::Arena {
            name: name.to_owned(),
            source: package_source(cooked, name)?,
        });
    }
    let custom_donor = donor_name.starts_with(CUSTOM_DONOR_PREFIX);
    let scenery_source = if custom_donor {
        custom_donor_path(donor_name)?
    } else {
        validate_name(donor_name)?;
        if !SCENERY_DONORS
            .iter()
            .any(|(package, _)| package.eq_ignore_ascii_case(donor_name))
        {
            return Err("That package is not a game-safe background donor.".into());
        }
        package_source(cooked, donor_name)?
    };
    let hacked_tokyo = donor_name.eq_ignore_ascii_case("BG_NeoTokyo_Hax");
    // Sunset Dunes keeps its sky dome in the main arena, separate from its exterior.
    let preserve_host_atmosphere =
        hacked_tokyo || donor_name.eq_ignore_ascii_case(SUNSET_DUNES_SCENERY);
    let scenery_source = if hacked_tokyo {
        package_source(cooked, "neotokyo_hax_signs_off_p")?
    } else {
        scenery_source
    };
    let mut scenery = UpkPackage::load(&scenery_source)?;
    if patch_donor_outside_only(&mut scenery, custom_donor)? == 0 {
        return Err(format!(
            "'{donor_name}' has no recognised sky or out-of-bounds scenery to transfer"
        ));
    }
    let bytes = scenery.repacked_fast()?;
    if custom_donor {
        UpkPackage::from_bytes(bytes.clone())
            .map_err(|error| format!("Filtered background failed round-trip validation: {error}"))?;
    }
    // Scenery filtering edits components and gameplay actors, leaving WorldInfo intact.
    Ok(PreparedDonor::Scenery(PreparedScenery {
        scenery: bytes,
        world: (!preserve_host_atmosphere).then_some(scenery),
        generated: HashSet::new(),
    }))
}

fn apply_prepared(
    cooked: &Path,
    state_dir: &Path,
    host_name: &str,
    donor_name: &str,
    donor: &mut PreparedDonor,
    only_unchanged: bool,
) -> Result<bool, String> {
    validate_name(host_name)?;
    if host_name.eq_ignore_ascii_case(donor_name.strip_prefix(ARENA_DONOR_PREFIX).unwrap_or(donor_name)) {
        return Err("Pick two different arenas.".into());
    }
    let host_path = map_path(cooked, host_name);
    if !host_path.is_file() {
        return Err(format!("Host arena is missing: {host_name}.upk"));
    }
    let mut state = load_state(state_dir)?;
    if state.contains_key(host_name) {
        if only_unchanged {
            return Ok(false);
        }
        undo_core(cooked, &mut state, host_name)?;
        save_state(state_dir, &state)?;
    }
    let old_copies = matching_files(cooked, |name| {
        name.starts_with("HBNX_") && name.ends_with(".upk")
    });
    let old_backups = matching_files(cooked, |name| name.ends_with(".upk.hbnx_mapbak"));
    let backup = backup_path(&host_path);
    if backup.exists() {
        return Err("This arena has a background backup that Hebnix does not own. Restore it before changing this background.".into());
    }
    std::fs::copy(&host_path, &backup).map_err(|e| format!("Could not back up host arena: {e}"))?;
    let result = match donor {
        PreparedDonor::Arena { name, source } => {
            super::map_swap::apply_arena(cooked, host_name, source, name)
        }
        PreparedDonor::Scenery(scenery) => apply_inner(cooked, &host_path, donor_name, scenery),
    };
    match result {
        Ok((copy_name, patched_sub_levels)) => {
            state.insert(
                host_name.to_string(),
                SwapState {
                    donor: donor_name.to_string(),
                    copy_name: copy_name.clone(),
                    patched_sub_levels,
                    extra_copies: Vec::new(),
                },
            );
            if let Err(e) = save_state(state_dir, &state) {
                let _ = undo_core(cooked, &mut state, host_name);
                return Err(e);
            }
            if let PreparedDonor::Scenery(scenery) = donor {
                scenery.generated.insert(copy_name);
            }
            Ok(true)
        }
        Err(error) => {
            for new_backup in matching_files(cooked, |name| name.ends_with(".upk.hbnx_mapbak"))
                .difference(&old_backups)
            {
                let live =
                    PathBuf::from(new_backup.to_string_lossy().trim_end_matches(BACKUP_SUFFIX));
                let _ = restore_file(cooked, new_backup, &live);
            }
            for new_copy in matching_files(cooked, |name| {
                name.starts_with("HBNX_") && name.ends_with(".upk")
            })
            .difference(&old_copies)
            {
                let _ = std::fs::remove_file(new_copy);
            }
            Err(error)
        }
    }
}

fn apply_inner(
    cooked: &Path,
    host_path: &Path,
    donor_name: &str,
    donor: &PreparedScenery,
) -> Result<(String, Vec<String>), String> {
    let mut host = UpkPackage::load(host_path)?;
    let slot = host
        .find_stream_name_index()
        .ok_or("This map has no streaming slot and cannot host a background")?;
    let slot_name = host.names[slot].clone();
    let mut copy_name = make_copy_name(&donor_copy_label(donor_name), slot_name.len())?;
    if !donor.generated.contains(&copy_name) {
        copy_name = available_copy_name(cooked, copy_name)?;
    }
    let generated = map_path(cooked, &copy_name);
    if !donor.generated.contains(&copy_name) || !generated.is_file() {
        std::fs::write(&generated, &donor.scenery)
            .map_err(|e| format!("Could not create donor scenery package: {e}"))?;
    }
    let preserve_host_atmosphere = donor.world.is_none();
    let stream_indices = host.find_all_stream_name_indices();
    let mut patched_sub_levels = Vec::new();
    for index in stream_indices.into_iter().filter(|i| *i != slot) {
        let name = host.names[index].clone();
        if !visible_sublevel(&name) {
            continue;
        }
        let path = map_path(cooked, &name);
        if !path.is_file() {
            continue;
        }
        let backup = backup_path(&path);
        if !backup.exists() {
            std::fs::copy(&path, &backup)
                .map_err(|e| format!("Could not back up sub-level {name}: {e}"))?;
        }
        let mut sub = UpkPackage::load(&backup)?;
        let mut patches =
            host_sky_patches(&sub, preserve_host_atmosphere)?;
        if !preserve_host_atmosphere {
            patches.extend(disable_old_atmosphere(&sub)?);
        }
        apply_patches(&mut sub, patches)?;
        super::upk_history::install_linked(cooked, &path, &sub.repacked_fast()?, "Arena background")?;
        patched_sub_levels.push(name);
    }
    host.rename_name(slot, &copy_name)?;
    let mut patches = host_sky_patches(&host, preserve_host_atmosphere)?;
    if let Some(world) = &donor.world {
        patches.extend(disable_old_atmosphere(&host)?);
        patches.extend(world_visual_patches(&host, world)?);
    }
    apply_patches(&mut host, patches)?;
    super::upk_history::install_linked(cooked, host_path, &host.repacked_fast()?, "Arena background")?;
    Ok((copy_name, patched_sub_levels))
}

fn remove_background_inner(
    cooked: &Path,
    host_path: &Path,
    donor_name: &str,
) -> Result<(String, Vec<String>), String> {
    let mut host = UpkPackage::load(host_path)?;
    let slot = host
        .find_stream_name_index()
        .ok_or("This map has no streaming slot and cannot host a background")?;
    let slot_name = host.names[slot].clone();
    let copy_name = make_copy_name("EMPTY", slot_name.len())?;
    let generated = map_path(cooked, &copy_name);
    let mut generated_pkg = UpkPackage::load(&map_path(cooked, donor_name))?;
    clear_scenery(&mut generated_pkg)?;
    generated_pkg.save_fast(&generated)?;
    let stream_indices = host.find_all_stream_name_indices();
    let mut patched_sub_levels = Vec::new();
    for index in stream_indices.into_iter().filter(|i| *i != slot) {
        let name = host.names[index].clone();
        if !visible_sublevel(&name) {
            continue;
        }
        let path = map_path(cooked, &name);
        if !path.is_file() {
            continue;
        }
        let backup = backup_path(&path);
        if !backup.exists() {
            std::fs::copy(&path, &backup)
                .map_err(|e| format!("Could not back up sub-level {name}: {e}"))?;
        }
        let mut sub = UpkPackage::load(&backup)?;
        let mut patches = host_sky_patches(&sub, false)?;
        patches.extend(disable_old_atmosphere(&sub)?);
        apply_patches(&mut sub, patches)?;
        super::upk_history::install_linked(cooked, &path, &sub.repacked_fast()?, "Arena background")?;
        patched_sub_levels.push(name);
    }
    host.rename_name(slot, &copy_name)?;
    let mut patches = host_sky_patches(&host, false)?;
    patches.extend(disable_old_atmosphere(&host)?);
    apply_patches(&mut host, patches)?;
    super::upk_history::install_linked(cooked, host_path, &host.repacked_fast()?, "Arena background")?;
    Ok((copy_name, patched_sub_levels))
}
fn patch_donor_outside_only(
    package: &mut UpkPackage,
    permissive_custom: bool,
) -> Result<usize, String> {
    let mut patches = Vec::new();
    let mut retained = 0usize;
    for e in &package.exports {
        let class_name = package.class_of(e);
        let class = strip(&class_name);
        let mesh_name = match class {
            "StaticMeshComponent" | "InstancedStaticMeshComponent" => Some("StaticMesh"),
            "SkeletalMeshComponent" => Some("SkeletalMesh"),
            _ => None,
        };
        let particle = class == "ParticleSystemComponent";
        let gameplay_actor = is_gameplay_actor_class(class);
        let geometry = matches!(class, "BrushComponent" | "ModelComponent");
        let collision = class.contains("Collision") || class == "BrushComponent";
        if mesh_name.is_none() && !particle && !gameplay_actor && !geometry && !collision {
            continue;
        }
        let props = package.parse_props(e);
        let mesh_prop = mesh_name.and_then(|name| {
            props.iter().find(|p| p.name == name && p.tag_type == "ObjectProperty")
        });
        if let Some(p) = mesh_prop {
            let off = e.serial_offset + p.value_offset;
            let reference = package.read_int(off)?;
            if reference != 0 {
                let mesh_name = package.obj_name(reference);
                let mesh = strip(&mesh_name);
                if if permissive_custom {
                    custom_donor_outside_scope(mesh)
                } else {
                    donor_outside_scope(mesh)
                } {
                    retained += 1
                } else {
                    patches.push((off, vec![0; 4]))
                }
            }
        }
        if particle {
            if let Some(p) = props
                .iter()
                .find(|p| p.name == "Template" && p.tag_type == "ObjectProperty")
            {
                let off = e.serial_offset + p.value_offset;
                let reference = package.read_int(off)?;
                if reference != 0 {
                    if ambient_particle(package, e, reference) {
                        retained += 1
                    } else {
                        patches.push((off, vec![0; 4]))
                    }
                }
            }
        }
        if gameplay_actor {
            for p in &props {
                if p.tag_type == "ObjectProperty" {
                    let off = e.serial_offset + p.value_offset;
                    if package.read_int(off)? != 0 {
                        patches.push((off, vec![0; 4]));
                    }
                }
            }
        }
        if geometry {
            for p in &props {
                if p.tag_type == "ObjectProperty"
                    && (p.name == "Brush" || p.name == "Model" || p.name == "BrushAggGeom")
                {
                    let off = e.serial_offset + p.value_offset;
                    if package.read_int(off)? != 0 {
                        patches.push((off, vec![0; 4]));
                    }
                }
            }
        }
        if mesh_prop.is_some() || collision || gameplay_actor {
            for p in &props {
                if p.tag_type == "BoolProperty"
                    && (p.name.contains("Collide")
                        || p.name.contains("Block")
                        || p.name.contains("RigidBody"))
                {
                    patches.push((e.serial_offset + p.value_offset - 1, vec![0]))
                }
            }
        }
    }
    apply_patches(package, patches)?;
    Ok(retained)
}

/// Parses and filters a custom background in memory without changing the source file.
/// This is also useful to validate newly dropped-in packages before applying them.
#[allow(dead_code)]
pub fn analyze_custom_background(path: &Path) -> Result<usize, String> {
    let mut package = UpkPackage::load(path)?;
    patch_donor_outside_only(&mut package, true)
}

#[allow(dead_code)]
pub fn custom_background_mesh_report(path: &Path) -> Result<Vec<(bool, String, String)>, String> {
    let package = UpkPackage::load(path)?;
    let mut report = Vec::new();
    for export in &package.exports {
        let class_name = package.class_of(export);
        let class = strip(&class_name);
        let property = match class {
            "StaticMeshComponent" | "InstancedStaticMeshComponent" => "StaticMesh",
            "SkeletalMeshComponent" => "SkeletalMesh",
            _ => continue,
        };
        let Some(prop) = package
            .parse_props(export)
            .into_iter()
            .find(|prop| prop.name == property && prop.tag_type == "ObjectProperty")
        else {
            continue;
        };
        let reference = package.read_int(export.serial_offset + prop.value_offset)?;
        if reference == 0 {
            continue;
        }
        let mesh = strip(&package.obj_name(reference)).to_string();
        report.push((
            custom_donor_outside_scope(&mesh),
            mesh,
            package.object_path(export.table_index as i32 + 1),
        ));
    }
    Ok(report)
}

fn clear_scenery(package: &mut UpkPackage) -> Result<(), String> {
    let mut patches = Vec::new();
    for e in &package.exports {
        let class_name = package.class_of(e);
        let class = strip(&class_name);
        let props = package.parse_props(e);
        let mesh_property = match class {
            "StaticMeshComponent" | "InstancedStaticMeshComponent" => "StaticMesh",
            "SkeletalMeshComponent" => "SkeletalMesh",
            _ => "",
        };
        for p in &props {
            let offset = e.serial_offset + p.value_offset;
            if (!mesh_property.is_empty()
                && p.name == mesh_property
                && p.tag_type == "ObjectProperty")
                || (class == "ParticleSystemComponent"
                    && p.name == "Template"
                    && p.tag_type == "ObjectProperty")
            {
                patches.push((offset, vec![0; 4]));
            }
            if p.tag_type == "BoolProperty"
                && (p.name.contains("Collide")
                    || p.name.contains("Block")
                    || p.name.contains("RigidBody"))
            {
                patches.push((offset - 1, vec![0]));
            }
        }
    }
    apply_patches(package, patches)
}
fn host_sky_patches(package: &UpkPackage, preserve_sky: bool) -> Result<Vec<Patch>, String> {
    let mut patches = Vec::new();
    for e in &package.exports {
        let class_name = package.class_of(e);
        let class = strip(&class_name);
        if class != "StaticMeshComponent"
            && class != "InstancedStaticMeshComponent"
            && class != "SkeletalMeshComponent"
        {
            continue;
        }
        let prop_name = if class == "SkeletalMeshComponent" {
            "SkeletalMesh"
        } else {
            "StaticMesh"
        };
        if let Some(p) = package
            .parse_props(e)
            .into_iter()
            .find(|p| p.name == prop_name && p.tag_type == "ObjectProperty")
        {
            let off = e.serial_offset + p.value_offset;
            let reference = package.read_int(off)?;
            let mesh_name = package.obj_name(reference);
            let in_replaced_scope = if preserve_sky {
                is_backdrop(&mesh_name) || is_atmosphere_visual(&mesh_name)
            } else {
                host_background_scope(&mesh_name)
            };
            if reference != 0 && in_replaced_scope {
                patches.push((off, vec![0; 4]))
            }
        }
    }
    Ok(patches)
}

pub(super) fn disable_old_atmosphere(package: &UpkPackage) -> Result<Vec<Patch>, String> {
    const GLOBAL: &[&str] = &[
        "DirectionalLightComponent",
        "DominantDirectionalLightComponent",
        "SkyLightComponent",
        "SkyLightVolumeComponent_TA",
        "PointLightComponent",
        "SpotLightComponent",
        "PointLightComponent_TA",
        "SpotLightComponent_TA",
        "DominantSpotLightComponent",
    ];
    let mut patches = Vec::new();
    for e in &package.exports {
        let class_name = package.class_of(e);
        let class = strip(&class_name);
        let props = package.parse_props(e);
        if GLOBAL.contains(&class) {
            for p in &props {
                let off = e.serial_offset + p.value_offset;
                if p.tag_type == "FloatProperty" && p.name == "Brightness" {
                    patches.push((off, vec![0; 4]))
                } else if p.tag_type == "StructProperty"
                    && ["LightColor", "LowerColor", "UpperColor"].contains(&p.name.as_str())
                {
                    patches.push((off, vec![0; p.size.min(4)]))
                } else if p.tag_type == "BoolProperty" && p.name == "bEnabled" {
                    patches.push((off - 1, vec![0]))
                }
            }
        } else if class == "ExponentialHeightFogComponent" || class == "HeightFogComponent" {
            if let Some(p) = props
                .iter()
                .find(|p| p.name == "FogDensity" && p.tag_type == "FloatProperty")
            {
                patches.push((e.serial_offset + p.value_offset, vec![0; 4]))
            }
        } else if class == "LensFlareComponent" {
            if let Some(p) = props
                .iter()
                .find(|p| p.name == "Template" && p.tag_type == "ObjectProperty")
            {
                patches.push((e.serial_offset + p.value_offset, vec![0; 4]))
            }
        }
    }
    Ok(patches)
}

pub(super) fn world_visual_patches(host: &UpkPackage, donor: &UpkPackage) -> Result<Vec<Patch>, String> {
    let Some(he) = host
        .exports
        .iter()
        .find(|e| strip(&host.class_of(e)) == "WorldInfo")
    else {
        return Ok(Vec::new());
    };
    let Some(de) = donor
        .exports
        .iter()
        .find(|e| strip(&donor.class_of(e)) == "WorldInfo")
    else {
        return Ok(Vec::new());
    };
    let donor_props = donor.parse_props(de);
    let mut by_name: HashMap<&str, &Prop> = HashMap::new();
    for p in &donor_props {
        by_name.insert(&p.name, p);
    }
    let mut patches = Vec::new();
    for hp in host.parse_props(he) {
        if !visual_world_property(&hp.name) {
            continue;
        }
        let Some(dp) = by_name.get(hp.name.as_str()) else {
            continue;
        };
        if hp.tag_type != dp.tag_type || hp.size != dp.size {
            continue;
        }
        if hp.tag_type == "BoolProperty" {
            patches.push((
                he.serial_offset + hp.value_offset - 1,
                vec![u8::from(dp.bool_value.unwrap_or(false))],
            ));
        } else if [
            "FloatProperty",
            "IntProperty",
            "StructProperty",
            "ByteProperty",
        ]
        .contains(&hp.tag_type.as_str())
        {
            let start = de.serial_offset + dp.value_offset;
            let data = donor
                .image
                .get(start..start + dp.size)
                .ok_or("Donor WorldInfo property is outside its export")?
                .to_vec();
            patches.push((he.serial_offset + hp.value_offset, data));
        }
    }
    Ok(patches)
}

fn visual_world_property(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        "fog",
        "scene_",
        "ambient",
        "environment",
        "colorgrading",
        "colourgrading",
        "postprocess",
        "bloom",
        "dof",
        "depthoffield",
        "motionblur",
        "tonemap",
        "lut",
        "exposure",
        "sky",
        "sun",
        "lightmass",
    ]
    .iter()
    .any(|x| n.contains(x))
}
fn ambient_particle(package: &UpkPackage, e: &ExportEntry, reference: i32) -> bool {
    let mut description = package.obj_name(reference);
    let mut outer = e.outer_index;
    for _ in 0..8 {
        if outer <= 0 {
            break;
        }
        let Some(owner) = package.exports.get((outer - 1) as usize) else {
            break;
        };
        description.push(' ');
        description.push_str(&package.name_of(owner.object_name));
        outer = owner.outer_index;
    }
    let lower = description.to_ascii_lowercase();
    ![
        "boost",
        "explosion",
        "goal",
        "pickup",
        "ball",
        "centerpiece",
    ]
    .iter()
    .any(|x| lower.contains(x))
}
pub(super) fn is_gameplay_actor_class(class: &str) -> bool {
    let n = class.to_ascii_lowercase();
    [
        "gameevent",
        "gameinfo",
        "goal",
        "pylon_soccar",
        "playerstart",
        "spawnpoint",
        "spectatorvolume",
        "trigger",
        "vehiclepickup",
        "boostpickup",
    ]
    .iter()
    .any(|hint| n.contains(hint))
}

fn donor_outside_scope(mesh: &str) -> bool {
    if is_gameplay_structure(mesh) {
        return false;
    }
    is_core_background(mesh) || is_extended_scenery(mesh)
}

fn custom_donor_outside_scope(mesh: &str) -> bool {
    !is_gameplay_structure(mesh) && !is_custom_arena_shell(mesh)
}

fn is_custom_arena_shell(mesh: &str) -> bool {
    let n = mesh.to_ascii_lowercase();
    [
        "oobfloor",
        "side_frame",
        "side_trim",
        "inner_glass",
        "midglass",
        "midhexglass",
        "adframes",
        "matineecam",
    ]
    .iter()
    .any(|hint| n.contains(hint))
        || matches!(n.as_str(), "net" | "ad1" | "ad2")
}

fn host_background_scope(mesh: &str) -> bool {
    if is_gameplay_structure(mesh) {
        return false;
    }
    is_core_background(mesh) || is_extended_scenery(mesh)
}

fn is_core_background(mesh: &str) -> bool {
    is_sky(mesh) || is_backdrop(mesh) || is_atmosphere_visual(mesh)
}

fn is_sky(mesh: &str) -> bool {
    let n = mesh.to_ascii_lowercase();
    !n.contains("skyscraper")
        && (n.contains("skysphere")
            || n.contains("skydome")
            || n.contains("skybox")
            || n.starts_with("sky")
            || n.contains("cloud")
            || n.contains("moon")
            || n.contains("aurora")
            || n.contains("starfield")
            || n.contains("sky_rift"))
}

fn is_backdrop(mesh: &str) -> bool {
    let n = mesh.to_ascii_lowercase();
    n.contains("_oob")
        || n.starts_with("oob_")
        || n.contains("mountain")
        || n.contains("skyscraper")
        || n.contains("cityground")
        || n.starts_with("city_")
        || n.contains("skyline")
        || n.contains("backdrop")
        || n.contains("distant")
        || n.contains("horizon")
        || n.contains("treeline")
}

fn is_atmosphere_visual(mesh: &str) -> bool {
    let n = mesh.to_ascii_lowercase();
    [
        "fog",
        "mist",
        "haze",
        "smoke",
        "lightcone",
        "lightbeam",
        "godray",
    ]
    .iter()
    .any(|hint| n.contains(hint))
}

fn is_extended_scenery(mesh: &str) -> bool {
    let n = mesh.to_ascii_lowercase();
    [
        "building",
        "tower",
        "bridge",
        "skyway",
        "tree",
        "terrain",
        "rock",
        "roof",
        "water",
        "pond",
        "moat",
        "streetsign",
        "balloon",
        "planter",
        "bush",
        "road",
        "floor",
        "station",
        "slums",
        "factory",
        "silo",
        "scraper",
        "subway",
        "track",
        "tram",
        "antenna",
        "sign",
        "loft",
        "adframe",
        "escalator",
        "satdish",
        "hub_",
        "hubbase",
        "hubpipes",
    ]
    .iter()
    .any(|hint| n.contains(hint))
}

fn is_gameplay_structure(mesh: &str) -> bool {
    let n = mesh.to_ascii_lowercase();
    [
        "goal",
        "field_",
        "field.",
        "field-",
        "cs_field",
        "playingfield",
        "playfield",
        "pitch",
        "soccar",
        "boostpad",
        "collision",
        "centerpiece",
        "field_wall",
        "fieldwall",
        "fieldfloor",
        "grassfield",
        "stadium",
        "arena",
        "cage",
        "seat",
        "bleacher",
        "centerwall",
        "staircasewall",
        "path_lowpoly_wall",
        "ball_defaultball",
        "circle_sprite",
        "bleacheregg",
        "simplicity_",
    ]
    .iter()
    .any(|hint| n.contains(hint))
        && !is_atmosphere_visual(mesh)
}

pub(super) fn visible_sublevel(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    !["sfx", "audio", "sound", "ambient", "tutorial"]
        .iter()
        .any(|x| n.contains(x))
}
pub(super) fn apply_patches(package: &mut UpkPackage, patches: Vec<Patch>) -> Result<(), String> {
    for (offset, data) in patches {
        package.patch(offset, &data)?
    }
    Ok(())
}
pub(super) fn make_copy_name(donor: &str, len: usize) -> Result<String, String> {
    if len < 6 {
        return Err("This host's streaming slot is too short".into());
    }
    let mut name = format!("HBNX_{donor}");
    if name.len() < len {
        name.push_str(&"_".repeat(len - name.len()))
    }
    name.truncate(len);
    Ok(name)
}

fn available_copy_name(cooked: &Path, preferred: String) -> Result<String, String> {
    if !map_path(cooked, &preferred).exists() {
        return Ok(preferred);
    }
    // The streaming name must keep its original byte length. Truncation can
    // make two donors share a name, especially day and night variants.
    let digits = (preferred.len() - "HBNX_".len()).min(4);
    let prefix = &preferred[..preferred.len() - digits];
    for index in 0..(1usize << (4 * digits)) {
        let candidate = format!("{prefix}{index:0digits$x}");
        if !map_path(cooked, &candidate).exists() {
            return Ok(candidate);
        }
    }
    Err("No unused scenery package name fits this arena's streaming slot.".into())
}

pub(super) fn donor_copy_label(donor: &str) -> String {
    donor
        .strip_prefix(CUSTOM_DONOR_PREFIX)
        .and_then(|name| Path::new(name).file_stem())
        .and_then(|name| name.to_str())
        .unwrap_or(donor)
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

pub fn custom_backgrounds_dir() -> Result<PathBuf, String> {
    let executable = std::env::current_exe()
        .map_err(|e| format!("Could not locate the Hebnix executable: {e}"))?;
    let parent = executable
        .parent()
        .ok_or("The Hebnix executable has no parent folder")?;
    Ok(parent.join("backgrounds"))
}

fn custom_donor_path(donor: &str) -> Result<PathBuf, String> {
    let file_name = donor
        .strip_prefix(CUSTOM_DONOR_PREFIX)
        .ok_or("Invalid custom background identifier")?;
    let candidate = Path::new(file_name);
    if candidate.file_name().and_then(|name| name.to_str()) != Some(file_name) {
        return Err("Invalid custom background filename.".into());
    }
    let extension = candidate
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !extension.eq_ignore_ascii_case("upk") && !extension.eq_ignore_ascii_case("udk") {
        return Err("Custom backgrounds must be .upk or .udk files.".into());
    }
    Ok(custom_backgrounds_dir()?.join(candidate))
}

fn undo(cooked: &Path, state_dir: &Path, host: &str) -> Result<String, String> {
    validate_name(host)?;
    ensure_game_closed("restoring an arena")?;
    let mut state = load_state(state_dir)?;
    if !state.contains_key(host) {
        return Err("That arena does not have an active background change.".into());
    }
    undo_core(cooked, &mut state, host)?;
    save_state(state_dir, &state)?;
    Ok(format!(
        "Restored {}'s original background.",
        friendly(host)
    ))
}
fn reset(cooked: &Path, state_dir: &Path) -> Result<String, String> {
    ensure_game_closed("restoring arenas")?;
    let mut state = load_state(state_dir)?;
    let hosts: Vec<String> = state.keys().cloned().collect();
    for host in hosts {
        undo_core(cooked, &mut state, &host)?
    }
    save_state(state_dir, &state)?;
    Ok("Restored all original arena backgrounds.".into())
}
fn undo_core(cooked: &Path, state: &mut State, host: &str) -> Result<(), String> {
    let Some(swap) = state.remove(host) else {
        return Ok(());
    };
    let live = map_path(cooked, host);
    let backup = backup_path(&live);
    if backup.is_file() {
        restore_file(cooked, &backup, &live)?
    }
    for sub in &swap.patched_sub_levels {
        let live = map_path(cooked, sub);
        let backup = backup_path(&live);
        if backup.is_file() {
            restore_file(cooked, &backup, &live)?
        }
    }
    for generated_name in std::iter::once(&swap.copy_name).chain(&swap.extra_copies) {
        let still_needed = state.values().any(|other| {
            other.copy_name.eq_ignore_ascii_case(generated_name)
                || other
                    .extra_copies
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(generated_name))
        });
        if !still_needed {
            let generated = map_path(cooked, generated_name);
            if generated.exists() {
                std::fs::remove_file(&generated)
                    .map_err(|e| format!("Could not remove {}: {e}", generated.display()))?;
            }
        }
    }
    Ok(())
}
fn restore_file(cooked: &Path, backup: &Path, live: &Path) -> Result<(), String> {
    super::upk_history::install_linked(cooked, live, &std::fs::read(backup).map_err(|e| e.to_string())?, "Restore arena background")
        .map_err(|e| format!("Could not restore {}: {e}", live.display()))?;
    std::fs::remove_file(backup)
        .map_err(|e| format!("Could not remove backup {}: {e}", backup.display()))
}
fn load_state(dir: &Path) -> Result<State, String> {
    let path = dir.join(MANIFEST);
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let bytes =
        std::fs::read(&path).map_err(|e| format!("Could not read background state: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("Background state is invalid: {e}"))
}
fn save_state(dir: &Path, state: &State) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Could not create state folder: {e}"))?;
    let path = dir.join(MANIFEST);
    let temp = dir.join(format!("{MANIFEST}.tmp"));
    let bytes = serde_json::to_vec(state).map_err(|e| format!("Could not serialize state: {e}"))?;
    std::fs::write(&temp, bytes).map_err(|e| format!("Could not write state: {e}"))?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("Could not replace state: {e}"))?
    }
    std::fs::rename(temp, path).map_err(|e| format!("Could not finish state update: {e}"))
}
fn ensure_game_closed(action: &str) -> Result<(), String> {
    if hebnix_sdk::process::detector::rocket_league_running_checked()
        .map_err(|error| format!("Could not verify Rocket League is closed: {error}"))?
    {
        Err(format!("Close Rocket League before {action}."))
    } else {
        Ok(())
    }
}
fn matching_files(cooked: &Path, predicate: impl Fn(&str) -> bool) -> HashSet<PathBuf> {
    std::fs::read_dir(cooked)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(&predicate)
        })
        .collect()
}
fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        Err("Invalid arena package name.".into())
    } else {
        Ok(())
    }
}
fn package_source(cooked: &Path, name: &str) -> Result<PathBuf, String> {
    let live = map_path(cooked, name);
    let backup = backup_path(&live);
    let source = if backup.is_file() { backup } else { live };
    if source.is_file() {
        Ok(source)
    } else {
        Err(format!(
            "Required background scenery package is missing: {name}.upk"
        ))
    }
}
fn map_path(cooked: &Path, name: &str) -> PathBuf {
    cooked.join(format!("{name}.upk"))
}
fn backup_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}{BACKUP_SUFFIX}", path.display()))
}
fn friendly(name: &str) -> String {
    name.replace('_', " ").trim().to_string()
}

#[cfg(test)]
mod arena_roundtrip {
    use super::*;

    /// HBNX_COOKED=<CookedPCConsole> HBNX_WORK=<scratch folder>
    /// cargo test -p hebnix-app arena_roundtrip -- --ignored --nocapture
    #[test]
    #[ignore = "copies about 100 MB from a Rocket League install"]
    fn apply_then_undo_restores_every_byte() {
        let source = PathBuf::from(std::env::var("HBNX_COOKED").expect("set HBNX_COOKED"));
        let work = PathBuf::from(std::env::var("HBNX_WORK").expect("set HBNX_WORK"));
        assert!(!work.starts_with(&source), "HBNX_WORK must not be inside the game folder");
        let cooked = work.join("cooked");
        let state_dir = work.join("state");
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&cooked).unwrap();
        let names = [
            "Underwater_P",
            "Underwater_FX",
            "Underwater_Field",
            "Underwater_Lights",
            "Underwater_Plants",
            "Underwater_OOB_foliage",
            "Underwater_SFX",
            "Park_P",
        ];
        let mut before = std::collections::HashMap::new();
        for name in names {
            let file = format!("{name}.upk");
            std::fs::copy(source.join(&file), cooked.join(&file)).unwrap();
            before.insert(file.clone(), std::fs::read(cooked.join(&file)).unwrap());
        }

        let message = apply(&cooked, &state_dir, "Underwater_P", "arena:Park_P").unwrap();
        println!("{message}");
        assert_ne!(std::fs::read(cooked.join("Underwater_P.upk")).unwrap(), before["Underwater_P.upk"]);
        assert_eq!(std::fs::read(cooked.join("Park_P.upk")).unwrap(), before["Park_P.upk"]);
        assert_eq!(matching_files(&cooked, |n| n.starts_with("HBNX_")).len(), 1);

        let mut state = load_state(&state_dir).unwrap();
        assert!(state["Underwater_P"].donor.starts_with(ARENA_DONOR_PREFIX));
        undo_core(&cooked, &mut state, "Underwater_P").unwrap();
        save_state(&state_dir, &state).unwrap();

        for (file, bytes) in &before {
            assert_eq!(&std::fs::read(cooked.join(file)).unwrap(), bytes, "{file} differs after undo");
        }
        assert!(matching_files(&cooked, |n| n.starts_with("HBNX_")).is_empty());
        assert!(matching_files(&cooked, |n| n.ends_with(BACKUP_SUFFIX)).is_empty());
        let _ = std::fs::remove_dir_all(&work);
    }
}
