//! Torrent metainfo — parses `.torrent` files into strongly typed structs.
//!
//! Handles both single file and multi file torrents. Computes the `info_hash`
//! by re-encoding the raw `info` dictionary and hashing with SHA-1.

use crate::bencode::{decoder, Value};
use sha1::{Digest, Sha1};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MetainfoError {
    #[error("Failed to read torrent file: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to decode bencode: {0}")]
    Decode(#[from] decoder::DecodeError),

    #[error("Missing required field: {0}")]
    MissingField(&'static str),

    #[error("Invalid field type for '{0}'")]
    InvalidField(&'static str),

    #[error("Piece hashes length ({0}) is not a multiple of 20")]
    MalformedPieces(usize),

    #[error("Failed to find 'info' dictionary in raw bytes")]
    InfoDictNotFound,
}

/// Represents a single file within a torrent
#[derive(Debug, Clone)]
pub struct FileInfo {
    /// Length of the file in bytes
    pub length: u64,
    /// Path components (e.g., ["dir", "subdir", "file.txt"])
    pub path: Vec<String>,
}

/// Parsed torrent metainfo.
#[derive(Debug, Clone)]
pub struct Metainfo {
    /// Primary tracker announce URL
    pub announce: String,
    /// Optional list of backup tracker URLs
    pub announce_list: Vec<Vec<String>>,
    /// SHA-1 hash of the bencoded `info` dictionary (20 bytes)
    pub info_hash: [u8; 20],
    /// Name of the torrent (top-level directory or file name)
    pub name: String,
    /// Length of each piece in bytes
    pub piece_length: u64,
    /// SHA-1 hashes for each piece (each is 20 bytes)
    pub piece_hashes: Vec<[u8; 20]>,
    /// Total length of all files combined
    pub total_length: u64,
    /// Files in this torrent
    pub files: Vec<FileInfo>,
    /// Whether this is a single-file torrent
    pub is_single_file: bool,
}

impl Metainfo {
    /// Parse a `.torrent` file from disk
    pub fn from_file(path: &std::path::Path) -> Result<Self, MetainfoError> {
        let data = std::fs::read(path)?;
        Self::from_bytes(&data)
    }

    /// Parse torrent metainfo from raw bencoded bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self, MetainfoError> {
        let value = decoder::decode(data)?;

        let _dict = value.as_dict().ok_or(MetainfoError::InvalidField("root"))?;

        // Extract announce URL
        let announce = value
            .get_str("announce")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Extract announce-list (optional)
        let announce_list = Self::parse_announce_list(&value);

        // Extract the info dictionary
        let info = value
            .get_str("info")
            .ok_or(MetainfoError::MissingField("info"))?;

        let _info_dict = info
            .as_dict()
            .ok_or(MetainfoError::InvalidField("info"))?;

        // Compute info_hash by finding the raw `info` value in the original bytes
        // and hashing it with SHA-1.
        let info_hash = Self::compute_info_hash(data)?;

        // Extract common info fields
        let name = info
            .get_str("name")
            .and_then(|v| v.as_str())
            .ok_or(MetainfoError::MissingField("info.name"))?
            .to_string();

        let piece_length = info
            .get_str("piece length")
            .and_then(|v| v.as_integer())
            .ok_or(MetainfoError::MissingField("info.piece length"))?
            as u64;

        let pieces_raw = info
            .get_str("pieces")
            .and_then(|v| v.as_bytes())
            .ok_or(MetainfoError::MissingField("info.pieces"))?;

        let piece_hashes = Self::split_piece_hashes(pieces_raw)?;

        // Determine single-file vs multi-file
        let (files, total_length, is_single_file) =
            if let Some(length_val) = info.get_str("length") {
                // Single-file torrent
                let length = length_val
                    .as_integer()
                    .ok_or(MetainfoError::InvalidField("info.length"))?
                    as u64;
                let files = vec![FileInfo {
                    length,
                    path: vec![name.clone()],
                }];
                (files, length, true)
            } else if let Some(files_val) = info.get_str("files") {
                // Multi-file torrent
                let file_list = files_val
                    .as_list()
                    .ok_or(MetainfoError::InvalidField("info.files"))?;

                let mut files = Vec::new();
                let mut total = 0u64;

                for file_val in file_list {
                    let _file_dict = file_val
                        .as_dict()
                        .ok_or(MetainfoError::InvalidField("info.files[].dict"))?;

                    let length = file_val
                        .get_str("length")
                        .and_then(|v| v.as_integer())
                        .ok_or(MetainfoError::MissingField("info.files[].length"))?
                        as u64;

                    let path_list = file_val
                        .get_str("path")
                        .and_then(|v| v.as_list())
                        .ok_or(MetainfoError::MissingField("info.files[].path"))?;

                    let path: Vec<String> = path_list
                        .iter()
                        .filter_map(|p| p.as_str().map(|s| s.to_string()))
                        .collect();

                    total += length;
                    files.push(FileInfo { length, path });
                }

                (files, total, false)
            } else {
                return Err(MetainfoError::MissingField("info.length or info.files"));
            };

        Ok(Metainfo {
            announce,
            announce_list,
            info_hash,
            name,
            piece_length,
            piece_hashes,
            total_length,
            files,
            is_single_file,
        })
    }

    /// Compute the SHA-1 hash of the raw bencoded `info` dictionary.
    fn compute_info_hash(data: &[u8]) -> Result<[u8; 20], MetainfoError> {
        // We need to find the raw bytes of the info dictionary in the original data.
        // Look for "4:info" key in the top-level dictionary.
        let info_key = b"4:infod";
        let mut pos = None;

        for i in 0..data.len().saturating_sub(info_key.len()) {
            if &data[i..i + info_key.len()] == info_key {
                pos = Some(i + 6); // skip "4:info", point to the 'd'
                break;
            }
        }

        let start = pos.ok_or(MetainfoError::InfoDictNotFound)?;

        // Parse from this position to find where the info dict ends
        let (_, consumed) = decoder::decode_partial(&data[start..])?;

        let info_bytes = &data[start..start + consumed];
        let mut hasher = Sha1::new();
        hasher.update(info_bytes);
        let result = hasher.finalize();
        let mut hash = [0u8; 20];
        hash.copy_from_slice(&result);
        Ok(hash)
    }

    /// Split the concatenated piece hashes into individual 20-byte SHA-1 hashes.
    fn split_piece_hashes(raw: &[u8]) -> Result<Vec<[u8; 20]>, MetainfoError> {
        if raw.len() % 20 != 0 {
            return Err(MetainfoError::MalformedPieces(raw.len()));
        }

        let count = raw.len() / 20;
        let mut hashes = Vec::with_capacity(count);

        for i in 0..count {
            let mut hash = [0u8; 20];
            hash.copy_from_slice(&raw[i * 20..(i + 1) * 20]);
            hashes.push(hash);
        }

        Ok(hashes)
    }

    /// Parse the optional `announce-list` field.
    fn parse_announce_list(root: &Value) -> Vec<Vec<String>> {
        root.get_str("announce-list")
            .and_then(|v| v.as_list())
            .map(|tiers| {
                tiers
                    .iter()
                    .filter_map(|tier| {
                        tier.as_list().map(|urls| {
                            urls.iter()
                                .filter_map(|u| u.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get the output path for this torrent.
    #[allow(dead_code)]
    pub fn output_path(&self, base_dir: &std::path::Path) -> PathBuf {
        base_dir.join(&self.name)
    }

    /// Calculate the expected length of a specific piece.
    pub fn piece_size(&self, index: usize) -> u64 {
        let start = index as u64 * self.piece_length;
        let remaining = self.total_length.saturating_sub(start);
        std::cmp::min(self.piece_length, remaining)
    }

    /// Format the total size as a human-readable string.
    pub fn human_size(&self) -> String {
        let bytes = self.total_length as f64;
        if bytes < 1024.0 {
            format!("{bytes:.0} B")
        } else if bytes < 1024.0 * 1024.0 {
            format!("{:.1} KiB", bytes / 1024.0)
        } else if bytes < 1024.0 * 1024.0 * 1024.0 {
            format!("{:.1} MiB", bytes / (1024.0 * 1024.0))
        } else {
            format!("{:.2} GiB", bytes / (1024.0 * 1024.0 * 1024.0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_piece_hashes() {
        let raw = vec![0u8; 60]; // 3 hashes
        let hashes = Metainfo::split_piece_hashes(&raw).unwrap();
        assert_eq!(hashes.len(), 3);
    }

    #[test]
    fn test_split_piece_hashes_invalid() {
        let raw = vec![0u8; 25]; // Not a multiple of 20
        assert!(Metainfo::split_piece_hashes(&raw).is_err());
    }

    #[test]
    fn test_human_size() {
        let mut meta = Metainfo {
            announce: String::new(),
            announce_list: vec![],
            info_hash: [0; 20],
            name: "test".into(),
            piece_length: 256 * 1024,
            piece_hashes: vec![],
            total_length: 1024 * 1024 * 512,
            files: vec![],
            is_single_file: true,
        };
        assert_eq!(meta.human_size(), "512.0 MiB");

        meta.total_length = 1024;
        assert_eq!(meta.human_size(), "1.0 KiB");
    }
}
