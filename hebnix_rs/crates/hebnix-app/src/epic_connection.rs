use std::time::Duration;

use crossbeam_channel::{Receiver, unbounded};
use eframe::egui;


#[derive(Default)]
pub struct RepairState {
    pub confirm: bool,
    pub running: bool,
    pub result: Option<Result<(), String>>,
    completion: Option<Receiver<Result<(), String>>>,
}

impl RepairState {
    pub fn begin(&mut self, ctx: &egui::Context) {
        if self.running {
            return;
        }
        if hebnix_sdk::process::is_rocket_league_running() {
            self.confirm = true;
        } else {
            self.run(ctx);
        }
    }

    fn run(&mut self, ctx: &egui::Context) {
        self.confirm = false;
        self.result = None;
        self.running = true;
        let (tx, rx) = unbounded();
        self.completion = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            while hebnix_sdk::process::is_rocket_league_running() {
                std::thread::sleep(Duration::from_millis(500));
            }
            let result = repair();
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        if let Some(result) = self.completion.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.running = false;
            self.completion = None;
            self.result = Some(result);
        }
        if self.confirm {
            egui::Window::new("Fix Epic Connection")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Rocket League needs to quit before fixing the Epic connection. Continue when it closes?");
                    ui.horizontal(|ui| {
                        if ui.button("Yes").clicked() {
                            self.run(ctx);
                        }
                        if ui.button("No").clicked() {
                            self.confirm = false;
                        }
                    });
                });
        }
        if let Some(result) = &self.result {
            let title = if result.is_ok() { "Spoofer Cleared" } else { "Fix Epic Connection" };
            let mut close = false;
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    if let Err(error) = result {
                        ui.label(format!("Could not clear the spoofer: {error}"));
                    } else {
                        ui.label("Spoofer Cleared");
                    }
                    if ui.button("OK").clicked() {
                        close = true;
                    }
                });
            if close {
                self.result = None;
            }
        }
    }
}

fn repair() -> Result<(), String> {
    let system_root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let path = std::path::PathBuf::from(system_root).join(r"System32\drivers\etc\hosts");
    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("Could not read hosts file: {error}"))?;
    let kept: Vec<&str> = content.lines()
        .filter(|line| !line.contains("#hebnix") && !line.contains("# hebnix spoofer"))
        .collect();
    if kept.len() != content.lines().count() {
        let mut output = kept.join("\r\n");
        if !output.is_empty() {
            output.push_str("\r\n");
        }
        std::fs::write(&path, output)
            .map_err(|error| format!("Could not update hosts file: {error}. Try running Hebnix as administrator."))?;
    }
    let user_profile = std::env::var_os("USERPROFILE")
        .ok_or_else(|| "USERPROFILE is not set".to_string())?;
    let web_cache = std::path::PathBuf::from(user_profile)
        .join(r"Documents\My Games\Rocket League\TAGame\Cache\WebCache");
    match std::fs::remove_dir_all(web_cache) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Could not clear Rocket League WebCache: {error}")),
    }
    use std::os::windows::process::CommandExt;
    let status = std::process::Command::new("ipconfig")
        .arg("/flushdns")
        .creation_flags(0x08000000)
        .status()
        .map_err(|error| format!("Could not flush DNS: {error}"))?;
    if !status.success() {
        return Err(format!("DNS flush exited with {status}"));
    }
    Ok(())
}
