use super::{
    SwapCategory, SwapPaint, SwapperState, browser::Preview, is_universal_car, item_identity,
    item_label, swap_compatible, t,
};
use base64::Engine;
use serde_json::{Value, json};
use std::{collections::HashSet, path::Path};

#[derive(serde::Deserialize)]
pub(crate) struct WebSwap {
    pub source: String,
    pub target: String,
    pub owned_only: bool,
    pub match_car: bool,
    pub paint: usize,
    pub custom_colour: [u8; 3],
    pub speed: f32,
}

pub(crate) fn web_category(index: usize) -> Option<SwapCategory> {
    SwapCategory::ALL.get(index).copied()
}

impl SwapperState {
    pub(crate) fn web_recolour_category(&mut self, next: Option<usize>) -> SwapCategory {
        if let Some(category) = next.and_then(web_category).filter(|category| category.supports_recolour()) {
            if self.recolour_category != category {
                self.recolour_category = category;
                self.recolour_status = None;
            }
        }
        self.recolour_category
    }

    pub(crate) fn web_recolour_catalog(
        &mut self, ctx: &eframe::egui::Context, cooked: &Path, owned: &HashSet<i64>,
        paints: &crate::owned_inventory::OwnedPaints, previews: &[String],
    ) -> Value {
        let category = self.recolour_category;
        let mut model = self.web_catalog(ctx, category, cooked, owned, previews);
        let Some(items) = self.catalogs.get(&category) else { return model };
        let by_identity: std::collections::HashMap<_, _> = items.iter().filter(|item| super::can_recolour_owned(category, item, owned))
            .map(|item| (item_identity(item), item)).collect();
        if let Some(rows) = model["items"].as_array_mut() {
            rows.retain_mut(|row| {
                let Some(item) = row["id"].as_str().and_then(|id| by_identity.get(id)) else { return false; };
                let active = self.active.iter().find(|active| active.category == category.slug() && active.target_matches(item));
                let colour = active.map_or(SwapPaint::Default, |active| active.paint);
                let factory: Vec<_> = item.product_id.and_then(|id| paints.get(&id)).into_iter().flatten()
                    .map(|&id| crate::patcher::painted_swap::factory_paint_label(id)).collect();
                row["active"] = json!(active.is_some());
                row["recoloured"] = json!(colour != SwapPaint::Default);
                row["painted"] = json!(!factory.is_empty());
                row["factory"] = json!(factory);
                row["paint"] = json!(SwapPaint::ALL.iter().position(|item| *item == colour)
                    .or_else(|| matches!(colour, SwapPaint::Custom(_)).then_some(SwapPaint::ALL.len() - 1)).unwrap_or(0));
                row["rgb"] = json!(colour.rgb());
                true
            });
        }
        model
    }

    pub(crate) fn web_recolour_status(&self) -> Value {
        match &self.recolour_status {
            Some(Ok(message)) => json!({ "message": message, "error": false }),
            Some(Err(message)) => json!({ "message": message, "error": true }),
            None => Value::Null,
        }
    }

    pub(crate) fn web_apply_recolour(
        &mut self, cooked: &Path, ids: Vec<String>, owned: &HashSet<i64>, paint: usize,
        custom: [u8; 3], tx: &crossbeam_channel::Sender<crate::messages::AppMsg>,
    ) {
        use crate::messages::AppMsg;
        if crate::messages::block_item_action_if_game_running(tx) { return; }
        let Some(mut paint) = SwapPaint::ALL.get(paint).copied() else { return };
        if matches!(paint, SwapPaint::Custom(_)) { paint = SwapPaint::Custom(custom); }
        let category = self.recolour_category;
        let backups = cooked.join("Backups");
        self.load_active(&backups);
        let requested: HashSet<_> = ids.into_iter().collect();
        let Some(items) = self.catalogs.get(&category) else { return };
        let targets: Vec<_> = items.iter().filter(|item| requested.contains(&item_identity(item))
            && super::can_recolour_owned(category, item, owned)
            && (paint != SwapPaint::Default || self.active.iter().any(|active| active.category == category.slug() && active.target_matches(item))))
            .cloned().collect();
        if targets.is_empty() { return; }
        let mut successes = 0;
        let mut failures = Vec::new();
        for item in &targets {
            let label = item_label(category, item);
            match self.apply_recolour(category, item, paint, owned, cooked, &backups) {
                Ok(()) => {
                    successes += 1;
                    let _ = tx.send(AppMsg::Log(format!("[Recolour] {label}: {}", paint.description())));
                }
                Err(error) => {
                    let error = format!("{label}: {error}");
                    let _ = tx.send(AppMsg::Log(format!("[Recolour] {error}")));
                    failures.push(error);
                }
            }
        }
        let summary = super::t_args("recolour-multi-result", &[("count", (successes as i64).into()), ("total", (targets.len() as i64).into())]);
        self.recolour_status = Some(if failures.is_empty() { Ok(summary) } else {
            failures.insert(0, summary);
            Err(failures.join("\n"))
        });
    }
    pub(crate) fn web_loadout_preview(
        &mut self, ctx: &eframe::egui::Context, category: SwapCategory, cooked: &Path, id: i64,
    ) -> Option<String> {
        let item = self.catalogs.get(&category)?.iter().find(|item| item.product_id == Some(id))?.clone();
        let filename = super::preview_thumbnail(category, &item, cooked)?;
        self.collect_thumbnails();
        self.queue_thumbnail_for_context(ctx, category, &filename, cooked, &item);
        let resolved = super::ResolvedItem { thumbnail: Some(filename), available: true };
        match self.browser_preview(category, &item, &resolved) {
            Preview::Ready(bytes) => Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes.as_ref()))),
            _ => None,
        }
    }

    pub(crate) fn web_catalog(
        &mut self,
        ctx: &eframe::egui::Context,
        category: SwapCategory,
        cooked: &Path,
        owned: &HashSet<i64>,
        preview_ids: &[String],
    ) -> Value {
        let Some(items) = self.catalogs.get(&category).cloned() else {
            return json!({
                "status": self.errors.get(&category).map(String::as_str).unwrap_or("Catalog could not be loaded"),
                "failed": true,
            });
        };
        let Some(resolved) = self.resolve_items(ctx, category, cooked, &items) else {
            return json!({ "status": t("resolved-items-preparing-items") });
        };
        self.load_active(&cooked.join("Backups"));
        self.collect_thumbnails();
        let mut previews = serde_json::Map::new();
        let rows: Vec<_> = items.iter().zip(resolved.iter()).filter(|(_, resolved)| resolved.available)
            .map(|(item, resolved)| {
                let id = item_identity(item);
                if preview_ids.contains(&id) {
                    if let Some(filename) = resolved.thumbnail.as_deref() {
                        self.queue_thumbnail_for_context(ctx, category, filename, cooked, item);
                    }
                    let preview = match self.browser_preview(category, item, resolved) {
                        Preview::Ready(bytes) => json!({ "image": format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes.as_ref())) }),
                        Preview::Loading => json!({ "status": t("items-browser-preview-loading") }),
                        Preview::Missing => json!({ "status": t("items-browser-preview-missing") }),
                        Preview::Failed(error) => json!({ "status": t("items-browser-preview-failed"), "error": error }),
                    };
                    previews.insert(id.clone(), preview);
                }
                json!({
                "id": id,
                "label": item_label(category, item),
                "package": item.upk,
                "kind": item.upk_type,
                "car": item.car_key,
                "owned": item.product_id.is_some_and(|id| owned.contains(&id)),
                "applied": self.active.iter().any(|swap| swap.category == category.slug() && swap.source_matches(item)),
                })
            }).collect();
        let cars: Vec<_> = self
            .resolutions
            .get(&category)
            .and_then(|entry| entry.cars.as_deref())
            .into_iter()
            .flatten()
            .map(|(id, label, product)| {
                json!({
                    "id": id,
                    "label": label,
                    "universal": is_universal_car(label),
                    "owned": product.is_some_and(|id| owned.contains(&id)),
                })
            })
            .collect();
        json!({ "items": rows, "cars": cars, "previews": previews })
    }

    pub(crate) fn apply_web_swap(
        &mut self,
        ctx: &eframe::egui::Context,
        category: SwapCategory,
        cooked: &Path,
        owned: &HashSet<i64>,
        request: WebSwap,
    ) -> Result<String, String> {
        let items = self
            .catalogs
            .get(&category)
            .cloned()
            .ok_or("Catalog is unavailable")?;
        let resolved = self
            .resolve_items(ctx, category, cooked, &items)
            .ok_or("Catalog is still loading")?;
        let find = |id: &str| {
            items
                .iter()
                .zip(resolved.iter())
                .find(|(item, info)| info.available && item_identity(item) == id)
                .map(|(item, _)| item)
        };
        let source = find(&request.source).ok_or("Select an available item to use")?;
        let target = find(&request.target).ok_or("Select an available item to replace")?;
        if request.source == request.target
            || !swap_compatible(category, source, target)
            || (category == SwapCategory::Skins
                && request.match_car
                && source.car_key != target.car_key)
            || (request.owned_only && !target.product_id.is_some_and(|id| owned.contains(&id)))
        {
            return Err(
                "The selected items are incompatible or the replacement is unavailable.".into(),
            );
        }
        let paint = if category.supports_recolour() {
            match SwapPaint::ALL
                .get(request.paint)
                .copied()
                .ok_or("Invalid paint")?
            {
                SwapPaint::Custom(_) => SwapPaint::Custom(request.custom_colour),
                paint => paint,
            }
        } else {
            SwapPaint::Default
        };
        let speed = if self.speed_enabled {
            request.speed.clamp(
                crate::speed_patch::MIN_MULTIPLIER,
                crate::speed_patch::MAX_MULTIPLIER,
            )
        } else {
            1.0
        };
        let backups = cooked.join("Backups");
        self.load_active(&backups);
        self.apply_swap(category, source, target, paint, speed, cooked, &backups)?;
        Ok(format!(
            "Applied {} to {}",
            item_label(category, source),
            item_label(category, target)
        ))
    }

    pub(crate) fn web_active_catalog(
        &mut self,
        ctx: &eframe::egui::Context,
        cooked: &Path,
        preview_ids: &[String],
    ) -> Value {
        let backups = cooked.join("Backups");
        self.load_active(&backups);
        self.collect_thumbnails();
        let mut previews = serde_json::Map::new();
        let items: Vec<_> = self.active.clone().into_iter().map(|swap| {
            let id = swap.key();
            if preview_ids.contains(&id) {
                let category = SwapCategory::from_slug(&swap.category);
                let item = category.and_then(|category| self.catalogs.get(&category))
                    .and_then(|catalog| catalog.iter().find(|item| swap.source_matches(item))).cloned();
                let preview = match (category, item) {
                    (Some(category), Some(item)) => {
                        let thumbnail = super::preview_thumbnail(category, &item, cooked);
                        if let Some(filename) = thumbnail.as_deref() {
                            self.queue_thumbnail_for_context(ctx, category, filename, cooked, &item);
                        }
                        self.browser_preview(category, &item, &super::ResolvedItem { thumbnail, available: true })
                    }
                    _ => Preview::Missing,
                };
                previews.insert(id.clone(), match preview {
                    Preview::Ready(bytes) => json!({ "image": format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes.as_ref())) }),
                    Preview::Loading => json!({ "status": t("items-browser-preview-loading") }),
                    Preview::Missing => json!({ "status": t("items-browser-preview-missing") }),
                    Preview::Failed(error) => json!({ "status": t("items-browser-preview-failed"), "error": error }),
                });
            }
            json!({
                "id": id, "label": swap.source_name, "target": swap.target_name,
                "package": swap.source_upk, "applied": true,
                "paint": (swap.paint != SwapPaint::Default).then(|| swap.paint.description()),
            })
        }).collect();
        json!({ "items": items, "previews": previews })
    }

    pub(crate) fn restore_web_active(
        &mut self,
        cooked: &Path,
        key: &str,
    ) -> Result<String, String> {
        let backups = cooked.join("Backups");
        self.load_active(&backups);
        let swap = self
            .active
            .iter()
            .find(|swap| swap.key() == key)
            .ok_or("The swap is no longer active")?;
        let name = swap.target_name.clone();
        self.restore_swap(key, cooked, &backups)?;
        Ok(format!("Restored {name}"))
    }
}
