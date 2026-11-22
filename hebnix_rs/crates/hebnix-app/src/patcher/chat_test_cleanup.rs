//! Hebnix 2.3.1 had a Chat Test that put a block at the end of
//! MatchStatsExporter_TA.ProcessSocketCommand for {"Command":"HebnixChatTest",...}.
//! Later versions dropped the feature, but a game it was turned on in still has
//! the block. Other features that edit the function (Auto Upload, the Workshop
//! host) only accept blocks they know, so the leftover one is recognized here,
//! built the same way it was installed, and taken out.
use super::script_patch::{Script, replace_script};
use super::upk_package::UpkPackage;
use super::workshop_host::{COMMAND, call, context, field, jump, native, object, payload, store, string};

const PREFIX: &str = "{\"Command\":\"HebnixChatTest\",\"Data\":";
const CHAT: &str = "GFxData_Chat_TA";
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

/// The three message formats the Chat Test was released with.
#[derive(Clone, Copy)]
enum Format {
    NamedSender,
    PlainNotification,
    ColoredNotification,
}

/// The block as 2.3.1 wrote it, its jump counted from the start of the
/// function: `base` is where it starts, in memory form.
fn block(upk: &UpkPackage, base: usize, format: Format) -> Result<Script, String> {
    let command = field(upk, 0x46, &format!("{COMMAND}.CommandJson"))?;
    let length = token(&[0x24, PREFIX.len() as u8]);
    let left = native(0x80, &[command.clone(), length]);
    let mut condition = Script::new();
    condition.raw(&[0xbf]).append(&left).append(&string(PREFIX)).raw(&[0x16]);

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
        call(upk, "ProjectX.GFxDataStore_X.GetObject", &[field(upk, 0x20, CHAT)?, token(&[0x48])])?,
        Some("ProjectX.GFxDataStore_X.GetObject.ReturnValue"),
    )?;
    let mut chat = field(upk, 0x2e, CHAT)?;
    chat.append(&get_chat);
    let (team, name, message) = match format {
        Format::NamedSender => (token(&[0x25]), string("[HEBNIX]"), text),
        Format::PlainNotification => (
            token(&[0x1d, 0xff, 0xff, 0xff, 0xff]),
            string(""),
            native(0xf8, &[string("[HEBNIX] "), text]),
        ),
        Format::ColoredNotification => (token(&[0x1d, 0xff, 0xff, 0xff, 0xff]), string(""), text),
    };
    let flash = call(
        upk,
        "GFxData_Chat_TA.OnChatMessage",
        &[team, name, message, token(&[0x24, 0]), token(&[0x28]), sender, token(&[0x24, 0]), string("")],
    )?;
    let mut body = context(upk, chat, flash, None)?;
    body.raw(&[0x04, 0x0b]);
    let mut out = jump(base + 3 + condition.mem_size() + body.mem_size(), &condition)?;
    out.append(&body);
    Ok(out)
}

/// Whether the function still has the Chat Test block somewhere.
pub(crate) fn present(script: &Script) -> bool {
    script.bytes().windows(PREFIX.len()).any(|part| part == PREFIX.as_bytes())
}

/// Take the block out if it sits right before the function's end. Ok(false)
/// when it isn't there (another block may be after it), nothing changed.
pub(crate) fn remove_at_tail(upk: &mut UpkPackage) -> Result<bool, String> {
    let (_, mut script) = payload(upk)?;
    if !present(&script) || !script.bytes().ends_with(END) {
        return Ok(false);
    }
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
            script.cut(start, hook.len(), hook.refs());
            replace_script(upk, COMMAND, &script)?;
            return Ok(true);
        }
    }
    Ok(false)
}
