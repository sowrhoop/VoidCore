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
        return Ok(false);
    }

    let exe_asset = release.assets.iter()
        .find(|a| a.name == "voidcore-service.exe")
        .ok_or("Release has no service exe asset")?;

    let sig_asset = release.assets.iter()
        .find(|a| a.name == format!("{}.sig", exe_asset.name))
        .ok_or("Release has no signature asset")?;

    let mut exe_bytes = Vec::new();
    ureq::get(&exe_asset.browser_download_url).call()?.into_reader().read_to_end(&mut exe_bytes)?;

    let mut sig_bytes = Vec::new();
    ureq::get(&sig_asset.browser_download_url).call()?.into_reader().read_to_end(&mut sig_bytes)?;

    if sig_bytes.len() < 64 { return Err("Signature file too short".into()); }

    let pub_bytes = hex::decode(&cfg.pubkey_hex)?;
    let pub_array: [u8; 32] = pub_bytes.as_slice().try_into().map_err(|_| "Public key must be 32 bytes")?;
    let public_key = VerifyingKey::from_bytes(&pub_array)?;

    let sig_array: [u8; 64] = sig_bytes[..64].try_into().map_err(|_| "Signature must be 64 bytes")?;
    
    // Corrected to directly bind without `?`
    let signature = Signature::from_bytes(&sig_array);

    public_key.verify(&exe_bytes, &signature).map_err(|_| "Signature verification FAILED")?;

    let target = Path::new(INSTALL_DIR).join(&exe_asset.name);
    let old = Path::new(INSTALL_DIR).join(format!("{}.old", &exe_asset.name));
    let staging = Path::new(INSTALL_DIR).join(format!("{}.new", &exe_asset.name));

    fs::write(&staging, &exe_bytes)?;
    let _ = fs::remove_file(&old);
    if target.exists() { fs::rename(&target, &old)?; }
    
    if let Err(e) = fs::rename(&staging, &target) {
        if old.exists() { let _ = fs::rename(&old, &target); }
        return Err(e.into());
    }

    write_registry_version(cfg.version_code);
    Ok(true)
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