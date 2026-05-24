//! Bencode decoder — recursive-descent parser
//!
//! Parses a byte slice into a [`Value`] tree

use super::Value;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("Unexpected end of input at position {0}")]
    UnexpectedEof(usize),

    #[error("Invalid integer at position {0}")]
    InvalidInteger(usize),

    #[error("Invalid string length at position {0}")]
    InvalidStringLength(usize),

    #[error("Unexpected byte '{0}' at position {1}")]
    UnexpectedByte(u8, usize),

    #[error("Dictionary keys must be byte strings at position {0}")]
    InvalidDictKey(usize),

    #[error("Trailing data after position {0}")]
    TrailingData(usize),
}

/// A cursor based decoder that tracks its position through the input
struct Decoder<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Peek at the current byte without advancing
    fn peek(&self) -> Result<u8, DecodeError> {
        self.data
            .get(self.pos)
            .copied()
            .ok_or(DecodeError::UnexpectedEof(self.pos))
    }

    /// Consume one byte and advance the cursor
    fn advance(&mut self) -> Result<u8, DecodeError> {
        let byte = self.peek()?;
        self.pos += 1;
        Ok(byte)
    }

    /// Expect a specific byte, returning an error if it doesn't match
    fn expect(&mut self, expected: u8) -> Result<(), DecodeError> {
        let byte = self.advance()?;
        if byte != expected {
            Err(DecodeError::UnexpectedByte(byte, self.pos - 1))
        } else {
            Ok(())
        }
    }

    /// Parse any bencode value based on the leading byte
    fn parse_value(&mut self) -> Result<Value, DecodeError> {
        match self.peek()? {
            b'i' => self.parse_integer(),
            b'l' => self.parse_list(),
            b'd' => self.parse_dict(),
            b'0'..=b'9' => self.parse_bytes(),
            other => Err(DecodeError::UnexpectedByte(other, self.pos)),
        }
    }

    /// Parse an integer: `i<number>e`
    fn parse_integer(&mut self) -> Result<Value, DecodeError> {
        self.expect(b'i')?;
        let start = self.pos;

        // Find the terminating 'e'
        while self.pos < self.data.len() && self.data[self.pos] != b'e' {
            self.pos += 1;
        }

        let num_str = std::str::from_utf8(&self.data[start..self.pos])
            .map_err(|_| DecodeError::InvalidInteger(start))?;

        let num: i64 = num_str
            .parse()
            .map_err(|_| DecodeError::InvalidInteger(start))?;

        self.expect(b'e')?;
        Ok(Value::Integer(num))
    }

    /// Parse a byte string: `<length>:<data>`
    fn parse_bytes(&mut self) -> Result<Value, DecodeError> {
        let start = self.pos;

        // Parse the length prefix
        while self.pos < self.data.len() && self.data[self.pos] != b':' {
            self.pos += 1;
        }

        let len_str = std::str::from_utf8(&self.data[start..self.pos])
            .map_err(|_| DecodeError::InvalidStringLength(start))?;

        let length: usize = len_str
            .parse()
            .map_err(|_| DecodeError::InvalidStringLength(start))?;

        self.expect(b':')?;

        if self.pos + length > self.data.len() {
            return Err(DecodeError::UnexpectedEof(self.pos));
        }

        let bytes = self.data[self.pos..self.pos + length].to_vec();
        self.pos += length;
        Ok(Value::Bytes(bytes))
    }

    /// Parse a list: `l<values>e`
    fn parse_list(&mut self) -> Result<Value, DecodeError> {
        self.expect(b'l')?;
        let mut items = Vec::new();

        while self.peek()? != b'e' {
            items.push(self.parse_value()?);
        }

        self.expect(b'e')?;
        Ok(Value::List(items))
    }

    /// Parse a dictionary: `d<key><value>...e`
    /// Keys must be byte strings and appear in sorted order per the spec
    fn parse_dict(&mut self) -> Result<Value, DecodeError> {
        self.expect(b'd')?;
        let mut map = BTreeMap::new();

        while self.peek()? != b'e' {
            let key_pos = self.pos;
            let key_val = self.parse_value()?;
            let key = match key_val {
                Value::Bytes(b) => b,
                _ => return Err(DecodeError::InvalidDictKey(key_pos)),
            };
            let value = self.parse_value()?;
            map.insert(key, value);
        }

        self.expect(b'e')?;
        Ok(Value::Dict(map))
    }
}

/// Decode a bencoded byte slice into a [`Value`]
pub fn decode(data: &[u8]) -> Result<Value, DecodeError> {
    let mut decoder = Decoder::new(data);
    let value = decoder.parse_value()?;

    if decoder.pos != data.len() {
        return Err(DecodeError::TrailingData(decoder.pos));
    }

    Ok(value)
}

/// Decode a bencoded byte slice, returning the value and the number of bytes consumed
/// This is useful when parsing sub-sections (e.g. the `info` dict for hashing)
pub fn decode_partial(data: &[u8]) -> Result<(Value, usize), DecodeError> {
    let mut decoder = Decoder::new(data);
    let value = decoder.parse_value()?;
    Ok((value, decoder.pos))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_integer() {
        assert_eq!(decode(b"i42e").unwrap(), Value::Integer(42));
        assert_eq!(decode(b"i-7e").unwrap(), Value::Integer(-7));
        assert_eq!(decode(b"i0e").unwrap(), Value::Integer(0));
    }

    #[test]
    fn test_decode_bytes() {
        assert_eq!(
            decode(b"4:spam").unwrap(),
            Value::Bytes(b"spam".to_vec())
        );
        assert_eq!(decode(b"0:").unwrap(), Value::Bytes(vec![]));
    }

    #[test]
    fn test_decode_list() {
        let val = decode(b"l4:spam4:eggse").unwrap();
        assert_eq!(
            val,
            Value::List(vec![
                Value::Bytes(b"spam".to_vec()),
                Value::Bytes(b"eggs".to_vec()),
            ])
        );
    }

    #[test]
    fn test_decode_dict() {
        let val = decode(b"d3:cow3:moo4:spam4:eggse").unwrap();
        let dict = val.as_dict().unwrap();
        assert_eq!(
            dict.get(&b"cow"[..]).unwrap(),
            &Value::Bytes(b"moo".to_vec())
        );
        assert_eq!(
            dict.get(&b"spam"[..]).unwrap(),
            &Value::Bytes(b"eggs".to_vec())
        );
    }

    #[test]
    fn test_decode_nested() {
        let val = decode(b"d4:listli1ei2ei3ee5:valuei42ee").unwrap();
        let list = val.get_str("list").unwrap().as_list().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(val.get_str("value").unwrap().as_integer().unwrap(), 42);
    }

    #[test]
    fn test_decode_error_trailing() {
        assert!(decode(b"i42eXXX").is_err());
    }

    #[test]
    fn test_decode_error_eof() {
        assert!(decode(b"i42").is_err());
    }
}
