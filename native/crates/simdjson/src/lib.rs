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

/// The kinds and fields a JSON tree is written with: tree-sitter-json's ids, which the caller looks up.
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

/// A node of the tree: its kind, its field under its parent (0 for none), its span, and its parent's row
/// (`u32::MAX` for the document).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Node {
    pub kind: u16,
    pub field: u16,
    pub start: u32,
    pub end: u32,
    pub parent: u32,
}

/// A JSON document's tree, tree-sitter-json's node for node: `document`, `object`, `pair` (its `key` and
/// `value`), `array`, `string` (its `string_content` and `escape_sequence`), `number`, `true`, `false`,
/// `null`; in preorder, without the tokens each node implies (braces, brackets, commas, colons, quotes).
#[derive(Clone, Debug, Default)]
pub struct Tree {
    pub nodes: Vec<Node>,
    /// The rows of the nodes in a value's place (tree-sitter-json's `_value` supertype): the document's
    /// child, each array element, each pair's value.
    pub values: Vec<u32>,
}

/// Why a document has no tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// simdjson refused it.
    Simdjson(Error),
    /// Its index is not one a valid document has (never, for a document simdjson accepts).
    Shape(&'static str),
    /// It is past the tree's offsets (4 GB).
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

/// `src`'s tree, read from simdjson's index (its DOM parser validates the whole document first).
///
/// What the index leaves to the tree: a string's closing quote and a scalar's end are the last byte before
/// the next index that is not whitespace; a string's escapes are found with `memchr`, and its content is
/// the runs between them. Each escape is two bytes, `\` and
/// its letter (tree-sitter-json's `escape_sequence`: a `\u`'s four digits are content). The document runs
/// from its first token to the end of the file, as tree-sitter's root does.
pub fn tree(src: &[u8], kinds: &Kinds) -> Result<Tree, Refusal> {
    if src.len() >= u32::MAX as usize {
        return Err(Refusal::TooLarge);
    }
    // simdjson skips a UTF-8 byte order mark, and its offsets start after it.
    let shift = if src.starts_with(b"\xef\xbb\xbf") { 3 } else { 0 };
    structurals(src, |index| build(src, index, shift, kinds)).map_err(Refusal::Simdjson)?
}

#[derive(Clone, Copy, PartialEq)]
enum Open {
    Object,
    Array,
    Pair,
}

fn build(src: &[u8], index: &[u32], shift: u32, k: &Kinds) -> Result<Tree, Refusal> {
    let len = src.len() as u32;
    let node = |kind, field, start, end, parent| Node { kind, field, start, end, parent };
    let end_before = |start: u32, next: u32| {
        let mut end = next;
        while end > start + 1 && matches!(src[end as usize - 1], b' ' | b'\t' | b'\n' | b'\r') {
            end -= 1;
        }
        end
    };
    let first = *index.first().ok_or(Refusal::Shape("an empty index"))? + shift;
    let mut tree = Tree { nodes: Vec::with_capacity(index.len() * 2), values: Vec::with_capacity(index.len()) };
    tree.nodes.push(node(k.document, 0, first, len, u32::MAX));
    let mut open: Vec<(Open, u32)> = Vec::with_capacity(64);
    // A value ended at `end`: the pair it is the value of ends there too.
    let close_pair = |open: &mut Vec<(Open, u32)>, nodes: &mut [Node], end: u32| {
        if let Some(&(Open::Pair, pair)) = open.last() {
            nodes[pair as usize].end = end;
            open.pop();
        }
    };
    for (i, &at) in index.iter().enumerate() {
        let at = at + shift;
        let byte = src[at as usize];
        match byte {
            b',' | b':' => continue,
            b'}' | b']' => {
                let (_, row) = open.pop().ok_or(Refusal::Shape("a close with nothing open"))?;
                tree.nodes[row as usize].end = at + 1;
                close_pair(&mut open, &mut tree.nodes, at + 1);
                continue;
            }
            _ => {}
        }
        let next = index.get(i + 1).map_or(len, |&n| n + shift);
        // A key (a string in an object), or a value.
        let (parent, field, value) = match open.last() {
            None => (0, 0, true),
            Some(&(Open::Array, row)) => (row, 0, true),
            Some(&(Open::Pair, row)) => (row, k.value_field, true),
            Some(&(Open::Object, row)) => {
                let pair = tree.nodes.len() as u32;
                tree.nodes.push(node(k.pair, 0, at, at, row));
                open.push((Open::Pair, pair));
                (pair, k.key_field, false)
            }
        };
        let me = tree.nodes.len() as u32;
        if value {
            tree.values.push(me);
        }
        match byte {
            b'{' => {
                tree.nodes.push(node(k.object, field, at, at, parent));
                open.push((Open::Object, me));
            }
            b'[' => {
                tree.nodes.push(node(k.array, field, at, at, parent));
                open.push((Open::Array, me));
            }
            b'"' => {
                let end = end_before(at, next);
                tree.nodes.push(node(k.string, field, at, end, parent));
                let (mut from, close) = (at + 1, end - 1);
                while let Some(found) = memchr::memchr(b'\\', &src[from as usize..close as usize]) {
                    let escape = from + found as u32;
                    if escape > from {
                        tree.nodes.push(node(k.string_content, 0, from, escape, me));
                    }
                    tree.nodes.push(node(k.escape_sequence, 0, escape, escape + 2, me));
                    from = escape + 2;
                }
                if close > from {
                    tree.nodes.push(node(k.string_content, 0, from, close, me));
                }
                if value {
                    close_pair(&mut open, &mut tree.nodes, end);
                }
            }
            _ => {
                let kind = match byte {
                    b't' => k.true_,
                    b'f' => k.false_,
                    b'n' => k.null,
                    _ => k.number,
                };
                let end = end_before(at, next);
                tree.nodes.push(node(kind, field, at, end, parent));
                close_pair(&mut open, &mut tree.nodes, end);
            }
        }
    }
    if !open.is_empty() {
        return Err(Refusal::Shape("a node left open"));
    }
    Ok(tree)
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

    /// The tree's cost alone, for `/usr/bin/time -l`: `RERAIL_TREE_RUN` is `none` (read the files), `index`
    /// (parse and index) or `tree`, 10 rounds over the repository's `.json` files up to 1 MB.
    #[test]
    #[ignore = "a measurement: run by name with RERAIL_TREE_RUN"]
    fn tree_cost() {
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
        let run = std::env::var("RERAIL_TREE_RUN").unwrap_or_default();
        let mut n = 0usize;
        for _ in 0..10 {
            for f in &files {
                n += match run.as_str() {
                    "index" => structurals(f, <[u32]>::len).unwrap(),
                    "tree" => tree(f, &kinds).unwrap().nodes.len(),
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
