// Repackaged service entrypoint. This file is an initial port of the original
// single-file project into the service crate. Behavior is intentionally kept
// functionally identical where possible; later changes will move configuration
// to runtime config.json and add IPC endpoints.

mod service_impl;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    service_impl::main()
}
