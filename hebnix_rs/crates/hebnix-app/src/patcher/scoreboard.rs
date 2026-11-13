//! Local scoreboard hook. Each edit appends before the final return so all
//! original jump destinations are adjusted when extending anonymized paths.
use super::*;
use native::{call, conditional, text, virtual_call};
const SLOT: &str = "SpecialEventData";
const LOAD: &str = "loadFileToString"; // UE FNames are case insensitive; Workshop owns its own slot.
// FName lookup is case-insensitive in UE. These spellings keep the
// existing hook operands and record the individual choices in TAGame.upk.
const MMR_SLOT: &str = "loadFileToStrinG";
const NAMES_SLOT: &str = "loadFileToStriNg";
const BOTH_SLOT: &str = "loadFileToStriNG";
const PATCHED_SLOTS: [&str; 4] = [LOAD, MMR_SLOT, NAMES_SLOT, BOTH_SLOT];

#[derive(Clone, Copy, Default)]
pub(crate) struct Options {
    pub show_mmr: bool,
    pub unanonymize: bool,
}

const NAME: &str = "GFxData_PRI_TA.UpdatePlayerName";
const DATA: &str = "GFxData_PRI_TA.UpdatePRIData";
const TITLE: &str = "GFxData_PRI_TA.SetPlayerTitle";
const SPECS: [(&str, usize, usize, &str); 3] = [
    (
        TITLE,
        569,
        357,
        "f756f33fe1f72140b0f6b6cba6c3a886e08d67eed6c1173212d4f966569770de",
    ),
    (
        NAME,
        436,
        296,
        "d9691095e549ba0d8a1f0e69837eef72d8d1419852cacd7620d0fdce5dce5c42",
    ),
    (
        DATA,
        947,
        611,
        "245114f9a086a5bafcb9d9ad3e9c39c2a8db2894f0ab2821e7398c046c2e0ae5",
    ),
];

fn concat(left: Code, right: Code) -> Code {
    Code::raw(&[0xf8])
        .then(left)
        .then(right)
        .then(Code::raw(&[0x16]))
}

fn lookup(p: &UpkPackage, playlist: Code, scratch: Code) -> Result<Code, String> {
    let pri = variable(p, 1, "GFxData_PRI_TA.PRI")?;
    let event = field(p, pri.clone(), "PRI_TA.GameEvent")?;
    let world = field(p, pri, "Engine.Actor.WorldInfo")?;
    let gri = Code::raw(&[0x2e])
        .reference(object(p, "GRI_TA")?)
        .then(field(p, world, "Engine.WorldInfo.GRI")?);
    // Same online/LAN distinction as MatchStatsExporter_TA.GetMatchGUID.
    let lan = context(
        event,
        virtual_call(p, "IsPlayingLan", Code::default())?,
        object(p, "GameEvent_TA.IsPlayingLan.ReturnValue")?,
    )?;
    let guid = conditional(
        lan,
        field(p, gri.clone(), "GRI_TA.LanMatchGUID")?,
        field(p, gri, "ProjectX.GRI_X.MatchGUID")?,
    )?;
    let mut id = variable(p, 1, "GFxData_PRI_TA.PlayerIDString")?;
    for delimiter in ["|", "/", "\\", ":"] {
        // Native Repl(str, match, replacement, optional caseSensitive).
        id = Code::raw(&[0xc9])
            .then(id)
            .then(text(delimiter))
            .then(text("_"))
            .then(Code::raw(&[0x48, 0x16]));
    }
    let mut path = text(r"..\..\TAGame\HebnixScoreboard\m_");
    for part in [guid, text("_"), id, text("_"), playlist, text(".mmr")] {
        path = concat(path, part);
    }
    static_context(
        p,
        "Core.FileSystem",
        virtual_call(p, LOAD, path.then(scratch))?,
        object(p, "Core.FileSystem.LoadFileToBytes.ReturnValue")?,
    )
}

fn additions(p: &UpkPackage, path: &str, start: usize, version: u8) -> Result<Code, String> {
    if path == DATA {
        // Refresh after stats/ping changes so an asynchronous lookup can arrive
        // after the row was created. Bots use their own name handler.
        let bot = field(
            p,
            variable(p, 1, "GFxData_PRI_TA.PRI")?,
            "Engine.PlayerReplicationInfo.bBot",
        )?;
        return branch(
            Code::raw(&[0x81]).then(bot).then(Code::raw(&[0x16])),
            call(p, NAME, Code::default())?,
            start,
        );
    }
    let event = field(p, variable(p, 1, "GFxData_PRI_TA.PRI")?, "PRI_TA.GameEvent")?;
    let playlist = context(
        event,
        call(p, "GameEvent_TA.GetPlaylist", Code::default())?,
        object(p, "GameEvent_TA.GetPlaylist.ReturnValue")?,
    )?;
    let number = field(p, playlist, "ProjectX.GameSettingPlaylist_X.PlaylistId")?;
    let scratch = variable(
        p,
        0x2b,
        "GFxData_PRI_TA.UpdatePlayerName.NoneCoalescing_0x1",
    )?;
    let name = variable(p, 1, "GFxData_PRI_TA.PlayerName")?;
    let clear = Code::raw(&[0x0f]).then(scratch.clone()).then(text(""));
    let primary = lookup(p, Code::raw(&[0x38, 0x53]).then(number), scratch.clone())?;
    let fallback = lookup(p, text(if version >= 3 { "selected" } else { "11" }), scratch.clone())?;
    let missing = Code::raw(&[0x81]).then(primary).then(Code::raw(&[0x16]));
    let retry = branch(missing, fallback, start + clear.memory())?;
    Ok(clear
        .then(retry)
        .then(Code::raw(&[0x0f]))
        .then(name.clone())
        .then(concat(scratch, name)))
}

fn branch(condition: Code, body: Code, start: usize) -> Result<Code, String> {
    validate(&condition)?;
    validate(&body)?;
    let end =
        u16::try_from(start + 3 + condition.memory() + body.memory()).map_err(|e| e.to_string())?;
    Ok(Code::raw(&[7])
        .then(Code::raw(&end.to_le_bytes()))
        .then(condition)
        .then(body))
}

fn checked_slot(p: &UpkPackage) -> Result<(usize, bool), String> {
    if let Some(i) = p
        .names
        .iter()
        .position(|s| PATCHED_SLOTS.contains(&s.as_str()))
    {
        return Ok((i, true));
    }
    let i = p
        .names
        .iter()
        .position(|s| s == SLOT)
        .ok_or("Scoreboard name slot unavailable")?;
    // Guard the researched slot against future builds or third-party patches.
    if p.import_debug()
        .iter()
        .any(|(a, b, c)| [a, b, c].iter().any(|s| s.contains(SLOT)))
        || p.exports.iter().any(|e| {
            e.object_name.name_index == i
                || p.image[e.serial_offset..e.serial_offset + e.serial_size]
                    .windows(8)
                    .any(|w| {
                        w[..4] == (i as u32).to_le_bytes()
                            && matches!(
                                u32::from_le_bytes(w[4..].try_into().unwrap()),
                                0..=1000 | u32::MAX
                            )
                    })
        })
    {
        return Err("The scoreboard name slot is in use".into());
    }
    Ok((i, false))
}

fn name_override(p: &UpkPackage, start: usize) -> Result<Code, String> {
    let scratch = variable(
        p,
        0x2b,
        "GFxData_PRI_TA.UpdatePlayerName.NoneCoalescing_0x1",
    )?;
    let name = variable(p, 1, "GFxData_PRI_TA.PlayerName")?;
    branch(
        lookup(p, text("name"), scratch.clone())?,
        Code::raw(&[0x0f]).then(name).then(scratch),
        start,
    )
}

fn early_name(p: &UpkPackage, version: u8) -> Result<Code, String> {
    let rename = name_override(p, 346)?;
    if version < 2 {
        return Ok(rename);
    }
    let mmr = additions(p, NAME, 346 + rename.memory(), version)?;
    Ok(rename.then(mmr))
}
fn version_additions(
    p: &UpkPackage,
    path: &str,
    start: usize,
    version: u8,
) -> Result<Code, String> {
    let original = additions(p, path, start, version)?;
    if path != DATA || version < 2 {
        return Ok(original);
    }
    let title = field(p, variable(p, 1, "GFxData_PRI_TA.PRI")?, "PRI_TA.Title")?;
    Ok(original.then(call(p, TITLE, title)?))
}
fn title_guard(p: &UpkPackage) -> Result<Code, String> {
    let load = lookup(p, text("name"), variable(p, 1, "GFxData_PRI_TA.XPTitle")?)?;
    branch(
        Code::raw(&[0x81]).then(load).then(Code::raw(&[0x16])),
        Code::raw(&[4, 11]),
        169,
    )
}
fn title_payload(p: &UpkPackage, original: &[u8]) -> Result<Vec<u8>, String> {
    let guard = title_guard(p)?;
    let delta = guard.memory() - 2;
    let mut out = original.to_vec();
    for (offset, target) in [(59, 171), (149, 351), (223, 452)] {
        out[48 + offset + 1..48 + offset + 3]
            .copy_from_slice(&((target + delta) as u16).to_le_bytes());
    }
    out.splice(48 + 109..48 + 111, guard.bytes.iter().copied());
    out[40..44].copy_from_slice(&((569 + delta) as u32).to_le_bytes());
    out[44..48].copy_from_slice(&((357 + guard.bytes.len() - 2) as u32).to_le_bytes());
    Ok(out)
}
fn restore_title(p: &UpkPackage, actual: &[u8]) -> Result<Vec<u8>, String> {
    let guard = title_guard(p)?;
    let mut out = actual.to_vec();
    if out.get(48 + 109..48 + 109 + guard.bytes.len()) != Some(guard.bytes.as_slice()) {
        return Err("Unknown title hook".into());
    }
    out.splice(48 + 109..48 + 109 + guard.bytes.len(), [4, 11]);
    for (offset, target) in [(59, 171u16), (149, 351), (223, 452)] {
        out[48 + offset + 1..48 + offset + 3].copy_from_slice(&target.to_le_bytes());
    }
    out[40..44].copy_from_slice(&569u32.to_le_bytes());
    out[44..48].copy_from_slice(&357u32.to_le_bytes());
    if title_payload(p, &out)? != actual {
        return Err("Title hook mismatch".into());
    }
    Ok(out)
}
fn payload(
    p: &UpkPackage,
    path: &str,
    original: &[u8],
    memory: usize,
    storage: usize,
    version: u8,
) -> Result<Vec<u8>, String> {
    if path == TITLE {
        return title_payload(p, original);
    }
    let early = if version > 0 && path == NAME {
        early_name(p, version)?
    } else {
        Code::default()
    };
    let tail_start = memory - 3 + early.memory();
    let rename = if version > 0 && path == NAME {
        name_override(p, tail_start)?
    } else {
        Code::default()
    };
    let extra = rename.clone().then(version_additions(
        p,
        path,
        tail_start + rename.memory(),
        version,
    )?);
    let mut result = original.to_vec();
    result.splice(
        48 + storage - 3..48 + storage - 3,
        extra.bytes.iter().copied(),
    );
    if !early.bytes.is_empty() {
        let target = u16::try_from(348 + early.memory()).map_err(|e| e.to_string())?;
        result[48 + 204..48 + 206].copy_from_slice(&target.to_le_bytes());
        result.splice(48 + 238..48 + 238, early.bytes.iter().copied());
    }
    result[40..44]
        .copy_from_slice(&((memory + early.memory() + extra.memory()) as u32).to_le_bytes());
    result[44..48]
        .copy_from_slice(&((storage + early.bytes.len() + extra.bytes.len()) as u32).to_le_bytes());
    Ok(result)
}

fn prepare(p: &mut UpkPackage, on: bool) -> Result<bool, String> {
    let (index, enabled) = checked_slot(p)?;
    p.rename_names_same_size(&[(index, LOAD.into())])?;
    let mut changed = enabled != on;
    for (path, memory, storage, hash) in SPECS {
        let (idx, actual) = bytes(p, path)?;
        let actual = actual.to_vec();
        let mut found = None;
        for version in [3, 2, 1, 0] {
            if path == TITLE {
                let mut original = actual.clone();
                if enabled && hex::encode(Sha256::digest(&original)) != hash {
                    if let Ok(restored) = restore_title(p, &actual) {
                        original = restored;
                    }
                }
                if hex::encode(Sha256::digest(&original)) == hash {
                    found = Some(original);
                }
                break;
            }
            let early = if version > 0 && path == NAME {
                early_name(p, version)?
            } else {
                Code::default()
            };
            let tail_start = memory - 3 + early.memory();
            let rename = if version > 0 && path == NAME {
                name_override(p, tail_start)?
            } else {
                Code::default()
            };
            let tail = rename.clone().then(version_additions(
                p,
                path,
                tail_start + rename.memory(),
                version,
            )?);
            let mut original = actual.clone();
            if enabled {
                let pos = 48 + storage - 3 + early.bytes.len();
                if original.get(pos..pos + tail.bytes.len()) != Some(tail.bytes.as_slice()) {
                    continue;
                }
                original.drain(pos..pos + tail.bytes.len());
                if !early.bytes.is_empty() {
                    if original.get(48 + 238..48 + 238 + early.bytes.len())
                        != Some(early.bytes.as_slice())
                    {
                        continue;
                    }
                    original.drain(48 + 238..48 + 238 + early.bytes.len());
                    original[48 + 204..48 + 206].copy_from_slice(&348u16.to_le_bytes());
                }
                original[40..44].copy_from_slice(&(memory as u32).to_le_bytes());
                original[44..48].copy_from_slice(&(storage as u32).to_le_bytes());
                if payload(p, path, &original, memory, storage, version)? != actual {
                    continue;
                }
            }
            if hex::encode(Sha256::digest(&original)) == hash {
                found = Some(original);
                break;
            }
        }
        let original = found.ok_or_else(|| format!("Unrecognized scoreboard hook in {path}"))?;
        let desired = if on {
            payload(p, path, &original, memory, storage, 3)?
        } else {
            original
        };
        if desired != actual {
            p.replace_export_payload(idx, &desired)?;
            changed = true;
        }
    }
    if !on {
        p.rename_names_same_size(&[(index, SLOT.into())])?;
    }
    Ok(changed)
}

pub(crate) fn set_options(game: &Path, options: Options) -> Result<(), String> {
    let desired = match (options.show_mmr, options.unanonymize) {
        (false, false) => SLOT,
        (true, false) => MMR_SLOT,
        (false, true) => NAMES_SLOT,
        (true, true) => BOTH_SLOT,
    };
    let on = desired != SLOT;
    let _lock = PACKAGE_EDIT.lock().map_err(|e| e.to_string())?;
    let closed = || {
        if hebnix_sdk::process::is_rocket_league_running() {
            Err("Close Rocket League before changing Show MMR.".to_string())
        } else {
            Ok(())
        }
    };
    closed()?;
    let directory = cooked(game);
    let source = super::super::game_path::resolve(&directory, "TAGame.upk");
    let digest = file_digest(&source)?;
    let mut p = UpkPackage::load(&source)?;
    if !on && !p.names.iter().any(|s| PATCHED_SLOTS.contains(&s.as_str())) {
        return Ok(());
    }
    let (index, _) = checked_slot(&p)?;
    let previous = p.names[index].clone();
    let changed = prepare(&mut p, on)?;
    if on {
        p.rename_names_same_size(&[(index, desired.into())])?;
    }
    if changed || previous != desired {
        let tmp = source.with_extension("scoreboard.tmp");
        p.save_fast(&tmp)?;
        let check = UpkPackage::load(&tmp)?;
        if p.names != check.names
            || p.exports.len() != check.exports.len()
            || p.exports.iter().zip(&check.exports).any(|(a, b)| {
                p.image[a.serial_offset..a.serial_offset + a.serial_size]
                    != check.image[b.serial_offset..b.serial_offset + b.serial_size]
            })
        {
            return Err("Scoreboard staging verification failed".into());
        }
        closed()?;
        if file_digest(&source)? != digest {
            return Err("TAGame.upk changed during staging. Try again.".into());
        }
        fs::rename(tmp, source).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(crate) fn installed(game: &Path) -> Result<Options, String> {
    installed_with_version(game).map(|(options, _)| options)
}

pub(crate) fn installed_with_version(game: &Path) -> Result<(Options, bool), String> {
    let mut p = UpkPackage::load(&super::super::game_path::resolve(
        &cooked(game),
        "TAGame.upk",
    ))?;
    let Some(slot) = p.names.iter().find(|s| PATCHED_SLOTS.contains(&s.as_str())) else {
        return Ok((Options::default(), false));
    };
    let slot = slot.clone();
    let legacy = prepare(&mut p, true)?;
    let options = match slot.as_str() {
        MMR_SLOT => (true, false),
        NAMES_SLOT => (false, true),
        BOTH_SLOT => (true, true),
        // Older versions shared one hook and kept preferences outside the
        // package. Read them only after finding and validating that hook.
        _ => fs::read(game.join("TAGame/HebnixScoreboard/options.json"))
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .filter(|&(mmr, names)| mmr || names)
            .unwrap_or((true, false)),
    };
    Ok((Options { show_mmr: options.0, unanonymize: options.1 }, legacy))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires the installed Rocket League package"]
    fn scoreboard_round_trip() {
        let source = std::env::var("HEBNIX_SCOREBOARD_TEST_PACKAGE").unwrap();
        let mut p = UpkPackage::load(Path::new(&source)).unwrap();
        let original_names = p.names.clone();
        let original = SPECS.map(|(n, _, _, _)| bytes(&p, n).unwrap().1.to_owned());
        assert!(prepare(&mut p, true).unwrap());
        assert!(!prepare(&mut p, true).unwrap());
        if let Ok(output) = std::env::var("HEBNIX_SCOREBOARD_TEST_OUTPUT") {
            p.save_fast(Path::new(&output)).unwrap();
            p = UpkPackage::load(Path::new(&output)).unwrap();
        }
        assert!(prepare(&mut p, false).unwrap());
        assert_eq!(original_names, p.names);
        for ((n, _, _, _), saved) in SPECS.iter().zip(&original) {
            assert_eq!(bytes(&p, n).unwrap().1, saved.as_slice());
        }
        // Existing installations must upgrade the previous fallback in place.
        let (slot, _) = checked_slot(&p).unwrap();
        p.rename_names_same_size(&[(slot, LOAD.into())]).unwrap();
        for ((path, memory, storage, _), saved) in SPECS.iter().zip(&original) {
            let (index, _) = bytes(&p, path).unwrap();
            let legacy = payload(&p, path, saved, *memory, *storage, 2).unwrap();
            p.replace_export_payload(index, &legacy).unwrap();
        }
        assert!(prepare(&mut p, true).unwrap());
        assert!(!prepare(&mut p, true).unwrap());
        assert!(prepare(&mut p, false).unwrap());
        assert_eq!(original_names, p.names);
        for ((path, _, _, _), saved) in SPECS.iter().zip(&original) {
            assert_eq!(bytes(&p, path).unwrap().1, saved.as_slice());
        }
    }
}
