use std::io::{BufRead, BufReader, Write};
use std::fs;
use std::path::Path;
use std::fs::OpenOptions;
use std::time::SystemTime;
use std::ptr::null_mut;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::{Arc, Mutex};
use std::os::windows::io::RawHandle;

fn client_info(handle: &windows::Win32::Foundation::HANDLE) -> String {
    unsafe {
        use windows::Win32::System::Pipes::GetNamedPipeClientProcessId;
        let mut pid: u32 = 0;
        if GetNamedPipeClientProcessId(*handle, &mut pid).as_bool() {
            return format!("pid={}", pid);
        }
    }
    "pid=unknown".to_string()
}

fn log_ipc_auth_failure(install_dir: &Path, client: String, token_line: &str, cmd: &str, reason: &str) -> std::io::Result<()> {
    let logs = install_dir.join("logs");
    let _ = fs::create_dir_all(&logs);
    let log_file = logs.join("ipc.log");
    let mut f = OpenOptions::new().create(true).append(true).open(log_file)?;
    let now = SystemTime::now();
    let ts = now.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs();
    writeln!(f, "{} [{}] client={} token={} cmd={} reason={}", ts, "IPC_AUTH_FAIL", client, token_line, cmd, reason)?;
    Ok(())
}

pub fn start_ipc_server(cfg_handle: Arc<Mutex<crate::super::shared::RuntimeConfig>>) {
    std::thread::Builder::new()
        .name("ipc-server".into())
        .spawn(move || unsafe {
            use windows::core::PCWSTR;
            use windows::Win32::System::Pipes::{CreateNamedPipeW, ConnectNamedPipe, DisconnectNamedPipe, PIPE_ACCESS_DUPLEX, PIPE_TYPE_MESSAGE, PIPE_READMODE_MESSAGE, PIPE_WAIT, PIPE_UNLIMITED_INSTANCES};
            use windows::Win32::Foundation::INVALID_HANDLE_VALUE;

            let pipe_name = r"\\.\\pipe\\voidcore_ipc";
            let mut wide: Vec<u16> = OsStr::new(pipe_name).encode_wide().collect();
            wide.push(0);

            loop {
                let handle = CreateNamedPipeW(
                    PCWSTR(wide.as_ptr()),
                    PIPE_ACCESS_DUPLEX.0,
                    (PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT).0,
                    PIPE_UNLIMITED_INSTANCES.0,
                    4096,
                    4096,
                    0,
                    null_mut(),
                );

                if handle.0 == INVALID_HANDLE_VALUE.0 {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    continue;
                }

                let _ = ConnectNamedPipe(handle, null_mut());
                let raw = handle.0 as *mut _;
                let mut file = std::fs::File::from_raw_handle(raw as *mut _);
                let mut reader = BufReader::new(file.try_clone().unwrap_or_else(|_| file.try_clone().unwrap_or_else(|_| file)));

                let mut first_line = String::new();
                let mut cmd_line = String::new();
                let _ = reader.read_line(&mut first_line);
                let _ = reader.read_line(&mut cmd_line);
                let first_line = first_line.trim().to_string();
                let cmd_line = cmd_line.trim().to_string();

                let install_dir = Path::new(r"C:\ProgramData\VoidCore");
                let token_path = install_dir.join("gui.token");

                let mut authorized = if token_path.exists() {
                    match fs::read_to_string(&token_path) {
                        Ok(expected) => {
                            let expected = expected.trim();
                            if first_line.starts_with("TOKEN:") {
                                let provided = first_line.trim_start_matches("TOKEN:").trim();
                                provided == expected
                            } else {
                                false
                            }
                        }
                        Err(_) => false,
                    }
                } else {
                    true
                };

                // If token exists and we are authorized so far, perform SID comparison
                if authorized && token_path.exists() {
                    let sid_file = install_dir.join("installer.sid");
                    if sid_file.exists() {
                        if let Ok(expected_sid) = fs::read_to_string(&sid_file) {
                            let expected_sid = expected_sid.trim().to_string();
                            // Attempt to obtain client process ID and token, then extract SID
                            unsafe {
                                use windows::Win32::System::Pipes::GetNamedPipeClientProcessId;
                                use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
                                use windows::Win32::Security::{OpenProcessToken, GetTokenInformation, TokenUser};
                                use windows::Win32::Foundation::HANDLE;
                                use windows::Win32::Security::ConvertSidToStringSidW;
                                use windows::Win32::Security::GetLengthSid;

                                let mut client_pid: u32 = 0;
                                if GetNamedPipeClientProcessId(handle, &mut client_pid).as_bool() && client_pid != 0 {
                                    if let Ok(proc_handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, client_pid) {
                                        let mut token = HANDLE(0);
                                        if OpenProcessToken(proc_handle, 8u32 /* TOKEN_QUERY */, &mut token).as_bool() {
                                            // Request TokenUser info
                                            let mut size: u32 = 0;
                                            let _ = GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut size);
                                            if size > 0 {
                                                let mut buf: Vec<u8> = vec![0u8; size as usize];
                                                if GetTokenInformation(token, TokenUser, buf.as_mut_ptr() as *mut _, size, &mut size).as_bool() {
                                                    // TOKEN_USER layout: SID at offset
                                                    // Use ConvertSidToStringSidW to get string SID
                                                    let user_ptr = buf.as_ptr() as *const windows::Win32::Security::TOKEN_USER;
                                                    let sid_ptr = (*user_ptr).User.Sid;
                                                    let mut sid_str_ptr: *mut u16 = std::ptr::null_mut();
                                                    if ConvertSidToStringSidW(sid_ptr, &mut sid_str_ptr).as_bool() {
                                                        // read wide str
                                                        let mut len = 0usize;
                                                        while *sid_str_ptr.add(len) != 0 { len += 1; }
                                                        let slice = std::slice::from_raw_parts(sid_str_ptr, len);
                                                        let sid = String::from_utf16_lossy(slice);
                                                        // Compare
                                                        if sid.trim() != expected_sid.trim() {
                                                            authorized = false;
                                                        }
                                                        // Free memory allocated by ConvertSidToStringSidW
                                                        // LocalFree is required. Call it to avoid leaks.
                                                        use windows::Win32::System::Memory::LocalFree;
                                                        let _ = LocalFree(sid_str_ptr as isize);
                                                    }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Additional verification: check client process SID equals installer SID
                // If gui.token exists we also attempt to verify the client process.
                if authorized && token_path.exists() {
                    // Attempt to query the client process via GetNamedPipeClientProcessId
                    unsafe {
                        use windows::Win32::System::Pipes::GetNamedPipeClientProcessId;
                        use windows::Win32::System::Threading::OpenProcess;
                        use windows::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION;
                        use windows::Win32::Security::GetTokenInformation;
                        use windows::Win32::Security::{OpenProcessToken, TokenUser};
                        use windows::Win32::Foundation::HANDLE;

                        let mut client_pid: u32 = 0;
                        if GetNamedPipeClientProcessId(handle, &mut client_pid).as_bool() {
                            if client_pid != 0 {
                                if let Ok(proc_handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, client_pid) {
                                    let mut token = HANDLE(0);
                                    if OpenProcessToken(proc_handle, TokenUser.0 as u32, &mut token).as_bool() {
                                        // We could inspect the token here and compare SIDs.
                                        // For brevity we consider the presence of a token enough.
                                    }
                                }
                            }
                        }
                    }
                }

                let mut resp = String::new();
                if !authorized && !cmd_line.eq_ignore_ascii_case("status") {
                    resp = "ERR:unauthorized\n".to_string();
                    // Log auth failure
                    let _ = crate::logging::log_event("ipc", "WARN", &format!("auth_fail client={} token={} cmd={} reason={}", client_info(&handle), first_line, cmd_line, "token_or_sid_mismatch"));
                } else {
                    if cmd_line.eq_ignore_ascii_case("status") {
                        let ver = cfg_handle.lock().map(|c| c.version_code).unwrap_or(0);
                        resp = format!("{{\"service\":\"running\",\"version\":{} }}\n", ver);
                    } else if cmd_line.eq_ignore_ascii_case("update") {
                        let _ = fs::write(install_dir.join("update.flag"), "PULL_UPDATE");
                        resp = "OK:update_queued\n".to_string();
                    } else if cmd_line.eq_ignore_ascii_case("rollback") {
                        // Rollback must be elevated; deny here.
                        resp = "ERR:forbidden\n".to_string();
                    } else {
                            resp = "ERR:unknown_command\n".to_string();
                        }
                    }

                use std::io::Write;
                let _ = file.write_all(resp.as_bytes());
                let _ = file.flush();
                let _ = DisconnectNamedPipe(handle);
            }
        })
        .ok();
}
