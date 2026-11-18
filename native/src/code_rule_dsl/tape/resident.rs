//! A stored tape image read in place (PIAT §20; decisions 0025, 0026, 0032): the column layout
//! of a v9np image's bytes, and what a reader of those bytes derives once per admission: a
//! fixed-width table of the image's codes, and each node's parent from one stack pass over the
//! sizes. Neither is stored; they are the reader's state beside the image (decision 0026).

use super::*;

/// The header's bytes: magic, language, widths, version, source length, five lengths. (v12 dropped
/// the source's digest: the store pairs a tape with its source.)
const HEADER: usize = HEADER_BYTES;
/// `HEADER`, for a reader that decodes the header alone first.
pub const HEADER_LEN: usize = HEADER;

/// What admission reads of format 16 (for a reader that does not link the format's crate).
pub use rerail_tape_format::f16::{Tape16Z, placement};

/// A stored image's format, from its magic: the device reads format 15's columns and format 16's arrays in place;
/// a 16z image is laid out as the format 16 tape it is the blocks of where it is placed (its pieces decoded on the device, `residency::slabs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stored {
    F15,
    F16,
    F16Z,
}

/// The format of the image whose first bytes are `head`.
pub fn stored(head: &[u8]) -> Result<Stored, String> {
    match head.get(..8) {
        Some(m) if m == TAPE_MAGIC => Ok(Stored::F15),
        Some(m) if m == rerail_tape_format::f16::MAGIC16 => Ok(Stored::F16),
        Some(m) if m == rerail_tape_format::f16::MAGIC16Z => Ok(Stored::F16Z),
        _ => Err("wrong tape image magic".into()),
    }
}

/// The language of a format 16 or 16z image whose first bytes are `head`: its table and choices.
pub fn lang_of(head: &[u8]) -> Result<&'static rerail_tape_format::f16::Lang, String> {
    let tag = *head.get(8).ok_or("truncated tape image")?;
    rerail_tape_format::f16::lang(tag).ok_or_else(|| format!("a format 16 tape of tag {tag}, which has no table"))
}

/// The device's codes for a code table (format 16's: its language's), named as `TapeDoc` names its nodes.
pub fn device_codes_of(table: &rerail_tape_format::tape::CodeTable, grammar: &Grammar) -> Result<Vec<DeviceCode>, String> {
    let mut codes = Vec::with_capacity(table.len());
    for n in 0..table.len() as u32 {
        let e = table.entry(n);
        if e.classes.len() > MAX_NODE_SUPERTYPES {
            return Err("a code with more classes than the device reads".into());
        }
        let error = e.flags & CODE_ERROR != 0;
        let kind = if error { ERROR_SYMBOL } else { e.kind };
        let named = e.alias.as_ref().map_or_else(|| grammar.kind_is_named(kind), |alias| alias.1);
        let mut classes = [0_u16; MAX_NODE_SUPERTYPES];
        classes[..e.classes.len()].copy_from_slice(&e.classes);
        codes.push(DeviceCode {
            kind,
            field: e.field,
            flags: u8::from(named) | (u8::from(e.flags & CODE_EXTRA != 0) << 1) | (u8::from(e.flags & CODE_MISSING != 0) << 2) | (u8::from(error) << 3),
            class_count: e.classes.len() as u8,
            reserved: 0,
            classes,
        });
    }
    Ok(codes)
}

/// The device's codes of the table a document's tape numbers its nodes by (format 16's: its language's): what the
/// reads written once (decision 0310) index by a node's number, for its kind, field, flags and classes.
pub fn doc_codes(doc: &TapeDoc) -> Result<Vec<DeviceCode>, String> {
    rerail_tape_format::with_tape!(doc.tape(), |t| device_codes_of(t.table(), doc.grammar()))
}

/// What a document's codes are keyed by where documents share a table (format 16's: its language's): the table.
pub fn codes_key(doc: &TapeDoc) -> usize {
    rerail_tape_format::with_tape!(doc.tape(), |t| std::ptr::from_ref(t.table()) as usize)
}

/// The node and code counts an image's header declares, after its magic and version.
pub fn header_counts(header: &[u8]) -> Result<(usize, usize), String> {
    if header.len() < HEADER || &header[..8] != TAPE_MAGIC {
        return Err("wrong tape image magic".into());
    }
    if u32::from_le_bytes(header[header::VERSION..header::VERSION + 4].try_into().expect("four bytes")) != TAPE_FORMAT_VERSION {
        return Err("unsupported tape image version".into());
    }
    Ok((u64_at(header, header::NODES)?, u64_at(header, header::CODES)?))
}
const _: () = assert!(HEADER == HEADER_BYTES && HEADER % COLUMN_ALIGN == 0);

/// Where an image's columns and tables are, as byte offsets from the image's start.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Layout {
    pub nodes: u32,
    /// The widths of `size`, `off` and `tail` (2 or 4).
    pub widths: [u8; 3],
    /// Tails are measured from the node's own start (`TAIL_FROM_START`).
    pub from_start: bool,
    pub code: usize,
    pub size: usize,
    pub base: usize,
    pub off: usize,
    pub tail: usize,
    pub(in crate::code_rule_dsl) codes: usize,
    pub code_count: usize,
    aliases: usize,
    alias_count: usize,
    pub source_len: usize,
}

/// A code as the device reads it: 24 bytes, fixed. `flags` are `TapeDoc::transducer_flags`' bits
/// (named, extra, missing, error); `kind` is `transducer_kind`'s (ERROR is `ERROR_SYMBOL`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DeviceCode {
    pub kind: u16,
    pub field: u16,
    pub flags: u8,
    pub class_count: u8,
    pub(in crate::code_rule_dsl) reserved: u16,
    pub classes: [u16; MAX_NODE_SUPERTYPES],
}
const _: () = assert!(std::mem::size_of::<DeviceCode>() == 24);

fn u64_at(bytes: &[u8], at: usize) -> Result<usize, String> {
    let value = u64::from_le_bytes(bytes.get(at..at + 8).ok_or("truncated tape image")?.try_into().expect("eight bytes"));
    usize::try_from(value).map_err(|_| "tape image length overflow".to_owned())
}

/// The layout of the image `bytes`, checked against its header: magic, version, widths and every
/// length, as `TapeImage::from_bytes` checks them, without copying a column.
pub fn layout(bytes: &[u8]) -> Result<Layout, String> {
    if bytes.len() < HEADER || &bytes[..8] != TAPE_MAGIC {
        return Err("wrong tape image magic".into());
    }
    let widths = [bytes[header::WIDTHS], bytes[header::WIDTHS + 1], bytes[header::WIDTHS + 2]];
    if !matches!(widths[0], 2 | 4) || !matches!(widths[1], 2 | 4) || !matches!(widths[2] & !TAIL_FROM_START, 2 | 4) {
        return Err("noncanonical tape image header".into());
    }
    if u32::from_le_bytes(bytes[header::VERSION..header::VERSION + 4].try_into().expect("four bytes")) != TAPE_FORMAT_VERSION {
        return Err("unsupported tape image version".into());
    }
    let source_len = u64_at(bytes, header::SOURCE_LEN)?;
    let [nodes, code_count, code_bytes, alias_count, alias_bytes] =
        [0, 1, 2, 3, 4].map(|at| u64_at(bytes, header::NODES + 8 * at));
    let (nodes, code_count, code_bytes, alias_count, alias_bytes) = (nodes?, code_count?, code_bytes?, alias_count?, alias_bytes?);
    if nodes == 0 || nodes > u32::MAX as usize || code_count == 0 || code_count > usize::from(u16::MAX) + 1 {
        return Err("invalid tape image table lengths".into());
    }
    let tail_width = widths[2] & !TAIL_FROM_START;
    let lengths = [
        nodes * 2,
        nodes * usize::from(widths[0]),
        nodes.div_ceil(CHECKPOINT_STRIDE) * 4,
        nodes * usize::from(widths[1]),
        nodes * usize::from(tail_width),
    ];
    let mut at = HEADER;
    let mut starts = [0; 5];
    for (start, length) in starts.iter_mut().zip(lengths) {
        *start = at;
        at += padded(length);
    }
    if at + code_bytes + alias_bytes != bytes.len() {
        return Err("invalid tape image table lengths".into());
    }
    Ok(Layout {
        nodes: nodes as u32,
        widths: [widths[0], widths[1], tail_width],
        from_start: widths[2] & TAIL_FROM_START != 0,
        code: starts[0],
        size: starts[1],
        base: starts[2],
        off: starts[3],
        tail: starts[4],
        codes: at,
        code_count,
        aliases: at + code_bytes,
        alias_count,
        source_len,
    })
}

/// The image's codes as the device reads them, named as `TapeDoc` names its nodes (an alias's
/// flag, else the grammar's).
pub fn device_codes(bytes: &[u8], layout: &Layout, grammar: &Grammar) -> Result<Vec<DeviceCode>, String> {
    let truncated = || "truncated tape image".to_owned();
    // The aliases' named flags, in order.
    let mut named = Vec::with_capacity(layout.alias_count);
    let mut at = layout.aliases;
    for _ in 0..layout.alias_count {
        named.push(*bytes.get(at).ok_or_else(truncated)? == 1);
        let length = u32::from_le_bytes(bytes.get(at + 1..at + 5).ok_or_else(truncated)?.try_into().expect("four bytes"));
        at += 5 + length as usize;
    }
    let mut codes = Vec::with_capacity(layout.code_count);
    let mut at = layout.codes;
    for _ in 0..layout.code_count {
        let entry = bytes.get(at..at + 6).ok_or_else(truncated)?;
        let kf = u16::from_le_bytes([entry[0], entry[1]]);
        let flags = entry[2];
        let alias = u16::from_le_bytes([entry[3], entry[4]]);
        let class_count = entry[5];
        if usize::from(class_count) > MAX_NODE_SUPERTYPES {
            return Err("noncanonical tape code".into());
        }
        let mut classes = [0_u16; MAX_NODE_SUPERTYPES];
        for (slot, class) in classes.iter_mut().enumerate().take(usize::from(class_count)) {
            let at = at + 6 + 2 * slot;
            *class = u16::from_le_bytes(bytes.get(at..at + 2).ok_or_else(truncated)?.try_into().expect("two bytes"));
        }
        at += 6 + 2 * usize::from(class_count);
        let error = flags & CODE_ERROR != 0;
        let kind = if error { ERROR_SYMBOL } else { kf & 1023 };
        let is_named = if alias == SENTINEL {
            grammar.kind_is_named(kind)
        } else {
            *named.get(usize::from(alias)).ok_or("noncanonical tape code")?
        };
        codes.push(DeviceCode {
            kind,
            field: kf >> 10,
            flags: u8::from(is_named) | (u8::from(flags & CODE_EXTRA != 0) << 1) | (u8::from(flags & CODE_MISSING != 0) << 2) | (u8::from(error) << 3),
            class_count,
            reserved: 0,
            classes,
        });
    }
    Ok(codes)
}

/// A column cell (2 or 4 bytes, little-endian) of row `i`.
pub fn cell_at(bytes: &[u8], column: usize, width: u8, i: usize) -> u32 {
    let at = column + i * usize::from(width);
    if width == 2 { u32::from(u16::from_le_bytes([bytes[at], bytes[at + 1]])) } else { u32::from_le_bytes(bytes[at..at + 4].try_into().expect("u32 width")) }
}

/// Node `i`'s subtree size.
pub fn size_at(bytes: &[u8], layout: &Layout, i: usize) -> u32 {
    cell_at(bytes, layout.size, layout.widths[0], i)
}

/// Each node's parent (`u32::MAX` at the root) into `out`, from the sizes: the enclosing subtree on
/// a stack, as `restore_tree` rebuilds them. Fails on a size past the tape's end.
pub fn parents(bytes: &[u8], layout: &Layout, out: &mut [u32]) -> Result<(), String> {
    let count = layout.nodes as usize;
    if out.len() != count {
        return Err("parent column length".into());
    }
    let mut open: Vec<(u32, u64)> = Vec::new();
    for (i, parent) in out.iter_mut().enumerate() {
        while open.last().is_some_and(|&(_, end)| end <= i as u64) {
            open.pop();
        }
        *parent = open.last().map_or(NONE, |&(at, _)| at);
        let size = u64::from(size_at(bytes, layout, i));
        if size == 0 || i as u64 + size > count as u64 {
            return Err("invalid subtree boundary".into());
        }
        open.push((i as u32, i as u64 + size));
    }
    Ok(())
}

// tests: a test the engine hosts (it reads above this crate; tools/cratify/test-hosts.tsv)
