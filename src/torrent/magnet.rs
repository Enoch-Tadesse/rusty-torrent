//! Magnet URI parser.
//!
//! Parses `magnet:?xt=urn:btih:<info_hash>&dn=<name>&tr=<tracker>` URIs.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MagnetError {
    #[error("Invalid magnet URI: must start with 'magnet:?'")]
    InvalidScheme,

    #[error("Missing 'xt' parameter (exact topic / info hash)")]
    MissingInfoHash,

    #[error("Invalid info hash: expected 40 hex characters, got {0}")]
    InvalidInfoHash(usize),

    #[error("Info hash must use 'urn:btih:' prefix")]
    InvalidUrn,

    #[error("Failed to decode hex info hash: {0}")]
    HexDecode(String),
}

/// A parsed magnet link.
#[derive(Debug, Clone)]
pub struct MagnetLink {
    /// The 20-byte info hash
    pub info_hash: [u8; 20],
    /// Display name (optional)
    pub display_name: Option<String>,
    /// Tracker URLs (optional, may have multiple)
    pub trackers: Vec<String>,
}

impl MagnetLink {
    /// Parse a magnet URI string.
    pub fn parse(uri: &str) -> Result<Self, MagnetError> {
        if !uri.starts_with("magnet:?") {
            return Err(MagnetError::InvalidScheme);
        }

        let query = &uri[8..]; // skip "magnet:?"
        let params: Vec<(&str, &str)> = query
            .split('&')
            .filter_map(|pair| {
                let mut parts = pair.splitn(2, '=');
                Some((parts.next()?, parts.next().unwrap_or("")))
            })
            .collect();

        // Extract info hash from xt parameter
        let xt = params
            .iter()
            .find(|(k, _)| *k == "xt")
            .map(|(_, v)| *v)
            .ok_or(MagnetError::MissingInfoHash)?;

        let info_hash = Self::parse_xt(xt)?;

        // Extract display name
        let display_name = params
            .iter()
            .find(|(k, _)| *k == "dn")
            .map(|(_, v)| url_decode(v));

        // Extract trackers
        let trackers: Vec<String> = params
            .iter()
            .filter(|(k, _)| *k == "tr")
            .map(|(_, v)| url_decode(v))
            .collect();

        Ok(MagnetLink {
            info_hash,
            display_name,
            trackers,
        })
    }

    /// Parse the `xt` parameter: `urn:btih:<hex_hash>`
    fn parse_xt(xt: &str) -> Result<[u8; 20], MagnetError> {
        let hash_hex = xt.strip_prefix("urn:btih:").ok_or(MagnetError::InvalidUrn)?;

        if hash_hex.len() != 40 {
            return Err(MagnetError::InvalidInfoHash(hash_hex.len()));
        }

        let mut info_hash = [0u8; 20];
        for i in 0..20 {
            info_hash[i] = u8::from_str_radix(&hash_hex[i * 2..i * 2 + 2], 16)
                .map_err(|e| MagnetError::HexDecode(e.to_string()))?;
        }

        Ok(info_hash)
    }

    /// Format the info hash as a hex string
    pub fn info_hash_hex(&self) -> String {
        self.info_hash
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

/// Simple percent-decoding for URL parameters
fn url_decode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars();

    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            } else {
                result.push('%');
                result.push_str(&hex);
            }
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_magnet() {
        let uri = "magnet:?xt=urn:btih:da39a3ee5e6b4b0d3255bfef95601890afd80709&dn=test+file&tr=http%3A%2F%2Ftracker.example.com%3A6969%2Fannounce";
        let magnet = MagnetLink::parse(uri).unwrap();

        assert_eq!(magnet.display_name.as_deref(), Some("test file"));
        assert_eq!(magnet.trackers.len(), 1);
        assert_eq!(
            magnet.trackers[0],
            "http://tracker.example.com:6969/announce"
        );
        assert_eq!(
            magnet.info_hash_hex(),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
    }

    #[test]
    fn test_invalid_magnet() {
        assert!(MagnetLink::parse("http://example.com").is_err());
        assert!(MagnetLink::parse("magnet:?dn=test").is_err());
    }

    #[test]
    fn test_url_decode() {
        assert_eq!(url_decode("hello%20world"), "hello world");
        assert_eq!(url_decode("hello+world"), "hello world");
        assert_eq!(
            url_decode("http%3A%2F%2Fexample.com"),
            "http://example.com"
        );
    }
}
