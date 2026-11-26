//! A stored tape read in place (PIAT §20; decisions 0025, 0026, 0032, 0336): its format from its magic, its
//! language's table, and that table's codes as the device reads them, once a slab (decision 0336 §7).

use super::*;

/// The bytes of a stored tape's header a reader decodes first (`slabs::planned`): its magic, tag and revision, its
/// source's length and its node count.
pub const HEAD: usize = 32;

/// What admission reads of format 16 (for a reader that does not link the format's crate).
pub use rerail_tape_format::f16::{Tape16Z, placement};
pub use rerail_tape_format::home::{Home, Room};

/// A stored image's format, from its magic: the device reads format 16's arrays in place; a 16z image is placed as
/// stored and read through a cache of its decoded blocks (decision 0353 step 4b).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stored {
    F16,
    F16Z,
}

/// The format of the image whose first bytes are `head`.
pub fn stored(head: &[u8]) -> Result<Stored, String> {
    match head.get(..8) {
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

// tests: a test the engine hosts (it reads above this crate; tools/cratify/test-hosts.tsv)
