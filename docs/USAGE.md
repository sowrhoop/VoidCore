Quick usage notes
=================

Building locally (Windows):

  cargo build --workspace --release

The service and gui binaries will be available under:

  target\x86_64-pc-windows-msvc\release\voidcore-service.exe
  target\x86_64-pc-windows-msvc\release\voidcore-gui.exe

Installer
---------
The minimal installer builder is in tools/installer-builder. It is currently a
stub that stages files under the OUT_DIR. A future patch will produce a
self-extracting EXE.

CI Signing
----------
Add CERT_PFX and CERT_PASSWORD as GitHub Secrets (see docs/CI_SIGNING.md) to
enable CI artifact signing.
