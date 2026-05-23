VoidCore GUI/Installer Implementation Plan

Overview
--------
This document summarises the refactor and GUI implementation plan. The
repository has been reorganised into a Rust workspace with crates:

- crates/service     — system daemon (runs as SYSTEM)
- crates/gui         — native Win32 GUI client (non-elevated for status + update)
- crates/shared      — shared config and IPC types
- tools/installer-builder — Rust-only helper to produce self-extracting installers

High-level steps already performed
- Created Rust workspace and initial crates (service, gui, shared, tools/installer-builder)
- Added placeholder implementations for service, GUI, and installer-builder so the
  repository builds and can be extended incrementally.

Next implementation phases
1) Move runtime config into C:\ProgramData\VoidCore\config.json and implement
   secure ACLs for that file.
2) Implement secure named-pipe IPC between GUI and service with a gui.token file
   containing a shared secret; non-elevated GUI will be allowed to view status
   and request update only.
3) Port enforcement logic and updater into the service crate while preserving
   anti-rollback logic. Calls that perform destructive system changes will
   snapshot state into C:\ProgramData\VoidCore\backups\<timestamp> for rollback.
4) Implement GUI windows/tabs for Installer, Status, Config (view-only unless
   elevated), Update (trigger/report), Backups (view snapshots; rollback requires
   elevation), Logs (tail & search), and Uninstall (elevated).
5) Implement tools/installer-builder to create a self-extracting installer EXE
   that writes files, sets ACLs, creates the Windows service, and registers
   Add/Remove Programs entries. Document how to run the tool.
6) Add GitHub Actions workflow to build x86_64 artifacts on windows-latest,
   sign using CERT_PFX (base64) and CERT_PASSWORD from repo secrets, and upload
   signed voidcore.exe + voidcore.exe.sig to releases on tag push.

Security & Safety Notes
- This software makes system-wide changes; test inside a VM snapshot before
  deploying into production.
- Rollbacks and boot-level changes are inherently risky — GUI will require
  two-step confirmation for such operations.

How patches will be delivered
- I will modify files locally and produce patches (git-style) for you to review
  and commit. I will not push branches or commit to your repository.
