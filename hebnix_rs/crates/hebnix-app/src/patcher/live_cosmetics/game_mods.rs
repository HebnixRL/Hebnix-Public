//! Dormant Game Mods, prepared once and enabled by file flags in the game.
use super::runtime::*;
use crate::patcher::{actions, bots_freeplay, celebration, chat_export, dispatcher, forfeit_command, lan_commands,
    script_patch::{Script, function_payload}, upk_package::UpkPackage};
use std::path::Path;
type R<T> = Result<T, String>;
pub const TARGETS: &[&str] = &["PlayerController_TA.NetClientInputRate", "PlayerInput_TA.GetDefaultActions",
    "HUDBase_TA.AddChatMessage", "GFxData_TextModerationManager_TA.HandleMessageReceived", "TextChatActor_TA.ServerSendMessageToPlayers"];
fn bytes(p: &UpkPackage, path: &str) -> R<(usize, Vec<u8>)> {
    let index = p.find_object(path).filter(|i| *i > 0).ok_or_else(|| format!("Missing {path}"))? as usize - 1;
    let e = &p.exports[index]; Ok((index, p.image[e.serial_offset..e.serial_offset + e.serial_size].to_vec()))
}
fn choice(condition: Script, yes: Script, no: Script) -> R<Script> {
    let mut code = raw(&[0x45]);
    code.append(&condition).raw(&u16::try_from(yes.mem_size()).map_err(|_| "Conditional overflow")?.to_le_bytes()).append(&yes)
        .raw(&u16::try_from(no.mem_size()).map_err(|_| "Conditional overflow")?.to_le_bytes()).append(&no);
    Ok(code)
}
fn save(p: &UpkPackage, key: &str, value: Script) -> R<Script> {
    // Use the same provider as runtime::get. The private getter is a safe
    // receiver for Set once evaluated through its saved-object expression.
    super::runtime::set(p, key, value)
}
pub fn bindings(p: &UpkPackage, base: usize) -> R<Script> {
    let input = var(p, 0x01, "Engine.PlayerController.PlayerInput")?;
    let mut script = Script::new();
    let owner = cast(0x56, input.clone());
    let changed_owner = native(0xc0, &[get(p, "HbxBindOwner")?, owner.clone()]);
    let mut reset = Script::new();
    for id in ["forfeit", "celebration", "bots-freeplay"] { for kind in ["key", "pad"] {
        reset.append(&save(p, &format!("HbxBound.{id}.{kind}"), text(""))?);
    }}
    reset.append(&save(p, "HbxBindOwner", owner)?);
    script.append(&guard(base, changed_owner, reset)?);
    for (slot, id) in [(1, "forfeit"), (2, "celebration"), (3, "bots-freeplay")] { for kind in ["key", "pad"] {
        let bound_key = format!("HbxBound.{id}.{kind}");
        let command_key = format!("HbxPriorBind.{id}.{kind}");
        let previous = get(p, &bound_key)?;
        let wanted = choice(flag(p, &format!("mods.{id}"))?, get(p, &format!("HbxLive.bind.{id}.{kind}"))?, text(""))?;
        let changed = native(0xc0, &[previous.clone(), wanted.clone()]);
        let body_base = base + script.mem_size() + 3 + changed.mem_size();
        let mut body = Script::new();
        let restore = context(p, input.clone(), virtual_call(p, "SetBind", &[cast(0x60, previous.clone()), get(p, &command_key)?])?, None)?;
        body.append(&guard(body_base, native(0xc0, &[previous, text("")]), restore)?);
        let read = context(p, input.clone(), virtual_call(p, "GetBind", &[cast(0x60, wanted.clone())])?, Some("UISavedValues_TA.Get.ReturnValue"))?;
        let mut apply = save(p, &command_key, read)?;
        apply.append(&context(p, input.clone(), virtual_call(p, "SetBind", &[cast(0x60, wanted.clone()), text(&format!("NetClientInputRate {slot}"))])?, None)?);
        body.append(&guard(body_base + body.mem_size(), native(0xc0, &[wanted.clone(), text("")]), apply)?);
        body.append(&save(p, &bound_key, wanted)?);
        script.append(&guard(base + script.mem_size(), changed, body)?);
    }}
    Ok(script)
}
pub fn prepare(p: &mut UpkPackage, backups: &Path) -> R<Vec<(String, Vec<u8>, Vec<u8>)>> {
    crate::patcher::instant_suite::edit_bridge(p, false)?;
    dispatcher::normalize_for_live(p, backups)?;
    chat_export::normalize_for_live(p)?;
    lan_commands::normalize_for_live(p)?;
    for slot in 1..=3 { if actions::has(p, slot) { actions::remove(p, slot)?; } }
    let mut compiled = Vec::new();
    let (_, before) = bytes(p, TARGETS[1])?;
    for slot in 1..=3 { actions::add(p, slot, &format!("NetClientInputRate {slot}"))?; }
    let (index, after) = bytes(p, TARGETS[1])?;
    compiled.push((TARGETS[1].into(), before.clone(), after));
    p.replace_export_payload(index, &before)?;
    let (_, before) = bytes(p, TARGETS[0])?;
    let mut script = Script::new();
    for (id, key, mut body, jumps) in [
        (1, "forfeit", forfeit_command::live_body(p)?, vec![]),
        { let (b, j) = celebration::live_body(p)?; (2, "celebration", b, j) },
        { let (b, j) = bots_freeplay::live_body(p)?; (3, "bots-freeplay", b, j) },
    ] {
        let rate = native(0x9a, &[var(p, 0x2b, "PlayerController_TA.NetClientInputRate.Rate")?, int(id)]);
        let enabled = flag(p, &format!("mods.{key}"))?;
        let base = script.mem_size() + 3 + rate.mem_size();
        let body_base = base + 3 + enabled.mem_size();
        for at in jumps {
            let target = u16::from_le_bytes(body.bytes()[at..at + 2].try_into().unwrap()) as usize;
            body.set_word(at, u16::try_from(target + body_base).map_err(|_| "Game Mod branch overflow")?);
        }
        let mut execute = guard(base, enabled, body)?;
        execute.append(&ret(raw(&[0x0b])));
        script.append(&guard(script.mem_size(), rate, execute)?);
    }
    script.raw(&[0x04, 0x0b, 0x4c]);
    compiled.push((TARGETS[0].into(), before.clone(), function_payload(&before, &script)?));
    for (path, statement) in chat_export::live_bodies(p)? {
        let (_, before) = bytes(p, path)?;
        let mut script = Script::of_function(&before)?;
        if !script.bytes().ends_with(&[0x04, 0x0b, 0x4c]) { return Err("Unexpected chat function ending".into()); }
        let body = guard(script.mem_size() - 3, flag(p, "mods.chat-export")?, statement)?;
        script.insert_before_tail(3, &body);
        compiled.push((path.into(), before.clone(), function_payload(&before, &script)?));
    }
    let path = TARGETS[4]; let (_, before) = bytes(p, path)?;
    let mut script = Script::of_function(&before)?;
    let enabled = flag(p, "mods.lan-commands")?;
    let base = script.mem_size() - 3;
    let body = lan_commands::live_body(p, base + 3 + enabled.mem_size())?;
    script.insert_before_tail(3, &guard(base, enabled, body)?);
    compiled.push((path.into(), before.clone(), function_payload(&before, &script)?));
    Ok(compiled)
}
