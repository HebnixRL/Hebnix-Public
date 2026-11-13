use super::{abc, movie, run};
use crossbeam_channel::{Receiver, TryRecvError, unbounded};
use eframe::egui;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
pub struct RankPick {
    pub playlist: u8,
    pub tier: u8,
    pub division: u8,
    pub mmr: usize,
    pub players: u8,
}

fn apply(base: &Path, cooked: &Path, picks: &[RankPick]) -> Result<String, String> {
    if picks.is_empty() {
        return run(base, cooked, true, None, None);
    }
    if picks
        .iter()
        .any(|pick| pick.tier > 22 || pick.division > 3 || !(100..=9999).contains(&pick.mmr))
    {
        return Err("Choose a valid rank, division and MMR between 100 and 9999.".into());
    }
    run(
        base,
        cooked,
        true,
        Some(&|raw| movie::rewrite(raw, &|data| abc::patch_ranks(data, picks))),
        None,
    )
}

#[derive(Default)]
pub struct RankPatchState {
    result: Option<Receiver<Result<String, String>>>,
    pub status: String,
}

impl RankPatchState {
    pub fn busy(&self) -> bool {
        self.result.is_some()
    }

    pub fn poll(&mut self) -> bool {
        let Some(receiver) = &self.result else {
            return false;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err("Rank patch worker stopped.".into()),
        };
        self.result = None;
        let applied = result.is_ok();
        self.status = result.unwrap_or_else(|error| format!("Rank patch failed: {error}"));
        applied
    }

    pub fn start(
        &mut self,
        base: PathBuf,
        cooked: PathBuf,
        picks: Vec<RankPick>,
        ctx: egui::Context,
    ) {
        if self.busy() {
            return;
        }
        let (tx, rx) = unbounded();
        self.result = Some(rx);
        self.status = "Preparing rank UI packages…".into();
        std::thread::spawn(move || {
            let _ = tx.send(apply(&base, &cooked, &picks));
            ctx.request_repaint();
        });
    }
}

#[cfg(test)]
#[test]
fn rank_patch_round_trips_game_movies() {
    let Some(source) = std::env::var_os("HEBNIX_RANK_FIXTURE").map(PathBuf::from) else {
        return;
    };
    let temp =
        std::env::temp_dir().join(format!("hebnix-rank-test-{:016x}", rand::random::<u64>()));
    let cooked = temp.join("TAGame/CookedPCConsole");
    std::fs::create_dir_all(&cooked).unwrap();
    let binaries = temp.join("Binaries/Win64");
    std::fs::create_dir_all(&binaries).unwrap();
    std::fs::write(binaries.join("RocketLeague.exe"), b"fixture build").unwrap();
    let files = ["GFX_PlayMenuV4_SF.upk", "GFX_HudMatchInfo_SF.upk"];
    for file in files {
        std::fs::copy(source.join(file), cooked.join(file)).unwrap();
    }
    let pick = RankPick {
        playlist: 11,
        tier: 22,
        division: 0,
        mmr: 1800,
        players: 4,
    };
    apply(&temp, &cooked, &[pick]).unwrap();
    let first = files.map(|file| std::fs::read(cooked.join(file)).unwrap());
    for (file, patched) in files.into_iter().zip(&first) {
        assert_ne!(
            *patched,
            std::fs::read(source.join(file)).unwrap(),
            "{file}"
        );
    }

    apply(&temp, &cooked, &[RankPick { mmr: 1900, ..pick }]).unwrap();
    for (file, patched) in files.into_iter().zip(&first) {
        assert_ne!(
            *patched,
            std::fs::read(cooked.join(file)).unwrap(),
            "{file}"
        );
    }
    apply(&temp, &cooked, &[]).unwrap();
    for file in files {
        let restored = super::UpkPackage::load(&cooked.join(file)).unwrap();
        let original = super::UpkPackage::load(&source.join(file)).unwrap();
        assert_eq!(restored.names, original.names);
        assert_eq!(restored.exports.len(), original.exports.len());
        for index in 0..original.exports.len() {
            assert_eq!(
                super::data(&restored, index).unwrap(),
                super::data(&original, index).unwrap()
            );
        }
    }
    std::fs::remove_dir_all(temp).unwrap();
}

#[cfg(test)]
#[test]
fn legacy_titles_and_ranks_can_be_updated_and_restored_independently() {
    let Some(original) = std::env::var_os("HEBNIX_SHARED_UI_FIXTURE").map(PathBuf::from) else {
        return;
    };
    let root =
        std::env::temp_dir().join(format!("hebnix-shared-ui-{:016x}", rand::random::<u64>()));
    let cooked = root.join("TAGame/CookedPCConsole");
    let binaries = root.join("Binaries/Win64");
    std::fs::create_dir_all(&cooked).unwrap();
    std::fs::create_dir_all(&binaries).unwrap();
    std::fs::write(binaries.join("RocketLeague.exe"), b"fixture build").unwrap();
    let file = cooked.join("GFX_BlueprintsWalkthrough_SF.upk");
    std::fs::copy(&original, &file).unwrap();
    let exports = || {
        let package = super::UpkPackage::load(&file).unwrap();
        (0..package.exports.len())
            .map(|index| super::data(&package, index).unwrap().to_vec())
            .collect::<Vec<_>>()
    };
    let vanilla = exports();
    let mut title = super::TitleOverride {
        target_id: None,
        original: None,
        text: "Shared movie test".into(),
        color: 0xff0000,
        glow: false,
        glow_color: None,
    };
    let mut rank = RankPick {
        playlist: 11,
        tier: 22,
        division: 0,
        mmr: 1800,
        players: 4,
    };
    super::apply(&root, &cooked, std::slice::from_ref(&title)).unwrap();
    let title_only = exports();
    assert_ne!(title_only, vanilla);
    // Older manifests have only the original package and export hashes.
    std::fs::remove_dir_all(super::folder(&root, &cooked, "title_upk").join("exports")).unwrap();
    apply(&root, &cooked, &[rank]).unwrap();
    assert_ne!(exports(), title_only);
    rank.mmr = 1900;
    apply(&root, &cooked, &[rank]).unwrap();
    apply(&root, &cooked, &[]).unwrap();
    assert_eq!(exports(), title_only);

    apply(&root, &cooked, &[rank]).unwrap();
    title.text = "Updated shared movie test".into();
    super::apply(&root, &cooked, std::slice::from_ref(&title)).unwrap();
    let both_updated = exports();
    apply(&root, &cooked, &[]).unwrap();
    assert_ne!(exports(), title_only);
    assert_ne!(exports(), both_updated);
    apply(&root, &cooked, &[rank]).unwrap();
    assert_eq!(exports(), both_updated);
    super::restore(&root, &cooked).unwrap();
    let rank_only = exports();
    assert_ne!(rank_only, vanilla);
    apply(&root, &cooked, &[]).unwrap();
    assert_eq!(exports(), vanilla);
    apply(&root, &cooked, &[rank]).unwrap();
    assert_eq!(exports(), rank_only);
    std::fs::remove_dir_all(root).unwrap();
}
