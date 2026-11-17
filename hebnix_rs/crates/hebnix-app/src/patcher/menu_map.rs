//! Complete menu-arena replacement. Gameplay packages are only read; menu-only
//! copies contain the field, goals and recursively discovered streaming layers.
use super::{
    backup_guard, game_path,
    upk_package::{UpkPackage, strip},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MENU: &str = "MENU_Main_p.upk";
const STATE: &str = ".hebnix-menu-map";
const MANIFEST: &str = "manifest.json";
const BACKUP: &str = "original.bin";

#[derive(Clone, Debug)]
pub struct Arena {
    pub package: String,
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MenuFile {
    name: String,
    sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActiveSwap {
    /// The vanilla arena package, or the map id for a workshop map.
    pub arena: String,
    /// Display name of a workshop map. Absent for vanilla arenas.
    #[serde(default)]
    pub label: Option<String>,
    /// Experimental camera changes applied with the swap, one per camera.
    #[serde(default)]
    pub cameras: Vec<CameraTweak>,
    /// Older manifests stored a single camera change here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    camera: Option<CameraTweak>,
    /// Experimental move of the car spots, which the home-screen camera follows.
    #[serde(default)]
    pub car: Option<CarTweak>,
    build: String,
    original_sha256: String,
    patched_sha256: String,
    files: Vec<MenuFile>,
}

/// Rotator units per degree (UE3 stores a full turn as 65536).
const ROTATOR_PER_DEGREE: f32 = 65536.0 / 360.0;

/// A camera placed in the menu level, as the original menu stores it.
#[derive(Clone, Debug)]
pub struct MenuCamera {
    /// Object name, e.g. `CameraActor_2`. Identifies the camera in a tweak.
    pub name: String,
    /// The level designer's tag, e.g. `MainMenuPlay`.
    pub tag: String,
    pub location: [f32; 3],
    /// Pitch, yaw, roll in degrees.
    pub rotation: [f32; 3],
    /// Absent when the camera keeps the engine default.
    pub fov: Option<f32>,
}

/// Offsets added to one menu camera. Positions are in Unreal units, angles in
/// degrees, `fov` replaces the camera's own field of view.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CameraTweak {
    pub camera: String,
    pub offset: [f32; 3],
    pub pitch: f32,
    pub yaw: f32,
    #[serde(default)]
    pub fov: Option<f32>,
}

impl CameraTweak {
    pub fn is_noop(&self) -> bool {
        self.offset == [0.0; 3] && self.pitch == 0.0 && self.yaw == 0.0 && self.fov.is_none()
    }
}

/// Moves the cars shown on the home screen. The home camera is not one of the
/// level's cameras: it orbits the player's car, so moving the car moves it.
/// The party slots turn with the player's car so the line-up stays together.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CarTweak {
    pub offset: [f32; 3],
    pub yaw: f32,
}

impl CarTweak {
    pub fn is_noop(&self) -> bool {
        self.offset == [0.0; 3] && self.yaw == 0.0
    }
}

/// Everything changed in the menu besides the arena.
#[derive(Clone, Debug, Default)]
pub struct MenuTweaks {
    pub cameras: Vec<CameraTweak>,
    pub car: Option<CarTweak>,
}

/// The player's car spot. The other `CarPreviewActor_TA_<n>` spots hold party members.
const PLAYER_CAR: &str = "CarPreviewActor_TA";

/// A car spot on the home screen, as the original menu stores it.
#[derive(Clone, Debug)]
pub struct CarSpot {
    pub name: String,
    pub location: [f32; 3],
    /// Degrees.
    pub yaw: f32,
}

/// Byte offsets of a camera's editable values in the decompressed image.
struct CameraSlots {
    location: usize,
    rotation: Option<usize>,
    fov: Option<usize>,
}

fn name_index(package: &UpkPackage, name: &str) -> Option<i32> {
    package
        .names
        .iter()
        .position(|n| n == name)
        .map(|i| i as i32)
}

/// Find a property's value by its serialized tag inside one export. The
/// generic property walker stops at CameraActor's post-process struct, so the
/// tag is matched directly: name, type, size, array index and, for structs,
/// the struct name.
fn find_tagged(
    package: &UpkPackage,
    range: std::ops::Range<usize>,
    prop: &str,
    kind: &str,
    struct_name: Option<&str>,
    size: i32,
) -> Option<usize> {
    let mut tag = Vec::new();
    for value in [
        name_index(package, prop)?,
        0,
        name_index(package, kind)?,
        0,
        size,
        0,
    ] {
        tag.extend_from_slice(&value.to_le_bytes());
    }
    if let Some(struct_name) = struct_name {
        tag.extend_from_slice(&name_index(package, struct_name)?.to_le_bytes());
        tag.extend_from_slice(&0i32.to_le_bytes());
    }
    let end = range.end.min(package.image.len());
    let bytes = package.image.get(range.start..end)?;
    let at = bytes.windows(tag.len()).position(|w| w == tag.as_slice())?;
    let value = range.start + at + tag.len();
    (value + size as usize <= end).then_some(value)
}

fn camera_slots(package: &UpkPackage, e: &super::upk_package::ExportEntry) -> Option<CameraSlots> {
    let range = e.serial_offset..e.serial_offset + e.serial_size;
    Some(CameraSlots {
        location: find_tagged(
            package,
            range.clone(),
            "Location",
            "StructProperty",
            Some("Vector"),
            12,
        )?,
        rotation: find_tagged(
            package,
            range.clone(),
            "Rotation",
            "StructProperty",
            Some("Rotator"),
            12,
        ),
        fov: find_tagged(package, range, "FOVAngle", "FloatProperty", None, 4),
    })
}

fn read_f32(package: &UpkPackage, offset: usize) -> Result<f32, String> {
    Ok(f32::from_bits(package.read_int(offset)? as u32))
}

fn exports_of<'a>(
    package: &'a UpkPackage,
    class: &'a str,
) -> impl Iterator<Item = (&'a super::upk_package::ExportEntry, String)> {
    package
        .exports
        .iter()
        .filter(move |e| strip(&package.class_of(e)) == class)
        .map(|e| (e, package.name_of(e.object_name)))
}

fn camera_exports(
    package: &UpkPackage,
) -> impl Iterator<Item = (&super::upk_package::ExportEntry, String)> {
    exports_of(package, "CameraActor")
}

/// Location and rotation offsets of each placed car spot. The class default
/// object has no location and is skipped.
fn car_slots(package: &UpkPackage) -> Vec<(String, usize, Option<usize>)> {
    exports_of(package, "CarPreviewActor_TA")
        .filter_map(|(e, name)| {
            let range = e.serial_offset..e.serial_offset + e.serial_size;
            let location = find_tagged(
                package,
                range.clone(),
                "Location",
                "StructProperty",
                Some("Vector"),
                12,
            )?;
            let rotation = find_tagged(
                package,
                range,
                "Rotation",
                "StructProperty",
                Some("Rotator"),
                12,
            );
            Some((name, location, rotation))
        })
        .collect()
}

fn read_vector(package: &UpkPackage, at: usize) -> Result<[f32; 3], String> {
    Ok([
        read_f32(package, at)?,
        read_f32(package, at + 4)?,
        read_f32(package, at + 8)?,
    ])
}

fn read_car_spots(package: &UpkPackage) -> Result<Vec<CarSpot>, String> {
    let mut spots = car_slots(package)
        .into_iter()
        .map(|(name, location, rotation)| {
            let yaw = match rotation {
                Some(at) => package.read_int(at + 4)? as f32 / ROTATOR_PER_DEGREE,
                None => 0.0,
            };
            Ok(CarSpot {
                name,
                location: read_vector(package, location)?,
                yaw,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    spots.sort_by(|a, b| (a.name != PLAYER_CAR, &a.name).cmp(&(b.name != PLAYER_CAR, &b.name)));
    Ok(spots)
}

/// Values saved by the MenuCam BakkesMod plugin's Export for Hebnix button
/// (`bakkesmod/data/menucam/hebnix.json`). Its numbers mean the same as
/// Hebnix's own camera and car tweaks.
#[derive(Debug, Default, Deserialize)]
pub struct MenuCamExport {
    #[serde(default)]
    pub car: Option<CarTweak>,
    #[serde(default)]
    pub cameras: Vec<CameraTweak>,
}

pub fn read_menucam_export(path: &Path) -> Result<MenuCamExport, String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|e| format!("{} is not a MenuCam export: {e}", path.display()))
}

/// The home screen's car spots in the unmodified menu, the player's first.
pub fn car_spots(cooked: &Path) -> Result<Vec<CarSpot>, String> {
    let backup = state_dir(cooked).join(BACKUP);
    let path = if backup.is_file() {
        backup
    } else {
        menu(cooked)
    };
    read_car_spots(&UpkPackage::load(&path)?)
}

fn apply_car(menu: &mut UpkPackage, tweak: &CarTweak) -> Result<(), String> {
    let slots = car_slots(menu);
    let pivot = slots
        .iter()
        .find(|(name, ..)| name == PLAYER_CAR)
        .map(|(_, at, _)| read_vector(menu, *at))
        .transpose()?
        .ok_or("This game version's menu has no car spot to move.")?;
    let (sin, cos) = tweak.yaw.to_radians().sin_cos();
    for (_, location, rotation) in slots {
        let [x, y, z] = read_vector(menu, location)?;
        let (dx, dy) = (x - pivot[0], y - pivot[1]);
        // Turn the line-up around the player's car (UE yaw turns X towards Y).
        let moved = [
            pivot[0] + dx * cos - dy * sin + tweak.offset[0],
            pivot[1] + dx * sin + dy * cos + tweak.offset[1],
            z + tweak.offset[2],
        ];
        for (i, value) in moved.iter().enumerate() {
            menu.patch(location + i * 4, &value.to_le_bytes())?;
        }
        if tweak.yaw != 0.0 {
            let at = rotation.ok_or("A car spot has no stored rotation to change.")?;
            let yaw = menu.read_int(at + 4)? + (tweak.yaw * ROTATOR_PER_DEGREE).round() as i32;
            menu.patch_i32(at + 4, yaw)?;
        }
    }
    Ok(())
}

fn read_cameras(package: &UpkPackage) -> Result<Vec<MenuCamera>, String> {
    let mut cameras = Vec::new();
    for (e, name) in camera_exports(package) {
        let Some(slots) = camera_slots(package, e) else {
            continue;
        };
        let range = e.serial_offset..e.serial_offset + e.serial_size;
        let tag = find_tagged(package, range, "Tag", "NameProperty", None, 8)
            .and_then(|at| package.read_int(at).ok())
            .and_then(|i| package.names.get(i as usize).cloned())
            .unwrap_or_default();
        let mut location = [0.0; 3];
        for (i, value) in location.iter_mut().enumerate() {
            *value = read_f32(package, slots.location + i * 4)?;
        }
        let mut rotation = [0.0; 3];
        if let Some(at) = slots.rotation {
            for (i, value) in rotation.iter_mut().enumerate() {
                *value = package.read_int(at + i * 4)? as f32 / ROTATOR_PER_DEGREE;
            }
        }
        let fov = slots.fov.map(|at| read_f32(package, at)).transpose()?;
        cameras.push(MenuCamera {
            name,
            tag,
            location,
            rotation,
            fov,
        });
    }
    // The likely home-screen camera first, then by tag.
    cameras.sort_by(|a, b| {
        (a.tag != "MainMenuPlay", &a.tag, &a.name).cmp(&(b.tag != "MainMenuPlay", &b.tag, &b.name))
    });
    Ok(cameras)
}

/// Cameras of the unmodified menu: the backup while a swap is active.
pub fn menu_cameras(cooked: &Path) -> Result<Vec<MenuCamera>, String> {
    let backup = state_dir(cooked).join(BACKUP);
    let path = if backup.is_file() {
        backup
    } else {
        menu(cooked)
    };
    read_cameras(&UpkPackage::load(&path)?)
}

fn apply_camera(menu: &mut UpkPackage, tweak: &CameraTweak) -> Result<(), String> {
    let slots = camera_exports(menu)
        .find(|(_, name)| *name == tweak.camera)
        .and_then(|(e, _)| camera_slots(menu, e))
        .ok_or_else(|| format!("The menu has no camera named {}.", tweak.camera))?;
    for (i, delta) in tweak.offset.iter().enumerate() {
        let at = slots.location + i * 4;
        let value = read_f32(menu, at)? + delta;
        menu.patch(at, &value.to_le_bytes())?;
    }
    if tweak.pitch != 0.0 || tweak.yaw != 0.0 {
        let at = slots
            .rotation
            .ok_or("This camera has no stored rotation to change.")?;
        for (i, delta) in [tweak.pitch, tweak.yaw].iter().enumerate() {
            let value = menu.read_int(at + i * 4)? + (delta * ROTATOR_PER_DEGREE).round() as i32;
            menu.patch_i32(at + i * 4, value)?;
        }
    }
    if let Some(fov) = tweak.fov {
        if !(10.0..=170.0).contains(&fov) {
            return Err("Field of view must be between 10 and 170.".into());
        }
        let at = slots
            .fov
            .ok_or("This camera has no stored field of view to change.")?;
        menu.patch(at, &fov.to_le_bytes())?;
    }
    Ok(())
}

fn menu(cooked: &Path) -> PathBuf {
    game_path::resolve(cooked, MENU)
}

fn state_dir(cooked: &Path) -> PathBuf {
    cooked.join(STATE)
}

fn valid_package(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

fn arena_package(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    valid_package(name)
        && lower.ends_with("_p")
        && !lower.starts_with("bg_")
        && !lower.starts_with("menu_")
        && !["_oob", "_fx", "_vfx", "_signs", "woods_forest"]
            .iter()
            .any(|part| lower.contains(part))
}

pub fn installed_arenas(cooked: &Path) -> Result<Vec<Arena>, String> {
    let mut arenas = Vec::new();
    for entry in fs::read_dir(cooked).map_err(|e| format!("Could not read the game maps: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("upk"))
        {
            continue;
        }
        let Some(package) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        // Filter names before asking for metadata. CookedPCConsole contains
        // thousands of unrelated packages, and per-file stat calls are slow.
        if !arena_package(package) {
            continue;
        }
        let file_type = entry.file_type().map_err(|e| e.to_string())?;
        if file_type.is_file() || (file_type.is_symlink() && path.is_file()) {
            arenas.push(Arena {
                label: crate::discord_presence::display_map_name(package),
                package: package.to_string(),
            });
        }
    }
    arenas.sort_by_key(|arena| (arena.label.to_lowercase(), arena.package.to_lowercase()));
    Ok(arenas)
}

fn hash(path: &Path) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let size = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if size == 0 {
            break;
        }
        digest.update(&buffer[..size]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn owned_alias(name: &str) -> bool {
    name.strip_suffix(".upk")
        .and_then(|s| s.strip_prefix("HM"))
        .is_some_and(|tail| tail.len() >= 3 && tail.bytes().all(|c| c.is_ascii_hexdigit()))
}

/// The menu swap owns `file` while active: restore only accepts the exact bytes
/// it installed, so any other edit to the menu leaves it unrestorable.
pub fn owns(cooked: &Path, file: &str) -> bool {
    file.eq_ignore_ascii_case(MENU) && !matches!(active(cooked), Ok(None))
}

pub fn active(cooked: &Path) -> Result<Option<ActiveSwap>, String> {
    let path = state_dir(cooked).join(MANIFEST);
    let data = match fs::read(&path) {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Could not read the menu map state: {e}")),
    };
    let state: ActiveSwap = serde_json::from_slice(&data)
        .map_err(|e| format!("The menu map state could not be read: {e}"))?;
    let mut seen = HashSet::new();
    if !valid_package(&state.arena)
        || state
            .files
            .iter()
            .any(|file| !owned_alias(&file.name) || !seen.insert(&file.name))
    {
        return Err("The menu map state contains invalid file names.".into());
    }
    Ok(Some(state))
}

impl ActiveSwap {
    /// Every camera change, including one from an older manifest.
    pub fn camera_tweaks(&self) -> Vec<CameraTweak> {
        self.cameras.iter().chain(&self.camera).cloned().collect()
    }

    pub fn display_name(&self) -> String {
        self.label
            .clone()
            .unwrap_or_else(|| crate::discord_presence::display_map_name(&self.arena))
    }
}

fn ensure_closed() -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        Err("Close Rocket League before applying or restoring the menu map.".into())
    } else {
        Ok(())
    }
}

/// Discover every layer rather than swapping only the arena's distant scenery.
/// `root` may live outside the game folder (a workshop map in the cache); its
/// streaming layers are looked up in `cooked`. Workshop maps often keep
/// references to layers that were never shipped, so with `skip_missing` those
/// are left out instead of failing.
fn layers_from(cooked: &Path, root: PathBuf, skip_missing: bool) -> Result<Vec<PathBuf>, String> {
    let mut queue: VecDeque<String> = VecDeque::new();
    let mut seen = HashSet::new();
    let mut layers = Vec::new();
    if let Some(stem) = root.file_stem().and_then(|s| s.to_str()) {
        seen.insert(stem.to_ascii_lowercase());
    }
    let mut next = Some(root);
    loop {
        let path = match next.take() {
            Some(path) => path,
            None => {
                let Some(name) = queue.pop_front() else { break };
                if !valid_package(&name) {
                    return Err(format!("Invalid arena layer: {name}"));
                }
                if !seen.insert(name.to_ascii_lowercase()) {
                    continue;
                }
                let path = game_path::resolve(cooked, format!("{name}.upk"));
                if skip_missing && !path.is_file() {
                    continue;
                }
                path
            }
        };
        if layers.len() >= 64 {
            return Err("This arena has too many streaming layers.".into());
        }
        let package = UpkPackage::load(&path)
            .map_err(|e| format!("Could not load {}: {e}", path.display()))?;
        for index in package.find_all_stream_name_indices() {
            queue.push_back(package.names[index].clone());
        }
        layers.push(path);
    }
    Ok(layers)
}

fn arena_layers(cooked: &Path, arena: &str) -> Result<Vec<PathBuf>, String> {
    if !valid_package(arena) {
        return Err(format!("Invalid arena layer: {arena}"));
    }
    layers_from(
        cooked,
        game_path::resolve(cooked, format!("{arena}.upk")),
        false,
    )
}

#[derive(Clone)]
struct StreamSlot {
    export: usize,
    name: usize,
    original: String,
}

fn arena_slots(menu: &UpkPackage) -> Result<Vec<StreamSlot>, String> {
    let mut slots = Vec::new();
    for e in &menu.exports {
        if !menu.class_of(e).starts_with("LevelStreamingKismet") {
            continue;
        }
        let props = menu.serialized_props(e)?.0;
        let Some(prop) = props.iter().find(|p| p.name == "PackageName") else {
            continue;
        };
        let index = menu.read_int(e.serial_offset + prop.value_offset)? as usize;
        let name = menu.names.get(index).ok_or("Invalid menu streaming name")?;
        // Preserve the actual menu, garage, cinematics and introduction sequences.
        if name.starts_with("bg_")
            || name.ends_with("_sfx")
            || name.ends_with("_vfx")
            || name.starts_with("outlaw_oasis_")
            || name.starts_with("neotokyo_hax_signs")
        {
            slots.push(StreamSlot {
                export: e.table_index,
                name: index,
                original: name.clone(),
            });
        }
    }
    let primary = slots
        .iter()
        .position(|s| s.original == "bg_stadium")
        .ok_or("This game version has no supported main menu stadium slot.")?;
    slots.swap(0, primary);
    Ok(slots)
}

/// Redirect arena streaming actions, including the default stadium's field and
/// goals. Editing only the seasonal background leaves the original grass loaded.
fn redirect_arena_actions(menu: &mut UpkPackage, slots: &[StreamSlot]) -> Result<usize, String> {
    let arena_names: HashSet<_> = slots.iter().map(|s| s.name).collect();
    let mut offsets = Vec::new();
    for e in &menu.exports {
        if !menu.class_of(e).starts_with("SeqAct_MultiLevelStreaming") {
            continue;
        }
        let props = menu.serialized_props(e)?.0;
        let Some(levels) = props.iter().find(|p| p.name == "Levels") else {
            continue;
        };
        let start = e.serial_offset + levels.value_offset;
        let end = start + levels.size;
        let count = menu.read_int(start)?;
        if !(0..=256).contains(&count) {
            return Err("Invalid menu arena list.".into());
        }
        let mut at = start + 4;
        for _ in 0..count {
            let (props, next) = menu.nested_props(at, end)?;
            for prop in props {
                if prop.tag_type == "NameProperty"
                    && arena_names.contains(&(menu.read_int(prop.value_offset)? as usize))
                {
                    offsets.push(prop.value_offset);
                }
            }
            at = next;
        }
        if at != end {
            return Err("Unexpected menu arena list layout.".into());
        }
    }
    if offsets.is_empty() {
        return Err("No main menu arena loading actions were found.".into());
    }
    for offset in &offsets {
        menu.patch_i32(*offset, slots[0].name as i32)?;
    }
    Ok(offsets.len())
}

fn alias(index: usize, length: usize) -> Result<String, String> {
    if length < 5 || index > 0xfff {
        return Err("Unsupported menu package name slot.".into());
    }
    Ok(format!("HM{index:0width$X}", width = length - 2))
}

/// Prepare the complete patch in an isolated directory. No installed files change.
fn prepare(
    cooked: &Path,
    layers: &[PathBuf],
    tweaks: &MenuTweaks,
    output: &Path,
) -> Result<Vec<MenuFile>, String> {
    let mut menu = UpkPackage::load(&menu(cooked))?;
    let slots = arena_slots(&menu)?;
    if layers.len() > slots.len() {
        return Err(format!(
            "This arena needs {} layers; this menu supports {}.",
            layers.len(),
            slots.len()
        ));
    }
    let always_loaded = menu
        .exports
        .iter()
        .find(|e| menu.class_of(e).starts_with("LevelStreamingAlwaysLoaded"))
        .ok_or("No menu streaming template was found.")?
        .class_index;
    redirect_arena_actions(&mut menu, &slots)?;
    for camera in &tweaks.cameras {
        apply_camera(&mut menu, camera)?;
    }
    if let Some(car) = &tweaks.car {
        apply_car(&mut menu, car)?;
    }
    let mut files = Vec::new();
    for (i, (layer, slot)) in layers.iter().zip(&slots).enumerate() {
        let name = alias(i, slot.original.len())?;
        let filename = format!("{name}.upk");
        if cooked.join(&filename).exists() {
            return Err(format!(
                "{filename} already exists. Restore the previous menu map first."
            ));
        }
        menu.rename_name(slot.name, &name)?;
        if i != 0 {
            menu.patch_i32(menu.exports[slot.export].entry_offset, always_loaded)?;
        }
        let staged = output.join(format!("{i}.bin"));
        fs::copy(layer, &staged).map_err(|e| format!("Could not prepare the arena: {e}"))?;
        files.push(MenuFile {
            name: filename,
            sha256: hash(&staged)?,
        });
    }
    let path = output.join("patched.bin");
    menu.save(&path)?;
    let verified = UpkPackage::load(&path)?;
    if verified.image != menu.image {
        return Err("Menu package verification failed.".into());
    }
    Ok(files)
}

fn copy_new(source: &Path, destination: &Path) -> Result<(), String> {
    let mut input = fs::File::open(source).map_err(|e| e.to_string())?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|e| format!("Could not create {}: {e}", destination.display()))?;
    let result = std::io::copy(&mut input, &mut output)
        .and_then(|_| output.flush())
        .and_then(|_| output.sync_all());
    drop(output);
    if result.is_err() {
        // create_new succeeded, so this is our incomplete copy, not another mod.
        let _ = fs::remove_file(destination);
    }
    result.map_err(|e| e.to_string())
}

fn replace_menu(source: &Path, cooked: &Path) -> Result<(), String> {
    let temporary = cooked.join(".hebnix-menu-map-replace.tmp");
    copy_new(source, &temporary)?;
    let result = fs::rename(&temporary, menu(cooked))
        .map_err(|e| format!("Could not replace the menu: {e}"));
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

pub fn apply(cooked: &Path, arena: &str, tweaks: MenuTweaks) -> Result<String, String> {
    ensure_closed()?;
    if active(cooked)?.is_some() {
        return Err("Restore the current menu map before applying another.".into());
    }
    if !installed_arenas(cooked)?
        .iter()
        .any(|entry| entry.package == arena)
    {
        return Err("Choose an installed arena from the menu map list.".into());
    }
    let layers = arena_layers(cooked, arena)?;
    apply_layers(cooked, arena, None, tweaks, &layers)
}

/// A map package has a world. Anything else (a texture pack, a cosmetic) would
/// leave the menu without a stadium.
fn is_map(package: &UpkPackage) -> bool {
    package
        .exports
        .iter()
        .any(|e| matches!(strip(&package.class_of(e)), "World" | "Level"))
}

/// Use a downloaded or imported workshop map as the menu stadium.
pub fn apply_custom(
    cooked: &Path,
    map_file: &Path,
    id: &str,
    label: &str,
    tweaks: MenuTweaks,
) -> Result<String, String> {
    ensure_closed()?;
    if active(cooked)?.is_some() {
        return Err("Restore the current menu map before applying another.".into());
    }
    if !valid_package(id) {
        return Err("This workshop map has an unsupported id.".into());
    }
    let package =
        UpkPackage::load(map_file).map_err(|e| format!("Could not load the workshop map: {e}"))?;
    if !is_map(&package) {
        return Err("This file is not a map.".into());
    }
    drop(package);
    let layers = layers_from(cooked, map_file.to_path_buf(), true)?;
    apply_layers(cooked, id, Some(label.to_string()), tweaks, &layers)
}

fn apply_layers(
    cooked: &Path,
    arena: &str,
    label: Option<String>,
    tweaks: MenuTweaks,
    layers: &[PathBuf],
) -> Result<String, String> {
    let tweaks = MenuTweaks {
        cameras: tweaks
            .cameras
            .into_iter()
            .filter(|tweak| !tweak.is_noop())
            .collect(),
        car: tweaks.car.filter(|tweak| !tweak.is_noop()),
    };
    let state = state_dir(cooked);
    // Never erase unknown/orphaned backup files automatically.
    fs::create_dir(&state).map_err(|e| format!("Could not create the menu backup folder: {e}"))?;
    let prepared = (|| {
        let build = backup_guard::build_id(cooked)?;
        let original_sha256 = hash(&menu(cooked))?;
        let files = prepare(cooked, layers, &tweaks, &state)?;
        fs::copy(menu(cooked), state.join(BACKUP)).map_err(|e| e.to_string())?;
        if hash(&state.join(BACKUP))? != original_sha256 {
            return Err("The menu changed while preparing the backup.".into());
        }
        let manifest = ActiveSwap {
            arena: arena.to_string(),
            label,
            cameras: tweaks.cameras,
            camera: None,
            car: tweaks.car,
            build,
            original_sha256,
            patched_sha256: hash(&state.join("patched.bin"))?,
            files,
        };
        // Persist the recovery record BEFORE copying anything into the game.
        fs::write(
            state.join(MANIFEST),
            serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        Ok::<_, String>(manifest)
    })();
    let manifest = match prepared {
        Ok(state) => state,
        Err(error) => {
            let _ = fs::remove_dir_all(&state); // exclusively created above; live files untouched
            return Err(error);
        }
    };
    let installed = (|| {
        ensure_closed()?;
        for (i, file) in manifest.files.iter().enumerate() {
            copy_new(&state.join(format!("{i}.bin")), &cooked.join(&file.name))?;
        }
        ensure_closed()?;
        if hash(&menu(cooked))? != manifest.original_sha256 {
            return Err("The menu changed before the patch could be installed.".into());
        }
        replace_menu(&state.join("patched.bin"), cooked)?;
        if hash(&menu(cooked))? != manifest.patched_sha256 {
            return Err("Installed menu verification failed.".into());
        }
        Ok::<_, String>(())
    })();
    if let Err(error) = installed {
        return match restore(cooked) {
            Ok(_) => Err(format!("{error} The original menu was kept.")),
            Err(rollback) => Err(format!(
                "{error} Close the game and restore the menu map to finish recovery: {rollback}"
            )),
        };
    }
    Ok(format!(
        "Main menu map changed to {}. Start Rocket League to see it.",
        manifest.display_name()
    ))
}

#[derive(Debug, PartialEq)]
enum RestoreAction {
    Restore,
    KeepCurrent,
}

fn restore_action(state: &ActiveSwap, build: &str, live: &str) -> Result<RestoreAction, String> {
    if live == state.original_sha256 {
        return Ok(RestoreAction::KeepCurrent);
    }
    if build != state.build {
        if live != state.patched_sha256 {
            return Ok(RestoreAction::KeepCurrent);
        }
        return Err("Rocket League was updated. Verify its game files in the launcher, then use Restore to clear the old menu map.".into());
    }
    if live != state.patched_sha256 {
        return Err("The menu was changed by another tool. Verify its game files before restoring this backup.".into());
    }
    Ok(RestoreAction::Restore)
}

pub fn restore(cooked: &Path) -> Result<String, String> {
    ensure_closed()?;
    let Some(manifest) = active(cooked)? else {
        return Err("No menu map change is active.".into());
    };
    let state = state_dir(cooked);
    let action = restore_action(
        &manifest,
        &backup_guard::build_id(cooked)?,
        &hash(&menu(cooked))?,
    )?;
    // Check every owned file before restoring or removing anything. Missing
    // aliases are allowed so an interrupted installation can always be restored.
    for file in &manifest.files {
        let path = cooked.join(&file.name);
        if path.exists() && hash(&path)? != file.sha256 {
            return Err(format!(
                "{} changed outside Hebnix; it was left untouched.",
                file.name
            ));
        }
    }
    if action == RestoreAction::Restore {
        if hash(&state.join(BACKUP))? != manifest.original_sha256 {
            return Err("The original menu backup failed verification.".into());
        }
        replace_menu(&state.join(BACKUP), cooked)?;
        if hash(&menu(cooked))? != manifest.original_sha256 {
            return Err("Restored menu verification failed.".into());
        }
    }
    for file in &manifest.files {
        let path = cooked.join(&file.name);
        if path.exists() {
            fs::remove_file(path).map_err(|e| e.to_string())?;
        }
    }
    fs::remove_dir_all(&state)
        .map_err(|e| format!("Menu restored, but the backup folder could not be cleared: {e}"))?;
    Ok("Original main menu map restored.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_camera_manifest_still_reads() {
        let state: ActiveSwap = serde_json::from_str(
            r#"{"arena": "Stadium_P", "camera": {"camera": "CameraActor_2", "offset": [1.0, 2.0, 3.0], "pitch": 0.0, "yaw": 0.0},
                "build": "", "original_sha256": "", "patched_sha256": "", "files": []}"#,
        )
        .unwrap();
        let cameras = state.camera_tweaks();
        assert_eq!(cameras.len(), 1);
        assert_eq!(cameras[0].offset, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn menucam_export_reads_car_and_camera() {
        let path = std::env::temp_dir().join(format!("hebnix_menucam_{}.json", std::process::id()));
        fs::write(
            &path,
            r#"{"car": {"offset": [100.0, -17118.0, 6215.0], "yaw": 0.0},
                "cameras": [{"camera": "CameraActor_2", "offset": [0.0, 0.0, 10.0], "pitch": 4.3, "yaw": -13.5, "fov": null}]}"#,
        )
        .unwrap();
        let export = read_menucam_export(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(export.car.unwrap().offset, [100.0, -17118.0, 6215.0]);
        let camera = &export.cameras[0];
        assert_eq!(camera.camera, "CameraActor_2");
        assert_eq!(camera.fov, None);
    }

    #[test]
    fn active_swap_owns_only_the_menu() {
        let cooked = std::env::temp_dir().join(format!("hebnix_menu_owns_{}", std::process::id()));
        fs::create_dir_all(state_dir(&cooked)).unwrap();
        assert!(!owns(&cooked, MENU));
        fs::write(
            state_dir(&cooked).join(MANIFEST),
            r#"{"arena":"Stadium_P","build":"b","original_sha256":"o","patched_sha256":"p","files":[]}"#,
        )
        .unwrap();
        assert!(owns(&cooked, "menu_main_p.upk"));
        assert!(!owns(&cooked, "Stadium_P.upk"));
        fs::remove_dir_all(&cooked).unwrap();
    }

    #[test]
    fn manifests_from_older_builds_still_read() {
        let old = r#"{"arena":"Stadium_P","build":"b","original_sha256":"o","patched_sha256":"p","files":[]}"#;
        let state: ActiveSwap = serde_json::from_str(old).unwrap();
        assert_eq!(state.label, None);
        assert_eq!(state.arena, "Stadium_P");
    }

    #[test]
    fn workshop_maps_show_their_own_name() {
        let state = ActiveSwap {
            arena: "local_ab12".into(),
            label: Some("Cool Map".into()),
            cameras: Vec::new(),
            camera: None,
            car: None,
            build: String::new(),
            original_sha256: String::new(),
            patched_sha256: String::new(),
            files: Vec::new(),
        };
        assert_eq!(state.display_name(), "Cool Map");
    }

    #[test]
    fn missing_custom_root_is_an_error() {
        let dir = std::env::temp_dir().join(format!("hebnix_menu_map_{}", std::process::id()));
        assert!(layers_from(&dir, dir.join("nope.upk"), true).is_err());
    }

    /// `HEBNIX_MENU=<path to MENU_Main_p.upk or its backup> cargo test live_menu_camera -- --ignored`
    #[test]
    #[ignore]
    fn live_menu_camera_tweak_round_trips() {
        let path = PathBuf::from(std::env::var("HEBNIX_MENU").expect("HEBNIX_MENU"));
        let mut menu = UpkPackage::load(&path).unwrap();
        let cameras = read_cameras(&menu).unwrap();
        assert!(!cameras.is_empty());
        let before = &cameras[0];
        let tweak = CameraTweak {
            camera: before.name.clone(),
            offset: [100.0, -50.0, 25.0],
            pitch: 5.0,
            yaw: -10.0,
            fov: before.fov.map(|_| 90.0),
        };
        apply_camera(&mut menu, &tweak).unwrap();
        let out = std::env::temp_dir().join(format!("hebnix_menu_cam_{}.upk", std::process::id()));
        menu.save(&out).unwrap();
        let after = read_cameras(&UpkPackage::load(&out).unwrap()).unwrap();
        let _ = fs::remove_file(&out);
        let after = after.iter().find(|c| c.name == before.name).unwrap();
        for i in 0..3 {
            assert!((after.location[i] - before.location[i] - tweak.offset[i]).abs() < 0.01);
        }
        assert!((after.rotation[0] - before.rotation[0] - 5.0).abs() < 0.01);
        assert!((after.rotation[1] - before.rotation[1] + 10.0).abs() < 0.01);
        if before.fov.is_some() {
            assert_eq!(after.fov, Some(90.0));
        }
        println!(
            "{} {:?} -> {:?}",
            before.tag, before.location, after.location
        );
    }

    /// `HEBNIX_MENU=<menu path> cargo test live_car_spot -- --ignored`
    #[test]
    #[ignore]
    fn live_car_spot_tweak_turns_the_line_up_around_the_player() {
        let path = PathBuf::from(std::env::var("HEBNIX_MENU").expect("HEBNIX_MENU"));
        let mut menu = UpkPackage::load(&path).unwrap();
        let before = read_car_spots(&menu).unwrap();
        assert_eq!(before[0].name, PLAYER_CAR);
        let tweak = CarTweak {
            offset: [10.0, 20.0, 30.0],
            yaw: 90.0,
        };
        apply_car(&mut menu, &tweak).unwrap();
        let out = std::env::temp_dir().join(format!("hebnix_menu_car_{}.upk", std::process::id()));
        menu.save(&out).unwrap();
        let after = read_car_spots(&UpkPackage::load(&out).unwrap()).unwrap();
        let _ = fs::remove_file(&out);
        let pivot = before[0].location;
        for (b, a) in before.iter().zip(&after) {
            let (dx, dy) = (b.location[0] - pivot[0], b.location[1] - pivot[1]);
            // 90 degrees: (dx, dy) becomes (-dy, dx)
            assert!((a.location[0] - (pivot[0] - dy + 10.0)).abs() < 0.05);
            assert!((a.location[1] - (pivot[1] + dx + 20.0)).abs() < 0.05);
            assert!((a.location[2] - (b.location[2] + 30.0)).abs() < 0.05);
            assert!((a.yaw - b.yaw - 90.0).abs() < 0.05);
        }
        println!("{:?}", after);
    }
}
