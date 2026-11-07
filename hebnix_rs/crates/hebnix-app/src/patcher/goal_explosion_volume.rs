//! Goal explosion volume, per Wwise bank (SFX_GoalExplosion_*.bnk). Every Play
//! target of each bank gets `gain` dB added to its Volume. Volume is a bundle
//! entry (pID 0) on the object: when it exists its float changes, when it is
//! missing the entry is inserted and the object and HIRC sizes grow by 5 bytes.
//! `goal_explosion_volume.json` holds the offsets and checksums for one game
//! build; a build that does not match is refused.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

const MANIFEST: &str = include_str!("goal_explosion_volume.json");
const DIR: &str = "GoalExplosionVolume";
const STATE: &str = "state.json";
const VOLUME: u8 = 0;
const INSERT_LEN: u32 = 5;

pub const MIN_DB: f32 = -96.0;
pub const MAX_DB: f32 = 6.0;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    banks: Vec<Bank>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Bank {
    bank: String,
    sha256: String,
    hirc_len_pos: usize,
    hirc_len: u32,
    targets: Vec<Target>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Target {
    c_props: usize,
    obj_size_pos: usize,
    obj_size: u32,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    gain: f32,
}

fn manifest() -> Result<Manifest, String> {
    serde_json::from_str(MANIFEST).map_err(|e| format!("goal_explosion_volume.json: {e}"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn mismatch(bank: &Bank) -> String {
    format!("{} does not match this Rocket League build. Verify game files.", bank.bank)
}

/// Patch one bank's original bytes. No checksum check, so tests can use small buffers.
fn patch_bytes(original: &[u8], bank: &Bank, gain: f32) -> Result<Vec<u8>, String> {
    let bad = || mismatch(bank);
    let mut edits: Vec<(usize, usize, Vec<u8>)> = Vec::new();
    let mut hirc_delta = 0;
    for t in &bank.targets {
        let c = t.c_props;
        let n = *original.get(c).ok_or_else(bad)? as usize;
        let ids = original.get(c + 1..c + 1 + n).ok_or_else(bad)?;
        let values = c + 1 + n;
        if let Some(k) = ids.iter().position(|&id| id == VOLUME) {
            let pos = values + 4 * k;
            let bytes: [u8; 4] = original.get(pos..pos + 4).ok_or_else(bad)?.try_into().map_err(|_| bad())?;
            let raised = f32::from_le_bytes(bytes) + gain;
            edits.push((pos, 4, raised.to_le_bytes().to_vec()));
        } else {
            let body = original.get(values..values + 4 * n).ok_or_else(bad)?;
            let mut buf = Vec::with_capacity(1 + 5 * n + 5);
            buf.push((n + 1) as u8);
            buf.push(VOLUME);
            buf.extend_from_slice(ids);
            buf.extend_from_slice(&gain.to_le_bytes());
            buf.extend_from_slice(body);
            edits.push((c, 1 + 5 * n, buf));
            edits.push((t.obj_size_pos, 4, (t.obj_size + INSERT_LEN).to_le_bytes().to_vec()));
            hirc_delta += INSERT_LEN;
        }
    }
    if hirc_delta > 0 {
        let len = bank.hirc_len + hirc_delta;
        edits.push((bank.hirc_len_pos, 4, len.to_le_bytes().to_vec()));
    }
    edits.sort_by_key(|e| e.0);
    let mut out = Vec::with_capacity(original.len() + 64);
    let mut last = 0;
    for (pos, old_len, buf) in edits {
        if pos < last || pos + old_len > original.len() {
            return Err(format!("{}: patch position out of range", bank.bank));
        }
        out.extend_from_slice(&original[last..pos]);
        out.extend_from_slice(&buf);
        last = pos + old_len;
    }
    out.extend_from_slice(&original[last..]);
    Ok(out)
}

fn patch(original: &[u8], bank: &Bank, gain: f32) -> Result<Vec<u8>, String> {
    if sha256_hex(original) != bank.sha256 {
        return Err(mismatch(bank));
    }
    patch_bytes(original, bank, gain)
}

fn ensure_closed() -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        Err("Close Rocket League before changing the goal explosion volume.".into())
    } else {
        Ok(())
    }
}

fn write_live(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temp = path.with_extension("bnk.hebnix_tmp");
    std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
    let copied = std::fs::copy(&temp, path);
    let _ = std::fs::remove_file(&temp);
    copied.map(|_| ()).map_err(|e| format!("Could not write {}: {e}", path.display()))
}

/// Set every goal explosion bank's Play targets to `gain` dB, from the backed-up originals.
pub fn apply(cooked: &Path, backups: &Path, gain: f32) -> Result<(), String> {
    if !(MIN_DB..=MAX_DB).contains(&gain) {
        return Err(format!("Goal explosion volume must be between {MIN_DB} and {MAX_DB} dB"));
    }
    ensure_closed()?;
    let manifest = manifest()?;
    let dir = backups.join(DIR);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for bank in &manifest.banks {
        let original = dir.join(&bank.bank);
        if original.exists() {
            continue;
        }
        let live = std::fs::read(cooked.join(&bank.bank)).map_err(|e| e.to_string())?;
        if sha256_hex(&live) != bank.sha256 {
            return Err(mismatch(bank));
        }
        std::fs::write(&original, &live).map_err(|e| e.to_string())?;
    }
    for bank in &manifest.banks {
        let original = std::fs::read(dir.join(&bank.bank)).map_err(|e| e.to_string())?;
        let patched = patch(&original, bank, gain)?;
        write_live(&cooked.join(&bank.bank), &patched)?;
    }
    let saved = serde_json::to_vec(&Saved { gain }).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(STATE), saved).map_err(|e| e.to_string())
}

/// Put every goal explosion bank back to its original bytes.
pub fn restore(cooked: &Path, backups: &Path) -> Result<(), String> {
    let dir = backups.join(DIR);
    if !dir.exists() {
        return Ok(());
    }
    ensure_closed()?;
    let manifest = manifest()?;
    for bank in &manifest.banks {
        let original = dir.join(&bank.bank);
        if original.exists() {
            let bytes = std::fs::read(&original).map_err(|e| e.to_string())?;
            write_live(&cooked.join(&bank.bank), &bytes)?;
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// The gain the patch is set to, if it is on.
pub fn current(backups: &Path) -> Option<f32> {
    let bytes = std::fs::read(backups.join(DIR).join(STATE)).ok()?;
    serde_json::from_slice::<Saved>(&bytes).ok().map(|saved| saved.gain)
}

/// Page state: the slider and the job writing the banks.
pub struct State {
    gain: f32,
    loaded: bool,
    job: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    status: String,
}

impl Default for State {
    fn default() -> Self {
        Self { gain: 0.0, loaded: false, job: None, status: String::new() }
    }
}

impl State {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    pub fn render(&mut self, ui: &mut eframe::egui::Ui, cooked: &Path, backups: &Path) {
        use crate::i18n::{t, t_args};
        use eframe::egui;
        if !self.loaded {
            self.loaded = true;
            if let Some(gain) = current(backups) {
                self.gain = gain;
            }
        }
        if let Some(job) = &self.job {
            match job.try_recv() {
                Ok(result) => {
                    self.status = result.unwrap_or_else(|e| e);
                    self.job = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.job = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint(),
            }
        }
        ui.heading(t("goal-explosion-volume-title"));
        ui.label(t("goal-explosion-volume-description"));
        ui.add_space(8.0);
        ui.add_enabled(
            !self.busy(),
            egui::Slider::new(&mut self.gain, MIN_DB..=MAX_DB).suffix(" dB"),
        );
        if self.gain <= MIN_DB {
            ui.label(t("goal-explosion-volume-mute"));
        }
        ui.horizontal(|ui| {
            let apply_clicked = ui
                .add_enabled(!self.busy(), egui::Button::new(t("goal-explosion-apply")))
                .clicked();
            let restore_clicked = ui
                .add_enabled(
                    !self.busy() && current(backups).is_some(),
                    egui::Button::new(t("goal-explosion-restore")),
                )
                .clicked();
            if apply_clicked || restore_clicked {
                let (cooked, backups, gain) = (cooked.to_path_buf(), backups.to_path_buf(), self.gain);
                let (tx, rx) = std::sync::mpsc::channel();
                self.job = Some(rx);
                self.status.clear();
                std::thread::spawn(move || {
                    let result = if apply_clicked {
                        apply(&cooked, &backups, gain).map(|_| {
                            t_args("goal-explosion-volume-applied", &[("db", format!("{gain:.0}").into())])
                        })
                    } else {
                        restore(&cooked, &backups).map(|_| t("goal-explosion-volume-restored"))
                    };
                    let _ = tx.send(result);
                });
            }
        });
        if self.busy() {
            ui.spinner();
        } else if !self.status.is_empty() {
            ui.label(&self.status);
        } else if let Some(gain) = current(backups) {
            ui.label(t_args("goal-explosion-volume-active", &[("db", format!("{gain:.0}").into())]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bank(targets: Vec<Target>) -> Bank {
        Bank { bank: "test.bnk".into(), sha256: String::new(), hirc_len_pos: 4, hirc_len: 14, targets }
    }

    /// Object at 8 (size 14, size field at 9), bundle at 20: n=1, pID 5, MakeUpGain -3.0.
    fn original_insert() -> Vec<u8> {
        let mut b = vec![0u8; 27];
        b[4..8].copy_from_slice(&14u32.to_le_bytes());
        b[8] = 2;
        b[9..13].copy_from_slice(&14u32.to_le_bytes());
        b[20] = 1;
        b[21] = 5;
        b[22..26].copy_from_slice(&(-3.0f32).to_le_bytes());
        b
    }

    #[test]
    fn inserts_volume_when_missing() {
        let original = original_insert();
        let target = Target { c_props: 20, obj_size_pos: 9, obj_size: 14 };
        let out = patch_bytes(&original, &bank(vec![target]), 6.0).unwrap();
        assert_eq!(out.len(), original.len() + 5);
        assert_eq!(&out[4..8], &19u32.to_le_bytes());
        assert_eq!(&out[9..13], &19u32.to_le_bytes());
        assert_eq!(out[20], 2);
        assert_eq!(out[21], 0);
        assert_eq!(out[22], 5);
        assert_eq!(&out[23..27], &6.0f32.to_le_bytes());
        assert_eq!(&out[27..31], &(-3.0f32).to_le_bytes());
    }

    #[test]
    fn raises_existing_volume() {
        let mut original = vec![0u8; 31];
        original[20] = 2;
        original[21] = 0;
        original[22] = 5;
        original[23..27].copy_from_slice(&(-18.0f32).to_le_bytes());
        original[27..31].copy_from_slice(&(-25.0f32).to_le_bytes());
        let target = Target { c_props: 20, obj_size_pos: 9, obj_size: 14 };
        let out = patch_bytes(&original, &bank(vec![target]), 6.0).unwrap();
        assert_eq!(out.len(), original.len());
        assert_eq!(&out[23..27], &(-12.0f32).to_le_bytes());
        assert_eq!(&out[27..31], &(-25.0f32).to_le_bytes());
    }

    #[test]
    fn refuses_out_of_range_target() {
        let original = original_insert();
        let target = Target { c_props: 500, obj_size_pos: 9, obj_size: 14 };
        assert!(patch_bytes(&original, &bank(vec![target]), 6.0).is_err());
    }

    #[test]
    fn manifest_parses() {
        let m = manifest().unwrap();
        assert!(!m.banks.is_empty());
        assert!(m.banks.iter().all(|b| !b.targets.is_empty()));
    }
}
