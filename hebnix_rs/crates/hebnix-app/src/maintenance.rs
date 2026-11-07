//! Windows maintenance runs outside Hebnix so the executable is no longer locked.
use std::path::{Path, PathBuf};

#[derive(Default, Clone)]
pub struct UninstallUi {
    pub tab: bool,
    pub application: bool,
    pub settings: bool,
    pub extensions: bool,
    pub assets: bool,
    pub restore_game: bool,
    pub certificate: bool,
    pub confirm: bool,
    pub error: Option<String>,
}

impl UninstallUi {
    /// Returns true only after the user confirms the selected removal scope.
    pub fn render(
        &mut self,
        ui: &mut eframe::egui::Ui,
        can_restore_game: bool,
        base: &Path,
    ) -> bool {
        use crate::i18n::t;
        if !cfg!(windows) {
            ui.label(t("maintenance-windows"));
            return false;
        }
        ui.checkbox(&mut self.application, t("maintenance-app"));
        ui.checkbox(&mut self.settings, t("maintenance-settings"));
        ui.checkbox(&mut self.extensions, t("maintenance-extensions"));
        ui.checkbox(&mut self.assets, t("maintenance-assets"));
        if can_restore_game {
            ui.checkbox(&mut self.restore_game, t("maintenance-restore"));
        }
        ui.checkbox(&mut self.certificate, t("maintenance-certificate"));
        ui.label(t("maintenance-note"));
        ui.label(base.display().to_string());
        eframe::egui::CollapsingHeader::new(t("maintenance-files")).show(ui, |ui| {
            for path in removal_paths(base, self) {
                ui.monospace(path.display().to_string());
            }
            if self.application {
                if let Ok(exe) = std::env::current_exe() {
                    ui.monospace(exe.display().to_string());
                }
            }
        });
        if let Some(error) = &self.error {
            ui.colored_label(eframe::egui::Color32::RED, error);
        }
        let selected = self.application
            || self.settings
            || self.extensions
            || self.assets
            || self.restore_game
            || self.certificate;
        if ui
            .add_enabled(selected, eframe::egui::Button::new(t("maintenance-remove")))
            .clicked()
        {
            self.confirm = true;
        }
        let mut confirmed = false;
        if self.confirm {
            ui.separator();
            ui.label(t("maintenance-confirm"));
            ui.horizontal(|ui| {
                confirmed = ui.button(t("maintenance-confirm-button")).clicked();
                if ui.button(t("maintenance-cancel")).clicked() {
                    self.confirm = false;
                }
            });
        }
        confirmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selective_removal_never_includes_the_parent_or_unselected_data() {
        let base = Path::new("C:/Users/example/Downloads");
        let options = UninstallUi {
            extensions: true,
            ..Default::default()
        };
        let paths = removal_paths(base, &options);
        assert!(paths.contains(&base.join("plugins")));
        assert!(!paths.contains(&base.join("config.toml")));
        assert!(!paths.contains(&base.join("state")));
        assert!(!paths.contains(&base.to_path_buf()));
        assert!(paths.iter().all(|p| p.parent() == Some(base)));
    }
    #[test]
    fn recovery_and_certificate_data_need_explicit_related_options() {
        let base = Path::new("C:/portable");
        let mut options = UninstallUi {
            application: true,
            settings: true,
            assets: true,
            ..Default::default()
        };
        assert!(!removal_paths(base, &options).contains(&base.join("state")));
        assert!(!removal_paths(base, &options).contains(&base.join("spoofer")));
        options.restore_game = true;
        options.certificate = true;
        assert!(removal_paths(base, &options).contains(&base.join("state")));
        assert!(removal_paths(base, &options).contains(&base.join("spoofer")));
    }
}

#[cfg(windows)]
fn powershell() -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut command = std::process::Command::new(
        PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
            .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
    );
    command.creation_flags(0x08000000);
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-WindowStyle",
        "Hidden",
        "-ExecutionPolicy",
        "Bypass",
    ]);
    command
}

#[cfg(windows)]
fn helper(plan: serde_json::Value) -> Result<(), String> {
    let dir =
        std::env::temp_dir().join(format!("hebnix-maintenance-{:016x}", rand::random::<u64>()));
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let script = dir.join("maintenance.ps1");
    let plan_path = dir.join("plan.json");
    std::fs::write(&script, include_str!("maintenance.ps1")).map_err(|e| e.to_string())?;
    std::fs::write(
        &plan_path,
        serde_json::to_vec(&plan).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    powershell()
        .arg("-File")
        .arg(&script)
        .arg("-PlanPath")
        .arg(&plan_path)
        .spawn()
        .map_err(|e| format!("Could not start maintenance: {e}"))?;
    Ok(())
}

pub fn launch_update(staged: &Path, base: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        return helper(serde_json::json!({
            "mode": "update", "pid": std::process::id(), "exe": exe, "base": base,
            "staged": staged, "backup": exe.with_extension(format!("old-{:016x}.exe", rand::random::<u64>()))
        }));
    }
    #[cfg(not(windows))]
    {
        let _ = (staged, base);
        Err("Direct EXE updates are available on Windows only.".into())
    }
}

// Explicit owned entries: never recursively delete the executable/data parent,
// which may be Downloads, a USB drive, or a developer's custom data directory.
pub fn removal_paths(base: &Path, options: &UninstallUi) -> Vec<PathBuf> {
    let mut names = Vec::new();
    if options.settings {
        names.extend([
            "config.toml",
            "config.toml.tmp",
            "config.ini",
            "spoofer_settings.json",
            "owned_products.json",
            "friends.json",
            "replay_upload_history.json",
            "replay_pending_matches.json",
            "replay_upload_history.tmp",
            "replay_pending_matches.tmp",
        ]);
    }
    if options.extensions {
        names.extend([
            "plugins",
            "themes",
            "pending-plugin-installs",
            "pending-theme-installs",
        ]);
    }
    if options.assets {
        names.extend([
            "assets",
            "fonts",
            "balls",
            "boosts",
            "decals",
            "cars",
            "anthems",
            "presets",
            "curl-impersonate",
            "tools",
            "multiplayer-lan",
        ]);
    }
    if options.application {
        names.extend([
            "hebnix.log",
            "hebnix.log.old",
            "hebnix-lite.log",
            "crash.txt",
            "theme_errors.txt",
            ".desktop-shortcut-v1",
            ".desktop-shortcut-lite-v1",
            ".appdata-migration-v1",
            ".launch",
            ".first",
            "hebnix.ico",
            "updater",
            "rank_spoofer_frames.log",
        ]);
    }
    if options.restore_game && options.settings {
        names.extend(["state", "title_upk"]);
    }
    if options.certificate && options.settings {
        names.push("spoofer");
    }
    names.into_iter().map(|name| base.join(name)).collect()
}

pub fn launch_uninstall(base: &Path, options: &UninstallUi) -> Result<(), String> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut paths = removal_paths(base, options);
        if options.application {
            paths.push(exe.clone());
        }
        if options.application && options.settings {
            if let Some(parent) = exe.parent() {
                paths.push(parent.join("portable.txt"));
            }
        }
        return helper(serde_json::json!({
            "mode": "uninstall", "pid": std::process::id(), "exe": exe,
            "paths": paths, "remove_app": options.application, "base": base,
            "shortcut": dirs::desktop_dir().map(|p| p.join(shortcut_name()))
        }));
    }
    #[cfg(not(windows))]
    {
        let _ = (base, options);
        Err("Self-uninstall is available on Windows only.".into())
    }
}

fn shortcut_name() -> &'static str {
    if cfg!(feature = "lite")
        && std::env::current_exe().is_ok_and(|p| {
            p.file_stem()
                .is_some_and(|s| s.to_string_lossy().contains("lite"))
        })
    {
        "Hebnix Lite.lnk"
    } else {
        "Hebnix.lnk"
    }
}

/// One attempt per data directory; deleting a shortcut later is respected.
pub fn first_launch_shortcut(base: &Path) {
    #[cfg(windows)]
    {
        let marker = base.join(if shortcut_name() == "Hebnix Lite.lnk" {
            ".desktop-shortcut-lite-v1"
        } else {
            ".desktop-shortcut-v1"
        });
        if marker.exists() {
            return;
        }
        let result = (|| -> Result<(), String> {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let link = dirs::desktop_dir()
                .ok_or("Desktop folder unavailable")?
                .join(shortcut_name());
            let output = powershell().arg("-Command").arg(
                "$ErrorActionPreference='Stop'; if (!(Test-Path -LiteralPath $env:HEBNIX_SHORTCUT)) { $s=(New-Object -ComObject WScript.Shell).CreateShortcut($env:HEBNIX_SHORTCUT); $s.TargetPath=$env:HEBNIX_EXE; $s.WorkingDirectory=[IO.Path]::GetDirectoryName($env:HEBNIX_EXE); $s.IconLocation=$env:HEBNIX_EXE+',0'; $s.Save() }"
            ).env("HEBNIX_SHORTCUT", link).env("HEBNIX_EXE", exe).output().map_err(|e| e.to_string())?;
            if !output.status.success() {
                return Err(String::from_utf8_lossy(&output.stderr).into_owned());
            }
            std::fs::write(marker, b"shortcut initialized\n").map_err(|e| e.to_string())
        })();
        if let Err(error) = result {
            tracing::warn!("Desktop shortcut: {error}");
        }
    }
    #[cfg(not(windows))]
    let _ = base;
}

pub fn acknowledge_update() {
    static ACK: std::sync::Once = std::sync::Once::new();
    ACK.call_once(|| {
        if let Some(path) = std::env::var_os("HEBNIX_UPDATE_ACK") {
            if let Err(error) = std::fs::write(PathBuf::from(path), b"started") {
                tracing::warn!("Could not acknowledge update startup: {error}");
            }
        }
    });
}

pub fn remove_certificate() -> Result<(), String> {
    #[cfg(windows)]
    {
        let output = powershell().arg("-Command").arg(
            r"$ErrorActionPreference='Stop'; Get-ChildItem Cert:\CurrentUser\Root | Where-Object { $_.GetNameInfo([System.Security.Cryptography.X509Certificates.X509NameType]::SimpleName, $false) -eq 'Hebnix Local CA' } | Remove-Item -Force"
        ).output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into_owned());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    Err("Certificate removal from this tab is available on Windows only.".into())
}
