//! A program as a flat image (decision 0173): a fixed set of regions, every reference an id, the bytes kept in the
//! store's database as a source, and read by viewing them in place.
//!
//! ```text
//! header   magic, format, region count, then per region: item width, item count, byte offset (16-byte aligned)
//! strings  bytes, end offsets, and a linear hash table of 64-byte buckets (text → atom id)
//! rels     relation number → its atom; atom → relation number + 1 (0: none)
//! nodes    kind (u8), size (u32), val (u32), in preorder; ints (i64) for the values past 31 bits
//! rules    each rule's first node, its variables' range, its hash
//! parts    declarations and facts (sorted by atom), inputs, outputs, each with its hash
//! indexes  rules by head relation, and the literals that read each relation: CSR, keyed in a delta
//! delta    the parent's rules removed, its declarations and facts gone (an overlay's image only)
//! ```
//!
//! ```text
//! // p(x, y) :- e(x, y), x < 3.        relations: p = 0, e = 1   variables: x = slot 0, y = slot 1
//! //   node  kind  size  val
//! //   0     rule  10    0
//! //   1     atom  3     rel 0     the head
//! //   2     var   1     slot 0
//! //   3     var   1     slot 1
//! //   4     atom  3     rel 1     the first literal
//! //   5     var   1     slot 0
//! //   6     var   1     slot 1
//! //   7     cmp   3     op <      left, then right
//! //   8     var   1     slot 0
//! //   9     int   1     3         small, so held in val
//! ```
//!
//! **Ids are global to a stack.** An image's atoms, relations and rules are numbered after its parent's
//! (`atom_base`, `rel_base`, `rule_base`): a delta holds only the strings, relations and rules it adds, and every id
//! means the same thing in every version of the stack. A text is held once in a stack, by the first image that needs
//! it. A fold keeps the atoms' and relations' ids (its tables are its stack's, laid end to end).
//!
//! The image is read through handles (`handles`: rules, atoms, literals, terms), versions (`version`: lookups by
//! id and by name, rules by head and by reader) and written through `build::Builder`, never by indexing regions:
//! the regions and their layout are this module's.
use super::front::{Lists, Name, Program, Value};
use super::program_facts::{self, Facts};
use std::collections::BTreeMap;

pub mod about;
pub mod analyses;
pub mod build;
pub mod csr;
mod encoder;
pub mod extents;
pub mod grouped;
pub mod handles;
pub mod strings;
mod tree;
pub mod version;

pub use about::{About, LatticeRef, NativeKind, NativeRef};
pub use analyses::{Edges, Strata};
pub use build::{Builder, Rel, RuleBuilder, T};
pub use extents::Extents;
pub use grouped::Grouped;
pub use handles::{AggRef, AtomRef, CmpOp, Lit, LitRef, RuleRef, TermRef};
pub use strings::Strings;
pub use tree::Tree;
pub use version::{DeclRef, FactsRef, Made, Version};

pub const MAGIC: [u8; 8] = *b"RRPIMG\x00\x03";
/// 3: every relation the program names has an id, those its declarations, facts, inputs, outputs and about rows
/// name included (`about::RELATION_COLUMNS`), so a format-2 image is read again from its text.
pub const FORMAT: u32 = 3;
/// The program the `.about` rows are written as (`program_facts`), as the text has them.
const ABOUT: &str = "a";

// ---- ids ----

macro_rules! id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
        #[repr(transparent)]
        pub struct $name(pub u32);
        impl $name {
            pub fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}
id!(
    /// A string: a name, a constant, a type. Global to a stack.
    AtomId
);
id!(
    /// A relation's number. Global to a stack.
    RelId
);
id!(
    /// A rule's number. Global to a stack: an overlay's added rules follow its parent's.
    RuleId
);
id!(
    /// A variable of one rule: its first appearance's order in the rule.
    Slot
);

/// The regions, in their order in the image. Named here for the image's own code and for tests that damage one;
/// everything else reads an image by its handles and versions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
pub enum R {
    Meta,
    StrBytes,
    StrEnds,
    StrBuckets,
    StrOver,
    Rels,
    AtomRel,
    Kind,
    Size,
    Val,
    Ints,
    RuleNode,
    RuleVars,
    RuleHash,
    Vars,
    DeclName,
    DeclCols,
    Cols,
    DeclHash,
    FactRel,
    FactRows,
    RowCells,
    CellKind,
    CellVal,
    FactHash,
    Inputs,
    Outputs,
    HeadStart,
    HeadRules,
    ReadStart,
    ReadRule,
    ReadNode,
    Removed,
    DeclGone,
    FactGone,
    /// A delta's index keys: the relations its `HeadStart` and `ReadStart` are over (empty in a base: dense).
    HeadKeys,
    ReadKeys,
    /// A delta's atoms of its parent's that it makes relations: (atom << 32 | relation), sorted.
    Promoted,
}
pub(crate) const REGIONS: usize = 38;
/// Each region's item width in bytes. Buckets are 64-byte records.
const WIDTH: [u32; REGIONS] = [8, 1, 4, 64, 64, 4, 4, 1, 4, 4, 8, 4, 4, 8, 4, 4, 4, 4, 8, 4, 4, 4, 1, 8, 8, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 8];
const HEADER: usize = 16 + 24 * REGIONS;

/// `Meta`'s words.
pub(crate) mod meta {
    /// 0: a base; 1: an overlay's delta.
    pub const KIND: usize = 0;
    /// Where this image's rules, atoms and relations are numbered from: its parent's id spaces.
    pub const RULE_BASE: usize = 1;
    pub const ATOM_BASE: usize = 2;
    pub const REL_BASE: usize = 3;
    pub const HASH_LEVEL: usize = 4;
    pub const HASH_SPLIT: usize = 5;
    pub const HASH_COUNT: usize = 6;
    /// Bit 0: the inputs are this image's; bit 1, the outputs; bit 2, the `.about` rows. A base has all three.
    pub const OWNS: usize = 7;
    pub const INPUTS_HASH: usize = 8;
    pub const OUTPUTS_HASH: usize = 9;
    /// The sum of this image's own hashes: its rules', declarations', fact relations', and its inputs' and outputs'
    /// where it owns them. A base's digest; an overlay's part of its own.
    pub const OWN_SUM: usize = 10;
    pub const WORDS: usize = 11;
}

/// Node kinds (the low five bits of `Kind`; the high three are the kind's flags).
pub(crate) mod kind {
    pub const RULE: u8 = 1;
    pub const ATOM: u8 = 2;
    pub const CMP: u8 = 3;
    pub const AGG: u8 = 4;
    pub const NEAREST: u8 = 5;
    pub const AST: u8 = 6;
    pub const VAR: u8 = 7;
    pub const STR: u8 = 8;
    pub const INT: u8 = 9;
    pub const WILD: u8 = 10;
    pub const TOP: u8 = 11;
    pub const CALL: u8 = 12;
    pub const MASK: u8 = 0x1f;
    /// An atom's flags.
    pub const NEG: u8 = 0x20;
    pub const GATED: u8 = 0x40;
    /// An int's flag: `val` indexes `Ints`.
    pub const BIG: u8 = 0x80;
    /// Whether `val` of a node of kind `k` is an atom id (a string, a call's function, a kindset, a matcher).
    pub fn val_is_atom(k: u8) -> bool {
        matches!(k & MASK, STR | CALL | NEAREST | AST)
    }
}
/// A fact relation's flag in `FactRel`: one of the `.about` rows.
pub(crate) const ABOUT_BIT: u32 = 1 << 31;

pub fn hash<T: std::hash::Hash + ?Sized>(x: &T) -> u64 {
    use std::hash::BuildHasher;
    foldhash::quality::FixedState::with_seed(0x0173).hash_one(x)
}

pub(crate) fn bad(what: impl std::fmt::Display) -> String {
    format!("program image: {what}")
}

// ---- the image: bytes, 16-byte aligned, and its region table ----

/// An image's bytes, held at 16-byte alignment, viewed by region.
pub struct Image {
    buf: Vec<u128>,
    len: usize,
    regions: [(usize, usize); REGIONS],
}

impl Image {
    /// The image of `bytes`, copied to an aligned buffer and checked: its header, every region's width and bounds,
    /// every reference, and every rule's node tree. A checked image's handles read it without further checks.
    pub fn open(bytes: &[u8]) -> Result<Image, String> {
        let mut buf = vec![0u128; bytes.len().div_ceil(16)];
        // SAFETY: `buf` holds at least `bytes.len()` bytes.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf.as_mut_ptr().cast::<u8>(), bytes.len()) };
        let mut image = Image { buf, len: bytes.len(), regions: [(0, 0); REGIONS] };
        let b = image.bytes();
        if b.len() < HEADER || b[..8] != MAGIC {
            return Err(bad("no image"));
        }
        let word = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().expect("four bytes"));
        let long = |at: usize| u64::from_le_bytes(b[at..at + 8].try_into().expect("eight bytes"));
        if word(8) != FORMAT {
            return Err(bad(format!("format {}, not {FORMAT}", word(8))));
        }
        if word(12) as usize != REGIONS {
            return Err(bad(format!("{} regions, not {REGIONS}", word(12))));
        }
        let mut regions = [(0, 0); REGIONS];
        for (r, region) in regions.iter_mut().enumerate() {
            let at = 16 + 24 * r;
            let (width, count, offset) = (word(at), long(at + 8) as usize, long(at + 16) as usize);
            if width != WIDTH[r] {
                return Err(bad(format!("region {r} is {width} bytes wide, not {}", WIDTH[r])));
            }
            let end = count.checked_mul(width as usize).and_then(|n| n.checked_add(offset));
            if offset % 16 != 0 || offset < HEADER || end.is_none_or(|e| e > b.len()) {
                return Err(bad(format!("region {r} out of bounds")));
            }
            *region = (offset, count);
        }
        image.regions = regions;
        if image.u64s(R::Meta).len() != meta::WORDS {
            return Err(bad("no meta"));
        }
        image.check()?;
        Ok(image)
    }

    pub fn bytes(&self) -> &[u8] {
        // SAFETY: `buf` holds `len` initialized bytes.
        unsafe { std::slice::from_raw_parts(self.buf.as_ptr().cast::<u8>(), self.len) }
    }

    fn slice<T>(&self, r: R) -> &[T] {
        let (offset, count) = self.regions[r as usize];
        // SAFETY: `open` checked the region's bounds and width, and offsets are 16-byte aligned in a 16-byte-aligned
        // buffer; every T here is a plain integer (or an array of them) with no invalid bit patterns.
        unsafe { std::slice::from_raw_parts(self.bytes().as_ptr().add(offset).cast::<T>(), count) }
    }
    pub(crate) fn u8s(&self, r: R) -> &[u8] {
        self.slice(r)
    }
    pub(crate) fn u32s(&self, r: R) -> &[u32] {
        self.slice(r)
    }
    pub(crate) fn u64s(&self, r: R) -> &[u64] {
        self.slice(r)
    }
    fn buckets(&self, r: R) -> &[[u64; 8]] {
        self.slice(r)
    }
    /// Region `r`'s byte offset and item count (a test's way to damage one).
    pub fn region(&self, r: R) -> (usize, usize) {
        self.regions[r as usize]
    }
    fn meta(&self, w: usize) -> u64 {
        self.u64s(R::Meta)[w]
    }
    pub fn is_delta(&self) -> bool {
        self.meta(meta::KIND) == 1
    }
    pub fn rule_base(&self) -> u32 {
        self.meta(meta::RULE_BASE) as u32
    }
    pub fn atom_base(&self) -> u32 {
        self.meta(meta::ATOM_BASE) as u32
    }
    pub fn rel_base(&self) -> u32 {
        self.meta(meta::REL_BASE) as u32
    }
    /// The rules this image holds.
    pub fn rules(&self) -> usize {
        self.u32s(R::RuleNode).len()
    }
    /// The atoms this image holds (its own; its parent's are below `atom_base`).
    pub fn atoms(&self) -> usize {
        self.u32s(R::StrEnds).len()
    }
    pub fn rels(&self) -> usize {
        self.u32s(R::Rels).len()
    }
    /// One past the last atom id, relation and rule of this image's id spaces.
    pub fn atom_end(&self) -> u32 {
        self.atom_base() + self.atoms() as u32
    }
    pub fn rel_end(&self) -> u32 {
        self.rel_base() + self.rels() as u32
    }
    pub fn rule_end(&self) -> u32 {
        self.rule_base() + self.rules() as u32
    }
    pub(crate) fn own_sum(&self) -> u64 {
        self.meta(meta::OWN_SUM)
    }
    pub(crate) fn owns(&self) -> u64 {
        self.meta(meta::OWNS)
    }
    pub(crate) fn inputs_hash(&self) -> u64 {
        self.meta(meta::INPUTS_HASH)
    }
    pub(crate) fn outputs_hash(&self) -> u64 {
        self.meta(meta::OUTPUTS_HASH)
    }

    // -- strings and relations, by global id; each answers for its own ids only --

    fn atom_bytes(&self, local: usize) -> &[u8] {
        let ends = self.u32s(R::StrEnds);
        let start = if local == 0 { 0 } else { ends[local - 1] as usize };
        &self.u8s(R::StrBytes)[start..ends[local] as usize]
    }
    /// The text of atom `a`, one of this image's own.
    pub(crate) fn text(&self, a: AtomId) -> &str {
        // SAFETY: `check` found every string UTF-8.
        unsafe { std::str::from_utf8_unchecked(self.atom_bytes((a.0 - self.atom_base()) as usize)) }
    }
    /// The atom of `text`, if this image holds it.
    pub(crate) fn find(&self, text: &str) -> Option<AtomId> {
        let h = strings::hash(text.as_bytes());
        strings::find_in(self.buckets(R::StrBuckets), self.buckets(R::StrOver), self.meta(meta::HASH_LEVEL) as u32, self.meta(meta::HASH_SPLIT) as u32, h, |a| {
            self.atom_bytes(a as usize) == text.as_bytes()
        })
        .map(|a| AtomId(a + self.atom_base()))
    }
    /// What this image says of atom `a`'s relation: `Some(Some(r))` or `Some(None)` when it knows (it holds the atom,
    /// or makes it a relation); `None` when the answer is its parent's.
    pub(crate) fn rel_of(&self, a: AtomId) -> Option<Option<RelId>> {
        if a.0 >= self.atom_base() {
            return Some(self.u32s(R::AtomRel)[(a.0 - self.atom_base()) as usize].checked_sub(1).map(RelId));
        }
        let p = self.u64s(R::Promoted);
        p.binary_search_by_key(&a.0, |&x| (x >> 32) as u32).ok().map(|k| Some(RelId(p[k] as u32)))
    }
    /// The atom naming relation `r`, one of this image's own.
    pub(crate) fn rel_atom(&self, r: RelId) -> AtomId {
        AtomId(self.u32s(R::Rels)[(r.0 - self.rel_base()) as usize])
    }

    // -- rules --

    pub(crate) fn rule_node(&self, k: u32) -> u32 {
        self.u32s(R::RuleNode)[k as usize]
    }
    pub(crate) fn rule_vars(&self, k: u32) -> &[u32] {
        let v = self.u32s(R::RuleVars);
        &self.u32s(R::Vars)[v[k as usize] as usize..v[k as usize + 1] as usize]
    }
    pub(crate) fn rule_hash(&self, k: u32) -> u64 {
        self.u64s(R::RuleHash)[k as usize]
    }
    pub(crate) fn node(&self, at: u32) -> (u8, u32, u32) {
        let at = at as usize;
        (self.u8s(R::Kind)[at], self.u32s(R::Size)[at], self.u32s(R::Val)[at])
    }
    pub(crate) fn big(&self, k: u32) -> i64 {
        self.u64s(R::Ints)[k as usize] as i64
    }
    /// The rules (local indexes) whose head is relation `rel`.
    pub(crate) fn heads(&self, rel: RelId) -> &[u32] {
        let r = csr::Csr::new(self.u32s(R::HeadKeys), self.u32s(R::HeadStart)).range(rel.0);
        &self.u32s(R::HeadRules)[r]
    }
    /// The (local rule, literal node) pairs that read relation `rel` in their bodies.
    pub(crate) fn reads(&self, rel: RelId) -> impl Iterator<Item = (u32, u32)> + '_ {
        let r = csr::Csr::new(self.u32s(R::ReadKeys), self.u32s(R::ReadStart)).range(rel.0);
        self.u32s(R::ReadRule)[r.clone()].iter().copied().zip(self.u32s(R::ReadNode)[r].iter().copied())
    }
    /// The parent's rules this delta removes (global ids, sorted).
    pub(crate) fn removed(&self) -> &[u32] {
        self.u32s(R::Removed)
    }

    // -- parts --

    /// This image's declaration of the relation named `a`, by its index here.
    pub(crate) fn decl(&self, a: AtomId) -> Option<usize> {
        self.u32s(R::DeclName).binary_search(&a.0).ok()
    }
    pub(crate) fn decl_gone(&self, a: AtomId) -> bool {
        self.u32s(R::DeclGone).binary_search(&a.0).is_ok()
    }
    pub(crate) fn decl_count(&self) -> usize {
        self.u32s(R::DeclName).len()
    }
    pub(crate) fn decl_name(&self, k: usize) -> AtomId {
        AtomId(self.u32s(R::DeclName)[k])
    }
    pub(crate) fn decl_cols(&self, k: usize) -> impl ExactSizeIterator<Item = (AtomId, AtomId)> + '_ {
        let e = self.u32s(R::DeclCols);
        let c = self.u32s(R::Cols);
        (e[k]..e[k + 1]).map(move |i| (AtomId(c[2 * i as usize]), AtomId(c[2 * i as usize + 1])))
    }
    pub(crate) fn decl_hash(&self, k: usize) -> u64 {
        self.u64s(R::DeclHash)[k]
    }
    pub(crate) fn fact(&self, a: AtomId, about: bool) -> Option<usize> {
        self.u32s(R::FactRel).binary_search(&(a.0 | if about { ABOUT_BIT } else { 0 })).ok()
    }
    pub(crate) fn fact_gone(&self, a: AtomId, about: bool) -> bool {
        self.u32s(R::FactGone).binary_search(&(a.0 | if about { ABOUT_BIT } else { 0 })).is_ok()
    }
    pub(crate) fn fact_count(&self) -> usize {
        self.u32s(R::FactRel).len()
    }
    /// Fact relation `k`'s name and whether it is one of the `.about` rows.
    pub(crate) fn fact_key(&self, k: usize) -> (AtomId, bool) {
        let a = self.u32s(R::FactRel)[k];
        (AtomId(a & !ABOUT_BIT), a & ABOUT_BIT != 0)
    }
    pub(crate) fn fact_hash(&self, k: usize) -> u64 {
        self.u64s(R::FactHash)[k]
    }
    /// Fact relation `k`'s rows, each its cells: an int, or a string's atom.
    pub(crate) fn fact_rows(&self, k: usize) -> impl ExactSizeIterator<Item = impl ExactSizeIterator<Item = Cell> + '_> + '_ {
        let (rows, cells, kinds, vals) = (self.u32s(R::FactRows), self.u32s(R::RowCells), self.u8s(R::CellKind), self.u64s(R::CellVal));
        (rows[k]..rows[k + 1]).map(move |row| {
            (cells[row as usize]..cells[row as usize + 1]).map(move |c| if kinds[c as usize] == 0 { Cell::Int(vals[c as usize] as i64) } else { Cell::Str(AtomId(vals[c as usize] as u32)) })
        })
    }
    pub(crate) fn inputs(&self) -> &[u32] {
        self.u32s(R::Inputs)
    }
    pub(crate) fn outputs(&self) -> &[u32] {
        self.u32s(R::Outputs)
    }

    // -- the check --

    /// The image's references, each inside what it names: offsets ascending and in bounds, every atom, relation and
    /// slot an id this image or its parent holds, tables sorted where they are searched, and every rule's nodes a
    /// tree of the grammar (`check_rule`). A delta's references below its bases are its parent's, checked when it is
    /// stacked on one (`Version::over`).
    fn check(&self) -> Result<(), String> {
        let atoms = self.atoms();
        let mut last = 0;
        for &e in self.u32s(R::StrEnds) {
            if e < last || e as usize > self.u8s(R::StrBytes).len() {
                return Err(bad("string ends"));
            }
            last = e;
        }
        for k in 0..atoms {
            if std::str::from_utf8(self.atom_bytes(k)).is_err() {
                return Err(bad("a string is not UTF-8"));
            }
        }
        if !self.is_delta() && (self.atom_base() != 0 || self.rel_base() != 0 || self.rule_base() != 0) {
            return Err(bad("a base numbers from 0"));
        }
        let atom_end = self.atom_end();
        let rel_end = self.rel_end();
        let in_atoms = |r: R| self.u32s(r).iter().all(|&a| a < atom_end);
        for r in [R::Rels, R::Vars, R::Cols, R::DeclName, R::Inputs, R::Outputs, R::DeclGone] {
            if !in_atoms(r) {
                return Err(bad(format!("{r:?} names no string")));
            }
        }
        for r in [R::FactRel, R::FactGone] {
            if !self.u32s(r).iter().all(|&a| (a & !ABOUT_BIT) < atom_end) {
                return Err(bad(format!("{r:?} names no string")));
            }
        }
        let sorted = |v: &[u32]| v.windows(2).all(|w| w[0] < w[1]);
        for r in [R::DeclName, R::DeclGone, R::FactRel, R::FactGone, R::Removed, R::HeadKeys, R::ReadKeys] {
            if !sorted(self.u32s(r)) {
                return Err(bad(format!("{r:?} is not sorted")));
            }
        }
        if self.u32s(R::AtomRel).len() != atoms || !self.u32s(R::AtomRel).iter().all(|&r| r <= rel_end) {
            return Err(bad("atom → relation"));
        }
        let promoted = self.u64s(R::Promoted);
        if !promoted.windows(2).all(|w| w[0] >> 32 < w[1] >> 32) || !promoted.iter().all(|&p| ((p >> 32) as u32) < self.atom_base() && (p as u32) >= self.rel_base() && (p as u32) < rel_end) {
            return Err(bad("promoted atoms"));
        }
        let offsets = |r: R, n: usize, of: usize| {
            let v = self.u32s(r);
            v.len() == n + 1 && v[0] == 0 && v.windows(2).all(|w| w[0] <= w[1]) && *v.last().unwrap() as usize == of
        };
        let rules = self.rules();
        let nodes = self.u8s(R::Kind).len();
        if self.u32s(R::Size).len() != nodes || self.u32s(R::Val).len() != nodes || self.u64s(R::RuleHash).len() != rules {
            return Err(bad("columns differ in length"));
        }
        if !offsets(R::RuleVars, rules, self.u32s(R::Vars).len()) || !self.u32s(R::RuleNode).iter().all(|&n| (n as usize) < nodes) {
            return Err(bad("rules"));
        }
        let decls = self.decl_count();
        if !offsets(R::DeclCols, decls, self.u32s(R::Cols).len() / 2) || self.u32s(R::Cols).len() % 2 != 0 || self.u64s(R::DeclHash).len() != decls {
            return Err(bad("declarations"));
        }
        let facts = self.fact_count();
        let rows = self.u32s(R::RowCells).len().saturating_sub(1);
        let cells = self.u8s(R::CellKind).len();
        if !offsets(R::FactRows, facts, rows) || !offsets(R::RowCells, rows, cells) || self.u64s(R::CellVal).len() != cells || self.u64s(R::FactHash).len() != facts {
            return Err(bad("facts"));
        }
        for (k, &v) in self.u8s(R::CellKind).iter().zip(self.u64s(R::CellVal)) {
            if *k > 1 || (*k == 1 && v >= atom_end as u64) {
                return Err(bad("a fact's cell"));
            }
        }
        // The indexes: dense over this image's relations in a base, keyed in a delta.
        for (keys, start, values, what) in [(R::HeadKeys, R::HeadStart, R::HeadRules, "rules by head"), (R::ReadKeys, R::ReadStart, R::ReadRule, "reads")] {
            let k = self.u32s(keys);
            let n = if self.is_delta() { k.len() } else { self.rels() };
            if (!self.is_delta() && !k.is_empty()) || !k.iter().all(|&r| r < rel_end) || !offsets(start, n, self.u32s(values).len()) || !self.u32s(values).iter().all(|&r| (r as usize) < rules) {
                return Err(bad(what));
            }
        }
        if self.u32s(R::ReadNode).len() != self.u32s(R::ReadRule).len() || !self.u32s(R::ReadNode).iter().all(|&n| (n as usize) < nodes) {
            return Err(bad("reads"));
        }
        let removed = self.removed();
        if (!self.is_delta() && !removed.is_empty()) || removed.last().is_some_and(|&r| r >= self.rule_base()) {
            return Err(bad("removed rules"));
        }
        let level = self.meta(meta::HASH_LEVEL);
        let buckets = self.buckets(R::StrBuckets).len() as u64;
        if level > 31 || buckets != (1u64 << level) + self.meta(meta::HASH_SPLIT) || self.meta(meta::HASH_COUNT) != atoms as u64 {
            return Err(bad("string table"));
        }
        let mut next = 0u32;
        for k in 0..rules as u32 {
            let at = self.rule_node(k);
            if at != next {
                return Err(bad(format!("rule {k} does not follow the rule before it")));
            }
            next = self.check_rule(k)?;
        }
        if next as usize != nodes {
            return Err(bad("nodes past the last rule"));
        }
        Ok(())
    }

    /// Rule `k`'s nodes, as the grammar has them; the node after them.
    ///
    /// ```text
    /// rule  := RULE atom lit*
    /// lit   := atom | CMP term term | AGG (VAR | WILD) [VAR* STR, a join] atom | NEAREST VAR VAR | AST VAR VAR (STR VAR)*
    /// atom  := ATOM term*
    /// term  := VAR | WILD | TOP | INT | STR | CALL term*
    /// ```
    fn check_rule(&self, k: u32) -> Result<u32, String> {
        let c = Check { im: self, vars: self.rule_vars(k).len() as u32, nodes: self.u8s(R::Kind).len() as u32, atom_end: self.atom_end(), rel_end: self.rel_end() };
        let at = self.rule_node(k);
        let (kd, size, _) = c.node(at, c.nodes)?;
        if kd != kind::RULE {
            return Err(bad(format!("rule {k} starts at no rule node")));
        }
        let end = at + size;
        let mut p = c.atom(at + 1, end)?;
        while p < end {
            p = c.literal(p, end)?;
        }
        Ok(end)
    }
}

/// A fact's cell, by id.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cell {
    Int(i64),
    Str(AtomId),
}

struct Check<'i> {
    im: &'i Image,
    vars: u32,
    nodes: u32,
    atom_end: u32,
    rel_end: u32,
}

impl Check<'_> {
    /// Node `at`, inside `end`: its kind (flags masked), size and value.
    fn node(&self, at: u32, end: u32) -> Result<(u8, u32, u32), String> {
        if at >= end || at >= self.nodes {
            return Err(bad(format!("node {at} past its parent")));
        }
        let (k, size, val) = self.im.node(at);
        if size == 0 || at.checked_add(size).is_none_or(|e| e > end) {
            return Err(bad(format!("node {at} overruns its parent")));
        }
        Ok((k & kind::MASK, size, val))
    }
    fn leaf(&self, at: u32, end: u32, want: u8) -> Result<u32, String> {
        let (k, size, val) = self.node(at, end)?;
        if k != want || size != 1 {
            return Err(bad(format!("node {at}: kind {k}, not {want}")));
        }
        if (k == kind::VAR && val >= self.vars) || (k == kind::STR && val >= self.atom_end) {
            return Err(bad(format!("node {at}'s value {val}")));
        }
        Ok(at + 1)
    }
    fn term(&self, at: u32, end: u32) -> Result<u32, String> {
        let (k, size, val) = self.node(at, end)?;
        let flags = self.im.node(at).0 & !kind::MASK;
        match k {
            kind::VAR | kind::STR => self.leaf(at, end, k),
            kind::WILD | kind::TOP if size == 1 => Ok(at + 1),
            kind::INT if size == 1 => {
                if flags & kind::BIG != 0 && val as usize >= self.im.u64s(R::Ints).len() {
                    return Err(bad(format!("node {at}: an int past the table")));
                }
                Ok(at + 1)
            }
            kind::CALL if val < self.atom_end => {
                let e = at + size;
                let mut p = at + 1;
                while p < e {
                    p = self.term(p, e)?;
                }
                Ok(e)
            }
            _ => Err(bad(format!("node {at}: no term (kind {k})"))),
        }
    }
    fn atom(&self, at: u32, end: u32) -> Result<u32, String> {
        let (k, size, val) = self.node(at, end)?;
        if k != kind::ATOM || val >= self.rel_end {
            return Err(bad(format!("node {at}: no atom")));
        }
        let e = at + size;
        let mut p = at + 1;
        while p < e {
            p = self.term(p, e)?;
        }
        Ok(e)
    }
    fn literal(&self, at: u32, end: u32) -> Result<u32, String> {
        let (k, size, val) = self.node(at, end)?;
        let e = at + size;
        let done = |p: u32| if p == e { Ok(e) } else { Err(bad(format!("node {at}: more than its literal"))) };
        match k {
            kind::ATOM => self.atom(at, end),
            kind::CMP if (val as usize) < handles::CmpOp::ALL.len() => {
                let p = self.term(at + 1, e)?;
                done(self.term(p, e)?)
            }
            kind::AGG if self.im.node(at).0 >> 5 <= 5 && val < self.vars => {
                let mut p = at + 1;
                p = match self.node(p, e)?.0 {
                    kind::WILD => self.leaf(p, e, kind::WILD)?,
                    _ => self.leaf(p, e, kind::VAR)?,
                };
                if self.im.node(at).0 >> 5 == 5 {
                    while self.node(p, e)?.0 == kind::VAR {
                        p = self.leaf(p, e, kind::VAR)?;
                    }
                    p = self.leaf(p, e, kind::STR)?;
                }
                done(self.atom(p, e)?)
            }
            kind::NEAREST if val < self.atom_end => {
                let p = self.leaf(at + 1, e, kind::VAR)?;
                done(self.leaf(p, e, kind::VAR)?)
            }
            kind::AST if val < self.atom_end => {
                let mut p = self.leaf(at + 1, e, kind::VAR)?;
                p = self.leaf(p, e, kind::VAR)?;
                while p < e {
                    p = self.leaf(p, e, kind::STR)?;
                    p = self.leaf(p, e, kind::VAR)?;
                }
                Ok(e)
            }
            _ => Err(bad(format!("node {at}: no literal (kind {k})"))),
        }
    }
}

// ---- writing an image's regions ----

/// The regions of an image being written, each appended to, then laid out once.
pub(crate) struct Writer {
    regions: Vec<Vec<u8>>,
}

impl Default for Writer {
    fn default() -> Self {
        Writer { regions: vec![Vec::new(); REGIONS] }
    }
}

impl Writer {
    pub fn u8(&mut self, r: R, v: u8) {
        self.regions[r as usize].push(v);
    }
    pub fn u32(&mut self, r: R, v: u32) {
        self.regions[r as usize].extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, r: R, v: u64) {
        self.regions[r as usize].extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32s(&mut self, r: R, v: &[u32]) {
        let out = &mut self.regions[r as usize];
        out.reserve(v.len() * 4);
        for x in v {
            out.extend_from_slice(&x.to_le_bytes());
        }
    }
    pub fn u64s(&mut self, r: R, v: &[u64]) {
        let out = &mut self.regions[r as usize];
        out.reserve(v.len() * 8);
        for x in v {
            out.extend_from_slice(&x.to_le_bytes());
        }
    }
    pub fn raw(&mut self, r: R, bytes: &[u8]) {
        self.regions[r as usize].extend_from_slice(bytes);
    }
    /// The image: the header, then each region at a 16-byte offset.
    pub fn finish(self) -> Vec<u8> {
        let mut out = vec![0u8; HEADER];
        out[..8].copy_from_slice(&MAGIC);
        out[8..12].copy_from_slice(&FORMAT.to_le_bytes());
        out[12..16].copy_from_slice(&(REGIONS as u32).to_le_bytes());
        for (r, bytes) in self.regions.iter().enumerate() {
            while out.len() % 16 != 0 {
                out.push(0);
            }
            let at = 16 + 24 * r;
            let offset = out.len() as u64;
            let count = (bytes.len() / WIDTH[r] as usize) as u64;
            out[at..at + 4].copy_from_slice(&WIDTH[r].to_le_bytes());
            out[at + 8..at + 16].copy_from_slice(&count.to_le_bytes());
            out[at + 16..at + 24].copy_from_slice(&offset.to_le_bytes());
            out.extend_from_slice(bytes);
        }
        out
    }
}

// ---- a program to and from its image ----

/// The image of `prog`: a base, every part its own. Refuses what the image has no form for (maintenance's parts,
/// which the text refuses too) and a comparison it does not know.
pub fn encode(prog: &Program) -> Result<Vec<u8>, String> {
    Builder::of_program(prog)?.finish_bytes()
}

/// The program image `bytes` holds, its lists taken from the run's `lists`.
pub fn decode(bytes: &[u8], lists: &Lists) -> Result<Program, String> {
    Version::open(bytes)?.program(lists)
}

/// The program of a version's parts and rules, assembled as the text's reader assembles it: the `.about` rows give
/// the program's other fields; its facts are the facts' own (the rows' `fact_rel` restates their names).
pub(crate) fn assemble(rules: Vec<super::front::Rule>, decls: BTreeMap<Name, super::front::List<(Name, Name)>>, facts: BTreeMap<String, Vec<super::front::List<Value>>>, about: Facts, inputs: Vec<String>, outputs: Vec<String>) -> Result<Program, String> {
    let mut prog = program_facts::program(&about, ABOUT)?;
    prog.facts = facts;
    prog.decls = decls;
    prog.inputs = inputs;
    prog.outputs = outputs;
    prog.rules = rules;
    Ok(prog)
}

/// The `.about` rows of `prog`: its fields besides rules, declarations, facts, inputs and outputs, as facts.
pub fn about_of(prog: &Program) -> Facts {
    let mut about = Facts::new();
    program_facts::write_about(&mut about, ABOUT, prog);
    about
}
