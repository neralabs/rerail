//! `intern` (decision 0686, B5; *0686: Operations* §10; B§7's second answerer, Intern: Pure, text ↔ symbol): a
//! changed document's identifiers and strings written into the atoms block, the one program global every level
//! shares, with its node → symbol facts. Its slots: the blocks of the document's source and of its tape (read, where
//! the file layer placed them; two blobs, which may lie in one block or two), the atoms block (read-write) and the
//! document's symbols block (read-write: its header the host's, its facts the operation's).
//!
//! The symbols block is the document's node → symbol relation, partitioned by the document (its node ids are its own,
//! L9) and laid out by node id: for each node, the symbol of its text when it is a leaf (an identifier, a variable, an
//! operator, a literal: what the `text` view reads) and the symbol of its value when it is a string (what `value`
//! reads), 0 where it has none. Its header: the body's length (a block's first word, what a freeze keeps), the nodes'
//! count, the document's path and module (the answer's, written by the host) and their symbols. Once written it is
//! frozen, and housed as a blob. The language's names (its kinds and fields) and K-form's step kinds are interned
//! too, so a read finds every name a view or a pass compares.
//!
//! A symbol is an atom's id: the same text has the same id across runs and documents. Nothing reads the atoms to
//! write them but this operation: a view reads the symbols block, and finds names (`find_in`), never interns.

use crate::tape::Reads;
use crate::{Ctx, Tile, TwinPhase};
use rerail_blocks::atoms::{append_in, find_in};
use rerail_blocks::gens::{BODY_AT, set_word, word};
use rerail_blocks::maps::hash_bytes;

crate::params! {
    /// Where the document's source and tape lie in their blocks, by offset and length.
    pub struct InternP {
        pub src_at: u64,
        pub src_len: u64,
        pub tape_at: u64,
        pub tape_len: u64,
    }
}

/// K-form's step kinds, which a pass writes as symbols.
pub const STEP_KINDS: [&str; 3] = ["atom", "not", "cmp"];

/// The slots.
const SOURCE: usize = 0;
const TAPE: usize = 1;
pub const ATOMS: usize = 2;
const SYMS: usize = 3;

const LEN_AT: usize = BODY_AT;
const N_AT: usize = BODY_AT + 8;
const PATH_SYM_AT: usize = BODY_AT + 16;
const MODULE_SYM_AT: usize = BODY_AT + 20;
const PATH_LEN_AT: usize = BODY_AT + 24;
const MODULE_LEN_AT: usize = BODY_AT + 28;
const NAMES_AT: usize = BODY_AT + 32;

fn nodes_at(path_len: usize, module_len: usize) -> usize {
    NAMES_AT + (path_len + module_len).next_multiple_of(8)
}

/// A symbols block's body bytes for `n` nodes and the answer's `path` and `module`.
pub fn syms_bytes(n: usize, path: &str, module: &str) -> usize {
    nodes_at(path.len(), module.len()) + 8 * n - BODY_AT
}

/// A symbols block's header, written by the host before `intern` runs: the nodes' count, the path and the module.
pub fn init_syms(bytes: &mut [u8], n: usize, path: &str, module: &str) {
    set_word::<u64>(bytes, LEN_AT, syms_bytes(n, path, module) as u64);
    set_word::<u64>(bytes, N_AT, n as u64);
    set_word::<u32>(bytes, PATH_LEN_AT, path.len() as u32);
    set_word::<u32>(bytes, MODULE_LEN_AT, module.len() as u32);
    bytes[NAMES_AT..NAMES_AT + path.len()].copy_from_slice(path.as_bytes()); // copy: derived (the answer's path, written once)
    bytes[NAMES_AT + path.len()..NAMES_AT + path.len() + module.len()].copy_from_slice(module.as_bytes()); // copy: derived (its module)
}

/// The nodes' count.
pub fn count(syms: &[u8]) -> usize {
    word::<u64>(syms, N_AT) as usize
}

/// The document's path and module, as the answer named them.
pub fn names(syms: &[u8]) -> (&str, &str) {
    let (p, m) = (word::<u32>(syms, PATH_LEN_AT) as usize, word::<u32>(syms, MODULE_LEN_AT) as usize);
    let s = |a: usize, b: usize| std::str::from_utf8(&syms[a..b]).expect("names written from a &str");
    (s(NAMES_AT, NAMES_AT + p), s(NAMES_AT + p, NAMES_AT + p + m))
}

/// The path's and the module's symbols.
pub fn doc_syms(syms: &[u8]) -> (u32, u32) {
    (word::<u32>(syms, PATH_SYM_AT), word::<u32>(syms, MODULE_SYM_AT))
}

fn node_at(syms: &[u8], n: usize) -> usize {
    nodes_at(word::<u32>(syms, PATH_LEN_AT) as usize, word::<u32>(syms, MODULE_LEN_AT) as usize) + 8 * n
}

/// Node `n`'s text symbol (a leaf's), 0 for none.
pub fn text(syms: &[u8], n: usize) -> u32 {
    word::<u32>(syms, node_at(syms, n))
}

/// Node `n`'s value symbol (a string's), 0 for none.
pub fn value(syms: &[u8], n: usize) -> u32 {
    word::<u32>(syms, node_at(syms, n) + 4)
}

/// `s`'s id in the atoms, made when it has none.
fn put(atoms: &mut [u8], s: &str) -> u32 {
    let h = hash_bytes(s.as_bytes());
    find_in(atoms, s, h).or_else(|| append_in(atoms, s, h)).expect("room in the atoms block")
}

/// `intern`'s one item: the document's names, then every node's, in node order (new ids by that order).
fn intern(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let p = <InternP as crate::Params>::read(c.params);
    let ([source, tape], [atoms, syms]) = c.views([SOURCE, TAPE], [ATOMS, SYMS]);
    let src = std::str::from_utf8(&source[p.src_at as usize..(p.src_at + p.src_len) as usize]).expect("a source the file layer checked is UTF-8");
    let r = Reads::in_place(&tape[p.tape_at as usize..(p.tape_at + p.tape_len) as usize], src).expect("a tape the parser wrote");
    let language = rerail_rr_syntax::tape::KIND_NAMES.iter().chain(rerail_rr_syntax::tape::FIELD_NAMES).chain(&STEP_KINDS);
    for n in language.filter(|n| !n.is_empty()) {
        put(atoms, n);
    }
    let (ps, ms) = {
        let (path, module) = names(syms);
        (put(atoms, path), put(atoms, module))
    };
    set_word::<u32>(syms, PATH_SYM_AT, ps);
    set_word::<u32>(syms, MODULE_SYM_AT, ms);
    assert_eq!(count(syms), r.len(), "a symbols block of the tape's nodes");
    for n in 0..r.len() {
        let t = if r.is_leaf(n) { put(atoms, r.text(n)) } else { 0 };
        let v = r.string(n).map_or(0, |s| put(atoms, &s));
        let at = node_at(syms, n);
        set_word::<u32>(syms, at, t);
        set_word::<u32>(syms, at + 4, v);
    }
}

/// `intern`: one item, the whole document (its new ids in node order).
pub const INTERN: &[TwinPhase] = &[TwinPhase { name: "intern", items: |_| 1, run: intern, again: None }];
