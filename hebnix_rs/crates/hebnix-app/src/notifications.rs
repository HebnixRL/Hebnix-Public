//! One-shot startup announcements. Network/JSON failures are deliberately silent.

use std::time::Duration;

use eframe::egui;
use serde::Deserialize;

use crate::i18n::t;

const ENDPOINT: &str = "https://api.hebnix.com/notifications";

pub fn render_clock_warning(ctx: &egui::Context, open: &mut bool) -> bool {
    *open |= hebnix_sdk::req_auth::take_clock_warning();
    if !*open {
        return false;
    }

    let modal = egui::Modal::new(egui::Id::new("req_clock_warning")).show(ctx, |ui| {
        ui.set_width(400.0_f32.min((ctx.content_rect().width() - 48.0).max(120.0)));
        ui.heading(t("req-clock-warning-title"));
        ui.add(egui::Label::new(t("req-clock-warning-body")).wrap());
        ui.add_space(8.0);
        ui.button(t("btn-ok")).clicked()
    });
    *open = !modal.inner;
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct Notification {
    pub id: u64,
    pub message: String,
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default, rename = "link-text")]
    pub link_text: Option<String>,
}

impl Notification {
    pub fn is_newer_than(&self, dismissed_id: u64) -> bool {
        self.id > dismissed_id && !self.message.trim().is_empty()
    }

    pub fn link_url(&self) -> Option<reqwest::Url> {
        let link = self.link.as_deref()?.trim();
        if link.is_empty() {
            return None;
        }
        let url = reqwest::Url::parse(ENDPOINT).ok()?.join(link).ok()?;
        matches!(url.scheme(), "http" | "https").then_some(url)
    }
}

/// Shared announcement UI for both editions. Only an explicit close dismisses it.
pub fn render_popup(ctx: &egui::Context, notification: &Notification) -> bool {
    let mut dismissed = false;
    let id = egui::Id::new("web_notification");
    egui::Modal::new(id).show(ctx, |ui| {
        ui.set_width(440.0_f32.min((ctx.content_rect().width() - 48.0).max(120.0)));
        ui.horizontal(|ui| {
            ui.heading("Notification");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                dismissed |= ui.button("×").on_hover_text("Dismiss").clicked();
            });
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .max_height((ctx.content_rect().height() * 0.6).max(80.0))
            .show(ui, |ui| crate::panel::followed(ui, |ui| {
                ui.add(egui::Label::new(&notification.message).wrap());
            }));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if let Some(url) = notification.link_url() {
                let text = notification
                    .link_text
                    .as_deref()
                    .filter(|text| !text.trim().is_empty())
                    .unwrap_or("Open link");
                if ui.button(text).clicked() {
                    let _ = open::that(url.as_str());
                }
            }
            dismissed |= ui.button("Dismiss").clicked();
        });
    });
    let layer = egui::LayerId::new(egui::Order::Foreground, id);
    ctx.move_to_top(layer);
    ctx.memory_mut(|memory| memory.set_modal_layer(layer));
    // Ignore backdrop clicks/Escape: the announcement stays until explicitly closed.
    dismissed
}

pub fn check_on_start(
    dismissed_id: u64,
    tx: crossbeam_channel::Sender<crate::messages::AppMsg>,
    ctx: eframe::egui::Context,
) {
    let _ = std::thread::Builder::new()
        .name("notification-checker".into())
        .spawn(move || {
            let notification = (|| {
                ureq::AgentBuilder::new()
                    .try_proxy_from_env(false)
                    .build()
                    .get(ENDPOINT)
                    .set("User-Agent", "Hebnix")
                    .timeout(Duration::from_secs(10))
                    .call()
                    .ok()?
                    .into_json::<Notification>()
                    .ok()
            })();
            if let Some(notification) = notification.filter(|n| n.is_newer_than(dismissed_id)) {
                let _ = tx.send(crate::messages::AppMsg::NotificationFetched(notification));
                ctx.request_repaint();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_new_nonempty_announcements_are_shown() {
        let mut notification: Notification =
            serde_json::from_str(r#"{"id":4,"message":"New announcement"}"#).unwrap();
        assert!(notification.is_newer_than(3));
        assert!(!notification.is_newer_than(4));
        assert!(!notification.is_newer_than(5));
        notification.message = " \n\t".into();
        assert!(!notification.is_newer_than(0));
    }

    #[test]
    fn optional_links_and_link_text_follow_the_api_format() {
        let notification: Notification = serde_json::from_str(
            r#"{"id":1,"message":"News","link":"/news","link-text":"Read more"}"#,
        )
        .unwrap();
        assert_eq!(notification.link_text.as_deref(), Some("Read more"));
        assert_eq!(
            notification.link_url().unwrap().as_str(),
            "https://api.hebnix.com/news"
        );
        let notification: Notification =
            serde_json::from_str(r#"{"id":1,"message":"News"}"#).unwrap();
        assert!(notification.link_url().is_none());
    }

    #[test]
    fn non_web_links_are_not_offered() {
        for link in ["", " ", "javascript:alert(1)", "file:///C:/example.txt"] {
            let notification = Notification {
                id: 1,
                message: "News".into(),
                link: Some(link.into()),
                link_text: None,
            };
            assert!(notification.link_url().is_none(), "{link}");
        }
    }
}
