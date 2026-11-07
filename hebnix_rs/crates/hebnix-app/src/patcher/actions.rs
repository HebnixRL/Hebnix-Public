//! Key-bindable actions of our own. A key binding names an Action, and
//! PlayerInput_TA.GetDefaultActions is the only place that maps an Action to a
//! console command, one `DefaultActions.AddItem(...)` statement each. This adds
//! statements of the same shape before the function returns.
//!
//! The name table cannot grow, so the action names are one name that already
//! exists plus a number: slot 2 is `Checkpoint_2` in TAInput.ini, the name
//! `Checkpoint` with number 3 in the bytecode (number = suffix + 1). The
//! category is NotShown, which keeps the entry out of the controls menu.
use super::script_patch::{Script, replace_script};
use super::upk_package::UpkPackage;

const FUNCTION: &str = "PlayerInput_TA.GetDefaultActions";
const LIST: &str = "PlayerInput_TA.GetDefaultActions.DefaultActions";
/// An existing name nothing uses as an action.
const ACTION_BASE: &str = "Checkpoint";
const CATEGORY: &str = "NotShown";
/// ChatPreset1's entry, a name, a name and a string member. Every new entry
/// copies its shape and takes its property and local references.
const TEMPLATE: &[u8] = b"ChatPreset 0\0";
/// Return(list), ReturnNothing(return value), EndOfScript.
const TAIL: usize = 13;
/// Object references in one entry: the list, then three members of three and
/// the item.
const REFS: usize = 11;
/// Offsets inside an entry, counted from its first byte.
const ACTION_AT: usize = 27;
const ACTION_PROPERTY: usize = 12;
const STRUCT_AT: usize = 16;
const LOCAL_AT: usize = 23;
const CATEGORY_PROPERTY: usize = 38;
const COMMAND_PROPERTY: usize = 64;

/// The Action name to write in TAInput.ini for `slot`.
pub fn action_name(slot: u32) -> String {
    format!("{ACTION_BASE}_{slot}")
}

fn read(bytes: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

/// Start and end of every `DefaultActions.AddItem` entry in `script`.
fn entries(script: &[u8], list: i32) -> Vec<(usize, usize)> {
    let mut head = vec![0x57, 0x07, 0x2b];
    head.extend(list.to_le_bytes());
    let starts: Vec<usize> = (0..script.len().saturating_sub(head.len()))
        .filter(|&i| script[i..].starts_with(&head))
        .collect();
    let tail = script.len() - TAIL;
    starts.iter().enumerate().map(|(n, &s)| (s, starts.get(n + 1).copied().unwrap_or(tail))).collect()
}

fn action_bytes(base: i32, slot: u32) -> Vec<u8> {
    let mut out = vec![0x21];
    out.extend(base.to_le_bytes());
    out.extend((slot as i32 + 1).to_le_bytes());
    out
}

struct Function {
    script: Script,
    list: i32,
    base: i32,
}

fn load(upk: &UpkPackage) -> Result<Function, String> {
    let find = |path: &str| upk.find_object(path).ok_or_else(|| format!("{path} was not found"));
    let export = find(FUNCTION)? as usize - 1;
    let entry = &upk.exports[export];
    let payload = &upk.image[entry.serial_offset..entry.serial_offset + entry.serial_size];
    let base = upk.names.iter().position(|n| n == ACTION_BASE).ok_or("The action name is missing")?;
    Ok(Function { script: Script::of_function(payload)?, list: find(LIST)?, base: base as i32 })
}

fn find_entry(function: &Function, slot: u32) -> Option<(usize, usize)> {
    let script = function.script.bytes();
    let wanted = action_bytes(function.base, slot);
    entries(script, function.list).into_iter().find(|&(start, _)| script.get(start + ACTION_AT..start + ACTION_AT + 9) == Some(&wanted[..]))
}

/// Whether `slot` has an entry.
pub fn has(upk: &UpkPackage, slot: u32) -> bool {
    load(upk).is_ok_and(|function| find_entry(&function, slot).is_some())
}

/// One entry: Context-free `DefaultActions.AddItem((S.Action = 'name', S.Category =
/// 'NotShown', S.Command = "command", S))`, the skip word counting the item in
/// memory form.
fn entry(list: i32, local: i32, strukt: i32, properties: [i32; 3], action: &[u8], category: i32, command: &str) -> Script {
    let member = |inner: &mut Script, property: i32| {
        inner.raw(&[0x0f, 0x35]).object(property).object(strukt).raw(&[0x00, 0x01, 0x2b]).object(local);
    };
    let mut inner = Script::new();
    inner.raw(&[0x54]);
    member(&mut inner, properties[0]);
    inner.raw(action);
    member(&mut inner, properties[1]);
    inner.raw(&[0x21]).name(category);
    member(&mut inner, properties[2]);
    inner.raw(&[0x1f]).raw(command.as_bytes()).raw(&[0x00]);
    inner.raw(&[0x16, 0x2b]).object(local).raw(&[0x16]);
    let mut out = Script::new();
    out.raw(&[0x57, 0x07, 0x2b]).object(list).raw(&(inner.mem_size() as u16).to_le_bytes()).append(&inner);
    out
}

/// Add an entry for `slot` that runs the console command `command`.
pub fn add(upk: &mut UpkPackage, slot: u32, command: &str) -> Result<(), String> {
    let mut function = load(upk)?;
    if find_entry(&function, slot).is_some() {
        return Ok(());
    }
    let script = function.script.bytes();
    let (start, _) = entries(script, function.list)
        .into_iter()
        .find(|&(start, end)| script[start..end].windows(TEMPLATE.len()).any(|w| w == TEMPLATE))
        .ok_or("The ChatPreset entry was not found")?;
    let category = upk.names.iter().position(|n| n == CATEGORY).ok_or("The NotShown category is missing")?;
    let new = entry(
        function.list,
        read(script, start + LOCAL_AT),
        read(script, start + STRUCT_AT),
        [read(script, start + ACTION_PROPERTY), read(script, start + CATEGORY_PROPERTY), read(script, start + COMMAND_PROPERTY)],
        &action_bytes(function.base, slot),
        category as i32,
        command,
    );
    function.script.insert_before_tail(TAIL, &new);
    replace_script(upk, FUNCTION, &function.script)
}

/// Remove the entry for `slot`, if there is one.
pub fn remove(upk: &mut UpkPackage, slot: u32) -> Result<(), String> {
    let mut function = load(upk)?;
    let Some((start, end)) = find_entry(&function, slot) else {
        return Ok(());
    };
    function.script.cut(start, end - start, REFS);
    replace_script(upk, FUNCTION, &function.script)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Rebuild ChatPreset1's own entry and compare it with the game's bytes,
    /// then add, find and remove an entry. Run with HEBNIX_UPK_TEST_SOURCE set
    /// to a vanilla TAGame.upk.
    #[test]
    #[ignore]
    fn real_package() {
        let source = std::env::var("HEBNIX_UPK_TEST_SOURCE").unwrap();
        let mut upk = UpkPackage::load(Path::new(&source)).unwrap();
        let function = load(&upk).unwrap();
        let script = function.script.bytes().to_vec();
        let (start, end) = entries(&script, function.list)
            .into_iter()
            .find(|&(s, e)| script[s..e].windows(TEMPLATE.len()).any(|w| w == TEMPLATE))
            .unwrap();
        let name = |n: &str| upk.names.iter().position(|x| x == n).unwrap() as i32;
        let mut chat = vec![0x21];
        chat.extend(name("ChatPreset1").to_le_bytes());
        chat.extend([0; 4]);
        let rebuilt = entry(
            function.list,
            read(&script, start + LOCAL_AT),
            read(&script, start + STRUCT_AT),
            [read(&script, start + ACTION_PROPERTY), read(&script, start + CATEGORY_PROPERTY), read(&script, start + COMMAND_PROPERTY)],
            &chat,
            name("Chat"),
            "ChatPreset 0",
        );
        assert_eq!(rebuilt.bytes(), &script[start..end]);
        assert_eq!(rebuilt.mem_size(), (end - start) + 4 * REFS);

        let original = upk.exports[upk.find_object(FUNCTION).unwrap() as usize - 1].serial_size;
        assert!(!has(&upk, 2));
        add(&mut upk, 2, "ShowAccountPicker").unwrap();
        add(&mut upk, 7, "ShowControllerApplet").unwrap();
        assert!(has(&upk, 2) && has(&upk, 7) && !has(&upk, 3));
        let out = std::env::temp_dir().join("hebnix_actions_test.upk");
        upk.save_fast(&out).unwrap();
        let mut reloaded = UpkPackage::load(&out).unwrap();
        let _ = std::fs::remove_file(out);
        assert!(has(&reloaded, 2) && has(&reloaded, 7));
        remove(&mut reloaded, 2).unwrap();
        remove(&mut reloaded, 7).unwrap();
        assert!(!has(&reloaded, 2) && !has(&reloaded, 7));
        let after = &reloaded.exports[reloaded.find_object(FUNCTION).unwrap() as usize - 1];
        assert_eq!(after.serial_size, original);
    }
}


