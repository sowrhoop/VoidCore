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
    #[serde(default)]
    pub trusted_publishers: HashSet<String>,
    pub github_repo: String,
    pub pubkey_hex: String,
    
    #[serde(skip, default = "binary_version")]
    pub version_code: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        let whitelist_raw = option_env!("APP_WHITELIST").unwrap_or("code,docker,python,wt,msedge,cursor,brave,vscodium");
        let blocklist_raw = option_env!("URL_BLOCKLIST").unwrap_or("reddit.com,twitter.com,x.com,instagram.com,facebook.com,tiktok.com,youtube.com,twitch.tv,netflix.com,9gag.com,discord.com");
        let publishers_raw = option_env!("TRUSTED_PUBLISHERS").unwrap_or("Microsoft Corporation,Brave Software,Docker Inc,Python Software Foundation,OpenJS Foundation,Postman,WireGuard,Martin Prikryl,Igor Pavlov,Intel Corporation,Johannes Schindelin,GitHub");
        
        Self {
            whitelist: whitelist_raw.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect(),
            url_blocklist: blocklist_raw.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect(),
            trusted_publishers: publishers_raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            
            github_repo: "sowrhoop/VoidCore".to_string(),
            pubkey_hex: option_env!("VOIDCORE_PUBKEY").unwrap_or("0000000000000000000000000000000000000000000000000000000000000000").to_string(),
            version_code: binary_version(),
        }
    }
}