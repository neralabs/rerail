//! A document's tape as its reads are told where it lies: byte offsets from a base (the document's
//! first byte in its batch's chunk block on the device, decision 0289 §4; the tape's own bytes on the
//! host), its format, its widths. One definition: the device's `DocView` is this, and its MSL is
//! written from it ([`msl`]), with each field's offset asserted against Rust's.

/// A document's view (120 bytes, u32s; the device's `DocView`, `metal_tape.metal`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TapeView {
    /// The document's first byte in its batch's chunk block (the device's; 0 on the host): every
    /// offset below counts from it, so a copy into another block changes `at` alone (0289 §4).
    pub at: u32,
    pub nodes: u32,
    pub node_base: u32,
    /// The tape's format: 15 or 16 (a 16z tape is laid out as format 16 where it is placed).
    pub format: u32,
    /// Format 15: `size`'s, `off`'s and `tail`'s widths in bytes, at bits 0, 4 and 8; bit 12: tails
    /// measured from the node's own start. Format 16: the slot, gap and length widths at bits 0, 4
    /// and 8, the checkpoints' at 12, and the four side lists' value widths at 16, 20, 24 and 28.
    pub widths: u32,
    /// Format 15's columns (code, size, base, off, tail). Format 16 (block-major, decision 0399): block
    /// 0's start, then the bytes a row holds before the slots, the gaps, the lengths and the checkpoints
    /// (a block of R rows holds column r at `R * columns[r]` from its start).
    pub columns: [u32; 5],
    /// Format 16: the blocks' stride, in bytes (a block is `F16_PART` nodes).
    pub stride: u32,
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
    /// Format 15: line starts (u32, document-local bytes), the first 0. Format 16: its newline index
    /// (`rerail_tape_format::newlines`). And how many lines.
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
const _: () = assert!(std::mem::size_of::<TapeView>() == 120);

/// The view's MSL: `struct TapeView`, `DocView` its name in the kernels, and each field's offset and
/// the size asserted against this definition's, so the kernels cannot read a layout of their own.
pub fn msl() -> String {
    use std::mem::offset_of;
    let fields: [(&str, &str, usize); 20] = [
        ("uint", "at", offset_of!(TapeView, at)),
        ("uint", "nodes", offset_of!(TapeView, nodes)),
        ("uint", "node_base", offset_of!(TapeView, node_base)),
        ("uint", "format", offset_of!(TapeView, format)),
        ("uint", "widths", offset_of!(TapeView, widths)),
        ("uint", "columns[5]", offset_of!(TapeView, columns)),
        ("uint", "stride", offset_of!(TapeView, stride)),
        ("uint", "side[4]", offset_of!(TapeView, side)),
        ("uint", "parts[4]", offset_of!(TapeView, parts)),
        ("uint", "cut", offset_of!(TapeView, cut)),
        ("uint", "per", offset_of!(TapeView, per)),
        ("uint", "codes", offset_of!(TapeView, codes)),
        ("uint", "source", offset_of!(TapeView, source)),
        ("uint", "source_len", offset_of!(TapeView, source_len)),
        ("uint", "lines", offset_of!(TapeView, lines)),
        ("uint", "line_count", offset_of!(TapeView, line_count)),
        ("uint", "code_count", offset_of!(TapeView, code_count)),
        ("uint", "source_base", offset_of!(TapeView, source_base)),
        ("uint", "line_base", offset_of!(TapeView, line_base)),
        ("uint", "tape", offset_of!(TapeView, tape)),
    ];
    let mut s = String::from("struct TapeView {\n");
    for (ty, name, _) in fields.iter() {
        s += &format!("    {ty} {name};\n");
    }
    s += "};\ntypedef TapeView DocView;\n";
    s += &format!("static_assert(sizeof(TapeView) == {}, \"TapeView: size\");\n", std::mem::size_of::<TapeView>());
    for (_, name, at) in fields.iter() {
        let name = name.split('[').next().unwrap_or(name);
        s += &format!("static_assert(__builtin_offsetof(TapeView, {name}) == {at}, \"TapeView.{name}\");\n");
    }
    s
}

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
    pub const STRIDE: i32 = offset_of!(TapeView, stride) as i32;
    pub const SIDE: i32 = offset_of!(TapeView, side) as i32;
    pub const PARTS: i32 = offset_of!(TapeView, parts) as i32;
    pub const CUT: i32 = offset_of!(TapeView, cut) as i32;
    pub const PER: i32 = offset_of!(TapeView, per) as i32;
    pub const LINES: i32 = offset_of!(TapeView, lines) as i32;
    pub const LINE_COUNT: i32 = offset_of!(TapeView, line_count) as i32;
}

#[cfg(test)]
mod msl_tests {
    /// The MSL lists every field: their words add up to the view's.
    #[test]
    fn the_views_msl_names_every_word() {
        let msl = super::msl();
        let words: usize = msl
            .lines()
            .filter_map(|l| l.trim().strip_prefix("uint "))
            .map(|f| f.trim_end_matches(';').split_once('[').map_or(1, |(_, n)| n.trim_end_matches(']').parse::<usize>().unwrap()))
            .sum();
        assert_eq!(4 * words, std::mem::size_of::<super::TapeView>(), "{msl}");
    }
}
