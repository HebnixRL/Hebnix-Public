//! HTTP feature requirements and certificate controls shared by Customisation and System.
use super::*;

#[derive(Clone, Copy, Default)]
pub(super) struct HttpSpooferFeatures {
    username: bool,
    friends: bool,
}

impl HttpSpooferFeatures {
    pub(super) fn any(self) -> bool {
        self.username || self.friends
    }

    fn can_run(self, admin: bool, certificate: bool) -> bool {
        self.any() && admin && certificate
    }
}

impl HebnixApp {
    pub(super) fn http_spoofer_features(&self) -> HttpSpooferFeatures {
        HttpSpooferFeatures {
            username: self.spoofer_username_enabled,
            friends: self.spoofer_friends_enabled,
        }
    }

    pub(super) fn http_spoofer_ready(&self) -> bool {
        self.http_spoofer_features()
            .can_run(!spoofer::http_needs_elevated_relaunch(), self.spoofer_cert_installed)
    }

    pub(super) fn prompt_for_missing_spoofer_certificate(&mut self) {
        self.spoofer_cert_installed = spoofer::ca::is_current_installed(&self.base_dir);
        let requested = self.http_spoofer_features();
        if requested.any() && spoofer::http_needs_elevated_relaunch() {
            let mut pending = self.pending_spoofer_features.unwrap_or_default();
            pending.username |= requested.username;
            pending.friends |= requested.friends;
            self.pending_spoofer_features = Some(pending);
            self.disable_certificate_features();
            self.admin_prompt_open = true;
        } else if requested.any() && !self.spoofer_cert_installed {
            self.disable_certificate_features();
            self.spoofer_certificate_prompt_open = true;
            self.console.write("[Spoofer] Install the certificate in Settings > System before enabling HTTP features again.");
        } else if self.spoofer_cert_installed {
            self.spoofer_mgr.certificate_setup_ready();
            self.spoofer_certificate_prompt_open = false;
        }
    }

    pub(super) fn install_spoofer_certificate(&mut self) {
        if spoofer::http_needs_elevated_relaunch() {
            self.admin_prompt_open = true;
            return;
        }
        match spoofer::install_certificate(&self.base_dir) {
            Ok(()) => {
                self.spoofer_cert_installed = spoofer::ca::is_current_installed(&self.base_dir);
                if self.spoofer_cert_installed {
                    self.spoofer_mgr.certificate_setup_ready();
                    self.spoofer_certificate_prompt_open = false;
                    self.console.write(
                        "[Spoofer] Certificate installed. HTTP features can now be enabled.",
                    );
                    self.evaluate_proxies();
                } else {
                    self.spoofer_certificate_prompt_open = true;
                    self.console.write("[Spoofer] Certificate installation did not establish trust. Try installing it again.");
                }
            }
            Err(error) => {
                self.spoofer_certificate_prompt_open = true;
                self.console
                    .write(format!("[Spoofer] Certificate install failed: {error}"));
            }
        }
    }

    pub(super) fn render_spoofer_certificate_settings(&mut self, ui: &mut egui::Ui) {
        #[cfg(not(target_os = "linux"))]
        {
            ui.heading("Account HTTPS proxy certificate");
            ui.weak("Name and Friends use the account HTTPS proxy while Rocket League is running. It routes only api.epicgames.dev and leaves Windows proxy settings unchanged. Administrator access and this certificate are required.");
        }
        #[cfg(target_os = "linux")]
        {
            ui.heading("HTTP proxy certificate");
            ui.weak("Name and Friends point Rocket League's Wine prefix at Hebnix's HTTP proxy before the game starts (and put it back after). No administrator restart is needed; installing or removing this certificate asks for your password once.");
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(t("spoofer-certificate-status"));
            ui.label(if self.spoofer_cert_installed {
                t("spoofer-installed")
            } else {
                t("spoofer-missing")
            });
            if self.spoofer_cert_installed {
                if ui.button(t("spoofer-remove")).clicked() {
                    if !spoofer::http_needs_elevated_relaunch() {
                        self.disable_certificate_features();
                        if self.spoofer_mgr.account_redirect_active() {
                            self.console.write("[Spoofer] The account redirect could not be removed. The certificate is retained until cleanup succeeds.");
                        } else { match spoofer::uninstall_certificate(&self.base_dir) {
                            Ok(()) => self.console.write("[Spoofer] Certificate removed."),
                            Err(error) => self
                                .console
                                .write(format!("[Spoofer] Certificate removal failed: {error}")),
                        } }
                        self.spoofer_cert_installed =
                            spoofer::ca::is_current_installed(&self.base_dir);
                    } else {
                        self.admin_prompt_open = true;
                    }
                }
            } else if ui.button(t("spoofer-install-certificate")).clicked() {
                self.install_spoofer_certificate();
            }
            if ui
                .button("↻")
                .on_hover_text(t("spoofer-refresh-status"))
                .clicked()
            {
                self.prompt_for_missing_spoofer_certificate();
                self.evaluate_proxies();
            }
        });
        if spoofer::http_needs_elevated_relaunch() {
            ui.weak("Restart as Administrator to install or remove the certificate.");
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(t("spoofer-http-proxy"));
            ui.label(if self.spoofer_mgr.http_running() {
                t("spoofer-running")
            } else if self.proxy_options().account && !self.spoofer_mgr.game_running() {
                t("spoofer-waiting-for-game")
            } else {
                t("spoofer-stopped")
            });
        });
    }

    pub(super) fn render_admin_prompt(&mut self, ctx: &egui::Context) {
        if !self.admin_prompt_open {
            return;
        }
        let mut restart = false;
        let mut cancel = false;
        egui::Window::new(t("admin-prompt-administrator-required"))
            .id(egui::Id::new("admin-prompt-administrator-required"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(t("admin-prompt-this-action-requires-hebnix-to-be"));
                if self.pending_spoofer_features.is_some() {
                    ui.label("Restart as Administrator to enable the selected HTTP features. They stay off if elevation is declined.");
                } else {
                    ui.label("Restart as Administrator, then manage the certificate in Settings > System.");
                }
                ui.horizontal(|ui| {
                    restart = ui.button("Restart as Administrator").clicked();
                    cancel = ui.button(t("spawner-enable-prompt-cancel")).clicked();
                });
            });
        if restart {
            self.admin_prompt_open = false;
            if let Some(pending) = self.pending_spoofer_features.take() {
                self.spoofer_username_enabled = pending.username;
                self.spoofer_friends_enabled = pending.friends;
            }
            // Persist the request for the elevated process, without starting a
            // proxy in this non-admin process.
            self.save_config();
            self.save_friends_internal();
            if spoofer::spawn_elevated_relaunch() {
                self.spoofer_mgr.shutdown();
                self.workshop.suspend_multiplayer();
                std::process::exit(0);
            }
            self.disable_certificate_features();
            self.console.write(
                "[Spoofer] Couldn't relaunch as Administrator. HTTP features remain disabled.",
            );
        } else if cancel {
            self.admin_prompt_open = false;
            self.pending_spoofer_features = None;
        }
    }
}
