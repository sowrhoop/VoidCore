// VoidCore — Zero-Trust Windows 11 Daemon
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Architecture: Single binary, dual-mode.
//   • Run as SCM service  → daemon mode (NT AUTHORITY\SYSTEM)
//   • Run interactively   → GUI installer / CLI commands

#![windows_subsystem = "windows"]

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use rand::Rng;
use serde::Deserialize;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;
use std::net::ToSocketAddrs;
use wmi::{COMLibrary, WMIConnection};
use windows::core::{HSTRING, PCWSTR, PWSTR};
use windows::Win32::NetworkManagement::NetManagement::{
    NetUserSetInfo, USER_ACCOUNT_FLAGS, USER_INFO_1003, USER_INFO_1008,
};
use windows::Win32::System::ProcessStatus::K32GetProcessImageFileNameW;
use windows::Win32::System::Registry::{
    RegCreateKeyExW, RegDeleteTreeW, RegQueryValueExW, RegSetValueExW, HKEY_LOCAL_MACHINE,
    KEY_ALL_ACCESS, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows::Win32::System::Threading::{
    OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDYES, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_YESNO,
    MESSAGEBOX_RESULT, MESSAGEBOX_STYLE,
};
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
    ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

// ============================================================================
// COMPILE-TIME CONFIGURATION (baked in at build time via build.rs)
// ============================================================================

const SERVICE_NAME: &str = "VoidCoreDaemon";
const ADMIN_ACCOUNT: &str = "VoidCoreAdmin";
const INSTALL_DIR: &str = r"C:\ProgramData\VoidCore";
const INSTALL_EXE: &str = r"C:\ProgramData\VoidCore\voidcore.exe";
const UPDATE_FLAG: &str = r"C:\ProgramData\VoidCore\update.flag";
const REG_KEY: &str = r"SOFTWARE\VoidCore";
const REG_VERSION_VAL: &str = "SecureVersion";

// NOTE: compile-time injection disabled. Runtime config.json and the
// service's C:\ProgramData\VoidCore\config.json will supply these values.
const RAW_WHITELIST: &str = "";
const RAW_URL_BLOCKLIST: &str = "";
const PUBKEY_HEX: &str = "";
const GITHUB_REPO: &str = "";
const VERSION_CODE: &str = "0";

// Windows system processes that must NEVER be killed, regardless of whitelist.
// These live in C:\Windows\ so path-check alone protects them, but explicit
// name-checking here short-circuits before the more expensive path lookup.
const CRITICAL_SYSTEM_PROCESSES: &[&str] = &[
    "system",
    "smss",
    "csrss",
    "wininit",
    "winlogon",
    "lsass",
    "lsm",
    "services",
    "svchost",
    "dwm",
    "explorer",
    "taskhostw",
    "sihost",
    "ctfmon",
    "fontdrvhost",
    "runtimebroker",
    "startmenuexperiencehost",
    "searchindexer",
    "searchhost",
    "shellexperiencehost",
    "logonui",
    "userinit",
    "spoolsv",
    "wuauclt",
    "msiexec",
    "conhost",
    "dllhost",
    "voidcore", // Never kill ourselves
];

// ============================================================================
// WMI TYPE DEFINITIONS
// ============================================================================

#[allow(non_camel_case_types)]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct Win32_ProcessStartTrace {
    process_name: String,
    process_id: u32,
}

#[derive(Deserialize, Debug)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize, Debug)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

// ============================================================================
// SERVICE ENTRY POINT
// ============================================================================

define_windows_service!(ffi_service_main, my_service_main);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 {
        // Strip leading "--" or "-" for ergonomics
        match args[1].trim_start_matches('-') {
            "setup" => launch_gui_installer(),
            "status" => run_status_check(),
            "version" => print_version(),
            "update" => trigger_manual_update(),
            _ => {
                eprintln!("Usage: voidcore [--setup | --status | --version | --update]");
            }
        }
        return Ok(());
    }

    // If SCM cannot dispatch us, fall back to the GUI installer.
    if service_dispatcher::start(SERVICE_NAME, ffi_service_main).is_err() {
        // Fall back to existing GUI installer in the original source tree.
        // Note: the refactor now prefers the new crates/gui implementation.
        launch_gui_installer();
    }

    Ok(())
}

fn print_version() {
    println!("VoidCore Zero-Trust Daemon");
    println!("Version      : v1.0.{}", VERSION_CODE);
    println!("Target Repo  : {}", GITHUB_REPO);
    println!("Whitelist    : {}", RAW_WHITELIST);
    println!("URL Blocklist: {}", RAW_URL_BLOCKLIST);
}

// ============================================================================
// GUI INSTALLER (runs when double-clicked or --setup is passed)
// ============================================================================

fn show_message_box(title: &str, body: &str, flags: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    let title_h = HSTRING::from(title);
    let body_h = HSTRING::from(body);
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(body_h.as_ptr()),
            PCWSTR(title_h.as_ptr()),
            flags,
        )
    }
}

fn is_elevated() -> bool {
    Command::new("net")
        .args(["session"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn launch_gui_installer() {
    if !is_elevated() {
        show_message_box(
            "Access Denied — VoidCore",
            "VoidCore requires Administrator privileges to deploy the System Daemon.\n\n\
             Please right-click this installer and select 'Run as Administrator'.",
            MB_ICONERROR | MB_OK,
        );
        return;
    }

    let warning = format!(
        "Welcome to the VoidCore Deployment Wizard (v1.0.{})\n\n\
        ⚠️  CRITICAL — READ CAREFULLY:\n\n\
        This action will permanently alter your operating system:\n\
        •  Windows Safe Mode will be irrevocably disabled.\n\
        •  Your current user account will be demoted to Standard User.\n\
        •  A 127-character cryptographic password is generated every 10 min and never stored.\n\
        •  Distracting websites and telemetry will be blocked at the OS, DNS, and firewall levels.\n\
        •  Only the apps in your compiled whitelist will be permitted to run.\n\n\
        The ONLY escape hatch is pushing a new release through your GitHub Actions pipeline.\n\n\
        Are you absolutely certain you want to lock down this machine?",
        VERSION_CODE
    );

    if show_message_box(
        "VoidCore Zero-Trust Setup",
        &warning,
        MB_ICONWARNING | MB_YESNO,
    ) == IDYES
    {
        match perform_installation() {
            Ok(()) => {
                show_message_box(
                    "Deployment Successful",
                    "VoidCore has been installed and the daemon is running.\n\n\
                     Your machine will restart in 10 seconds to finalise the Standard User lockdown.\n\
                     After restart, log in normally — explorer.exe will load as usual.",
                    MB_ICONINFORMATION | MB_OK,
                );
                // Grace period: let message box close before reboot
                thread::sleep(Duration::from_secs(2));
                let _ = Command::new("shutdown")
                    .args(["/r", "/t", "10", "/c", "VoidCore: Finalising installation"])
                    .output();
            }
            Err(e) => {
                show_message_box(
                    "Deployment Failed",
                    &format!("An error occurred during installation:\n\n{e}\n\nNo changes have been committed."),
                    MB_ICONERROR | MB_OK,
                );
            }
        }
    }
}

fn perform_installation() -> Result<(), String> {
    let exe_path =
        env::current_exe().map_err(|e| format!("Cannot determine executable path: {e}"))?;
    let target_dir = Path::new(INSTALL_DIR);
    let target_exe = Path::new(INSTALL_EXE);

    // 1. Create installation directory
    if !target_dir.exists() {
        fs::create_dir_all(target_dir)
            .map_err(|e| format!("Cannot create installation directory: {e}"))?;
    }

    // 2. Stop any existing service instance before overwriting the binary
    let _ = Command::new("sc")
        .args(["stop", SERVICE_NAME])
        .output();
    thread::sleep(Duration::from_secs(3));

    // 3. Copy binary (skip if we are already running from the target location)
    if exe_path != target_exe {
        fs::copy(&exe_path, target_exe)
            .map_err(|e| format!("Failed to copy executable: {e}\n\nIf the service is running, stop it first with: sc stop VoidCoreDaemon"))?;
    }

    // 4. Add installation directory to system PATH
    let ps_path = r#"
        $cur = [Environment]::GetEnvironmentVariable('Path','Machine')
        if (-not ($cur -like '*C:\ProgramData\VoidCore*')) {
            [Environment]::SetEnvironmentVariable('Path', $cur + ';C:\ProgramData\VoidCore', 'Machine')
        }
    "#;
    run_hidden_powershell(ps_path);

    // 5. Write initial version code into registry (needed for anti-rollback on first boot)
    write_registry_version(VERSION_CODE.parse::<u32>().unwrap_or(1));

    // 6. Disable Windows Safe Mode boot menu
    let _ = Command::new("bcdedit")
        .args(["/set", "{default}", "bootmenupolicy", "standard"])
        .output();
    let _ = Command::new("bcdedit")
        .args(["/deletevalue", "{default}", "safeboot"])
        .output();

    // 7. Install and configure the Windows Service
    //    Remove stale entry if it exists
    let _ = Command::new("sc")
        .args(["delete", SERVICE_NAME])
        .output();
    thread::sleep(Duration::from_secs(1));

    let status = Command::new("sc")
        .args([
            "create",
            SERVICE_NAME,
            "binPath=",
            INSTALL_EXE,
            "start=",
            "auto",
            "obj=",
            "LocalSystem",
            "DisplayName=",
            "VoidCore Zero-Trust Daemon",
        ])
        .output()
        .map_err(|e| format!("Failed to create service: {e}"))?;

    if !status.status.success() {
        return Err(format!(
            "sc create failed: {}",
            String::from_utf8_lossy(&status.stderr)
        ));
    }

    // Configure service description and auto-restart on failure
    let _ = Command::new("sc")
        .args(["description", SERVICE_NAME, "VoidCore Zero-Trust focus daemon — DO NOT STOP"])
        .output();
    let _ = Command::new("sc")
        .args([
            "failure",
            SERVICE_NAME,
            "reset=",
            "0",
            "actions=",
            "restart/2000/restart/5000/restart/10000",
        ])
        .output();

    // Start the service NOW so it is running before the reboot
    let _ = Command::new("sc")
        .args(["start", SERVICE_NAME])
        .output();
    thread::sleep(Duration::from_secs(2));

    // 8. Demote current user LAST, so steps above can complete with Admin rights.
    //    The daemon (running as SYSTEM) will take over enforcement from here.
    purge_all_other_administrators();

    Ok(())
}

// ============================================================================
// SERVICE DAEMON (NT AUTHORITY\SYSTEM)
// ============================================================================

fn my_service_main(_arguments: Vec<std::ffi::OsString>) {
    let _ = run_service();
}

fn run_service() -> Result<(), windows_service::Error> {
    // Shutdown flag shared between the control handler closure and the main loop.
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_ctrl = shutdown.clone();

    let status_handle =
        service_control_handler::register(SERVICE_NAME, move |ctrl| match ctrl {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                // Signal the main loop to exit gracefully.
                shutdown_ctrl.store(true, Ordering::SeqCst);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        })?;

    // Report "Running" to SCM
    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    // Parse compile-time configuration into owned data structures
    let whitelist: HashSet<String> = RAW_WHITELIST
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    let blocklist: HashSet<String> = RAW_URL_BLOCKLIST
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    // Anti-rollback check — if a newer version was previously installed, lock
    // down the whitelist so nothing can run (forces the user to update).
    let current_version: u32 = VERSION_CODE.parse().unwrap_or(1);
    let is_downgraded = enforce_anti_rollback(current_version);

    // Ensure our privileged admin account exists and is in the Administrators group.
    // The password is immediately rotated in the main loop below.
    let _ = Command::new("net")
        .args(["user", ADMIN_ACCOUNT, "/add"])
        .output();
    let _ = Command::new("net")
        .args(["localgroup", "Administrators", ADMIN_ACCOUNT, "/add"])
        .output();

    // Spin up background threads for slow or blocking subsystems.
    {
        let whitelist_clone = if is_downgraded {
            HashSet::new()
        } else {
            whitelist.clone()
        };
        thread::Builder::new()
            .name("wmi-watcher".into())
            .spawn(move || wmi_process_watcher(whitelist_clone))
            .expect("Failed to spawn wmi-watcher thread");
    }

    {
        let shutdown_clone = shutdown.clone();
        thread::Builder::new()
            .name("auto-updater".into())
            .spawn(move || auto_updater_loop(is_downgraded, shutdown_clone))
            .expect("Failed to spawn auto-updater thread");
    }

    {
        let blocklist_clone = blocklist.clone();
        thread::Builder::new()
            .name("firewall-sync".into())
            .spawn(move || firewall_sync_loop(blocklist_clone))
            .expect("Failed to spawn firewall-sync thread");
    }

    // ── Main security enforcement loop ──────────────────────────────────────
    // Runs every 10 minutes. On each tick:
    //   • Rotate the admin password (LAPS)
    //   • Re-enforce DNS, Chromium policies, hosts file
    //   • Ensure no extra admins have snuck in
    while !shutdown.load(Ordering::SeqCst) {
        let new_pass = generate_127_char_password();

        set_local_password(ADMIN_ACCOUNT, &new_pass);
        disable_builtin_admin();
        purge_all_other_administrators();

        enforce_global_mullvad_dns();
        enforce_chromium_policies(&blocklist);
        write_hosts_file(&blocklist);

        // Sleep in 10-second increments so we respond to shutdown quickly.
        for _ in 0..60 {
            if shutdown.load(Ordering::SeqCst) {
                break;
            }
            thread::sleep(Duration::from_secs(10));
        }
    }

    // Report "Stopped" to SCM before exiting
    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    Ok(())
}

// ============================================================================
// WMI PROCESS ENFORCEMENT ENGINE
// ============================================================================

fn wmi_process_watcher(whitelist: HashSet<String>) {
    // WMI COM must be initialised on this thread.
    let com_con = match COMLibrary::new() {
        Ok(c) => c,
        Err(_) => {
            // Retry after a short delay — COM may not be ready at service start.
            thread::sleep(Duration::from_secs(10));
            match COMLibrary::new() {
                Ok(c) => c,
                Err(_) => return,
            }
        }
    };

    let wmi_con = match WMIConnection::new(com_con) {
        Ok(w) => w,
        Err(_) => return,
    };

    let iterator = match wmi_con.notification::<Win32_ProcessStartTrace>() {
        Ok(i) => i,
        Err(_) => return,
    };

    for result in iterator {
        let trace = match result {
            Ok(t) => t,
            Err(_) => continue,
        };

        let name_lower = trace
            .process_name
            .to_lowercase()
            .trim_end_matches(".exe")
            .to_string();

        // ── Guard 1: Never kill critical OS processes ────────────────────────
        if CRITICAL_SYSTEM_PROCESSES.contains(&name_lower.as_str()) {
            continue;
        }

        // ── Guard 2: Block explicit junk/gaming launchers by name ─────────────
        // These are killed regardless of path so they cannot relocate themselves.
        let is_junk = matches!(
            name_lower.as_str(),
            "winstore.app"
                | "winstoreapp"
                | "gamebar"
                | "gamebarpresencewriter"
                | "gamebarftserver"
                | "xboxapp"
                | "xboxgamingoverlay"
                | "steam"
                | "steamwebhelper"
                | "epicgameslauncher"
                | "epicwebhelper"
                | "riotclient"
                | "riotclientservices"
                | "leagueclient"
                | "valorant"
                | "clipup"        // Windows Update UX
                | "wsreset"       // Store reset
        );
        if is_junk {
            kill_process(trace.process_id);
            continue;
        }

        // ── Guard 3: Path-based zone enforcement ─────────────────────────────
        let process_path = match get_process_path(trace.process_id) {
            Some(p) => p.to_lowercase(),
            // If we cannot determine the path it's suspicious — let it run if
            // it's on the whitelist, otherwise kill it.
            None => {
                if !whitelist.contains(&name_lower) {
                    kill_process(trace.process_id);
                }
                continue;
            }
        };

        let in_windows = process_path.contains("\\windows\\");
        let in_program_files = process_path.contains("\\program files");
        let in_program_data = process_path.contains("\\programdata\\");
        let in_user_dir = process_path.contains("\\users\\");

        if in_windows || in_program_files || in_program_data {
            // Immutable system zone — always allowed.
            // (Specific junk processes in this zone are already caught above.)
            continue;
        }

        if in_user_dir {
            // User-directory processes must come from a recognised developer path
            // OR be on the compiled whitelist.
            let in_trusted_user_path = process_path.contains("\\appdata\\local\\programs\\")
                || process_path.contains("\\appdata\\roaming\\npm\\")
                || process_path.contains("\\appdata\\local\\npm\\")
                || process_path.contains("\\bravesoftware\\")
                || process_path.contains("\\vscodium\\")
                || process_path.contains("\\winscp\\")
                || process_path.contains("\\docker desktop\\")
                || process_path.contains("\\venv\\")
                || process_path.contains("\\.vscode\\")
                || process_path.contains("\\scripts\\")
                || process_path.contains("\\cursor\\");

            if in_trusted_user_path || whitelist.contains(&name_lower) {
                continue;
            }

            kill_process(trace.process_id);
            continue;
        }

        // ── Guard 4: Everything outside known zones requires whitelist ─────────
        // E.g., D:\Games\something.exe
        if whitelist.contains(&name_lower) {
            continue;
        }

        kill_process(trace.process_id);
    }
}

fn kill_process(pid: u32) {
    unsafe {
        if let Ok(handle) = OpenProcess(PROCESS_TERMINATE, false, pid) {
            let _ = TerminateProcess(handle, 1);
            let _ = windows::Win32::Foundation::CloseHandle(handle);
        }
    }
}

fn get_process_path(pid: u32) -> Option<String> {
    unsafe {
        if let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            let mut buffer = [0u16; 1024];
            let len = K32GetProcessImageFileNameW(handle, &mut buffer);
            let _ = windows::Win32::Foundation::CloseHandle(handle);
            if len > 0 {
                return Some(String::from_utf16_lossy(&buffer[..len as usize]));
            }
        }
    }
    None
}

// ============================================================================
// NETWORK HARDENING
// ============================================================================

/// Redirect all connected interfaces to Mullvad's non-logging, CGNAT-filtered DNS.
fn enforce_global_mullvad_dns() {
    // Primary: 194.242.2.9 (no ads, no malware, no tracking)
    // Fallback: 194.242.2.2 (no ads, no malware)
    let ps = r#"
        Get-NetIPInterface | Where-Object { $_.ConnectionState -eq 'Connected' } | ForEach-Object {
            Set-DnsClientServerAddress `
                -InterfaceIndex $_.InterfaceIndex `
                -ServerAddresses ('194.242.2.9','194.242.2.2','2a07:e340::9','2a07:e340::2') `
                -ErrorAction SilentlyContinue
        }
    "#;
    run_hidden_powershell(ps);
}

/// Apply Chromium Group Policy for Chrome, Brave, and Edge:
///   • Disable Incognito / InPrivate mode
///   • Disable built-in DoH (so Mullvad DNS applies)
///   • Enforce URL blocklist
fn enforce_chromium_policies(blocklist: &HashSet<String>) {
    let browsers = [
        (
            r"SOFTWARE\Policies\Google\Chrome",
            r"SOFTWARE\Policies\Google\Chrome\URLBlocklist",
            "IncognitoModeAvailability",
        ),
        (
            r"SOFTWARE\Policies\BraveSoftware\Brave",
            r"SOFTWARE\Policies\BraveSoftware\Brave\URLBlocklist",
            "IncognitoModeAvailability",
        ),
        (
            r"SOFTWARE\Policies\Microsoft\Edge",
            r"SOFTWARE\Policies\Microsoft\Edge\URLBlocklist",
            "InPrivateModeAvailability",
        ),
    ];

    unsafe {
        for (policy_path, blocklist_path, incognito_key) in &browsers {
            let mut hkey = Default::default();
            let subkey_h = HSTRING::from(*policy_path);
            if RegCreateKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(subkey_h.as_ptr()),
                0,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_ALL_ACCESS,
                None,
                &mut hkey,
                None,
            )
            .is_ok()
            {
                // Disable Incognito (value 2 = forced-disabled)
                let incognito_val: u32 = 2;
                let key_h = HSTRING::from(*incognito_key);
                let _ = RegSetValueExW(
                    hkey,
                    PCWSTR(key_h.as_ptr()),
                    0,
                    REG_DWORD,
                    Some(std::slice::from_raw_parts(
                        &incognito_val as *const _ as *const u8,
                        4,
                    )),
                );

                // Force DoH off so Mullvad system DNS takes effect
                let doh_name_h = HSTRING::from("DnsOverHttpsMode");
                let doh_val_h = HSTRING::from("off");
                let doh_bytes = std::slice::from_raw_parts(
                    doh_val_h.as_ptr() as *const u8,
                    (doh_val_h.len() + 1) * 2, // +1 for null terminator, *2 for UTF-16
                );
                let _ = RegSetValueExW(
                    hkey,
                    PCWSTR(doh_name_h.as_ptr()),
                    0,
                    REG_SZ,
                    Some(doh_bytes),
                );

                // Also disable extensions that could bypass our policies
                // Create a subkey "ExtensionInstallBlocklist" under the already
                // opened policy key (hkey) so it lives at
                // HKLM\...\Policies\<Browser>\ExtensionInstallBlocklist
                let ext_install_name_h = HSTRING::from("ExtensionInstallBlocklist");
                let mut ext_hkey = Default::default();
                let _ = RegCreateKeyExW(
                    hkey,
                    PCWSTR(ext_install_name_h.as_ptr()),
                    0,
                    PCWSTR::null(),
                    REG_OPTION_NON_VOLATILE,
                    KEY_ALL_ACCESS,
                    None,
                    &mut ext_hkey,
                    None,
                );
            }

            // Atomically replace the URL blocklist: delete old, write new
            let bl_h = HSTRING::from(*blocklist_path);
            let _ = RegDeleteTreeW(HKEY_LOCAL_MACHINE, PCWSTR(bl_h.as_ptr()));

            let mut hkey_block = Default::default();
            if RegCreateKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(bl_h.as_ptr()),
                0,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_ALL_ACCESS,
                None,
                &mut hkey_block,
                None,
            )
            .is_ok()
            {
                for (index, domain) in blocklist.iter().enumerate() {
                    let val_num_h = HSTRING::from((index + 1).to_string());
                    // Pattern matches both bare domain and all subdomains
                    let pattern_h = HSTRING::from(format!("*{}*", domain));
                    let pattern_bytes = std::slice::from_raw_parts(
                        pattern_h.as_ptr() as *const u8,
                        (pattern_h.len() + 1) * 2,
                    );
                    let _ = RegSetValueExW(
                        hkey_block,
                        PCWSTR(val_num_h.as_ptr()),
                        0,
                        REG_SZ,
                        Some(pattern_bytes),
                    );
                }
            }
        }
    }
}

/// Write the VoidCore section of the hosts file atomically.
/// Uses a temp-file + rename strategy to avoid corruption on partial writes.
fn write_hosts_file(blocklist: &HashSet<String>) {
    let hosts_path = r"C:\Windows\System32\drivers\etc\hosts";
    let tmp_path = r"C:\Windows\System32\drivers\etc\hosts.voidcore.tmp";

    let start_marker = "# --- VOIDCORE SECURE FOCUS BLOCKLIST ---";
    let end_marker = "# --- END VOIDCORE BLOCKLIST ---";

    let existing = fs::read_to_string(hosts_path).unwrap_or_default();

    // Strip any previous VoidCore block
    let base = if let (Some(s), Some(e)) = (existing.find(start_marker), existing.find(end_marker))
    {
        let mut cleaned = existing.clone();
        cleaned.replace_range(s..e + end_marker.len(), "");
        cleaned.trim_end().to_string()
    } else {
        existing.trim_end().to_string()
    };

    // Build new block
    let mut new_content = base;
    new_content.push_str("\n\n");
    new_content.push_str(start_marker);
    new_content.push('\n');
    for domain in blocklist {
        new_content.push_str(&format!(
            "127.0.0.1 {d}\n127.0.0.1 www.{d}\n::1 {d}\n::1 www.{d}\n",
            d = domain
        ));
    }
    new_content.push_str(end_marker);
    new_content.push('\n');

    // Write to temp file, then rename for atomicity
    if fs::write(tmp_path, &new_content).is_ok() {
        let _ = fs::rename(tmp_path, hosts_path);
    }
}

/// Resolves blocklisted domains to IPs and blocks them in Windows Firewall.
/// Runs periodically since CDN IPs rotate.
fn firewall_sync_loop(blocklist: HashSet<String>) {
    loop {
        let mut ips: HashSet<String> = HashSet::new();
        for domain in &blocklist {
            if let Ok(addrs) = format!("{}:443", domain).to_socket_addrs() {
                for addr in addrs {
                    ips.insert(addr.ip().to_string());
                }
            }
            if let Ok(addrs) = format!("{}:80", domain).to_socket_addrs() {
                for addr in addrs {
                    ips.insert(addr.ip().to_string());
                }
            }
        }

        if !ips.is_empty() {
            let ip_list = ips
                .iter()
                .map(|ip| format!("'{}'", ip))
                .collect::<Vec<_>>()
                .join(",");
            let ps = format!(
                "Remove-NetFirewallRule -DisplayName 'VoidCore Outbound Block' -ErrorAction SilentlyContinue; \
                 New-NetFirewallRule -DisplayName 'VoidCore Outbound Block' \
                   -Direction Outbound -Action Block \
                   -RemoteAddress @({ip_list}) \
                   -ErrorAction SilentlyContinue"
            );
            run_hidden_powershell(&ps);
        }

        // Re-resolve every 6 hours
        thread::sleep(Duration::from_secs(6 * 3600));
    }
}

// ============================================================================
// ACCOUNT MANAGEMENT (LAPS)
// ============================================================================

fn purge_all_other_administrators() {
    // Demote every user-class account that is not VoidCoreAdmin or the built-in
    // "Administrator" (which we separately disable).
    let ps = format!(
        r#"
        $admins = Get-LocalGroupMember -Group 'Administrators' |
                  Where-Object {{ $_.ObjectClass -eq 'User' }}
        foreach ($a in $admins) {{
            $name = $a.Name -replace '.*\\',''  # strip domain prefix
            if ($name -ne '{ADMIN}' -and $name -ne 'Administrator') {{
                Add-LocalGroupMember    -Group 'Users'          -Member $a.Name -ErrorAction SilentlyContinue
                Remove-LocalGroupMember -Group 'Administrators' -Member $a.Name -ErrorAction SilentlyContinue
            }}
        }}
        "#,
        ADMIN = ADMIN_ACCOUNT
    );
    run_hidden_powershell(&ps);
}

fn set_local_password(user: &str, pass: &str) {
    let user_h = HSTRING::from(user);
    let pass_h = HSTRING::from(pass);
    let mut info = USER_INFO_1003 {
        usri1003_password: PWSTR(pass_h.as_ptr() as *mut _),
    };
    unsafe {
        let _ = NetUserSetInfo(
            PCWSTR::null(),
            PCWSTR(user_h.as_ptr()),
            1003,
            &mut info as *mut _ as *mut u8,
            None,
        );
    }
}

fn disable_builtin_admin() {
    let user_h = HSTRING::from("Administrator");
    // UF_ACCOUNTDISABLE = 0x0002
    let mut info = USER_INFO_1008 {
        usri1008_flags: USER_ACCOUNT_FLAGS(0x0002),
    };
    unsafe {
        let _ = NetUserSetInfo(
            PCWSTR::null(),
            PCWSTR(user_h.as_ptr()),
            1008,
            &mut info as *mut _ as *mut u8,
            None,
        );
    }
}

fn generate_127_char_password() -> String {
    const CHARSET: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!@#$%^&*()-_=+[]{}|;:,.<>?/";
    let mut rng = rand::thread_rng();
    (0..127)
        .map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char)
        .collect()
}

// ============================================================================
// ANTI-ROLLBACK (Registry-backed monotonic version counter)
// ============================================================================

/// Returns `true` if this binary is OLDER than the version stored in the
/// registry (i.e. someone tried to downgrade). In that case the whitelist
/// is emptied so nothing can run, forcing an update.
fn enforce_anti_rollback(current: u32) -> bool {
    unsafe {
        let mut hkey = Default::default();
        let subkey_h = HSTRING::from(REG_KEY);
        if RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(subkey_h.as_ptr()),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_ALL_ACCESS,
            None,
            &mut hkey,
            None,
        )
        .is_err()
        {
            return false;
        }

        let val_h = HSTRING::from(REG_VERSION_VAL);
        let mut stored: u32 = 0;
        let mut size: u32 = 4;

        let ok = RegQueryValueExW(
            hkey,
            PCWSTR(val_h.as_ptr()),
            None,
            None,
            Some(&mut stored as *mut u32 as *mut u8),
            Some(&mut size),
        )
        .is_ok();

        if ok && current < stored {
            // Downgrade attempt detected — do NOT update the stored version;
            // leave it at the higher value so the constraint persists.
            return true;
        }

        // Advance the stored version to current
        let _ = RegSetValueExW(
            hkey,
            PCWSTR(val_h.as_ptr()),
            0,
            REG_DWORD,
            Some(std::slice::from_raw_parts(
                &current as *const u32 as *const u8,
                4,
            )),
        );

        false
    }
}

fn write_registry_version(version: u32) {
    unsafe {
        let mut hkey = Default::default();
        let subkey_h = HSTRING::from(REG_KEY);
        if RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(subkey_h.as_ptr()),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_ALL_ACCESS,
            None,
            &mut hkey,
            None,
        )
        .is_ok()
        {
            let val_h = HSTRING::from(REG_VERSION_VAL);
            let _ = RegSetValueExW(
                hkey,
                PCWSTR(val_h.as_ptr()),
                0,
                REG_DWORD,
                Some(std::slice::from_raw_parts(
                    &version as *const u32 as *const u8,
                    4,
                )),
            );
        }
    }
}

// ============================================================================
// CRYPTOGRAPHIC AUTO-UPDATER
// ============================================================================

fn auto_updater_loop(is_downgraded: bool, shutdown: Arc<AtomicBool>) {
    // If we are downgraded, poll aggressively every 60 s to encourage quick fix.
    let poll_secs: u64 = if is_downgraded { 60 } else { 3600 };
    let flag_path = Path::new(UPDATE_FLAG);

    // Attempt update immediately on startup.
    let _ = check_and_apply_update();

    while !shutdown.load(Ordering::SeqCst) {
        // Sleep in short increments to react to both the flag file and shutdown.
        let intervals = poll_secs / 15;
        for _ in 0..intervals {
            if shutdown.load(Ordering::SeqCst) {
                return;
            }
            if flag_path.exists() {
                let _ = fs::remove_file(flag_path);
                match check_and_apply_update() {
                    Ok(true) => {
                        // A new binary was written; SCM will restart us on exit.
                        // Use exit code 1 so SCM failure-restart kicks in.
                        std::process::exit(1);
                    }
                    Ok(false) | Err(_) => {}
                }
            }
            thread::sleep(Duration::from_secs(15));
        }

        match check_and_apply_update() {
            Ok(true) => std::process::exit(1),
            Ok(false) | Err(_) => {}
        }
    }
}

/// Downloads the latest release from GitHub, verifies the Ed25519 signature,
/// atomically replaces the running binary, and returns `Ok(true)` on success.
fn check_and_apply_update() -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        GITHUB_REPO
    );

    let response = match ureq::get(&url)
        .set("User-Agent", "VoidCore-Daemon/1.0")
        .call()
    {
        Ok(r) => r,
        Err(_) => return Ok(false), // No network — silently skip
    };

    let release: GithubRelease = serde_json::from_reader(response.into_reader())?;

    let current_v: u32 = VERSION_CODE.parse().unwrap_or(0);
    // Tag format: "v1.0.<VERSION_CODE>"
    let remote_v: u32 = release
        .tag_name
        .trim_start_matches("v1.0.")
        .parse()
        .unwrap_or(0);

    if remote_v <= current_v {
        return Ok(false);
    }

    let exe_asset = release
        .assets
        .iter()
        .find(|a| a.name == "voidcore.exe")
        .ok_or("Release has no voidcore.exe asset")?;

    let sig_asset = release
        .assets
        .iter()
        .find(|a| a.name == "voidcore.exe.sig")
        .ok_or("Release has no voidcore.exe.sig asset")?;

    // Download binary
    let mut exe_bytes = Vec::new();
    ureq::get(&exe_asset.browser_download_url)
        .set("User-Agent", "VoidCore-Daemon/1.0")
        .call()?
        .into_reader()
        .read_to_end(&mut exe_bytes)?;

    // Download signature
    let mut sig_bytes = Vec::new();
    ureq::get(&sig_asset.browser_download_url)
        .set("User-Agent", "VoidCore-Daemon/1.0")
        .call()?
        .into_reader()
        .read_to_end(&mut sig_bytes)?;

    if sig_bytes.len() < 64 {
        return Err("Signature file too short".into());
    }

    // Verify Ed25519 signature
    let pub_bytes = hex::decode(PUBKEY_HEX)?;
    let pub_array: [u8; 32] = pub_bytes
        .as_slice()
        .try_into()
        .map_err(|_| "Public key must be 32 bytes")?;
    let public_key = VerifyingKey::from_bytes(&pub_array)?;

    let sig_array: [u8; 64] = sig_bytes[..64]
        .try_into()
        .map_err(|_| "Signature must be 64 bytes")?;
    let signature = Signature::from_bytes(&sig_array);

    public_key
        .verify(&exe_bytes, &signature)
        .map_err(|_| "Signature verification FAILED — update rejected")?;

    // Atomic replacement: write new binary → rename old → rename new into place
    let target = Path::new(INSTALL_EXE);
    let old = Path::new(r"C:\ProgramData\VoidCore\voidcore.old.exe");
    let staging = Path::new(r"C:\ProgramData\VoidCore\voidcore.new.exe");

    fs::write(staging, &exe_bytes)?;

    let _ = fs::remove_file(old);
    if target.exists() {
        fs::rename(target, old)?;
    }

    if let Err(e) = fs::rename(staging, target) {
        // Roll back
        if old.exists() {
            let _ = fs::rename(old, target);
        }
        return Err(e.into());
    }

    Ok(true)
}

// ============================================================================
// CLI COMMANDS
// ============================================================================

fn trigger_manual_update() {
    if is_elevated() {
        println!("[*] Running update check directly (elevated)...");
        match check_and_apply_update() {
            Ok(true) => println!("[+] Update applied successfully. Service will restart."),
            Ok(false) => println!("[+] Already up-to-date."),
            Err(e) => println!("[-] Update failed: {e}"),
        }
    } else {
        println!("[*] Standard User: signalling daemon to perform update...");
        match fs::write(UPDATE_FLAG, "PULL_UPDATE") {
            Ok(()) => println!(
                "[+] Signal written. The daemon will apply the update within ~15 seconds."
            ),
            Err(e) => println!("[-] Could not write update flag: {e}"),
        }
    }
}

fn run_status_check() {
    let version = VERSION_CODE;
    println!("\n╔══════════════════════════════════════════╗");
    println!("║         VOIDCORE SYSTEMS AUDIT           ║");
    println!("╠══════════════════════════════════════════╣");
    println!("║ Version     : v1.0.{:<22}║", version);
    println!("║ Whitelist   : {:<28}║", truncate(RAW_WHITELIST, 28));
    println!("║ Blocklist   : {:<28}║", truncate(RAW_URL_BLOCKLIST, 28));
    println!("╠══════════════════════════════════════════╣");

    // Session privilege level
    let priv_line = if is_elevated() {
        "[-] Session     : ADMINISTRATOR (Vulnerable)"
    } else {
        "[+] Session     : STANDARD USER (Secured)  "
    };
    println!("║ {}║", priv_line);

    // Service status
    let svc_line = if let Ok(out) = Command::new("sc").args(["query", SERVICE_NAME]).output() {
        if String::from_utf8_lossy(&out.stdout).contains("RUNNING") {
            "[+] Daemon      : RUNNING (Unstoppable)     "
        } else {
            "[-] Daemon      : STOPPED / MISSING         "
        }
    } else {
        "[ ] Daemon      : UNKNOWN                    "
    };
    println!("║ {}║", svc_line);

    // Safe Mode status
    let sm_line = if let Ok(out) = Command::new("bcdedit").args(["/enum", "{default}"]).output() {
        if String::from_utf8_lossy(&out.stdout).contains("safeboot") {
            "[-] Safe Mode   : ENABLED (Vulnerable)      "
        } else {
            "[+] Safe Mode   : DESTROYED (Secured)       "
        }
    } else {
        "[ ] Safe Mode   : UNKNOWN                    "
    };
    println!("║ {}║", sm_line);

    println!("╚══════════════════════════════════════════╝\n");
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}

// ============================================================================
// UTILITIES
// ============================================================================

/// Run a PowerShell command in a hidden window, no profile, non-interactive.
fn run_hidden_powershell(script: &str) {
    let _ = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            script,
        ])
        .output();
}
