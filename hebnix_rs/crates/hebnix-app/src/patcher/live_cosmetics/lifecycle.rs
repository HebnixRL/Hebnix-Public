use super::*;
use super::super::title_patcher;
use super::super::upk_package::UpkPackage;
use sha2::{Digest, Sha256};
use std::sync::{atomic::{AtomicBool, Ordering}, OnceLock};

static COOKED: OnceLock<Mutex<PathBuf>> = OnceLock::new();
static BUSY: AtomicBool = AtomicBool::new(false);
#[cfg(windows)]
#[path = "operation_recovery.rs"]
mod operation_recovery;
pub fn managed() -> bool { COOKED.get().is_some() }
pub(super) fn runtime_report_path() -> Option<PathBuf> {
    Some(directory().join("runtime.ini"))
}
#[derive(Serialize, Deserialize)]
struct Function { path: String, before: Vec<u8>, after: Vec<u8>, #[serde(default)] older: Vec<Vec<u8>> }
#[derive(Clone, Serialize, Deserialize)]
struct Attachment { file: String, path: String, before: String, after: String, #[serde(default)] older: Vec<String>, #[serde(default)] before_hash: String, #[serde(default)] after_hash: String }
#[derive(Serialize, Deserialize)]
struct Receipt {
    version: u32,
    cooked: PathBuf,
    functions: Vec<Function>,
    before_hash: String,
    after_hash: String,
    names: Vec<(usize, String, String)>,
    imports: Vec<(usize, Vec<u8>, Vec<u8>)>,
    #[serde(default)]
    runtime_revision: u32,
    #[serde(default)]
    attachments: Vec<Attachment>,
}
#[cfg(test)]
thread_local! { static TEST_ROOT: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) }; }
fn directory() -> PathBuf {
    #[cfg(test)]
    if let Some(root) = TEST_ROOT.with(|r| r.borrow().clone()) { return root.join("state"); }
    crate::config::base_dir().join("state/live-cosmetics")
}
fn receipt_path() -> PathBuf { directory().join("transaction.json") }
fn hash(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
fn blob(bytes: &[u8]) -> Result<String, String> {
    let digest = hash(bytes);
    let path = directory().join(format!("payload-{digest}.bin"));
    if !path.is_file() { super::super::upk_history::replace(&path, bytes)?; }
    Ok(digest)
}
fn read_blob(digest: &str) -> Result<Vec<u8>, String> {
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) { return Err("Invalid live movie journal reference".into()); }
    let bytes = fs::read(directory().join(format!("payload-{digest}.bin"))).map_err(|e| e.to_string())?;
    if hash(&bytes) != digest { return Err("Live movie recovery payload failed verification".into()); }
    Ok(bytes)
}
fn prepare_movies(cooked: &Path, old: &[Attachment]) -> Result<Vec<Attachment>, String> {
    let mut attachments = Vec::new();
    for (file, _) in super::movies::FILES {
        let previous = old.iter().find(|a| a.file == *file);
        let baseline = previous.map(|a| read_blob(&a.before)).transpose()?;
        let (path, before, after) = super::movies::prepare(cooked, file, baseline.as_deref())?;
        let before = blob(&before)?; let after = blob(&after)?;
        let mut older = previous.map(|a| a.older.clone()).unwrap_or_default();
        if let Some(previous) = previous.filter(|a| a.after != after) { older.push(previous.after.clone()); }
        let unchanged = previous.filter(|a| a.before == before && a.after == after);
        attachments.push(Attachment { file: file.to_string(), path, before, after, older,
            before_hash: unchanged.map(|a| a.before_hash.clone()).unwrap_or_default(),
            after_hash: unchanged.map(|a| a.after_hash.clone()).unwrap_or_default() });
    }
    Ok(attachments)
}
fn payload<'a>(p: &'a UpkPackage, path: &str) -> Result<(usize, &'a [u8]), String> {
    let i = p.find_object(path).filter(|i| *i > 0).ok_or_else(|| format!("Missing live cosmetic function: {path}"))? as usize - 1;
    let e = &p.exports[i];
    Ok((i, &p.image[e.serial_offset..e.serial_offset + e.serial_size]))
}
fn lock(takeover: bool) -> Result<fs::File, String> {
    fs::create_dir_all(directory()).map_err(|e| e.to_string())?;
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let path = directory().join("operation.lock");
    let mut opened = options.open(&path);
    #[cfg(windows)]
    if opened.as_ref().is_err_and(|e| matches!(e.raw_os_error(), Some(32 | 33))) {
        status("Recovering the previous live cosmetic operation…");
        if takeover { operation_recovery::release_previous_worker(&path)?; }
        for _ in 0..40 {
            if opened.is_ok() { break; }
            std::thread::sleep(std::time::Duration::from_millis(100));
            opened = options.open(&path);
        }
    }
    let file = opened.map_err(|e| format!("Could not recover the previous live cosmetic operation: {e}"))?;
    // Unix opens are shareable: hold an OS lock until this File is dropped.
    // A crashed process releases the lock; a stale lock file remains harmless.
    #[cfg(unix)]
    file.try_lock().map_err(|e| format!("Could not recover the previous live cosmetic operation: {e}"))?;
    Ok(file)
}
fn allowed(owner: u32) -> Result<(), String> {
    #[cfg(test)]
    if TEST_ROOT.with(|r| r.borrow().is_some()) { return Ok(()); }
    if hebnix_sdk::process::is_rocket_league_running() { return Err("Waiting for Rocket League to close".into()); }
    if crate::watchdog::replacement_is_running(owner) { return Err("A newer Hebnix session owns live cosmetics".into()); }
    Ok(())
}
fn receipt() -> Result<Option<Receipt>, String> {
    match fs::read(receipt_path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| format!("Live cosmetic recovery journal could not be read: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
fn save(r: &Receipt) -> Result<(), String> {
    super::super::upk_history::replace(&receipt_path(), &serde_json::to_vec(r).map_err(|e| e.to_string())?)
}

pub fn start(root: &str) {
    let cooked = Path::new(root).join("TAGame/CookedPCConsole");
    if !cooked.is_dir() {
        status("Error: select a valid Rocket League installation in Settings.");
        return;
    }
    *COOKED.get_or_init(|| Mutex::new(cooked.clone())).lock().unwrap_or_else(|e| e.into_inner()) = cooked.clone();
    let mut settings = values();
    let backups = cooked.join("Backups");
    for (id, marker) in [("forfeit", "ForfeitCommand/state.json"), ("celebration", "Celebration/state.json"),
        ("bots-freeplay", "BotsFreeplay/state.json"), ("chat-export", "ChatExport/state.json"),
        ("lan-commands", "LanCommands/state.json"), ("chat-paste", "ChatPaste/vanilla.swf"), ("instant-suite", "InstantSuite.enabled")] {
        settings.game_mods.entry(id.into()).or_insert_with(|| backups.join(marker).is_file());
    }
    let cfg = crate::config::Config::load(&crate::config::base_dir());
    super::apply_game_options(&mut settings, &cfg.game_mods);
    if let Err(e) = update(&cooked, settings) { status(format!("Error: {e}")); return; }
    if hebnix_sdk::process::is_rocket_league_running() {
        status("Checking installed live features…");
        std::thread::spawn(move || {
            let installed = (|| -> Result<bool, String> {
                let Some(r) = receipt()? else { return Ok(false); };
                if r.cooked != cooked || r.runtime_revision != runtime::REVISION { return Ok(false); }
                let bytes = fs::read(cooked.join("TAGame.upk")).map_err(|e| e.to_string())?;
                if hash(&bytes) != r.after_hash {
                    // Other Hebnix features may alter the same package. Verify
                    // our owned functions rather than declaring those absent.
                    let package = UpkPackage::from_bytes(bytes)?;
                    for function in &r.functions {
                        if payload(&package, &function.path)?.1 != function.after { return Ok(false); }
                    }
                    for (index, _, installed) in &r.names {
                        if package.names.get(*index) != Some(installed) { return Ok(false); }
                    }
                }
                for attachment in &r.attachments {
                    let path = super::super::game_path::resolve(&cooked, &attachment.file);
                    let bytes = fs::read(path).map_err(|e| e.to_string())?;
                    if hash(&bytes) != attachment.after_hash {
                        let package = UpkPackage::from_bytes(bytes)?;
                        if payload(&package, &attachment.path)?.1 != read_blob(&attachment.after)? { return Ok(false); }
                    }
                }
                Ok(true)
            })().unwrap_or(false);
            status(if installed { "Live features ready" } else { "Close Rocket League to prepare live features" });
            // Wait for a safe repair opportunity if hooks are absent or outdated.
            if !installed {
                while hebnix_sdk::process::is_rocket_league_running() { std::thread::sleep(std::time::Duration::from_millis(500)); }
                reconcile_async();
            }
        });
    } else { reconcile_async(); }
}
pub fn reconcile_async() {
    if hebnix_sdk::process::is_rocket_league_running() {
        status("Live setup will be checked when Rocket League closes");
        return;
    }
    let Some(cooked) = COOKED.get().and_then(|p| p.lock().ok().map(|p| p.clone())) else { return; };
    if BUSY.swap(true, Ordering::AcqRel) { return; }
    std::thread::spawn(move || {
        status("Preparing live features…");
        let mut attempt = 0;
        loop {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reconcile(&cooked, true, std::process::id())))
                .unwrap_or_else(|_| Err("Live recovery stopped unexpectedly; the saved journal was retained".into()));
            match outcome {
                Ok(()) => { status("Live features ready"); break; }
                Err(error) => {
                    if hebnix_sdk::process::is_rocket_league_running() {
                        status("Error: Rocket League is using the game files. Live patching will resume after Rocket League closes.");
                        while hebnix_sdk::process::is_rocket_league_running()
                            && !crate::watchdog::replacement_is_running(std::process::id()) {
                            std::thread::sleep(std::time::Duration::from_millis(500));
                        }
                        if crate::watchdog::replacement_is_running(std::process::id()) { break; }
                        attempt = 0;
                        continue;
                    }
                    tracing::warn!(attempt, "Live cosmetics: {error}");
                    status(format!("Error: {error}"));
                    if attempt == 2 || hebnix_sdk::process::is_rocket_league_running()
                        || crate::watchdog::replacement_is_running(std::process::id()) { break; }
                    // Re-read the durable journal and actual package contents on
                    // every attempt. Never restart from assumed vanilla files.
                    std::thread::sleep(std::time::Duration::from_secs(1 + attempt));
                    attempt += 1;
                }
            }
        }
        BUSY.store(false, Ordering::Release);
    });
}
pub fn cleanup_after_exit(owner: u32) {
    while hebnix_sdk::process::is_rocket_league_running() {
        if crate::watchdog::replacement_is_running(owner) { return; }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    for attempt in 0..20 {
        if crate::watchdog::replacement_is_running(owner) { return; }
        let result = (|| {
            let Some(r) = receipt()? else { return Ok(()); };
            reconcile(&r.cooked, false, owner)
        })();
        match result {
            Ok(()) => return,
            Err(e) => {
                tracing::warn!(attempt, "Live cosmetic cleanup: {e}");
                if attempt == 19 { return; }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
    }
}

fn reconcile(cooked: &Path, install: bool, owner: u32) -> Result<(), String> {
    #[cfg(test)]
    if let Some(root) = TEST_ROOT.with(|r| r.borrow().clone()) {
        assert!(cooked.canonicalize().unwrap().starts_with(root.canonicalize().unwrap()), "test writes must stay inside its isolated copy");
    }
    allowed(owner)?;
    let _process_lock = lock(install && crate::watchdog::parent_pid().is_none())?;
    let _movie_lock = super::super::title_patcher::UI_PATCH_LOCK.lock().map_err(|e| e.to_string())?;
    let _package_lock = super::super::profile_pictures::PACKAGE_EDIT.lock().map_err(|e| e.to_string())?;
    allowed(owner)?;
    if install { runtime::verify_dependencies(cooked)?; }
    // Retire the old literal-scale patch before this function enters the live journal.
    // This is idempotent: an interrupted migration resumes from its existing backup.
    if install && super::super::goal_explosion::current(&cooked.join("Backups")).is_some() {
        super::super::goal_explosion::restore(cooked, &cooked.join("Backups"))?;
        allowed(owner)?;
    }
    let live = cooked.join("TAGame.upk");
    let raw = fs::read(&live).map_err(|e| e.to_string())?;
    let current_hash = hash(&raw);
    let mut saved = receipt()?;
    if saved.as_ref().is_some_and(|r| r.version != 1 || r.cooked != cooked) {
        return Err("Live cosmetic recovery belongs to another game installation. Restore it before changing installations".into());
    }
    let upgrade = install && saved.as_ref().is_some_and(|r| r.runtime_revision != runtime::REVISION);
    if let Some(r) = saved.as_mut().filter(|_| !upgrade) {
        if &current_hash == if install { &r.after_hash } else { &r.before_hash } { return reconcile_movies(r, install, owner); }
        let (expected, variant) = if install { (&r.before_hash, "enabled.upk") } else { (&r.after_hash, "disabled.upk") };
        // The fast path is valid only for this exact package, including other patches.
        if &current_hash == expected {
            if let Ok(bytes) = fs::read(directory().join(variant)) {
                let wanted = if install { &r.after_hash } else { &r.before_hash };
                if &hash(&bytes) == wanted {
                    allowed(owner)?;
                    super::super::upk_history::replace(&live, &bytes)?;
                    return reconcile_movies(r, install, owner);
                }
            }
        }
    } else if !install { return Ok(()); }
    let mut package = UpkPackage::from_bytes(raw)?;
    let previous_payloads: std::collections::HashMap<_, _> = runtime::TARGETS.iter().copied()
        .chain(super::game_mods::TARGETS.iter().copied())
        .chain(["GFxData_PRI_TA.UpdatePlayerName", "GFxData_PRI_TA.UpdatePRIData", "GFxData_PRI_TA.SetPlayerTitle"])
        .map(|path| Ok((path.to_owned(), payload(&package, path)?.1.to_vec())))
        .collect::<Result<_, String>>()?;
    if saved.is_none() {
        // Never bake another program's live-control payload into our rollback.
        for path in runtime::TARGETS {
            let (_, bytes) = payload(&package, path)?;
            if bytes.windows(3).any(|w| w == b"S.C") || bytes.windows(16).any(|w| w == b"127.0.0.1:49320/") {
                return Err("Restore Shift's live patches before installing Hebnix live cosmetics".into());
            }
        }
        let mut names = runtime::prepare_names(&mut package)?;
        let imports = runtime::prepare_imports(&mut package)?;
        let (prepared, scoreboard, scoreboard_names) = super::super::profile_pictures::scoreboard::prepare_live(package)?;
        package = prepared;
        names.extend(scoreboard_names);
        let mods = super::game_mods::prepare(&mut package, &cooked.join("Backups"))?;
        let mut functions = runtime::generate_with_paths(&package, &cooked.parent().ok_or("Missing TAGame directory")?.join("HbxLiveCosmetics.ini"), &directory().join("runtime.ini"))?.into_iter().map(|(path, after)| {
            let before = payload(&package, &path)?.1.to_vec();
            Ok(Function { path, before, after, older: Vec::new() })
        }).collect::<Result<Vec<_>, String>>()?;
        functions.extend(scoreboard.into_iter().map(|(path, before, after)| Function { path, before, after, older: vec![] }));
        functions.extend(mods.into_iter().map(|(path, before, after)| Function { path, before, after, older: vec![] }));
        for f in &mut functions {
            if let Some(previous) = previous_payloads.get(&f.path).filter(|old| **old != f.before) { f.older.push(previous.clone()); }
        }
        let attachments = prepare_movies(cooked, &[])?;
        saved = Some(Receipt { version: 1, cooked: cooked.to_path_buf(), functions, names, imports, attachments, runtime_revision: runtime::REVISION, before_hash: String::new(), after_hash: String::new() });
    }
    let mut r = saved.unwrap();
    let mut changed = false;
    if upgrade {
        for f in &r.functions {
            let (index, current) = payload(&package, &f.path)?;
            if current != f.before && current != f.after && !f.older.iter().any(|old| old == current) {
                return Err(format!("{} changed outside Hebnix. Live hook upgrade left the package untouched", f.path));
            }
            package.replace_export_payload(index, &f.before)?;
        }
    }
    if install {
        let mut names = Vec::new();
        for (index, before, after) in &r.names {
            match package.names.get(*index) {
                Some(current) if current == after => {},
                Some(current) if current == before => names.push((*index, after.clone())),
                _ => return Err("Live cosmetic name slots changed outside Hebnix. Recovery retained the journal".into()),
            }
        }
        if !names.is_empty() { package.rename_names_same_size(&names)?; changed = true; }
        for (slot, before, after) in &r.imports {
            let current = package.import_bytes(*slot)?;
            if current == *after { continue; }
            if current != *before { return Err("Native live cosmetic imports changed outside Hebnix".into()); }
            package.restore_import_bytes(*slot, after)?;
            changed = true;
        }
    }
    if upgrade {
        r.names.extend(runtime::prepare_names(&mut package)?);
        r.imports.extend(runtime::prepare_imports(&mut package)?);
        let (prepared, scoreboard, scoreboard_names) = super::super::profile_pictures::scoreboard::prepare_live(package)?;
        package = prepared;
        r.names.extend(scoreboard_names);
        let mods = super::game_mods::prepare(&mut package, &cooked.join("Backups"))?;
        let mut generated = runtime::generate_with_paths(&package, &cooked.parent().ok_or("Missing TAGame directory")?.join("HbxLiveCosmetics.ini"), &directory().join("runtime.ini"))?.into_iter().map(|(path, after)| {
            Ok((path.clone(), payload(&package, &path)?.1.to_vec(), after))
        }).collect::<Result<Vec<_>, String>>()?;
        generated.extend(scoreboard);
        generated.extend(mods);
        for (path, before, after) in generated {
            if let Some(f) = r.functions.iter_mut().find(|f| f.path == path) {
                if f.after != after { f.older.push(std::mem::replace(&mut f.after, after)); }
            } else {
                let older = previous_payloads.get(&path).filter(|old| **old != before).cloned().into_iter().collect();
                r.functions.push(Function { before, path, after, older });
            }
        }
        r.runtime_revision = runtime::REVISION;
        r.attachments = prepare_movies(cooked, &r.attachments)?;
        changed = true;
    }
    for f in &r.functions {
        let (index, current) = payload(&package, &f.path)?;
        let wanted = if install { &f.after } else { &f.before };
        if current == wanted { continue; }
        let other = if install { &f.before } else { &f.after };
        if current != other && !f.older.iter().any(|old| old == current) {
            return Err(format!("{} changed outside Hebnix's live cosmetics. Recovery retained the journal and left the package untouched", f.path));
        }
        package.replace_export_payload(index, wanted)?;
        changed = true;
    }
    if !changed { return reconcile_movies(&mut r, install, owner); }
    let bytes = package.repacked_fast()?;
    let staged = directory().join("staged.upk");
    super::super::upk_history::replace(&staged, &bytes)?;
    let check = UpkPackage::load(&staged)?;
    for f in &r.functions {
        if payload(&check, &f.path)?.1 != if install { &f.after[..] } else { &f.before[..] } { return Err("Staged live cosmetic verification failed".into()); }
    }
    drop(check); // Do not retain a second decompressed package while packing the restore variant.
    for f in &r.functions {
        let (index, _) = payload(&package, &f.path)?;
        package.replace_export_payload(index, if install { &f.before } else { &f.after })?;
    }
    let opposite = package.repacked_fast()?;
    let (disabled, enabled) = if install { (&opposite, &bytes) } else { (&bytes, &opposite) };
    r.before_hash = hash(disabled);
    r.after_hash = hash(enabled);
    super::super::upk_history::replace(&directory().join("disabled.upk"), disabled)?;
    super::super::upk_history::replace(&directory().join("enabled.upk"), enabled)?;
    save(&r)?; // Durable ownership before the only write to the live package.
    allowed(owner)?;
    super::super::upk_history::replace(&live, &bytes)?;
    let _ = fs::remove_file(staged);
    reconcile_movies(&mut r, install, owner)
}

fn reconcile_movies(r: &mut Receipt, install: bool, owner: u32) -> Result<(), String> {
    for index in 0..r.attachments.len() {
        allowed(owner)?;
        let mut a = r.attachments[index].clone();
        if !super::movies::FILES.iter().any(|(file, path)| *file == a.file && *path == a.path) { return Err("Unknown live movie recovery target".into()); }
        let live = super::super::game_path::resolve(&r.cooked, &a.file);
        let raw = fs::read(&live).map_err(|e| e.to_string())?;
        let digest = hash(&raw);
        let wanted_hash = if install { &a.after_hash } else { &a.before_hash };
        if &digest == wanted_hash { continue; }
        let other_hash = if install { &a.before_hash } else { &a.after_hash };
        let cache = |on| directory().join(format!("{}-{}.upk", a.file, if on { "enabled" } else { "disabled" }));
        if &digest == other_hash {
            if let Ok(bytes) = fs::read(cache(install)) {
                if &hash(&bytes) == wanted_hash {
                    allowed(owner)?;
                    super::super::upk_history::replace(&live, &bytes)?;
                    continue;
                }
            }
        }
        // Titles sit on top of the HUD. Edit the movie underneath them and put them back.
        let _ui_patch = (a.file == title_patcher::BOOST_HUD)
            .then(|| title_patcher::UI_PATCH_LOCK.lock()).transpose().map_err(|_| "UI patch lock unavailable")?;
        let mut p = UpkPackage::from_bytes(raw.clone())?;
        let slot = payload(&p, &a.path)?.0;
        let titles = if a.file == title_patcher::BOOST_HUD { title_patcher::hud_titles(&r.cooked, &p, slot)? } else { None };
        if let Some(titles) = &titles { p.replace_export_payload(slot, &titles.original)?; }
        let (slot, current) = payload(&p, &a.path)?;
        let current_hash = hash(current);
        let wanted = if install { &a.after } else { &a.before };
        if &current_hash == wanted { continue; }
        if current_hash != a.before && current_hash != a.after && !a.older.contains(&current_hash) {
            let before = read_blob(&a.before)?;
            let after = read_blob(&a.after)?;
            let (rebased_before, rebased_after) = super::movies::rebase_owned(&mut p, &a.file, &before, &after)
                .map_err(|error| format!("{} has conflicting movie edits: {error}. Recovery kept them untouched", a.file))?;
            let mut known = vec![a.before.clone(), a.after.clone(), current_hash.clone()];
            known.retain(|value| !a.older.contains(value));
            a.older.extend(known);
            a.before = blob(&rebased_before)?;
            a.after = blob(&rebased_after)?;
            a.before_hash.clear(); a.after_hash.clear();
            r.attachments[index] = a.clone();
            // Commit provenance before touching the game. A failed/stopped
            // recovery can recognise the old export and resume this exact pair.
            save(r)?;
        }
        let wanted = if install { &a.after } else { &a.before };
        let desired = read_blob(wanted)?;
        p.replace_export_payload(slot, &desired)?;
        let staged = match &titles {
            Some(titles) => {
                let titled = title_patcher::retitle_hud(&p, slot, &titles.rules)?;
                p.replace_export_payload(slot, &titled)?;
                titled
            }
            None => desired.clone(),
        };
        let bytes = p.repacked_fast()?;
        let check = UpkPackage::from_bytes(bytes.clone())?;
        if payload(&check, &a.path)?.1 != staged { return Err("Live movie staging verification failed".into()); }
        drop(check);
        if titles.is_some() {
            // Cached whole-package variants would skip Titles' restore record.
            r.attachments[index].before_hash.clear();
            r.attachments[index].after_hash.clear();
        } else {
            p.replace_export_payload(slot, &read_blob(if install { &a.before } else { &a.after })?)?;
            let opposite = p.repacked_fast()?;
            let (disabled, enabled) = if install { (&opposite, &bytes) } else { (&bytes, &opposite) };
            r.attachments[index].before_hash = hash(disabled);
            r.attachments[index].after_hash = hash(enabled);
            super::super::upk_history::replace(&cache(false), disabled)?;
            super::super::upk_history::replace(&cache(true), enabled)?;
        }
        save(r)?;
        allowed(owner)?;
        if fs::read(&live).map_err(|e| e.to_string())? != raw { return Err("A live movie package changed during staging".into()); }
        super::super::upk_history::replace(&live, &bytes)?;
        if titles.is_some() { title_patcher::sync_hud_titles(&r.cooked, slot, &desired, &staged)?; }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn verify(cooked: &Path, on: bool) {
        let r = receipt().unwrap().unwrap();
        let p = UpkPackage::load(&cooked.join("TAGame.upk")).unwrap();
        for f in &r.functions { assert_eq!(payload(&p, &f.path).unwrap().1, if on { &f.after[..] } else { &f.before[..] }, "{}", f.path); }
        drop(p);
        for a in &r.attachments {
            let p = UpkPackage::load(&cooked.join(&a.file)).unwrap();
            assert_eq!(payload(&p, &a.path).unwrap().1, read_blob(if on { &a.after } else { &a.before }).unwrap());
        }
    }
    #[test]
    #[ignore = "full install/recovery/upgrade/removal test on copies of explicitly supplied game packages"]
    fn copied_packages_recover_and_upgrade() {
        let source = PathBuf::from(std::env::var("HEBNIX_UPK_TEST_SOURCE").unwrap());
        let parent = source.parent().unwrap();
        let root = std::env::temp_dir().join(format!("hebnix-live-integration-{}", rand::random::<u64>()));
        let cooked = root.join("TAGame/CookedPCConsole");
        fs::create_dir_all(&cooked).unwrap();
        fs::copy(&source, cooked.join("TAGame.upk")).unwrap();
        for file in ["Core.upk", "Engine.upk", "ProjectX.upk"] { fs::copy(parent.join(file), cooked.join(file)).unwrap(); }
        for (file, _) in super::super::movies::FILES { fs::copy(parent.join(file), cooked.join(file)).unwrap(); }
        TEST_ROOT.with(|r| *r.borrow_mut() = Some(root.clone()));
        println!("Installing all live hooks into isolated copies");
        reconcile(&cooked, true, std::process::id()).unwrap();
        verify(&cooked, true);
        let mut r = receipt().unwrap().unwrap();
        let target = r.functions.iter_mut().find(|f| f.path == "CarMeshComponentBase_TA.SetMaterialColors").unwrap();
        let offset = target.after.windows(2).position(|w| w == [0x38, 0x4c]).unwrap();
        target.after[offset + 1] = 0x4a; // Simulate the previously shipped integer reader.
        let obsolete = target.after.clone();
        let mut p = UpkPackage::load(&cooked.join("TAGame.upk")).unwrap();
        let index = payload(&p, &target.path).unwrap().0;
        p.replace_export_payload(index, &obsolete).unwrap();
        fs::write(cooked.join("TAGame.upk"), p.repacked_fast().unwrap()).unwrap();
        drop(p);
        r.runtime_revision = runtime::REVISION - 1;
        save(&r).unwrap();
        println!("Upgrading obsolete hooks");
        reconcile(&cooked, true, std::process::id()).unwrap();
        let r = receipt().unwrap().unwrap();
        let target = r.functions.iter().find(|f| f.path == "CarMeshComponentBase_TA.SetMaterialColors").unwrap();
        assert_ne!(target.after, obsolete);
        assert!(target.older.contains(&obsolete));
        verify(&cooked, true);
        println!("Simulating interrupted removal across packages");
        let mut p = UpkPackage::load(&cooked.join("TAGame.upk")).unwrap();
        let index = payload(&p, &r.functions[0].path).unwrap().0;
        p.replace_export_payload(index, &r.functions[0].before).unwrap();
        fs::write(cooked.join("TAGame.upk"), p.repacked_fast().unwrap()).unwrap();
        drop(p);
        let a = &r.attachments[0];
        let mut p = UpkPackage::load(&cooked.join(&a.file)).unwrap();
        let index = payload(&p, &a.path).unwrap().0;
        p.replace_export_payload(index, &read_blob(&a.before).unwrap()).unwrap();
        fs::write(cooked.join(&a.file), p.repacked_fast().unwrap()).unwrap();
        drop(p);
        println!("Reopening Hebnix during a partially completed restoration");
        reconcile(&cooked, true, std::process::id()).unwrap();
        verify(&cooked, true);
        // Resume cleanup again; neither path may discard the original baseline.
        reconcile(&cooked, false, std::process::id()).unwrap();
        verify(&cooked, false);
        println!("Checking cached reinstall and cleanup");
        reconcile(&cooked, true, std::process::id()).unwrap();
        verify(&cooked, true);
        reconcile(&cooked, false, std::process::id()).unwrap();
        verify(&cooked, false);
        TEST_ROOT.with(|r| *r.borrow_mut() = None);
        let temp = std::env::temp_dir().canonicalize().unwrap();
        assert!(root.canonicalize().unwrap().starts_with(temp));
        fs::remove_dir_all(root).unwrap();
    }
}
