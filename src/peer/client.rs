//! Async per-peer client.
//!
//! Manages a single TCP connection to a peer, handling handshake, bitfield
//! exchange, and message send/receive.

use super::bitfield::Bitfield;
use super::handshake;
use super::message::{Message, MessageError, MessageId};
use super::PeerAddr;
use std::time::Instant;
use thiserror::Error;
use tokio::io::BufStream;
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("Connection failed: {0}")]
    Connect(#[from] std::io::Error),
    #[error("Handshake failed: {0}")]
    Handshake(#[from] handshake::HandshakeError),
    #[error("Message error: {0}")]
    Message(#[from] MessageError),
    #[error("Connection timed out")]
    Timeout,
}

/// Statistics tracked per peer for scoring.
#[derive(Debug, Clone)]
pub struct PeerStats {
    pub bytes_downloaded: u64,
    pub pieces_completed: u32,
    pub failed_attempts: u32,
    #[allow(dead_code)]
    pub connected_at: Instant,
    pub last_activity: Instant,
}

impl PeerStats {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            bytes_downloaded: 0,
            pieces_completed: 0,
            failed_attempts: 0,
            connected_at: now,
            last_activity: now,
        }
    }

    /// Speed in bytes per second.
    #[allow(dead_code)]
    pub fn download_speed(&self) -> f64 {
        let elapsed = self.last_activity.duration_since(self.connected_at);
        if elapsed.as_secs_f64() < 0.001 {
            return 0.0;
        }
        self.bytes_downloaded as f64 / elapsed.as_secs_f64()
    }

    /// Peer score: higher is better. Penalizes failures exponentially.
    #[allow(dead_code)]
    pub fn score(&self) -> f64 {
        let speed = self.download_speed();
        let penalty = 2.0_f64.powi(self.failed_attempts as i32);
        speed / penalty
    }
}

/// An active connection to a single peer.
pub struct PeerClient {
    #[allow(dead_code)]
    pub addr: PeerAddr,
    pub stream: BufStream<TcpStream>,
    pub choked: bool,
    pub bitfield: Bitfield,
    #[allow(dead_code)]
    pub peer_id: [u8; 20],
    pub stats: PeerStats,
}

impl PeerClient {
    /// Connect to a peer, perform handshake, and receive bitfield.
    pub async fn connect(
        addr: &PeerAddr,
        our_peer_id: &[u8; 20],
        info_hash: &[u8; 20],
        num_pieces: usize,
        connect_timeout: Duration,
    ) -> Result<Self, ClientError> {
        // Connect with timeout
        let tcp = timeout(connect_timeout, TcpStream::connect(addr.to_string()))
            .await
            .map_err(|_| ClientError::Timeout)?
            .map_err(ClientError::Connect)?;

        let mut stream = BufStream::new(tcp);

        // Perform handshake with timeout
        let hs_result = timeout(
            Duration::from_secs(10),
            handshake::perform_handshake(&mut stream, info_hash, our_peer_id),
        )
        .await
        .map_err(|_| ClientError::Timeout)??;

        // Receive bitfield with a generous timeout, many peers send it right
        // after the handshake but some are slow. We wait up to 10 s and
        // accept an empty bitfield (leecher with no pieces yet) rather than
        // failing the whole connection.
        let bitfield = timeout(
            Duration::from_secs(10),
            Self::recv_bitfield(&mut stream, num_pieces),
        )
        .await
        .unwrap_or_else(|_| Ok(Bitfield::new(num_pieces)))?;

        Ok(Self {
            addr: addr.clone(),
            stream,
            choked: true,
            bitfield,
            peer_id: hs_result.peer_id,
            stats: PeerStats::new(),
        })
    }

    /// Receive the bitfield message from the peer.
    ///
    /// After the handshake the peer may send:
    ///   • keep-alive (length == 0)  → wait for the next message
    ///   • Bitfield                  → parse and return it
    ///   • Unchoke / Have / Choke    → ignore and keep reading
    ///   • Anything else             → assume no bitfield, return empty
    ///
    /// Returning an empty bitfield for a leecher is fine; the engine will
    /// simply not send it any work and move on to the next peer.
    async fn recv_bitfield(
        stream: &mut BufStream<TcpStream>,
        num_pieces: usize,
    ) -> Result<Bitfield, ClientError> {
        loop {
            match Message::read(stream).await? {
                // Keep-alive,  keep waiting
                None => continue,
                Some(m) if m.id == MessageId::Bitfield => {
                    return Ok(Bitfield::from_bytes(m.payload, num_pieces));
                }
                Some(m)
                    if m.id == MessageId::Unchoke
                        || m.id == MessageId::Have
                        || m.id == MessageId::Choke
                        || m.id == MessageId::Interested
                        || m.id == MessageId::NotInterested =>
                {
                    // These can arrive before the bitfield, keep reading.
                    continue;
                }
                Some(_) => {
                    // Any other message type means the peer won't send a
                    // bitfield.  Return an empty one (safe: no pieces → no
                    // work assigned to this peer).
                    return Ok(Bitfield::new(num_pieces));
                }
            }
        }
    }

    /// Send an interested message.
    pub async fn send_interested(&mut self) -> Result<(), ClientError> {
        Message::new(MessageId::Interested)
            .write(&mut self.stream)
            .await?;
        Ok(())
    }

    /// Send a have message.
    pub async fn send_have(&mut self, index: u32) -> Result<(), ClientError> {
        Message::have(index).write(&mut self.stream).await?;
        Ok(())
    }

    /// Send a request message.
    pub async fn send_request(
        &mut self,
        index: u32,
        begin: u32,
        length: u32,
    ) -> Result<(), ClientError> {
        Message::request(index, begin, length)
            .write(&mut self.stream)
            .await?;
        Ok(())
    }

    /// Read the next message from the peer.
    pub async fn read_message(&mut self) -> Result<Option<Message>, ClientError> {
        let msg = Message::read(&mut self.stream).await?;
        self.stats.last_activity = Instant::now();
        Ok(msg)
    }

    /// Process an incoming message, updating internal state.
    pub fn handle_message(&mut self, msg: &Message) {
        match msg.id {
            MessageId::Choke => self.choked = true,
            MessageId::Unchoke => self.choked = false,
            MessageId::Have => {
                if let Ok(index) = msg.parse_have() {
                    self.bitfield.set_piece(index as usize);
                }
            }
            _ => {}
        }
    }

    /// Record that we downloaded some bytes.
    pub fn record_download(&mut self, bytes: u64) {
        self.stats.bytes_downloaded += bytes;
        self.stats.last_activity = Instant::now();
    }

    /// Record a completed piece.
    pub fn record_piece_complete(&mut self) {
        self.stats.pieces_completed += 1;
    }

    /// Record a failure.
    pub fn record_failure(&mut self) {
        self.stats.failed_attempts += 1;
    }
}
