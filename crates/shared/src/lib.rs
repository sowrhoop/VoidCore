use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RuntimeConfig {
    pub whitelist: HashSet<String>,
    pub url_blocklist: HashSet<String>,
    pub github_repo: String,
    pub pubkey_hex: String,
    pub version_code: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            whitelist: ["code","docker","python","wt","msedge","cursor","brave","vscodium"].iter().map(|s| s.to_string()).collect(),
            url_blocklist: ["reddit.com","twitter.com","x.com","instagram.com","facebook.com","tiktok.com","youtube.com","twitch.tv","netflix.com","9gag.com","discord.com"].iter().map(|s| s.to_string()).collect(),
            github_repo: "sowrhoop/VoidCore".to_string(),
            pubkey_hex: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            version_code: 1,
        }
    }
}
