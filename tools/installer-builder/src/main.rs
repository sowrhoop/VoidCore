use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;
use rand::RngCore;
use hex;

fn run_cmd(cmd: &mut Command) -> anyhow::Result<()> {
    let output = cmd.output()?;
    if !output.status.success() {
        eprintln!("Command failed: {:?}\nstdout:{}\nstderr:{}", cmd, String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }
    Ok(())
}

// Helper to run commands that might legitimately fail (e.g., deleting a non-existent Safe Mode entry)
fn run_cmd_silent(cmd: &mut Command) {
    let _ = cmd.output();
}

fn stage(out_dir: &str) -> anyhow::Result<()> {
    // Staging directory
    let staging = Path::new(out_dir).join("voidcore_staging");
    let _ = fs::create_dir_all(&staging);

    // Dynamically locate the binaries whether built with an explicit target or standard release
    let svc_src_default = Path::new("target").join("release").join("voidcore-service.exe");
    let gui_src_default = Path::new("target").join("release").join("voidcore-gui.exe");
    
    let svc_src_msvc = Path::new("target").join("x86_64-pc-windows-msvc").join("release").join("voidcore-service.exe");
    let gui_src_msvc = Path::new("target").join("x86_64-pc-windows-msvc").join("release").join("voidcore-gui.exe");

    let svc_src = if svc_src_default.exists() { svc_src_default } else { svc_src_msvc };
    let gui_src = if gui_src_default.exists() { gui_src_default } else { gui_src_msvc };

    let svc_dst = staging.join("voidcore-service.exe");
    let gui_dst = staging.join("voidcore-gui.exe");
    let _ = fs::copy(&svc_src, &svc_dst);
    let _ = fs::copy(&gui_src, &gui_dst);

    // Create default config.json
    let mut cfg = voidcore_shared::RuntimeConfig::default();
    
    // Inject the real public key if provided via the environment (Crucial for CI/CD)
    if let Ok(pk) = env::var("PUBLIC_KEY") {
        cfg.pubkey_hex = pk;
    }

    let cfg_json = serde_json::to_string_pretty(&cfg)?;
    fs::write(staging.join("config.json"), cfg_json)?;

    // Create gui.token with a cryptographically random value
    let mut rng = rand::thread_rng();
    let mut buf = [0u8; 32];
    rng.fill_bytes(&mut buf);
    let token = hex::encode(buf);
    fs::write(staging.join("gui.token"), token)?;
    
    // Record installer account SID for IPC verification
    let whoami_out = Command::new("whoami").output()?;
    let installer = String::from_utf8_lossy(&whoami_out.stdout).trim().to_string();
    
    let sid_ps = format!(r#"(New-Object System.Security.Principal.NTAccount('{inst}')).Translate([System.Security.Principal.SecurityIdentifier]).Value"#, inst=installer);
    let sid_out = Command::new("powershell").args(["-NoProfile","-NonInteractive","-Command", &sid_ps]).output()?;
    let sid = String::from_utf8_lossy(&sid_out.stdout).trim().to_string();
    if !sid.is_empty() {
        fs::write(staging.join("installer.sid"), sid)?;
    }

    println!("Staging directory prepared: {}", staging.display());
    Ok(())
}

fn install_from_stage(out_dir: &str) -> anyhow::Result<()> {
    // Ensure we are elevated
    let elevated = Command::new("net").args(["session"]).output().map(|o| o.status.success()).unwrap_or(false);
    if !elevated {
        anyhow::bail!("Installer must be run elevated (Run as Administrator)");
    }

    let staging = Path::new(out_dir).join("voidcore_staging");
    if !staging.exists() {
        anyhow::bail!("Staging directory not found: {}", staging.display());
    }

    let target_dir = Path::new(r"C:\ProgramData\VoidCore");
    if !target_dir.exists() {
        fs::create_dir_all(target_dir)?;
    }

    // Copy files from staging to target
    for entry in fs::read_dir(&staging)? {
        let entry = entry?;
        let file_name = entry.file_name();
        let src = entry.path();
        let dst = target_dir.join(&file_name);
        let _ = fs::copy(&src, &dst);
    }

    // Ensure PATH contains C:\ProgramData\VoidCore
    let ps_path = r#"
        $cur = [Environment]::GetEnvironmentVariable('Path','Machine')
        if (-not ($cur -like '*C:\ProgramData\VoidCore*')) {
            [Environment]::SetEnvironmentVariable('Path', $cur + ';C:\ProgramData\VoidCore', 'Machine')
        }
    "#;
    run_cmd(Command::new("powershell").args(["-NoProfile","-NonInteractive","-WindowStyle","Hidden","-Command", ps_path]))?;

    // Set strict ACLs
    let whoami_out = Command::new("whoami").output()?;
    let installer = String::from_utf8_lossy(&whoami_out.stdout).trim().to_string();

    let gui_token_path = r"C:\ProgramData\VoidCore\gui.token";
    let cfg_path = r"C:\ProgramData\VoidCore\config.json";

    run_cmd(Command::new("icacls").args([gui_token_path, "/inheritance:r"]))?;
    run_cmd(Command::new("icacls").args([gui_token_path, "/grant", "SYSTEM:F"]))?;
    run_cmd(Command::new("icacls").args([gui_token_path, "/grant", "Administrators:F"]))?;
    run_cmd(Command::new("icacls").args([gui_token_path, "/grant", &format!("{}:R", installer)]))?;

    run_cmd(Command::new("icacls").args([cfg_path, "/inheritance:r"]))?;
    run_cmd(Command::new("icacls").args([cfg_path, "/grant", "SYSTEM:F"]))?;
    run_cmd(Command::new("icacls").args([cfg_path, "/grant", "Administrators:F"]))?;
    run_cmd(Command::new("icacls").args([cfg_path, "/grant", &format!("{}:R", installer)]))?;

    // Create VoidCoreAdmin Account (CRITICAL: Prevents total system lockout)
    run_cmd_silent(Command::new("net").args(["user", "VoidCoreAdmin", "V0idC0reTempP@ss!", "/add"]));
    run_cmd_silent(Command::new("net").args(["localgroup", "Administrators", "VoidCoreAdmin", "/add"]));

    // Disable Windows Safe Mode (Security enforcement)
    run_cmd_silent(Command::new("bcdedit").args(["/set", "{default}", "bootmenupolicy", "standard"]));
    run_cmd_silent(Command::new("bcdedit").args(["/deletevalue", "{default}", "safeboot"]));

    // Register Windows Service
    run_cmd_silent(Command::new("sc").args(["stop", "VoidCoreDaemon"]));
    std::thread::sleep(std::time::Duration::from_secs(2));
    run_cmd_silent(Command::new("sc").args(["delete", "VoidCoreDaemon"]));
    
    run_cmd(Command::new("sc").args([
        "create", "VoidCoreDaemon", "binPath=", "C:\\ProgramData\\VoidCore\\voidcore-service.exe",
        "start=", "auto", "obj=", "LocalSystem", "DisplayName=", "VoidCore Zero-Trust Daemon",
    ]))?;

    run_cmd(Command::new("sc").args(["description", "VoidCoreDaemon", "VoidCore Zero-Trust focus daemon — DO NOT STOP"]))?;
    run_cmd(Command::new("sc").args(["failure", "VoidCoreDaemon", "reset=", "0", "actions=", "restart/2000/restart/5000/restart/10000"]))?;
    run_cmd(Command::new("sc").args(["start", "VoidCoreDaemon"]))?;

    // Create Uninstall registry entry
    let uninstall_ps = r#"
        $key = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\VoidCore'
        New-Item -Path $key -Force | Out-Null
        New-ItemProperty -Path $key -Name DisplayName -Value 'VoidCore' -PropertyType String -Force | Out-Null
        New-ItemProperty -Path $key -Name DisplayVersion -Value '1.0' -PropertyType String -Force | Out-Null
        New-ItemProperty -Path $key -Name Publisher -Value 'VoidCore' -PropertyType String -Force | Out-Null
        New-ItemProperty -Path $key -Name UninstallString -Value '"C:\\ProgramData\\VoidCore\\voidcore-gui.exe" --uninstall' -PropertyType String -Force | Out-Null
        New-ItemProperty -Path $key -Name InstallLocation -Value 'C:\\ProgramData\\VoidCore' -PropertyType String -Force | Out-Null
    "#;
    run_cmd(Command::new("powershell").args(["-NoProfile","-NonInteractive","-WindowStyle","Hidden","-Command", uninstall_ps]))?;

    // Create Start Menu shortcut
    let start_ps = r#"
        $programs = "$env:ProgramData\Microsoft\Windows\Start Menu\Programs"
        $dir = Join-Path $programs 'VoidCore'
        if (!(Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
        $ws = New-Object -ComObject WScript.Shell
        $lnk = $ws.CreateShortcut((Join-Path $dir 'VoidCore.lnk'))
        $lnk.TargetPath = 'C:\\ProgramData\\VoidCore\\voidcore-gui.exe'
        $lnk.WorkingDirectory = 'C:\\ProgramData\\VoidCore'
        $lnk.IconLocation = 'C:\\ProgramData\\VoidCore\\voidcore-gui.exe,0'
        $lnk.Save()
    "#;
    run_cmd(Command::new("powershell").args(["-NoProfile","-NonInteractive","-WindowStyle","Hidden","-Command", start_ps]))?;

    // Demote current installer account
    let demote_ps = r#"
        $admins = Get-LocalGroupMember -Group 'Administrators' | Where-Object { $_.ObjectClass -eq 'User' }
        foreach ($a in $admins) {
            $name = $a.Name -replace '.*\\','' 
            if ($name -ne 'VoidCoreAdmin' -and $name -ne 'Administrator') {
                Add-LocalGroupMember -Group 'Users' -Member $a.Name -ErrorAction SilentlyContinue
                Remove-LocalGroupMember -Group 'Administrators' -Member $a.Name -ErrorAction SilentlyContinue
            }
        }
    "#;
    run_cmd(Command::new("powershell").args(["-NoProfile","-NonInteractive","-WindowStyle","Hidden","-Command", demote_ps]))?;

    // Finalise: reboot after short delay
    run_cmd(Command::new("shutdown").args(["/r","/t","10","/c","VoidCore: Finalising installation... Rebooting."]))?;

    println!("Installation complete; system will reboot shortly.");
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let out_dir = env::var("OUT_DIR").unwrap_or_else(|_| "dist".to_string());
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 && args[1] == "--install" {
        install_from_stage(&out_dir)?;
    } else {
        stage(&out_dir)?;
        println!("Run this tool with --install (elevated) to perform the actual installation.");
    }
    Ok(())
}