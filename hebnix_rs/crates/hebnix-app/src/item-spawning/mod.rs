use rand::Rng;

pub const QUALITIES: [&str; 10] = [
    "None",
    "Uncommon",
    "Rare",
    "Very Rare",
    "Import",
    "Exotic",
    "Black Market",
    "Premium",
    "Limited",
    "Legacy",
];
pub const PAINTS: [&str; 18] = [
    "None",
    "Crimson",
    "Lime",
    "Black",
    "Sky Blue",
    "Cobalt",
    "Burnt Sienna",
    "Forest Green",
    "Purple",
    "Pink",
    "Orange",
    "Grey",
    "Titanium White",
    "Saffron",
    "Gold",
    "Rose Gold",
    "White Gold",
    "Onyx",
];
pub const CERTIFICATIONS: [&str; 16] = [
    "None",
    "Aviator",
    "Playmaker",
    "Show-off",
    "Acrobat",
    "Tactician",
    "Sweeper",
    "Guardian",
    "Scorer",
    "Juggler",
    "Sniper",
    "Paragon",
    "Goalkeeper",
    "Striker",
    "Turtle",
    "Victor",
];

#[derive(Debug, Clone)]
pub struct ItemSpawnRequest {
    pub product_id: i64,
    pub series_id: i64,
    pub quality: usize,
    pub paint: usize,
    pub certification: usize,
    pub quantity: usize,
}

#[derive(Debug)]
pub struct ItemSpawnForm {
    product_id: String,
    series_id: String,
    quality: usize,
    paint: usize,
    certification: usize,
    quantity: usize,
    pub status: Option<Result<String, String>>,
}

impl Default for ItemSpawnForm {
    fn default() -> Self {
        Self {
            product_id: String::new(),
            series_id: "1".into(),
            quality: 0,
            paint: 0,
            certification: 0,
            quantity: 1,
            status: None,
        }
    }
}

impl ItemSpawnForm {
    pub fn render(&mut self, ui: &mut eframe::egui::Ui) -> Option<ItemSpawnRequest> {
        use eframe::egui;
        ui.heading("Item Spawning");
        ui.label("Enter the item requirements, then send it through the active PsyNet WebSocket.");
        ui.add_space(12.0);
        egui::Grid::new("item_spawn_requirements")
            .num_columns(2)
            .spacing([16.0, 10.0])
            .show(ui, |ui| {
                ui.label("Product ID");
                ui.text_edit_singleline(&mut self.product_id);
                ui.end_row();
                ui.label("Series ID");
                ui.text_edit_singleline(&mut self.series_id);
                ui.end_row();
                ui.label("Quality");
                egui::ComboBox::from_id_salt("item_quality")
                    .selected_text(QUALITIES[self.quality])
                    .show_ui(ui, |ui| {
                        for (index, name) in QUALITIES.iter().enumerate() {
                            ui.selectable_value(&mut self.quality, index, *name);
                        }
                    });
                ui.end_row();
                ui.label("Paint / color");
                egui::ComboBox::from_id_salt("item_paint")
                    .selected_text(PAINTS[self.paint])
                    .show_ui(ui, |ui| {
                        for (index, name) in PAINTS.iter().enumerate() {
                            ui.selectable_value(&mut self.paint, index, *name);
                        }
                    });
                ui.end_row();
                ui.label("Certification");
                egui::ComboBox::from_id_salt("item_cert")
                    .selected_text(CERTIFICATIONS[self.certification])
                    .show_ui(ui, |ui| {
                        for (index, name) in CERTIFICATIONS.iter().enumerate() {
                            ui.selectable_value(&mut self.certification, index, *name);
                        }
                    });
                ui.end_row();
                ui.label("Quantity");
                ui.add(egui::DragValue::new(&mut self.quantity).range(1..=100));
                ui.end_row();
            });
        ui.add_space(12.0);
        let clicked = ui
            .add_enabled(
                !self.product_id.trim().is_empty(),
                egui::Button::new("Spawn Item"),
            )
            .clicked();
        if let Some(status) = &self.status {
            match status {
                Ok(text) => {
                    ui.colored_label(egui::Color32::LIGHT_GREEN, text);
                }
                Err(text) => {
                    ui.colored_label(egui::Color32::LIGHT_RED, text);
                }
            }
        }
        if !clicked {
            return None;
        }
        let product_id = match self.product_id.trim().parse::<i64>() {
            Ok(value) if value > 0 => value,
            _ => {
                self.status = Some(Err("Product ID must be a positive integer.".into()));
                return None;
            }
        };
        let series_id = match self.series_id.trim().parse::<i64>() {
            Ok(value) if value > 0 => value,
            _ => {
                self.status = Some(Err("Series ID must be a positive integer.".into()));
                return None;
            }
        };
        self.status = None;
        Some(ItemSpawnRequest {
            product_id,
            series_id,
            quality: self.quality,
            paint: self.paint,
            certification: self.certification,
            quantity: self.quantity,
        })
    }
}

pub fn reward_message(request: &ItemSpawnRequest, psy_time: i64) -> Result<String, String> {
    let mut products = Vec::with_capacity(request.quantity);
    for _ in 0..request.quantity {
        let mut attributes = vec![serde_json::json!({
            "Key": "Quality",
            "Value": request.quality
        })];
        if request.paint > 0 {
            attributes.push(serde_json::json!({"Key": "Painted", "Value": request.paint}));
        }
        if request.certification > 0 {
            attributes.push(serde_json::json!({
                "Key": "Certified",
                "Value": request.certification
            }));
        }
        products.push(serde_json::json!({
            "AddedTimestamp": psy_time,
            "UpdatedTimestamp": psy_time,
            "InstanceID": format!("{:032x}", rand::thread_rng().r#gen::<u128>()),
            "ProductID": request.product_id,
            "SeriesID": request.series_id,
            "TradeHold": -2,
            "Attributes": attributes
        }));
    }
    let body = serde_json::to_string(&serde_json::json!({
        "RocketPassInfo": {"TierLevel": 0, "bOwnsPremium": false, "XPMultiplier": 0.0},
        "ProductData": products,
        "RewardDrops": [],
        "ChallengeRewards": [],
        "CurrencyDrops": [],
        "Source": "",
        "MatchGUID": ""
    }))
    .map_err(|error| error.to_string())?;
    let sig = crate::spoofer::rules::psy_response_signature(&psy_time.to_string(), body.as_bytes());
    Ok(format!(
        "PsyService: Reward/RewardResult\r\nPsyServiceVersion: 2\r\nPsyTime: {psy_time}\r\nPsySig: {sig}\r\n\r\n{body}"
    ))
}
