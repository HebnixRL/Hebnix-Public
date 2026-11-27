use super::*;

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum WebAction {
    DismissMessage,
    StockEngine { team: usize, product: i32 },
    Creator { action: super::creator::Action },
    Previews {
        positions: Vec<usize>,
    },
    Account {
        index: Option<usize>,
    },
    Import {
        code: String,
    },
    Plan {
        base: usize,
        target: usize,
        replace: bool,
        name: String,
    },
    SaveImport {
        apply: bool,
    },
    CancelImport,
    Copy {
        position: usize,
        bakkes: bool,
    },
    Open {
        position: usize,
        kind: String,
    },
    Close,
    Team {
        team: usize,
    },
    Colour {
        team: usize,
        primary: bool,
        index: u8,
    },
    Item {
        team: usize,
        slot: u8,
        option: usize,
    },
    Confirm {
        name: Option<String>,
        destination: Option<usize>,
    },
}

impl WebAction {
    pub(crate) fn refreshes_inventory(&self) -> bool {
        matches!(self, Self::Confirm { .. } | Self::SaveImport { .. } | Self::Import { .. }
            | Self::Creator { action: super::creator::Action::Finish { .. } })
    }
}

pub(super) const SLOTS: &[u8] = &[0, 1, 2, 3, 4, 5, 13, 14, 15];

pub(super) fn category(slot: u8) -> Option<SwapCategory> {
    if slot == 0 {
        Some(SwapCategory::Bodies)
    } else {
        category_for_equip_slot(slot)
    }
}

impl LoadoutsPanel {
    pub(crate) fn sync_global_save(&mut self, path: Option<&Path>, refresh: bool) {
        let initial = !self.loaded;
        if initial { self.loaded = true; self.refresh_accounts(); }
        let changed = path.is_some_and(|path| self.save.as_deref() != Some(path));
        if changed {
            self.save = path.map(Path::to_path_buf);
            self.creator = None;
            self.dialog = None; self.code = None; self.mode = SaveMode::New;
            self.base = 0; self.target = 0;
        }
        if refresh { self.refresh_accounts(); if let Some(path) = path { self.save = Some(path.to_path_buf()); } }
        if initial || changed || refresh { self.reload(); }
    }

    pub(crate) fn web_snapshot(
        &mut self,
        swapper: &mut SwapperState,
        catalog_loading: bool,
        cooked: &Path,
        ctx: &egui::Context,
    ) -> Value {
        if !self.loaded {
            self.loaded = true;
            self.refresh_accounts();
            self.reload();
        }
        self.web_plan(swapper);
        let account = self
            .accounts
            .iter()
            .position(|account| Some(&account.path) == self.save.as_ref());
        let rows: Vec<_> = self.names.iter().zip(&self.bodies).enumerate().map(|(position, (name, body))| {
            let car = swapper.product_name(SwapCategory::Bodies, *body).unwrap_or_else(|| {
                t(if catalog_loading { "app-loading-item-catalogs" } else { "loadouts-unknown-car" })
            });
            let image = if self.web_previews.contains(&position) {
                swapper.web_loadout_preview(ctx, SwapCategory::Bodies, cooked, *body)
            } else { None };
            json!({ "position": position, "name": name, "body": body, "car": car, "image": image })
        }).collect();
        let plan = self.plan.as_ref().map(|(_, plan)| match plan {
            Ok(plan) => {
                let rows: Vec<_> = plan.rows.iter().map(|row| {
                    let you = row.skipped.as_ref().map_or_else(|| row.you.clone(), |reason| format!("{} ({reason})", row.you));
                    json!([slot_name(row.slot), row.others, you, row.affects.join(", ")])
                }).collect();
                json!({ "rows": rows, "name": plan.name,
                    "note": t_args("loadouts-bakkes-plan-note", &[("base", self.names[plan.base].as_str().into())]) })
            }
            Err(error) => json!({ "error": error }),
        });
        let dialog = self.web_dialog(swapper, cooked, ctx);
        let mut labels: serde_json::Map<String, Value> = [
            ("description", "loadouts-description"),
            ("save", "loadouts-save"),
            ("refresh", "loadouts-refresh-saves"),
            ("noSave", "loadouts-no-save"),
            ("select", "loadouts-select-save"),
            ("import", "loadouts-import-title"),
            ("hint", "loadouts-import-hint"),
            ("closed", "loadouts-close-game"),
            ("cars", "loadouts-cars"),
            ("list", "loadouts-list-title"),
            ("search", "loadouts-search"),
            ("copy", "loadouts-copy-code"),
            ("lds", "loadouts-format-lds"),
            ("bakkes", "loadouts-format-bakkes"),
            ("edit", "loadouts-edit"),
            ("move", "loadouts-move"),
            ("delete", "loadouts-delete"),
            ("confirm", "loadouts-save-changes"),
            ("cancel", "loadouts-cancel"),
            ("position", "loadouts-move-position"),
            ("blue", "loadouts-team-blue"),
            ("orange", "loadouts-team-orange"),
            ("owned", "loadouts-owned-only"),
            ("primary", "loadouts-primary-colour"),
            ("secondary", "loadouts-secondary-colour"),
            ("itemSearch", "loadouts-search-items"),
            ("base", "loadouts-bakkes-base"),
            ("baseHint", "loadouts-bakkes-base-hint"),
            ("name", "loadouts-bakkes-name"),
            ("target", "loadouts-bakkes-target"),
            ("new", "loadouts-bakkes-mode-new"),
            ("replace", "loadouts-bakkes-mode-replace"),
            ("apply", "loadouts-import-apply"),
            ("importOnly", "loadouts-import-only"),
            ("empty", "loadouts-empty"),
        ]
        .into_iter()
        .map(|(key, message)| (key.to_owned(), Value::String(t(message))))
        .collect();
        labels.insert(
            "plan".into(),
            Value::Array(
                [
                    "loadouts-bakkes-col-slot",
                    "loadouts-bakkes-col-others",
                    "loadouts-bakkes-col-you",
                    "loadouts-bakkes-col-affects",
                ]
                .into_iter()
                .map(|message| Value::String(t(message)))
                .collect(),
            ),
        );
        json!({
            "page": "loadouts", "rows": rows, "account": account,
            "preview_positions": self.web_previews, "creatorAttempt": self.creator_attempt,
            "accounts": self.accounts.iter().map(|account| &account.display_name).collect::<Vec<_>>(),
            "closed": !hebnix_sdk::process::is_rocket_league_running(),
            "input": self.input, "message": self.message, "dialog": dialog,
            "import": self.code.as_ref().map(|code| {
                let mut summary = t_args("loadouts-bakkes-summary", &[("count", code.blue.items.len().to_string().into())]);
                if code.blue.colors.is_some() {
                    summary.push_str(&format!(" · {}", t("loadouts-bakkes-colours")));
                }
                json!({ "base": self.base, "target": self.target, "replace": self.mode == SaveMode::Replace,
                    "name": self.name, "summary": summary, "preview": plan })
            }),
            "labels": labels,
        })
    }

    fn web_plan(&mut self, swapper: &SwapperState) {
        let (Some(save), Some(code), Some(last)) = (
            self.save.as_deref(),
            self.code.as_ref(),
            self.names.len().checked_sub(1),
        ) else {
            return;
        };
        self.base = self.base.min(last);
        self.target = self.target.min(last);
        let key = (self.base, self.mode, self.target, self.name.clone());
        if self.plan.as_ref().is_none_or(|(built, _)| *built != key) {
            let plan = build_plan(save, code, swapper, &self.names, key.clone());
            self.plan = Some((key, plan));
        }
    }

    fn web_dialog(
        &mut self,
        swapper: &mut SwapperState,
        cooked: &Path,
        ctx: &egui::Context,
    ) -> Option<Value> {
        let (position, dialog) = self.dialog.as_mut()?;
        let current = self.names.get(*position)?;
        Some(match dialog {
            LoadoutDialog::Delete => json!({ "kind": "delete", "position": position,
                "message": t_args("loadouts-delete-confirm", &[("name", current.as_str().into())]) }),
            LoadoutDialog::Move(target, _) => {
                json!({ "kind": "move", "position": position, "name": current, "target": *target - 1 })
            }
            LoadoutDialog::Edit {
                name,
                preset,
                owned,
                team,
                changes,
                palettes,
                ..
            } => {
                if let PaletteLoad::Loading(job) = palettes {
                    match job.try_recv() {
                        Ok(result) => *palettes = PaletteLoad::Ready(result),
                        Err(TryRecvError::Disconnected) => {
                            *palettes = PaletteLoad::Ready(Err(
                                "Could not load the car colour palettes.".into(),
                            ))
                        }
                        Err(TryRecvError::Empty) => {}
                    }
                }
                let paint = &preset.teams[*team];
                let palette = match palettes {
                    PaletteLoad::Loading(_) => json!({ "loading": true }),
                    PaletteLoad::Ready(Err(error)) => json!({ "error": error }),
                    PaletteLoad::Ready(Ok(palettes)) => {
                        json!({ "primary": palettes[*team], "secondary": palettes[2] })
                    }
                };
                let slots: Vec<_> = SLOTS.iter().filter_map(|&slot| {
                    let category = category(slot)?;
                    let product = paint.products[slot as usize];
                    let selected = changes.iter().find(|(t, s, _, _)| *t == *team && *s == slot as usize)
                        .map(|(_, _, upper, lower)| (*upper, *lower))
                        .or_else(|| paint.items[slot as usize].and_then(|index| preset.items.get(index))
                            .map(|item| (item.instance_upper, item.instance_lower)));
                    let mut choices: Vec<_> = owned.iter().enumerate().filter_map(|(index, &(id, upper, lower, colour))| {
                        if !swapper.loadout_item_compatible(category, id as i64, paint.products[0] as i64) { return None; }
                        let mut label = swapper.product_name(category, id as i64)?;
                        if colour > 0 {
                            label.push_str(&format!(" ({})", crate::patcher::painted_swap::factory_paint_label(colour)));
                        }
                        Some((label, index, selected == Some((upper, lower))))
                    }).collect();
                    choices.sort_unstable();
                    let label = choices.iter().find(|(_, _, selected)| *selected).map(|(label, _, _)| label.clone())
                        .or_else(|| swapper.product_name(category, product as i64)).unwrap_or_else(|| if product==0 {if slot==13 {"Default".into()} else {"None".into()}} else {format!("#{product}")});
                    let image = swapper.web_loadout_preview(ctx, category, cooked, product as i64);
                    let engines:Vec<_>=if slot==13 {swapper.owned_engine_audio_ids().iter().filter_map(|&id|Some(json!({"product":i32::try_from(id).ok()?,"label":swapper.product_name(category,id)?}))).collect()} else {Vec::new()};
                    Some(json!({ "engines":engines,"slot": slot, "name": slot_name(slot), "current": label, "choices": choices, "image": image, "product": product }))
                }).collect();
                json!({ "kind": "edit", "position": position, "name": name, "team": team, "slots": slots, "palette": palette,
                    "primary": paint.team_color, "secondary": paint.custom_color })
            }
        })
    }

    pub(crate) fn web_action(
        &mut self,
        action: WebAction,
        swapper: &mut SwapperState,
        cooked: &Path,
        presets: &mut PresetStore,
        ctx: &egui::Context,
    ) -> Option<String> {
        let closed = !hebnix_sdk::process::is_rocket_league_running();
        match action {
            WebAction::DismissMessage => self.message.clear(),
            WebAction::StockEngine {team,product} if team<2 && swapper.owned_engine_audio_ids().contains(&(product as i64)) => {
                if let Some((_,LoadoutDialog::Edit{preset,changes,..}))=&mut self.dialog {
                    preset.teams[team].products[13]=product;preset.teams[team].items[13]=None;
                    changes.retain(|(t,s,_,_)| *t!=team || *s!=13);
                }
            }
            WebAction::Creator { action } => return self.creator_action(action, swapper, cooked, presets, ctx),
            WebAction::Previews { positions } => {
                self.web_previews = positions.into_iter().take(96).collect()
            }
            WebAction::Account { index } => {
                let before = self.save.clone();
                if let Some(index) = index {
                    self.save = Some(self.accounts.get(index)?.path.clone());
                } else {
                    self.refresh_accounts();
                }
                if self.save != before {
                    self.dialog = None;
                    self.code = None;
                    self.mode = SaveMode::New;
                    self.base = 0;
                    self.target = 0;
                }
                self.message.clear();
                self.reload();
            }
            WebAction::Import { code } if closed => {
                self.input = code;
                self.read_input();
            }
            WebAction::Plan {
                base,
                target,
                replace,
                name,
            } => {
                self.base = base;
                self.target = target;
                self.mode = if replace {
                    SaveMode::Replace
                } else {
                    SaveMode::New
                };
                self.name = name;
            }
            WebAction::SaveImport { apply } if closed => {
                self.web_plan(swapper);
                let save = self.save.as_deref()?;
                let (_, Ok(plan)) = self.plan.take()? else {
                    return None;
                };
                match save_plan(save, cooked, &plan, presets) {
                    Ok(mut message) => {
                        if !apply {
                            message.push(' ');
                            message.push_str(&t("loadouts-bakkes-apply-hint"));
                        }
                        self.message = message;
                        self.reload();
                        self.code = None;
                        self.input.clear();
                        return apply.then_some(plan.name);
                    }
                    Err(error) => self.message = error,
                }
            }
            WebAction::CancelImport => {
                self.code = None;
                self.plan = None;
            }
            WebAction::Copy { position, bakkes } if position < self.names.len() => {
                self.copy_code(ctx, position, bakkes)
            }
            WebAction::Open { position, kind } if closed && position < self.names.len() => {
                let save = self.save.as_deref()?;
                let dialog = match kind.as_str() {
                    "delete" => Ok(LoadoutDialog::Delete),
                    "move" => {
                        loadout_order(save).map(|order| LoadoutDialog::Move(position + 1, order))
                    }
                    "edit" => loadout_preset(save, position).and_then(|preset| {
                        let owned = loadout_owned_instances(save)?;
                        let (send, receive) = mpsc::channel();
                        let cooked = cooked.to_owned();
                        let ctx = ctx.clone();
                        std::thread::spawn(move || {
                            let _ = send.send(crate::patcher::colours::loadout_palettes(&cooked));
                            ctx.request_repaint();
                        });
                        Ok(LoadoutDialog::Edit {
                            name: self.names[position].clone(),
                            expected: preset.clone(),
                            preset,
                            owned,
                            team: 0,
                            search: String::new(),
                            changes: Vec::new(),
                            palettes: PaletteLoad::Loading(receive),
                        })
                    }),
                    _ => return None,
                };
                match dialog {
                    Ok(dialog) => self.dialog = Some((position, dialog)),
                    Err(error) => self.message = error,
                }
            }
            WebAction::Close => self.dialog = None,
            WebAction::Team { team } if team < 2 => {
                if let Some((_, LoadoutDialog::Edit { team: current, .. })) = self.dialog.as_mut() {
                    *current = team;
                }
            }
            WebAction::Colour {
                team,
                primary,
                index,
            } if team < 2 => {
                if let Some((
                    _,
                    LoadoutDialog::Edit {
                        preset,
                        palettes: PaletteLoad::Ready(Ok(palettes)),
                        ..
                    },
                )) = self.dialog.as_mut()
                    && usize::from(index) < palettes[if primary { team } else { 2 }].len()
                {
                    if primary {
                        preset.teams[team].team_color = index;
                    } else {
                        preset.teams[team].custom_color = index;
                    }
                }
            }
            WebAction::Item { team, slot, option } if team < 2 && SLOTS.contains(&slot) => {
                if let Some((
                    _,
                    LoadoutDialog::Edit {
                        preset,
                        owned,
                        changes,
                        ..
                    },
                )) = self.dialog.as_mut()
                    && let Some(&(id, upper, lower, _)) = owned.get(option)
                    && let Some(category) = category(slot)
                    && swapper.loadout_item_compatible(
                        category,
                        id as i64,
                        preset.teams[team].products[0] as i64,
                    )
                    && swapper.product_name(category, id as i64).is_some()
                {
                    preset.teams[team].products[slot as usize] = id;
                    changes.retain(|(t, s, _, _)| *t != team || *s != slot as usize);
                    changes.push((team, slot as usize, upper, lower));
                }
            }
            WebAction::Confirm { name, destination } if closed => {
                let (position, dialog) = self.dialog.as_ref()?;
                let position = *position;
                let edit = match dialog {
                    LoadoutDialog::Delete => Edit::Delete { position },
                    LoadoutDialog::Move(_, expected) => Edit::Move {
                        from: position,
                        to: destination.filter(|index| *index < self.names.len())?,
                        expected: expected.clone(),
                    },
                    LoadoutDialog::Edit {
                        preset,
                        expected,
                        changes,
                        ..
                    } => Edit::Update {
                        position,
                        name: name
                            .map(|name| name.trim().to_owned())
                            .filter(|name| !name.is_empty())?,
                        stock_engines: std::array::from_fn(|team| (preset.teams[team].products[13]!=expected.teams[team].products[13] && preset.teams[team].items[13].is_none() && !changes.iter().any(|(t,s,_,_)| *t==team && *s==13)).then_some(preset.teams[team].products[13])),
                        expected: expected.clone(),
                        changes: changes.clone(),
                        colours: preset
                            .teams
                            .each_ref()
                            .map(|team| [team.team_color, team.custom_color]),
                    },
                };
                let save = self.save.as_deref()?;
                let result = match loadout_summaries(save) {
                    Ok(current)
                        if current
                            .iter()
                            .map(|(name, body)| (name, body))
                            .eq(self.names.iter().zip(&self.bodies)) =>
                    {
                        apply_loadout_edit(save, &edit)
                    }
                    Ok(_) => Err(t("loadouts-save-changed")),
                    Err(error) => Err(error),
                };
                self.dialog = None;
                self.finish(result);
            }
            _ => {}
        }
        None
    }
}
