//! A document's tape as its reads are told where it lies: byte offsets from a base (the slab a
//! document is placed in on the device, the tape's own bytes on the host), its format, its widths.
//! One definition: the device's `DocView` is this.

/// A document's view (116 bytes, u32s; the device's `DocView`, `metal_tape.metal`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TapeView {
    /// The batch's slab binding the document is in (the device's).
    pub slab: u32,
    pub nodes: u32,
    pub node_base: u32,
    /// The tape's format: 15 or 16 (a 16z tape is laid out as format 16 where it is placed).
    pub format: u32,
    /// Format 15: `size`'s, `off`'s and `tail`'s widths in bytes, at bits 0, 4 and 8; bit 12: tails
    /// measured from the node's own start. Format 16: the slot, gap and length widths at bits 0, 4
    /// and 8, the checkpoints' at 12, and the four side lists' value widths at 16, 20, 24 and 28.
    pub widths: u32,
    /// Format 15's columns (code, size, base, off, tail), or format 16's arrays (codes, slots, gaps,
    /// lengths, checkpoints).
    pub columns: [u32; 5],
    /// Format 16: the four side lists (codes' low parts, escaped slots, gaps, lengths), and each
    /// one's part starts (u32s, a part a 1,024 nodes).
    pub side: [u32; 4],
    pub parts: [u32; 4],
    /// Format 16: its language's cutoff and the numbers a prefix byte covers.
    pub cut: u32,
    pub per: u32,
    /// The code table (the device's `DeviceCode`s; format 16's: the language's).
    pub codes: u32,
    pub source: u32,
    pub source_len: u32,
    /// Line starts (u32, document-local bytes), the first 0, and how many.
    pub lines: u32,
    pub line_count: u32,
    /// The code table's length.
    pub code_count: u32,
    /// The document's place in a batch's numbering of source bytes and lines.
    pub source_base: u32,
    pub line_base: u32,
    /// Where the stored tape starts.
    pub tape: u32,
}
const _: () = assert!(std::mem::size_of::<TapeView>() == 116);

/// Format 16's numbers the reads are written for (`rerail_tape_format::f16`'s; a test there holds
/// them equal): checkpoints every `K` nodes, side lists in parts of `PART` nodes, blocks of `BLOCK`.
pub const F16_K: u32 = 8;
pub const F16_BLOCK: u32 = 256;
pub const F16_PART: u32 = 1024;
pub const F16_SUB: u32 = F16_PART / F16_BLOCK;
/// Format 15's checkpoint stride (`base`'s, a u32 every 16 nodes) and its tails-from-start flag.
pub const F15_STRIDE_SHIFT: u32 = 4;
pub const F15_TAIL_FROM_START: u32 = 1 << 12;

/// A field's byte offset in a view.
pub mod at {
    use super::TapeView;
    use std::mem::offset_of;
    pub const NODES: i32 = offset_of!(TapeView, nodes) as i32;
    pub const FORMAT: i32 = offset_of!(TapeView, format) as i32;
    pub const WIDTHS: i32 = offset_of!(TapeView, widths) as i32;
    pub const COLUMNS: i32 = offset_of!(TapeView, columns) as i32;
    pub const SIDE: i32 = offset_of!(TapeView, side) as i32;
    pub const PARTS: i32 = offset_of!(TapeView, parts) as i32;
    pub const CUT: i32 = offset_of!(TapeView, cut) as i32;
    pub const PER: i32 = offset_of!(TapeView, per) as i32;
}
