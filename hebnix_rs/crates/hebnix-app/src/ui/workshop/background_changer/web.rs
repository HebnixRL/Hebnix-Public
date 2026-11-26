use super::{ARENAS, AppMsg, BackgroundChangerState, NO_BACKGROUND, arena_of, t, t_args};
use base64::Engine;
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
};

type PreviewIndex = Arc<HashMap<String, PathBuf>>;
type PreviewBatch = (PreviewIndex, Vec<(String, Option<String>)>);

#[derive(Default)]
pub(super) struct Previews {
    root: PathBuf,
    images: HashMap<String, Option<String>>,
    files: Option<PreviewIndex>,
    job: Option<Receiver<PreviewBatch>>,
    requested: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ArenaAction {
    Preview { ids: Vec<String> },
    Apply { host: String, donor: String },
    ApplyAll { donor: String },
    Restore { host: String },
    RestoreAll,
    Refresh,
    Folder,
}

impl BackgroundChangerState {
    pub(crate) fn web_snapshot(&mut self, rl_path: &str, full: bool, ctx: &egui::Context) -> Value {
        if self.last_rl_path != rl_path {
            self.refresh(rl_path);
        }
        let hosts: Vec<_> = self
            .installed_hosts
            .iter()
            .map(|&index| {
                let (id, name) = ARENAS[index];
                json!({ "id": id, "name": name })
            })
            .collect();
        let mut donors: Vec<_> = self
            .installed_donors
            .iter()
            .filter(|entry| arena_of(&entry.package).is_some() == full)
            .map(|entry| json!({ "id": entry.package, "name": Self::message_name(&entry.package) }))
            .collect();
        if !full {
            donors.insert(
                0,
                json!({ "id": NO_BACKGROUND, "name": t("render-no-background") }),
            );
        }
        let mut active: Vec<_> = self
            .active
            .iter()
            .filter(|(_, swap)| arena_of(&swap.donor).is_some() == full)
            .map(|(host, swap)| {
                (
                    host.clone(),
                    Self::display_name(host),
                    Self::message_name(&swap.donor),
                )
            })
            .collect();
        active.sort_unstable_by(|a, b| a.1.cmp(&b.1));
        self.poll_web_previews(rl_path, full, ctx);
        let previews: HashMap<_, _> = self
            .web
            .requested
            .iter()
            .filter_map(|id| self.web.images.get(id).map(|value| (id, value)))
            .collect();
        let remaining = self.bulk_targets().len();
        let bulk_warning = t_args(
            "render-bulk-background-map-counts",
            &[
                ("targets", remaining.to_string().into()),
                (
                    "skipped",
                    self.installed_existing_change_count().to_string().into(),
                ),
            ],
        );
        json!({
            "full": full, "hosts": hosts, "donors": donors, "previews": previews,
            "active": active, "busy": self.busy, "status": self.status,
            "changed": self.status != t("default-choose-the-arena-you-want-to"),
            "running": hebnix_sdk::process::is_rocket_league_running(),
            "bulk_targets": remaining, "bulk_warning": bulk_warning,
        })
    }

    pub(crate) fn web_action(
        &mut self,
        action: ArenaAction,
        rl_path: &str,
        full: bool,
        tx: &Sender<AppMsg>,
        ctx: &egui::Context,
    ) {
        if self.last_rl_path != rl_path {
            self.refresh(rl_path);
        }
        match action {
            ArenaAction::Preview { ids } => {
                self.web.requested = ids.into_iter().take(96).collect();
                self.poll_web_previews(rl_path, full, ctx);
            }
            ArenaAction::Refresh => {
                self.web.images.clear();
                self.web.files = None;
                self.web.job = None;
                self.refresh(rl_path);
            }
            ArenaAction::Folder if !full => self.open_background_folder(),
            _ if self.busy || hebnix_sdk::process::is_rocket_league_running() => {}
            ArenaAction::Apply { host, donor }
                if self.valid_host(&host)
                    && self.valid_donor(&donor, full)
                    && host != arena_of(&donor).unwrap_or(&donor) =>
            {
                self.host.clone_from(&host);
                self.donor.clone_from(&donor);
                self.launch(
                    if donor == NO_BACKGROUND {
                        "remove"
                    } else {
                        "apply"
                    },
                    rl_path,
                    Some(host),
                    Some(donor),
                    tx,
                    ctx,
                );
            }
            ArenaAction::ApplyAll { donor }
                if !full && donor != NO_BACKGROUND && self.valid_donor(&donor, false) =>
            {
                self.donor = donor;
                self.launch_all(rl_path, tx, ctx);
            }
            ArenaAction::Restore { host }
                if self.valid_host(&host)
                    && self
                        .active
                        .get(&host)
                        .is_some_and(|swap| arena_of(&swap.donor).is_some() == full) =>
            {
                self.launch("undo", rl_path, Some(host), None, tx, ctx);
            }
            ArenaAction::RestoreAll => self.restore_mode(rl_path, full, tx, ctx),
            _ => {}
        }
    }

    fn valid_host(&self, host: &str) -> bool {
        self.installed_hosts
            .iter()
            .any(|&index| ARENAS[index].0 == host)
    }

    fn valid_donor(&self, donor: &str, full: bool) -> bool {
        (donor == NO_BACKGROUND && !full)
            || (arena_of(donor).is_some() == full
                && self
                    .installed_donors
                    .iter()
                    .any(|entry| entry.package == donor))
    }

    fn poll_web_previews(&mut self, rl_path: &str, full: bool, ctx: &egui::Context) {
        let cooked = Self::cooked_dir(rl_path);
        if self.web.root != cooked {
            self.web.root = cooked.clone();
            self.web.images.clear();
            self.web.files = None;
            self.web.job = None;
        }
        if let Some(rx) = &self.web.job {
            match rx.try_recv() {
                Ok((files, images)) => {
                    self.web.files = Some(files);
                    self.web.images.extend(images);
                }
                Err(crossbeam_channel::TryRecvError::Empty) => return,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {}
            }
            self.web.job = None;
        }
        let pending: Vec<_> = self
            .web
            .requested
            .iter()
            .filter(|id| !self.web.images.contains_key(*id))
            .filter(|id| self.valid_host(id) || self.valid_donor(id, full))
            .cloned()
            .collect();
        if pending.is_empty() {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.web.job = Some(rx);
        let ctx = ctx.clone();
        let files = self.web.files.clone();
        std::thread::spawn(move || {
            let files = files.unwrap_or_else(|| {
                Arc::new(
                    std::fs::read_dir(&cooked)
                        .into_iter()
                        .flatten()
                        .flatten()
                        .filter_map(|entry| {
                            Some((
                                entry.file_name().to_str()?.to_ascii_lowercase(),
                                entry.path(),
                            ))
                        })
                        .filter(|(name, _)| {
                            name.ends_with("_mapimage_sf.upk")
                                || name.ends_with("_t.upk")
                                || name.ends_with("_t_sf.upk")
                        })
                        .collect(),
                )
            });
            let images = pending
                .into_iter()
                .map(|id| {
                    let image = arena_preview(&files, arena_of(&id).unwrap_or(&id));
                    (id, image)
                })
                .collect();
            let _ = tx.send((files, images));
            ctx.request_repaint();
        });
    }
}

fn arena_preview(files: &HashMap<String, PathBuf>, package: &str) -> Option<String> {
    if !package
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return None;
    }
    let base = package.strip_suffix("_P").unwrap_or(package);
    let stock = base.strip_prefix("BG_").unwrap_or(base);
    let alias = match stock.to_ascii_lowercase().as_str() {
        "shattershot_vfx" => "shattershot",
        "woods_day" => "woods",
        "utopiasnow" => "utopiastadium_snow",
        "labs_4v4_arena15_retro_oob" => "labs_4v4_arena15_retro",
        _ => stock,
    };
    let mut stems = vec![alias, base, stock];
    stems.dedup();
    if let Some((family, _)) = stock.split_once('_') {
        stems.push(family);
    }
    for stem in stems {
        for suffix in [
            "_P_MapImage_SF.upk",
            "_MapImage_SF.upk",
            "_T.upk",
            "_T_SF.upk",
        ] {
            if let Some(file) = files.get(&format!("{stem}{suffix}").to_ascii_lowercase()) {
                if let Ok(bytes) = crate::patcher::cosmetic_thumbnail::extract_arena_preview(file) {
                    return Some(format!(
                        "data:image/jpeg;base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(bytes)
                    ));
                }
            }
        }
    }
    None
}
