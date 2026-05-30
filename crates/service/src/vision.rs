//! Offline NSFW screen guard (MobileNetV2 ONNX, GantMan/nsfw_model labels).
//! Captures the active user's desktop locally — no network, no image storage.

use image::{imageops::FilterType, RgbaImage};
use ndarray::Array4;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::TensorRef;
use std::ffi::OsStr;
use std::io::Read;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const MODEL_DIR: &str = r"C:\ProgramData\VoidCore\models";
const MODEL_FILE: &str = "nsfw_mobilenet2_224.onnx";
const MODEL_URL: &str =
    "https://raw.githubusercontent.com/Kazuhito00/nsfw_model_onnx_sample/main/model/nsfw_mobilenet2_224.onnx";

const SCAN_INTERVAL: Duration = Duration::from_secs(5);
const LOCK_THRESHOLD: f32 = 0.85;
const LOCK_COOLDOWN: Duration = Duration::from_secs(60);
const INPUT_SIZE: u32 = 224;

/// GantMan/nsfw_model class order (alphabetical in training code).
const CLASS_LABELS: [&str; 5] = ["drawings", "hentai", "neutral", "porn", "sexy"];

static LAST_CAPTURE_WARN: Mutex<Option<(String, Instant)>> = Mutex::new(None);
static HELPER_MODE_LOGGED: AtomicBool = AtomicBool::new(false);

/// CLI entry for a short-lived capture helper spawned in the user's session.
pub fn try_run_snapshot_cli() -> bool {
    let args: Vec<String> = std::env::args().collect();
    let Some(out_path) = args
        .windows(2)
        .find(|w| w[0] == "--vision-snapshot")
        .map(|w| w[1].clone())
    else {
        return false;
    };

    match capture_desktop_direct() {
        Ok(Some(img)) => {
            let code = if img.save(&out_path).is_ok() { 0 } else { 1 };
            std::process::exit(code);
        }
        Ok(None) => std::process::exit(2),
        Err(_) => std::process::exit(1),
    }
}

pub fn start_nsfw_guard() {
    thread::Builder::new()
        .name("voidcore-nsfw-guard".into())
        .spawn(|| {
            if let Err(e) = run_guard_loop() {
                let _ = super::logging::log_event(
                    "vision",
                    "ERROR",
                    &format!("NSFW guard stopped: {e}"),
                );
            }
        })
        .ok();
}

fn run_guard_loop() -> Result<(), String> {
    let model_path = ensure_model()?;
    let session = Mutex::new(build_session(&model_path)?);

    let _ = super::logging::log_event(
        "vision",
        "INFO",
        "NSFW guard online (MobileNetV2 ONNX, local-only inference)",
    );

    let mut last_lock = Instant::now() - LOCK_COOLDOWN;

    loop {
        thread::sleep(SCAN_INTERVAL);

        if last_lock.elapsed() < LOCK_COOLDOWN {
            continue;
        }

        let capture = match capture_active_desktop() {
            Ok(Some(img)) => img,
            Ok(None) => continue,
            Err(e) => {
                log_capture_skip(&e);
                continue;
            }
        };

        let scores = match infer(&session, &capture) {
            Ok(s) => s,
            Err(e) => {
                let _ = super::logging::log_event(
                    "vision",
                    "WARN",
                    &format!("Inference failed: {e}"),
                );
                continue;
            }
        };

        let illicit = illicit_score(&scores);
        if illicit < LOCK_THRESHOLD {
            continue;
        }

        let label = top_illicit_label(&scores);
        if lock_active_session().is_ok() {
            last_lock = Instant::now();
            let _ = super::logging::log_event(
                "vision",
                "BLOCK",
                &format!(
                    "Locked workstation | trigger={label}={illicit:.3} threshold={LOCK_THRESHOLD} | scores: {}",
                    format_scores(&scores)
                ),
            );
        }
    }
}

fn log_capture_skip(msg: &str) {
    let mut guard = LAST_CAPTURE_WARN.lock().unwrap();
    let now = Instant::now();
    if let Some((ref prev, ref t)) = *guard {
        if prev == msg && t.elapsed() < Duration::from_secs(60) {
            return;
        }
    }
    *guard = Some((msg.to_string(), now));
    let _ = super::logging::log_event(
        "vision",
        "WARN",
        &format!("Screen capture skipped: {msg}"),
    );
}

fn ensure_model() -> Result<String, String> {
    let dir = Path::new(MODEL_DIR);
    let path = dir.join(MODEL_FILE);
    if path.exists() {
        return Ok(path.to_string_lossy().into_owned());
    }

    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let _ = super::logging::log_event(
        "vision",
        "INFO",
        "Downloading MobileNetV2 NSFW ONNX model (~10 MB, one-time)...",
    );

    let response = ureq::get(MODEL_URL)
        .call()
        .map_err(|e| format!("model download failed: {e}"))?;

    if response.status() != 200 {
        return Err(format!("model download HTTP {}", response.status()));
    }

    let mut reader = response.into_reader();
    let mut data = Vec::new();
    reader.read_to_end(&mut data).map_err(|e| e.to_string())?;

    std::fs::write(&path, &data).map_err(|e| e.to_string())?;
    let _ = super::logging::log_event("vision", "INFO", "NSFW model cached locally");
    Ok(path.to_string_lossy().into_owned())
}

fn build_session(model_path: &str) -> Result<Session, String> {
    Session::builder()
        .map_err(|e| e.to_string())?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(|e| e.to_string())?
        .with_intra_threads(1)
        .map_err(|e| e.to_string())?
        .with_inter_threads(1)
        .map_err(|e| e.to_string())?
        .commit_from_file(model_path)
        .map_err(|e| e.to_string())
}

fn infer(session: &Mutex<Session>, frame: &RgbaImage) -> Result<[f32; 5], String> {
    let input = preprocess(frame)?;
    let contiguous = input.as_standard_layout();

    let mut guard = session
        .lock()
        .map_err(|_| "session lock poisoned".to_string())?;
    let outputs = guard
        .run(
            ort::inputs![TensorRef::from_array_view(contiguous.view())
                .map_err(|e| e.to_string())?],
        )
        .map_err(|e| e.to_string())?;

    let (_, data) = outputs[0]
        .try_extract_tensor::<f32>()
        .map_err(|e| e.to_string())?;

    if data.len() < 5 {
        return Err(format!("unexpected output length {}", data.len()));
    }

    Ok([data[0], data[1], data[2], data[3], data[4]])
}

fn preprocess(frame: &RgbaImage) -> Result<Array4<f32>, String> {
    let resized = image::imageops::resize(frame, INPUT_SIZE, INPUT_SIZE, FilterType::Triangle);
    let mut tensor = Array4::<f32>::zeros((1, INPUT_SIZE as usize, INPUT_SIZE as usize, 3));

    for (x, y, pixel) in resized.enumerate_pixels() {
        let r = pixel[0] as f32 / 255.0;
        let g = pixel[1] as f32 / 255.0;
        let b = pixel[2] as f32 / 255.0;
        tensor[[0, y as usize, x as usize, 0]] = r;
        tensor[[0, y as usize, x as usize, 1]] = g;
        tensor[[0, y as usize, x as usize, 2]] = b;
    }

    Ok(tensor)
}

fn illicit_score(scores: &[f32; 5]) -> f32 {
    // drawings=0, hentai=1, neutral=2, porn=3, sexy=4
    scores[1].max(scores[3]).max(scores[4])
}

fn format_scores(scores: &[f32; 5]) -> String {
    CLASS_LABELS
        .iter()
        .zip(scores.iter())
        .map(|(label, score)| format!("{label}={score:.3}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn top_illicit_label(scores: &[f32; 5]) -> &'static str {
    let mut best = 1usize;
    let mut val = scores[1];
    for idx in [1usize, 3, 4] {
        if scores[idx] > val {
            val = scores[idx];
            best = idx;
        }
    }
    CLASS_LABELS[best]
}

fn is_desktop_access_error(err: &str) -> bool {
    err.contains("OpenInputDesktop")
        || err.contains("OpenDesktop")
        || err.contains("Incorrect function")
        || err.contains("SetThreadDesktop")
        || err.contains("OpenWindowStation")
        || err.contains("SetProcessWindowStation")
}

fn capture_active_desktop() -> Result<Option<RgbaImage>, String> {
    let guard = match ActiveUserGuard::new() {
        Ok(g) => g,
        Err(_) => return Ok(None),
    };

    let inline = (|| {
        ensure_interactive_window_station()?;
        capture_input_desktop()
    })();

    match inline {
        Ok(img) => Ok(img),
        Err(e) if is_desktop_access_error(&e) => {
            drop(guard);
            match capture_via_user_session_helper() {
                Ok(img) => {
                    if !HELPER_MODE_LOGGED.swap(true, Ordering::Relaxed) {
                        let _ = super::logging::log_event(
                            "vision",
                            "INFO",
                            "Screen capture using user-session helper (Session 0 service)",
                        );
                    }
                    Ok(img)
                }
                Err(helper_err) => Err(format!("inline: {e}; helper: {helper_err}")),
            }
        }
        Err(e) => Err(e),
    }
}

fn capture_desktop_direct() -> Result<Option<RgbaImage>, String> {
    bitblt_screen()
}

fn lock_active_session() -> Result<(), String> {
    let _guard = ActiveUserGuard::new()?;
    unsafe {
        use windows::Win32::System::Shutdown::LockWorkStation;
        LockWorkStation().map_err(|e| format!("LockWorkStation: {e}"))
    }
}

struct ActiveUserGuard {
    token: windows::Win32::Foundation::HANDLE,
}

impl ActiveUserGuard {
    fn new() -> Result<Self, String> {
        unsafe {
            use windows::Win32::Foundation::CloseHandle;
            use windows::Win32::Security::{
                DuplicateTokenEx, ImpersonateLoggedOnUser, SecurityImpersonation, TokenImpersonation,
                TOKEN_ALL_ACCESS,
            };
            use windows::Win32::System::RemoteDesktop::{
                WTSGetActiveConsoleSessionId, WTSQueryUserToken,
            };

            let session = WTSGetActiveConsoleSessionId();
            if session == 0xFFFF_FFFF {
                return Err("no active console session".into());
            }

            let mut session_token = windows::Win32::Foundation::HANDLE::default();
            WTSQueryUserToken(session, &mut session_token).map_err(|e| e.to_string())?;

            let mut imp_token = windows::Win32::Foundation::HANDLE::default();
            DuplicateTokenEx(
                session_token,
                TOKEN_ALL_ACCESS,
                None,
                SecurityImpersonation,
                TokenImpersonation,
                &mut imp_token,
            )
            .map_err(|e| {
                let _ = CloseHandle(session_token);
                e.to_string()
            })?;
            let _ = CloseHandle(session_token);

            ImpersonateLoggedOnUser(imp_token).map_err(|e| {
                let _ = CloseHandle(imp_token);
                e.to_string()
            })?;

            Ok(Self { token: imp_token })
        }
    }
}

impl Drop for ActiveUserGuard {
    fn drop(&mut self) {
        unsafe {
            use windows::Win32::Foundation::CloseHandle;
            use windows::Win32::Security::RevertToSelf;
            let _ = RevertToSelf();
            if self.token.0 != 0 {
                let _ = CloseHandle(self.token);
            }
        }
    }
}

fn ensure_interactive_window_station() -> Result<(), String> {
    static WINSTA: OnceLock<Result<(), String>> = OnceLock::new();
    WINSTA
        .get_or_init(|| unsafe {
            use windows::Win32::Foundation::GetLastError;
            use windows::Win32::System::StationsAndDesktops::{
                CloseWindowStation, OpenWindowStationW, SetProcessWindowStation,
            };

            // WINSTA_ALL_ACCESS (0x37F) — not exported as a named constant in windows 0.52.
            const WINSTA_ALL: u32 = 0x0000_037F;

            let mut name = to_wide("WinSta0");
            let winsta = OpenWindowStationW(
                windows::core::PCWSTR(name.as_mut_ptr()),
                false,
                WINSTA_ALL,
            )
            .map_err(|e| format!("OpenWindowStation: {e}"))?;

            if SetProcessWindowStation(winsta).is_err() {
                let _ = CloseWindowStation(winsta);
                return Err(format!(
                    "SetProcessWindowStation: {:?}",
                    GetLastError()
                ));
            }
            Ok(())
        })
        .clone()
}

fn capture_via_user_session_helper() -> Result<Option<RgbaImage>, String> {
    let session = unsafe { windows::Win32::System::RemoteDesktop::WTSGetActiveConsoleSessionId() };
    if session == 0xFFFF_FFFF {
        return Ok(None);
    }

    let snapshot_path = std::env::temp_dir().join(format!(
        "voidcore-vision-{}.png",
        std::process::id()
    ));
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let cmd = format!(
        "\"{}\" --vision-snapshot \"{}\"",
        exe.display(),
        snapshot_path.display()
    );

    unsafe {
        use windows::core::{PCWSTR, PWSTR};
        use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
        use windows::Win32::Security::{
            DuplicateTokenEx, SecurityImpersonation, TokenPrimary, TOKEN_ALL_ACCESS,
        };
        use windows::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
        use windows::Win32::System::RemoteDesktop::WTSQueryUserToken;
        use windows::Win32::System::Threading::{
            CreateProcessAsUserW, GetExitCodeProcess, WaitForSingleObject, CREATE_NO_WINDOW,
            CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTUPINFOW,
        };

        let mut user_token = HANDLE::default();
        WTSQueryUserToken(session, &mut user_token)
            .map_err(|e| format!("WTSQueryUserToken: {e}"))?;

        let mut primary = HANDLE::default();
        DuplicateTokenEx(
            user_token,
            TOKEN_ALL_ACCESS,
            None,
            SecurityImpersonation,
            TokenPrimary,
            &mut primary,
        )
        .map_err(|e| {
            let _ = CloseHandle(user_token);
            format!("DuplicateTokenEx: {e}")
        })?;
        let _ = CloseHandle(user_token);

        let mut env_block = std::ptr::null_mut();
        let _ = CreateEnvironmentBlock(&mut env_block, primary, false);

        let mut cmd_wide = to_wide(&cmd);
        let mut desktop_wide = to_wide("winsta0\\default");

        let mut si = STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            lpDesktop: PWSTR(desktop_wide.as_mut_ptr()),
            ..Default::default()
        };
        let mut pi = PROCESS_INFORMATION::default();

        let create_result = CreateProcessAsUserW(
            primary,
            PCWSTR::null(),
            PWSTR(cmd_wide.as_mut_ptr()),
            None,
            None,
            false,
            CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW,
            if env_block.is_null() {
                None
            } else {
                Some(env_block as *const _)
            },
            PCWSTR::null(),
            &mut si,
            &mut pi,
        );

        if !env_block.is_null() {
            let _ = DestroyEnvironmentBlock(env_block);
        }
        let _ = CloseHandle(primary);

        create_result.map_err(|e| format!("CreateProcessAsUserW: {e}"))?;

        let wait = WaitForSingleObject(pi.hProcess, 15000);
        let mut exit_code = 1u32;
        let _ = GetExitCodeProcess(pi.hProcess, &mut exit_code);
        let _ = CloseHandle(pi.hProcess);
        let _ = CloseHandle(pi.hThread);

        if wait != WAIT_OBJECT_0 || exit_code != 0 {
            let _ = std::fs::remove_file(&snapshot_path);
            return Err(format!("snapshot helper exit={exit_code}"));
        }
    }

    if !snapshot_path.exists() {
        return Err("snapshot file missing".into());
    }

    let img = image::open(&snapshot_path)
        .map_err(|e| e.to_string())?
        .to_rgba8();
    let _ = std::fs::remove_file(&snapshot_path);
    Ok(Some(img))
}

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn open_interactive_desktop() -> Result<windows::Win32::System::StationsAndDesktops::HDESK, String> {
    unsafe {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::GetLastError;
        use windows::Win32::System::StationsAndDesktops::{
            OpenDesktopW, OpenInputDesktop, DESKTOP_ACCESS_FLAGS, DESKTOP_CONTROL_FLAGS,
            DESKTOP_ENUMERATE, DESKTOP_READOBJECTS,
        };

        let access = DESKTOP_READOBJECTS.0 | DESKTOP_ENUMERATE.0;

        let mut default_name = to_wide("Default");
        if let Ok(desk) = OpenDesktopW(
            PCWSTR(default_name.as_mut_ptr()),
            DESKTOP_CONTROL_FLAGS(0),
            false,
            access,
        ) {
            return Ok(desk);
        }

        if let Ok(desk) =
            OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_ACCESS_FLAGS(access))
        {
            return Ok(desk);
        }

        Err(format!("OpenDesktop: {:?}", GetLastError()))
    }
}

fn capture_input_desktop() -> Result<Option<RgbaImage>, String> {
    unsafe {
        use windows::Win32::Foundation::GetLastError;
        use windows::Win32::System::StationsAndDesktops::{
            CloseDesktop, GetThreadDesktop, SetThreadDesktop,
        };
        use windows::Win32::System::Threading::GetCurrentThreadId;

        let desktop = open_interactive_desktop()?;
        let old_desktop = GetThreadDesktop(GetCurrentThreadId())
            .map_err(|e| format!("GetThreadDesktop: {e}"))?;
        if SetThreadDesktop(desktop).is_err() {
            let _ = CloseDesktop(desktop);
            return Err(format!("SetThreadDesktop: {:?}", GetLastError()));
        }

        let result = bitblt_screen();

        let _ = SetThreadDesktop(old_desktop);
        let _ = CloseDesktop(desktop);
        result
    }
}

fn bitblt_screen() -> Result<Option<RgbaImage>, String> {
    unsafe {
        use windows::Win32::Graphics::Gdi::{
            BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
            GetDC, GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER,
            BI_RGB, DIB_RGB_COLORS, SRCCOPY,
        };
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

        let width = GetSystemMetrics(SM_CXSCREEN) as u32;
        let height = GetSystemMetrics(SM_CYSCREEN) as u32;
        if width == 0 || height == 0 {
            return Ok(None);
        }

        let hdc_screen = GetDC(None);
        if hdc_screen.0 == 0 {
            return Err("GetDC failed".into());
        }

        let hdc_mem = CreateCompatibleDC(hdc_screen);
        let hbm = CreateCompatibleBitmap(hdc_screen, width as i32, height as i32);
        let old_bm = SelectObject(hdc_mem, hbm);
        let blt_ok = BitBlt(
            hdc_mem,
            0,
            0,
            width as i32,
            height as i32,
            hdc_screen,
            0,
            0,
            SRCCOPY,
        );

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let stride = (width * 4) as usize;
        let mut buffer = vec![0u8; stride * height as usize];
        let lines = if blt_ok.is_ok() {
            GetDIBits(
                hdc_mem,
                hbm,
                0,
                height,
                Some(buffer.as_mut_ptr() as *mut _),
                &mut bmi,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };

        SelectObject(hdc_mem, old_bm);
        let _ = DeleteObject(hbm);
        let _ = DeleteDC(hdc_mem);
        let _ = ReleaseDC(None, hdc_screen);

        if lines == 0 {
            return Err("GetDIBits failed".into());
        }

        let mut img = RgbaImage::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let i = (y as usize * stride) + (x as usize * 4);
                let b = buffer[i];
                let g = buffer[i + 1];
                let r = buffer[i + 2];
                let a = buffer[i + 3];
                img.put_pixel(x, y, image::Rgba([r, g, b, a]));
            }
        }

        Ok(Some(img))
    }
}
