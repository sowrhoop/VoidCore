use std::io::{BufRead, BufReader, Write};
use std::fs::{self, OpenOptions};
use std::path::Path;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::{Arc, Mutex};
use std::os::windows::io::{FromRawHandle, RawHandle};
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
            use windows::Win32::System::Pipes::{CreateNamedPipeW, ConnectNamedPipe, DisconnectNamedPipe, PIPE_ACCESS_DUPLEX, PIPE_TYPE_MESSAGE, PIPE_READMODE_MESSAGE, PIPE_WAIT, PIPE_UNLIMITED_INSTANCES, GetNamedPipeClientProcessId};
            use windows::Win32::Foundation::{INVALID_HANDLE_VALUE, HANDLE, HLOCAL};
            use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
            use windows::Win32::Security::{OpenProcessToken, GetTokenInformation, TokenUser, TOKEN_USER, ConvertSidToStringSidW};
            use windows::Win32::System::Memory::LocalFree;

            let pipe_name = r"\\.\pipe\voidcore_ipc";
            let mut wide: Vec<u16> = OsStr::new(pipe_name).encode_wide().collect();
            wide.push(0);

            loop {
                let handle = CreateNamedPipeW(
                    PCWSTR(wide.as_ptr()),
                    PIPE_ACCESS_DUPLEX.0,
                    (PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT).0,
                    PIPE_UNLIMITED_INSTANCES.0,
                    4096, 4096, 0, None,
                );

                if handle.0 == INVALID_HANDLE_VALUE.0 {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    continue;
                }

                let _ = ConnectNamedPipe(handle, None);
                let mut file = std::fs::File::from_raw_handle(handle.0 as *mut _);
                
                // Clone the handle for the reader so we don't drop the pipe early
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

                    // Advanced SID Verification to ensure caller matches installer
                    if authorized && token_path.exists() {
                        if let Ok(expected_sid) = fs::read_to_string(install_dir.join("installer.sid")) {
                            let expected_sid = expected_sid.trim().to_string();
                            let mut client_pid: u32 = 0;
                            
                            if GetNamedPipeClientProcessId(handle, &mut client_pid).as_bool() && client_pid != 0 {
                                if let Ok(proc_handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, client_pid) {
                                    let mut token = HANDLE(0);
                                    if OpenProcessToken(proc_handle, 8u32, &mut token).as_bool() {
                                        let mut size: u32 = 0;
                                        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut size);
                                        if size > 0 {
                                            let mut buf = vec![0u8; size as usize];
                                            if GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr() as *mut _), size, &mut size).as_bool() {
                                                let user_ptr = buf.as_ptr() as *const TOKEN_USER;
                                                let mut sid_str_ptr = PWSTR::null();
                                                
                                                if ConvertSidToStringSidW((*user_ptr).User.Sid, &mut sid_str_ptr).as_bool() {
                                                    let mut len = 0usize;
                                                    while *sid_str_ptr.0.add(len) != 0 { len += 1; }
                                                    let sid = String::from_utf16_lossy(std::slice::from_raw_parts(sid_str_ptr.0, len));
                                                    
                                                    if sid.trim() != expected_sid { authorized = false; }
                                                    
                                                    // Critical: prevent memory leak from LocalAlloc
                                                    let _ = LocalFree(HLOCAL(sid_str_ptr.0 as *mut _));
                                                }
                                            }
                                        }
                                    }
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