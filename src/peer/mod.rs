//! # Peer Module
//!
//! BitTorrent peer wire protocol implementation.
//! Handles handshakes, message serialization, bitfield tracking, and async peer clients.

pub mod bitfield;
pub mod client;
pub mod handshake;
pub mod message;

use std::fmt;
use std::net::IpAddr;

/// A peer's network address.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PeerAddr {
    pub ip: IpAddr,
    pub port: u16,
}

impl fmt::Display for PeerAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.ip, self.port)
    }
}
