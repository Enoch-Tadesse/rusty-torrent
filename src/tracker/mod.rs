//! # Tracker Module
//!
//! HTTP and UDP tracker communication for the BitTorrent protocol.

pub mod http;
pub mod udp;

use crate::peer::PeerAddr;

/// Response from a tracker announce.
#[derive(Debug, Clone)]
pub struct AnnounceResponse {
    /// Interval in seconds before the next announce.
    pub interval: u64,
    /// List of peers returned by the tracker.
    pub peers: Vec<PeerAddr>,
}

