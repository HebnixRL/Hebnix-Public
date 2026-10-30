//! Profile-backed, per-identity extension of the validated avatar proof.
//! Does not add fields to native-backed instances or add exports/imports.
use super::*;
use std::collections::HashSet;
const PREFIX: &str = "http://127.0.0.1:17895/profile/";
const PERSONA: &str = "GFxData_Workshop_TA.OnAvatarPersonaHandler";

fn text(value: &str) -> Code {
    let mut code = Code::raw(&[0x1f]);
    code.bytes.extend(value.as_bytes());
    code.bytes.push(0);
    code
}
fn call(package: &UpkPackage, function: &str, args: Code) -> Result<Code, String> {
    Ok(Code::raw(&[0x1c])
        .reference(object(package, function)?)
        .then(args)
        .then(Code::raw(&[0x16])))
}
fn virtual_call(package: &UpkPackage, function: &str, args: Code) -> Result<Code, String> {
    Ok(Code::raw(&[0x1b])
        .then(name(package, function)?)
        .then(args)
        .then(Code::raw(&[0x16])))
}
fn delegate(package: &UpkPackage, function: &str) -> Result<Code, String> {
    Ok(Code::raw(&[0x43])
        .then(name(package, function)?)
        .reference(0))
}
fn image_url(package: &UpkPackage) -> Result<Code, String> {
    Ok(Code::raw(&[0x35])
        .reference(object(
            package,
            "IpDrv.OnlineImageDownloaderWeb.OnlineImageDownload.URL",
        )?)
        .reference(object(
            package,
            "IpDrv.OnlineImageDownloaderWeb.OnlineImageDownload",
        )?)
        .then(Code::raw(&[0, 0]))
        .then(variable(package, 0x2b, &format!("{CALLBACK}.Image"))?))
}
fn local_id(package: &UpkPackage) -> Result<Code, String> {
    let player = field(
        package,
        variable(package, 0x01, "ProjectX.GFxDataRow_X.Shell")?,
        "ProjectX.GFxShell_X.Player",
    )?;
    let online = field(package, player, "ProjectX.LocalPlayer_X.OnlinePlayer")?;
    field(package, online, "ProjectX.OnlinePlayer_X.PlayerID")
}
fn target_id(package: &UpkPackage) -> Result<Code, String> {
    let length = Code::raw(&[0xef])
        .then(image_url(package)?)
        .then(Code::raw(&[0x16]));
    let length = Code::raw(&[0x93])
        .then(length)
        .then(Code::raw(&[0x2c, PREFIX.len() as u8, 0x16]));
    let suffix = Code::raw(&[0xea])
        .then(image_url(package)?)
        .then(length)
        .then(Code::raw(&[0x16]));
    let convert = virtual_call(package, "StringToUniqueNetId", suffix)?;
    // Native converter's return is UniqueNetId, matching this existing property.
    static_context(
        package,
        "ProjectX.Online_X",
        convert,
        object(package, "PlayerVanity_TA.PlayerID")?,
    )
}
fn stringify(package: &UpkPackage, id: Code) -> Result<Code, String> {
    let convert = virtual_call(package, "UniqueNetIdToString", id)?;
    static_context(
        package,
        "ProjectX.Online_X",
        convert,
        object(package, "PlayerVanity_TA.PlayerName")?,
    )
}
fn request_url(package: &UpkPackage, id: Code) -> Result<Code, String> {
    // F8 concatenates directly. A8 is the space-inserting @ operator and
    // produced invalid URLs such as /profile/ Epic|...|0 in profiles-2.
    Ok(Code::raw(&[0xf8])
        .then(text(PREFIX))
        .then(stringify(package, id)?)
        .then(Code::raw(&[0x16])))
}
fn downloader(package: &UpkPackage) -> Result<Code, String> {
    let provider = call(package, "Core.Object.GetObjectProvider", Code::default())?;
    let game = context(
        provider,
        call(
            package,
            "Core.ObjectProvider.Get",
            Code::raw(&[0x20]).reference(object(package, "ProjectX.OnlineGame_Base_X")?),
        )?,
        object(package, "Core.ObjectProvider.Get.ReturnValue")?,
    )?;
    field(package, game, "ProjectX.OnlineGame_Base_X.ImageDownloader")
}
fn condition_not_none(code: Code) -> Code {
    Code::raw(&[0xce]).then(code).then(Code::raw(&[0x2a, 0x16]))
}
fn condition_none(code: Code) -> Code {
    Code::raw(&[0xcf]).then(code).then(Code::raw(&[0x2a, 0x16]))
}
fn log(message: &str) -> Code {
    Code::raw(&[0xe7])
        .then(text(message))
        .then(Code::raw(&[0x48, 0x48, 0x16]))
}

// Branch targets are absolute expanded memory offsets. Construction is kept
// separate from expression validation so every patch site binds its own base.
fn guarded(condition: Code, body: Code, base: usize) -> Result<Code, String> {
    validate(&condition)?;
    validate(&body)?;
    let end = u16::try_from(base + 3 + condition.memory() + body.memory())
        .map_err(|_| "Guard branch overflow")?;
    Ok(Code::raw(&[0x07])
        .then(Code::raw(&end.to_le_bytes()))
        .then(condition)
        .then(body))
}
fn ensure_names(package: &mut UpkPackage) -> Result<Vec<(String, String)>, String> {
    let wanted = [
        "StringToUniqueNetId",
        "QueueURLForDownload",
        "IsURLTracked",
        "AddCallbackToDownload",
        "OnDownloadComplete",
        "DownloadNextImage",
    ];
    let mut referenced = vec![false; package.names.len()];
    // Follow the existing RE kit's same-length unused-name technique. Four
    // bytes alone also match unrelated object indices, so look for complete
    // FNames with the instance range used by that kit, plus sentinel -1.
    // Export/import names are explicitly protected regardless of instance.
    for w in package.image.windows(8) {
        let i = u32::from_le_bytes(w[..4].try_into().unwrap()) as usize;
        let instance = u32::from_le_bytes(w[4..].try_into().unwrap());
        if i < referenced.len() && (instance <= 1000 || instance == u32::MAX) {
            referenced[i] = true;
        }
    }
    for export in &package.exports {
        referenced[export.object_name.name_index] = true;
    }
    let lookup: std::collections::HashMap<_, _> = package
        .names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    for (class_package, class_name, path) in package.import_debug() {
        for value in [&class_package, &class_name, &path] {
            for component in value.split('.') {
                if let Some(&index) = lookup.get(crate::patcher::upk_package::strip(component)) {
                    referenced[index] = true;
                }
            }
        }
    }
    let mut renamed = Vec::new();
    let mut edits = Vec::new();
    for value in wanted {
        if package.names.iter().any(|name| name == value) {
            continue;
        }
        let index = package
            .names
            .iter()
            .enumerate()
            .filter(|(i, name)| {
                *i > 256
                    && !referenced[*i]
                    && name.len() > 4
                    && name.bytes().all(|b| b.is_ascii_alphanumeric())
                    && !wanted.contains(&name.as_str())
            })
            .min_by_key(|(_, name)| name.len().abs_diff(value.len()))
            .map(|(i, _)| i)
            .ok_or_else(|| format!("No unused name slot for {value}"))?;
        let old = package.names[index].clone();
        referenced[index] = true;
        let delta = value.len() as isize - old.len() as isize;
        if delta != 0 {
            let donor = package
                .names
                .iter()
                .enumerate()
                .find(|(i, n)| {
                    *i > 256
                        && !referenced[*i]
                        && n.bytes().all(|b| b.is_ascii_alphanumeric())
                        && n.len() as isize - delta >= 16
                        && !wanted.contains(&n.as_str())
                })
                .map(|(i, _)| i)
                .ok_or_else(|| format!("No unused name-size donor for {value}"))?;
            let size = (package.names[donor].len() as isize - delta) as usize;
            let mut replacement = format!("HbxUnused{donor}");
            replacement.extend(std::iter::repeat_n('X', size - replacement.len()));
            if package.names.contains(&replacement) {
                return Err("Name donor collision".into());
            }
            referenced[donor] = true;
            renamed.push((package.names[donor].clone(), replacement.clone()));
            edits.push((donor, replacement));
        }
        renamed.push((old, value.into()));
        edits.push((index, value.into()));
    }
    package.rename_names_same_size(&edits)?;
    Ok(renamed)
}

fn profile_request(package: &UpkPackage) -> Result<Code, String> {
    let url = request_url(
        package,
        variable(package, 0x2b, &format!("{PERSONA}.PlayerNetId"))?,
    )?;
    let downloader = downloader(package)?;
    let tracked = context(
        downloader.clone(),
        virtual_call(package, "IsURLTracked", url.clone())?,
        object(package, "PlayerVanity_TA.HasPlayerID.ReturnValue")?,
    )?;
    // A tracked URL already has this bridge's completion callback. Returning
    // avoids synchronous cached callbacks recursively creating avatar rows.
    let add = Code::default();
    validate(&tracked)?;
    validate(&add)?;
    // Internal HTTP completion must be bound to the downloader, while the
    // decoded-image completion remains bound to this bridge data object.
    let internal = context(
        downloader.clone(),
        delegate(package, "OnDownloadComplete")?,
        object(package, "VanitySetManager_TA.ProcessAvatars.Callback")?,
    )?;
    let queued = context(
        downloader.clone(),
        virtual_call(
            package,
            "QueueURLForDownload",
            url.then(internal)
                .then(delegate(package, "OnEventImageDownloaded")?)
                .then(Code::raw(&[0x28])),
        )?,
        0,
    )?;
    let drain = context(
        downloader,
        virtual_call(package, "DownloadNextImage", Code::default())?,
        0,
    )?;
    let request_log = Code::raw(&[0xe7, 0xa8])
        .then(text("HEBNIX_PROFILE_REQUEST: "))
        .then(request_url(
            package,
            variable(package, 0x2b, &format!("{PERSONA}.PlayerNetId"))?,
        )?)
        .then(Code::raw(&[0x16, 0x48, 0x48, 0x16]));
    let queued = request_log.then(queued).then(drain);
    validate(&queued)?;
    let no_queue = 3 + tracked.memory() + add.memory() + 2;
    let mut code = Code::raw(&[0x07])
        .then(Code::raw(&(no_queue as u16).to_le_bytes()))
        .then(tracked)
        .then(add)
        .then(Code::raw(&[0x04, 0x0b]));
    code = code.then(queued).then(Code::raw(&[0x04, 0x0b, 0x4c]));
    Ok(code)
}
fn generic_assignment(package: &UpkPackage) -> Result<Code, String> {
    let id = target_id(package)?;
    let manager = field(
        package,
        data_row(package, "GFxData_PlayerVanity_TA")?,
        "GFxData_PlayerVanity_TA.VanityMgr",
    )?;
    let avatar = context(
        manager,
        call(package, "VanitySetManager_TA.GetAvatar", id.clone())?,
        object(package, "VanitySetManager_TA.GetAvatar.ReturnValue")?,
    )?;
    let update = context(
        avatar.clone(),
        call(
            package,
            "PlayerAvatar_TA.HandleUpdateTexture",
            image_texture(package)?,
        )?,
        0,
    )?;
    let row = static_context(
        package,
        "GFxData_PlayerAvatar_TA",
        call(
            package,
            "GFxData_PlayerAvatar_TA.GetOrCreate",
            variable(package, 0x01, "ProjectX.GFxDataRow_X.Shell")?.then(avatar),
        )?,
        object(package, "GFxData_PlayerAvatar_TA.GetOrCreate.ReturnValue")?,
    )?;
    let ids = "GFxData_Workshop_TA.RetrievedNetIds";
    let array = Code::raw(&[0x57, 0x28])
        .then(variable(package, 0x01, ids)?)
        .then(Code::raw(&[0, 0]))
        .then(id)
        .then(Code::raw(&[0x16]))
        .then(variable(package, 0x01, ids)?);
    let notify = context(
        data_row(package, "GFxData_PlayerVanity_TA")?,
        call(
            package,
            "GFxData_PlayerVanity_TA.OnPlayerAvatarsUpdated",
            array,
        )?,
        0,
    )?;
    let code = update.then(row).then(notify).then(log(
        "HEBNIX_PROFILE_AVATAR: image assigned to URL identity and UI notified",
    ));
    validate(&code)?;
    Ok(code)
}
fn generic_callback(package: &UpkPackage, original: &[u8]) -> Result<(Vec<u8>, i32), String> {
    let prefix = Code::raw(&[0x9a, 0xd1])
        .then(image_url(package)?)
        .then(text(PREFIX))
        .then(Code::raw(&[0x16, 0x25, 0x16]));
    validate(&prefix)?;
    let texture = condition_not_none(image_texture(package)?);
    let assignment = generic_assignment(package)?;
    // Prefix mismatch goes to vanilla Workshop behavior. Failed avatar
    // downloads return without changing any player's current texture.
    let texture_base = 3 + prefix.memory();
    let texture_guard = guarded(texture, assignment, texture_base)?;
    let length = 3 + prefix.memory() + texture_guard.memory() + 2;
    let mut code = Code::raw(&[0x07])
        .then(Code::raw(&(length as u16).to_le_bytes()))
        .then(prefix)
        .then(texture_guard)
        .then(Code::raw(&[0x04, 0x0b]));
    let mut vanilla = original.to_vec();
    if vanilla.get(42..45) != Some(&[0x07, 0x7a, 0]) {
        return Err("Unexpected Workshop callback layout".into());
    }
    vanilla[43..45].copy_from_slice(
        &u16::try_from(122 + length)
            .map_err(|_| "Callback branch overflow")?
            .to_le_bytes(),
    );
    let memory = 194 + length as i32;
    code.bytes.extend(vanilla);
    Ok((code.bytes, memory))
}
fn avatar_row_hook(package: &UpkPackage, base: usize) -> Result<Code, String> {
    let avatar = variable(
        package,
        0x2b,
        "GFxData_PlayerAvatar_TA.SetAvatar.InPlayerAvatar",
    )?;
    let bridge = data_row(package, "GFxData_Workshop_TA")?;
    let store = field(
        package,
        variable(package, 0x01, "ProjectX.GFxDataRow_X.Shell")?,
        "ProjectX.GFxShell_X.DataStore",
    )?;
    let create = context(
        store,
        call(
            package,
            "ProjectX.GFxDataStore_X.CreateObject",
            Code::raw(&[0x20])
                .reference(object(package, "GFxData_Workshop_TA")?)
                .then(Code::raw(&[0x48, 0x48])),
        )?,
        object(package, "ProjectX.GFxDataStore_X.CreateObject.ReturnValue")?,
    )?;
    let live = condition_not_none(avatar.clone());
    // Bind the nested guards from their actual absolute memory starts.
    let inside = base + 3 + live.memory();
    let ensure = guarded(condition_none(bridge.clone()), create, inside)?;
    let request = context(
        bridge,
        call(
            package,
            PERSONA,
            field(package, avatar, "PlayerVanity_TA.PlayerID")?
                .then(Code::raw(&[0x2a]))
                .then(text("")),
        )?,
        0,
    )?;
    validate(&request)?;
    let mut inner = ensure;
    inner = inner.then(request);
    // inner includes a bound guard; validate its straight-line constituents
    // above rather than treating an absolute control-flow block as an expr.
    let end = base + 3 + live.memory() + inner.memory();
    let code = Code::raw(&[0x07])
        .then(Code::raw(&(end as u16).to_le_bytes()))
        .then(live)
        .then(inner);
    Ok(code)
}

fn replace_function(
    package: &mut UpkPackage,
    path: &str,
    body: &[u8],
    memory: i32,
    expected: (i32, i32),
) -> Result<usize, String> {
    let index = object(package, path)? as usize - 1;
    let export = package.exports[index].clone();
    let old_memory = package.read_int(export.serial_offset + 40)?;
    let old_storage = package.read_int(export.serial_offset + 44)?;
    if (old_memory, old_storage) != expected {
        return Err(format!("{path} changed layout: {old_memory}/{old_storage}"));
    }
    let old = &package.image[export.serial_offset..export.serial_offset + export.serial_size];
    let mut payload = old[..48].to_vec();
    payload[40..44].copy_from_slice(&memory.to_le_bytes());
    payload[44..48].copy_from_slice(&(body.len() as i32).to_le_bytes());
    payload.extend(body);
    payload.extend(&old[48 + old_storage as usize..]);
    package.replace_export_payload(index, &payload)?;
    Ok(index)
}
fn function_body(package: &UpkPackage, path: &str) -> Result<Vec<u8>, String> {
    let e = &package.exports[object(package, path)? as usize - 1];
    let storage = package.read_int(e.serial_offset + 44)? as usize;
    Ok(package.image[e.serial_offset + 48..e.serial_offset + 48 + storage].to_vec())
}

pub(super) struct Changes {
    pub(super) exports: Vec<usize>,
    pub(super) renamed: Vec<(String, String)>,
}
pub(super) fn patch(package: &mut UpkPackage) -> Result<Changes, String> {
    let persona = &package.exports[object(package, PERSONA)? as usize - 1];
    let persona_memory = package.read_int(persona.serial_offset + 40)?;
    if ![162, 182].contains(&persona_memory) {
        return Err("Avatar callback changed in this Rocket League version".into());
    }
    let renamed = ensure_names(package)?;
    let mut changed = HashSet::new();
    let callback_original = function_body(package, CALLBACK)?;
    let (callback, memory) = generic_callback(package, &callback_original)?;
    changed.insert(replace_function(
        package,
        CALLBACK,
        &callback,
        memory,
        (194, 142),
    )?);
    let request = profile_request(package)?;
    changed.insert(replace_function(
        package,
        PERSONA,
        &request.bytes,
        request.memory() as i32,
        (persona_memory, 114),
    )?);
    let set_avatar = "GFxData_PlayerAvatar_TA.SetAvatar";
    let original = function_body(package, set_avatar)?;
    if original.len() != 40 || original[37..] != [0x04, 0x0b, 0x4c] {
        return Err("Avatar attachment function changed".into());
    }
    let hook = avatar_row_hook(package, 53)?;
    let mut body = original[..37].to_vec();
    body.extend(&hook.bytes);
    body.extend([0x04, 0x0b, 0x4c]);
    changed.insert(replace_function(
        package,
        set_avatar,
        &body,
        56 + hook.memory() as i32,
        (56, 40),
    )?);
    // Proactively create rows for menu requests even when Epic's normal
    // subsystem has no avatar provider. The original request still runs.
    let update = "GFxData_PlayerVanity_TA.UpdatePlayerAvatars";
    let original = function_body(package, update)?;
    let args = variable(package, 0x2b, &format!("{update}.PlayerIds"))?
        .then(variable(package, 0x01, "ProjectX.GFxDataRow_X.Shell")?)
        .then(delegate(package, "OnPlayerAvatarsUpdated")?);
    // Parameter order follows the actual property Next chain:
    // PlayerIds -> Shell -> Callback -> local PlayerID.
    let process = context(
        variable(package, 0x01, "GFxData_PlayerVanity_TA.VanityMgr")?,
        call(package, "VanitySetManager_TA.ProcessAvatars", args)?,
        0,
    )?;
    validate(&process)?;
    let mut body = process.bytes.clone();
    body.extend(original);
    changed.insert(replace_function(
        package,
        update,
        &body,
        111 + process.memory() as i32,
        (111, 75),
    )?);
    Ok(Changes { exports: changed.into_iter().collect(), renamed })
}
