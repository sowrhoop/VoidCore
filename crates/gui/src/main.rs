#![windows_subsystem = "windows"] // Launch without a console window

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use windows::core::{PCWSTR, HSTRING};
use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_OK, MB_ICONINFORMATION, MB_ICONERROR};

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn main() {
    let pipe_name = to_wide(r"\\.\pipe\voidcore_ipc");

    unsafe {
        let handle_result = CreateFileW(
            PCWSTR(pipe_name.as_ptr()),
            0xC0000000, // GENERIC_READ | GENERIC_WRITE
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            Default::default(),
            None,
        );

        let handle = match handle_result {
            Ok(h) if h.0 != 0 && h.0 != INVALID_HANDLE_VALUE.0 => h,
            _ => {
                let title = HSTRING::from("VoidCore Interface");
                let body = HSTRING::from("Error: Could not connect to the VoidCore Background Daemon.\nMake sure the service is running.");
                let _ = MessageBoxW(None, PCWSTR(body.as_ptr()), PCWSTR(title.as_ptr()), MB_OK | MB_ICONERROR);
                return;
            }
        };

        let mut file = std::fs::File::from_raw_handle(handle.0 as *mut _);
        
        // Authenticate
        if let Ok(token) = std::fs::read_to_string(r"C:\ProgramData\VoidCore\gui.token") {
            let _ = file.write_all(format!("TOKEN:{}\n", token.trim()).as_bytes());
        } else {
            let _ = file.write_all(b"\n");
        }
        
        // Request Status
        let _ = file.write_all(b"status\n");
        let _ = file.flush();
        
        // Read exactly one line to avoid blocking on EOF across the privilege boundary
        let mut reader = BufReader::new(file);
        let mut resp = String::new();
        
        if reader.read_line(&mut resp).is_ok() && !resp.trim().is_empty() {
            let title = HSTRING::from("VoidCore System Status");
            let display_text = format!("Daemon responded:\n\n{}", resp.trim());
            let body = HSTRING::from(display_text);
            let _ = MessageBoxW(None, PCWSTR(body.as_ptr()), PCWSTR(title.as_ptr()), MB_OK | MB_ICONINFORMATION);
        } else {
            let title = HSTRING::from("VoidCore Interface");
            let body = HSTRING::from("Error: Connected to daemon, but received no response.");
            let _ = MessageBoxW(None, PCWSTR(body.as_ptr()), PCWSTR(title.as_ptr()), MB_OK | MB_ICONERROR);
        }
    }
}