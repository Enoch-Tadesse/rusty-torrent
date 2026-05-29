//! UDP tracker client (BEP 15).
//!
//! UDP tracker protocol with connection and announce phases.

use super::AnnounceResponse;
use crate::peer::PeerAddr;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::io::Cursor;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;
use thiserror::Error;
use tokio::net::UdpSocket;

#[derive(Debug, Error)]
pub enum UdpTrackerError {
    #[error("Invalid tracker URL: {0}")]
    InvalidUrl(String),

    #[error("Socket error: {0}")]
    Socket(#[from] std::io::Error),

    #[error("Tracker error: {0}")]
    TrackerError(String),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("Timeout")]
    Timeout,

    #[error("Connection failed")]
    ConnectionFailed,
}

/// Parse UDP tracker URL (udp://host:port or udp://host:port/path)
fn parse_udp_url(url: &str) -> Result<(String, u16), UdpTrackerError> {
    if !url.starts_with("udp://") {
        return Err(UdpTrackerError::InvalidUrl(
            "URL must start with udp://".to_string(),
        ));
    }

    let rest = &url[6..];
    // Extract host:port part (everything before first / if present)
    let host_port = rest.split('/').next().unwrap_or(rest);

    // Split on last colon to separate host and port
    if let Some(colon_pos) = host_port.rfind(':') {
        let host = &host_port[..colon_pos];
        let port_str = &host_port[colon_pos + 1..];
        let port = port_str
            .parse::<u16>()
            .map_err(|_| UdpTrackerError::InvalidUrl("Invalid port".to_string()))?;
        return Ok((host.to_string(), port));
    }

    Err(UdpTrackerError::InvalidUrl(
        "Missing port number".to_string(),
    ))
}

/// Send a connection request and receive connection ID, then immediately
/// send the announce on the **same socket** (required by BEP 15 — the
/// tracker validates that the connection ID is used from the same source
/// IP/port it was issued to).
async fn connect_and_announce(
    addr: SocketAddr,
    info_hash: &[u8; 20],
    peer_id: &[u8; 20],
    port: u16,
    uploaded: u64,
    downloaded: u64,
    left: u64,
) -> Result<AnnounceResponse, UdpTrackerError> {
    // Bind once — reuse for both connect and announce phases.
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.connect(addr).await?;

    // Connect
    let connect_transaction_id: u32 = rand::random();
    let mut connect_req: Vec<u8> = Vec::with_capacity(16);
    connect_req.write_u64::<BigEndian>(0x41727101980)?; // magic constant
    connect_req.write_u32::<BigEndian>(0)?;              // action = connect
    connect_req.write_u32::<BigEndian>(connect_transaction_id)?;

    tokio::time::timeout(Duration::from_secs(15), socket.send(&connect_req))
        .await
        .map_err(|_| UdpTrackerError::Timeout)??;

    let mut connect_resp = [0u8; 16];
    tokio::time::timeout(Duration::from_secs(15), socket.recv(&mut connect_resp))
        .await
        .map_err(|_| UdpTrackerError::Timeout)??;

    let mut cursor = Cursor::new(&connect_resp);
    let action = cursor.read_u32::<BigEndian>()?;
    let resp_txid = cursor.read_u32::<BigEndian>()?;
    let connection_id = cursor.read_u64::<BigEndian>()?;

    if action != 0 {
        return Err(UdpTrackerError::ConnectionFailed);
    }
    if resp_txid != connect_transaction_id {
        return Err(UdpTrackerError::InvalidResponse(
            "Connect transaction ID mismatch".to_string(),
        ));
    }

    // Announce
    let announce_transaction_id: u32 = rand::random();
    let mut ann_req: Vec<u8> = Vec::with_capacity(98);
    ann_req.write_u64::<BigEndian>(connection_id)?;         // connection_id
    ann_req.write_u32::<BigEndian>(1)?;                     // action = announce
    ann_req.write_u32::<BigEndian>(announce_transaction_id)?;
    ann_req.extend_from_slice(info_hash);                   // info_hash (20 bytes)
    ann_req.extend_from_slice(peer_id);                     // peer_id   (20 bytes)
    ann_req.write_u64::<BigEndian>(downloaded)?;
    ann_req.write_u64::<BigEndian>(left)?;
    ann_req.write_u64::<BigEndian>(uploaded)?;
    ann_req.write_u32::<BigEndian>(0)?;                     // event = none
    ann_req.write_u32::<BigEndian>(0)?;                     // ip = default
    ann_req.write_u32::<BigEndian>(rand::random())?;        // key
    ann_req.write_i32::<BigEndian>(-1)?;                    // num_want = -1 (default)
    ann_req.write_u16::<BigEndian>(port)?;

    tokio::time::timeout(Duration::from_secs(15), socket.send(&ann_req))
        .await
        .map_err(|_| UdpTrackerError::Timeout)??;

    // Allocate enough for the header (20 bytes) + up to 500 peers (3000 bytes)
    let mut ann_resp = vec![0u8; 20 + 6 * 500];
    let n = tokio::time::timeout(Duration::from_secs(15), socket.recv(&mut ann_resp))
        .await
        .map_err(|_| UdpTrackerError::Timeout)??;

    if n < 20 {
        return Err(UdpTrackerError::InvalidResponse(
            "Announce response too short".to_string(),
        ));
    }
    ann_resp.truncate(n);

    let mut cursor = Cursor::new(&ann_resp);
    let action = cursor.read_u32::<BigEndian>()?;
    let resp_txid = cursor.read_u32::<BigEndian>()?;

    if action == 3 {
        // Error response
        let msg = String::from_utf8_lossy(&ann_resp[8..]).to_string();
        return Err(UdpTrackerError::TrackerError(msg));
    }
    if action != 1 {
        return Err(UdpTrackerError::InvalidResponse(
            "Unexpected action in announce response".to_string(),
        ));
    }
    if resp_txid != announce_transaction_id {
        return Err(UdpTrackerError::InvalidResponse(
            "Announce transaction ID mismatch".to_string(),
        ));
    }

    let interval = cursor.read_u32::<BigEndian>()? as u64;
    let _leechers = cursor.read_u32::<BigEndian>()?;
    let _seeders = cursor.read_u32::<BigEndian>()?;

    // Remaining bytes are peers: 4 bytes IP + 2 bytes port each
    let peer_data = &ann_resp[20..];
    let mut peers = Vec::new();
    for chunk in peer_data.chunks_exact(6) {
        let ip = IpAddr::V4(Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3]));
        let port = u16::from_be_bytes([chunk[4], chunk[5]]);
        if port != 0 {
            peers.push(PeerAddr { ip, port });
        }
    }

    tracing::info!(
        "UDP tracker returned {} peers (re-announce in {}s)",
        peers.len(),
        interval
    );

    Ok(AnnounceResponse { interval, peers })
}

/// Announce to a UDP tracker.
pub async fn announce(
    tracker_url: &str,
    info_hash: &[u8; 20],
    peer_id: &[u8; 20],
    port: u16,
    uploaded: u64,
    downloaded: u64,
    left: u64,
) -> Result<AnnounceResponse, UdpTrackerError> {
    tracing::info!("Announcing to UDP tracker: {tracker_url}");

    let (host, tracker_port) = parse_udp_url(tracker_url)?;

    // Resolve hostname to IP
    let addr_str = format!("{}:{}", host, tracker_port);
    let mut addrs = tokio::net::lookup_host(&addr_str)
        .await
        .map_err(|e| UdpTrackerError::InvalidUrl(format!("DNS lookup failed: {}", e)))?;

    let addr = addrs
        .next()
        .ok_or_else(|| UdpTrackerError::InvalidUrl("No addresses found".to_string()))?;

    connect_and_announce(addr, info_hash, peer_id, port, uploaded, downloaded, left).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_udp_url() {
        let (host, port) = parse_udp_url("udp://tracker.example.com:6969").unwrap();
        assert_eq!(host, "tracker.example.com");
        assert_eq!(port, 6969);
    }

    #[test]
    fn test_parse_udp_url_with_path() {
        let (host, port) = parse_udp_url("udp://tracker.opentrackr.org:1337/announce").unwrap();
        assert_eq!(host, "tracker.opentrackr.org");
        assert_eq!(port, 1337);
    }

    #[test]
    fn test_parse_udp_url_invalid() {
        assert!(parse_udp_url("http://tracker.example.com:6969").is_err());
        assert!(parse_udp_url("udp://invalid").is_err());
    }
}
