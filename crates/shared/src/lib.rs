use serde::{Deserialize, Serialize};
use std::collections::HashSet;

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
    
    // NEW: Cryptographically trusted software publishers
    #[serde(default)]
    pub trusted_publishers: HashSet<String>,
    
    pub github_repo: String,
    pub pubkey_hex: String,
    
    #[serde(skip, default = "binary_version")]
    pub version_code: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            whitelist: ["code","docker","python","wt","msedge","cursor","brave","vscodium"].iter().map(|s| s.to_string()).collect(),
            url_blocklist: ["reddit.com","twitter.com","x.com","instagram.com","facebook.com","tiktok.com","youtube.com","twitch.tv","netflix.com","9gag.com","discord.com"].iter().map(|s| s.to_string()).collect(),
            
            // Map the exact strings found in the Authenticode Subject certificates
            trusted_publishers: [
                "Brave Software, Inc.",
                "Microsoft Corporation",
                "GitHub, Inc.",
                "Discord Inc."
            ].iter().map(|s| s.to_string()).collect(),
            
            github_repo: "sowrhoop/VoidCore".to_string(),
            pubkey_hex: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            version_code: binary_version(),
        }
    }
}