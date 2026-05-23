CI Signing with GitHub Actions
================================

This repository includes a GitHub Actions workflow (.github/workflows/windows-ci.yml)
that builds x86_64 artifacts on windows-latest and can sign them using a PFX
certificate provided via repository secrets.

Secrets required
- CERT_PFX: base64-encoded .pfx contents (do NOT commit the raw PFX to the repo).
- CERT_PASSWORD: password for the PFX.

How to produce CERT_PFX value locally (PowerShell):

  $bytes = [System.IO.File]::ReadAllBytes('path\\to\\cert.pfx')
  $b64 = [System.Convert]::ToBase64String($bytes)
  Write-Output $b64

Add the two values as repository secrets (Settings → Secrets → Actions). The
workflow triggers on pushing tags like v1.0.1 and will upload artifacts; it will
only sign artifacts if both secrets are present.
