//! # Bencode Module
//!
//! A hand written, zero copy bencode encoder/decoder for the BitTorrent protocol.
//! Supports integers, byte strings, lists and dictionaries.

pub mod decoder;
pub mod encoder;

use std::collections::BTreeMap;
use std::fmt;

/// A Bencode value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// An integer, e.g. `i42e`.
    Integer(i64),
    /// A byte string, e.g. `4:spam`.
    Bytes(Vec<u8>),
    /// An ordered list of values, e.g. `l4:spam4:eggse`.
    List(Vec<Value>),
    /// A dictionary (ordered by keys), e.g. `d3:cow3:moo4:spam4:eggse`.
    Dict(BTreeMap<Vec<u8>, Value>),
}

impl Value {
    /// Tries to interpret this value as an integer
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Value::Integer(n) => Some(*n),
            _ => None,
        }
    }

    /// Tries to interpret this value as a byte slice
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }

    /// Tries to interpret this value as a UTF-8 string
    pub fn as_str(&self) -> Option<&str> {
        self.as_bytes().and_then(|b| std::str::from_utf8(b).ok())
    }

    /// Tries to interpret this value as a list
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(l) => Some(l),
            _ => None,
        }
    }

    /// Tries to interpret this value as a dictionary
    pub fn as_dict(&self) -> Option<&BTreeMap<Vec<u8>, Value>> {
        match self {
            Value::Dict(d) => Some(d),
            _ => None,
        }
    }

    /// Look up a key in a dictionary value
    pub fn get(&self, key: &[u8]) -> Option<&Value> {
        self.as_dict().and_then(|d| d.get(key))
    }

    /// Look up a key by string in a dictionary value
    pub fn get_str(&self, key: &str) -> Option<&Value> {
        self.get(key.as_bytes())
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Integer(n) => write!(f, "{n}"),

            // Try displaying byte strings as UTF-8 when possible
            // Fall back to a byte count for arbitrary binary data
            Value::Bytes(b) => match std::str::from_utf8(b) {
                Ok(s) => write!(f, "\"{s}\""),
                Err(_) => write!(f, "<{} bytes>", b.len()),
            },

            Value::List(l) => {
                write!(f, "[")?;

                for (i, v) in l.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }

                    write!(f, "{v}")?;
                }

                write!(f, "]")
            }

            Value::Dict(d) => {
                write!(f, "{{")?;

                for (i, (k, v)) in d.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }

                    match std::str::from_utf8(k) {
                        Ok(s) => write!(f, "\"{s}\": {v}")?,
                        Err(_) => write!(f, "<{} bytes>: {v}", k.len())?,
                    }
                }

                write!(f, "}}")
            }
        }
    }
}
