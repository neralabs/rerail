//! A stored source's or tape's bytes as the store's reads hand them out: read in place in their segment (decision
//! 0295 step 5). The store keeps no decompressed copies, as no source or tape is compressed (2026-11-19).

use super::CachedBytes;
use std::ops::Deref;

impl CachedBytes {
    /// Bytes read where they are in a segment.
    pub fn mapped(bytes: super::SegmentBytes) -> Self {
        Self { bytes }
    }
}

impl Deref for CachedBytes {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        self.bytes.bytes()
    }
}
