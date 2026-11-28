use super::*;
use super::web::{SLOTS, category};
use std::collections::{BTreeMap, HashSet};

const REQUIRED_SLOTS: &[u8] = &[0, 2, 3, 14, 15];

pub(super) struct Creator {
    preset: loadout_code::Preset,
    owned: Vec<(i32, u64, u64, i64)>,
    swaps: BTreeMap<(usize, u8), i64>,
    palettes: PaletteLoad,
    slot: u8,
    team: usize,
    previews: Vec<String>,
    inherited: HashSet<(usize,usize,i32)>,
    stock_engines: Vec<i32>,
    review: bool,
    affected: Option<Vec<Value>>,
}
impl Creator {
    fn equipped(&self, team: usize, slot: u8) -> i64 {
        self.preset.teams[team].products[slot as usize] as i64
    }

    fn desired(&self, team: usize, slot: u8) -> i64 {
        self.swaps.get(&(team, slot)).copied().unwrap_or_else(|| self.equipped(team, slot))
    }

    fn missing_required(&self) -> Vec<u8> {
        REQUIRED_SLOTS.iter().copied().filter(|&slot| {
            (0..self.preset.teams.len()).any(|team| self.equipped(team, slot) <= 0)
        }).collect()
    }
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Action {
    Begin { copy: Option<usize> }, Cancel,
    Review,
    Slot { slot: u8 }, Team { team: usize },
    Previews { ids: Vec<String> },
    Pick { product: i64, replacement: Option<usize> },
    Colour { team: usize, primary: bool, index: u8 },
    Finish { name: String },
}
impl LoadoutsPanel {
    pub(super) fn creator_use_palette_colour(&mut self, team: usize, primary: bool, index: u8, rgb: [u8; 3]) {
        let Some(draft) = self.creator.as_mut() else { return };
        let PaletteLoad::Ready(Ok(palettes)) = &mut draft.palettes else { return };
        super::show_palette_cell(palettes, team, primary, index, rgb);
        if primary { draft.preset.teams[team].team_color = index; } else { draft.preset.teams[team].custom_color = index; }
    }

    pub(crate) fn creator_snapshot(&mut self, swapper: &mut SwapperState, cooked: &Path, ctx: &egui::Context) -> Value {
        let Some(draft) = self.creator.as_mut() else { return Value::Null };
        if let PaletteLoad::Loading(job) = &draft.palettes {
            match job.try_recv() {
                Ok(value) => draft.palettes = PaletteLoad::Ready(value),
                Err(TryRecvError::Disconnected) => draft.palettes = PaletteLoad::Ready(Err("Could not load colours".into())),
                Err(TryRecvError::Empty) => (),
            }
        }
        draft.stock_engines.extend(swapper.owned_engine_audio_ids().iter().filter_map(|&id|i32::try_from(id).ok()));
        draft.stock_engines.sort_unstable();draft.stock_engines.dedup();
        let team = draft.team;
        let equipped_body = draft.equipped(team, 0);
        let desired_body = draft.desired(team, 0);
        let cat = category(draft.slot).unwrap();
        let mut owned_ids: HashSet<_> = draft.owned.iter().map(|item| item.0 as i64).collect();
        if cat==SwapCategory::Engines { owned_ids.extend(draft.stock_engines.iter().map(|&id|id as i64)); }
        if cat == SwapCategory::Skins {
            owned_ids.retain(|&id| swapper.loadout_item_compatible(cat, id, equipped_body));
        }
        let mut catalog = swapper.web_catalog(ctx, cat, cooked, &owned_ids, &draft.previews);
        if let Some(items) = catalog["items"].as_array_mut() {
            items.retain(|item| item["product"].as_i64().is_some_and(|id| swapper.loadout_item_compatible(cat, id, desired_body)));
        }
        let mut replacements: Vec<_> = draft.owned.iter().enumerate().filter_map(|(index, &(id, _, _, paint))| {
            if !swapper.loadout_item_compatible(cat, id as i64, equipped_body) { return None }
            let mut label = swapper.product_name(cat, id as i64)?;
            if paint > 0 { label.push_str(&format!(" ({})", crate::patcher::painted_swap::factory_paint_label(paint))); }
            Some(json!({"index":index,"label":label,"product":id}))
        }).collect();
        if cat==SwapCategory::Engines {
            for (index,&id) in draft.stock_engines.iter().enumerate() {
                if let Some(label)=swapper.product_name(cat,id as i64) { replacements.push(json!({"index":draft.owned.len()+index,"label":label,"product":id})); }
            }
        }
        replacements.sort_by_cached_key(|item| item["label"].as_str().unwrap_or("").to_lowercase());
        let mut seen = HashSet::new();
        replacements.retain(|item| seen.insert((item["product"].as_i64(), item["label"].as_str().unwrap_or("").to_owned())));
        if let Some(items)=catalog["items"].as_array_mut() {
            items.sort_by_cached_key(|item| (cat==SwapCategory::Skins && item["universal"].as_bool().unwrap_or(false),item["label"].as_str().unwrap_or("").to_lowercase()));
        }
        let slots: Vec<_> = SLOTS.iter().map(|&slot| {
            let equipped = draft.equipped(team, slot);
            let wanted = draft.desired(team, slot);
            json!({"slot":slot,"name":slot_name(slot),"product":wanted,"equipped":equipped,
                "equipped_label":category(slot).and_then(|cat| swapper.product_name(cat,equipped)),
                "label":category(slot).and_then(|cat| swapper.product_name(cat,wanted)).unwrap_or_else(|| if slot==13 {"Default".into()} else {"None".into()})})
        }).collect();
        let palette = match &draft.palettes {
            PaletteLoad::Loading(_) => json!({"loading":true}),
            PaletteLoad::Ready(Err(error)) => json!({"error":error}),
            PaletteLoad::Ready(Ok(p)) => json!({"primary":p[team],"secondary":p[2]}),
        };
        let colours: Vec<_> = (0..2).map(|team| {
            let palette=match &draft.palettes {
                PaletteLoad::Ready(Ok(p))=>json!({"primary":p[team],"secondary":p[2]}),
                _=>Value::Null,
            };
            json!({"team":team,"palette":palette,"primary":draft.preset.teams[team].team_color,"secondary":draft.preset.teams[team].custom_color})
        }).collect();
        let mut affected=Vec::new();
        if draft.review && draft.affected.is_none() && let Some(save)=self.save.as_deref() {
            for (position,name) in self.names.iter().enumerate() {
                if let Ok(existing)=loadout_preset(save,position) {
                    let mut changes=std::collections::BTreeSet::new();
                    for (&(team,slot),&wanted) in &draft.swaps {
                        let stock=draft.preset.teams[team].products[slot as usize] as i64;
                        for (other_team,paint) in existing.teams.iter().enumerate() {
                            if paint.products[slot as usize] as i64==stock {
                                let cat=category(slot).unwrap();
                                changes.insert(format!("{} · {}: {} → {}",if other_team==0 {"Blue"} else {"Orange"},slot_name(slot),swapper.product_name(cat,stock).unwrap_or_else(||format!("#{stock}")),swapper.product_name(cat,wanted).unwrap_or_else(||format!("#{wanted}"))));
                            }
                        }
                    }
                    if !changes.is_empty() { affected.push(json!({"name":name,"changes":changes})); }
                }
            }
        }
        if draft.review && draft.affected.is_none() { draft.affected=Some(affected); }
        let affected=draft.affected.as_deref().unwrap_or(&[]);
        let missing_required: Vec<_> = draft.missing_required().into_iter().map(slot_name).collect();
        json!({"missing_required":missing_required,"name":draft.preset.name,"slot":draft.slot,"team":team,"slots":slots,"catalog":catalog,
            "replacements":replacements,"palette":palette,"colours":colours,"affected":affected,"primary":draft.preset.teams[team].team_color,
            "secondary":draft.preset.teams[team].custom_color,"preview_ids":draft.previews})
    }
    pub(crate) fn creator_action(&mut self, action: Action, swapper: &mut SwapperState, cooked: &Path, presets: &mut PresetStore, ctx: &egui::Context) -> Option<String> {
        if matches!(&action, Action::Finish {..}) { self.creator_attempt = self.creator_attempt.wrapping_add(1); }
        let result: Result<Option<String>,String> = (|| {
            match action {
                Action::Cancel => self.creator = None,
                Action::Begin { copy } => {
                    if hebnix_sdk::process::is_rocket_league_running() { return Err("Rocket League must be closed.".into()); }
                    let save = self.save.as_deref().ok_or("Select a Rocket League save in quick settings first.")?;
                    let mut preset = loadout_preset(save, copy.unwrap_or(0))?;
                    if copy.is_none() {
                        preset.name.clear();
                        for team in &mut preset.teams {
                            for &slot in SLOTS.iter() {
                                team.products[slot as usize] = 0; team.items[slot as usize] = None;
                            }
                        }
                    } else { preset.name = format!("{} Copy", preset.name); }
                    let mut stock_engines=Vec::new();
                    let full_save=save_file::load(save,false).map_err(|error|error.to_string())?;
                    let instances=loadout_owned_instances(save)?;
                    for item in full_save.inventory() {
                        if item.product_id>0 && item.product_id<=i32::MAX as i64 && !instances.iter().any(|owned|owned.0 as i64==item.product_id) && swapper.product_name(SwapCategory::Engines,item.product_id).is_some() {
                            let blueprint=item.attributes.as_array().into_iter().flatten().any(|attribute| {
                                let attribute=attribute.as_u64().and_then(|index|full_save.objects.get(index as usize)).unwrap_or(attribute);
                                attribute["__type"]=="TAGame.ProductAttribute_Blueprint_TA"
                            });
                            if !blueprint { stock_engines.push(item.product_id as i32); }
                        }
                    }
                    // Stock engine audio may be equipped by product ID without an instance.
                    for position in 0..self.names.len() { if let Ok(existing)=loadout_preset(save,position) { for paint in existing.teams { let id=paint.products[13]; if id>0 && !instances.iter().any(|item|item.0==id) && swapper.product_name(SwapCategory::Engines,id as i64).is_some() {stock_engines.push(id);} } } }
                    stock_engines.extend(swapper.owned_engine_audio_ids().iter().filter_map(|&id|i32::try_from(id).ok()));
                    stock_engines.sort_unstable();stock_engines.dedup();
                    let inherited = preset.teams.iter().enumerate().flat_map(|(team, paint)| paint.products.iter().enumerate().map(move |(slot, &id)| (team,slot,id))).collect();
                    let owned = loadout_owned_instances(save)?;
                    let (send, receive) = mpsc::channel();
                    let cooked = cooked.to_owned(); let ctx = ctx.clone();
                    std::thread::spawn(move || { let _ = send.send(crate::patcher::colours::loadout_palettes(&cooked)); ctx.request_repaint(); });
                    self.creator = Some(Creator { preset, owned, swaps:BTreeMap::new(), palettes:PaletteLoad::Loading(receive), slot:0, team:0, previews:Vec::new(), inherited, stock_engines, review:false, affected:None });
                    self.message.clear();
                }
                Action::Review => {if let Some(draft)=&mut self.creator {draft.review=true;}},
                Action::Slot { slot } if SLOTS.contains(&slot) => { if let Some(draft) = &mut self.creator { if draft.slot != slot { draft.previews.clear(); } draft.slot = slot; } }
                Action::Team { team } if team < 2 => { if let Some(draft) = &mut self.creator { draft.team = team; } }
                Action::Previews { ids } => { if let Some(draft) = &mut self.creator { draft.previews = ids.into_iter().take(72).collect(); } }
                Action::Colour { team, primary, index } if team < 2 => {
                    if let Some(draft) = &mut self.creator {
                        let PaletteLoad::Ready(Ok(palettes)) = &draft.palettes else { return Err("Colours are not ready".into()) };
                        if index as usize >= palettes[if primary {team} else {2}].len() { return Err("Invalid colour".into()) }
                        if primary { draft.preset.teams[team].team_color=index; } else { draft.preset.teams[team].custom_color=index; }
                    }
                }
                Action::Pick { product, replacement } => {
                    let draft = self.creator.as_mut().ok_or("Start a preset first")?;
                    draft.affected=None;
                    let replacement_stock = replacement.and_then(|index| index.checked_sub(draft.owned.len())).and_then(|index|draft.stock_engines.get(index)).copied();
                    let replacement_key = replacement.and_then(|index| draft.owned.get(index).copied());
                    let save = self.save.as_deref().ok_or("Select a save first")?;
                    refresh_inventory(draft, loadout_owned_instances(save)?)?;
                    let replacement = replacement_key.and_then(|key| draft.owned.iter().position(|item| *item == key));
                    let slot = draft.slot; let team = draft.team; let cat = category(slot).unwrap();
                    let equipped_body = draft.equipped(team, 0);
                    let desired_body = draft.desired(team, 0);
                    if cat==SwapCategory::Engines && (draft.stock_engines.contains(&(product as i32)) || replacement_stock.is_some()) {
                        let stock=if draft.stock_engines.contains(&(product as i32)) {product as i32} else {replacement_stock.unwrap()};
                        draft.preset.teams[team].products[slot as usize]=stock;draft.preset.teams[team].items[slot as usize]=None;
                        draft.swaps.remove(&(team,slot));
                        if stock as i64!=product {draft.swaps.insert((team,slot),product);}
                    } else if product == 0 && !REQUIRED_SLOTS.contains(&slot) {
                        draft.preset.teams[team].products[slot as usize]=0; draft.preset.teams[team].items[slot as usize]=None; draft.swaps.remove(&(team,slot));
                    } else {
                        if !swapper.loadout_item_compatible(cat, product, desired_body) || swapper.product_name(cat,product).is_none() { return Err("This item is not compatible with the selected body".into()) }
                        let direct = draft.owned.iter().position(|item| {
                            item.0 as i64 == product
                                && swapper.loadout_item_compatible(cat, product, equipped_body)
                        });
                        let index = replacement.or(direct).ok_or("Choose an owned item to replace")?;
                        let &(id, upper, lower, _) = draft.owned.get(index).ok_or("Owned item is no longer available")?;
                        if !swapper.loadout_item_compatible(cat, id as i64, equipped_body) { return Err("Choose a compatible owned replacement".into()) }
                        let item_index = draft.preset.items.len();
                        draft.preset.items.push(loadout_code::Item { product_id:id,instance_upper:upper,instance_lower:lower,..Default::default() });
                        draft.preset.teams[team].products[slot as usize]=id;
                        draft.preset.teams[team].items[slot as usize]=Some(item_index);
                        draft.swaps.remove(&(team,slot));
                        if id as i64 != product { draft.swaps.insert((team,slot), product); }
                    }
                    draft.inherited.remove(&(team,slot as usize,draft.preset.teams[team].products[slot as usize]));
                    let other = 1 - team;
                    draft.preset.teams[other].products[slot as usize] = draft.preset.teams[team].products[slot as usize];
                    draft.preset.teams[other].items[slot as usize] = draft.preset.teams[team].items[slot as usize];
                    draft.inherited.retain(|&(t,s,_)| !((t==team || t==other) && s==slot as usize));
                    draft.swaps.remove(&(other,slot));
                    if let Some(&wanted)=draft.swaps.get(&(team,slot)) { draft.swaps.insert((other,slot),wanted); }
                    if slot == 0 {
                        for team in 0..draft.preset.teams.len() {
                            let equipped_decal = draft.equipped(team, 1);
                            let desired_decal = draft.desired(team, 1);
                            if equipped_decal != 0 && (
                                !swapper.loadout_item_compatible(SwapCategory::Skins, equipped_decal, draft.equipped(team, 0))
                                || !swapper.loadout_item_compatible(SwapCategory::Skins, desired_decal, draft.desired(team, 0))
                            ) {
                                draft.preset.teams[team].products[1] = 0;
                                draft.preset.teams[team].items[1] = None;
                                draft.swaps.remove(&(team, 1));
                            }
                        }
                    }
                    // Remove unused references so only real, equipped inventory items are imported.
                    let old = draft.preset.items.clone(); let mut compact = Vec::new();
                    for paint in &mut draft.preset.teams { for index in paint.items.iter_mut().flatten() { let item=old[*index].clone(); let next=compact.iter().position(|saved| saved == &item).unwrap_or_else(|| { compact.push(item); compact.len()-1 }); *index=next; } }
                    draft.preset.items=compact;
                }
                Action::Finish { name } => {
                    if hebnix_sdk::process::is_rocket_league_running() { return Err("Close Rocket League before creating a preset".into()) }
                    let save=self.save.as_deref().ok_or("Select a Rocket League save first")?;
                    let draft=self.creator.as_mut().ok_or("Start a preset first")?;
                    let missing = draft.missing_required();
                    if !missing.is_empty() {
                        let names = missing.into_iter().map(slot_name).collect::<Vec<_>>();
                        return Err(format!("Select the required items: {}", names.join(", ")));
                    }
                    let name=name.trim();
                    if name.is_empty() || name.chars().count()>64 { return Err("Enter a name between 1 and 64 characters".into()) }
                    if self.names.iter().any(|n| n.eq_ignore_ascii_case(name)) || presets.presets.iter().any(|p| p.name.eq_ignore_ascii_case(name)) { return Err("That preset name already exists".into()) }
                    if presets.presets.len()>=10 { return Err("The swap preset limit is 10. Remove an unused preset first.".into()) }
                    let current=loadout_owned_instances(save)?;
                    let mut picks=Vec::new(); let mut targets=BTreeMap::new();
                    for (&(team,slot), &wanted) in &draft.swaps {
                        let stock=draft.preset.teams[team].products[slot as usize] as i64;
                        if let Some(previous)=targets.insert((slot,stock),wanted) {
                            if previous != wanted { return Err("The same owned item cannot have different swaps for blue and orange. Choose another replacement.".into()) }
                            continue;
                        }
                        picks.push(SwapPick {category:category(slot).unwrap(),stock_product:stock,product:wanted,paint:0});
                    }
                    refresh_inventory(draft, current)?;
                    let (swaps, skipped)=swapper.bakkes_preset_swaps(&picks);
                    if !skipped.is_empty() || swaps.len()!=picks.len() { return Err(format!("Could not prepare item swaps: {}", skipped.join("; "))) }
                    let mut copy=draft.preset.clone(); copy.name=name.into();
                    if !swaps.is_empty() {
                        let report=swapper.apply_preset_swaps(&swaps,cooked,&cooked.join("Backups"));
                        if !report.skipped.is_empty() || report.applied != swaps.len() {
                            return Err(format!("Loadout was not saved because its item swaps failed: {}",report.skipped.join("; ")));
                        }
                    }
                    let position=self.names.len();
                    apply_loadout_edit(save,&Edit::Import(vec![copy]))?;
                    let preset=crate::presets::Preset {name:name.into(),swaps,..Default::default()};
                    if let Err(error)=presets.save(preset) { apply_loadout_edit(save,&Edit::Delete {position})?; return Err(error) }
                    self.creator=None; self.reload(); self.message="Loadout Created".into();
                    return Ok(None);
                }
                _ => (),
            }
            Ok(None)
        })();
        match result { Ok(apply)=>apply, Err(error)=>{self.message=error;None} }
    }
}

// Existing loadouts can contain legacy instance references absent from inventory.
// Keep their product-only stock slots; never import those stale IDs as new items.
fn refresh_inventory(draft: &mut Creator, current: Vec<(i32,u64,u64,i64)>) -> Result<(),String> {
    let old = draft.preset.items.clone();
    let mut items = Vec::new();
    for (team, paint) in draft.preset.teams.iter_mut().enumerate() {
        for (slot, reference) in paint.items.iter_mut().enumerate() {
            let Some(index) = *reference else { continue };
            let item = old.get(index).ok_or("Invalid loadout item reference")?;
            let found = current.iter().find(|&&(id,upper,lower,_)| id==item.product_id && upper==item.instance_upper && lower==item.instance_lower)
                .or_else(|| current.iter().find(|&&(id,_,_,_)| id==item.product_id));
            let Some(&(id,upper,lower,_)) = found else {
                if draft.inherited.contains(&(team,slot,paint.products[slot])) { *reference=None; continue }
                return Err(format!("{} is no longer owned. Choose another item or replacement.", slot_name(slot as u8)));
            };
            let mut item=item.clone();item.product_id=id;item.instance_upper=upper;item.instance_lower=lower;
            let next=items.iter().position(|saved| saved==&item).unwrap_or_else(||{items.push(item);items.len()-1});
            *reference=Some(next);
        }
    }
    draft.preset.items=items;
    draft.owned=current;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn draft() -> Creator {
        let mut preset=loadout_code::Preset::default();
        preset.items.push(loadout_code::Item {product_id:23,instance_upper:10,instance_lower:20,..Default::default()});
        preset.teams[0].products[0]=23; preset.teams[0].items[0]=Some(0);
        Creator {preset,owned:Vec::new(),swaps:BTreeMap::new(),palettes:PaletteLoad::Ready(Err(String::new())),slot:0,team:0,previews:Vec::new(),inherited:HashSet::new(),stock_engines:Vec::new(),review:false,affected:None}
    }
    #[test]
    fn a_body_swap_keeps_the_equipped_and_displayed_bodies_separate() {
        let mut draft = draft();
        draft.swaps.insert((0, 0), 42);
        assert_eq!(draft.equipped(0, 0), 23);
        assert_eq!(draft.desired(0, 0), 42);
    }

    #[test]
    fn all_required_slots_must_be_equipped_on_both_teams() {
        let mut draft = draft();
        assert_eq!(draft.missing_required(), REQUIRED_SLOTS);
        for team in &mut draft.preset.teams {
            for &slot in REQUIRED_SLOTS {
                team.products[slot as usize] = 23;
            }
        }
        assert!(draft.missing_required().is_empty());
        draft.preset.teams[1].products[14] = 0;
        assert_eq!(draft.missing_required(), vec![14]);
    }

    #[test]
    fn inherited_stale_instances_remain_stock_without_importing_inventory() {
        let mut draft=draft();draft.inherited.insert((0,0,23));
        refresh_inventory(&mut draft,Vec::new()).unwrap();
        assert_eq!(draft.preset.teams[0].products[0],23);
        assert_eq!(draft.preset.teams[0].items[0],None);
        assert!(draft.preset.items.is_empty());
    }
    #[test]
    fn owned_instances_refresh_without_restarting_the_creator() {
        let mut draft=draft();
        refresh_inventory(&mut draft,vec![(23,30,40,0)]).unwrap();
        assert_eq!(draft.preset.items[0].instance_upper,30);
        assert_eq!(draft.preset.items[0].instance_lower,40);
        assert_eq!(draft.owned,vec![(23,30,40,0)]);
    }
    #[test]
    fn a_missing_new_selection_is_not_imported_as_an_owned_item() {
        let mut draft=draft();
        assert!(refresh_inventory(&mut draft,Vec::new()).is_err());
    }
}
