#![windows_subsystem = "windows"] // Launch without a console window

use eframe::egui;
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use chrono::{TimeZone, Local};
use voidcore_shared::RuntimeConfig;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([700.0, 500.0])
            .with_min_inner_size([600.0, 400.0])
            .with_title("VoidCore Command Center"),
        ..Default::default()
    };

    eframe::run_native(
        "VoidCore Dashboard",
        options,
        Box::new(|cc| {
            // Apply custom dark theme visuals
            let mut visuals = egui::Visuals::dark();
            visuals.window_rounding = 8.0.into();
            visuals.panel_fill = egui::Color32::from_rgb(18, 18, 22);
            visuals.selection.bg_fill = egui::Color32::from_rgb(0, 150, 255); // Cyan accent
            cc.egui_ctx.set_visuals(visuals);
            
            Box::new(VoidCoreApp::new())
        }),
    )
}

struct VoidCoreApp {
    selected_tab: Tab,
    daemon_status: String,
    daemon_version: String,
    update_message: Option<String>,
    config: RuntimeConfig,
    logs: Vec<String>,
}

#[derive(PartialEq)]
enum Tab {
    Overview,
    Enforcements,
    Logs,
}

impl VoidCoreApp {
    fn new() -> Self {
        let (status, version) = fetch_daemon_status();
        
        let config = fs::read_to_string(r"C:\ProgramData\VoidCore\config.json")
            .ok()
            .and_then(|s| serde_json::from_str::<RuntimeConfig>(&s).ok())
            .unwrap_or_default();

        Self {
            selected_tab: Tab::Overview,
            daemon_status: status,
            daemon_version: version,
            update_message: None,
            config,
            logs: vec![],
        }
    }

    fn reload_logs(&mut self) {
        self.logs.clear();
        if let Ok(content) = fs::read_to_string(r"C:\ProgramData\VoidCore\logs\enforce.log") {
            for line in content.lines().rev().take(100) {
                let parts: Vec<&str> = line.splitn(3, ' ').collect();
                if parts.len() == 3 {
                    if let Ok(timestamp) = parts[0].parse::<i64>() {
                        if let match_opt = Local.timestamp_opt(timestamp, 0) {
                            if let chrono::LocalResult::Single(dt) = match_opt {
                                let formatted = format!("{}  {}  {}", dt.format("%Y-%m-%d %H:%M:%S"), parts[1], parts[2]);
                                self.logs.push(formatted);
                                continue;
                            }
                        }
                    }
                }
                self.logs.push(line.to_string());
            }
        }
        if self.logs.is_empty() {
            self.logs.push("No recent enforcements logged.".to_string());
        }
    }
}

impl eframe::App for VoidCoreApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // TOP PANEL: Branding and Actions
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.heading(egui::RichText::new("🛡 VoidCore").strong().size(22.0).color(egui::Color32::WHITE));
                
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⟳ Check for Updates").clicked() {
                        self.update_message = Some(send_update_command());
                    }
                });
            });
            ui.add_space(8.0);
            
            if let Some(msg) = &self.update_message {
                ui.label(egui::RichText::new(msg).color(egui::Color32::from_rgb(0, 200, 100)));
                ui.add_space(4.0);
            }
        });

        // SIDE PANEL: Navigation
        egui::SidePanel::left("side_panel").resizable(false).exact_width(140.0).show(ctx, |ui| {
            ui.add_space(10.0);
            ui.selectable_value(&mut self.selected_tab, Tab::Overview, "📊 Overview");
            ui.add_space(5.0);
            ui.selectable_value(&mut self.selected_tab, Tab::Enforcements, "🔒 Policies");
            ui.add_space(5.0);
            if ui.selectable_value(&mut self.selected_tab, Tab::Logs, "📝 Audit Logs").clicked() {
                self.reload_logs();
            }
        });

        // CENTRAL PANEL: Content
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(10.0);
            match self.selected_tab {
                Tab::Overview => {
                    ui.heading("System Status");
                    ui.separator();
                    ui.add_space(10.0);
                    
                    let status_color = if self.daemon_status == "running" {
                        egui::Color32::from_rgb(0, 255, 100)
                    } else {
                        egui::Color32::from_rgb(255, 50, 50)
                    };

                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Daemon State:").strong().size(16.0));
                        ui.label(egui::RichText::new(self.daemon_status.to_uppercase()).strong().size(16.0).color(status_color));
                    });
                    
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Architecture:").strong().size(16.0));
                        ui.label(egui::RichText::new("Zero-Trust (SYSTEM)").size(16.0).color(egui::Color32::LIGHT_GRAY));
                    });

                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Engine Version:").strong().size(16.0));
                        ui.label(egui::RichText::new(format!("v1.0.{}", self.daemon_version)).size(16.0).color(egui::Color32::LIGHT_GRAY));
                    });

                    ui.add_space(30.0);
                    ui.label(egui::RichText::new("The machine is fully secured. Safe mode is destroyed. Administrators have been purged. You are locked in.").italics().color(egui::Color32::GRAY));
                },
                Tab::Enforcements => {
                    ui.heading("Active Zero-Trust Policies");
                    ui.separator();
                    
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add_space(10.0);
                        ui.label(egui::RichText::new("🌐 Network Layer").strong().color(egui::Color32::from_rgb(0, 150, 255)));
                        ui.label("  ✔ OS DNS locked to Mullvad Ad/Tracker/Malware filter (194.242.2.9).");
                        ui.label("  ✔ Chromium DNS-over-HTTPS cryptographically forced.");
                        ui.label(format!("  ✔ Firewall Outbound Drops dynamically active for {} domains.", self.config.url_blocklist.len()));
                        
                        ui.add_space(15.0);
                        ui.label(egui::RichText::new("🔒 Application Layer").strong().color(egui::Color32::from_rgb(0, 150, 255)));
                        ui.label("  ✔ Incognito and InPrivate Browsing completely disabled.");
                        ui.label("  ✔ Chromium Extensions blocked universally (Asterisk rule).");
                        ui.label("  ✔ Bitwarden explicitly allowed in ExtensionInstallAllowlist.");
                        
                        ui.add_space(15.0);
                        ui.label(egui::RichText::new("🛡 Execution Layer").strong().color(egui::Color32::from_rgb(0, 150, 255)));
                        ui.label(format!("  ✔ Whitelist strict enforcement active ({} explicit apps).", self.config.whitelist.len()));
                        ui.label(format!("  ✔ Authenticode fallback active ({} trusted publishers).", self.config.trusted_publishers.len()));
                        ui.label("  ✔ 15-Minute Timebomb heuristic active on verified installers.");
                    });
                },
                Tab::Logs => {
                    ui.horizontal(|ui| {
                        ui.heading("Enforcement Audit Logs");
                        if ui.button("↻ Refresh").clicked() {
                            self.reload_logs();
                        }
                    });
                    ui.separator();
                    
                    egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                        for log in &self.logs {
                            let color = if log.contains("[BLOCK]") {
                                egui::Color32::from_rgb(255, 100, 100)
                            } else if log.contains("[ALLOW]") {
                                egui::Color32::from_rgb(100, 255, 100)
                            } else {
                                egui::Color32::LIGHT_GRAY
                            };
                            ui.label(egui::RichText::new(log).color(color).family(egui::FontFamily::Monospace));
                        }
                    });
                }
            }
        });
    }
}

// IPC HELPER FUNCTIONS
fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn connect_to_daemon() -> Result<std::fs::File, ()> {
    let pipe_name = to_wide(r"\\.\pipe\voidcore_ipc");
    unsafe {
        use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
        use windows::core::PCWSTR;

        let handle = CreateFileW(
            PCWSTR(pipe_name.as_ptr()),
            0xC0000000, // GENERIC_READ | GENERIC_WRITE
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            Default::default(),
            None,
        );

        if let Ok(h) = handle {
            if h.0 != 0 && h.0 != INVALID_HANDLE_VALUE.0 {
                return Ok(std::fs::File::from_raw_handle(h.0 as *mut _));
            }
        }
    }
    Err(())
}

fn fetch_daemon_status() -> (String, String) {
    if let Ok(mut file) = connect_to_daemon() {
        if let Ok(token) = std::fs::read_to_string(r"C:\ProgramData\VoidCore\gui.token") {
            let _ = file.write_all(format!("TOKEN:{}\n", token.trim()).as_bytes());
        } else {
            let _ = file.write_all(b"\n");
        }
        
        let _ = file.write_all(b"status\n");
        let _ = file.flush();
        
        let mut reader = BufReader::new(file);
        let mut resp = String::new();
        if reader.read_line(&mut resp).is_ok() && resp.trim().starts_with('{') {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&resp) {
                let status = json["service"].as_str().unwrap_or("error").to_string();
                let version = json["version"].as_u64().unwrap_or(0).to_string();
                return (status, version);
            }
        }
    }
    ("disconnected".to_string(), "0".to_string())
}

fn send_update_command() -> String {
    if let Ok(mut file) = connect_to_daemon() {
        if let Ok(token) = std::fs::read_to_string(r"C:\ProgramData\VoidCore\gui.token") {
            let _ = file.write_all(format!("TOKEN:{}\n", token.trim()).as_bytes());
        } else {
            let _ = file.write_all(b"\n");
        }
        
        let _ = file.write_all(b"update\n");
        let _ = file.flush();
        return "Update requested. System daemon will hot-swap and reboot in ~15 seconds.".to_string();
    }
    "Failed to reach daemon. Are you sure it is running?".to_string()
}