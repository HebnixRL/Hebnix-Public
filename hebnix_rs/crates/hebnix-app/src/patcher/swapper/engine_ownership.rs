use super::*;
use crate::patcher::upk_package::{UpkPackage,strip};
#[derive(Default)]
pub(super) struct State {
    key: String,
    result: Option<Receiver<HashSet<i64>>>,
    pub(super) ids: HashSet<i64>,
}
impl SwapperState {
    pub(crate) fn owned_engine_audio_ids(&self) -> &HashSet<i64> { &self.engine_ownership.ids }
    pub(crate) fn refresh_engine_audio_ownership(&mut self, ctx:&egui::Context,cooked:&Path,owned:&mut HashSet<i64>) {
        let Some(bodies)=self.catalogs.get(&SwapCategory::Bodies) else {return};
        let mut sources:Vec<_>=bodies.iter().filter(|body|body.product_id.is_some_and(|id|owned.contains(&id)))
            .map(|body|(body.product_id.unwrap(),body.upk.clone())).collect();
        sources.sort_unstable();sources.dedup();
        let key=format!("{}:{sources:?}",cooked.display());
        if self.engine_ownership.key!=key {
            self.engine_ownership=State {key,result:None,ids:HashSet::new()};
            let (tx,rx)=unbounded();self.engine_ownership.result=Some(rx);
            let cooked=cooked.to_owned();let context=ctx.clone();
            std::thread::spawn(move || {
                let mut ids=HashSet::new();
                for (_,file) in sources {
                    let backup=cooked.join("Backups").join(format!("{file}.bak"));
                    let path=if backup.is_file(){backup}else{crate::patcher::game_path::resolve(&cooked,&file)};
                    if let Ok(package)=UpkPackage::load(&path) {
                        for export in &package.exports {
                            if strip(&package.class_of(export))!="ProductAsset_Body_TA" {continue}
                            for prop in package.parse_props(export) {
                                if prop.name!="EngineAudioReference" || prop.tag_type!="ObjectProperty" {continue}
                                let Ok(reference)=package.read_int(export.serial_offset+prop.value_offset) else {continue};
                                if reference<=0 {continue}
                                let Some(audio)=package.exports.get(reference as usize-1) else {continue};
                                if let Some(product)=package.parse_props(audio).into_iter().find(|p|p.name=="ProductID" && p.tag_type=="IntProperty") {
                                    if let Ok(id)=package.read_int(audio.serial_offset+product.value_offset) {if id>0 {ids.insert(id as i64);}}
                                }
                            }
                        }
                    }
                }
                let _=tx.send(ids);context.request_repaint();
            });
        }
        if let Some(result)=self.engine_ownership.result.as_ref().and_then(|rx|rx.try_recv().ok()) {
            self.engine_ownership.ids=result;self.engine_ownership.result=None;
        }
        owned.extend(self.engine_ownership.ids.iter().copied());
    }
}
