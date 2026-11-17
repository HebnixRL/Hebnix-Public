//! Local match chat through the StatsAPI command socket and the HUD's Flash event.
use super::script_patch::{Script, replace_script};
use super::upk_package::UpkPackage;
use super::workshop_host::{
    COMMAND, call, context, field, jump, native, object, payload, store, string,
};
use eframe::egui;
use hebnix_sdk::stats::{WsCommand, websocket::WsStatsClient};
use std::{path::Path, sync::mpsc};

const NAME: &str = "HebnixChatTest";
const PREFIX: &str = "{\"Command\":\"HebnixChatTest\",\"Data\":";
const CHAT: &str = "GFxData_Chat_TA";
const ENABLED: &str = "ChatTest.enabled";
const END: &[u8] = &[0x04, 0x0b, 0x4c];

fn token(bytes: &[u8]) -> Script {
    Script::from_parts(bytes.to_vec(), 0)
}

fn class_call(upk: &UpkPackage, class: &str, body: Script, result: &str) -> Result<Script, String> {
    let mut out = Script::new();
    out.raw(&[0x5e, 0x12, 0, 0x20])
        .object(object(upk, class)?)
        .raw(&(body.mem_size() as u16).to_le_bytes())
        .object(object(upk, result)?)
        .raw(&[0])
        .append(&body);
    Ok(out)
}

#[derive(Clone, Copy)]
enum Format {
    NamedSender,
    PlainNotification,
    ColoredNotification,
}

fn block(upk: &UpkPackage, base: usize, format: Format) -> Result<Script, String> {
    let command = field(upk, 0x46, &format!("{COMMAND}.CommandJson"))?;
    let length = token(&[0x24, PREFIX.len() as u8]);
    let left = native(0x80, &[command.clone(), length]);
    let mut condition = Script::new();
    condition
        .raw(&[0xbf])
        .append(&left)
        .append(&string(PREFIX))
        .raw(&[0x16]);

    let json = class_call(
        upk,
        "Engine.JsonObject",
        call(upk, "Engine.JsonObject.DecodeJson", &[command])?,
        "Engine.JsonObject.DecodeJson.ReturnValue",
    )?;
    let data = context(
        upk,
        json,
        call(upk, "Engine.JsonObject.GetObject", &[string("Data")])?,
        Some("Engine.JsonObject.GetObject.ReturnValue"),
    )?;
    let text = context(
        upk,
        data,
        call(
            upk,
            "Engine.JsonObject.GetStringValue",
            &[string(match format {
                Format::ColoredNotification => "FormattedText",
                _ => "Text",
            })],
        )?,
        Some("Engine.JsonObject.GetStringValue.ReturnValue"),
    )?;
    let controller = field(upk, 0x01, "MatchStatsExporter_TA.PrimaryPC")?;
    let pri = context(
        upk,
        controller,
        field(upk, 0x01, "Engine.Controller.PlayerReplicationInfo")?,
        Some("Engine.Controller.PlayerReplicationInfo"),
    )?;
    let sender = context(
        upk,
        pri,
        field(upk, 0x01, "Engine.PlayerReplicationInfo.UniqueId")?,
        Some("Engine.PlayerReplicationInfo.UniqueId"),
    )?;
    let get_chat = context(
        upk,
        store(upk)?,
        call(
            upk,
            "ProjectX.GFxDataStore_X.GetObject",
            &[field(upk, 0x20, CHAT)?, token(&[0x48])],
        )?,
        Some("ProjectX.GFxDataStore_X.GetObject.ReturnValue"),
    )?;
    let mut chat = field(upk, 0x2e, CHAT)?;
    chat.append(&get_chat);
    // A named HUD sender always gets a colon. Use the notification format
    // with the prefix included in the text to display the exact label.
    let (team, name, message) = match format {
        Format::NamedSender => (token(&[0x25]), string("[HEBNIX]"), text),
        Format::PlainNotification => (
            token(&[0x1d, 0xff, 0xff, 0xff, 0xff]),
            string(""),
            native(0xf8, &[string("[HEBNIX] "), text]),
        ),
        Format::ColoredNotification => (
            token(&[0x1d, 0xff, 0xff, 0xff, 0xff]),
            string(""),
            text,
        ),
    };
    let flash = call(
        upk,
        "GFxData_Chat_TA.OnChatMessage",
        &[
            team,
            name,
            message,
            token(&[0x24, 0]),
            token(&[0x28]),
            sender,
            token(&[0x24, 0]),
            string(""),
        ],
    )?;
    let mut body = context(upk, chat, flash, None)?;
    body.raw(&[0x04, 0x0b]);
    let mut out = jump(
        base + 3 + condition.mem_size() + body.mem_size(),
        &condition,
    )?;
    out.append(&body);
    Ok(out)
}

fn has(upk: &UpkPackage) -> Result<bool, String> {
    Ok(payload(upk)?
        .1
        .bytes()
        .windows(PREFIX.len())
        .any(|part| part == PREFIX.as_bytes()))
}

fn edit_tail(upk: &mut UpkPackage, on: bool) -> Result<(), String> {
    let (_, mut script) = payload(upk)?;
    if !script.bytes().ends_with(END) {
        return Err(format!(
            "{COMMAND} changed layout in this Rocket League version."
        ));
    }
    if on {
        let hook = block(upk, script.mem_size() - END.len(), Format::ColoredNotification)?;
        script.insert_before_tail(END.len(), &hook);
    } else {
        let mut matched = None;
        for format in [Format::ColoredNotification, Format::PlainNotification, Format::NamedSender] {
            let probe = block(upk, 0, format)?;
            let Some(start) = script.len().checked_sub(END.len() + probe.len()) else {
                continue;
            };
            let Some(base) = script.mem_size().checked_sub(END.len() + probe.mem_size()) else {
                continue;
            };
            let hook = block(upk, base, format)?;
            if script.bytes().get(start..start + hook.len()) == Some(hook.bytes()) {
                matched = Some((start, hook));
                break;
            }
        }
        let (start, hook) = matched.ok_or("The Chat Test hook changed layout.")?;
        script.cut(start, hook.len(), hook.refs());
    }
    replace_script(upk, COMMAND, &script)
}

pub(crate) fn detach(upk: &mut UpkPackage) -> Result<bool, String> {
    let installed = has(upk)?;
    if installed {
        edit_tail(upk, false)?;
    }
    Ok(installed)
}

pub(crate) fn attach(upk: &mut UpkPackage) -> Result<(), String> {
    edit_tail(upk, true)
}

fn edit(upk: &mut UpkPackage, on: bool) -> Result<bool, String> {
    if has(upk)? == on {
        return Ok(false);
    }
    let restart = super::workshop_host::detach_freeplay_restart(upk)?;
    edit_tail(upk, on)?;
    if let Some(version) = restart {
        super::workshop_host::attach_freeplay_restart(upk, version)?;
    }
    Ok(true)
}

fn set_enabled(cooked: &Path, backups: &Path, on: bool) -> Result<(), String> {
    let _lock = super::profile_pictures::PACKAGE_EDIT
        .lock()
        .map_err(|e| e.to_string())?;
    let closed = || {
        if hebnix_sdk::process::is_rocket_league_running() {
            Err("Close Rocket League before changing Chat Test.".to_owned())
        } else {
            Ok(())
        }
    };
    closed()?;
    super::backup_guard::check_install(cooked, backups)?;
    let live = super::game_path::resolve(cooked, "TAGame.upk");
    let before = std::fs::read(&live).map_err(|e| e.to_string())?;
    let mut upk = UpkPackage::from_bytes(before.clone())?;
    if edit(&mut upk, on)? {
        let after = upk.repacked_fast()?;
        if has(&UpkPackage::from_bytes(after.clone())?)? != on {
            return Err("Chat Test patch validation failed.".into());
        }
        closed()?;
        super::upk_history::install_linked_from(cooked, &live, &before, &after, "Chat Test")?;
    }
    let marker = backups.join(ENABLED);
    if on {
        super::upk_history::replace(&marker, b"on")
    } else {
        match std::fs::remove_file(marker) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }
}

pub(super) fn recover_after_update(cooked: &Path, archive: &Path) -> Result<(), String> {
    if archive.join(ENABLED).is_file() {
        set_enabled(cooked, &cooked.join("Backups"), true)?;
    }
    Ok(())
}

pub(super) fn restore(cooked: &Path, backups: &Path) -> Result<(), String> {
    if backups.join(ENABLED).is_file() {
        set_enabled(cooked, backups, false)?;
    }
    Ok(())
}

const MAX_CHARS: usize = 256;

pub(crate) fn enabled(backups: &Path) -> bool {
    backups.join(ENABLED).is_file()
}

/// Queue a message on the current local StatsAPI connection.
pub(crate) fn send(socket: &WsStatsClient, text: &str) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > MAX_CHARS || text.chars().any(char::is_control) {
        return Err("Enter a single line of 1–256 characters.".into());
    }
    if !socket.connected() {
        return Err("Rocket League StatsAPI is disconnected.".into());
    }
    let escaped = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let formatted = format!("<font color=\"#FF0000\">[HEBNIX]</font> <font color=\"#FFFFFF\">{escaped}</font>");
    let command = WsCommand::with_data(
        NAME,
        serde_json::json!({ "Text": text, "FormattedText": formatted }),
    )
        .map_err(|e| e.to_string())?;
    socket.send_command(command).map_err(|e| e.to_string())
}

#[derive(Default)]
pub struct State {
    text: String,
    pending: Option<mpsc::Receiver<Result<(), String>>>,
    status: String,
}

impl State {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn tick(&mut self) {
        if let Some(result) = self.pending.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("Chat Test patch operation stopped.".into()))
            }
            Err(mpsc::TryRecvError::Empty) => None,
        }) {
            self.pending = None;
            self.status = result.err().unwrap_or_default();
        }
    }

    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        cooked: &Path,
        backups: &Path,
        socket: &WsStatsClient,
    ) {
        use crate::i18n::t;
        let enabled = enabled(backups);
        let running = hebnix_sdk::process::is_rocket_league_running();
        let mut wanted = enabled;
        if ui
            .add_enabled(
                !running && !self.busy(),
                egui::Checkbox::new(&mut wanted, t("game-mod-chat-test")),
            )
            .on_disabled_hover_text(t("game-mod-chat-test-close"))
            .changed()
        {
            let (tx, rx) = mpsc::channel();
            self.pending = Some(rx);
            self.status.clear();
            let (cooked, backups, ctx) = (cooked.to_owned(), backups.to_owned(), ui.ctx().clone());
            std::thread::spawn(move || {
                let _ = tx.send(set_enabled(&cooked, &backups, wanted));
                ctx.request_repaint();
            });
        }
        ui.separator();
        ui.label(t("game-mod-chat-test-description"));
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.text)
                    .hint_text(t("game-mod-chat-test-placeholder"))
                    .char_limit(MAX_CHARS),
            );
            if ui
                .add_enabled(
                    enabled
                        && running
                        && socket.connected()
                        && !self.busy()
                        && !self.text.trim().is_empty(),
                    egui::Button::new(t("game-mod-chat-test-send")),
                )
                .on_disabled_hover_text(t("game-mod-chat-test-connect"))
                .clicked()
            {
                self.status = send(socket, &self.text).err().unwrap_or_default();
            }
        });
        if self.busy() {
            ui.spinner();
        }
        if !self.status.is_empty() {
            ui.label(&self.status);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires HEBNIX_UPK_TEST_SOURCE pointing to TAGame.upk"]
    fn real_package_round_trip() {
        let source = std::env::var_os("HEBNIX_UPK_TEST_SOURCE").unwrap();
        let mut package = UpkPackage::load(Path::new(&source)).unwrap();
        edit(&mut package, false).unwrap();
        let before = payload(&package).unwrap().1;
        let restart = super::super::workshop_host::detach_freeplay_restart(&mut package).unwrap();
        let (_, mut script) = payload(&package).unwrap();
        let legacy = block(&package, script.mem_size() - END.len(), Format::NamedSender).unwrap();
        script.insert_before_tail(END.len(), &legacy);
        replace_script(&mut package, COMMAND, &script).unwrap();
        if let Some(version) = restart {
            super::super::workshop_host::attach_freeplay_restart(&mut package, version).unwrap();
        }
        assert!(edit(&mut package, false).unwrap());
        assert_eq!(payload(&package).unwrap().1.bytes(), before.bytes());
        assert!(edit(&mut package, true).unwrap());
        let packed = package.repacked_fast().unwrap();
        let mut reloaded = UpkPackage::from_bytes(packed).unwrap();
        assert!(has(&reloaded).unwrap());
        assert!(!edit(&mut reloaded, true).unwrap());
        assert!(edit(&mut reloaded, false).unwrap());
        let restored = payload(&reloaded).unwrap().1;
        assert_eq!(restored.bytes(), before.bytes());
        assert_eq!(restored.mem_size(), before.mem_size());
        let command = WsCommand::with_data(NAME, serde_json::json!({"Text": "Hello"})).unwrap();
        assert!(serde_json::to_string(&command).unwrap().starts_with(PREFIX));
    }
}
