//! Bitfield — tracks which pieces a peer has.
//!
//! Each bit represents one piece. Bit 7 of byte 0 is piece 0, bit 6 is piece 1, etc.

/// A bitfield indicating which pieces a peer has available.
#[derive(Debug, Clone)]
pub struct Bitfield {
    data: Vec<u8>,
    num_pieces: usize,
}

impl Bitfield {
    /// Create a new empty bitfield for `num_pieces` pieces.
    pub fn new(num_pieces: usize) -> Self {
        let num_bytes = (num_pieces + 7) / 8;
        Self {
            data: vec![0u8; num_bytes],
            num_pieces,
        }
    }

    /// Create a bitfield from raw bytes received from a peer.
    pub fn from_bytes(data: Vec<u8>, num_pieces: usize) -> Self {
        Self { data, num_pieces }
    }

    /// Check if the peer has a specific piece.
    pub fn has_piece(&self, index: usize) -> bool {
        if index >= self.num_pieces {
            return false;
        }
        let byte_index = index / 8;
        let bit_index = index % 8;
        if byte_index >= self.data.len() {
            return false;
        }
        self.data[byte_index] & (1 << (7 - bit_index)) != 0
    }

    /// Mark a piece as available.
    pub fn set_piece(&mut self, index: usize) {
        if index >= self.num_pieces {
            return;
        }
        let byte_index = index / 8;
        let bit_index = index % 8;
        if byte_index < self.data.len() {
            self.data[byte_index] |= 1 << (7 - bit_index);
        }
    }

    /// Clear a piece (mark as unavailable).
    #[allow(dead_code)]
    pub fn clear_piece(&mut self, index: usize) {
        if index >= self.num_pieces {
            return;
        }
        let byte_index = index / 8;
        let bit_index = index % 8;
        if byte_index < self.data.len() {
            self.data[byte_index] &= !(1 << (7 - bit_index));
        }
    }

    /// Count the number of pieces this peer has.
    #[allow(dead_code)]
    pub fn count(&self) -> usize {
        let mut count = 0;
        for i in 0..self.num_pieces {
            if self.has_piece(i) {
                count += 1;
            }
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_bitfield() {
        let bf = Bitfield::new(16);
        assert_eq!(bf.data.len(), 2);
        assert!(!bf.has_piece(0));
        assert!(!bf.has_piece(15));
    }

    #[test]
    fn test_set_and_has() {
        let mut bf = Bitfield::new(24);
        bf.set_piece(0);
        bf.set_piece(7);
        bf.set_piece(15);
        bf.set_piece(23);

        assert!(bf.has_piece(0));
        assert!(bf.has_piece(7));
        assert!(bf.has_piece(15));
        assert!(bf.has_piece(23));
        assert!(!bf.has_piece(1));
        assert!(!bf.has_piece(8));
    }

    #[test]
    fn test_clear() {
        let mut bf = Bitfield::new(8);
        bf.set_piece(3);
        assert!(bf.has_piece(3));
        bf.clear_piece(3);
        assert!(!bf.has_piece(3));
    }

    #[test]
    fn test_count() {
        let mut bf = Bitfield::new(16);
        bf.set_piece(0);
        bf.set_piece(5);
        bf.set_piece(10);
        assert_eq!(bf.count(), 3);
    }

    #[test]
    fn test_out_of_bounds() {
        let mut bf = Bitfield::new(8);
        bf.set_piece(100); // should not panic
        assert!(!bf.has_piece(100));
    }

    #[test]
    fn test_from_bytes() {
        let data = vec![0b10110000, 0b00000001];
        let bf = Bitfield::from_bytes(data, 16);
        assert!(bf.has_piece(0)); // bit 7 of byte 0
        assert!(!bf.has_piece(1)); // bit 6 of byte 0
        assert!(bf.has_piece(2)); // bit 5 of byte 0
        assert!(bf.has_piece(3)); // bit 4 of byte 0
        assert!(bf.has_piece(15)); // bit 0 of byte 1
    }
}
