//! Offline NSFW screen guard (MobileNetV2 ONNX, GantMan/nsfw_model labels).
//! Captures the active user's desktop locally — no network, no image storage.

use image::{imageops::FilterType, RgbaImage};
use ndarray::Array4;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::TensorRef;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

const INSTALL_DIR: &str = r"C:\ProgramData\VoidCore";
const MODEL_DIR: &str = r"C:\ProgramData\VoidCore\models";
const MODEL_FILE: &str = "nsfw_mobilenet2_224.onnx";
const MODEL_URL: &str =
    "https://raw.githubusercontent.com/Kazuhito00/nsfw_model_onnx_sample/main/model/nsfw_mobilenet2_224x224.onnx";

const SCAN_INTERVAL: Duration = Duration::from_secs(5);
const LOCK_THRESHOLD: f32 = 0.85;
const LOCK_COOLDOWN: Duration = Duration::from_secs(60);
const INPUT_SIZE: u32 = 224;

/// GantMan/nsfw_model class order (alphabetical in training code).
const CLASS_LABELS: [&str; 5] = ["drawings", "hentai", "neutral", "porn", "sexy"];

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
    init_ort_runtime()?;
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
                let _ = super::logging::log_event(
                    "vision",
                    "WARN",
                    &format!("Screen capture skipped: {e}"),
                );
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

fn init_ort_runtime() -> Result<(), String> {
    if let Ok(path) = std::env::var("ORT_DYLIB_PATH") {
        let path = PathBuf::from(path);
        if path.is_file() {
            ort::init_from(&path)
                .map_err(|e| e.to_string())?
                .commit();
            return Ok(());
        }
    }

    for candidate in ort_dylib_candidates() {
        if candidate.is_file() {
            ort::init_from(&candidate)
                .map_err(|e| e.to_string())?
                .commit();
            return Ok(());
        }
    }

    Err(format!(
        "onnxruntime.dll not found (expected beside voidcore-service.exe or in {INSTALL_DIR})"
    ))
}

fn ort_dylib_candidates() -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from(INSTALL_DIR).join("onnxruntime.dll")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            paths.push(dir.join("onnxruntime.dll"));
        }
    }
    paths
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

fn capture_active_desktop() -> Result<Option<RgbaImage>, String> {
    let _guard = match ActiveUserGuard::new() {
        Ok(g) => g,
        Err(_) => return Ok(None),
    };
    capture_input_desktop()
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
            use windows::Win32::Security::ImpersonateLoggedOnUser;
            use windows::Win32::System::RemoteDesktop::{
                WTSGetActiveConsoleSessionId, WTSQueryUserToken,
            };

            let session = WTSGetActiveConsoleSessionId();
            if session == 0xFFFF_FFFF {
                return Err("no active console session".into());
            }

            let mut token = windows::Win32::Foundation::HANDLE::default();
            WTSQueryUserToken(session, &mut token).map_err(|e| e.to_string())?;
            ImpersonateLoggedOnUser(token).map_err(|e| e.to_string())?;

            Ok(Self { token })
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

fn capture_input_desktop() -> Result<Option<RgbaImage>, String> {
    unsafe {
        use windows::Win32::Foundation::GetLastError;
        use windows::Win32::Graphics::Gdi::{
            BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
            GetDC, GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER,
            BI_RGB, DIB_RGB_COLORS, SRCCOPY,
        };
        use windows::Win32::System::StationsAndDesktops::{
            CloseDesktop, GetThreadDesktop, OpenInputDesktop, SetThreadDesktop,
            DESKTOP_ACCESS_FLAGS, DESKTOP_CONTROL_FLAGS, DESKTOP_ENUMERATE, DESKTOP_READOBJECTS,
        };
        use windows::Win32::System::Threading::GetCurrentThreadId;
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

        let access = DESKTOP_ACCESS_FLAGS(DESKTOP_READOBJECTS.0 | DESKTOP_ENUMERATE.0);
        let desktop = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, access)
            .map_err(|e| format!("OpenInputDesktop: {e}"))?;
        let old_desktop = GetThreadDesktop(GetCurrentThreadId())
            .map_err(|e| format!("GetThreadDesktop: {e}"))?;
        if SetThreadDesktop(desktop).is_err() {
            let _ = CloseDesktop(desktop);
            return Err(format!("SetThreadDesktop: {:?}", GetLastError()));
        }

        let width = GetSystemMetrics(SM_CXSCREEN) as u32;
        let height = GetSystemMetrics(SM_CYSCREEN) as u32;
        if width == 0 || height == 0 {
            let _ = SetThreadDesktop(old_desktop);
            let _ = CloseDesktop(desktop);
            return Ok(None);
        }

        let hdc_screen = GetDC(None);
        if hdc_screen.0 == 0 {
            let _ = SetThreadDesktop(old_desktop);
            let _ = CloseDesktop(desktop);
            return Err("GetDC failed".into());
        }

        let hdc_mem = CreateCompatibleDC(hdc_screen);
        let hbm = CreateCompatibleBitmap(hdc_screen, width as i32, height as i32);
        let old_bm = SelectObject(hdc_mem, hbm);
        let blt_ok = BitBlt(hdc_mem, 0, 0, width as i32, height as i32, hdc_screen, 0, 0, SRCCOPY);

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
        let _ = SetThreadDesktop(old_desktop);
        let _ = CloseDesktop(desktop);

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
