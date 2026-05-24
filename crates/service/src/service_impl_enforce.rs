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
            }; // Semicolon here explicitly releases the WMI iterator borrow before context destruction
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
    
    // Correct mapping of PWSTR taking the mutable raw pointer to the U16 buffer string
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