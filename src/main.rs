//! # RustyTorrent
//!
//! A blazing-fast BitTorrent client written in safe Rust.
//! Features: rich TUI dashboard, download resume, magnet links, peer scoring.

mod bencode;
mod config;
mod download;
mod peer;
mod torrent;
mod tracker;
mod tui;

use clap::{Parser, Subcommand};
use config::RustyConfig;
use std::path::PathBuf;
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(
    name = "rusty",
    version = "0.1.0",
    about = "🦀 RustyTorrent — A blazing-fast BitTorrent client written in safe Rust",
    long_about = "RustyTorrent is a modern BitTorrent client with a rich TUI dashboard,\n\
                  download resume support, magnet link parsing, and peer scoring.\n\n\
                  Built with safe Rust, async I/O, and zero unsafe code."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Path to configuration file
    #[arg(short, long, default_value = "rusty.toml")]
    config: PathBuf,

    /// Disable TUI dashboard (log to stderr instead)
    #[arg(long, default_value_t = false)]
    no_tui: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Download a torrent from a .torrent file
    Download {
        /// Path to the .torrent file
        #[arg(value_name = "TORRENT_FILE")]
        torrent: PathBuf,

        /// Output file or directory path
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Parse and display info about a .torrent file
    Info {
        /// Path to the .torrent file
        #[arg(value_name = "TORRENT_FILE")]
        torrent: PathBuf,
    },

    /// Parse a magnet link and display its info
    Magnet {
        /// The magnet URI
        #[arg(value_name = "MAGNET_URI")]
        uri: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Load config — try rusty.toml then rusty.toml for backwards-compat
    let cfg = if cli.config == PathBuf::from("rusty.toml") && !cli.config.exists() {
        RustyConfig::load(&PathBuf::from("rusty.toml"))
    } else {
        RustyConfig::load(&cli.config)
    };

    match cli.command {
        Commands::Download { torrent, output } => {
            cmd_download(torrent, output, cfg, cli.no_tui).await?;
        }
        Commands::Info { torrent } => {
            cmd_info(torrent)?;
        }
        Commands::Magnet { uri } => {
            cmd_magnet(&uri)?;
        }
    }

    Ok(())
}

/// Display information about a .torrent file.
fn cmd_info(path: PathBuf) -> anyhow::Result<()> {
    let meta = torrent::metainfo::Metainfo::from_file(&path)?;

    println!("╔══════════════════════════════════════════════════════╗");
    println!("║           🦀 RustyTorrent — File Info               ║");
    println!("╠══════════════════════════════════════════════════════╣");
    println!("║ Name:        {:<40} ║", meta.name);
    println!("║ Size:        {:<40} ║", meta.human_size());
    println!("║ Pieces:      {:<40} ║", meta.piece_hashes.len());
    println!(
        "║ Piece Size:  {:<40} ║",
        format!("{} KiB", meta.piece_length / 1024)
    );
    println!("║ Announce:    {:<40} ║", truncate(&meta.announce, 40));
    println!(
        "║ Info Hash:   {:<40} ║",
        meta.info_hash.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    println!(
        "║ Files:       {:<40} ║",
        if meta.is_single_file {
            "1 (single-file)".to_string()
        } else {
            format!("{} files", meta.files.len())
        }
    );
    println!("╚══════════════════════════════════════════════════════╝");

    if !meta.is_single_file {
        println!("\n  Files:");
        for (i, file) in meta.files.iter().enumerate() {
            let path_str = file.path.join("/");
            let size = format_bytes(file.length);
            println!("    {i:>3}. {path_str} ({size})");
        }
    }

    if !meta.announce_list.is_empty() {
        println!("\n  Tracker tiers:");
        for (i, tier) in meta.announce_list.iter().enumerate() {
            for url in tier {
                println!("    Tier {i}: {url}");
            }
        }
    }

    Ok(())
}

/// Parse and display a magnet link.
fn cmd_magnet(uri: &str) -> anyhow::Result<()> {
    let magnet = torrent::magnet::MagnetLink::parse(uri)?;

    println!("╔══════════════════════════════════════════════════════╗");
    println!("║           🧲 RustyTorrent — Magnet Link             ║");
    println!("╠══════════════════════════════════════════════════════╣");
    println!(
        "║ Name:      {:<42} ║",
        magnet.display_name.as_deref().unwrap_or("(unknown)")
    );
    println!("║ Info Hash: {:<42} ║", magnet.info_hash_hex());
    println!("║ Trackers:  {:<42} ║", magnet.trackers.len());
    println!("╚══════════════════════════════════════════════════════╝");

    for (i, tracker) in magnet.trackers.iter().enumerate() {
        println!("  Tracker {i}: {tracker}");
    }

    Ok(())
}

/// Download a torrent file.
async fn cmd_download(
    torrent_path: PathBuf,
    output: Option<PathBuf>,
    cfg: RustyConfig,
    no_tui: bool,
) -> anyhow::Result<()> {
    // Set up logging
    if no_tui {
        tracing_subscriber::fmt()
            .with_env_filter("rusty_torrent=debug")
            .with_target(false)
            .init();
    } else {
        // Quiet logging when TUI is active (it handles display)
        tracing_subscriber::fmt()
            .with_env_filter("rusty_torrent=warn")
            .with_target(false)
            .with_writer(std::io::stderr)
            .init();
    }

    // Parse the torrent file
    let meta = torrent::metainfo::Metainfo::from_file(&torrent_path)?;

    println!("🦀 RustyTorrent — Downloading: {}", meta.name);
    println!("   Size: {} | Pieces: {}", meta.human_size(), meta.piece_hashes.len());

    // Determine output path
    let output_path = output.unwrap_or_else(|| {
        let mut p = cfg.download.download_dir.clone();
        p.push(&meta.name);
        p
    });

    // Create output directory
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Generate our peer ID
    let peer_id = tracker::http::generate_peer_id();

    // Build the full list of tracker URLs (all tiers + primary announce)
    let mut announce_urls: Vec<String> = Vec::new();
    for tier in &meta.announce_list {
        for url in tier {
            if !announce_urls.contains(url) {
                announce_urls.push(url.clone());
            }
        }
    }
    if !announce_urls.contains(&meta.announce) {
        announce_urls.push(meta.announce.clone());
    }

    // Contact ALL trackers concurrently and merge unique peers
    // This is the key fix: don't stop on the first success — gather
    // as many peers as possible to maximise download concurrency.
    let mut all_peers: Vec<peer::PeerAddr> = Vec::new();
    let mut announce_interval = 1800u64;

    let mut tracker_futures = Vec::new();
    for tracker_url in &announce_urls {
        let url = tracker_url.clone();
        let ih = meta.info_hash;
        let pid = peer_id;
        let port = cfg.network.listen_port;
        let left = meta.total_length;

        if url.starts_with("udp://") {
            tracker_futures.push(tokio::spawn(async move {
                println!("   Contacting tracker: {}", url);
                tracker::udp::announce(&url, &ih, &pid, port, 0, 0, left)
                    .await
                    .map_err(|e| e.to_string())
            }));
        } else if url.starts_with("http://") || url.starts_with("https://") {
            tracker_futures.push(tokio::spawn(async move {
                println!("   Contacting tracker: {}", url);
                tracker::http::announce(&url, &ih, &pid, port, 0, 0, left)
                    .await
                    .map_err(|e| e.to_string())
            }));
        }
    }

    for handle in tracker_futures {
        match handle.await {
            Ok(Ok(resp)) => {
                if resp.interval < announce_interval {
                    announce_interval = resp.interval;
                }
                for p in resp.peers {
                    if !all_peers.contains(&p) {
                        all_peers.push(p);
                    }
                }
            }
            Ok(Err(e)) => {
                tracing::warn!("Tracker failed: {}", e);
            }
            Err(e) => {
                tracing::warn!("Tracker task panicked: {}", e);
            }
        }
    }

    if all_peers.is_empty() {
        return Err(anyhow::anyhow!("No peers found from any tracker"));
    }

    println!(
        "   Found {} peers (re-announce in {}s)",
        all_peers.len(),
        announce_interval
    );

    // Stats channel for the TUI
    let (stats_tx, stats_rx) = mpsc::channel(64);

    // Create the download engine
    let engine = download::engine::DownloadEngine::new(
        meta.clone(),
        cfg.clone(),
        all_peers,
        stats_tx,
    );

    if no_tui {
        // Simple mode: just download and log
        let data = engine.download(&output_path).await?;
        std::fs::write(&output_path, &data)?;
        println!("Downloaded to {}", output_path.display());
    } else {
        // TUI mode: run dashboard alongside download
        let output_clone = output_path.clone();

        let download_handle = tokio::spawn(async move {
            engine.download(&output_clone).await
        });

        let tui_handle = tokio::spawn(async move {
            tui::dashboard::run_dashboard(stats_rx, cfg.tui.refresh_rate_ms).await
        });

        // Wait for download (TUI will exit when download finishes)
        let data = download_handle.await??;

        // Save the file
        std::fs::write(&output_path, &data)?;

        // Wait for TUI to clean up
        let _ = tui_handle.await;

        println!("Downloaded to {}", output_path.display());
    }

    Ok(())
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

fn format_bytes(bytes: u64) -> String {
    let b = bytes as f64;
    if b < 1024.0 {
        format!("{b:.0} B")
    } else if b < 1024.0 * 1024.0 {
        format!("{:.1} KiB", b / 1024.0)
    } else if b < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MiB", b / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GiB", b / (1024.0 * 1024.0 * 1024.0))
    }
}
