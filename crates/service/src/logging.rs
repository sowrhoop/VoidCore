use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Simple structured logger for the service. Writes to
/// C:\ProgramData\VoidCore\logs\<component>.log
pub fn log_event(component: &str, level: &str, message: &str) -> std::io::Result<()> {
    let base = Path::new(r"C:\ProgramData\VoidCore");
    let logs = base.join("logs");
    let _ = fs::create_dir_all(&logs);
    let file = logs.join(format!("{}.log", component));
    let mut f = OpenOptions::new().create(true).append(true).open(file)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    writeln!(f, "{} [{}] {}", now, level, message)?;
    Ok(())
}
