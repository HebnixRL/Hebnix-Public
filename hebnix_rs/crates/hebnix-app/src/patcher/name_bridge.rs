//! Experimental, one-shot file bridge for the inspected current Epic package.
//! Uses existing script locals and a zero-argument timer; adds no native fields.
use super::*;
use crate::patcher::upk_package::strip;

const HOST: &str = "GFxData_LocalPlayer_TA";
const POLL: &str = "GFxData_LocalPlayer_TA.ChangeName";
const REQUEST: &str = r"..\..\TAGame\HebnixNameBridge\request.txt";
// Private spelling avoids sharing ownership of the scoreboard's name slot.
const LOAD: &str = "LOADFILETOSTRING";
const DELETE: &str = "DELETEFILE";
const SPECS: [(&str, &str); 4] = [
    (
        "GFxData_LocalPlayer_TA.OnShellSet",
        "0a3800a098c41fdcb3d9e2bfbffcbf05aea9cb2023499dcdf7310988e57184f6",
    ),
    (
        "GFxData_LocalPlayer_TA.OnRemoved",
        "294c84ab5802d12a513d9fd1a2789b12ffdededa73a1de057e9ae97ba6c3a552",
    ),
    (
        "GFxData_LocalPlayer_TA.UpdateName",
        "a414d05fbb8c47c7cc9b2c8a1769962a33136d5ecd7fec77be0106815401b2c6",
    ),
    (
        POLL,
        "efa6ea950da68ae70e49e2ac702951dceeafd88049bee38f4e479406308a4c47",
    ),
];

#[derive(Serialize, Deserialize)]
struct BridgeReceipt {
    version: u32,
    build: String,
    source: String,
    exports: Vec<ExportSnapshot>,
    names: Vec<NameSnapshot>,
}

fn receipt(game: &Path) -> std::path::PathBuf {
    cooked(game).join("Backups/NameBridge/receipt.json")
}
pub(crate) fn request_path(game: &Path) -> std::path::PathBuf {
    game.join("TAGame/HebnixNameBridge/request.txt")
}
pub(crate) fn recorded(game: &Path) -> bool {
    receipt(game).is_file()
}
fn read(game: &Path) -> Result<BridgeReceipt, String> {
    serde_json::from_slice(&fs::read(receipt(game)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}
fn is_state(p: &UpkPackage, r: &BridgeReceipt, enabled: bool) -> bool {
    r.names
        .iter()
        .all(|n| p.names.get(n.index) == Some(if enabled { &n.patched } else { &n.original }))
        && r.exports.iter().all(|e| {
            bytes(p, &e.path)
                .is_ok_and(|(_, b)| b == if enabled { &e.patched } else { &e.original })
        })
}
pub(crate) fn installed(game: &Path) -> Result<bool, String> {
    if !recorded(game) {
        return Ok(false);
    }
    let r = read(game)?;
    let source = super::super::game_path::resolve(&cooked(game), "TAGame.upk");
    if r.version != 2
        || Path::new(&r.source) != source
        || r.build != super::super::backup_guard::build_id(&cooked(game))?
    {
        return Err("Name bridge backup belongs to another game build. Verify game files before reinstalling the trial.".into());
    }
    let p = UpkPackage::load(&source)?;
    if is_state(&p, &r, true) {
        Ok(true)
    } else if is_state(&p, &r, false) {
        Ok(false)
    } else {
        Err("Name bridge functions changed. Refusing to overwrite another patch.".into())
    }
}

// Current Epic packages use standard UE bytecode. The old installation uses
// remapped instructions and is intentionally rejected by the payload hashes.
fn text(s: &str) -> Code {
    Code::raw(&[0x1f])
        .then(Code::raw(s.as_bytes()))
        .then(Code::raw(&[0]))
}
fn var(p: &UpkPackage, local: bool, path: &str) -> Result<Code, String> {
    Ok(Code::raw(&[if local { 0x2b } else { 0x01 }]).reference(object(p, path)?))
}
fn invoke(p: &UpkPackage, function: &str, args: Code) -> Result<Code, String> {
    Ok(Code::raw(&[0x1b])
        .then(name(p, function)?)
        .then(args)
        .then(Code::raw(&[0x16])))
}
fn ctx(receiver: Code, expression: Code, result: i32, class: bool) -> Result<Code, String> {
    let skip = u16::try_from(expression.memory()).map_err(|e| e.to_string())?;
    Ok(Code::raw(&[if class { 0x12 } else { 0x19 }, 0])
        .then(receiver)
        .then(Code::raw(&skip.to_le_bytes()))
        .reference(result)
        .then(Code::raw(&[0]))
        .then(expression))
}
fn member(p: &UpkPackage, receiver: Code, field: &str) -> Result<Code, String> {
    ctx(receiver, var(p, false, field)?, object(p, field)?, false)
}
fn static_call(
    p: &UpkPackage,
    class: &str,
    function: &str,
    args: Code,
    result: i32,
) -> Result<Code, String> {
    ctx(
        Code::raw(&[0x20]).reference(object(p, class)?),
        invoke(p, function, args)?,
        result,
        true,
    )
}
fn branch(condition: Code, body: Code, base: usize) -> Result<Code, String> {
    let end =
        u16::try_from(base + 3 + condition.memory() + body.memory()).map_err(|e| e.to_string())?;
    Ok(Code::raw(&[0x07])
        .then(Code::raw(&end.to_le_bytes()))
        .then(condition)
        .then(body))
}
fn delegate(p: &UpkPackage) -> Result<Code, String> {
    Ok(Code::raw(&[0x43]).then(name(p, "UpdateName")?).reference(0))
}
fn controller(p: &UpkPackage) -> Result<Code, String> {
    let player = var(p, false, &format!("{HOST}.Player"))?;
    // A menu controller fails this cast; only a match controller consumes a request.
    Ok(Code::raw(&[0x2e])
        .reference(object(p, "PlayerController_TA")?)
        .then(member(p, player, "Engine.Player.Actor")?))
}
fn poll_body(p: &UpkPackage) -> Result<Code, String> {
    let pc = controller(p)?;
    let pri = member(p, pc, "Engine.Controller.PlayerReplicationInfo")?;
    let ready = Code::raw(&[0xce])
        .then(pri.clone())
        .then(Code::raw(&[0x2a, 0x16]));
    let scratch = var(p, true, &format!("{POLL}.NewName"))?;
    // Context only needs the return property's bool type, not its stored value.
    let bool_type = object(p, &format!("{HOST}.bPrimaryPlayer"))?;
    let load = static_call(
        p,
        "Core.FileSystem",
        LOAD,
        text(REQUEST).then(scratch.clone()),
        bool_type,
    )?;
    let nonempty = Code::raw(&[0xc0])
        .then(scratch.clone())
        .then(text(""))
        .then(Code::raw(&[0x16]));
    let consume = static_call(p, "Core.FileSystem", DELETE, text(REQUEST), bool_type)?;
    // Current Engine.upk no longer exports PlayerController.SetName or
    // ServerChangeName. Exercise the local PRI setter instead of invoking a
    // missing function. Explicit notification refreshes the client UI/persona.
    let rename = ctx(pri.clone(), invoke(p, "SetPlayerName", scratch)?, 0, false)?.then(ctx(
        pri,
        invoke(
            p,
            "ReplicatedEvent",
            Code::raw(&[0x21]).then(name(p, "PlayerName")?),
        )?,
        0,
        false,
    )?);
    // Delete first so the trial applies each request at most once.
    let inner_base = 3 + ready.memory() + load.memory();
    let consumed_base = inner_base + 3 + nonempty.memory();
    let once = branch(consume, rename, consumed_base)?;
    let work = load.then(branch(nonempty, once, inner_base)?);
    Ok(branch(ready, work, 0)?.then(Code::raw(&[0x04, 0x0b, 0x4c])))
}
fn additions(p: &UpkPackage, path: &str) -> Result<Code, String> {
    if path.ends_with(".UpdateName") {
        return Ok(Code::raw(&[0x1c])
            .reference(object(p, POLL)?)
            .then(text(""))
            .then(Code::raw(&[0x16])));
    }
    let args = if path.ends_with(".OnShellSet") {
        delegate(p)?.then(Code::raw(&[0x1e, 0, 0, 0x80, 0x3f, 0x27])) // 1.0 seconds, loop
    } else {
        delegate(p)?
    };
    static_call(
        p,
        "ProjectX.Timers_X",
        if path.ends_with(".OnShellSet") {
            "Set"
        } else {
            "Clear"
        },
        args,
        0,
    )
}

// Validate only the small dialect emitted above, independently of generation.
// Unreal expands serialized object references to 64-bit pointers at load time.
fn validate_bridge(code: &Code) -> Result<(), String> {
    struct Parser<'a> {
        bytes: &'a [u8],
        at: usize,
        refs: usize,
        boundaries: HashSet<usize>,
        jumps: Vec<usize>,
    }
    impl Parser<'_> {
        fn take(&mut self, n: usize) -> Result<&[u8], String> {
            let end = self.at.checked_add(n).ok_or("Bridge bytecode overflow")?;
            let value = self
                .bytes
                .get(self.at..end)
                .ok_or("Truncated bridge bytecode")?;
            self.at = end;
            Ok(value)
        }
        fn memory(&self) -> usize {
            self.at + self.refs * 4
        }
        fn reference(&mut self) -> Result<(), String> {
            self.take(4)?;
            self.refs += 1;
            Ok(())
        }
        fn parameters(&mut self, depth: usize) -> Result<(), String> {
            while self.bytes.get(self.at) != Some(&0x16) {
                self.expression(depth + 1)?;
            }
            self.take(1)?;
            Ok(())
        }
        fn expression(&mut self, depth: usize) -> Result<(), String> {
            if depth > 64 {
                return Err("Bridge bytecode nesting limit".into());
            }
            match self.take(1)?[0] {
                0x2b | 0x01 | 0x20 => self.reference()?,
                0x04 => self.expression(depth + 1)?,
                0x0b | 0x4c | 0x2a | 0x27 => (),
                0x1e => {
                    self.take(4)?;
                }
                0x21 => {
                    self.take(8)?;
                }
                0x2e => {
                    self.reference()?;
                    self.expression(depth + 1)?;
                }
                0x1f => while self.take(1)?[0] != 0 {},
                0x43 => {
                    self.take(8)?;
                    self.reference()?;
                }
                0x1c => {
                    self.reference()?;
                    self.parameters(depth + 1)?;
                }
                0x1b => {
                    self.take(8)?;
                    self.parameters(depth + 1)?;
                }
                0xc0 | 0xce => {
                    self.expression(depth + 1)?;
                    self.expression(depth + 1)?;
                    if self.take(1)? != [0x16] {
                        return Err("Invalid bridge native parameters".into());
                    }
                }
                0x19 | 0x12 => {
                    if self.take(1)? != [0] {
                        return Err("Invalid bridge context flags".into());
                    }
                    self.expression(depth + 1)?;
                    let skip = u16::from_le_bytes(self.take(2)?.try_into().unwrap()) as usize;
                    self.reference()?;
                    self.take(1)?;
                    let start = self.memory();
                    self.expression(depth + 1)?;
                    if self.memory() - start != skip {
                        return Err("Invalid bridge context skip".into());
                    }
                }
                0x07 => {
                    let target = u16::from_le_bytes(self.take(2)?.try_into().unwrap()) as usize;
                    self.jumps.push(target);
                    self.expression(depth + 1)?;
                }
                token => return Err(format!("Unsupported bridge instruction {token:02x}")),
            }
            Ok(())
        }
    }
    let mut p = Parser {
        bytes: &code.bytes,
        at: 0,
        refs: 0,
        boundaries: HashSet::new(),
        jumps: Vec::new(),
    };
    while p.at < p.bytes.len() {
        p.boundaries.insert(p.memory());
        p.expression(0)?;
    }
    p.boundaries.insert(p.memory());
    if p.refs != code.refs || p.jumps.iter().any(|to| !p.boundaries.contains(to)) {
        return Err("Invalid bridge reference count or branch destination".into());
    }
    Ok(())
}

fn reserve_names(p: &mut UpkPackage) -> Result<Vec<NameSnapshot>, String> {
    let mut used = vec![false; p.names.len()];
    for e in &p.exports {
        used[e.object_name.name_index] = true;
        for w in p.image[e.serial_offset..e.serial_offset + e.serial_size].windows(8) {
            let i = u32::from_le_bytes(w[..4].try_into().unwrap()) as usize;
            let n = u32::from_le_bytes(w[4..].try_into().unwrap());
            if i < used.len() && (n <= 1000 || n == u32::MAX) {
                used[i] = true;
            }
        }
    }
    let imports = p.import_debug();
    let mut available: Vec<bool> = p
        .names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            i > 256
                && !used[i]
                && n.len() > 4
                && n.bytes().all(|b| b.is_ascii_alphanumeric())
                && ![
                    "SpecialEventData",
                    "TransformedLeftX",
                    "TargetFind",
                    "ImportJSON",
                    "StringToUniqueNetId",
                    "QueueURLForDownload",
                    "IsURLTracked",
                    "AddCallbackToDownload",
                    "OnDownloadComplete",
                    "DownloadNextImage",
                ]
                .contains(&n.as_str())
                && !n.eq_ignore_ascii_case(LOAD)
                && !n.eq_ignore_ascii_case(DELETE)
                && !imports.iter().any(|(a, b, c)| {
                    [a, b, c]
                        .iter()
                        .any(|s| s.split('.').any(|part| strip(part) == n.as_str()))
                })
        })
        .collect();
    let mut result = Vec::new();
    for wanted in [LOAD, DELETE] {
        // Never borrow a name owned by another feature, even with different case.
        let slot = p
            .names
            .iter()
            .enumerate()
            .filter(|(i, _)| available[*i])
            .min_by_key(|(_, n)| n.len().abs_diff(wanted.len()))
            .map(|(i, _)| i)
            .ok_or("No safe unused name slot for the name bridge")?;
        available[slot] = false;
        // Current packages need balanced pairs: no safe slot is exactly the
        // requested length. Preserve total name-table size and every index.
        let delta = wanted.len() as isize - p.names[slot].len() as isize;
        if delta != 0 {
            let donor = p
                .names
                .iter()
                .enumerate()
                .find(|(i, n)| {
                    available[*i]
                        && n.len() as isize - delta >= format!("HbxNameUnused{i}").len() as isize
                })
                .map(|(i, _)| i)
                .ok_or("No safe name-size donor for the name bridge")?;
            available[donor] = false;
            let size = (p.names[donor].len() as isize - delta) as usize;
            let mut replacement = format!("HbxNameUnused{donor}");
            replacement.extend(std::iter::repeat_n('X', size - replacement.len()));
            if p.names.contains(&replacement) {
                return Err("Name bridge donor collision".into());
            }
            result.push(NameSnapshot {
                index: donor,
                original: p.names[donor].clone(),
                patched: replacement,
            });
        }
        result.push(NameSnapshot {
            index: slot,
            original: p.names[slot].clone(),
            patched: wanted.into(),
        });
    }
    let edits: Vec<_> = result
        .iter()
        .map(|n| (n.index, n.patched.clone()))
        .collect();
    p.rename_names_same_size(&edits)?;
    Ok(result)
}

fn create(p: &mut UpkPackage, build: String, source: String) -> Result<BridgeReceipt, String> {
    // Verify before changing a name or generating any replacement bytecode.
    for (path, hash) in SPECS {
        let actual = hex::encode(Sha256::digest(bytes(p, path)?.1));
        if actual != hash {
            tracing::warn!(
                function = path,
                expected = hash,
                actual = actual.as_str(),
                package = source.as_str(),
                "Name bridge layout mismatch"
            );
            return Err(format!(
                "Unsupported game layout for {path} in {source}. Expected the inspected current Epic package. Details are in the Hebnix log."
            ));
        }
    }
    let names = reserve_names(p)?;
    let mut exports = Vec::new();
    for (path, _) in SPECS {
        let (index, old) = bytes(p, path)?;
        let original = old.to_vec();
        let memory = i32::from_le_bytes(original[40..44].try_into().unwrap()) as usize;
        let storage = i32::from_le_bytes(original[44..48].try_into().unwrap()) as usize;
        if storage < 3 || original.get(48 + storage - 3..48 + storage) != Some(&[0x04, 0x0b, 0x4c])
        {
            return Err(format!("Unexpected function ending in {path}"));
        }
        let mut patched = original.clone();
        if path == POLL {
            let body = poll_body(p)?;
            validate_bridge(&body)?;
            patched[40..44].copy_from_slice(&(body.memory() as u32).to_le_bytes());
            patched[44..48].copy_from_slice(&(body.bytes.len() as u32).to_le_bytes());
            patched.splice(48..48 + storage, body.bytes);
        } else {
            // Append before the final return, keeping every existing absolute
            // branch target valid (a jump to that return now executes our tail).
            let tail = additions(p, path)?;
            validate_bridge(&tail)?;
            patched[40..44].copy_from_slice(&((memory + tail.memory()) as u32).to_le_bytes());
            patched[44..48].copy_from_slice(&((storage + tail.bytes.len()) as u32).to_le_bytes());
            patched.splice(48 + storage - 3..48 + storage - 3, tail.bytes);
        }
        p.replace_export_payload(index, &patched)?;
        exports.push(ExportSnapshot {
            path: path.into(),
            original,
            patched,
        });
    }
    Ok(BridgeReceipt {
        version: 2,
        build,
        source,
        exports,
        names,
    })
}

pub(crate) fn set_enabled(game: &Path, enabled: bool) -> Result<(), String> {
    let _lock = PACKAGE_EDIT.lock().map_err(|e| e.to_string())?;
    let closed = || {
        if hebnix_sdk::process::is_rocket_league_running() {
            Err("Close Rocket League before enabling or removing the name bridge.".to_string())
        } else {
            Ok(())
        }
    };
    closed()?;
    let directory = cooked(game);
    let source = super::super::game_path::resolve(&directory, "TAGame.upk");
    let digest = file_digest(&source)?;
    let build = super::super::backup_guard::build_id(&directory)?;
    let mut p = UpkPackage::load(&source)?;
    let r = if recorded(game) {
        let r = read(game)?;
        if r.version != 2 || Path::new(&r.source) != source || r.build != build {
            return Err(
                "Name bridge backup belongs to another game build; refusing to restore it.".into(),
            );
        }
        if !is_state(&p, &r, true) && !is_state(&p, &r, false) {
            return Err(
                "Name bridge functions changed; refusing to overwrite another patch.".into(),
            );
        }
        r
    } else if enabled {
        create(&mut p, build, source.to_string_lossy().into_owned())?
    } else {
        return Ok(());
    };
    let edits: Vec<_> = r
        .names
        .iter()
        .map(|n| {
            (
                n.index,
                if enabled {
                    n.patched.clone()
                } else {
                    n.original.clone()
                },
            )
        })
        .collect();
    p.rename_names_same_size(&edits)?;
    for e in &r.exports {
        let (i, _) = bytes(&p, &e.path)?;
        p.replace_export_payload(i, if enabled { &e.patched } else { &e.original })?;
    }
    let temporary = source.with_extension("name-bridge.tmp");
    let result = (|| {
        p.save_fast(&temporary)?;
        let check = UpkPackage::load(&temporary)?;
        if !is_state(&check, &r, enabled)
            || p.names != check.names
            || p.exports.len() != check.exports.len()
            || p.exports.iter().zip(&check.exports).any(|(a, b)| {
                p.image[a.serial_offset..a.serial_offset + a.serial_size]
                    != check.image[b.serial_offset..b.serial_offset + b.serial_size]
            })
        {
            return Err("Name bridge staging verification failed".into());
        }
        closed()?;
        if file_digest(&source)? != digest {
            return Err("TAGame.upk changed during staging; try again.".into());
        }
        let backup = receipt(game);
        fs::create_dir_all(backup.parent().unwrap()).map_err(|e| e.to_string())?;
        // Recovery is durable before the live package can change.
        if !backup.exists() {
            fs::write(
                &backup,
                serde_json::to_vec_pretty(&r).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        }
        clear_request(game)?;
        fs::rename(&temporary, &source).map_err(|e| e.to_string())?;
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

fn clear_request(game: &Path) -> Result<(), String> {
    match fs::remove_file(request_path(game)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
pub(crate) fn send(game: &Path, value: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 32 || value.chars().any(char::is_control) {
        return Err("Use a name of 1–32 characters without control characters.".into());
    }
    let path = request_path(game);
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    // UE's file helper reliably recognizes BOM-marked UTF-16, including Unicode.
    let mut bytes = vec![0xff, 0xfe];
    for unit in value.encode_utf16() {
        bytes.extend(unit.to_le_bytes());
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
    let result = fs::rename(&temporary, &path).map_err(|e| e.to_string());
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
