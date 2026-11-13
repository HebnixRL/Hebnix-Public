//! One console function that runs every Hebnix command. A key action runs a
//! console command (see actions.rs), a console command needs an exec function,
//! and the package cannot gain functions, so one existing exec function with an
//! int parameter, PlayerController_TA.NetClientInputRate(int Rate), is rewritten
//! into a chain of the game's own `if (Rate == id) { ... }`:
//!
//!     07 <end of block> 9a 2b <Rate> 2c <id> 16  <body>
//!
//! JumpIfNot, the native int == call (its parameters, then end), a local, a
//! byte constant. The jump target counts bytes in memory form, where object
//! references take 8 bytes. A feature adds or removes its own block, the
//! targets are worked out again each time, and the function's first form is
//! kept in Backups/Dispatcher and put back when the last block goes.
//!
//! A body may hold jumps of its own (an `if` inside a command). Jump targets
//! count from the start of the function, so a body is written with targets
//! counted from its own start, memory form, and the position of each target
//! word is passed to `install`. Those positions are kept in the saved state, and
//! the targets are shifted by the block's place whenever the blocks are put together.
use super::script_patch::{Script, function_payload};
use super::upk_package::UpkPackage;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

const DONOR: &str = "PlayerController_TA.NetClientInputRate";
const PARAM: &str = "PlayerController_TA.NetClientInputRate.Rate";
/// The console command a key action runs, with the block id after it.
pub const COMMAND: &str = "NetClientInputRate";
/// Return, Nothing, EndOfScript: how the function ends.
const END: [u8; 3] = [0x04, 0x0b, 0x4c];
/// A block's header in the file, and in memory (the parameter takes 8 bytes).
const HEADER: usize = 12;
const HEADER_MEMORY: usize = 16;

struct Block {
    id: u8,
    /// Jump targets inside are counted from the start of the body.
    body: Script,
    /// Byte positions in `body` of its jump target words.
    jumps: Vec<usize>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    build: String,
    /// The donor's export payload before any block.
    original: Vec<u8>,
    /// Jump target positions of each block's body, by block id.
    #[serde(default)]
    jumps: BTreeMap<u8, Vec<usize>>,
}

fn saved_path(backups: &Path) -> std::path::PathBuf {
    backups.join("Dispatcher").join("state.json")
}

fn read_saved(backups: &Path) -> Option<Saved> {
    serde_json::from_slice(&std::fs::read(saved_path(backups)).ok()?).ok()
}

pub(super) fn retain_recovery_state(cooked: &Path, archive: &Path, upk: &UpkPackage) -> Result<(), String> {
    let backups = cooked.join("Backups");
    let path = saved_path(&backups);
    if path.is_file() || ![1, 2, 3].into_iter().any(|id| has(upk, id)) {
        return Ok(());
    }
    let Some(saved) = read_saved(archive) else { return Ok(()) };
    if saved.build != super::backup_guard::build_id(cooked)? {
        return Err("The installed Game Mods dispatcher still uses an older build. Verify the game files and retry repatch.".into());
    }
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|error| error.to_string())?;
    super::upk_history::replace(&path, &serde_json::to_vec(&saved).map_err(|error| error.to_string())?)
}

fn export(upk: &UpkPackage) -> Result<usize, String> {
    upk.find_object(DONOR).filter(|index| *index > 0).map(|index| index as usize - 1).ok_or_else(|| format!("{DONOR} was not found"))
}

fn payload(upk: &UpkPackage, index: usize) -> Vec<u8> {
    let export = &upk.exports[index];
    upk.image[export.serial_offset..export.serial_offset + export.serial_size].to_vec()
}

fn rate(upk: &UpkPackage) -> Result<i32, String> {
    upk.find_object(PARAM).ok_or_else(|| format!("{PARAM} was not found"))
}

fn is_header(bytes: &[u8], at: usize, rate: i32) -> bool {
    bytes.get(at..at + HEADER).is_some_and(|h| h[0] == 0x07 && h[3] == 0x9a && h[4] == 0x2b && h[5..9] == rate.to_le_bytes() && h[9] == 0x2c && h[11] == 0x16)
}

/// The blocks of a rewritten function, or none for the game's own.
fn parse(script: &Script, rate: i32, jumps: &BTreeMap<u8, Vec<usize>>) -> Result<Vec<Block>, String> {
    let bytes = script.bytes();
    let starts: Vec<usize> = (0..bytes.len()).filter(|&at| is_header(bytes, at, rate)).collect();
    if starts.first() != Some(&0) {
        return Ok(Vec::new());
    }
    let end = bytes.len() - END.len();
    let mut memory = 0;
    let mut blocks = Vec::new();
    for (n, &start) in starts.iter().enumerate() {
        let next = starts.get(n + 1).copied().unwrap_or(end);
        let target = u16::from_le_bytes([bytes[start + 1], bytes[start + 2]]) as usize;
        let size = target.checked_sub(memory).filter(|size| *size >= HEADER_MEMORY).ok_or("A dispatcher block has a bad jump target")?;
        let refs = size
            .checked_sub(next - start)
            .filter(|extra| extra % 4 == 0)
            .map(|extra| extra / 4)
            .filter(|refs| *refs >= 1)
            .ok_or("A dispatcher block has an unexpected size")?;
        let id = bytes[start + 10];
        let mut body = Script::from_parts(bytes[start + HEADER..next].to_vec(), refs - 1);
        let own = jumps.get(&id).cloned().unwrap_or_default();
        let base = memory + HEADER_MEMORY;
        for &at in &own {
            let target = u16::from_le_bytes([body.bytes()[at], body.bytes()[at + 1]]) as usize;
            body.set_word(at, target.checked_sub(base).ok_or("A dispatcher jump points before its block")? as u16);
        }
        blocks.push(Block { id, body, jumps: own });
        memory = target;
    }
    Ok(blocks)
}

fn assemble(blocks: &[Block], rate: i32) -> Script {
    let mut out = Script::new();
    let mut memory = 0;
    for block in blocks {
        let base = memory + HEADER_MEMORY;
        memory = base + block.body.mem_size();
        let mut body = block.body.clone();
        for &at in &block.jumps {
            let target = u16::from_le_bytes([body.bytes()[at], body.bytes()[at + 1]]) as usize;
            body.set_word(at, (target + base) as u16);
        }
        out.raw(&[0x07]).raw(&(memory as u16).to_le_bytes()).raw(&[0x9a, 0x2b]).object(rate).raw(&[0x2c, block.id, 0x16]).append(&body);
    }
    out.raw(&END);
    out
}

fn current(upk: &UpkPackage, jumps: &BTreeMap<u8, Vec<usize>>) -> Result<(usize, Vec<u8>, Vec<Block>), String> {
    let index = export(upk)?;
    let old = payload(upk, index);
    let blocks = parse(&Script::of_function(&old)?, rate(upk)?, jumps)?;
    Ok((index, old, blocks))
}

fn saved_jumps(backups: &Path) -> BTreeMap<u8, Vec<usize>> {
    read_saved(backups).map(|saved| saved.jumps).unwrap_or_default()
}

/// Keep the jump positions of `blocks` in the saved state.
fn save_jumps(backups: &Path, blocks: &[Block]) -> Result<(), String> {
    let mut saved = read_saved(backups).ok_or("The dispatcher backup is missing. Verify the game files.")?;
    saved.jumps = blocks.iter().filter(|b| !b.jumps.is_empty()).map(|b| (b.id, b.jumps.clone())).collect();
    std::fs::write(saved_path(backups), serde_json::to_vec(&saved).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Whether block `id` is in the package.
pub fn has(upk: &UpkPackage, id: u8) -> bool {
    current(upk, &BTreeMap::new()).is_ok_and(|(_, _, blocks)| blocks.iter().any(|b| b.id == id))
}

/// Put `body` in as block `id`, replacing the block with that id. The first
/// block saves the game's function under `backups`. `jumps` are the byte
/// positions in `body` of its own jump targets, counted from the body's start.
pub fn install(upk: &mut UpkPackage, backups: &Path, build: &str, id: u8, body: &Script, jumps: &[usize]) -> Result<(), String> {
    let (index, old, mut blocks) = current(upk, &saved_jumps(backups))?;
    if blocks.is_empty() && read_saved(backups).is_none() {
        let path = saved_path(backups);
        std::fs::create_dir_all(path.parent().unwrap_or(backups)).map_err(|e| e.to_string())?;
        let saved = Saved { build: build.into(), original: old.clone(), jumps: BTreeMap::new() };
        std::fs::write(path, serde_json::to_vec(&saved).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    blocks.retain(|b| b.id != id);
    blocks.push(Block { id, body: body.clone(), jumps: jumps.to_vec() });
    let script = assemble(&blocks, rate(upk)?);
    upk.replace_export_payload(index, &function_payload(&old, &script)?)?;
    save_jumps(backups, &blocks)
}

/// Take block `id` out. The last block puts the game's function back.
pub fn remove(upk: &mut UpkPackage, backups: &Path, build: &str, id: u8) -> Result<(), String> {
    let (index, old, mut blocks) = current(upk, &saved_jumps(backups))?;
    if !blocks.iter().any(|b| b.id == id) {
        return Ok(());
    }
    blocks.retain(|b| b.id != id);
    if blocks.is_empty() {
        let saved = read_saved(backups).ok_or("The dispatcher backup is missing. Verify the game files.")?;
        if saved.build != build {
            return Err("The dispatcher backup belongs to another game build. Verify the game files.".into());
        }
        upk.replace_export_payload(index, &saved.original)?;
        let _ = std::fs::remove_dir_all(backups.join("Dispatcher"));
        return Ok(());
    }
    let script = assemble(&blocks, rate(upk)?);
    upk.replace_export_payload(index, &function_payload(&old, &script)?)?;
    save_jumps(backups, &blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(tag: u8, objects: usize) -> Script {
        let mut script = Script::new();
        script.raw(&[0x0b, tag]);
        for n in 0..objects {
            script.object(100 + n as i32);
        }
        script
    }

    #[test]
    fn blocks_survive_assembly_and_parsing() {
        let blocks = vec![Block { id: 1, body: body(7, 2), jumps: vec![] }, Block { id: 2, body: body(9, 0), jumps: vec![] }, Block { id: 5, body: body(3, 1), jumps: vec![] }];
        let script = assemble(&blocks, -77);
        // Wrap in a function payload shaped like a real one to reuse the size fields.
        let mut payload = vec![0; 40];
        payload.extend((script.mem_size() as u32).to_le_bytes());
        payload.extend((script.len() as u32).to_le_bytes());
        payload.extend(script.bytes());
        payload.extend([0, 0, 0, 0]);
        let parsed = parse(&Script::of_function(&payload).unwrap(), -77, &BTreeMap::new()).unwrap();
        assert_eq!(parsed.len(), 3);
        for (a, b) in blocks.iter().zip(&parsed) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.body.bytes(), b.body.bytes());
            assert_eq!(a.body.refs(), b.body.refs());
        }
        assert_eq!(assemble(&parsed, -77).bytes(), script.bytes());
    }

    /// A jump inside a body is shifted by the block's place and shifted back
    /// when the blocks are parsed again.
    #[test]
    fn inner_jumps_follow_their_block() {
        let mut inner = Script::new();
        inner.raw(&[0x06, 0, 0, 0x0b]).object(1);
        let end = inner.mem_size() as u16;
        inner.set_word(1, end);
        let blocks = vec![Block { id: 1, body: body(7, 1), jumps: vec![] }, Block { id: 2, body: inner.clone(), jumps: vec![1] }];
        let script = assemble(&blocks, -77);
        let second = HEADER_MEMORY + body(7, 1).mem_size();
        let at = HEADER + body(7, 1).len() + HEADER + 1;
        let target = u16::from_le_bytes([script.bytes()[at], script.bytes()[at + 1]]) as usize;
        assert_eq!(target, second + HEADER_MEMORY + inner.mem_size());
        let mut payload = vec![0; 40];
        payload.extend((script.mem_size() as u32).to_le_bytes());
        payload.extend((script.len() as u32).to_le_bytes());
        payload.extend(script.bytes());
        payload.extend([0, 0, 0, 0]);
        let jumps = BTreeMap::from([(2u8, vec![1usize])]);
        let parsed = parse(&Script::of_function(&payload).unwrap(), -77, &jumps).unwrap();
        assert_eq!(parsed[1].body.bytes(), inner.bytes());
    }

    #[test]
    fn header_jumps_over_its_own_block_in_memory_form() {
        let script = assemble(&[Block { id: 1, body: body(7, 1), jumps: vec![] }], 5);
        // 16 header bytes in memory + body of 2 bytes + one 8-byte reference.
        assert_eq!(u16::from_le_bytes([script.bytes()[1], script.bytes()[2]]), 16 + 2 + 8);
    }

    /// Matches the game's own `if (Index == 0)` in PRI_TA.SetPodiumTitle:
    /// 07 25 00 9a 2b <Index> 25 16, with the jump target in memory bytes.
    #[test]
    fn header_matches_the_games_own_comparison_shape() {
        let script = assemble(&[Block { id: 1, body: Script::new(), jumps: vec![] }], 0x0000e28c);
        assert_eq!(&script.bytes()[..12], &[0x07, 0x10, 0x00, 0x9a, 0x2b, 0x8c, 0xe2, 0x00, 0x00, 0x2c, 0x01, 0x16]);
    }

    /// Install, replace and remove blocks on a temp copy of the real package;
    /// the last removal must give the game's function back byte for byte.
    /// Run with HEBNIX_UPK_TEST_SOURCE set to a vanilla TAGame.upk.
    #[test]
    #[ignore]
    fn real_package() {
        let source = std::env::var("HEBNIX_UPK_TEST_SOURCE").unwrap();
        let backups = std::env::temp_dir().join("hebnix_dispatcher_test");
        let _ = std::fs::remove_dir_all(&backups);
        let mut upk = UpkPackage::load(Path::new(&source)).unwrap();
        let vanilla = payload(&upk, export(&upk).unwrap());
        assert!(!has(&upk, 1));
        install(&mut upk, &backups, "b", 1, &body(1, 2), &[]).unwrap();
        install(&mut upk, &backups, "b", 2, &body(2, 1), &[]).unwrap();
        install(&mut upk, &backups, "b", 1, &body(3, 0), &[]).unwrap();
        assert!(has(&upk, 1) && has(&upk, 2) && !has(&upk, 3));
        let out = std::env::temp_dir().join("hebnix_dispatcher_test.upk");
        upk.save_fast(&out).unwrap();
        let mut reloaded = UpkPackage::load(&out).unwrap();
        let _ = std::fs::remove_file(out);
        assert!(has(&reloaded, 1) && has(&reloaded, 2));
        remove(&mut reloaded, &backups, "b", 1).unwrap();
        assert!(!has(&reloaded, 1) && has(&reloaded, 2));
        remove(&mut reloaded, &backups, "b", 2).unwrap();
        assert_eq!(payload(&reloaded, export(&reloaded).unwrap()), vanilla);
        assert!(!saved_path(&backups).exists());
    }
}
