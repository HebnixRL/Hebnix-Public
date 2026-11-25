use super::{HebnixApp, PluginsSubTab, StatsApiNotice, Tab, WebPortNotice, t, t_args};
use eframe::egui;
use serde_json::{Value, json};

fn prompt(id: &str, title: String, lines: Vec<String>, buttons: &[(&str, String, bool)]) -> Value {
    let buttons = buttons
        .iter()
        .map(|(key, label, close)| {
            json!({
                "key": key, "label": label, "close": close,
            })
        })
        .collect::<Vec<_>>();
    json!({"id": id, "title": title, "lines": lines, "buttons": buttons})
}

impl HebnixApp {
    pub(super) fn show_global_dialog(&mut self, ctx: &egui::Context) {
        if self.hidden || self.modern_navigation.is_none() {
            return;
        }
        let dialog = self.global_dialog();
        if let Some(navigation) = &mut self.modern_navigation
            && let Err(error) = navigation.show_dialog(ctx, dialog.as_ref())
        {
            tracing::warn!(%error, "could not display Wry dialog");
        }
    }

    fn global_dialog(&self) -> Option<Value> {
        let close = || ("dismiss", t("tray-close"), true);
        let ok = || ("dismiss", t("btn-ok"), true);
        let cancel = || ("dismiss", t("spawner-enable-prompt-cancel"), true);
        if let Some(info) = &self.update_info {
            let buttons = [("primary", t("update-button"), false)];
            let mut model = prompt(
                "update",
                t("update-required-title"),
                vec![
                    t_args(
                        "update-required-heading",
                        &[("version", info.version.as_str().into())],
                    ),
                    t("update-locked-note"),
                ],
                if self.update_downloading {
                    &[]
                } else {
                    &buttons
                },
            );
            model["error"] = json!(self.update_error);
            if self.update_downloading {
                let progress = self
                    .update_progress
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                model["progress"] = json!({"label": t("update-downloading"), "value": progress.downloaded, "total": progress.total});
                model["lines"]
                    .as_array_mut()?
                    .push(json!(if progress.ready {
                        t("update-restarting")
                    } else {
                        format!(
                            "{} · {:.1} MB",
                            t("update-downloading"),
                            progress.downloaded as f64 / 1048576.0
                        )
                    }));
            }
            return Some(model);
        }
        if self.maintenance_running {
            return Some(prompt(
                "maintenance",
                t("maintenance-tab"),
                vec![t("maintenance-working")],
                &[],
            ));
        }
        if self.recovery_running() {
            return Some(prompt(
                "game-recovery",
                t("game-update-title"),
                self.web_recovery_status(),
                &[],
            ));
        }
        if let Some(notice) = self.statsapi_notice {
            let message = match notice {
                StatsApiNotice::NotConfigured => "Rocket League’s StatsAPI is missing. Set PacketSendRate to 20 to receive match data.".to_owned(),
                StatsApiNotice::TooLow(rate) => format!("PacketSendRate is {rate}. Hebnix requires a higher rate. Set it to 20."),
                StatsApiNotice::BelowTarget(rate) => format!("PacketSendRate is {rate}. Set it to 20 for more frequent plugin updates."),
                StatsApiNotice::AboveTarget(rate) => format!("PacketSendRate is {rate}. The recommended value is 20."),
            };
            let dismiss = if notice.blocking() {
                t("statsapi-notice-quit-hebnix")
            } else {
                t("statsapi-notice-proceed")
            };
            let mut model = prompt(
                "statsapi",
                t("statsapi-notice-statsapi-configuration"),
                vec![message],
                &[
                    ("primary", t("statsapi-notice-set-20"), false),
                    ("dismiss", dismiss, true),
                ],
            );
            model["error"] = json!(self.statsapi_apply_error);
            return Some(model);
        }
        if let Some(notice) = self.web_port_notice {
            let message = match notice {
                WebPortNotice::NotConfigured => {
                    "WebPort is missing from DefaultStatsAPI.ini. The standard port is 49124."
                        .to_owned()
                }
                WebPortNotice::InvalidPort(port) => {
                    format!("WebPort is set to {port}. The standard port is 49124.")
                }
            };
            return Some(prompt(
                "web-port",
                t("web-port-notice-webport-configuration"),
                vec![message],
                &[
                    ("primary", t("web-port-notice-set-49124"), true),
                    ("dismiss", t("web-port-notice-dismiss"), true),
                ],
            ));
        }
        if self.admin_prompt_open || self.colour_admin_prompt_open {
            let (id, body) = if self.admin_prompt_open {
                ("admin", t("admin-prompt-this-action-requires-hebnix-to-be"))
            } else {
                (
                    "colour-admin",
                    t("colour-admin-prompt-windows-denied-write-access-to-tagame"),
                )
            };
            return Some(prompt(
                id,
                t("admin-prompt-administrator-required"),
                vec![body],
                &[
                    (
                        "primary",
                        t("colour-admin-prompt-restart-as-administrator"),
                        true,
                    ),
                    cancel(),
                ],
            ));
        }
        if self.item_action_prompt_open {
            return Some(prompt(
                "close-game",
                t("item-action-prompt-rocket-league-is-open"),
                vec![t("item-action-prompt-rocket-league-must-be-closed-to")],
                &[
                    ("primary", t("item-action-prompt-quit-rocket-league"), true),
                    ("dismiss", t("item-action-prompt-close-prompt"), true),
                ],
            ));
        }
        let items = self.spoofer_mgr.item_status();
        if items.clearing && !self.item_clear_prompt_dismissed {
            let mut model = prompt("clear-items", "Clear Items".into(), vec![
                "Removes tracked instances from the most recently saved Rocket League account’s inventory, equipped loadouts and rotating save copies. Hebnix backs up the original saves first.".into(),
                "Close Rocket League and Item Spawner before removing tracked items.".into(), items.text.to_string(),
            ], &[("primary", "Check Again".into(), false), ("dismiss", if items.can_cancel { "Cancel" } else { "Dismiss" }.into(), !items.can_cancel)]);
            model["error"] = json!(self.item_clear_error);
            return Some(model);
        }
        if self.spoofer_certificate_prompt_open {
            return Some(prompt(
                "certificate",
                t("spoofer-certificate-required-title"),
                vec!["Reinstall the HTTP proxy certificate to use Player Name and Friends.".into()],
                &[
                    ("primary", t("spoofer-install-certificate"), false),
                    close(),
                ],
            ));
        }
        if self.restart_notice {
            return Some(prompt(
                "restart-game",
                t("about-hebnix"),
                vec![t("app-hebnix-has-initialised-statsapi-please-r")],
                &[("primary", t("action-restart-rocket-league"), true), ok()],
            ));
        }
        if self.fullscreen_notice {
            let mut model = prompt(
                "fullscreen",
                t("fullscreen-notice-overlay-unavailable"),
                vec![t("fullscreen-notice-rocket-league-is-set-to-fullscreen")],
                &[close()],
            );
            model["toggle"] = json!({"label": t("fullscreen-notice-never-show-again"), "checked": self.config.settings.suppress_fullscreen_warning});
            return Some(model);
        }
        if self.launch_path_notice {
            return Some(prompt(
                "launch-path",
                t("launch-path-notice-rocket-league"),
                vec![t("launch-notice-body")],
                &[ok()],
            ));
        }
        if self.epic_repair.confirm {
            return Some(prompt(
                "epic-repair",
                t("action-fix-epic-connection"),
                vec![t("epic-connection-rocket-league-must-close-to-repair")],
                &[
                    ("primary", t("plugin-delete-prompt-yes"), true),
                    ("dismiss", t("plugin-delete-prompt-no"), true),
                ],
            ));
        }
        if self.epic_repair.running {
            return Some(prompt(
                "epic-repair-working",
                t("action-fix-epic-connection"),
                vec!["Repairing Epic connection…".into()],
                &[],
            ));
        }
        if let Some(result) = &self.epic_repair.result {
            let message = match result {
                Ok(()) => t("epic-connection-epic-connection-repaired"),
                Err(error) => t_args(
                    "epic-connection-could-not-repair-the-epic-connection",
                    &[("error", error.as_str().into())],
                ),
            };
            return Some(prompt(
                "epic-repair-result",
                t("action-fix-epic-connection"),
                vec![message],
                &[ok()],
            ));
        }
        if self.install_modal.open {
            return Some(prompt(
                "install-plugin",
                t("plugins-install-plugin"),
                Vec::new(),
                &[
                    ("primary", t("install-modal-install-from-hebnix"), true),
                    ("zip", t("install-modal-install-from-zip"), false),
                    close(),
                ],
            ));
        }
        if let Some(entry) = &self.changelog_popup {
            let mut lines = vec![t_args(
                "changelog-heading",
                &[("version", entry.version.as_str().into())],
            )];
            if !entry.release_date.is_empty() {
                lines.push(entry.release_date.clone());
            }
            lines.extend(entry.changelog.lines().filter_map(|line| {
                let line = line.trim();
                (!line.is_empty()).then(|| {
                    for (tag, label) in [
                        ("[Added]", "changelog-added"),
                        ("[Fixed]", "changelog-fixed"),
                        ("[Removed]", "changelog-removed"),
                    ] {
                        if let Some(body) = line.strip_prefix(tag) {
                            return format!(
                                "{} {}",
                                t(label),
                                body.trim().trim_start_matches(':').trim()
                            );
                        }
                    }
                    line.to_owned()
                })
            }));
            return Some(prompt("changelog", t("changelog-title"), lines, &[close()]));
        }
        if self.req_clock_warning_open {
            return Some(prompt(
                "clock",
                t("req-clock-warning-title"),
                vec![t("req-clock-warning-body")],
                &[ok()],
            ));
        }
        if let Some((peer, map, busy)) = self.workshop.web_join_offer(&self.config.settings.rl_path)
        {
            let mut buttons = vec![("dismiss", t("join-mismatch-dismiss"), true)];
            if !busy {
                buttons.push(("primary", t("player-maps-download-to-workshop"), true));
            }
            return Some(prompt(
                "lan-join",
                t("join-mismatch-title"),
                vec![t_args("join-mismatch", &[("name", peer.into())]), map],
                &buttons,
            ));
        }
        if !(self.tab == Tab::Workshop
            && self.workshop.view == crate::ui::workshop::WorkshopView::Multiplayer)
            && let Some(name) = self.workshop.web_pending_peer()
        {
            return Some(prompt(
                "lan-install",
                t("peer-install-warning-install-from-another-player"),
                vec![
                    t_args(
                        "peer-install-warning-name-will-be-downloaded-straight-from",
                        &[("name", name.into())],
                    ),
                    t("peer-install-warning-hebnix-only-checks-that-the-file"),
                ],
                &[("primary", t("hebnix-install-install"), true), cancel()],
            ));
        }
        if let Some(notification) = &self.notification_popup {
            let mut buttons = Vec::new();
            if notification.link_url().is_some() {
                let label = notification
                    .link_text
                    .as_deref()
                    .filter(|label| !label.trim().is_empty())
                    .unwrap_or("Open link");
                buttons.push(("primary", label.into(), false));
            }
            buttons.push(("dismiss", "Dismiss".into(), true));
            return Some(prompt(
                "notification",
                "Notification".into(),
                vec![notification.message.clone()],
                &buttons,
            ));
        }
        None
    }

    pub(super) fn handle_global_dialog(&mut self, id: &str, key: &str, ctx: &egui::Context) {
        if !self
            .global_dialog()
            .is_some_and(|dialog| dialog["id"].as_str() == Some(id))
        {
            return;
        }
        match (id, key) {
            ("update", "primary") if !self.update_downloading => {
                if let Some(info) = self.update_info.clone() {
                    self.start_app_update(info, ctx);
                }
            }
            ("statsapi", "primary") => self.apply_statsapi_rate(),
            ("statsapi", "dismiss") => {
                if self.statsapi_notice.is_some_and(StatsApiNotice::blocking) {
                    self.force_quit(ctx);
                } else {
                    self.statsapi_notice = None;
                }
            }
            ("web-port", "primary") => self.apply_web_port_setting(),
            ("web-port", "dismiss") => self.web_port_notice = None,
            ("admin", "primary") => self.resolve_admin_prompt(true, false),
            ("admin", "dismiss") => self.resolve_admin_prompt(false, true),
            ("colour-admin", "primary") => {
                self.colour_admin_prompt_open = false;
                if crate::spoofer::spawn_elevated_relaunch() {
                    self.spoofer_mgr.shutdown();
                    self.workshop.suspend_multiplayer();
                    std::process::exit(0);
                }
                self.console
                    .write("[Colours] Hebnix could not relaunch as Administrator.");
            }
            ("colour-admin", "dismiss") => self.colour_admin_prompt_open = false,
            ("close-game", "primary") => {
                self.item_action_prompt_open = false;
                if let Err(error) = crate::winutil::kill_rocket_league() {
                    self.console
                        .write(format!("[Patcher] Could not close Rocket League: {error}"));
                }
            }
            ("close-game", "dismiss") => self.item_action_prompt_open = false,
            ("clear-items", "primary") => self.check_item_clear(),
            ("clear-items", "dismiss") => {
                if self.spoofer_mgr.item_status().can_cancel {
                    self.item_clear_error =
                        self.spoofer_mgr.cancel_item_clear().err().map(|error| {
                            format!("Could not cancel durably: {error}. Retry Cancel.")
                        });
                } else {
                    self.item_clear_prompt_dismissed = true;
                }
            }
            ("certificate", "primary") => self.install_spoofer_certificate(),
            ("certificate", "dismiss") => self.spoofer_certificate_prompt_open = false,
            ("restart-game", "primary") => {
                let path = std::path::Path::new(&self.config.settings.rl_path);
                let _ = crate::winutil::restart_rocket_league(&self.config.rl_launch, path);
                self.restart_notice = false;
            }
            ("restart-game", "dismiss") => self.restart_notice = false,
            ("fullscreen", "on" | "off") => {
                self.config.settings.suppress_fullscreen_warning = key == "on";
                self.save_config();
            }
            ("fullscreen", "dismiss") => {
                self.fullscreen_notice = false;
                self.fullscreen_notice_dismissed = true;
            }
            ("launch-path", "dismiss") => self.launch_path_notice = false,
            ("epic-repair", "primary") => {
                self.epic_repair.run(ctx);
                self.spoofer_mgr.shutdown();
            }
            ("epic-repair", "dismiss") => self.epic_repair.confirm = false,
            ("epic-repair-result", "dismiss") => self.epic_repair.result = None,
            ("install-plugin", "primary") => {
                self.install_modal.open = false;
                self.tab = Tab::Plugins;
                self.plugins_subtab = PluginsSubTab::Browse;
                self.fetch_hebnix_plugin();
            }
            ("install-plugin", "zip") => {
                let picker =
                    rfd::FileDialog::new().add_filter(t("install-modal-plugin-archives"), &["zip"]);
                if let Some(file) = crate::winutil::parent_file_dialog(picker).pick_file() {
                    match super::install_zip(&file, &self.plugin_dir) {
                        Ok(()) => {
                            self.plugin_mgr.refresh(&mut self.config, true);
                            self.save_config();
                            self.install_modal.open = false;
                        }
                        Err(error) => self.console.write(format!("[Plugins] {error}")),
                    };
                }
            }
            ("install-plugin", "dismiss") => self.install_modal.open = false,
            ("clock", "dismiss") => self.req_clock_warning_open = false,
            ("lan-join", "primary" | "dismiss") => self.workshop.web_resolve_join_offer(
                key == "primary",
                &self.config.settings.rl_path,
                ctx,
            ),
            ("lan-install", "primary" | "dismiss") => {
                self.workshop
                    .web_resolve_peer(key == "primary", &self.config.settings.rl_path, ctx)
            }
            ("changelog", "dismiss") => {
                self.changelog_popup = None;
                let _ = std::fs::remove_file(self.base_dir.join(".first"));
            }
            ("notification", "primary") => {
                if let Some(url) = self
                    .notification_popup
                    .as_ref()
                    .and_then(|notification| notification.link_url())
                {
                    let _ = open::that(url.as_str());
                }
            }
            ("notification", "dismiss") => {
                if let Some(notification) = self.notification_popup.take() {
                    self.config.notification_id = self.config.notification_id.max(notification.id);
                    self.save_config();
                }
            }
            _ => {}
        }
    }
}
