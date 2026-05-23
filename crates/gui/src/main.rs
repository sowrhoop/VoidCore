// Minimal Win32 GUI skeleton. Provides a single message box for now and will be
// extended with a full window and named-pipe client in subsequent patches.

use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_OK};
use windows::core::HSTRING;
use std::io::{Read, Write};
use std::os::windows::prelude::AsRawHandle;
use std::ptr::null_mut;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

// Simple named-pipe client which attempts to connect to \\.\pipe\voidcore_ipc and
// send a "status" request. The service will be implemented later to reply.
fn main() {
    let pipe_name = r"\\.\\pipe\\voidcore_ipc";

    // Convert pipe name to wide string
    let mut wide: Vec<u16> = OsStr::new(pipe_name).encode_wide().collect();
    wide.push(0);

    unsafe {
        use windows::Win32::System::Pipes::CreateFileW;
        use windows::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
        use windows::Win32::System::Threading::INFINITE;
        use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows::core::PCWSTR;

        let handle = CreateFileW(
            PCWSTR(wide.as_ptr()),
            0x40000000, // GENERIC_WRITE
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            Default::default(),
            None,
        );

        if handle.0 == 0 || handle.0 == INVALID_HANDLE_VALUE.0 {
            let title = HSTRING::from("VoidCore GUI (no service)");
            let body = HSTRING::from("Could not connect to VoidCore service via named pipe.");
            let _ = MessageBoxW(None, body.into(), title.into(), MB_OK);
            return;
        }

        // If connected, write a small request. (This is a best-effort; real
        // protocol will be added later.)
        use std::os::windows::io::FromRawHandle;
        let mut file = std::fs::File::from_raw_handle(handle.0 as *mut _);
        // Read gui.token to attach the TOKEN: header so the service accepts update
        if let Ok(token) = std::fs::read_to_string(r"C:\ProgramData\VoidCore\gui.token") {
            let token = token.trim();
            let _ = file.write_all(format!("TOKEN:{}\n", token).as_bytes());
        } else {
            let _ = file.write_all(b"\n");
        }
        let _ = file.write_all(b"status\n");
        let mut resp = [0u8; 512];
        let _ = file.read(&mut resp);
    }

    let title = HSTRING::from("VoidCore GUI (stub)");
    let body = HSTRING::from("Connected to service (or attempted). GUI features coming soon.");
    unsafe { MessageBoxW(None, body.clone().into(), title.clone().into(), MB_OK) };
}
