//! Reversible MatchHistory export in TAGame.upk.
use crate::patcher::{
    game_path,
    profile_pictures::{PACKAGE_EDIT, file_digest},
    upk_package::UpkPackage,
};
use eframe::egui;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
};

const INIT: &str = "GFxData_MatchHistory_TA.Init";
const COMMAND: &str = "MatchStatsExporter_TA.ProcessSocketCommand";
const MANAGER: &str = "GFxData_MatchHistoryManager_TA";
const INIT_HASH: &str = "c65e92e170e7e602fc6ea7e0d5b4df348ec62679e631cc266f67cc1ff5b2900e";
const COMMAND_HASH: &str = "e3d45769fefb8656d806aacdd55d6923d9d1b76d86b8575c9df947c46fd7cd7d";

#[derive(Default)]
pub(crate) struct State {
    game: PathBuf,
    enabled: bool,
    pending: Option<mpsc::Receiver<Result<bool, String>>>,
    status: String,
}

impl State {
    fn run(&mut self, ctx: &egui::Context, target: Option<bool>) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let game = self.game.clone();
        self.pending = Some(rx);
        self.status.clear();
        std::thread::spawn(move || {
            let result = match target {
                Some(on) => set_enabled(&game, on).map(|()| on),
                None => installed(&game),
            };
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    pub(crate) fn tick(&mut self, ctx: &egui::Context, game: &Path) {
        if let Some(result) = self.pending.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("Auto Upload patch operation stopped.".into()))
            }
            Err(mpsc::TryRecvError::Empty) => None,
        }) {
            self.pending = None;
            match result {
                Ok(on) => self.enabled = on,
                Err(error) => self.status = error,
            }
        }
        if self.game != game && !self.busy() {
            self.game = game.to_owned();
            self.enabled = false;
            self.status.clear();
            if !game.as_os_str().is_empty() {
                self.run(ctx, None);
            }
        }
    }

    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled && !self.busy()
    }

    pub(crate) fn show(&mut self, ui: &mut egui::Ui) {
        let mut on = self.enabled;
        if ui
            .add_enabled(
                !self.game.as_os_str().is_empty()
                    && !self.busy()
                    && !hebnix_sdk::process::is_rocket_league_running(),
                egui::Checkbox::new(&mut on, "Enable Auto Upload"),
            )
            .on_disabled_hover_text("Close Rocket League before changing Auto Upload.")
            .changed()
        {
            self.run(ui.ctx(), Some(on));
        }
        if self.busy() {
            ui.spinner();
        }
        if !self.status.is_empty() {
            ui.label(&self.status);
        }
    }
}

fn source(game: &Path) -> PathBuf {
    game_path::resolve(&game.join("TAGame/CookedPCConsole"), "TAGame.upk")
}

pub(crate) fn installed(game: &Path) -> Result<bool, String> {
    let mut package = UpkPackage::load(&source(game))?;
    let (enabled, _) = inspect(&mut package, None)?;
    Ok(enabled)
}

pub(crate) fn set_enabled(game: &Path, on: bool) -> Result<(), String> {
    let _lock = PACKAGE_EDIT.lock().map_err(|e| e.to_string())?;
    let closed = || {
        if hebnix_sdk::process::is_rocket_league_running() {
            Err("Close Rocket League before changing Auto Upload.".to_owned())
        } else {
            Ok(())
        }
    };
    closed()?;
    let source = source(game);
    let digest = file_digest(&source)?;
    let mut package = UpkPackage::load(&source)?;
    if !inspect(&mut package, Some(on))?.1 {
        return Ok(());
    }
    let staged = source.with_extension(format!("replay-{}.tmp", rand::random::<u64>()));
    let result = (|| {
        package.save_fast(&staged)?;
        let check = UpkPackage::load(&staged)?;
        if check.names != package.names
            || check.exports.len() != package.exports.len()
            || check.exports.iter().zip(&package.exports).any(|(a, b)| {
                check.image[a.serial_offset..a.serial_offset + a.serial_size]
                    != package.image[b.serial_offset..b.serial_offset + b.serial_size]
            })
        {
            return Err("Auto Upload staging verification failed.".into());
        }
        closed()?;
        if file_digest(&source)? != digest {
            return Err("TAGame.upk changed during staging. Try again.".into());
        }
        fs::rename(&staged, &source).map_err(|e| format!("Could not replace TAGame.upk: {e}"))
    })();
    let _ = fs::remove_file(staged);
    result
}

fn inspect(package: &mut UpkPackage, target: Option<bool>) -> Result<(bool, bool), String> {
    let builder = Builder(package);
    let hooks = [
        (INIT, builder.logger()?, true, INIT_HASH),
        (COMMAND, builder.command()?, false, COMMAND_HASH),
    ];
    let mut enabled = true;
    let mut changed = false;
    for (path, code, at_end, hash) in hooks {
        let (index, actual) = payload(package, path)?;
        let vanilla = original(&actual, &code, at_end, hash)?;
        enabled &= vanilla.is_some();
        let desired = match (target, vanilla) {
            (Some(true), None) => insert(&actual, &code, at_end)?,
            (Some(false), Some(vanilla)) => vanilla,
            _ => continue,
        };
        package.replace_export_payload(index, &desired)?;
        changed = true;
    }
    Ok((enabled, changed))
}

#[derive(Clone, Default)]
struct Code {
    bytes: Vec<u8>,
    memory: usize,
}
impl Code {
    fn raw(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.into(),
            memory: bytes.len(),
        }
    }
    fn op(byte: u8) -> Self {
        Self::raw(&[byte])
    }
    fn pointer(reference: i32) -> Self {
        Self {
            bytes: reference.to_le_bytes().into(),
            memory: 8,
        }
    }
    fn then(mut self, other: Self) -> Self {
        self.bytes.extend(other.bytes);
        self.memory += other.memory;
        self
    }
    fn string(text: &str) -> Self {
        Self::op(0x1f)
            .then(Self::raw(text.as_bytes()))
            .then(Self::op(0))
    }
    fn jump(condition: Self, destination: usize) -> Result<Self, String> {
        let offset =
            u16::try_from(destination).map_err(|_| "History bridge branch exceeds script limit")?;
        Ok(Self::op(7)
            .then(Self::raw(&offset.to_le_bytes()))
            .then(condition))
    }
}
struct Builder<'a>(&'a UpkPackage);
impl Builder<'_> {
    fn reference(&self, path: &str) -> Result<i32, String> {
        self.0
            .find_object(path)
            .ok_or_else(|| format!("Unsupported game package: missing {path}"))
    }
    fn field(&self, opcode: u8, path: &str) -> Result<Code, String> {
        Ok(Code::op(opcode).then(Code::pointer(self.reference(path)?)))
    }
    fn call(&self, path: &str, args: Vec<Code>) -> Result<Code, String> {
        Ok(args
            .into_iter()
            .fold(self.field(0x1c, path)?, Code::then)
            .then(Code::op(0x16)))
    }
    fn context(&self, target: Code, body: Code, property: Option<&str>) -> Result<Code, String> {
        let size =
            u16::try_from(body.memory).map_err(|_| "History context exceeds script limit")?;
        Ok(Code::raw(&[0x19, 0])
            .then(target)
            .then(Code::raw(&size.to_le_bytes()))
            .then(Code::pointer(
                property
                    .map(|p| self.reference(p))
                    .transpose()?
                    .unwrap_or(0),
            ))
            .then(Code::op(0))
            .then(body))
    }
    fn native(&self, opcode: u8, args: Vec<Code>) -> Code {
        args.into_iter()
            .fold(Code::op(opcode), Code::then)
            .then(Code::op(0x16))
    }
    fn tag(&self, name: &str) -> Result<Code, String> {
        let index = self
            .0
            .names
            .iter()
            .position(|n| n == name)
            .ok_or("Missing history log tag")?;
        Ok(Code::op(0x21)
            .then(Code::raw(&(index as u32).to_le_bytes()))
            .then(Code::raw(&[0; 4])))
    }
    fn manager(&self) -> Result<Code, String> {
        let get = self.context(
            self.store()?,
            self.call(
                "ProjectX.GFxDataStore_X.GetObject",
                vec![self.field(0x20, MANAGER)?, Code::op(0x48)],
            )?,
            Some("ProjectX.GFxDataStore_X.GetObject.ReturnValue"),
        )?;
        Ok(self.field(0x2e, MANAGER)?.then(get))
    }
    fn store(&self) -> Result<Code, String> {
        let player = self.context(
            self.field(1, "MatchStatsExporter_TA.PrimaryPC")?,
            self.call("PlayerControllerBase_TA.GetLocalPlayer", vec![])?,
            Some("PlayerControllerBase_TA.GetLocalPlayer.ReturnValue"),
        )?;
        self.context(
            self.call("GFxShell_TA.GetShell", vec![player])?,
            self.field(1, "ProjectX.GFxShell_X.DataStore")?,
            Some("ProjectX.GFxShell_X.DataStore"),
        )
    }
    fn logger(&self) -> Result<Code, String> {
        Ok(self.native(
            231,
            vec![
                self.context(
                    self.field(0x2b, &format!("{INIT}.Item"))?,
                    self.call("Core.Object.ToJson", vec![])?,
                    Some("Core.Object.ToJson.ReturnValue"),
                )?,
                self.tag("MatchHistory")?,
                Code::op(0x28),
            ],
        ))
    }
    fn command(&self) -> Result<Code, String> {
        let condition = self.native(
            204,
            vec![
                self.field(0x2b, &format!("{COMMAND}.CommandJson"))?,
                Code::string("{\"Command\":\"HebnixMatchHistory\"}"),
            ],
        );
        let is_none = self.native(207, vec![self.manager()?, Code::op(0x2a)]);
        let create = self.context(
            self.store()?,
            self.call(
                "ProjectX.GFxDataStore_X.CreateObject",
                vec![self.field(0x20, MANAGER)?, Code::op(0x48), Code::op(0x48)],
            )?,
            Some("ProjectX.GFxDataStore_X.CreateObject.ReturnValue"),
        )?;
        let first = 3 + condition.memory;
        let create_jump = Code::jump(is_none.clone(), first + 3 + is_none.memory + create.memory)?;
        let refresh = self.context(
            self.manager()?,
            self.call(&format!("{MANAGER}.RefreshCache"), vec![])?,
            None,
        )?;
        let sync = self.context(
            self.manager()?,
            self.call(&format!("{MANAGER}.SyncNewMatches"), vec![])?,
            None,
        )?;
        let marker = self.native(
            231,
            vec![
                Code::string("Hebnix live history requested"),
                self.tag("Log")?,
                Code::op(0x28),
            ],
        );
        let actions = create_jump
            .then(create)
            .then(refresh)
            .then(sync)
            .then(marker)
            .then(Code::raw(&[4, 11]));
        Ok(Code::jump(condition, first + actions.memory)?.then(actions))
    }
}

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn payload(package: &UpkPackage, path: &str) -> Result<(usize, Vec<u8>), String> {
    let reference = package
        .find_object(path)
        .filter(|r| *r > 0)
        .ok_or("History function missing")?;
    let entry = &package.exports[reference as usize - 1];
    Ok((
        entry.table_index,
        package.image[entry.serial_offset..entry.serial_offset + entry.serial_size].into(),
    ))
}
// The former installer emitted the same expressions. Undo only those
// expressions and verify the recovered vanilla payload before saving.
fn original(
    before: &[u8],
    code: &Code,
    at_end: bool,
    hash: &str,
) -> Result<Option<Vec<u8>>, String> {
    if hex::encode(Sha256::digest(before)) == hash {
        return Ok(None);
    }
    let invalid = || "Unrecognized MatchHistory function in TAGame.upk.".to_owned();
    let at = hook_offset(before, at_end).ok_or_else(invalid)?;
    let at = if at_end {
        at.checked_sub(code.bytes.len()).ok_or_else(invalid)?
    } else {
        at
    };
    if before.get(at..at + code.bytes.len()) != Some(code.bytes.as_slice()) {
        return Err(invalid());
    }
    let memory = word(before, 40)
        .checked_sub(code.memory as u32)
        .ok_or_else(invalid)?;
    let storage = word(before, 44)
        .checked_sub(code.bytes.len() as u32)
        .ok_or_else(invalid)?;
    let mut vanilla = before.to_vec();
    vanilla.drain(at..at + code.bytes.len());
    vanilla[40..44].copy_from_slice(&memory.to_le_bytes());
    vanilla[44..48].copy_from_slice(&storage.to_le_bytes());
    if hex::encode(Sha256::digest(&vanilla)) != hash {
        return Err(invalid());
    }
    Ok(Some(vanilla))
}

fn hook_offset(before: &[u8], at_end: bool) -> Option<usize> {
    before.get(..48)?;
    let end = 48usize.checked_add(word(before, 44) as usize)?;
    let tail = end.checked_sub(3)?;
    (tail >= 48 && before.get(tail..end)? == [4, 11, 76]).then_some(if at_end { tail } else { 48 })
}

fn insert(before: &[u8], code: &Code, at_end: bool) -> Result<Vec<u8>, String> {
    let at = hook_offset(before, at_end).ok_or("Unexpected MatchHistory function return.")?;
    let mut after = before.to_vec();
    after.splice(at..at, code.bytes.iter().copied());
    after[40..44].copy_from_slice(&(word(before, 40) + code.memory as u32).to_le_bytes());
    after[44..48].copy_from_slice(&(word(before, 44) + code.bytes.len() as u32).to_le_bytes());
    Ok(after)
}
