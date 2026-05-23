# VoidCore 🛡️ — Windows Zero-Trust Focus Daemon

VoidCore is a bare-metal, zero-dependency Windows 11 self-discipline daemon. It operates at `NT AUTHORITY\SYSTEM` to enforce deep focus and digital minimalism. Once installed, the only way to change its configuration is to push a new signed build through your GitHub Actions pipeline.

---

## ⚠️ Critical Warning

**VoidCore is a self-imposed cryptographic lockdown.** It:

- Destroys Windows Safe Mode
- Strips your account of Administrator rights
- Generates a 127-character random password every 10 minutes and never stores it
- Blocks distracting websites at the OS, DNS, browser-policy, and firewall levels
- Kills any process not on your compiled whitelist

**If you corrupt the CI/CD pipeline or forget your GitHub credentials, you will need to format your C:\ drive.** Use at your own risk. Keep your BitLocker recovery key physically offsite.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│  voidcore.exe  (single binary, two modes)                       │
│                                                                 │
│  Interactive  ──►  GUI Installer / CLI (--setup / --status)     │
│  SCM Service  ──►  NT AUTHORITY\SYSTEM Daemon                   │
│                         │                                       │
│          ┌──────────────┼──────────────────┐                    │
│          ▼              ▼                  ▼                    │
│   WMI Watcher    Auto-Updater       Firewall Sync               │
│  (kills non-     (hourly GitHub     (resolves + blocks          │
│   whitelist)      release check)     blocklisted IPs)           │
│                                                                 │
│  Main Loop (every 10 min):                                      │
│    • Rotate VoidCoreAdmin password (127-char, RAM only)         │
│    • Disable built-in Administrator account                     │
│    • Demote any new Administrators back to Standard User        │
│    • Re-enforce Mullvad DNS on all interfaces                   │
│    • Re-write Chrome/Brave/Edge Group Policies                  │
│    • Re-write /etc/hosts blocklist                              │
└─────────────────────────────────────────────────────────────────┘
```

### The Void Architecture

| Mechanism | Implementation |
|---|---|
| **Zero-Knowledge LAPS** | 127-char password, rotated every 10 min, never written to disk |
| **Cryptographic Updates** | Ed25519 signature verified before any binary is applied |
| **Compile-time Config** | Whitelist and blocklist are baked into the binary — no config files to edit |
| **Anti-Rollback** | Registry monotonic counter prevents downgrading to bypass features |
| **Process Enforcement** | WMI `Win32_ProcessStartTrace` terminates non-whitelisted processes in milliseconds |

---

## Deployment

### Step 1 — Generate an Ed25519 key pair

```bash
# Using Python (cross-platform)
python3 -c "
from cryptography.hazmat.primitives.asymmetric import ed25519
import os

priv = ed25519.Ed25519PrivateKey.generate()
pub  = priv.public_key()

priv_bytes = priv.private_bytes_raw()
pub_bytes  = pub.public_bytes_raw()

print('SIGNING_KEY (secret):', priv_bytes.hex())
print('PUBLIC_KEY  (public):', pub_bytes.hex())
"
```

### Step 2 — Configure GitHub Secrets

Go to **Repository → Settings → Secrets and variables → Actions** and add:

| Secret | Value |
|---|---|
| `APP_WHITELIST` | Comma-separated exe names without `.exe`, e.g. `code,docker,python,wt,msedge,cursor,brave` |
| `URL_BLOCKLIST` | Comma-separated domains, e.g. `reddit.com,twitter.com,youtube.com` |
| `PUBLIC_KEY` | Hex-encoded Ed25519 public key (64 hex chars) |
| `SIGNING_KEY` | Hex-encoded Ed25519 private key (64 hex chars) — **keep this secret** |

### Step 3 — Build

Trigger the **VoidCore Cryptographic Release** workflow. Download `voidcore.exe` from the release artifacts.

### Step 4 — The Point of No Return

> **Before proceeding:** Enable BitLocker on your C:\ drive and store the recovery key physically away from your machine.

Open an **Administrator Command Prompt** and run:

```cmd
voidcore.exe --setup
```

A GUI wizard will confirm your choices. After you click Yes:

1. The binary is copied to `C:\ProgramData\VoidCore\voidcore.exe`
2. The Windows Service is installed and started
3. Windows Safe Mode is disabled
4. Your account is demoted to Standard User
5. The machine restarts (10-second countdown)

After the restart, **log in normally**. Explorer will load as usual. The daemon runs silently in the background.

---

## Adding or Removing Apps

You cannot do this locally. Update the `APP_WHITELIST` secret on GitHub, push a commit (or trigger the workflow manually), and wait. The daemon checks for a new release every hour and applies it automatically.

To force an immediate update:

```cmd
voidcore --update
```

(This works even from a Standard User account — it writes a flag file that the SYSTEM daemon detects within 15 seconds.)

---

## CLI Reference

```
voidcore --setup      GUI installer wizard (requires Administrator)
voidcore --status     Print current enforcement status
voidcore --version    Print version and compile-time configuration
voidcore --update     Signal the daemon to pull the latest release
```

---

## Status Check

```
╔══════════════════════════════════════════╗
║         VOIDCORE SYSTEMS AUDIT           ║
╠══════════════════════════════════════════╣
║ Version     : v1.0.100042               ║
║ Whitelist   : code,docker,python,wt,m…  ║
║ Blocklist   : reddit.com,twitter.com,…  ║
╠══════════════════════════════════════════╣
║ [+] Session     : STANDARD USER (Secured)  ║
║ [+] Daemon      : RUNNING (Unstoppable)    ║
║ [+] Safe Mode   : DESTROYED (Secured)      ║
╚══════════════════════════════════════════╝
```

---

## License

GNU Affero General Public License v3.0 — see [LICENSE](LICENSE).
