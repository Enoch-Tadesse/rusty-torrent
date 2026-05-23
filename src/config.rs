use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Network-related configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct NetworkConfig {
    /// Port to listen on for incoming peer connections.
    pub listen_port: u16,
    /// Maximum number of simultaneous peer connections.
    pub max_peers: usize,
    /// Timeout in seconds when connecting to a peer.
    pub connection_timeout_secs: u64,
}

/// Download behavior configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct DownloadConfig {
    /// Max pieces being downloaded at the same time across all peers.
    #[allow(dead_code)]
    pub max_concurrent_pieces: usize,
    /// Block size in bytes (default: 16 KiB).
    pub block_size: usize,
    /// Directory to save downloaded files.
    pub download_dir: PathBuf,
    /// Maximum number of outstanding requests per peer.
    pub max_backlog: usize,
}

/// TUI dashboard configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct TuiConfig {
    /// Refresh interval for the dashboard in milliseconds.
    pub refresh_rate_ms: u64,
}

/// Top-level configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct RustyConfig {
    pub network: NetworkConfig,
    pub download: DownloadConfig,
    pub tui: TuiConfig,
}

impl Default for RustyConfig {
    fn default() -> Self {
        Self {
            network: NetworkConfig {
                listen_port: 6881,
                max_peers: 50,
                connection_timeout_secs: 5,
            },
            download: DownloadConfig {
                max_concurrent_pieces: 10,
                block_size: 16384,
                download_dir: PathBuf::from("./downloads"),
                max_backlog: 5,
            },
            tui: TuiConfig {
                refresh_rate_ms: 250,
            },
        }
    }
}

impl RustyConfig {
    /// Load configuration from a TOML file, falling back to defaults.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => toml::from_str(&contents).unwrap_or_else(|e| {
                tracing::warn!("Failed to parse config file: {e}. Using defaults.");
                Self::default()
            }),
            Err(_) => {
                tracing::info!("No config file found at {path:?}. Using defaults.");
                Self::default()
            }
        }
    }
}
