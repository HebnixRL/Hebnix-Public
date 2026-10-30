//! Experimental: show Rocket League's own Workshop menu on the Epic build and
//! fill it from a local folder. Epic has no Steam community content interface,
//! so the menu's item list is made config-loaded and its maps are written to
//! TARLBot.ini, an ini the game already reads. LoadAllWorkshopMapData is
//! rewritten to hand those items to HandleWorkshopData, which downloads each
//! PreviewURL. All package edits are same-size, undone by writing the vanilla
//! bytes back.
use super::{
    game_path,
    upk_package::{UpkPackage, strip},
};
use std::fs;
use std::path::{Path, PathBuf};

const PACKAGE: &str = "TAGame.upk";
const ENGINE: &str = "Engine.upk";
const CLASS: &str = "GFxData_Workshop_TA";
const NODE: &str = "Archetypes.Menu.MenuTreeBuilder.Workshop";
const SECTION: &str = "[TAGame.GFxData_Workshop_TA]";
/// Config name of RLBot_Util_TA. Its TARLBot.ini already exists, so the game
/// is known to load it.
const CONFIG: &str = "RLBot";
const CLASS_CONFIG: u8 = 0x04;
/// CPF_Config (0x4000), as the second byte of the property flags.
const PROPERTY_CONFIG: u8 = 0x40;
/// The node is Steam-only by default: an include list of OnlinePlatform_Steam.
const STEAM: &str = "OnlinePlatform_Steam";
/// Patched to an exclude list of a platform nobody runs, so it shows everywhere.
const NOBODY: &str = "OnlinePlatform_Unknown";
/// Preview packages written by earlier versions, removed on apply and restore.
const OLD_PREVIEWS: &str = "HebnixWorkshop";
/// Present while the menu should stay enabled, so other TAGame.upk patchers
/// that rebuild from a pristine copy can put it back. Holds the map folder
/// the player picked, so the list can be rebuilt without the menu page open.
const ENABLED: &str = "WorkshopMenu.enabled";
/// Fallback for maps loaded from the map browser with the "Workshop folder"
/// target while the menu has no folder yet, beside CookedPCConsole.
const MAPS_FOLDER: &str = "HebnixWorkshopMaps";

pub fn maps_folder(cooked: &Path) -> PathBuf {
    cooked.parent().unwrap_or(cooked).join(MAPS_FOLDER)
}

/// Where the map browser puts workshop maps: the folder the menu lists.
pub fn workshop_dir(cooked: &Path) -> PathBuf {
    enabled_folder(&cooked.join("Backups"))
        .filter(|folder| folder.is_dir())
        .unwrap_or_else(|| maps_folder(cooked))
}

pub const MAP_DIR_PREFIX: &str = "hebnix_";

/// A Hebnix map's folder name, apart from Steam's numeric item folders.
pub fn map_dir_name(id: &str) -> String {
    format!("{MAP_DIR_PREFIX}{id}")
}

/// The folder the menu was enabled with, or None while it is disabled.
pub fn enabled_folder(backups: &Path) -> Option<PathBuf> {
    fs::read_to_string(backups.join(ENABLED))
        .ok()
        .map(|folder| PathBuf::from(folder.trim()))
}

pub struct WorkshopMap {
    pub id: u64,
    pub title: String,
    pub author: String,
    pub description: String,
    pub path: PathBuf,
    pub preview_url: String,
}

struct Targets {
    class_flags: usize,
    config_name: usize,
    items_flags: usize,
    include_list: usize,
    platform: usize,
}

fn name_index(upk: &UpkPackage, name: &str) -> Result<i32, String> {
    upk.names
        .iter()
        .position(|n| n == name)
        .map(|i| i as i32)
        .ok_or_else(|| format!("{name} is missing from {PACKAGE}"))
}

fn export<'a>(
    upk: &'a UpkPackage,
    path: &str,
) -> Result<&'a super::upk_package::ExportEntry, String> {
    upk.find_object(path)
        .filter(|index| *index > 0)
        .map(|index| &upk.exports[index as usize - 1])
        .ok_or_else(|| format!("{path} was not found in {PACKAGE}"))
}

fn locate(upk: &UpkPackage) -> Result<Targets, String> {
    let changed = |what: &str| format!("{what} changed layout in this Rocket League version");
    let class = export(upk, CLASS)?;
    // A class export ends with its flags, 4 bytes, Within, ConfigName, 48
    // bytes of empty maps, DLLBindName, the default object and 4 bytes.
    let end = class.serial_offset + class.serial_size;
    let class_flags = end.checked_sub(84).ok_or_else(|| changed(CLASS))?;
    let config_name = end - 72;
    let default = upk.read_int(end - 8)?;
    let config = upk.read_int(config_name)?;
    if default <= 0
        || upk.asset_path(default) != format!("Default__{CLASS}")
        || ![name_index(upk, "None")?, name_index(upk, CONFIG)?].contains(&config)
        || upk.read_int(config_name + 4)? != 0
    {
        return Err(changed(CLASS));
    }
    let items = export(upk, &format!("{CLASS}.Items"))?;
    if strip(&upk.class_of(items)) != "ArrayProperty" || upk.read_int(items.serial_offset + 16)? != 1
    {
        return Err(changed("GFxData_Workshop_TA.Items"));
    }
    let node = export(upk, NODE)?;
    let (props, _) = upk.serialized_props(node)?;
    let availability = props
        .iter()
        .find(|p| p.name == "PlatformAvailability")
        .ok_or_else(|| changed("The Workshop menu node"))?;
    let start = node.serial_offset + availability.value_offset;
    let (inner, _) = upk.nested_props(start, start + availability.size)?;
    let include = inner
        .iter()
        .find(|p| p.name == "bPlatformIncludeList" && p.bool_value.is_some())
        .ok_or_else(|| changed("The Workshop menu node"))?;
    let platforms = inner
        .iter()
        .find(|p| p.name == "Platforms" && p.size == 12)
        .ok_or_else(|| changed("The Workshop menu node"))?;
    if upk.read_int(platforms.value_offset)? != 1 {
        return Err(changed("The Workshop menu node"));
    }
    Ok(Targets {
        class_flags,
        config_name,
        items_flags: items.serial_offset + 21,
        // nested offsets are absolute; the bool's value sits just before it
        include_list: include.value_offset - 1,
        platform: platforms.value_offset + 4,
    })
}

/// The bytes each target must hold, vanilla or patched.
fn values(upk: &UpkPackage, t: &Targets, on: bool) -> Result<Vec<(usize, Vec<u8>)>, String> {
    let flags = |at: usize, bit: u8| {
        let byte = upk.image[at];
        vec![if on { byte | bit } else { byte & !bit }]
    };
    let config = name_index(upk, if on { CONFIG } else { "None" })?;
    let platform = name_index(upk, if on { NOBODY } else { STEAM })?;
    Ok(vec![
        (t.class_flags, flags(t.class_flags, CLASS_CONFIG)),
        (t.config_name, config.to_le_bytes().to_vec()),
        (t.items_flags, flags(t.items_flags, PROPERTY_CONFIG)),
        (t.include_list, vec![u8::from(!on)]),
        (t.platform, platform.to_le_bytes().to_vec()),
    ])
}

type Values = fn(&UpkPackage, bool) -> Result<Vec<(usize, Vec<u8>)>, String>;

/// The map list Hebnix keeps for the running game, relative to
/// Binaries\Win64 like the game's own paths. Kept short: the whole call has to
/// fit in LoadAllWorkshopMapData's 103 bytes.
const LIST: &str = r"..\..\TAGame\HbxWS.json";
/// The first live builds' list paths and the property each passed to
/// ImportJSON. Still recognised so Enable can replace them.
const IMPORT_JSON_BUILDS: [(&str, &str); 2] = [(r"..\..\TAGame\HebnixWorkshop.json", ""), (LIST, "Items")];
/// Names the patch needs that TAGame.upk lacks, each put in place of an
/// unused name of the same length: nothing in the package refers to these.
/// ImportJSON is no longer called but stays renamed, so packages patched by
/// earlier builds restore to vanilla.
const NAMES: [(&str, &str); 2] = [("TransformedLeftX", "LoadFileToString"), ("TargetFind", "ImportJSON")];

/// The unused names, renamed while the menu is enabled.
fn names(upk: &UpkPackage, on: bool) -> Result<Vec<(usize, Vec<u8>)>, String> {
    NAMES
        .iter()
        .map(|(vanilla, patched)| {
            let index = upk
                .names
                .iter()
                .position(|n| n == vanilla || n == patched)
                .ok_or_else(|| format!("{vanilla} is missing from {PACKAGE}"))?;
            Ok((upk.name_string_offset(index)?, (if on { patched } else { vanilla }).as_bytes().to_vec()))
        })
        .collect()
}

/// LoadAllWorkshopMapData's whole body. Vanilla it asks the community content
/// interface, which Epic lacks, to download the items and call
/// HandleWorkshopData back. Patched it reloads Items from Hebnix's list file
/// each time the menu opens, so maps added while the game runs show up:
///
///     class'FileSystem'.static.LoadFileToString(LIST, ItemRemoved);
///     class'JSONSerializer_X'.static.DecodeObject(self, ItemRemoved);
///     HandleWorkshopData(Items, true);
///
/// DecodeObject runs the engine's JSON reader over the object's properties.
/// (ImportJSON, tried first, imports struct strings in a mode that takes the
/// whole rest of the text, so it cannot fill Items.)
///
/// ItemRemoved, a string the class already has, holds the text. It is only
/// shown for items with no title, and the list always has titles.
fn load_all(upk: &UpkPackage, on: bool) -> Result<Vec<(usize, Vec<u8>)>, String> {
    let path = format!("{CLASS}.LoadAllWorkshopMapData");
    let function = export(upk, &path)?;
    let object = |path: &str| {
        upk.find_object(path)
            .map(i32::to_le_bytes)
            .ok_or_else(|| format!("{path} was not found in {PACKAGE}"))
    };
    let name = |index: i32| [index.to_le_bytes(), [0; 4]].concat();
    let named = |n: &str| name_index(upk, n).map(name);
    let renamed = |pair: (&str, &str)| {
        name_index(upk, pair.0).or_else(|_| name_index(upk, pair.1)).map(name)
    };
    let local = object(&format!("{path}.ContentInterface"))?;
    let items = object(&format!("{CLASS}.Items"))?;
    let handle = named("HandleWorkshopData")?;
    let online_sub = object("ProjectX.Online_X.OnlineSub")?;
    let community = object("Engine.OnlineSubsystem.CommunityContentInterface")?;
    // ContentInterface = GetObjectProvider().Get(class'OnlineGame_X').OnlineSub.CommunityContentInterface
    let first = [
        &[0x0f, 0x2b][..],
        &local,
        &[0x5e, 0x19, 0x00, 0x19, 0x00, 0x19, 0x00, 0x1c],
        &object("Core.Object.GetObjectProvider")?,
        &[0x16, 0x13, 0x00],
        &object("Core.ObjectProvider.Get.ReturnValue")?,
        &[0x00, 0x1c],
        &object("Core.ObjectProvider.Get")?,
        &[0x20],
        &object("ProjectX.OnlineGame_X")?,
        &[0x16, 0x09, 0x00],
        &online_sub,
        &[0x00, 0x01],
        &online_sub,
        &[0x09, 0x00],
        &community,
        &[0x00, 0x01],
        &community,
    ]
    .concat();
    // ContentInterface.DownloadAllWorkshopData(HandleWorkshopData)
    let vanilla = [
        &first[..],
        &[0x5e, 0x19, 0x00, 0x4a, 0x2b],
        &local,
        &[0x1b, 0, 0, 0, 0, 0, 0, 0x1b],
        &named("DownloadAllWorkshopData")?,
        &[0x43],
        &handle,
        &[0; 4],
        &[0x16, 0x04, 0x0b, 0x4c],
    ]
    .concat();
    // the call HandleWorkshopData(Items, true); return;
    let handle_items = [&[0x1b][..], &handle, &[0x01], &items, &[0x27, 0x16]].concat();
    let pad = |mut body: Vec<u8>| {
        body.extend([0x04, 0x0b]);
        body.resize(vanilla.len() - 1, 0x0b);
        body.push(0x4c);
        body
    };
    // earlier builds kept the first statement and called HandleWorkshopData
    // after it; one passed the arguments swapped and crashed. Both are still
    // recognised so they can be replaced.
    let earlier = pad([&first[..], &handle_items].concat());
    let mut swapped = earlier.clone();
    swapped[first.len() + 9..first.len() + 15].rotate_right(1);
    let removed = object(&format!("{CLASS}.ItemRemoved"))?;
    let (load_file, import_json) = (renamed(NAMES[0])?, renamed(NAMES[1])?);
    let fits = |body: Vec<u8>| {
        let body = pad(body);
        if body.len() == vanilla.len() {
            Ok(body)
        } else {
            Err(format!("The {path} patch does not fit"))
        }
    };
    // ClassContext(class), skip (the call's size in memory, refs at 8
    // bytes), no return value, then the call
    let static_call = |class: &str, call: &[u8], refs: usize| -> Result<Vec<u8>, String> {
        let skip = u16::try_from(call.len() + 4 * refs).map_err(|e| e.to_string())?;
        Ok([
            &[0x5e, 0x12, 0x00, 0x20][..],
            &object(class)?,
            &skip.to_le_bytes(),
            &[0, 0, 0, 0, 0],
            call,
        ]
        .concat())
    };
    let load = |list: &str| -> Result<Vec<u8>, String> {
        let call = [&[0x1b][..], &load_file, &[0x1f], list.as_bytes(), &[0x00, 0x01], &removed, &[0x16]].concat();
        static_call("Core.FileSystem", &call, 1)
    };
    // DecodeObject(self, ItemRemoved): FinalFunction, Self, InstanceVariable
    let decode = [
        &[0x1c][..],
        &object("ProjectX.JSONSerializer_X.DecodeObject")?,
        &[0x17, 0x01],
        &removed,
        &[0x16],
    ]
    .concat();
    let patched = fits(
        [
            &load(LIST)?[..],
            &static_call("ProjectX.JSONSerializer_X", &decode, 2)?,
            &handle_items,
        ]
        .concat(),
    )?;
    let import_builds = IMPORT_JSON_BUILDS
        .iter()
        .map(|(list, property)| {
            let import = [&[0x1b][..], &import_json, &[0x1f], property.as_bytes(), &[0x00, 0x01], &removed, &[0x16]].concat();
            fits([&load(list)?[..], &import, &handle_items].concat())
        })
        .collect::<Result<Vec<_>, String>>()?;
    // memory size counts object refs as 8 bytes: vanilla has 12, the first
    // build 10, the ImportJSON builds 5 and this one 8 (two classes, two empty
    // returns, ItemRemoved twice, DecodeObject, Items)
    let (vanilla_memory, earlier_memory, import_memory, patched_memory) = (151, 143, 123, 135);
    let header = function.serial_offset + 40;
    let code = header + 8;
    let current = upk.image.get(code..code + vanilla.len());
    if upk.read_int(header + 4)? != 103
        || ![vanilla_memory, earlier_memory, import_memory, patched_memory].contains(&upk.read_int(header)?)
        || [&vanilla[..], &earlier, &swapped, &patched]
            .into_iter()
            .chain(import_builds.iter().map(Vec::as_slice))
            .all(|known| current != Some(known))
    {
        return Err(format!("{path} changed layout in this Rocket League version"));
    }
    let memory: i32 = if on { patched_memory } else { vanilla_memory };
    Ok(vec![
        (header, memory.to_le_bytes().to_vec()),
        (code, if on { patched } else { vanilla }),
    ])
}

/// OnAvatarPersonaHandler's last statement, Items[ItemsIdx].AuthorName =
/// OnlinePlayerName. HandleWorkshopData asks for every author's avatar, and
/// on Epic the reply names them all "Unknown", so the statement becomes
/// no-op tokens and the ini's names stay.
fn avatar_name(upk: &UpkPackage, on: bool) -> Result<Vec<(usize, Vec<u8>)>, String> {
    let path = format!("{CLASS}.OnAvatarPersonaHandler");
    let function = export(upk, &path)?;
    let object = |path: &str| {
        upk.find_object(path)
            .map(i32::to_le_bytes)
            .ok_or_else(|| format!("{path} was not found in {PACKAGE}"))
    };
    // Let, StructMember, then Items[ItemsIdx] and the local name
    let vanilla = [
        &[0x0f, 0x35][..],
        &object("Engine.OnlineSubsystem.DownloadedWorkshopData.AuthorName")?,
        &object("Engine.OnlineSubsystem.DownloadedWorkshopData")?,
        &[0x00, 0x01, 0x57, 0x00, 0x00, 0x2b],
        &object(&format!("{path}.ItemsIdx"))?,
        &[0x01],
        &object(&format!("{CLASS}.Items"))?,
        &[0x2b],
        &object(&format!("{path}.OnlinePlayerName"))?,
    ]
    .concat();
    let patched = vec![0x0b; vanilla.len()];
    // memory size counts object refs as 8 bytes: the statement holds five
    let (vanilla_memory, patched_memory) = (182, 162);
    let header = function.serial_offset + 40;
    // the statement ends just before the loop's end and the return
    let at = header + 8 + 114 - 5 - vanilla.len();
    let current = upk.image.get(at..at + vanilla.len());
    // jump targets are memory offsets, so the loop's end (the foreach skip
    // and the if's jump to the next item) moves back with the removed refs
    let code = header + 8;
    let (foreach, next) = (code + 17, code + 20);
    let jumps = |end: u16| [(foreach, end), (next, end - 1)];
    let ends = [vanilla_memory, patched_memory].map(|m| m as u16 - 4);
    let jump_at = |at: usize| upk.image.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    if !ends.iter().any(|end| jumps(*end).iter().all(|(at, to)| jump_at(*at) == Some(*to)))
        || upk.read_int(header + 4)? != 114
        || ![vanilla_memory, patched_memory].contains(&upk.read_int(header)?)
        || ![&vanilla[..], &patched].map(Some).contains(&current)
    {
        return Err(format!("{path} changed layout in this Rocket League version"));
    }
    let memory: i32 = if on { patched_memory } else { vanilla_memory };
    let mut changes = vec![
        (header, memory.to_le_bytes().to_vec()),
        (at, if on { patched } else { vanilla }),
    ];
    for (at, to) in jumps(memory as u16 - 4) {
        changes.push((at, to.to_le_bytes().to_vec()));
    }
    Ok(changes)
}

/// The map file an earlier build's CreateMatch patch read.
const LAN_MAP: &str = r"..\..\TAGame\HbxLan.txt";

/// An earlier build replaced GFxData_LanMatch_TA.CreateMatch's log statement
/// with a LoadFileToString of LAN_MAP into Settings.MapName. Creating a LAN
/// match then crashed the game, so it is only ever undone now.
fn create_match(upk: &UpkPackage, on: bool) -> Result<Vec<(usize, Vec<u8>)>, String> {
    let path = "GFxData_LanMatch_TA.CreateMatch";
    let function = export(upk, path)?;
    let object = |path: &str| {
        upk.find_object(path)
            .map(i32::to_le_bytes)
            .ok_or_else(|| format!("{path} was not found in {PACKAGE}"))
    };
    let settings = object(&format!("{path}.Settings"))?;
    let (structure, member) = ("ProjectX._Types_X.CustomMatchSettings", |m: &str| {
        object(&format!("ProjectX._Types_X.CustomMatchSettings.{m}"))
    });
    // StructMember(member, struct, no copy, lvalue flag) of the Settings local
    let field = |m: &str, lvalue: u8| -> Result<Vec<u8>, String> {
        Ok([&[0x35][..], &member(m)?, &object(structure)?, &[0x00, lvalue, 0x2b], &settings].concat())
    };
    let log_name = |m: &str| -> Result<Vec<u8>, String> { Ok([field(m, 0)?, vec![0x16]].concat()) };
    let vanilla = [
        &[0xe7, 0xa8, 0xa8, 0xa8, 0x1f][..],
        b"Start Lan match\0",
        &[0x38, 0x57],
        &log_name("MapName")?,
        &log_name("GameTags")?,
        &[0x38, 0x53],
        &log_name("MaxPlayerCount")?,
        &[0x48, 0x48, 0x16],
    ]
    .concat();
    let load_file = name_index(upk, NAMES[0].0)
        .or_else(|_| name_index(upk, NAMES[0].1))
        .map(|index| [index.to_le_bytes(), [0; 4]].concat())?;
    let call = [&[0x1b][..], &load_file, &[0x1f], LAN_MAP.as_bytes(), &[0x00], &field("MapName", 1)?, &[0x16]].concat();
    // ClassContext(class'FileSystem'), skip (the call's size in memory: three
    // refs at 8 bytes), no return value, then the call
    let skip = u16::try_from(call.len() + 4 * 3).map_err(|e| e.to_string())?;
    let mut patched = [
        &[0x5e, 0x12, 0x00, 0x20][..],
        &object("Core.FileSystem")?,
        &skip.to_le_bytes(),
        &[0, 0, 0, 0, 0],
        &call,
    ]
    .concat();
    if patched.len() > vanilla.len() {
        return Err(format!("The {path} patch does not fit"));
    }
    patched.resize(vanilla.len(), 0x0b);
    // memory size counts object refs as 8 bytes: the log holds nine, the call
    // five (FileSystem, the empty return, MapName, the struct, Settings)
    let (vanilla_memory, patched_memory) = (234, 218);
    let header = function.serial_offset + 40;
    // after Settings = GetCustomMatchSettings();
    let at = header + 8 + 12;
    let current = upk.image.get(at..at + vanilla.len());
    if upk.read_int(header + 4)? != 154
        || ![vanilla_memory, patched_memory].contains(&upk.read_int(header)?)
        || ![&vanilla[..], &patched].map(Some).contains(&current)
    {
        return Err(format!("{path} changed layout in this Rocket League version"));
    }
    let memory: i32 = if on { patched_memory } else { vanilla_memory };
    Ok(vec![
        (header, memory.to_le_bytes().to_vec()),
        (at, if on { patched } else { vanilla }),
    ])
}

/// Hebnix's LAN host settings, relative to Binaries\Win64. Kept short: the
/// whole statement has to fit where the extension check was. Present only
/// while LAN hosting is on.
const HOST: &str = r"..\..\HbxHost.txt";

/// The LAN host settings file, in the install folder.
pub fn host_path(cooked: &Path) -> PathBuf {
    cooked.parent().and_then(Path::parent).unwrap_or(cooked).join(HOST.trim_start_matches(r"..\..\"))
}

/// Turn LAN hosting from the Workshop menu on (with the URL text that goes
/// after the map's game tags) or off. Safe while the game runs: the file is
/// read each time a map is picked.
pub fn set_host(cooked: &Path, tags: Option<&str>) -> Result<(), String> {
    let path = host_path(cooked);
    match tags {
        Some(tags) => fs::write(&path, tags).map_err(|e| format!("Could not write {}: {e}", path.display())),
        None => remove_if_present(&path),
    }
}

fn remove_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

/// Where the host settings wait while LAN hosting is off, so Hebnix keeps them.
fn parked_path(cooked: &Path) -> PathBuf {
    host_path(cooked).with_extension("off")
}

/// The saved host settings, whether hosting is on or off.
pub fn read_host(cooked: &Path) -> Option<String> {
    fs::read_to_string(host_path(cooked)).or_else(|_| fs::read_to_string(parked_path(cooked))).ok()
}

/// Save the host settings and turn hosting on (the game reads the file) or off
/// (parked beside it).
pub fn store_host(cooked: &Path, text: &str, on: bool) -> Result<(), String> {
    let (live, parked) = (host_path(cooked), parked_path(cooked));
    let (write, other) = if on { (live, parked) } else { (parked, live) };
    fs::write(&write, text).map_err(|e| format!("Could not write {}: {e}", write.display()))?;
    remove_if_present(&other)
}

/// GFxData_Workshop_TA.LoadWorkshopMap, which travels to the picked map with
/// GameTags "Freeplay". Patched, the game tags come from Hebnix's host file
/// when it exists, so the same click hosts a LAN match:
///
///     GameTags = "Freeplay";
///     class'FileSystem'.static.LoadFileToString(HOST, GameTags);
///
/// The file holds e.g. `BotsNone,PlayerCount6?Playtest?Lan?Listen`. The game
/// splits tags on commas only and joins them back into the URL, so the
/// options after the last tag land in the URL as they are. A missing file
/// leaves GameTags untouched, so the map loads in freeplay as before.
///
/// Room for it comes from the .udk/.umap extension check, its error branch
/// and the old GameTags line. The menu only lists .udk files.
fn load_map(upk: &UpkPackage, on: bool) -> Result<Vec<(usize, Vec<u8>)>, String> {
    let path = format!("{CLASS}.LoadWorkshopMap");
    let function = export(upk, &path)?;
    let object = |path: &str| {
        upk.find_object(path)
            .map(i32::to_le_bytes)
            .ok_or_else(|| format!("{path} was not found in {PACKAGE}"))
    };
    let local = |name: &str| object(&format!("{path}.{name}"));
    let (extension, trimmed, tags) = (local("extension")?, local("TrimmedPath")?, local("GameTags")?);
    let file_system = object("Core.FileSystem")?;
    // extension = class'FileSystem'.static.GetFileExtension(TrimmedPath);
    // if (extension ~= ".udk" || extension ~= ".umap") {
    let check = [
        &[0x0f, 0x2b][..],
        &extension,
        &[0x5e, 0x12, 0x00, 0x20],
        &file_system,
        &[0x13, 0x00],
        &object("Core.FileSystem.GetFileExtension.ReturnValue")?,
        &[0x00, 0x1c],
        &object("Core.FileSystem.GetFileExtension")?,
        &[0x2b],
        &trimmed,
        &[0x16, 0x07, 0x76, 0x02, 0x84, 0xbf, 0x2b],
        &extension,
        b"\x1f.udk\0\x16\x18\x13\x00\xbf\x2b",
        &extension,
        b"\x1f.umap\0\x16\x16",
    ]
    .concat();
    let freeplay = [&[0x0f, 0x2b][..], &tags, b"\x1fFreeplay\0"].concat();
    // } else { WarnInternal("File not a udk or umap!"); }
    let error = b"\x06\x91\x02\xe8\x1fFile not a udk or umap!\0\x16".to_vec();
    let load_file = name_index(upk, NAMES[0].0)
        .or_else(|_| name_index(upk, NAMES[0].1))
        .map(|index| [index.to_le_bytes(), [0; 4]].concat())?;
    let call = [&[0x1b][..], &load_file, &[0x1f], HOST.as_bytes(), &[0x00, 0x2b], &tags, &[0x16]].concat();
    // ClassContext(class'FileSystem'), skip (the call's size in memory, one
    // ref at 8 bytes), no return value, then the call
    let skip = u16::try_from(call.len() + 4).map_err(|e| e.to_string())?;
    let mut patched = [
        &freeplay[..],
        &[0x5e, 0x12, 0x00, 0x20],
        &file_system,
        &skip.to_le_bytes(),
        &[0, 0, 0, 0, 0],
        &call,
    ]
    .concat();
    if patched.len() > check.len() {
        return Err(format!("The {path} patch does not fit"));
    }
    patched.resize(check.len(), 0x0b);
    // statement offsets in the code; the jumps removed with the check and its
    // else were the function's only ones
    let header = function.serial_offset + 40;
    let code = header + 8;
    let regions = [
        (code + 13, check, patched),
        (code + 186, freeplay.clone(), vec![0x0b; freeplay.len()]),
        (code + 427, error.clone(), vec![0x0b; error.len()]),
    ];
    // memory size counts object refs as 8 bytes: the check holds seven and
    // the old GameTags line one; the new lines hold four
    let (vanilla_memory, patched_memory) = (660, 644);
    if upk.read_int(header + 4)? != 460
        || ![vanilla_memory, patched_memory].contains(&upk.read_int(header)?)
        || !regions.iter().all(|(at, vanilla, patched)| {
            let current = upk.image.get(*at..*at + vanilla.len());
            current == Some(vanilla) || current == Some(patched)
        })
    {
        return Err(format!("{path} changed layout in this Rocket League version"));
    }
    let memory: i32 = if on { patched_memory } else { vanilla_memory };
    let mut changes = vec![(header, memory.to_le_bytes().to_vec())];
    for (at, vanilla, patched) in regions {
        changes.push((at, if on { patched } else { vanilla }));
    }
    Ok(changes)
}

fn tagame_values(upk: &UpkPackage, on: bool) -> Result<Vec<(usize, Vec<u8>)>, String> {
    let mut changes = values(upk, &locate(upk)?, on)?;
    changes.extend(names(upk, on)?);
    changes.extend(load_all(upk, on)?);
    changes.extend(avatar_name(upk, on)?);
    changes.extend(load_map(upk, on)?);
    changes.extend(create_match(upk, false)?);
    Ok(changes)
}

/// Earlier versions retyped DownloadedWorkshopData.Preview from
/// Texture2DDynamic to Texture so the ini could name a cooked texture. That
/// never loaded, so it is only ever undone now.
fn engine_values(upk: &UpkPackage, on: bool) -> Result<Vec<(usize, Vec<u8>)>, String> {
    let preview = export(upk, "OnlineSubsystem.DownloadedWorkshopData.Preview")?;
    // the property header is 40 bytes, then its GFx name string, then the class
    let name = upk.read_int(preview.serial_offset + 40)?;
    let at = preview.serial_offset + 44 + usize::try_from(name).map_err(|e| e.to_string())?;
    let class = |name: &str| {
        upk.find_object(name)
            .filter(|i| *i > 0 && upk.class_of(&upk.exports[*i as usize - 1]) == "Class")
            .ok_or_else(|| format!("{name} was not found in {ENGINE}"))
    };
    let (dynamic, texture) = (class("Texture2DDynamic")?, class("Texture")?);
    if strip(&upk.class_of(preview)) != "ObjectProperty"
        || ![dynamic, texture].contains(&upk.read_int(at)?)
    {
        return Err("DownloadedWorkshopData.Preview changed layout".into());
    }
    Ok(vec![(at, if on { texture } else { dynamic }.to_le_bytes().to_vec())])
}

fn is_patched(upk: &UpkPackage, values: Values) -> Result<bool, String> {
    Ok(values(upk, true)?
        .iter()
        .all(|(at, bytes)| upk.image[*at..*at + bytes.len()] == bytes[..]))
}

/// Whether the live packages have the Workshop menu enabled.
pub fn status(cooked: &Path) -> Result<bool, String> {
    is_patched(&UpkPackage::load(&game_path::resolve(cooked, PACKAGE))?, tagame_values)
}

fn set_live(cooked: &Path, file: &str, values: Values, on: bool) -> Result<(), String> {
    let live = game_path::resolve(cooked, file);
    let mut upk = UpkPackage::load(&live)?;
    let changes = values(&upk, on)?;
    if changes
        .iter()
        .all(|(at, bytes)| upk.image[*at..*at + bytes.len()] == bytes[..])
    {
        return Ok(());
    }
    for (at, bytes) in &changes {
        upk.patch(*at, bytes)?;
    }
    let temp = cooked.join(format!("{file}.workshop.tmp"));
    upk.save(&temp)?;
    let check = UpkPackage::load(&temp).and_then(|saved| Ok(is_patched(&saved, values)? == on));
    if check != Ok(true) {
        let _ = fs::remove_file(&temp);
        return Err(format!("Patched {file} failed validation: {check:?}"));
    }
    let copied = fs::copy(&temp, &live);
    let _ = fs::remove_file(&temp);
    copied
        .map(|_| ())
        .map_err(|e| format!("Could not write {file}: {e}"))
}

fn set_all(cooked: &Path, on: bool) -> Result<(), String> {
    set_live(cooked, PACKAGE, tagame_values, on)?;
    set_live(cooked, ENGINE, engine_values, false)
}

fn ini_path() -> Result<PathBuf, String> {
    hebnix_sdk::utils::paths::documents_dir()
        .map(|d| d.join("My Games/Rocket League/TAGame/Config/TARLBot.ini"))
        .ok_or_else(|| "Could not find the Documents folder".into())
}

fn quote(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// Replace this feature's section of an ini, keeping everything else.
fn with_section(ini: &str, maps: &[WorkshopMap]) -> String {
    let newline = if ini.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out = Vec::new();
    let mut inside = false;
    for line in ini.lines() {
        if line.trim_start().starts_with('[') {
            inside = line.trim().eq_ignore_ascii_case(SECTION);
        }
        if !inside {
            out.push(line.to_string());
        }
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    if !maps.is_empty() {
        out.push(String::new());
        out.push(SECTION.to_string());
        for map in maps {
            let author = if map.author.is_empty() { "Local" } else { &map.author };
            let folder = map.path.parent().unwrap_or(Path::new(""));
            let file = map.path.file_name().unwrap_or_default().to_string_lossy();
            out.push(format!(
                "Items=(ItemID={},Title=\"{}\",Description=\"{}\",AuthorName=\"{}\",ItemPath=\"{}\",Filename=\"{}\",FullPath=\"{}\",PreviewURL=\"{}\")",
                map.id,
                quote(&map.title),
                quote(&map.description),
                quote(author),
                quote(&folder.to_string_lossy()),
                quote(&file),
                quote(&map.path.to_string_lossy()),
                quote(&map.preview_url),
            ));
        }
    }
    let mut text = out.join(newline);
    text.push_str(newline);
    text
}

/// The list file LoadAllWorkshopMapData reads, under the install's TAGame.
fn list_path(cooked: &Path) -> PathBuf {
    cooked.parent().unwrap_or(cooked).join(Path::new(LIST).file_name().unwrap_or_default())
}

/// {"Items":[...]} for DecodeObject, field names as in DownloadedWorkshopData.
fn list_json(maps: &[WorkshopMap]) -> String {
    let items: Vec<_> = maps
        .iter()
        .map(|map| {
            serde_json::json!({
                "ItemID": map.id,
                "Title": map.title,
                "Description": map.description,
                "AuthorName": if map.author.is_empty() { "Local" } else { &map.author },
                "ItemPath": map.path.parent().unwrap_or(Path::new("")).to_string_lossy(),
                "Filename": map.path.file_name().unwrap_or_default().to_string_lossy(),
                "FullPath": map.path.to_string_lossy(),
                "PreviewURL": map.preview_url,
            })
        })
        .collect();
    serde_json::json!({ "Items": items }).to_string()
}

/// Rewrite the menu's map list. Safe while the game runs: the game reads the
/// ini at start-up and the json each time the menu opens.
pub fn write_list(cooked: &Path, maps: &[WorkshopMap]) -> Result<(), String> {
    let _ = fs::remove_file(cooked.parent().unwrap_or(cooked).join("HebnixWorkshop.json"));
    fs::write(list_path(cooked), list_json(maps)).map_err(|e| format!("Could not write the map list: {e}"))?;
    let path = ini_path()?;
    let old = fs::read_to_string(&path).unwrap_or_default();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(&path, with_section(&old, maps))
        .map_err(|e| format!("Could not write {}: {e}", path.display()))
}

pub fn apply(
    cooked: &Path,
    backups: &Path,
    folder: &Path,
    maps: &[WorkshopMap],
) -> Result<String, String> {
    write_list(cooked, maps)?;
    set_all(cooked, true)?;
    let _ = fs::remove_dir_all(cooked.join(OLD_PREVIEWS));
    fs::create_dir_all(backups).map_err(|e| e.to_string())?;
    fs::write(backups.join(ENABLED), folder.to_string_lossy().as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "Workshop menu enabled with {} maps, {} with previews",
        maps.len(),
        maps.iter().filter(|m| !m.preview_url.is_empty()).count()
    ))
}

pub fn restore(cooked: &Path, backups: &Path) -> Result<String, String> {
    set_all(cooked, false)?;
    write_list(cooked, &[])?;
    let _ = fs::remove_file(list_path(cooked));
    let _ = fs::remove_file(host_path(cooked));
    let _ = fs::remove_file(parked_path(cooked));
    let _ = fs::remove_dir_all(cooked.join(OLD_PREVIEWS));
    let _ = fs::remove_file(backups.join(ENABLED));
    Ok("Workshop menu restored".into())
}

/// Put the patch back after another feature rewrote TAGame.upk from a backup.
pub fn reapply(cooked: &Path, backups: &Path) -> Result<(), String> {
    if backups.join(ENABLED).is_file() {
        set_live(cooked, PACKAGE, tagame_values, true)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_is_replaced_and_escaped() {
        let ini = "[TAGame.RLBot_Util_TA]\r\nPacketSendRate=60\r\n\r\n[TAGame.GFxData_Workshop_TA]\r\nItems=(old)\r\n\r\n[IniVersion]\r\n0=1\r\n";
        let maps = [WorkshopMap {
            id: 7,
            title: "A \"b\"".into(),
            author: String::new(),
            description: String::new(),
            path: PathBuf::from(r"E:\maps\7\n.udk"),
            preview_url: "https://x/p.png".into(),
        }];
        let out = with_section(ini, &maps);
        let json: serde_json::Value = serde_json::from_str(&list_json(&maps)).unwrap();
        assert_eq!(json["Items"][0]["FullPath"], r"E:\maps\7\n.udk");
        assert_eq!(json["Items"][0]["AuthorName"], "Local");
        assert!(!out.contains("Items=(old)"));
        assert!(out.contains("[IniVersion]\r\n0=1\r\n\r\n[TAGame.GFxData_Workshop_TA]\r\n"));
        assert!(out.contains(r#"Title="A \"b\"""#));
        assert!(out.contains(r#"AuthorName="Local""#));
        assert!(out.contains(r#"FullPath="E:\\maps\\7\\n.udk",PreviewURL="https://x/p.png")"#));
        assert_eq!(
            with_section(&out, &[]),
            "[TAGame.RLBot_Util_TA]\r\nPacketSendRate=60\r\n\r\n[IniVersion]\r\n0=1\r\n"
        );
    }

    /// Patch and unpatch a copy of the real package. Run explicitly with
    /// HEBNIX_WORKSHOP_TEST_SOURCE set to a TAGame.upk.
    #[test]
    #[ignore]
    fn real_package_roundtrip() {
        let source = PathBuf::from(std::env::var("HEBNIX_WORKSHOP_TEST_SOURCE").unwrap());
        let cooked = std::env::temp_dir().join("hebnix_workshop_menu_test");
        let _ = fs::remove_dir_all(&cooked);
        fs::create_dir_all(&cooked).unwrap();
        let folder = source.parent().unwrap();
        for file in [PACKAGE, ENGINE] {
            fs::copy(folder.join(file), cooked.join(file)).unwrap();
        }
        set_all(&cooked, false).unwrap();
        let vanilla: Vec<_> = [PACKAGE, ENGINE]
            .map(|f| UpkPackage::load(&cooked.join(f)).unwrap().image)
            .into();
        assert!(!status(&cooked).unwrap());
        set_all(&cooked, true).unwrap();
        assert!(status(&cooked).unwrap());
        set_all(&cooked, false).unwrap();
        assert!(!status(&cooked).unwrap());
        for (file, image) in [PACKAGE, ENGINE].iter().zip(vanilla) {
            assert!(UpkPackage::load(&cooked.join(file)).unwrap().image == image);
        }
        let _ = fs::remove_dir_all(&cooked);
    }
}

