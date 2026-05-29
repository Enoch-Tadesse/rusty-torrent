//! BitTorrent wire protocol messages.
//!
//! Handles serialization and deserialization of all message types defined in BEP 3.

use bytes::{Buf, BufMut};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};

#[derive(Debug, Error)]
pub enum MessageError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid message ID: {0}")]
    InvalidId(u8),

    #[error("Message too large: {0} bytes")]
    TooLarge(u32),

    #[error("Unexpected message type: expected {expected}, got {got}")]
    UnexpectedType { expected: &'static str, got: u8 },

    #[error("Payload too short for {msg_type}: need {need}, got {got}")]
    PayloadTooShort {
        msg_type: &'static str,
        need: usize,
        got: usize,
    },

    #[error("Piece index mismatch: expected {expected}, got {got}")]
    IndexMismatch { expected: u32, got: u32 },
}

/// Message IDs as defined in the BitTorrent protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageId {
    Choke = 0,
    Unchoke = 1,
    Interested = 2,
    NotInterested = 3,
    Have = 4,
    Bitfield = 5,
    Request = 6,
    Piece = 7,
    Cancel = 8,
}

impl MessageId {
    pub fn from_byte(byte: u8) -> Result<Self, MessageError> {
        match byte {
            0 => Ok(Self::Choke),
            1 => Ok(Self::Unchoke),
            2 => Ok(Self::Interested),
            3 => Ok(Self::NotInterested),
            4 => Ok(Self::Have),
            5 => Ok(Self::Bitfield),
            6 => Ok(Self::Request),
            7 => Ok(Self::Piece),
            8 => Ok(Self::Cancel),
            other => Err(MessageError::InvalidId(other)),
        }
    }
}

/// A wire protocol message.
#[derive(Debug, Clone)]
pub struct Message {
    pub id: MessageId,
    pub payload: Vec<u8>,
}

/// Maximum allowed message size (16 MiB) to prevent memory exhaustion.
const MAX_MESSAGE_SIZE: u32 = 16 * 1024 * 1024;

impl Message {
    /// Create a new message with the given ID and no payload.
    pub fn new(id: MessageId) -> Self {
        Self {
            id,
            payload: Vec::new(),
        }
    }

    /// Create a new message with the given ID and payload.
    pub fn with_payload(id: MessageId, payload: Vec<u8>) -> Self {
        Self { id, payload }
    }

    /// Serialize the message into bytes: `<length:4><id:1><payload>`
    pub fn serialize(&self) -> Vec<u8> {
        let length = 1 + self.payload.len() as u32;
        let mut buf = Vec::with_capacity(4 + length as usize);
        buf.put_u32(length);
        buf.put_u8(self.id as u8);
        buf.extend_from_slice(&self.payload);
        buf
    }

    /// Read a message from an async reader.
    /// Returns `None` for keep-alive messages (length == 0).
    pub async fn read<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Option<Self>, MessageError> {
        // Read the 4-byte length prefix
        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf).await?;
        let length = u32::from_be_bytes(len_buf);

        // Keep-alive message
        if length == 0 {
            return Ok(None);
        }

        if length > MAX_MESSAGE_SIZE {
            return Err(MessageError::TooLarge(length));
        }

        // Read the message body
        let mut msg_buf = vec![0u8; length as usize];
        reader.read_exact(&mut msg_buf).await?;

        let id = MessageId::from_byte(msg_buf[0])?;
        let payload = msg_buf[1..].to_vec();

        Ok(Some(Self { id, payload }))
    }

    /// Write this message to an async writer and flush.
    pub async fn write<W: AsyncWrite + Unpin>(&self, writer: &mut W) -> Result<(), MessageError> {
        use tokio::io::AsyncWriteExt;
        let data = self.serialize();
        writer.write_all(&data).await?;
        writer.flush().await?;
        Ok(())
    }

    /// Build a `have` message announcing we have a piece.
    pub fn have(index: u32) -> Self {
        let mut payload = Vec::with_capacity(4);
        payload.put_u32(index);
        Self::with_payload(MessageId::Have, payload)
    }

    /// Build a `request` message asking for a block.
    pub fn request(index: u32, begin: u32, length: u32) -> Self {
        let mut payload = Vec::with_capacity(12);
        payload.put_u32(index);
        payload.put_u32(begin);
        payload.put_u32(length);
        Self::with_payload(MessageId::Request, payload)
    }

    /// Parse a `have` message to get the piece index.
    pub fn parse_have(&self) -> Result<u32, MessageError> {
        if self.id != MessageId::Have {
            return Err(MessageError::UnexpectedType {
                expected: "have",
                got: self.id as u8,
            });
        }
        if self.payload.len() < 4 {
            return Err(MessageError::PayloadTooShort {
                msg_type: "have",
                need: 4,
                got: self.payload.len(),
            });
        }
        let mut slice = &self.payload[..];
        Ok(slice.get_u32())
    }

    /// Parse a `piece` message and copy the data into the provided buffer.
    /// Returns the number of bytes copied.
    pub fn parse_piece(&self, expected_index: u32, buf: &mut [u8]) -> Result<usize, MessageError> {
        if self.id != MessageId::Piece {
            return Err(MessageError::UnexpectedType {
                expected: "piece",
                got: self.id as u8,
            });
        }
        if self.payload.len() < 8 {
            return Err(MessageError::PayloadTooShort {
                msg_type: "piece",
                need: 8,
                got: self.payload.len(),
            });
        }

        let mut slice = &self.payload[..];
        let index = slice.get_u32();
        let begin = slice.get_u32() as usize;

        if index != expected_index {
            return Err(MessageError::IndexMismatch {
                expected: expected_index,
                got: index,
            });
        }

        let block = &self.payload[8..];

        if begin + block.len() > buf.len() {
            return Err(MessageError::PayloadTooShort {
                msg_type: "piece data",
                need: begin + block.len(),
                got: buf.len(),
            });
        }

        buf[begin..begin + block.len()].copy_from_slice(block);
        Ok(block.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_choke() {
        let msg = Message::new(MessageId::Choke);
        let data = msg.serialize();
        assert_eq!(data, vec![0, 0, 0, 1, 0]); // length=1, id=0
    }

    #[test]
    fn test_serialize_have() {
        let msg = Message::have(42);
        let data = msg.serialize();
        // length=5 (1 id + 4 payload), id=4, index=42
        assert_eq!(data, vec![0, 0, 0, 5, 4, 0, 0, 0, 42]);
    }

    #[test]
    fn test_serialize_request() {
        let msg = Message::request(1, 0, 16384);
        let data = msg.serialize();
        assert_eq!(data.len(), 4 + 1 + 12); // 4 length + 1 id + 12 payload
        assert_eq!(data[4], 6); // Request ID
    }

    #[test]
    fn test_parse_have() {
        let msg = Message::have(99);
        assert_eq!(msg.parse_have().unwrap(), 99);
    }

    #[test]
    fn test_parse_piece() {
        let mut payload = Vec::new();
        payload.put_u32(5); // index
        payload.put_u32(0); // begin offset
        payload.extend_from_slice(&[1, 2, 3, 4, 5]); // block data

        let msg = Message::with_payload(MessageId::Piece, payload);

        let mut buf = vec![0u8; 10];
        let n = msg.parse_piece(5, &mut buf).unwrap();
        assert_eq!(n, 5);
        assert_eq!(&buf[..5], &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_parse_piece_wrong_index() {
        let mut payload = Vec::new();
        payload.put_u32(5);
        payload.put_u32(0);
        payload.extend_from_slice(&[1, 2, 3]);

        let msg = Message::with_payload(MessageId::Piece, payload);
        assert!(msg.parse_piece(3, &mut [0u8; 10]).is_err());
    }

    #[test]
    fn test_message_id_roundtrip() {
        for id in 0..=8u8 {
            let msg_id = MessageId::from_byte(id).unwrap();
            assert_eq!(msg_id as u8, id);
        }
        assert!(MessageId::from_byte(9).is_err());
    }
}
