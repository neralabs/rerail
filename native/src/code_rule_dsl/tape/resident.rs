//! A stored tape image read in place (PIAT §20; decisions 0025, 0026, 0032): the column layout
//! of a v9np image's bytes, and what a reader of those bytes derives once per admission: a
//! fixed-width table of the image's codes, and each node's parent from one stack pass over the
//! sizes. Neither is stored; they are the reader's state beside the image (decision 0026).

use super::*;

/// The header's bytes: magic, language, widths, version, source length, five lengths. (v12 dropped
/// the source's digest: the store pairs a tape with its source.)
const HEADER: usize = HEADER_BYTES;
/// `HEADER`, for a reader that decodes the header alone first.
pub(in crate::code_rule_dsl) const HEADER_LEN: usize = HEADER;

/// The node and code counts an image's header declares, after its magic and version.
pub(in crate::code_rule_dsl) fn header_counts(header: &[u8]) -> Result<(usize, usize), String> {
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
pub(in crate::code_rule_dsl) struct Layout {
    pub(in crate::code_rule_dsl) nodes: u32,
    /// The widths of `size`, `off` and `tail` (2 or 4).
    pub(in crate::code_rule_dsl) widths: [u8; 3],
    /// Tails are measured from the node's own start (`TAIL_FROM_START`).
    pub(in crate::code_rule_dsl) from_start: bool,
    /// The image holds named nodes and kept tokens only (`NAMED_ONLY`, RER-131): the device
    /// recognizes the rest (`token_parts`).
    pub(in crate::code_rule_dsl) named_only: bool,
    pub(in crate::code_rule_dsl) code: usize,
    pub(in crate::code_rule_dsl) size: usize,
    pub(in crate::code_rule_dsl) base: usize,
    pub(in crate::code_rule_dsl) off: usize,
    pub(in crate::code_rule_dsl) tail: usize,
    pub(in crate::code_rule_dsl) codes: usize,
    pub(in crate::code_rule_dsl) code_count: usize,
    aliases: usize,
    alias_count: usize,
    pub(in crate::code_rule_dsl) source_len: usize,
}

/// A code as the device reads it: 24 bytes, fixed. `flags` are `TapeDoc::transducer_flags`' bits
/// (named, extra, missing, error) and, in a named-only image, `DEVICE_TOKENS_KEPT`; `kind` is
/// `transducer_kind`'s (ERROR is `ERROR_SYMBOL`). `reserved` is, in a named-only image, the
/// record's name in the grammar's token tables (`TokenGrammar::device_sym`), else `NO_SYM`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct DeviceCode {
    pub(in crate::code_rule_dsl) kind: u16,
    pub(in crate::code_rule_dsl) field: u16,
    pub(in crate::code_rule_dsl) flags: u8,
    pub(in crate::code_rule_dsl) class_count: u8,
    pub(in crate::code_rule_dsl) reserved: u16,
    pub(in crate::code_rule_dsl) classes: [u16; MAX_NODE_SUPERTYPES],
}
const _: () = assert!(std::mem::size_of::<DeviceCode>() == 24);
/// A named-only record that keeps all of its tokens (`CODE_TOKENS`): none is recognized.
pub(in crate::code_rule_dsl) const DEVICE_TOKENS_KEPT: u8 = 16;

fn u64_at(bytes: &[u8], at: usize) -> Result<usize, String> {
    let value = u64::from_le_bytes(bytes.get(at..at + 8).ok_or("truncated tape image")?.try_into().expect("eight bytes"));
    usize::try_from(value).map_err(|_| "tape image length overflow".to_owned())
}

/// The layout of the image `bytes`, checked against its header: magic, version, widths and every
/// length, as `TapeImage::from_bytes` checks them, without copying a column.
pub(in crate::code_rule_dsl) fn layout(bytes: &[u8]) -> Result<Layout, String> {
    if bytes.len() < HEADER || &bytes[..8] != TAPE_MAGIC {
        return Err("wrong tape image magic".into());
    }
    // A named-only image (RER-131) has the same columns over its named nodes and kept tokens: the records a
    // named-only document holds in memory, so the device reads it in place.
    let named_only = bytes[header::WIDTHS] & NAMED_ONLY != 0;
    let widths = [bytes[header::WIDTHS] & !NAMED_ONLY, bytes[header::WIDTHS + 1], bytes[header::WIDTHS + 2]];
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
        named_only,
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
pub(in crate::code_rule_dsl) fn device_codes(bytes: &[u8], layout: &Layout, grammar: &Grammar) -> Result<Vec<DeviceCode>, String> {
    let truncated = || "truncated tape image".to_owned();
    // The aliases' named flags and names, in order.
    let mut named = Vec::with_capacity(layout.alias_count);
    let mut names = Vec::with_capacity(layout.alias_count);
    let mut at = layout.aliases;
    for _ in 0..layout.alias_count {
        named.push(*bytes.get(at).ok_or_else(truncated)? == 1);
        let length = u32::from_le_bytes(bytes.get(at + 1..at + 5).ok_or_else(truncated)?.try_into().expect("four bytes"));
        names.push(std::str::from_utf8(bytes.get(at + 5..at + 5 + length as usize).ok_or_else(truncated)?).map_err(|_| "noncanonical tape alias")?);
        at += 5 + length as usize;
    }
    // A named-only image's token tables name each record (`reserved`).
    let tokens = if layout.named_only {
        let language = grammar.kinds.grammar().ok_or("a named-only tape of a grammar tree-sitter does not parse")?;
        Some(tokens::TokenGrammar::of(grammar.lang, language).ok_or("a named-only tape without its grammar's automata")?)
    } else {
        None
    };
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
        let alias_name = (alias != SENTINEL).then(|| names[usize::from(alias)]);
        codes.push(DeviceCode {
            kind,
            field: kf >> 10,
            flags: u8::from(is_named)
                | (u8::from(flags & CODE_EXTRA != 0) << 1)
                | (u8::from(flags & CODE_MISSING != 0) << 2)
                | (u8::from(error) << 3)
                | if layout.named_only && flags & CODE_TOKENS != 0 { DEVICE_TOKENS_KEPT } else { 0 },
            class_count,
            reserved: tokens.as_ref().map_or(tokens::NO_SYM, |tokens| tokens.device_sym(kf & 1023, alias_name, error)),
            classes,
        });
    }
    Ok(codes)
}

/// A column cell (2 or 4 bytes, little-endian) of row `i`.
fn cell_at(bytes: &[u8], column: usize, width: u8, i: usize) -> u32 {
    let at = column + i * usize::from(width);
    if width == 2 { u32::from(u16::from_le_bytes([bytes[at], bytes[at + 1]])) } else { u32::from_le_bytes(bytes[at..at + 4].try_into().expect("u32 width")) }
}

/// Node `i`'s subtree size.
pub(in crate::code_rule_dsl) fn size_at(bytes: &[u8], layout: &Layout, i: usize) -> u32 {
    cell_at(bytes, layout.size, layout.widths[0], i)
}

/// Each node's parent (`u32::MAX` at the root) into `out`, from the sizes: the enclosing subtree on
/// a stack, as `restore_tree` rebuilds them. Fails on a size past the tape's end.
pub(in crate::code_rule_dsl) fn parents(bytes: &[u8], layout: &Layout, out: &mut [u32]) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Node `i` read from the image's bytes in place, as the device reads it.
    fn read(bytes: &[u8], layout: &Layout, codes: &[DeviceCode], i: usize) -> (DeviceCode, u32, u32, u32) {
        let code = codes[cell_at(bytes, layout.code, 2, i) as usize];
        let start = |i: usize| cell_at(bytes, layout.base, 4, i / CHECKPOINT_STRIDE) + cell_at(bytes, layout.off, layout.widths[1], i);
        let size = size_at(bytes, layout, i);
        let tail = cell_at(bytes, layout.tail, layout.widths[2], i);
        let end = if layout.from_start { start(i) } else { start(i + size as usize - 1) } + tail;
        (code, start(i), end, size)
    }

    #[test]
    fn an_image_read_in_place_is_the_restored_tape() {
        for (source, lang) in [
            ("value = left + right\nclass A:\n    def f(self, x=[]):\n        return (yield x)\n", GrammarDialect::Python),
            ("const f = (a, b) => { return a?.b ?? `t${b}`; };\n// c\n", GrammarDialect::JavaScript),
            ("fn main() { let x: Vec<u8> = vec![1, 2]; if let Some(y) = x.first() { println!(\"{y}\"); } }\n", GrammarDialect::Rust),
            ("def broken(:\n    pass\n", GrammarDialect::Python),
        ] {
            let grammar = Grammar::new(lang).unwrap();
            let native = ast_grep_core::tree_sitter::StrDoc::new(source, lang);
            let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap().primary;
            let tape = TapeDoc::restore(Arc::new(source.to_owned()), Arc::clone(&grammar), &image, &|| false).unwrap();
            // What the device reads: the document's full image (a tree-sitter tape is stored
            // named-only and restored whole, RER-131).
            assert!(image.is_named_only() == tape.restored_from_named_only());
            let bytes = tape.resident_parts().unwrap().0;
            let layout = layout(&bytes).unwrap();
            for column in [layout.code, layout.size, layout.base, layout.off, layout.tail] {
                assert_eq!(column % COLUMN_ALIGN, 0, "{lang:?}: columns are aligned");
            }
            assert_eq!(layout.nodes as usize, tape.transducer_len());
            assert_eq!(layout.source_len, source.len());
            let codes = device_codes(&bytes, &layout, &grammar).unwrap();
            let mut parents_column = vec![0; layout.nodes as usize];
            parents(&bytes, &layout, &mut parents_column).unwrap();
            for i in 0..layout.nodes {
                let (code, start, end, size) = read(&bytes, &layout, &codes, i as usize);
                assert_eq!(code.kind, tape.transducer_kind(i), "{lang:?} node {i} kind");
                assert_eq!(code.field, tape.transducer_field(i), "{lang:?} node {i} field");
                assert_eq!(code.flags, tape.transducer_flags(i), "{lang:?} node {i} flags");
                assert_eq!(start as usize..end as usize, tape.transducer_span(i), "{lang:?} node {i} span");
                assert_eq!(i + 1..i + size, tape.transducer_descendants(i), "{lang:?} node {i} size");
                assert_eq!((parents_column[i as usize] != NONE).then_some(parents_column[i as usize]), tape.transducer_parent(i), "{lang:?} node {i} parent");
                let classes = tape.transducer_supertypes().filter(|&(node, _)| node == i).map(|(_, class)| class).collect::<Vec<_>>();
                assert_eq!(&code.classes[..usize::from(code.class_count)], classes.as_slice(), "{lang:?} node {i} classes");
            }
        }
    }
}
