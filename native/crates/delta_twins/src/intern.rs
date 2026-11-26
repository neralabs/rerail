//! `intern` (decision 0686, B5; *0686: Operations* §10; B§7's second answerer, Intern: Pure, text ↔ symbol): a
//! changed document's identifiers and strings written into the atoms block, the one program global every level
//! shares, with its node → symbol facts. Its slots: the blocks of the document's source and of its tape (read, where
//! the file layer placed them; two blobs, which may lie in one block or two), the atoms block (read-write) and the
//! document's symbols block (read-write: its header the host's, its facts the operation's).
//!
//! The symbols block is the document's node → symbol relation, partitioned by the document (its node ids are its own,
//! L9) and laid out by node id: for each node, the symbol of its text when it is a name (an identifier or a variable:
//! what the `text` view reads; an operator's is the language's name for its token, found, never made) and the symbol
//! of its value when it is a string (what `value` reads), 0 where it has none. Only identifiers and strings make new
//! atoms: a number is its value, and punctuation is the language's. Its header: the body's length (a block's first word, what a freeze keeps), the nodes'
//! count, the document's path and module (the answer's, written by the host) and their symbols. Once written it is
//! frozen, and housed as a blob. The language's names (its kinds and fields) and K-form's step kinds are interned
//! once, by the first run that finds them missing (one find, of the last of them: they are appended in order), so a
//! read finds every name a view or a pass compares.
//!
//! A symbol is an atom's id: the same text has the same id across runs and documents. Nothing reads the atoms to
//! write them but this operation: a view reads the symbols block, and finds names (`find_in`), never interns.

use crate::tape::Reads;
use crate::{Ctx, Tile, TwinPhase};
use rerail_blocks::atoms::{append_in, find_in};
use rerail_blocks::gens::{BODY_AT, set_word, word};
use rerail_blocks::maps::hash_bytes;
use rerail_rr_syntax::tape::{FIRST_ANONYMOUS, KIND_NAMES, Kind};

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

/// A node's symbol the hash phase did not find: a name the scan makes, in node order.
const NEW: u32 = u32::MAX;
/// An operator's token the hash phase did not find: the scan finds it once the language's names are in (never made).
const FIND: u32 = u32::MAX - 1;

/// The document's source and tape, read where they lie.
fn reads<'b>(source: &'b [u8], tape: &'b [u8], p: &InternP) -> Reads<'b> {
    let src = std::str::from_utf8(&source[p.src_at as usize..(p.src_at + p.src_len) as usize]).expect("a source the file layer checked is UTF-8");
    Reads::in_place(&tape[p.tape_at as usize..(p.tape_at + p.tape_len) as usize], src).expect("a tape the parser wrote")
}

/// Node `n`'s two names to intern: its text's (an identifier's or a variable's, made when new; an operator's, found)
/// and its value's (a string's).
fn names_of<'r>(r: &'r Reads<'_>, n: usize) -> (Option<(&'r str, bool)>, Option<String>) {
    let k = r.node(n).0;
    let t = if k == Kind::Identifier as u16 || k == Kind::Variable as u16 {
        Some((r.text(n), true))
    } else if k >= FIRST_ANONYMOUS && r.is_leaf(n) {
        Some((r.text(n), false))
    } else {
        None
    };
    (t, r.string(n))
}

/// The hash phase (shape 18, a hash build: an item a node): each node's names probed in the atoms' hash table, the
/// found ones' ids written, the rest marked for the scan (`NEW`, `FIND`). It writes no atom.
fn hash(c: &mut Ctx<'_, '_, '_>, tile: Tile) {
    let p = <InternP as crate::Params>::read(c.params);
    let ([source, tape, atoms], [syms]) = c.views([SOURCE, TAPE, ATOMS], [SYMS]);
    let r = reads(source, tape, &p);
    assert_eq!(count(syms), r.len(), "a symbols block of the tape's nodes");
    let find = |s: &str| find_in(atoms, s, hash_bytes(s.as_bytes()));
    for n in tile.items() {
        let (t, v) = names_of(&r, n);
        let t = t.map_or(0, |(s, make)| find(s).unwrap_or(if make { NEW } else { FIND }));
        let v = v.map_or(0, |s| find(&s).unwrap_or(NEW));
        let at = node_at(syms, n);
        set_word::<u32>(syms, at, t);
        set_word::<u32>(syms, at + 4, v);
    }
}

/// The scan (shape 10, ordered, one item): the language's names on the first run that finds them missing, the
/// document's path and module, then every name the hash phase marked, in node order: new ids by that order.
fn scan(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let p = <InternP as crate::Params>::read(c.params);
    let ([source, tape], [atoms, syms]) = c.views([SOURCE, TAPE], [ATOMS, SYMS]);
    let r = reads(source, tape, &p);
    let language = || KIND_NAMES.iter().chain(rerail_rr_syntax::tape::FIELD_NAMES).chain(&STEP_KINDS).filter(|n| !n.is_empty());
    let last = language().next_back().expect("the language's names");
    if find_in(atoms, last, hash_bytes(last.as_bytes())).is_none() {
        for n in language() {
            put(atoms, n);
        }
    }
    let (ps, ms) = {
        let (path, module) = names(syms);
        (put(atoms, path), put(atoms, module))
    };
    set_word::<u32>(syms, PATH_SYM_AT, ps);
    set_word::<u32>(syms, MODULE_SYM_AT, ms);
    for n in 0..r.len() {
        let at = node_at(syms, n);
        let (t, v) = (word::<u32>(syms, at), word::<u32>(syms, at + 4));
        if t != NEW && t != FIND && v != NEW {
            continue;
        }
        let (tn, vn) = names_of(&r, n);
        if let Some((s, _)) = tn.filter(|_| t == NEW || t == FIND) {
            let id = if t == NEW { put(atoms, s) } else { find_in(atoms, s, hash_bytes(s.as_bytes())).unwrap_or(0) };
            set_word::<u32>(syms, at, id);
        }
        if let Some(s) = vn.filter(|_| v == NEW) {
            set_word::<u32>(syms, at + 4, put(atoms, &s));
        }
    }
}

/// `intern` (*0686: Operations* §10: a hash build, then a scan for new ids): the hash phase an item a node, then the
/// ordered scan, one item.
pub const INTERN: &[TwinPhase] = &[
    TwinPhase { name: "hash", items: |c| count(c.read(SYMS)) as u32, run: hash, again: None },
    TwinPhase { name: "scan", items: |_| 1, run: scan, again: None },
];
