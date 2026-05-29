//! HTTP tracker client.
//!
//! Announces to the tracker and parses the compact peer list response.

use super::AnnounceResponse;
use crate::bencode::decoder;
use crate::peer::PeerAddr;
use std::net::{IpAddr, Ipv4Addr};
use thiserror::Error;
use url::Url;

#[derive(Debug, Error)]
pub enum TrackerError {
    #[error("Failed to build tracker URL: {0}")]
    UrlBuild(String),

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Failed to decode tracker response: {0}")]
    Decode(#[from] decoder::DecodeError),

    #[error("Tracker returned error: {0}")]
    TrackerFailure(String),

    #[error("Malformed peer data: length {0} is not a multiple of 6")]
    MalformedPeers(usize),

    #[error("Missing field in tracker response: {0}")]
    MissingField(&'static str),
}

/// Generate a random 20-byte peer ID with a client prefix.
pub fn generate_peer_id() -> [u8; 20] {
    let mut peer_id = [0u8; 20];
    // Client prefix: RT = RustyTorrent, version 0001
    peer_id[..8].copy_from_slice(b"-RT0001-");
    // Fill the rest with random bytes
    for byte in &mut peer_id[8..] {
        *byte = rand::random();
    }
    peer_id
}

/// Build the tracker announce URL with query parameters.
pub fn build_announce_url(
    announce: &str,
    info_hash: &[u8; 20],
    peer_id: &[u8; 20],
    port: u16,
    uploaded: u64,
    downloaded: u64,
    left: u64,
) -> Result<String, TrackerError> {
    let mut url = Url::parse(announce).map_err(|e| TrackerError::UrlBuild(e.to_string()))?;

    // We need to manually build the query because info_hash and peer_id
    // contain arbitrary bytes that need special URL encoding.
    let mut query = String::new();

    // URL-encode the info_hash (raw bytes)
    query.push_str("info_hash=");
    for &byte in info_hash {
        query.push_str(&format!("%{byte:02X}"));
    }

    // URL-encode the peer_id (raw bytes)
    query.push_str("&peer_id=");
    for &byte in peer_id {
        query.push_str(&format!("%{byte:02X}"));
    }

    query.push_str(&format!("&port={port}"));
    query.push_str(&format!("&uploaded={uploaded}"));
    query.push_str(&format!("&downloaded={downloaded}"));
    query.push_str("&compact=1");
    query.push_str(&format!("&left={left}"));

    url.set_query(Some(&query));

    Ok(url.to_string())
}

/// Send an announce request to the tracker and parse the response.
pub async fn announce(
    announce_url: &str,
    info_hash: &[u8; 20],
    peer_id: &[u8; 20],
    port: u16,
    uploaded: u64,
    downloaded: u64,
    left: u64,
) -> Result<AnnounceResponse, TrackerError> {
    let url = build_announce_url(
        announce_url,
        info_hash,
        peer_id,
        port,
        uploaded,
        downloaded,
        left,
    )?;

    tracing::info!("Announcing to tracker: {url}");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let response = client.get(&url).send().await?;
    let body = response.bytes().await?;

    let value = decoder::decode(&body)?;

    // Check for tracker error
    if let Some(failure) = value.get_str("failure reason") {
        let msg = failure.as_str().unwrap_or("unknown error").to_string();
        return Err(TrackerError::TrackerFailure(msg));
    }

    // Parse interval
    let interval = value
        .get_str("interval")
        .and_then(|v| v.as_integer())
        .unwrap_or(900) as u64;

    // Parse compact peer list (6 bytes per peer: 4 bytes IP + 2 bytes port)
    let peers_raw = value
        .get_str("peers")
        .and_then(|v| v.as_bytes())
        .ok_or(TrackerError::MissingField("peers"))?;

    let peers = unmarshal_peers(peers_raw)?;

    tracing::info!(
        "Tracker returned {count} peers (re-announce in {interval}s)",
        count = peers.len()
    );

    Ok(AnnounceResponse { interval, peers })
}

/// Parse the compact peer list: each peer is 6 bytes (4 IP + 2 port).
pub fn unmarshal_peers(data: &[u8]) -> Result<Vec<PeerAddr>, TrackerError> {
    const PEER_SIZE: usize = 6;

    if data.len() % PEER_SIZE != 0 {
        return Err(TrackerError::MalformedPeers(data.len()));
    }

    let peers = data
        .chunks_exact(PEER_SIZE)
        .map(|chunk| {
            let ip = IpAddr::V4(Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3]));
            let port = u16::from_be_bytes([chunk[4], chunk[5]]);
            PeerAddr { ip, port }
        })
        .collect();

    Ok(peers)
}
