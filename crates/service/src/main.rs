// Rewritten service entrypoint managing SCM state and threads
mod logging;
mod service_impl_enforce;
mod service_impl_ipc;
mod service_impl_updater;

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
    // Attempt to start via Windows Service Control Manager
    if let Err(_) = service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
        // Fallback: Run directly (useful for local debugging)
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

    // Block here holding the main service lifecycle
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

    // Keep service alive
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}