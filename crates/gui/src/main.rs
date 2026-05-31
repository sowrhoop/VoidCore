#![windows_subsystem = "windows"]

mod theme;
mod ui;

use eframe::egui;
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use chrono::{Local, TimeZone};
use std::sync::Arc;
use theme::{card_frame, page_title, section_title, Palette};
use voidcore_shared::RuntimeConfig;

fn load_app_icon() -> Option<Arc<egui::IconData>> {
    let img = image::load_from_memory(include_bytes!("../assets/voidcore-icon.png")).ok()?;
    let img = img.into_rgba8();
    let (width, height) = img.dimensions();
    Some(Arc::new(egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    }))
}

fn show_error_msg(msg: &str) {
    unsafe {
        use windows::core::{HSTRING, PCWSTR};
        use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

        let title = HSTRING::from("VoidCore Diagnostics");
        let body = HSTRING::from(msg);
        let _ = MessageBoxW(
            None,
            PCWSTR(body.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("Critical UI Crash:\n\n{:?}", info);
        let _ = std::fs::write(r"C:\ProgramData\VoidCore\logs\gui-panic.log", &msg);
        show_error_msg(&msg);
    }));

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([920.0, 620.0])
        .with_min_inner_size([760.0, 500.0])
        .with_title("VoidCore Command Center");
    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let result = eframe::run_native(
        "VoidCore Dashboard",
        options,
        Box::new(|cc| {
            theme::apply_theme(&cc.egui_ctx);
            Box::new(VoidCoreApp::new())
        }),
    );

    if let Err(e) = result {
        let err_msg = format!(
            "The VoidCore graphics engine failed to initialize.\n\nTechnical Details:\n{}",
            e
        );
        show_error_msg(&err_msg);
    }
}

struct LogEntry {
    timestamp: i64,
    time: String,
    source: String,
    action: String,
    message: String,
}

const LOG_DIR: &str = r"C:\ProgramData\VoidCore\logs";

fn parse_log_line(line: &str) -> Option<(i64, String, String)> {
    let parts: Vec<&str> = line.splitn(3, ' ').collect();
    if parts.len() != 3 {
        return None;
    }
    let timestamp = parts[0].parse::<i64>().ok()?;
    let action = parts[1].replace('[', "").replace(']', "").trim().to_string();
    let message = parts[2].trim().to_string();
    Some((timestamp, action, message))
}

fn load_log_file(path: &str, source: &str, actions: Option<&[&str]>) -> Vec<LogEntry> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let mut entries = Vec::new();
    for line in content.lines() {
        let Some((timestamp, action, message)) = parse_log_line(line) else {
            continue;
        };
        if let Some(allowed) = actions {
            if !allowed.iter().any(|a| *a == action) {
                continue;
            }
        }
        let time = match Local.timestamp_opt(timestamp, 0) {
            chrono::LocalResult::Single(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            _ => timestamp.to_string(),
        };
        entries.push(LogEntry {
            timestamp,
            time,
            source: source.to_string(),
            action,
            message,
        });
    }
    entries
}

fn load_all_voidcore_logs(limit: usize) -> Vec<LogEntry> {
    let mut entries = Vec::new();
    if let Ok(dir) = fs::read_dir(LOG_DIR) {
        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("log") {
                continue;
            }
            let Some(source) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            entries.extend(load_log_file(
                &path.to_string_lossy(),
                source,
                None,
            ));
        }
    }
    entries.sort_by_key(|e| e.timestamp);
    if entries.len() > limit {
        entries.split_off(entries.len() - limit)
    } else {
        entries
    }
}

struct VoidCoreApp {
    selected_tab: Tab,
    daemon_status: String,
    daemon_version: String,
    update_message: Option<String>,
    update_message_ok: bool,
    elevate_path: String,
    config: RuntimeConfig,
    logs: Vec<LogEntry>,
    live_logs: Vec<LogEntry>,
    live_logs_follow: bool,
    last_status_refresh: f64,
    last_live_log_refresh: f64,
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Overview,
    Enforcements,
    Launchpad,
    Logs,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Enforcements => "Policies",
            Tab::Launchpad => "Launchpad",
            Tab::Logs => "Audit Logs",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Tab::Overview => "◆",
            Tab::Enforcements => "◇",
            Tab::Launchpad => "▶",
            Tab::Logs => "≡",
        }
    }
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
            update_message_ok: true,
            elevate_path: String::new(),
            config,
            logs: vec![],
            live_logs: vec![],
            live_logs_follow: true,
            last_status_refresh: 0.0,
            last_live_log_refresh: 0.0,
        };

        app.reload_logs();
        app.reload_live_logs();
        app
    }

    fn reload_logs(&mut self) {
        const ENFORCE_LOG: &str = r"C:\ProgramData\VoidCore\logs\enforce.log";

        let mut entries = load_log_file(ENFORCE_LOG, "enforce", None);

        entries.sort_by_key(|e| std::cmp::Reverse(e.timestamp));
        entries.truncate(150);
        self.logs = entries;
    }

    fn reload_live_logs(&mut self) {
        self.live_logs = load_all_voidcore_logs(300);
    }

    fn refresh_daemon_status(&mut self) {
        let (status, version) = fetch_daemon_status();
        self.daemon_status = status;
        self.daemon_version = version;
    }

    fn daemon_running(&self) -> bool {
        self.daemon_status == "running"
    }
}

impl eframe::App for VoidCoreApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let p = Palette::default();

        if !ctx.input(|i| i.raw.dropped_files.is_empty()) {
            if let Some(file) = ctx.input(|i| i.raw.dropped_files.first().cloned()) {
                if let Some(path) = file.path {
                    self.elevate_path = path.to_string_lossy().to_string();
                    self.selected_tab = Tab::Launchpad;
                }
            }
        }

        let now = ctx.input(|i| i.time);
        if now - self.last_status_refresh > 5.0 {
            self.refresh_daemon_status();
            self.last_status_refresh = now;
            ctx.request_repaint_after(std::time::Duration::from_secs(5));
        }

        if self.selected_tab == Tab::Overview && now - self.last_live_log_refresh > 2.0 {
            self.reload_live_logs();
            self.last_live_log_refresh = now;
            ctx.request_repaint_after(std::time::Duration::from_secs(2));
        }

        egui::TopBottomPanel::top("top_panel")
            .frame(egui::Frame::none().fill(p.bg_app))
            .show(ctx, |ui| {
                let mut trigger_update = false;
                ui::header_bar(ui, &p, &mut trigger_update);
                if trigger_update {
                    let msg = send_update_command();
                    self.update_message_ok = !msg.contains("Failed");
                    self.update_message = Some(msg);
                }
                if let Some(msg) = &self.update_message {
                    ui.add_space(8.0);
                    ui::toast_banner(ui, &p, msg, self.update_message_ok);
                    ui.add_space(4.0);
                }
            });

        egui::SidePanel::left("side_panel")
            .frame(
                egui::Frame::none()
                    .fill(p.bg_sidebar)
                    .stroke(egui::Stroke::new(1.0, p.border_subtle)),
            )
            .exact_width(200.0)
            .show(ctx, |ui| {
                ui.add_space(16.0);
                ui.label(
                    egui::RichText::new("NAVIGATION")
                        .size(11.0)
                        .color(p.text_muted),
                );
                ui.add_space(12.0);

                for tab in [
                    Tab::Overview,
                    Tab::Enforcements,
                    Tab::Launchpad,
                    Tab::Logs,
                ] {
                    let selected = self.selected_tab == tab;
                    if ui::nav_item(ui, &p, tab.label(), tab.icon(), selected).clicked() {
                        self.selected_tab = tab;
                        if tab == Tab::Logs {
                            self.reload_logs();
                        }
                        if tab == Tab::Overview {
                            self.reload_live_logs();
                            self.last_live_log_refresh = ctx.input(|i| i.time);
                        }
                    }
                    ui.add_space(4.0);
                }

                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(16.0);
                    ui.label(theme::muted_text(&format!(
                        "Engine v1.0.{}",
                        self.daemon_version
                    )));
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("Zero-Trust")
                            .size(11.0)
                            .italics()
                            .color(p.text_muted),
                    );
                });
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(p.bg_app)
                    .inner_margin(egui::Margin::symmetric(28.0, 24.0)),
            )
            .show(ctx, |ui| {
                match self.selected_tab {
                    Tab::Overview => self.show_overview(ui, &p),
                    Tab::Enforcements => self.show_enforcements(ui, &p),
                    Tab::Launchpad => self.show_launchpad(ui, &p),
                    Tab::Logs => self.show_logs(ui, &p),
                }
            });
    }
}

impl VoidCoreApp {
    fn show_overview(&mut self, ui: &mut egui::Ui, p: &Palette) {
        ui.horizontal(|ui| {
            ui.label(page_title("System Status"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui::secondary_button(ui, p, "Refresh").clicked() {
                    self.refresh_daemon_status();
                    self.reload_live_logs();
                }
            });
        });
        ui.add_space(6.0);
        ui.label(theme::muted_text(
            "Live connection to the VoidCore background service.",
        ));
        ui.add_space(24.0);

        ui.horizontal(|ui| {
            let card_w = (ui.available_width() - 16.0) / 2.0;

            card_frame(p).show(ui, |ui| {
                ui.set_width(card_w);
                ui.label(section_title("Daemon"));
                ui.add_space(14.0);
                ui::status_pill(
                    ui,
                    p,
                    &self.daemon_status.to_uppercase(),
                    self.daemon_running(),
                );
                ui.add_space(16.0);
                ui::stat_row(
                    ui,
                    p,
                    "Engine version",
                    &format!("v1.0.{}", self.daemon_version),
                    p.text_primary,
                );
                ui::stat_row(
                    ui,
                    p,
                    "Privilege model",
                    "NT AUTHORITY\\SYSTEM",
                    p.text_secondary,
                );
            });

            ui.add_space(16.0);

            card_frame(p).show(ui, |ui| {
                ui.set_width(card_w);
                ui.label(section_title("Protection Summary"));
                ui.add_space(14.0);
                ui::stat_row(
                    ui,
                    p,
                    "Whitelisted apps",
                    &self.config.whitelist.len().to_string(),
                    p.accent,
                );
                ui::stat_row(
                    ui,
                    p,
                    "Blocked domains",
                    &self.config.url_blocklist.len().to_string(),
                    p.accent,
                );
                ui::stat_row(
                    ui,
                    p,
                    "Trusted publishers",
                    &self.config.trusted_publishers.len().to_string(),
                    p.accent,
                );
            });
        });

        ui.add_space(20.0);

        card_frame(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("●")
                        .color(if self.daemon_running() {
                            p.success
                        } else {
                            p.warning
                        }),
                );
                ui.label(theme::body_text(
                    if self.daemon_running() {
                        "The machine is under active Zero-Trust enforcement. Safe mode is disabled, \
                         administrative surfaces are restricted, and execution policy is enforced by the daemon."
                    } else {
                        "The daemon is not reachable. Enforcement may be offline — verify the VoidCore \
                         Windows service is running."
                    },
                ));
            });
        });

        ui.add_space(16.0);
        self.show_live_log_panel(ui, p);
    }

    fn show_live_log_panel(&mut self, ui: &mut egui::Ui, p: &Palette) {
        card_frame(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(section_title("Live Activity Log"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !self.live_logs_follow {
                        if ui::secondary_button(ui, p, "Follow latest").clicked() {
                            self.live_logs_follow = true;
                        }
                    } else {
                        ui.label(
                            egui::RichText::new("Live · 2s refresh")
                                .size(11.0)
                                .color(p.success),
                        );
                    }
                });
            });
            ui.add_space(4.0);
            ui.label(theme::muted_text(
                "Merged stream from core, enforce, ipc, and other VoidCore logs.",
            ));
            ui.add_space(10.0);

            if self.live_logs.is_empty() {
                ui::empty_state(
                    ui,
                    p,
                    "No log activity yet",
                    "Events will appear here as the daemon writes to C:\\ProgramData\\VoidCore\\logs.",
                );
                return;
            }

            let scroll = egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(self.live_logs_follow)
                .max_height(240.0)
                .show(ui, |ui| {
                    for log in &self.live_logs {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                egui::RichText::new(&log.time)
                                    .size(12.0)
                                    .color(p.text_muted)
                                    .family(egui::FontFamily::Monospace),
                            );
                            ui.label(
                                egui::RichText::new(format!("[{}]", log.source))
                                    .size(12.0)
                                    .color(p.accent)
                                    .family(egui::FontFamily::Monospace),
                            );
                            let level_color = match log.action.as_str() {
                                "ERROR" | "BLOCK" | "IPC_AUTH_FAIL" => p.danger,
                                "WARN" => p.warning,
                                "ALLOW" | "INFO" => p.success,
                                _ => p.text_secondary,
                            };
                            ui.label(
                                egui::RichText::new(format!("[{}]", log.action))
                                    .size(12.0)
                                    .color(level_color)
                                    .family(egui::FontFamily::Monospace),
                            );
                            ui.label(
                                egui::RichText::new(&log.message)
                                    .size(12.0)
                                    .color(p.text_primary),
                            );
                        });
                        ui.add_space(2.0);
                    }
                });

            let viewport_h = scroll.inner_rect.height();
            let max_offset = (scroll.content_size.y - viewport_h).max(0.0);
            if scroll.state.offset.y < max_offset - 2.0 {
                self.live_logs_follow = false;
            }
        });
    }

    fn show_enforcements(&self, ui: &mut egui::Ui, p: &Palette) {
        ui.label(page_title("Security Policies"));
        ui.add_space(6.0);
        ui.label(theme::muted_text(
            "Active constraints applied by the VoidCore engine.",
        ));
        ui.add_space(20.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                policy_card(ui, p, "Network Layer", |ui, p| {
                    ui::policy_bullet(ui, p, "OS DNS locked to Mullvad Ad/Tracker/Malware filter.");
                    ui::policy_bullet(
                        ui,
                        p,
                        "Chromium DNS-over-HTTPS cryptographically forced.",
                    );
                    ui::policy_bullet(
                        ui,
                        p,
                        &format!(
                            "Firewall outbound drops active for {} blocked domains.",
                            self.config.url_blocklist.len()
                        ),
                    );
                });

                ui.add_space(12.0);

                policy_card(ui, p, "Application Layer", |ui, p| {
                    ui::policy_bullet(ui, p, "Incognito and InPrivate browsing disabled.");
                    ui::policy_bullet(
                        ui,
                        p,
                        "Chromium extensions blocked globally (wildcard rule).",
                    );
                    ui::policy_bullet(
                        ui,
                        p,
                        "Bitwarden allowed via ExtensionInstallAllowlist.",
                    );
                });

                ui.add_space(12.0);

                policy_card(ui, p, "Execution Layer", |ui, p| {
                    ui::policy_bullet(
                        ui,
                        p,
                        &format!(
                            "Whitelist enforcement ({} explicit applications).",
                            self.config.whitelist.len()
                        ),
                    );
                    ui::policy_bullet(
                        ui,
                        p,
                        &format!(
                            "Authenticode validation ({} trusted publishers).",
                            self.config.trusted_publishers.len()
                        ),
                    );
                    ui::policy_bullet(
                        ui,
                        p,
                        "15-minute timebomb heuristic on verified installers.",
                    );
                });
            });
    }

    fn show_launchpad(&mut self, ui: &mut egui::Ui, p: &Palette) {
        ui.label(page_title("Secure Elevation"));
        ui.add_space(6.0);
        ui.label(theme::body_text(
            "Whitelisted installers and tools may need Administrator rights. UAC would normally block them \
             on a locked-down account. Provide a path from a trusted publisher and the daemon will launch \
             it in a privileged desktop session.",
        ));
        ui.add_space(24.0);

        card_frame(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(section_title("Executable"));
            ui.add_space(12.0);

            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Path")
                        .size(13.0)
                        .color(p.text_muted),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.elevate_path)
                        .desired_width(f32::max(ui.available_width() - 50.0, 280.0))
                        .margin(egui::vec2(12.0, 10.0))
                        .hint_text("C:\\Path\\To\\application.exe"),
                );
            });

            ui.add_space(12.0);
            ui::drop_hint_frame(ui, p, !self.elevate_path.trim().is_empty());

            ui.add_space(16.0);

            if ui::primary_button(ui, p, "Launch Elevated", true).clicked() {
                let msg = send_elevate_command(&self.elevate_path);
                self.update_message_ok = !msg.to_lowercase().contains("fail");
                self.update_message = Some(msg);
            }
        });
    }

    fn show_logs(&mut self, ui: &mut egui::Ui, p: &Palette) {
        ui.horizontal(|ui| {
            ui.label(page_title("Audit Logs"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui::secondary_button(ui, p, "Refresh").clicked() {
                    self.reload_logs();
                }
            });
        });
        ui.add_space(6.0);
        ui.label(theme::muted_text(
            "Process enforcement events from enforce.log.",
        ));
        ui.add_space(16.0);

        egui::Frame::none()
            .fill(p.bg_card)
            .stroke(egui::Stroke::new(1.0, p.border_subtle))
            .rounding(egui::Rounding::same(10.0))
            .inner_margin(egui::Margin::same(12.0))
            .show(ui, |ui| {
                if self.logs.is_empty() {
                    ui::empty_state(
                        ui,
                        p,
                        "No log entries yet",
                        "Blocked or terminated events will appear here from enforce.log.",
                    );
                    return;
                }

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(400.0)
                    .show(ui, |ui| {
                        egui::Grid::new("logs_grid")
                            .striped(true)
                            .spacing([24.0, 10.0])
                            .min_col_width(120.0)
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new("TIMESTAMP")
                                        .size(11.0)
                                        .color(p.text_muted),
                                );
                                ui.label(
                                    egui::RichText::new("SOURCE")
                                        .size(11.0)
                                        .color(p.text_muted),
                                );
                                ui.label(
                                    egui::RichText::new("ACTION")
                                        .size(11.0)
                                        .color(p.text_muted),
                                );
                                ui.label(
                                    egui::RichText::new("DETAILS")
                                        .size(11.0)
                                        .color(p.text_muted),
                                );
                                ui.end_row();

                                for log in &self.logs {
                                    let action_color = match log.action.as_str() {
                                        "BLOCK" => p.danger,
                                        "ALLOW" => p.success,
                                        _ => p.text_secondary,
                                    };
                                    let source_color = p.text_muted;

                                    ui.label(
                                        egui::RichText::new(&log.time)
                                            .size(13.0)
                                            .color(p.text_muted)
                                            .family(egui::FontFamily::Monospace),
                                    );
                                    ui.label(
                                        egui::RichText::new(&log.source)
                                            .size(13.0)
                                            .color(source_color),
                                    );
                                    ui.label(
                                        egui::RichText::new(&log.action)
                                            .strong()
                                            .size(13.0)
                                            .color(action_color),
                                    );
                                    ui.label(
                                        egui::RichText::new(&log.message)
                                            .size(13.0)
                                            .color(p.text_primary),
                                    );
                                    ui.end_row();
                                }
                            });
                    });
            });
    }
}

fn policy_card(
    ui: &mut egui::Ui,
    p: &Palette,
    title: &str,
    body: impl FnOnce(&mut egui::Ui, &Palette),
) {
    card_frame(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(section_title(title));
        ui.add_space(10.0);
        body(ui, p);
    });
}

// ============================================================================
// IPC
// ============================================================================

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn connect_to_daemon() -> Result<std::fs::File, ()> {
    let pipe_name = to_wide(r"\\.\pipe\voidcore_ipc");
    unsafe {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        };

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
        return "Update requested. The daemon will hot-swap and reboot in ~15 seconds.".to_string();
    }
    "Failed to reach daemon. Verify the VoidCore service is running.".to_string()
}

fn send_elevate_command(path: &str) -> String {
    if path.trim().is_empty() {
        return "Please provide a valid executable path.".to_string();
    }
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
    "Failed to communicate with the background daemon.".to_string()
}
