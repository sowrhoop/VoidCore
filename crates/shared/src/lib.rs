use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Fetches the version number injected by GitHub Actions at compile time.
/// Defaults to 1 for local development if the environment variable is missing.
pub fn binary_version() -> u32 {
    option_env!("VOIDCORE_VERSION")
        .unwrap_or("1")
        .parse::<u32>()
        .unwrap_or(1)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RuntimeConfig {
    pub whitelist: HashSet<String>,
    pub url_blocklist: HashSet<String>,
    pub github_repo: String,
    pub pubkey_hex: String,
    
    // Security Fix: Do not save or load the version from config.json.
    // Always use the embedded version of the currently executing binary.
    #[serde(skip, default = "binary_version")]
    pub version_code: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            whitelist: ["code","docker","python","wt","msedge","cursor","brave","vscodium"].iter().map(|s| s.to_string()).collect(),
            url_blocklist: ["reddit.com","twitter.com","x.com","instagram.com","facebook.com","tiktok.com","youtube.com","twitch.tv","netflix.com","9gag.com","discord.com"].iter().map(|s| s.to_string()).collect(),
            github_repo: "sowrhoop/VoidCore".to_string(),
            pubkey_hex: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            version_code: binary_version(),
        }
    }
}