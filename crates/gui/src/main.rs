#![windows_subsystem = "windows"] // Launch without a console window

use eframe::egui;
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use chrono::{TimeZone, Local};
use voidcore_shared::RuntimeConfig;

fn show_error_msg(msg: &str) {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_OK, MB_ICONERROR};
        use windows::core::{PCWSTR, HSTRING};
        
        let title = HSTRING::from("VoidCore Diagnostics");
        let body = HSTRING::from(msg);
        let _ = MessageBoxW(None, PCWSTR(body.as_ptr()), PCWSTR(title.as_ptr()), MB_OK | MB_ICONERROR);
    }
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("Critical UI Crash:\n\n{:?}", info);
        let _ = std::fs::write(r"C:\ProgramData\VoidCore\logs\gui-panic.log", &msg);
        show_error_msg(&msg);
    }));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 550.0])
            .with_min_inner_size([700.0, 450.0])
            .with_title("VoidCore Command Center"),
        ..Default::default()
    };

    let result = eframe::run_native(
        "VoidCore Dashboard",
        options,
        Box::new(|cc| {
            let mut visuals = egui::Visuals::dark();
            visuals.window_rounding = 10.0.into();
            visuals.panel_fill = egui::Color32::from_rgb(14, 16, 20);
            visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(26, 28, 34);
            visuals.selection.bg_fill = egui::Color32::from_rgb(0, 160, 255);
            cc.egui_ctx.set_visuals(visuals);
            
            Box::new(VoidCoreApp::new())
        }),
    );

    if let Err(e) = result {
        let err_msg = format!("The VoidCore graphics engine failed to initialize.\n\nTechnical Details:\n{}", e);
        show_error_msg(&err_msg);
    }
}

struct LogEntry {
    time: String,
    action: String,
    message: String,
}

struct VoidCoreApp {
    selected_tab: Tab,
    daemon_status: String,
    daemon_version: String,
    update_message: Option<String>,
    elevate_path: String,
    config: RuntimeConfig,
    logs: Vec<LogEntry>,
}

#[derive(PartialEq)]
enum Tab {
    Overview,
    Enforcements,
    Launchpad,
    Logs,
}

impl VoidCoreApp {
    fn new() -> Self {
        let (status, version) = fetch_daemon_status();
        
        let config = fs::read_to_string(r"C:\ProgramData\VoidCore\config.json")
            .ok()
            .and_then(|s| serde_json::from_str::<RuntimeConfig>(&s).ok())
            .unwrap_or_default();

        let mut app = Self {
            selected_tab: Tab::Overview,
            daemon_status: status,
            daemon_version: version,
            update_message: None,
            elevate_path: String::new(),
            config,
            logs: vec![],
        };
        
        app.reload_logs();
        app
    }

    fn reload_logs(&mut self) {
        self.logs.clear();
        if let Ok(content) = fs::read_to_string(r"C:\ProgramData\VoidCore\logs\enforce.log") {
            for line in content.lines().rev().take(150) {
                let parts: Vec<&str> = line.splitn(3, ' ').collect();
                if parts.len() == 3 {
                    if let Ok(timestamp) = parts[0].parse::<i64>() {
                        if let chrono::LocalResult::Single(dt) = Local.timestamp_opt(timestamp, 0) {
                            self.logs.push(LogEntry {
                                time: dt.format("%Y-%m-%d %H:%M:%S").to_string(),
                                action: parts[1].replace("[", "").replace("]", "").trim().to_string(),
                                message: parts[2].trim().to_string(),
                            });
                        }
                    }
                }
            }
        }
    }
}

impl eframe::App for VoidCoreApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        
        // Handle Drag-and-Drop globally
        if !ctx.input(|i| i.raw.dropped_files.is_empty()) {
            if let Some(file) = ctx.input(|i| i.raw.dropped_files.first().cloned()) {
                if let Some(path) = file.path {
                    self.elevate_path = path.to_string_lossy().to_string();
                    self.selected_tab = Tab::Launchpad;
                }
            }
        }

        // TOP HEADER
        egui::TopBottomPanel::top("top_panel").frame(
            egui::Frame::none().fill(egui::Color32::from_rgb(20, 22, 28)).inner_margin(12.0)
        ).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(egui::RichText::new("🛡 VoidCore").strong().size(22.0).color(egui::Color32::WHITE));
                ui.label(egui::RichText::new("Zero-Trust Architecture").size(14.0).color(egui::Color32::GRAY));
                
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let btn = ui.add(egui::Button::new(egui::RichText::new("⟳ Check for Updates").size(13.0)).fill(egui::Color32::from_rgb(0, 100, 180)));
                    if btn.clicked() {
                        self.update_message = Some(send_update_command());
                    }
                });
            });
            
            if let Some(msg) = &self.update_message {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(msg).strong().color(egui::Color32::from_rgb(0, 200, 100)));
            }
        });

        // SIDE NAVIGATION
        egui::SidePanel::left("side_panel").frame(
            egui::Frame::none().fill(egui::Color32::from_rgb(18, 20, 24)).inner_margin(16.0)
        ).exact_width(160.0).show(ctx, |ui| {
            ui.style_mut().spacing.item_spacing.y = 12.0;
            
            ui.selectable_value(&mut self.selected_tab, Tab::Overview, egui::RichText::new("📊 Overview").size(16.0));
            ui.selectable_value(&mut self.selected_tab, Tab::Enforcements, egui::RichText::new("🔒 Policies").size(16.0));
            ui.selectable_value(&mut self.selected_tab, Tab::Launchpad, egui::RichText::new("🚀 Launchpad").size(16.0));
            
            if ui.selectable_value(&mut self.selected_tab, Tab::Logs, egui::RichText::new("📝 Audit Logs").size(16.0)).clicked() {
                self.reload_logs();
            }
        });

        // MAIN CONTENT AREA
        egui::CentralPanel::default().frame(
            egui::Frame::none().fill(egui::Color32::from_rgb(14, 16, 20)).inner_margin(24.0)
        ).show(ctx, |ui| {
            
            match self.selected_tab {
                Tab::Overview => {
                    ui.heading(egui::RichText::new("System Status").size(24.0).strong().color(egui::Color32::WHITE));
                    ui.add_space(20.0);
                    
                    let card_frame = egui::Frame::none()
                        .fill(egui::Color32::from_rgb(26, 28, 34))
                        .rounding(8.0)
                        .inner_margin(20.0);
                    
                    card_frame.show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        
                        let status_color = if self.daemon_status == "running" {
                            egui::Color32::from_rgb(50, 255, 120)
                        } else {
                            egui::Color32::from_rgb(255, 80, 80)
                        };

                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Daemon State:").strong().size(18.0).color(egui::Color32::LIGHT_GRAY));
                            ui.label(egui::RichText::new(self.daemon_status.to_uppercase()).strong().size(18.0).color(status_color));
                        });
                        
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Engine Version:").strong().size(16.0).color(egui::Color32::LIGHT_GRAY));
                            ui.label(egui::RichText::new(format!("v1.0.{}", self.daemon_version)).size(16.0).color(egui::Color32::WHITE));
                        });
                        
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Architecture:").strong().size(16.0).color(egui::Color32::LIGHT_GRAY));
                            ui.label(egui::RichText::new("Zero-Trust (NT AUTHORITY\\SYSTEM)").size(16.0).color(egui::Color32::from_rgb(200, 200, 200)));
                        });
                    });

                    ui.add_space(40.0);
                    ui.label(egui::RichText::new("The machine is fully secured. Safe mode is destroyed. Administrators have been purged. You are locked in.").italics().size(14.0).color(egui::Color32::GRAY));
                },
                Tab::Enforcements => {
                    ui.heading(egui::RichText::new("Active Security Constraints").size(24.0).strong().color(egui::Color32::WHITE));
                    ui.add_space(20.0);
                    
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let policy_frame = egui::Frame::none().fill(egui::Color32::from_rgb(26, 28, 34)).rounding(8.0).inner_margin(16.0);

                        policy_frame.show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(egui::RichText::new("🌐 Network Layer").strong().size(16.0).color(egui::Color32::from_rgb(0, 180, 255)));
                            ui.add_space(8.0);
                            ui.label("✔ OS DNS locked to Mullvad Ad/Tracker/Malware filter.");
                            ui.label("✔ Chromium DNS-over-HTTPS cryptographically forced.");
                            ui.label(format!("✔ Firewall Outbound Drops dynamically active for {} domains.", self.config.url_blocklist.len()));
                        });
                        ui.add_space(15.0);

                        policy_frame.show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(egui::RichText::new("🔒 Application Layer").strong().size(16.0).color(egui::Color32::from_rgb(0, 180, 255)));
                            ui.add_space(8.0);
                            ui.label("✔ Incognito and InPrivate Browsing completely disabled.");
                            ui.label("✔ Chromium Extensions blocked universally (Asterisk rule).");
                            ui.label("✔ Bitwarden explicitly allowed in ExtensionInstallAllowlist.");
                        });
                        ui.add_space(15.0);

                        policy_frame.show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(egui::RichText::new("🛡 Execution Layer").strong().size(16.0).color(egui::Color32::from_rgb(0, 180, 255)));
                            ui.add_space(8.0);
                            ui.label(format!("✔ Whitelist strict enforcement active ({} explicit apps).", self.config.whitelist.len()));
                            ui.label(format!("✔ Authenticode validation active ({} trusted publishers).", self.config.trusted_publishers.len()));
                            ui.label("✔ 15-Minute Timebomb heuristic active on verified installers.");
                        });
                    });
                },
                Tab::Launchpad => {
                    ui.heading(egui::RichText::new("Secure Elevation Broker").size(24.0).strong().color(egui::Color32::WHITE));
                    ui.add_space(20.0);
                    
                    ui.label(egui::RichText::new(
                        "Some whitelisted applications (like installers or flashing tools) require Administrator privileges to run. \
                        Because your account is locked down, Windows UAC will normally block them.\n\n\
                        Paste the path to the executable below. If it is from a Trusted Publisher, the VoidCore daemon will dynamically generate a privileged \
                        environment and launch the application directly on your desktop."
                    ).size(14.0).color(egui::Color32::LIGHT_GRAY));
                    
                    ui.add_space(30.0);
                    
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Executable Path:").strong().size(14.0));
                        ui.add(egui::TextEdit::singleline(&mut self.elevate_path).desired_width(450.0).margin(egui::vec2(8.0, 8.0)));
                    });
                    
                    ui.add_space(20.0);
                    
                    if ui.button(egui::RichText::new("⚡ Launch Elevated").size(16.0)).clicked() {
                        self.update_message = Some(send_elevate_command(&self.elevate_path));
                    }
                    
                    ui.add_space(40.0);
                    ui.label(egui::RichText::new("💡 Tip: You can drag and drop an .exe file anywhere onto this window to auto-fill the path.").italics().color(egui::Color32::from_rgb(100, 150, 200)));
                },
                Tab::Logs => {
                    ui.horizontal(|ui| {
                        ui.heading(egui::RichText::new("Execution Audit Logs").size(24.0).strong().color(egui::Color32::WHITE));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("↻ Refresh").clicked() {
                                self.reload_logs();
                            }
                        });
                    });
                    ui.add_space(20.0);
                    
                    let table_frame = egui::Frame::none().fill(egui::Color32::from_rgb(22, 24, 28)).rounding(6.0).inner_margin(8.0);
                    table_frame.show(ui, |ui| {
                        egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                            egui::Grid::new("logs_grid").striped(true).min_col_width(140.0).spacing([20.0, 8.0]).show(ui, |ui| {
                                ui.label(egui::RichText::new("Timestamp").strong().color(egui::Color32::LIGHT_GRAY));
                                ui.label(egui::RichText::new("Action").strong().color(egui::Color32::LIGHT_GRAY));
                                ui.label(egui::RichText::new("Details").strong().color(egui::Color32::LIGHT_GRAY));
                                ui.end_row();

                                if self.logs.is_empty() {
                                    ui.label(egui::RichText::new("No entries.").italics());
                                    ui.end_row();
                                }

                                for log in &self.logs {
                                    let action_color = match log.action.as_str() {
                                        "BLOCK" => egui::Color32::from_rgb(255, 80, 80),
                                        "ALLOW" => egui::Color32::from_rgb(80, 255, 120),
                                        _ => egui::Color32::LIGHT_GRAY,
                                    };

                                    ui.label(egui::RichText::new(&log.time).size(13.0).color(egui::Color32::GRAY));
                                    ui.label(egui::RichText::new(&log.action).strong().size(13.0).color(action_color));
                                    ui.label(egui::RichText::new(&log.message).size(13.0).color(egui::Color32::WHITE));
                                    ui.end_row();
                                }
                            });
                        });
                    });
                }
            }
        });
    }
}

// ============================================================================
// IPC HELPER FUNCTIONS
// ============================================================================

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
            0xC0000000, 
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

fn send_elevate_command(path: &str) -> String {
    if path.trim().is_empty() { return "Please provide a valid executable path.".to_string(); }
    if let Ok(mut file) = connect_to_daemon() {
        if let Ok(token) = std::fs::read_to_string(r"C:\ProgramData\VoidCore\gui.token") {
            let _ = file.write_all(format!("TOKEN:{}\n", token.trim()).as_bytes());
        } else {
            let _ = file.write_all(b"\n");
        }
        
        let _ = file.write_all(format!("elevate:{}\n", path).as_bytes());
        let _ = file.flush();
        
        let mut reader = BufReader::new(file);
        let mut resp = String::new();
        if reader.read_line(&mut resp).is_ok() {
            return resp.trim().to_string();
        }
    }
    "Failed to securely communicate with the background daemon.".to_string()
}