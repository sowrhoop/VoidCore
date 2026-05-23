Installer ACL guidance
======================

When the installer writes runtime files (config.json, gui.token, backups), it
must set Windows ACLs to prevent unauthorized modification. Recommended ACLs:

- C:\ProgramData\VoidCore\config.json
  - FULL control: SYSTEM, Administrators
  - READ/EXECUTE: Installer account (and optionally the service account)
  - No access: Authenticated Users / Everyone

- C:\ProgramData\VoidCore\gui.token
  - FULL control: SYSTEM, Administrators
  - READ only: Installer account (the account that performed the installation)
  - No access: Authenticated Users / Everyone

- C:\ProgramData\VoidCore\backups\*
  - FULL control: SYSTEM, Administrators
  - READ only: Installer account (optionally)

Setting ACLs from Rust
----------------------
Use the windows crate to call SetNamedSecurityInfoW or use PowerShell's
Set-Acl cmdlet run elevated. Example (PowerShell snippet the installer can run):

  $acl = Get-Acl -Path 'C:\ProgramData\VoidCore\config.json'
  $rule = New-Object System.Security.AccessControl.FileSystemAccessRule("BUILTIN\\Administrators","FullControl","Allow")
  $acl.SetAccessRule($rule)
  Set-Acl -Path 'C:\ProgramData\VoidCore\config.json' -AclObject $acl

Make sure to test ACL behaviour in a VM to ensure the demoted installing user
cannot overwrite tokens or config files.
