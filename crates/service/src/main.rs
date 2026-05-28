// Rewritten service entrypoint managing SCM state and threads
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use voidcore_shared::RuntimeConfig;
use windows_service::{define_windows_service, service_dispatcher};
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};

const SERVICE_NAME: &str = "VoidCoreDaemon";

define_windows_service!(ffi_service_main, my_service_main);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Err(_) = service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
        let _ = logging::log_event("core", "WARN", "Running outside of SCM");
        run_service()?;
    }
    Ok(())
}

fn my_service_main(_arguments: Vec<std::ffi::OsString>) {
    if let Err(e) = run_service_scm() {
        let _ = logging::log_event("core", "ERROR", &format!("Service failure: {}", e));
    }
}

fn run_service_scm() -> Result<(), windows_service::Error> {
    let status_handle = service_control_handler::register(SERVICE_NAME, move |ctrl| match ctrl {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            std::process::exit(0);
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;

    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    let _ = run_service();

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

fn run_service() -> Result<(), Box<dyn std::error::Error>> {
    let install_dir = Path::new(r"C:\ProgramData\VoidCore");
    if !install_dir.exists() {
        let _ = fs::create_dir_all(install_dir);
    }

    let cfg_path = install_dir.join("config.json");
    if !cfg_path.exists() {
        let _ = fs::write(&cfg_path, serde_json::to_string_pretty(&RuntimeConfig::default())?);
    }

    let cfg = fs::read_to_string(&cfg_path)
        .ok()
        .and_then(|s| serde_json::from_str::<RuntimeConfig>(&s).ok())
        .unwrap_or_default();
        
    let cfg_handle = Arc::new(Mutex::new(cfg));

    let _ = logging::log_event("core", "INFO", "Initializing subsystems...");
    
    service_impl_ipc::start_ipc_server(cfg_handle.clone());
    service_impl_updater::start_auto_updater(cfg_handle.clone());
    service_impl_enforce::start_enforcement(cfg_handle.clone());

    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

// -----------------------------------------------------------------------------
// INLINE MODULES
// -----------------------------------------------------------------------------

mod logging {
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn log_event(component: &str, level: &str, message: &str) -> std::io::Result<()> {
        let base = Path::new(r"C:\ProgramData\VoidCore");
        let logs = base.join("logs");
        let _ = fs::create_dir_all(&logs);
        let file = logs.join(format!("{}.log", component));
        let mut f = OpenOptions::new().create(true).append(true).open(file)?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        writeln!(f, "{} [{}] {}", now, level, message)?;
        Ok(())
    }
}

mod service_impl_enforce {
    use std::collections::HashSet;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::thread;
    use std::time::Duration;
    use voidcore_shared::RuntimeConfig;
    use wmi::{COMLibrary, WMIConnection};
    use serde::Deserialize;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows::Win32::System::ProcessStatus::K32GetProcessImageFileNameW;
    use windows::Win32::NetworkManagement::NetManagement::{NetUserSetInfo, USER_INFO_1003};
    use windows::Win32::System::Registry::{RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW, HKEY_LOCAL_MACHINE, KEY_ALL_ACCESS, REG_SZ, REG_OPTION_NON_VOLATILE};

    #[allow(non_camel_case_types)]
    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct Win32_ProcessStartTrace {
        process_name: String,
        process_id: u32,
    }

    fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    pub fn start_enforcement(cfg_handle: std::sync::Arc<std::sync::Mutex<RuntimeConfig>>) {
        let cfg_handle_wmi = cfg_handle.clone();
        thread::Builder::new()
            .name("wmi-watcher".into())
            .spawn(move || {
                let com = match COMLibrary::new() {
                    Ok(c) => c,
                    Err(_) => {
                        thread::sleep(Duration::from_secs(5));
                        COMLibrary::new().expect("COM initialization failed fatally")
                    }
                };
                
                let wmi_con = WMIConnection::new(com).unwrap();
                if let Ok(iterator) = wmi_con.notification::<Win32_ProcessStartTrace>() {
                    for result in iterator {
                        if let Ok(trace) = result {
                            let name_lower = trace.process_name.to_lowercase().trim_end_matches(".exe").to_string();
                            let whitelist: HashSet<String> = cfg_handle_wmi.lock().map(|c| c.whitelist.clone()).unwrap_or_default();

                            let critical = ["system","smss","csrss","wininit","winlogon","lsass","services","svchost","explorer","voidcore-service", "voidcore-gui"];
                            if critical.contains(&name_lower.as_str()) {
                                continue;
                            }

                            let mut allow = whitelist.contains(&name_lower);
                            
                            if !allow {
                                unsafe {
                                    if let Ok(proc_handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, trace.process_id) {
                                        let mut buffer = [0u16; 1024];
                                        let len = K32GetProcessImageFileNameW(proc_handle, &mut buffer);
                                        let _ = windows::Win32::Foundation::CloseHandle(proc_handle);
                                        
                                        if len > 0 {
                                            let path = String::from_utf16_lossy(&buffer[..len as usize]).to_lowercase();
                                            let trusted_paths = [
                                                "\\appdata\\local\\programs\\",
                                                "\\appdata\\roaming\\npm\\",
                                                "\\appdata\\local\\npm\\",
                                                "\\bravesoftware\\",
                                                "\\vscodium\\",
                                            ];
                                            for tp in &trusted_paths {
                                                if path.contains(tp) {
                                                    allow = true;
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            if !allow {
                                unsafe {
                                    if let Ok(handle) = OpenProcess(PROCESS_TERMINATE, false, trace.process_id) {
                                        let _ = TerminateProcess(handle, 1);
                                        let _ = windows::Win32::Foundation::CloseHandle(handle);
                                    }
                                }
                            }
                        }
                    }
                };
            })
            .ok();

        let cfg_handle_main = cfg_handle.clone();
        thread::Builder::new()
            .name("main-enforce".into())
            .spawn(move || {
                loop {
                    let cfg = cfg_handle_main.lock().map(|c| c.clone()).unwrap_or_default();
                    write_hosts_file(&cfg.url_blocklist);
                    write_chromium_policies(&cfg.url_blocklist);
                    rotate_local_admin_password();
                    purge_all_other_administrators();
                    thread::sleep(Duration::from_secs(10 * 60)); 
                }
            })
            .ok();
    }

    fn rotate_local_admin_password() {
        use rand::Rng;
        const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!@#$%^&*()-_=+[]{}|;:,.<>?/";
        let mut rng = rand::thread_rng();
        let pass: String = (0..127).map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char).collect();

        let user_w = to_wide("VoidCoreAdmin");
        let mut pass_w = to_wide(&pass);
        
        let mut info = USER_INFO_1003 { usri1003_password: PWSTR(pass_w.as_mut_ptr()) };
        unsafe {
            let _ = NetUserSetInfo(PCWSTR::null(), PCWSTR(user_w.as_ptr()), 1003, &mut info as *mut _ as *mut u8, None);
        }
    }

    fn purge_all_other_administrators() {
        let ps = r#"
            $admins = Get-LocalGroupMember -Group 'Administrators' | Where-Object { $_.ObjectClass -eq 'User' }
            foreach ($a in $admins) {
                $name = $a.Name -replace '.*\\',''
                if ($name -ne 'VoidCoreAdmin' -and $name -ne 'Administrator') {
                    Add-LocalGroupMember -Group 'Users' -Member $a.Name -ErrorAction SilentlyContinue
                    Remove-LocalGroupMember -Group 'Administrators' -Member $a.Name -ErrorAction SilentlyContinue
                }
            }
        "#;
        let _ = std::process::Command::new("powershell").args(["-NoProfile","-NonInteractive","-WindowStyle","Hidden","-Command", ps]).output();
    }

    fn write_hosts_file(blocklist: &std::collections::HashSet<String>) {
        use std::fs;
        let hosts_path = r"C:\Windows\System32\drivers\etc\hosts";
        let tmp_path = r"C:\Windows\System32\drivers\etc\hosts.voidcore.tmp";
        let start_marker = "# --- VOIDCORE SECURE FOCUS BLOCKLIST ---";
        let end_marker = "# --- END VOIDCORE BLOCKLIST ---";

        let existing = fs::read_to_string(hosts_path).unwrap_or_default();
        let base = if let (Some(s), Some(e)) = (existing.find(start_marker), existing.find(end_marker)) {
            let mut cleaned = existing.clone();
            cleaned.replace_range(s..e + end_marker.len(), "");
            cleaned.trim_end().to_string()
        } else {
            existing.trim_end().to_string()
        };

        let mut new_content = base;
        new_content.push_str("\n\n");
        new_content.push_str(start_marker);
        new_content.push('\n');
        for domain in blocklist {
            new_content.push_str(&format!("127.0.0.1 {d}\n127.0.0.1 www.{d}\n::1 {d}\n::1 www.{d}\n", d = domain));
        }
        new_content.push_str(end_marker);
        new_content.push('\n');

        if fs::write(tmp_path, &new_content).is_ok() {
            let _ = fs::rename(tmp_path, hosts_path);
        }
    }

    fn write_chromium_policies(blocklist: &std::collections::HashSet<String>) {
        let browsers = [
            ("SOFTWARE\\Policies\\Google\\Chrome", "SOFTWARE\\Policies\\Google\\Chrome\\URLBlocklist", "IncognitoModeAvailability"),
            ("SOFTWARE\\Policies\\BraveSoftware\\Brave", "SOFTWARE\\Policies\\BraveSoftware\\Brave\\URLBlocklist", "IncognitoModeAvailability"),
            ("SOFTWARE\\Policies\\Microsoft\\Edge", "SOFTWARE\\Policies\\Microsoft\\Edge\\URLBlocklist", "InPrivateModeAvailability"),
        ];

        unsafe {
            for (policy_path, blocklist_path, _incognito_key) in &browsers {
                let subkey_w = to_wide(*policy_path);
                let mut hkey = Default::default();
                let _ = RegCreateKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(subkey_w.as_ptr()), 0, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_ALL_ACCESS, None, &mut hkey, None);

                let bl_w = to_wide(*blocklist_path);
                let _ = RegDeleteTreeW(HKEY_LOCAL_MACHINE, PCWSTR(bl_w.as_ptr()));
                
                let mut hkey_block = Default::default();
                if RegCreateKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(bl_w.as_ptr()), 0, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_ALL_ACCESS, None, &mut hkey_block, None).is_ok() {
                    for (index, domain) in blocklist.iter().enumerate() {
                        let val_num_w = to_wide(&(index + 1).to_string());
                        let pattern_w = to_wide(&format!("*{}*", domain));
                        
                        let pattern_bytes = std::slice::from_raw_parts(pattern_w.as_ptr() as *const u8, pattern_w.len() * 2);
                        let _ = RegSetValueExW(hkey_block, PCWSTR(val_num_w.as_ptr()), 0, REG_SZ, Some(pattern_bytes));
                    }
                }
            }
        }
    }
}

mod service_impl_ipc {
    use std::io::{BufRead, BufReader, Write};
    use std::fs::{self, OpenOptions};
    use std::path::Path;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::{Arc, Mutex};
    use std::os::windows::io::FromRawHandle;
    use voidcore_shared::RuntimeConfig;

    fn log_ipc_auth_failure(install_dir: &Path, token_line: &str, cmd: &str, reason: &str) {
        let logs = install_dir.join("logs");
        let _ = fs::create_dir_all(&logs);
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(logs.join("ipc.log")) {
            let ts = std::time::SystemTime::now().duration_since(std::time::SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs();
            let _ = writeln!(f, "{} [IPC_AUTH_FAIL] token={} cmd={} reason={}", ts, token_line, cmd, reason);
        }
    }

    pub fn start_ipc_server(cfg_handle: Arc<Mutex<RuntimeConfig>>) {
        std::thread::Builder::new()
            .name("ipc-server".into())
            .spawn(move || unsafe {
                use windows::core::{PCWSTR, PWSTR};
                use windows::Win32::System::Pipes::{CreateNamedPipeW, ConnectNamedPipe, DisconnectNamedPipe, PIPE_TYPE_MESSAGE, PIPE_READMODE_MESSAGE, PIPE_WAIT, GetNamedPipeClientProcessId};
                use windows::Win32::Foundation::{HANDLE, HLOCAL, INVALID_HANDLE_VALUE, LocalFree};
                use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, OpenProcessToken};
                use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_USER, TOKEN_ACCESS_MASK};
                use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
                use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;

                let pipe_name = r"\\.\pipe\voidcore_ipc";
                let mut wide: Vec<u16> = OsStr::new(pipe_name).encode_wide().collect();
                wide.push(0);

                loop {
                    let handle = CreateNamedPipeW(
                        PCWSTR(wide.as_ptr()),
                        FILE_FLAGS_AND_ATTRIBUTES(3),
                        PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                        255, 4096, 4096, 0, None,
                    );

                    if handle.0 == INVALID_HANDLE_VALUE.0 || handle.0 == 0 {
                        std::thread::sleep(std::time::Duration::from_secs(5));
                        continue;
                    }

                    let _ = ConnectNamedPipe(handle, None);
                    let mut file = std::fs::File::from_raw_handle(handle.0 as *mut _);
                    
                    if let Ok(file_clone) = file.try_clone() {
                        let mut reader = BufReader::new(file_clone);
                        let mut first_line = String::new();
                        let mut cmd_line = String::new();
                        let _ = reader.read_line(&mut first_line);
                        let _ = reader.read_line(&mut cmd_line);
                        let first_line = first_line.trim().to_string();
                        let cmd_line = cmd_line.trim().to_string();

                        let install_dir = Path::new(r"C:\ProgramData\VoidCore");
                        let token_path = install_dir.join("gui.token");

                        let mut authorized = if token_path.exists() {
                            if let Ok(expected) = fs::read_to_string(&token_path) {
                                first_line.starts_with("TOKEN:") && first_line.trim_start_matches("TOKEN:").trim() == expected.trim()
                            } else { false }
                        } else { true };

                        if authorized && token_path.exists() {
                            if let Ok(expected_sid) = fs::read_to_string(install_dir.join("installer.sid")) {
                                let expected_sid = expected_sid.trim().to_string();
                                let mut client_pid: u32 = 0;
                                
                                if GetNamedPipeClientProcessId(handle, &mut client_pid).is_ok() && client_pid != 0 {
                                    if let Ok(proc_handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, client_pid) {
                                        let mut token = HANDLE(0);
                                        if OpenProcessToken(proc_handle, TOKEN_ACCESS_MASK(8u32), &mut token).is_ok() {
                                            let mut size: u32 = 0;
                                            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut size);
                                            if size > 0 {
                                                let mut buf = vec![0u8; size as usize];
                                                if GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr() as *mut _), size, &mut size).is_ok() {
                                                    let user_ptr = buf.as_ptr() as *const TOKEN_USER;
                                                    let mut sid_str_ptr = PWSTR::null();
                                                    
                                                    if ConvertSidToStringSidW((*user_ptr).User.Sid, &mut sid_str_ptr).is_ok() {
                                                        let mut len = 0usize;
                                                        while *sid_str_ptr.0.add(len) != 0 { len += 1; }
                                                        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(sid_str_ptr.0, len));
                                                        
                                                        if sid.trim() != expected_sid { authorized = false; }
                                                        
                                                        let _ = LocalFree(HLOCAL(sid_str_ptr.0 as *mut _));
                                                    }
                                                }
                                            }
                                        }
                                        let _ = windows::Win32::Foundation::CloseHandle(proc_handle);
                                    }
                                }
                            }
                        }

                        let resp = if !authorized && !cmd_line.eq_ignore_ascii_case("status") {
                            log_ipc_auth_failure(install_dir, &first_line, &cmd_line, "token_or_sid_mismatch");
                            "ERR:unauthorized\n".to_string()
                        } else {
                            match cmd_line.to_lowercase().as_str() {
                                "status" => format!("{{\"service\":\"running\",\"version\":{} }}\n", cfg_handle.lock().map(|c| c.version_code).unwrap_or(0)),
                                "update" => {
                                    let _ = fs::write(install_dir.join("update.flag"), "PULL_UPDATE");
                                    "OK:update_queued\n".to_string()
                                },
                                "rollback" => "ERR:forbidden\n".to_string(),
                                _ => "ERR:unknown_command\n".to_string(),
                            }
                        };

                        let _ = file.write_all(resp.as_bytes());
                        let _ = file.flush();
                    }
                    let _ = DisconnectNamedPipe(handle);
                }
            }).ok();
    }
}

mod service_impl_updater {
    use std::error::Error;
    use std::fs;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use ed25519_dalek::{Signature, VerifyingKey, Verifier};
    use voidcore_shared::RuntimeConfig;

    #[derive(serde::Deserialize, Debug)]
    struct GithubAsset {
        name: String,
        browser_download_url: String,
    }

    #[derive(serde::Deserialize, Debug)]
    struct GithubRelease {
        tag_name: String,
        assets: Vec<GithubAsset>,
    }

    const INSTALL_DIR: &str = r"C:\ProgramData\VoidCore";

    pub fn start_auto_updater(cfg_handle: Arc<Mutex<RuntimeConfig>>) {
        thread::Builder::new()
            .name("auto-updater".into())
            .spawn(move || {
                let _ = attempt_update(&cfg_handle);

                loop {
                    for _ in 0..(3600 / 15) {
                        thread::sleep(Duration::from_secs(15));
                        let flag = Path::new(INSTALL_DIR).join("update.flag");
                        if flag.exists() {
                            let _ = fs::remove_file(&flag);
                            if let Ok(true) = attempt_update(&cfg_handle) {
                                std::process::exit(1);
                            }
                        }
                    }
                    if let Ok(true) = attempt_update(&cfg_handle) {
                        std::process::exit(1);
                    }
                }
            })
            .ok();
    }

    fn attempt_update(cfg_handle: &Arc<Mutex<RuntimeConfig>>) -> Result<bool, Box<dyn Error + Send + Sync>> {
        let cfg = cfg_handle.lock().map_err(|_| "Lock poisoned")?;
        check_and_apply_update(&cfg)
    }

    fn check_and_apply_update(cfg: &RuntimeConfig) -> Result<bool, Box<dyn Error + Send + Sync>> {
        let url = format!("https://api.github.com/repos/{}/releases/latest", cfg.github_repo);

        let response = match ureq::get(&url).set("User-Agent", "VoidCore-Service/1.0").call() {
            Ok(r) => r,
            Err(_) => return Ok(false),
        };

        let release: GithubRelease = serde_json::from_reader(response.into_reader())?;
        let remote_v: u32 = release.tag_name.trim_start_matches("v1.0.").parse().unwrap_or(0);

        if remote_v <= cfg.version_code {
            return Ok(false); // No update required
        }

        let pub_bytes = hex::decode(&cfg.pubkey_hex)?;
        let pub_array: [u8; 32] = pub_bytes.as_slice().try_into().map_err(|_| "Public key must be 32 bytes")?;
        let public_key = VerifyingKey::from_bytes(&pub_array)?;

        // Update BOTH the service and GUI binaries if they exist in the release
        let binaries_to_update = ["voidcore-service.exe", "voidcore-gui.exe"];
        
        let mut updated_at_least_one = false;

        for binary_name in binaries_to_update {
            // Find assets, skip if this specific binary isn't in the release
            let exe_asset = match release.assets.iter().find(|a| a.name == binary_name) {
                Some(asset) => asset,
                None => continue,
            };

            let sig_asset = match release.assets.iter().find(|a| a.name == format!("{}.sig", binary_name)) {
                Some(asset) => asset,
                None => continue,
            };

            let mut exe_bytes = Vec::new();
            ureq::get(&exe_asset.browser_download_url).call()?.into_reader().read_to_end(&mut exe_bytes)?;

            let mut sig_bytes = Vec::new();
            ureq::get(&sig_asset.browser_download_url).call()?.into_reader().read_to_end(&mut sig_bytes)?;

            if sig_bytes.len() < 64 { return Err(format!("Signature file for {} too short", binary_name).into()); }

            let sig_array: [u8; 64] = sig_bytes[..64].try_into().map_err(|_| "Signature must be 64 bytes")?;
            let signature = Signature::from_bytes(&sig_array);

            // Halt immediately if ANY cryptographic verification fails
            public_key.verify(&exe_bytes, &signature)
                .map_err(|_| format!("Signature verification FAILED for {}", binary_name))?;

            let target = Path::new(INSTALL_DIR).join(binary_name);
            let old = Path::new(INSTALL_DIR).join(format!("{}.old", binary_name));
            let staging = Path::new(INSTALL_DIR).join(format!("{}.new", binary_name));

            fs::write(&staging, &exe_bytes)?;
            let _ = fs::remove_file(&old);
            
            // Note: Windows allows renaming an actively running executable file
            if target.exists() { fs::rename(&target, &old)?; }
            
            if let Err(e) = fs::rename(&staging, &target) {
                if old.exists() { let _ = fs::rename(&old, &target); }
                return Err(e.into());
            }
            
            updated_at_least_one = true;
        }

        if updated_at_least_one {
            write_registry_version(remote_v);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn write_registry_version(version: u32) {
        unsafe {
            use windows::Win32::System::Registry::{RegCreateKeyExW, RegSetValueExW, HKEY_LOCAL_MACHINE, KEY_ALL_ACCESS, REG_DWORD, REG_OPTION_NON_VOLATILE};
            use windows::core::PCWSTR;
            
            let subkey_w: Vec<u16> = OsStr::new("SOFTWARE\\VoidCore").encode_wide().chain(std::iter::once(0)).collect();
            let val_w: Vec<u16> = OsStr::new("SecureVersion").encode_wide().chain(std::iter::once(0)).collect();
            
            let mut hkey = Default::default();
            if RegCreateKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(subkey_w.as_ptr()), 0, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_ALL_ACCESS, None, &mut hkey, None).is_ok() {
                let _ = RegSetValueExW(hkey, PCWSTR(val_w.as_ptr()), 0, REG_DWORD, Some(std::slice::from_raw_parts(&version as *const u32 as *const u8, 4)));
            }
        }
    }
}