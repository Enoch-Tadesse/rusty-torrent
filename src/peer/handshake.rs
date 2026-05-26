//! BitTorrent handshake protocol.
//!
//! `<pstrlen:1><pstr:19><reserved:8><info_hash:20><peer_id:20>` = 68 bytes.

use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const PROTOCOL: &[u8] = b"BitTorrent protocol";
const HANDSHAKE_SIZE: usize = 68;

#[derive(Debug, Error)]
pub enum HandshakeError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid protocol length: {0}")]
    InvalidProtocolLength(u8),
    #[error("Unknown protocol")]
    UnknownProtocol,
    #[error("Info hash mismatch")]
    InfoHashMismatch,
}

#[derive(Debug, Clone)]
pub struct HandshakeResult {
    pub peer_id: [u8; 20],
    pub info_hash: [u8; 20],
}

/// Builds a 68-byte BitTorrent handshake message
///
/// Layout:
/// - 1 byte protocol string length
/// - 19 bytes protocol identifier
/// - 8 reserved bytes
/// - 20 bytes info hash
/// - 20 bytes peer id
///
/// The reserved bytes are currently set to zero
pub fn build_handshake(info_hash: &[u8; 20], peer_id: &[u8; 20]) -> [u8; HANDSHAKE_SIZE] {
    let mut buf = [0u8; HANDSHAKE_SIZE];
    buf[0] = PROTOCOL.len() as u8;
    buf[1..20].copy_from_slice(PROTOCOL);
    buf[28..48].copy_from_slice(info_hash);
    buf[48..68].copy_from_slice(peer_id);
    buf
}

/// Sends a handshake to the peer and waits for a valid response
///
/// This function:
/// 1. Builds the outgoing handshake packet
/// 2. Writes it to the stream
/// 3. Flushes buffered writers if necessary
/// 4. Reads the peer handshake response
/// 5. Verifies the returned info hash matches the expected torrent
///
/// Returns the peer's `peer_id` and `info_hash` on success
pub async fn perform_handshake<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    info_hash: &[u8; 20],
    peer_id: &[u8; 20],
) -> Result<HandshakeResult, HandshakeError> {
    let outgoing = build_handshake(info_hash, peer_id);
    stream.write_all(&outgoing).await?;
    // Important when using buffered streams such as BufStream.
    // Without flushing, the peer may never receive the handshake.
    stream.flush().await?;
    let result = read_handshake(stream).await?;
    if result.info_hash != *info_hash {
        return Err(HandshakeError::InfoHashMismatch);
    }
    Ok(result)
}

/// Reads and validates a BitTorrent handshake from a stream.
///
/// Validation steps:
/// - Checks protocol string length
/// - Verifies the protocol identifier
/// - Extracts the info hash
/// - Extracts the peer id
///
/// Does not verify the info hash against an expected value.
/// That validation is handled by `perform_handshake`.
pub async fn read_handshake<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<HandshakeResult, HandshakeError> {
    let mut pstrlen_buf = [0u8; 1];
    reader.read_exact(&mut pstrlen_buf).await?;
    if pstrlen_buf[0] as usize != PROTOCOL.len() {
        return Err(HandshakeError::InvalidProtocolLength(pstrlen_buf[0]));
    }
    let mut buf = [0u8; 67];
    reader.read_exact(&mut buf).await?;
    if &buf[..19] != PROTOCOL {
        return Err(HandshakeError::UnknownProtocol);
    }
    let mut info_hash = [0u8; 20];
    let mut peer_id = [0u8; 20];
    info_hash.copy_from_slice(&buf[27..47]);
    peer_id.copy_from_slice(&buf[47..67]);
    Ok(HandshakeResult { peer_id, info_hash })
}
