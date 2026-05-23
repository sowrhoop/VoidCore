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

fn stage(out_dir: &str) -> anyhow::Result<()> {
    // Staging directory
    let staging = Path::new(out_dir).join("voidcore_staging");
    let _ = fs::create_dir_all(&staging);

    // Copy built artifacts if present (best-effort)
    let svc_src = Path::new(r"target\x86_64-pc-windows-msvc\release\voidcore-service.exe");
    let gui_src = Path::new(r"target\x86_64-pc-windows-msvc\release\voidcore-gui.exe");
    let svc_dst = staging.join("voidcore-service.exe");
    let gui_dst = staging.join("voidcore-gui.exe");
    let _ = fs::copy(&svc_src, &svc_dst);
    let _ = fs::copy(&gui_src, &gui_dst);

    // Create default config.json and gui.token in the staging dir
    let cfg = voidcore_shared::RuntimeConfig::default();
    let cfg_json = serde_json::to_string_pretty(&cfg)?;
    fs::write(staging.join("config.json"), cfg_json)?;

    // Create gui.token with a random value
    let mut rng = rand::thread_rng();
    let mut buf = [0u8; 32];
    rng.fill_bytes(&mut buf);
    let token = hex::encode(buf);
    fs::write(staging.join("gui.token"), token)?;
    // Record installer account SID for IPC verification
    let whoami_out = Command::new("whoami").output()?;
    let installer = String::from_utf8_lossy(&whoami_out.stdout).trim().to_string();
    // Resolve SID via powershell and write to installer.sid
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
    // Ensure we are elevated (best-effort)
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

    // Ensure PATH contains C:\ProgramData\VoidCore (PowerShell)
    let ps_path = r#"
        $cur = [Environment]::GetEnvironmentVariable('Path','Machine')
        if (-not ($cur -like '*C:\ProgramData\VoidCore*')) {
            [Environment]::SetEnvironmentVariable('Path', $cur + ';C:\ProgramData\VoidCore', 'Machine')
        }
    "#;
    run_cmd(Command::new("powershell").args(["-NoProfile","-NonInteractive","-WindowStyle","Hidden","-Command", ps_path]))?;

    // Set strict ACLs on gui.token and config.json using icacls
    // Get installer account (DOMAIN\User)
    let whoami_out = Command::new("whoami").output()?;
    let installer = String::from_utf8_lossy(&whoami_out.stdout).trim().to_string();

    let gui_token_path = r"C:\ProgramData\VoidCore\gui.token";
    let cfg_path = r"C:\ProgramData\VoidCore\config.json";

    // Remove inheritance and grant SYSTEM & Administrators full, installer read
    run_cmd(Command::new("icacls").args([gui_token_path, "/inheritance:r"]))?;
    run_cmd(Command::new("icacls").args([gui_token_path, "/grant", "SYSTEM:F"]))?;
    run_cmd(Command::new("icacls").args([gui_token_path, "/grant", "Administrators:F"]))?;
    run_cmd(Command::new("icacls").args([gui_token_path, "/grant", &format!("{}:R", installer)]))?;

    // Config file: full for SYSTEM/Admins, read for installer
    run_cmd(Command::new("icacls").args([cfg_path, "/inheritance:r"]))?;
    run_cmd(Command::new("icacls").args([cfg_path, "/grant", "SYSTEM:F"]))?;
    run_cmd(Command::new("icacls").args([cfg_path, "/grant", "Administrators:F"]))?;
    run_cmd(Command::new("icacls").args([cfg_path, "/grant", &format!("{}:R", installer)]))?;

    // Register Windows Service (remove stale first)
    let _ = run_cmd(Command::new("sc").args(["delete", "VoidCoreDaemon"]));
    run_cmd(Command::new("sc").args([
        "create",
        "VoidCoreDaemon",
        "binPath=",
        "C:\\ProgramData\\VoidCore\\voidcore-service.exe",
        "start=",
        "auto",
        "obj=",
        "LocalSystem",
        "DisplayName=",
        "VoidCore Zero-Trust Daemon",
    ]))?;

    // Configure description and failure actions
    run_cmd(Command::new("sc").args(["description", "VoidCoreDaemon", "VoidCore Zero-Trust focus daemon — DO NOT STOP"]))?;
    run_cmd(Command::new("sc").args(["failure", "VoidCoreDaemon", "reset=", "0", "actions=", "restart/2000/restart/5000/restart/10000"]))?;

    // Start the service
    run_cmd(Command::new("sc").args(["start", "VoidCoreDaemon"]))?;

    // Create Uninstall registry entry via PowerShell
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

    // Create Start Menu shortcut via PowerShell
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

    // Demote current installer account (run the purge script used by the daemon)
    let demote_ps = r#"
        $admins = Get-LocalGroupMember -Group 'Administrators' |
                  Where-Object { $_.ObjectClass -eq 'User' }
        foreach ($a in $admins) {
            $name = $a.Name -replace '.*\\',''  # strip domain prefix
            if ($name -ne 'VoidCoreAdmin' -and $name -ne 'Administrator') {
                Add-LocalGroupMember    -Group 'Users'          -Member $a.Name -ErrorAction SilentlyContinue
                Remove-LocalGroupMember -Group 'Administrators' -Member $a.Name -ErrorAction SilentlyContinue
            }
        }
    "#;
    run_cmd(Command::new("powershell").args(["-NoProfile","-NonInteractive","-WindowStyle","Hidden","-Command", demote_ps]))?;

    // Finalise: reboot after short delay
    run_cmd(Command::new("shutdown").args(["/r","/t","10","/c","VoidCore: Finalising installation"]))?;

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
