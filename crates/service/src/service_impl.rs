// Rewritten service_impl: clean, minimal, and safe. We'll implement IPC
// hardening and runtime config loading here incrementally.

use std::error::Error;
use std::fs;
use std::path::Path;
use std::thread;
use std::time::Duration;
use std::sync::{Arc, Mutex};

use voidcore_shared::RuntimeConfig;

pub fn main() -> Result<(), Box<dyn Error>> {
    // Ensure install dir exists
    let install_dir = Path::new(r"C:\ProgramData\VoidCore");
    if !install_dir.exists() {
        let _ = fs::create_dir_all(install_dir);
    }

    // Ensure runtime config exists
    let cfg_path = install_dir.join("config.json");
    if !cfg_path.exists() {
        let _ = fs::write(&cfg_path, serde_json::to_string_pretty(&RuntimeConfig::default())?);
    }

    // Load runtime config into memory (simple initial load)
    let cfg = if let Ok(s) = fs::read_to_string(&cfg_path) {
        serde_json::from_str::<RuntimeConfig>(&s).unwrap_or_default()
    } else {
        RuntimeConfig::default()
    };
    let cfg_handle = Arc::new(Mutex::new(cfg));

    // Start IPC server thread (uses cfg_handle internally)
    crate::service_impl_ipc::start_ipc_server(cfg_handle.clone());

    // Start auto-updater thread
    crate::service_impl_updater::start_auto_updater(cfg_handle.clone());

    // Start enforcement subsystems (WMI watcher, firewall sync, main enforcement)
    crate::service_impl_enforce::start_enforcement(cfg_handle.clone());

    // Main loop placeholder
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}
