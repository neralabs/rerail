//! Per-node memo cells of two bits (0 not yet, 1, 2, 3: the matcher's verdict codes), packed four
//! to a byte and allocated on the first `set`: a file's memo costs n/4 bytes per memoised rule
//! instead of n (or a hash entry per node).

#[derive(Default)]
pub struct Trits {
    bits: Vec<u8>,
}

impl Trits {
    /// The cell of node `n` (0 when never set).
    #[inline]
    pub fn get(&self, n: u32) -> u8 {
        self.bits.get(n as usize >> 2).map_or(0, |b| (b >> ((n & 3) * 2)) & 3)
    }

    /// Sets node `n`'s cell to `v` (< 4), allocating cells for `len` nodes on first use.
    #[inline]
    pub fn set(&mut self, len: usize, n: u32, v: u8) {
        debug_assert!(v < 4 && (n as usize) < len);
        if self.bits.is_empty() {
            self.bits = vec![0; len.div_ceil(4)];
        }
        let (i, s) = (n as usize >> 2, (n & 3) * 2);
        self.bits[i] = (self.bits[i] & !(3 << s)) | (v << s);
    }
}
