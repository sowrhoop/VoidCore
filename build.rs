use std::env;

fn main() {
    // Re-run this build script if any of these change
    println!("cargo:rerun-if-env-changed=APP_WHITELIST");
    println!("cargo:rerun-if-env-changed=URL_BLOCKLIST");
    println!("cargo:rerun-if-env-changed=PUBLIC_KEY");
    println!("cargo:rerun-if-env-changed=GITHUB_REPO");
    println!("cargo:rerun-if-env-changed=VERSION_CODE");

    // The original project injected config via build-time env vars. For the
    // workspace refactor we keep the build script but it now emits nothing by
    // default; runtime configuration will be loaded from
    // C:\ProgramData\VoidCore\config.json by the service at runtime.
    println!("cargo:warning=Build-time injection disabled; runtime config.json will be used");
}
