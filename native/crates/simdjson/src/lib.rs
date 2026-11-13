//! simdjson (the C++ library, vendored at `native/vendor/simdjson-5.0.5`): a JSON document parsed
//! and validated in full by its DOM parser, and the structural index its first stage found.
//!
//! The index holds, in order, the offset of each structural byte (`{ } [ ] : ,`) and of each
//! scalar's first byte (a string's opening quote, a number's or a literal's first byte). Each
//! thread parses with a parser of its own.

use std::ffi::{CStr, c_char};

unsafe extern "C" {
    fn rerail_simdjson_structurals(src: *const c_char, len: usize, indexes: *mut *const u32, count: *mut u32) -> i32;
    fn rerail_simdjson_release_above(keep: usize);
    fn rerail_simdjson_error_message(code: i32) -> *const c_char;
    fn rerail_simdjson_implementation() -> *const c_char;
}

/// A thread keeps its parser's buffers for files up to this size; a larger file's are freed once
/// it is read (simdjson's DOM parser holds about 14 bytes a source byte).
const KEEP: usize = 1 << 20;

/// Why simdjson refused a document: its error code and message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error(pub i32);

impl Error {
    pub fn message(self) -> &'static str {
        // SAFETY: simdjson's messages are static, NUL-terminated strings.
        unsafe { CStr::from_ptr(rerail_simdjson_error_message(self.0)) }.to_str().unwrap_or("simdjson error")
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// Parses `src` with simdjson and gives `read` its structural index; `Err` where simdjson refuses it
/// (invalid JSON, an empty document, a number past its range, nesting past 1,024).
pub fn structurals<R>(src: &[u8], read: impl FnOnce(&[u32]) -> R) -> Result<R, Error> {
    let (mut indexes, mut count) = (std::ptr::null(), 0u32);
    // SAFETY: `src` is read for `len` bytes; the index stays valid until this thread's next call,
    // which nothing in `read` can make before it returns (the slice is not kept past it).
    let code = unsafe { rerail_simdjson_structurals(src.as_ptr().cast(), src.len(), &mut indexes, &mut count) };
    let result = if code != 0 {
        Err(Error(code))
    } else {
        let index = if count == 0 { &[][..] } else { unsafe { std::slice::from_raw_parts(indexes, count as usize) } };
        Ok(read(index))
    };
    if src.len() > KEEP {
        unsafe { rerail_simdjson_release_above(KEEP) };
    }
    result
}

/// The kinds and fields a JSON tape is written with: tree-sitter-json's ids, which the caller looks up.
#[derive(Clone, Copy, Debug)]
pub struct Kinds {
    pub document: u16,
    pub object: u16,
    pub pair: u16,
    pub array: u16,
    pub string: u16,
    pub string_content: u16,
    pub escape_sequence: u16,
    pub number: u16,
    pub true_: u16,
    pub false_: u16,
    pub null: u16,
    pub key_field: u16,
    pub value_field: u16,
}

/// A JSON document's tape, written straight from simdjson's index in the engine's v9np columns
/// (`tape::TapeImage`): tree-sitter-json's nodes in preorder (`document`, `object`, `pair` with its `key` and
/// `value`, `array`, `string` with its `string_content` and `escape_sequence`, `number`, `true`, `false`,
/// `null`), without the tokens each node implies.
///
/// One pass over the index writes every column at the node's preorder slot `ord`:
/// - at a node's start, its `code` (numbered by first use in the file), its `off` from its block's
///   `base`, and the `base` of each block of 16 nodes (starts never decrease in preorder);
/// - at its end, its `size` (the nodes written since it) and its `tail` (its end less the start of
///   the last node written, its last descendant).
///
/// `size`, `off` and `tail` are written as `u32`s, then narrowed to 2 bytes where the file's largest
/// value fits, as the tape stores them.
#[derive(Clone, Debug, Default)]
pub struct Tape {
    /// Each node's code, an index into `codes`, 2 bytes little-endian.
    pub code: Vec<u8>,
    /// The file's codes, in first-use order: kind, field, and whether the node is in a value's place
    /// (tree-sitter-json's `_value` supertype: the document's child, an array element, a pair's value).
    pub codes: Vec<(u16, u16, bool)>,
    pub size: Vec<u8>,
    pub base: Vec<u8>,
    pub off: Vec<u8>,
    pub tail: Vec<u8>,
    /// `size`, `off` and `tail`'s widths in bytes (2 or 4); every tail is from the last descendant's start.
    pub widths: [u8; 3],
}

impl Tape {
    pub fn node_count(&self) -> usize {
        self.code.len() / 2
    }
}

/// A block of the tape: `base` holds one start per this many nodes (`tape::CHECKPOINT_STRIDE`).
pub const STRIDE: usize = 16;

/// Why a document has no tape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// simdjson refused it.
    Simdjson(Error),
    /// Its index is not one a valid document has (never, for a document simdjson accepts).
    Shape(&'static str),
    /// It is past the tape's offsets (4 GB).
    TooLarge,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Simdjson(e) => write!(f, "simdjson: {e}"),
            Refusal::Shape(why) => f.write_str(why),
            Refusal::TooLarge => f.write_str("past 4 GB"),
        }
    }
}

/// `src`'s tape (simdjson's DOM parser validates the whole document first).
///
/// What the index leaves to the writer: a string's closing quote and a scalar's end are the last byte
/// before the next index that is not whitespace; a string's escapes are found with `memchr`, and its
/// content is the runs between them. Each escape is two bytes, `\` and its letter (tree-sitter-json's
/// `escape_sequence`: a `\u`'s four digits are content). The document runs from its first token to the
/// end of the file, as tree-sitter's root does.
pub fn tape(src: &[u8], kinds: &Kinds) -> Result<Tape, Refusal> {
    if src.len() >= u32::MAX as usize {
        return Err(Refusal::TooLarge);
    }
    // simdjson skips a UTF-8 byte order mark, and its offsets start after it.
    let shift = if src.starts_with(b"\xef\xbb\xbf") { 3 } else { 0 };
    structurals(src, |index| write(src, index, shift, kinds)).map_err(Refusal::Simdjson)?
}

#[derive(Clone, Copy, PartialEq)]
enum Open {
    Object,
    Array,
    Pair,
}

/// The kinds a code names, by slot; a code is `slot * 3 + field` (0 none, 1 `key`, 2 `value`).
const DOCUMENT: usize = 0;
const PAIR: usize = 1;
const CONTENT: usize = 2;
const ESCAPE: usize = 3;
const OBJECT: usize = 4;
const ARRAY: usize = 5;
const STRING: usize = 6;
const NUMBER: usize = 7;
const TRUE: usize = 8;
const FALSE: usize = 9;
const NULL: usize = 10;
const SLOTS: usize = 11;
const NO_FIELD: usize = 0;
const KEY: usize = 1;
const VALUE: usize = 2;

/// The columns as they are written: wide cells, narrowed at the end.
struct Writer<'k> {
    kinds: &'k Kinds,
    /// A code's id in this file, by `slot * 3 + field`; `u16::MAX` until first used.
    ids: [u16; SLOTS * 3],
    tape: Tape,
    size: Vec<u32>,
    off: Vec<u32>,
    tail: Vec<u32>,
    base: u32,
    /// The start of the last node written: an ending node's last descendant.
    last_start: u32,
    max_off: u32,
    max_tail: u32,
}

impl Writer<'_> {
    /// Writes a node's start columns; its `ord`.
    #[inline(always)]
    fn open(&mut self, slot: usize, field: usize, start: u32) -> u32 {
        let ord = self.size.len() as u32;
        let key = slot * 3 + field;
        let mut id = self.ids[key];
        if id == u16::MAX {
            id = self.tape.codes.len() as u16;
            self.ids[key] = id;
            let k = self.kinds;
            let kind = [k.document, k.pair, k.string_content, k.escape_sequence, k.object, k.array, k.string, k.number, k.true_, k.false_, k.null][slot];
            let field_id = [0, k.key_field, k.value_field][field];
            // A value's place: the kinds a value can be, unless keyed.
            self.tape.codes.push((kind, field_id, slot >= OBJECT && field != KEY));
        }
        self.tape.code.extend_from_slice(&id.to_le_bytes());
        if ord as usize % STRIDE == 0 {
            self.base = start;
            self.tape.base.extend_from_slice(&start.to_le_bytes());
        }
        let off = start - self.base;
        self.max_off = self.max_off.max(off);
        self.off.push(off);
        self.size.push(0);
        self.tail.push(0);
        self.last_start = start;
        ord
    }

    /// Writes an ended node's end columns.
    #[inline(always)]
    fn close(&mut self, ord: u32, end: u32) {
        let tail = end - self.last_start;
        self.max_tail = self.max_tail.max(tail);
        self.size[ord as usize] = self.size.len() as u32 - ord;
        self.tail[ord as usize] = tail;
    }

    /// A node with no children.
    #[inline(always)]
    fn leaf(&mut self, slot: usize, field: usize, start: u32, end: u32) {
        let ord = self.open(slot, field, start);
        self.close(ord, end);
    }

    fn finish(mut self) -> Tape {
        let narrow = |cells: &[u32], max: u32, out: &mut Vec<u8>| -> u8 {
            if max <= 0xffff {
                out.reserve(cells.len() * 2);
                out.extend(cells.iter().flat_map(|&c| (c as u16).to_le_bytes()));
                2
            } else {
                out.reserve(cells.len() * 4);
                out.extend(cells.iter().flat_map(|&c| c.to_le_bytes()));
                4
            }
        };
        let count = self.size.len() as u32;
        let t = &mut self.tape;
        t.widths = [narrow(&self.size, count, &mut t.size), narrow(&self.off, self.max_off, &mut t.off), narrow(&self.tail, self.max_tail, &mut t.tail)];
        self.tape
    }
}

fn write(src: &[u8], index: &[u32], shift: u32, kinds: &Kinds) -> Result<Tape, Refusal> {
    let len = src.len() as u32;
    let end_before = |start: u32, next: u32| {
        let mut end = next;
        while end > start + 1 && matches!(src[end as usize - 1], b' ' | b'\t' | b'\n' | b'\r') {
            end -= 1;
        }
        end
    };
    let first = *index.first().ok_or(Refusal::Shape("an empty index"))? + shift;
    // A node an index entry, and half again for pairs, contents and escapes (the vectors grow past it).
    let guess = index.len() + index.len() / 2;
    let mut w = Writer {
        kinds,
        ids: [u16::MAX; SLOTS * 3],
        tape: Tape { code: Vec::with_capacity(guess * 2), base: Vec::with_capacity(guess / STRIDE * 4 + 4), ..Tape::default() },
        size: Vec::with_capacity(guess),
        off: Vec::with_capacity(guess),
        tail: Vec::with_capacity(guess),
        base: 0,
        last_start: 0,
        max_off: 0,
        max_tail: 0,
    };
    let document = w.open(DOCUMENT, NO_FIELD, first);
    let mut open: Vec<(Open, u32)> = Vec::with_capacity(64);
    // A value ended at `end`: the pair it is the value of ends there too.
    let close_pair = |open: &mut Vec<(Open, u32)>, w: &mut Writer, end: u32| {
        if let Some(&(Open::Pair, pair)) = open.last() {
            w.close(pair, end);
            open.pop();
        }
    };
    for (i, &at) in index.iter().enumerate() {
        let at = at + shift;
        let byte = src[at as usize];
        match byte {
            b',' | b':' => continue,
            b'}' | b']' => {
                let (_, ord) = open.pop().ok_or(Refusal::Shape("a close with nothing open"))?;
                w.close(ord, at + 1);
                close_pair(&mut open, &mut w, at + 1);
                continue;
            }
            _ => {}
        }
        let next = index.get(i + 1).map_or(len, |&n| n + shift);
        // A key (a string in an object), or a value.
        let field = match open.last() {
            None | Some(&(Open::Array, _)) => NO_FIELD,
            Some(&(Open::Pair, _)) => VALUE,
            Some(&(Open::Object, _)) => {
                let pair = w.open(PAIR, NO_FIELD, at);
                open.push((Open::Pair, pair));
                KEY
            }
        };
        match byte {
            b'{' => {
                let ord = w.open(OBJECT, field, at);
                open.push((Open::Object, ord));
            }
            b'[' => {
                let ord = w.open(ARRAY, field, at);
                open.push((Open::Array, ord));
            }
            b'"' => {
                let end = end_before(at, next);
                let string = w.open(STRING, field, at);
                let (mut from, close) = (at + 1, end - 1);
                while let Some(found) = memchr::memchr(b'\\', &src[from as usize..close as usize]) {
                    let escape = from + found as u32;
                    if escape > from {
                        w.leaf(CONTENT, NO_FIELD, from, escape);
                    }
                    w.leaf(ESCAPE, NO_FIELD, escape, escape + 2);
                    from = escape + 2;
                }
                if close > from {
                    w.leaf(CONTENT, NO_FIELD, from, close);
                }
                w.close(string, end);
                if field != KEY {
                    close_pair(&mut open, &mut w, end);
                }
            }
            _ => {
                let slot = match byte {
                    b't' => TRUE,
                    b'f' => FALSE,
                    b'n' => NULL,
                    _ => NUMBER,
                };
                let end = end_before(at, next);
                w.leaf(slot, field, at, end);
                close_pair(&mut open, &mut w, end);
            }
        }
    }
    if !open.is_empty() {
        return Err(Refusal::Shape("a node left open"));
    }
    w.close(document, len);
    Ok(w.finish())
}

/// The kernel simdjson chose for this CPU (`arm64`, `haswell`, …).
pub fn implementation() -> &'static str {
    // SAFETY: the name is a static string of simdjson's.
    unsafe { CStr::from_ptr(rerail_simdjson_implementation()) }.to_str().unwrap_or("?")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_index_is_each_structural_and_scalar_start() {
        let src = br#" {"a": [1, true, "x\"y"], "b": null} "#;
        let index = structurals(src, <[u32]>::to_vec).unwrap();
        let at: String = index.iter().map(|&i| src[i as usize] as char).collect();
        assert_eq!(at, r#"{":[1,t,"],":n}"#);
        assert_ne!(implementation(), "fallback");
    }

    /// The tape's cost alone, for `/usr/bin/time -l`: `RERAIL_TAPE_RUN` is `none` (read the files), `index`
    /// (parse and index) or `tape`, 10 rounds over the repository's `.json` files up to 1 MB.
    #[test]
    #[ignore = "a measurement: run by name with RERAIL_TAPE_RUN"]
    fn tape_cost() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let (mut files, mut work) = (Vec::new(), vec![root]);
        while let Some(dir) = work.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let p = e.path();
                let skip = p.file_name().is_some_and(|n| [".git", "target", "node_modules"].contains(&n.to_string_lossy().as_ref()));
                if p.is_dir() && !skip {
                    work.push(p);
                } else if p.extension().is_some_and(|x| x == "json") && e.metadata().is_ok_and(|m| m.len() <= 1 << 20) {
                    files.extend(std::fs::read(&p).ok());
                }
            }
        }
        files.retain(|f| structurals(f, |_| ()).is_ok());
        let kinds = Kinds { document: 1, object: 2, pair: 3, array: 4, string: 5, string_content: 6, escape_sequence: 7, number: 8, true_: 9, false_: 10, null: 11, key_field: 1, value_field: 2 };
        let run = std::env::var("RERAIL_TAPE_RUN").unwrap_or_default();
        let mut n = 0usize;
        for _ in 0..10 {
            for f in &files {
                n += match run.as_str() {
                    "index" => structurals(f, <[u32]>::len).unwrap(),
                    "tape" => tape(f, &kinds).unwrap().node_count(),
                    _ => f.len(),
                };
            }
        }
        eprintln!("{} files, {n}", files.len());
    }

    #[test]
    fn invalid_documents_are_refused() {
        for bad in [&b""[..], b"{", b"[1,]", b"// c\n1", b"\"\\x\"", b"01", b"[\"\xff\"]", b"123456789012345678901234567890"] {
            assert!(structurals(bad, |_| ()).is_err(), "{:?}", String::from_utf8_lossy(bad));
        }
        assert!(structurals(b"\xef\xbb\xbf{}", |_| ()).is_ok());
    }
}
