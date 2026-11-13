//! A program as a flat image (decision 0173): a fixed set of regions, every reference an index, the bytes kept in the
//! store's database as a source, and read by viewing them in place.
//!
//! ```text
//! header   magic, format, region count, then per region: item width, item count, byte offset (16-byte aligned)
//! strings  bytes, end offsets, and a linear hash table of 64-byte buckets (text → atom id)
//! rels     relation number → its atom; atom → relation number + 1 (0: none)
//! nodes    kind (u8), size (u32), val (u32), in preorder; ints (i64) for the values past 31 bits
//! rules    each rule's first node, its variables' range, its hash
//! parts    declarations, facts (and the `.about` rows), inputs, outputs, each with its hash
//! indexes  rules by head relation, and the literals that read each relation, as CSR
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
//! Every name is an id into the image's own string table (the store's atoms take its place in a later stage). A
//! program read from an image is the program written, field by field: `encode` refuses what it cannot keep, as
//! `program_text::print` does.
use super::front::{AggOp, Atom, Literal, Lists, Name, Program, Rule, Term, Value};
use super::program_facts::{self, Facts};
use std::collections::{BTreeMap, HashMap};
use std::hash::BuildHasher;
use std::sync::Arc;

pub mod extents;
pub mod grouped;
pub mod strings;
pub mod version;

pub use extents::Extents;
pub use grouped::Grouped;
pub use strings::Strings;
pub use version::Version;

pub const MAGIC: [u8; 8] = *b"RRPIMG\x00\x01";
pub const FORMAT: u32 = 1;
/// The program the `.about` rows are written as (`program_facts`), as the text has them.
const ABOUT: &str = "a";

/// The regions, in their order in the image.
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
}
pub const REGIONS: usize = 35;
/// Each region's item width in bytes. Buckets are 64-byte records.
const WIDTH: [u32; REGIONS] = [8, 1, 4, 64, 64, 4, 4, 1, 4, 4, 8, 4, 4, 8, 4, 4, 4, 4, 8, 4, 4, 4, 1, 8, 8, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4];
const HEADER: usize = 16 + 24 * REGIONS;

/// `Meta`'s words.
mod meta {
    /// 0: a base; 1: an overlay's delta.
    pub const KIND: usize = 0;
    /// The rule ids an overlay's added rules start at: the parent's id space.
    pub const RULE_BASE: usize = 1;
    pub const HASH_LEVEL: usize = 2;
    pub const HASH_SPLIT: usize = 3;
    pub const HASH_COUNT: usize = 4;
    /// Bit 0: the inputs are this image's; bit 1, the outputs; bit 2, the `.about` rows. A base has all three.
    pub const OWNS: usize = 5;
    /// The hashes of the inputs, the outputs and the `.about` rows.
    pub const INPUTS_HASH: usize = 6;
    pub const OUTPUTS_HASH: usize = 7;
    pub const ABOUT_HASH: usize = 8;
    pub const WORDS: usize = 9;
}

/// Node kinds (the low five bits of `Kind`; the high three are the kind's flags).
pub mod kind {
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
}
const OPS: [&str; 6] = ["=", "!=", "<", "<=", ">", ">="];
/// A fact relation's flag in `FactRel`: one of the `.about` rows.
const ABOUT_BIT: u32 = 1 << 31;

pub fn hash<T: std::hash::Hash + ?Sized>(x: &T) -> u64 {
    foldhash::quality::FixedState::with_seed(0x0173).hash_one(x)
}

fn bad(what: impl std::fmt::Display) -> String {
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
    /// The image of `bytes`, copied to an aligned buffer and checked: its header, every region's width and bounds.
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
    pub fn u8s(&self, r: R) -> &[u8] {
        self.slice(r)
    }
    pub fn u32s(&self, r: R) -> &[u32] {
        self.slice(r)
    }
    pub fn u64s(&self, r: R) -> &[u64] {
        self.slice(r)
    }
    pub fn buckets(&self, r: R) -> &[[u64; 8]] {
        self.slice(r)
    }
    /// Region `r`'s byte offset and item count.
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
    pub fn rules(&self) -> usize {
        self.u32s(R::RuleNode).len()
    }

    /// The image's references, each inside what it names: offsets ascending and in bounds, every atom, relation and
    /// slot an index of its table. Node trees are checked as they are read (`decode`).
    fn check(&self) -> Result<(), String> {
        let atoms = self.u32s(R::StrEnds).len();
        let mut last = 0;
        for &e in self.u32s(R::StrEnds) {
            if e < last || e as usize > self.u8s(R::StrBytes).len() {
                return Err(bad("string ends"));
            }
            last = e;
        }
        for k in 0..atoms {
            if std::str::from_utf8(self.atom_bytes(k as u32)).is_err() {
                return Err(bad("a string is not UTF-8"));
            }
        }
        let in_atoms = |r: R| self.u32s(r).iter().all(|&a| (a as usize) < atoms);
        for r in [R::Rels, R::Vars, R::Cols, R::DeclName, R::Inputs, R::Outputs, R::DeclGone] {
            if !in_atoms(r) {
                return Err(bad(format!("{r:?} names no string")));
            }
        }
        for r in [R::FactRel, R::FactGone] {
            if !self.u32s(r).iter().all(|&a| ((a & !ABOUT_BIT) as usize) < atoms) {
                return Err(bad(format!("{r:?} names no string")));
            }
        }
        if self.u32s(R::AtomRel).len() != atoms || !self.u32s(R::AtomRel).iter().all(|&r| (r as usize) <= self.u32s(R::Rels).len()) {
            return Err(bad("atom → relation"));
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
        let decls = self.u32s(R::DeclName).len();
        if !offsets(R::DeclCols, decls, self.u32s(R::Cols).len() / 2) || self.u32s(R::Cols).len() % 2 != 0 || self.u64s(R::DeclHash).len() != decls {
            return Err(bad("declarations"));
        }
        let facts = self.u32s(R::FactRel).len();
        let rows = self.u32s(R::RowCells).len().saturating_sub(1);
        let cells = self.u8s(R::CellKind).len();
        if !offsets(R::FactRows, facts, rows) || !offsets(R::RowCells, rows, cells) || self.u64s(R::CellVal).len() != cells || self.u64s(R::FactHash).len() != facts {
            return Err(bad("facts"));
        }
        for (k, &v) in self.u8s(R::CellKind).iter().zip(self.u64s(R::CellVal)) {
            if *k > 1 || (*k == 1 && v as usize >= atoms) {
                return Err(bad("a fact's cell"));
            }
        }
        let rels = self.u32s(R::Rels).len();
        if !offsets(R::HeadStart, rels, self.u32s(R::HeadRules).len()) || !self.u32s(R::HeadRules).iter().all(|&r| (r as usize) < rules) {
            return Err(bad("rules by head"));
        }
        let reads = self.u32s(R::ReadRule).len();
        if !offsets(R::ReadStart, rels, reads) || self.u32s(R::ReadNode).len() != reads || !self.u32s(R::ReadRule).iter().all(|&r| (r as usize) < rules) {
            return Err(bad("reads"));
        }
        let removed = self.u32s(R::Removed);
        if !removed.windows(2).all(|w| w[0] < w[1]) || (!self.is_delta() && !removed.is_empty()) || removed.last().is_some_and(|&r| r >= self.rule_base()) {
            return Err(bad("removed rules"));
        }
        let level = self.meta(meta::HASH_LEVEL);
        let buckets = self.buckets(R::StrBuckets).len() as u64;
        if level > 31 || buckets != (1u64 << level) + self.meta(meta::HASH_SPLIT) || self.meta(meta::HASH_COUNT) != atoms as u64 {
            return Err(bad("string table"));
        }
        Ok(())
    }

    pub fn atoms(&self) -> usize {
        self.u32s(R::StrEnds).len()
    }
    fn atom_bytes(&self, a: u32) -> &[u8] {
        let ends = self.u32s(R::StrEnds);
        let start = if a == 0 { 0 } else { ends[a as usize - 1] as usize };
        &self.u8s(R::StrBytes)[start..ends[a as usize] as usize]
    }
    /// The text of atom `a`.
    pub fn atom(&self, a: u32) -> &str {
        // SAFETY: `check` found every string UTF-8.
        unsafe { std::str::from_utf8_unchecked(self.atom_bytes(a)) }
    }
    /// The atom of `text`, by the string table's hash.
    pub fn find(&self, text: &str) -> Option<u32> {
        let h = strings::hash(text.as_bytes());
        strings::find_in(self.buckets(R::StrBuckets), self.buckets(R::StrOver), self.meta(meta::HASH_LEVEL) as u32, self.meta(meta::HASH_SPLIT) as u32, h, |a| self.atom_bytes(a) == text.as_bytes())
    }
    /// The relation number of relation `name`.
    pub fn rel(&self, name: &str) -> Option<u32> {
        let a = self.find(name)?;
        self.u32s(R::AtomRel)[a as usize].checked_sub(1)
    }
    /// The rules (local indexes) whose head is relation `rel`.
    pub fn by_head(&self, rel: u32) -> &[u32] {
        let s = self.u32s(R::HeadStart);
        &self.u32s(R::HeadRules)[s[rel as usize] as usize..s[rel as usize + 1] as usize]
    }
    /// The (rule, literal node) pairs that read relation `rel` in their bodies.
    pub fn reads(&self, rel: u32) -> impl Iterator<Item = (u32, u32)> + '_ {
        let s = self.u32s(R::ReadStart);
        let (a, b) = (s[rel as usize] as usize, s[rel as usize + 1] as usize);
        self.u32s(R::ReadRule)[a..b].iter().copied().zip(self.u32s(R::ReadNode)[a..b].iter().copied())
    }
    pub fn rule_hash(&self, k: usize) -> u64 {
        self.u64s(R::RuleHash)[k]
    }
    /// The image's rules' hashes, summed: a part of its digest.
    pub fn rules_digest(&self) -> u64 {
        self.u64s(R::RuleHash).iter().fold(0u64, |a, &h| a.wrapping_add(h))
    }
}

// ---- writing ----

/// The regions of an image being written, each appended to, then laid out once.
pub struct Writer {
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

/// What an image holds of a program's parts besides its rules, for an overlay's delta: each `Some` is the delta's.
#[derive(Default)]
pub struct Parts<'p> {
    pub decls: Vec<(&'p Name, &'p [(Name, Name)])>,
    pub decls_gone: Vec<String>,
    pub facts: Vec<(&'p str, &'p [super::front::List<Value>], bool)>,
    pub facts_gone: Vec<(String, bool)>,
    pub inputs: Option<&'p [String]>,
    pub outputs: Option<&'p [String]>,
    pub about: Option<&'p Facts>,
}

/// The rules and parts of one image, encoded.
pub struct Encoder {
    w: Writer,
    strings: Strings,
    rels: Vec<u32>,
    rel_of: HashMap<u32, u32>,
    kind: Extents<u8>,
    size: Extents<u32>,
    val: Extents<u32>,
    ints: Vec<i64>,
    rule_node: Vec<u32>,
    rule_vars: Vec<u32>,
    vars: Vec<u32>,
    rule_hash: Vec<u64>,
    heads: Grouped,
    reads: Grouped,
    read_nodes: Vec<u32>,
    read_rules: Vec<u32>,
    slots: HashMap<String, u32>,
    slot_names: Vec<u32>,
    decl_cols: Vec<u32>,
    decl_ends: Vec<u32>,
    cells: u32,
    row_ends: Vec<u32>,
    fact_ends: Vec<u32>,
}

impl Default for Encoder {
    fn default() -> Self {
        Encoder {
            w: Writer::default(),
            strings: Strings::default(),
            rels: Vec::new(),
            rel_of: HashMap::new(),
            kind: Extents::new(12),
            size: Extents::new(12),
            val: Extents::new(12),
            ints: Vec::new(),
            rule_node: Vec::new(),
            rule_vars: vec![0],
            vars: Vec::new(),
            rule_hash: Vec::new(),
            heads: Grouped::default(),
            reads: Grouped::default(),
            read_nodes: Vec::new(),
            read_rules: Vec::new(),
            slots: HashMap::new(),
            slot_names: Vec::new(),
            decl_cols: Vec::new(),
            decl_ends: vec![0],
            cells: 0,
            row_ends: vec![0],
            fact_ends: vec![0],
        }
    }
}

impl Encoder {
    fn atom_id(&mut self, s: &str) -> u32 {
        self.strings.intern(s)
    }
    fn rel(&mut self, s: &str) -> u32 {
        let a = self.atom_id(s);
        let n = self.rels.len() as u32;
        *self.rel_of.entry(a).or_insert_with(|| {
            self.rels.push(a);
            n
        })
    }
    fn open(&mut self, k: u8, val: u32) -> usize {
        let at = self.kind.len();
        self.kind.push(k);
        self.size.push(0);
        self.val.push(val);
        at
    }
    fn close(&mut self, at: usize) {
        self.size.set(at, (self.kind.len() - at) as u32);
    }
    fn leaf(&mut self, k: u8, val: u32) {
        self.kind.push(k);
        self.size.push(1);
        self.val.push(val);
    }
    fn slot(&mut self, v: &str) -> u32 {
        if let Some(&s) = self.slots.get(v) {
            return s;
        }
        let s = self.slot_names.len() as u32;
        let a = self.atom_id(v);
        self.slot_names.push(a);
        self.slots.insert(v.to_owned(), s);
        s
    }
    fn var(&mut self, v: &str) {
        let s = self.slot(v);
        self.leaf(kind::VAR, s);
    }
    fn int(&mut self, i: i64) {
        if (0..1 << 31).contains(&i) {
            self.leaf(kind::INT, i as u32);
        } else {
            let at = self.ints.len() as u32;
            self.ints.push(i);
            self.leaf(kind::INT | kind::BIG, at);
        }
    }
    fn term(&mut self, t: &Term) {
        match t {
            Term::Var(v) => self.var(v),
            Term::Wild => self.leaf(kind::WILD, 0),
            Term::Top => self.leaf(kind::TOP, 0),
            Term::Int(i) => self.int(*i),
            Term::Str(s) => {
                let a = self.atom_id(s);
                self.leaf(kind::STR, a)
            }
            Term::Call(f, args) => {
                let a = self.atom_id(f);
                let at = self.open(kind::CALL, a);
                for x in args.iter() {
                    self.term(x);
                }
                self.close(at);
            }
        }
    }
    fn atom(&mut self, a: &Atom, rule: u32, body: bool) {
        let flags = if a.neg { kind::NEG } else { 0 } | if a.gated { kind::GATED } else { 0 };
        let r = self.rel(&a.rel);
        let at = self.open(kind::ATOM | flags, r);
        if body {
            let e = self.read_nodes.len() as u32;
            self.reads.push(r, e);
            self.read_nodes.push(at as u32);
            self.read_rules.push(rule);
        }
        for x in a.args.iter() {
            self.term(x);
        }
        self.close(at);
    }
    fn literal(&mut self, l: &Literal, rule: u32) -> Result<(), String> {
        match l {
            Literal::Atom(a) => self.atom(a, rule, true),
            Literal::Cmp { l, op, r } => {
                let o = OPS.iter().position(|x| x == op).ok_or_else(|| bad(format!("comparison {op:?}")))? as u32;
                let at = self.open(kind::CMP, o);
                self.term(l);
                self.term(r);
                self.close(at);
            }
            Literal::Agg { var, op, over, atom } => {
                let code = match op {
                    AggOp::Count => 0,
                    AggOp::Min => 1,
                    AggOp::Max => 2,
                    AggOp::Sum => 3,
                    AggOp::Product => 4,
                    AggOp::Join { .. } => 5,
                };
                let s = self.slot(var);
                let at = self.open(kind::AGG | code << 5, s);
                match over {
                    Some(v) => self.var(v),
                    None => self.leaf(kind::WILD, 0),
                }
                if let AggOp::Join { by, sep } = op {
                    for b in by {
                        self.var(b);
                    }
                    let a = self.atom_id(sep);
                    self.leaf(kind::STR, a);
                }
                self.atom(atom, rule, true);
                self.close(at);
            }
            Literal::Nearest { n, a, kindset } => {
                let k = self.atom_id(kindset);
                let at = self.open(kind::NEAREST, k);
                self.var(n);
                self.var(a);
                self.close(at);
            }
            Literal::Ast { document, node, matcher, captures } => {
                let m = self.atom_id(matcher);
                let at = self.open(kind::AST, m);
                self.var(document);
                self.var(node);
                for (n, v) in captures {
                    let a = self.atom_id(n);
                    self.leaf(kind::STR, a);
                    self.var(v);
                }
                self.close(at);
            }
        }
        Ok(())
    }
    /// Appends `rule`: its nodes, its variables, its hash, and its index entries.
    pub fn rule(&mut self, rule: &Rule) -> Result<(), String> {
        let k = self.rule_node.len() as u32;
        self.slots.clear();
        self.slot_names.clear();
        let at = self.open(kind::RULE, 0);
        self.rule_node.push(at as u32);
        self.atom(&rule.head, k, false);
        let head = self.rel(&rule.head.rel);
        self.heads.push(head, k);
        for l in rule.body.iter() {
            self.literal(l, k)?;
        }
        self.close(at);
        self.vars.extend_from_slice(&self.slot_names);
        self.rule_vars.push(self.vars.len() as u32);
        self.rule_hash.push(hash(rule));
        Ok(())
    }
    fn facts(&mut self, rel: &str, rows: &[super::front::List<Value>], about: bool) {
        let a = self.atom_id(rel);
        self.w.u32(R::FactRel, a | if about { ABOUT_BIT } else { 0 });
        self.w.u64(R::FactHash, hash(&(rel, rows, about)));
        for row in rows {
            for v in row.iter() {
                match v {
                    Value::Int(i) => {
                        self.w.u8(R::CellKind, 0);
                        self.w.u64(R::CellVal, *i as u64);
                    }
                    Value::Str(s) => {
                        let a = self.atom_id(s);
                        self.w.u8(R::CellKind, 1);
                        self.w.u64(R::CellVal, a as u64);
                    }
                }
                self.cells += 1;
            }
            self.row_ends.push(self.cells);
        }
        self.fact_ends.push(self.row_ends.len() as u32 - 1);
    }
    fn decl(&mut self, rel: &str, cols: &[(Name, Name)]) {
        let a = self.atom_id(rel);
        self.w.u32(R::DeclName, a);
        self.w.u64(R::DeclHash, hash(&(rel, cols)));
        for (n, t) in cols {
            let (n, t) = (self.atom_id(n), self.atom_id(t));
            self.decl_cols.push(n);
            self.decl_cols.push(t);
        }
        self.decl_ends.push(self.decl_cols.len() as u32 / 2);
    }
    fn names(&mut self, r: R, names: &[String]) -> u64 {
        for n in names {
            let a = self.atom_id(n);
            self.w.u32(r, a);
        }
        hash(names)
    }

    /// The image: `rules`, and `parts`; a delta when `delta` is the parent's rule id space and the rules it removes.
    pub fn finish(mut self, parts: &Parts<'_>, delta: Option<(u32, &[u32])>) -> Vec<u8> {
        for (rel, cols) in &parts.decls {
            self.decl(rel, cols);
        }
        for rel in &parts.decls_gone {
            let a = self.atom_id(rel);
            self.w.u32(R::DeclGone, a);
        }
        for (rel, rows, about) in &parts.facts {
            self.facts(rel, rows, *about);
        }
        for (rel, about) in &parts.facts_gone {
            let a = self.atom_id(rel);
            self.w.u32(R::FactGone, a | if *about { ABOUT_BIT } else { 0 });
        }
        let mut owns = 0u64;
        let (mut ih, mut oh, mut ah) = (0, 0, 0);
        if let Some(i) = parts.inputs {
            owns |= 1;
            ih = self.names(R::Inputs, i);
        }
        if let Some(o) = parts.outputs {
            owns |= 2;
            oh = self.names(R::Outputs, o);
        }
        if let Some(about) = parts.about {
            owns |= 4;
            ah = hash(about);
            for (rel, rows) in about.iter() {
                self.facts(rel, rows, true);
            }
        }
        let Encoder { mut w, strings, rels, kind, size, val, ints, rule_node, rule_vars, vars, rule_hash, heads, reads, read_nodes, read_rules, decl_cols, decl_ends, row_ends, fact_ends, .. } = self;
        // Strings, and each atom's relation.
        let atoms = strings.len();
        let mut atom_rel = vec![0u32; atoms];
        for (r, &a) in rels.iter().enumerate() {
            atom_rel[a as usize] = r as u32 + 1;
        }
        let (bytes, ends, table) = strings.into_parts();
        w.raw(R::StrBytes, &bytes);
        w.u32s(R::StrEnds, &ends);
        for b in &table.buckets {
            for x in b {
                w.u64(R::StrBuckets, *x);
            }
        }
        for b in &table.over {
            for x in b {
                w.u64(R::StrOver, *x);
            }
        }
        w.u32s(R::Rels, &rels);
        w.u32s(R::AtomRel, &atom_rel);
        // Nodes, one region a column, each extent in turn.
        for e in kind.extents() {
            w.raw(R::Kind, e);
        }
        for e in size.extents() {
            w.u32s(R::Size, e);
        }
        for e in val.extents() {
            w.u32s(R::Val, e);
        }
        for i in &ints {
            w.u64(R::Ints, *i as u64);
        }
        w.u32s(R::RuleNode, &rule_node);
        w.u32s(R::RuleVars, &rule_vars);
        for h in &rule_hash {
            w.u64(R::RuleHash, *h);
        }
        w.u32s(R::Vars, &vars);
        w.u32s(R::DeclCols, &decl_ends);
        w.u32s(R::Cols, &decl_cols);
        w.u32s(R::FactRows, &fact_ends);
        w.u32s(R::RowCells, &row_ends);
        // Indexes: rules by head, and reads, each as CSR over the relation numbers.
        let (starts, values) = heads.freeze(rels.len());
        w.u32s(R::HeadStart, &starts);
        w.u32s(R::HeadRules, &values);
        let (starts, entries) = reads.freeze(rels.len());
        w.u32s(R::ReadStart, &starts);
        let (rules, nodes): (Vec<u32>, Vec<u32>) = entries.iter().map(|&e| (read_rules[e as usize], read_nodes[e as usize])).unzip();
        w.u32s(R::ReadRule, &rules);
        w.u32s(R::ReadNode, &nodes);
        let (rule_base, removed) = delta.unwrap_or((0, &[]));
        w.u32s(R::Removed, removed);
        let m = [delta.is_some() as u64, rule_base as u64, table.level as u64, table.split as u64, table.count, owns, ih, oh, ah];
        for x in m {
            w.u64(R::Meta, x);
        }
        w.finish()
    }
}

/// The image of `prog`: a base, every part its own. Refuses what the image has no form for (maintenance's parts,
/// which the text refuses too) and a comparison it does not know.
pub fn encode(prog: &Program) -> Result<Vec<u8>, String> {
    if !prog.again.is_empty() || !prog.together.is_empty() || !prog.shapes.is_empty() {
        return Err(bad("maintenance's parts (again, together, shapes) have no image"));
    }
    let mut e = Encoder::default();
    for r in &prog.rules {
        e.rule(r)?;
    }
    let mut about = Facts::new();
    program_facts::write_about(&mut about, ABOUT, prog);
    let parts = Parts {
        decls: prog.decls.iter().map(|(n, c)| (n, c.as_slice())).collect(),
        facts: prog.facts.iter().map(|(r, rows)| (r.as_str(), rows.as_slice(), false)).collect(),
        inputs: Some(&prog.inputs),
        outputs: Some(&prog.outputs),
        about: Some(&about),
        ..Parts::default()
    };
    Ok(e.finish(&parts, None))
}

// ---- reading ----

/// Reads one image's rules and parts, its atoms made `Name`s once each.
pub struct Reader<'i> {
    pub image: &'i Image,
    names: Vec<Option<Name>>,
    lists: &'i Lists,
    /// Scratch for the terms of the lists being read, nested ones above their parent's (a call's inside an atom's):
    /// each list is interned from its slice and the slice popped, so a read allocates no list of its own.
    terms: Vec<Term>,
    literals: Vec<Literal>,
}

impl<'i> Reader<'i> {
    pub fn new(image: &'i Image, lists: &'i Lists) -> Reader<'i> {
        Reader { image, names: vec![None; image.atoms()], lists, terms: Vec::new(), literals: Vec::new() }
    }
    fn name(&mut self, a: u32) -> Name {
        let slot = &mut self.names[a as usize];
        slot.get_or_insert_with(|| Name::new(self.image.atom(a))).clone()
    }
    fn text(&self, a: u32) -> String {
        self.image.atom(a).to_owned()
    }
    /// Local rule `k`.
    pub fn rule(&mut self, k: usize) -> Result<Rule, String> {
        let im = self.image;
        let (kinds, sizes, vals) = (im.u8s(R::Kind), im.u32s(R::Size), im.u32s(R::Val));
        let at = im.u32s(R::RuleNode)[k] as usize;
        let vs = im.u32s(R::RuleVars);
        let vars = &im.u32s(R::Vars)[vs[k] as usize..vs[k + 1] as usize];
        if kinds.get(at) != Some(&kind::RULE) {
            return Err(bad(format!("rule {k} starts at no rule node")));
        }
        let end = at + sizes[at] as usize;
        if end > kinds.len() || sizes[at] < 2 {
            return Err(bad(format!("rule {k}'s size")));
        }
        let mut cur = Cursor { at: at + 1, end, kinds, sizes, vals, ints: im.u64s(R::Ints), vars };
        // A failed read leaves its partial lists behind.
        self.terms.clear();
        self.literals.clear();
        let head = self.atom(&mut cur)?;
        while cur.at < end {
            let l = self.literal(&mut cur)?;
            self.literals.push(l);
        }
        let body = self.lists.literals(&self.literals);
        self.literals.clear();
        Ok(Rule { head, body })
    }
    fn slot(&mut self, cur: &Cursor<'_>, s: u32) -> Result<u32, String> {
        cur.vars.get(s as usize).copied().ok_or_else(|| bad(format!("slot {s}")))
    }
    fn var_name(&mut self, cur: &mut Cursor<'_>) -> Result<String, String> {
        let (k, _, v) = cur.next(kind::VAR)?;
        let _ = k;
        let a = self.slot(cur, v)?;
        Ok(self.text(a))
    }
    fn term(&mut self, cur: &mut Cursor<'_>) -> Result<Term, String> {
        let (k, size, v) = cur.peek()?;
        cur.at += 1;
        Ok(match k & kind::MASK {
            kind::VAR => {
                let a = self.slot(cur, v)?;
                Term::Var(self.name(a))
            }
            kind::WILD => Term::Wild,
            kind::TOP => Term::Top,
            kind::INT if k & kind::BIG != 0 => Term::Int(*cur.ints.get(v as usize).ok_or_else(|| bad("an int"))? as i64),
            kind::INT => Term::Int(v as i64),
            kind::STR => Term::Str(self.atom_name(v)?),
            kind::CALL => {
                let end = cur.at - 1 + size as usize;
                let f = self.atom_name(v)?;
                Term::Call(f, self.term_list(cur, end)?)
            }
            other => return Err(bad(format!("term kind {other}"))),
        })
    }
    /// The terms from the cursor up to `end`, as a shared list.
    fn term_list(&mut self, cur: &mut Cursor<'_>, end: usize) -> Result<super::front::List<Term>, String> {
        let start = self.terms.len();
        while cur.at < end {
            let t = self.term(cur)?;
            self.terms.push(t);
        }
        let list = self.lists.terms(&self.terms[start..]);
        self.terms.truncate(start);
        Ok(list)
    }
    fn atom_name(&mut self, a: u32) -> Result<Name, String> {
        if (a as usize) < self.image.atoms() { Ok(self.name(a)) } else { Err(bad(format!("atom {a}"))) }
    }
    fn atom(&mut self, cur: &mut Cursor<'_>) -> Result<Atom, String> {
        let (k, size, v) = cur.peek()?;
        if k & kind::MASK != kind::ATOM {
            return Err(bad("no atom"));
        }
        let end = cur.at + size as usize;
        cur.at += 1;
        let rel = *self.image.u32s(R::Rels).get(v as usize).ok_or_else(|| bad(format!("relation {v}")))?;
        let rel = self.name(rel);
        let args = self.term_list(cur, end)?;
        Ok(Atom { rel, args, neg: k & kind::NEG != 0, gated: k & kind::GATED != 0 })
    }
    fn literal(&mut self, cur: &mut Cursor<'_>) -> Result<Literal, String> {
        let (k, size, v) = cur.peek()?;
        let end = cur.at + size as usize;
        Ok(match k & kind::MASK {
            kind::ATOM => Literal::Atom(self.atom(cur)?),
            kind::CMP => {
                cur.at += 1;
                let op = *OPS.get(v as usize).ok_or_else(|| bad(format!("operator {v}")))?;
                let l = self.term(cur)?;
                let r = self.term(cur)?;
                Literal::Cmp { l, op, r }
            }
            kind::AGG => {
                cur.at += 1;
                let a = self.slot(cur, v)?;
                let var = self.text(a);
                let over = match cur.peek()?.0 & kind::MASK {
                    kind::WILD => {
                        cur.at += 1;
                        None
                    }
                    _ => Some(self.var_name(cur)?),
                };
                let op = match k >> 5 {
                    0 => AggOp::Count,
                    1 => AggOp::Min,
                    2 => AggOp::Max,
                    3 => AggOp::Sum,
                    4 => AggOp::Product,
                    5 => {
                        let mut by = Vec::new();
                        while cur.peek()?.0 & kind::MASK == kind::VAR {
                            by.push(self.var_name(cur)?);
                        }
                        let (_, _, s) = cur.next(kind::STR)?;
                        AggOp::Join { by, sep: self.text(s) }
                    }
                    other => return Err(bad(format!("aggregate {other}"))),
                };
                let atom = self.atom(cur)?;
                Literal::Agg { var, op, over, atom }
            }
            kind::NEAREST => {
                cur.at += 1;
                let n = self.var_name(cur)?;
                let a = self.var_name(cur)?;
                Literal::Nearest { n, a, kindset: self.text(v) }
            }
            kind::AST => {
                cur.at += 1;
                let document = self.var_name(cur)?;
                let node = self.var_name(cur)?;
                let mut captures = Vec::new();
                while cur.at < end {
                    let (_, _, s) = cur.next(kind::STR)?;
                    captures.push((self.text(s), self.var_name(cur)?));
                }
                Literal::Ast { document, node, matcher: self.text(v), captures }
            }
            other => return Err(bad(format!("literal kind {other}"))),
        })
    }

    /// The declarations, by name, and those this image removes.
    pub fn decls(&mut self) -> (Vec<(Name, super::front::List<(Name, Name)>)>, Vec<String>) {
        let im = self.image;
        let ends = im.u32s(R::DeclCols);
        let cols = im.u32s(R::Cols);
        let mut out = Vec::new();
        for (k, &n) in im.u32s(R::DeclName).iter().enumerate() {
            let c: Vec<(Name, Name)> = (ends[k]..ends[k + 1]).map(|i| (self.name(cols[2 * i as usize]), self.name(cols[2 * i as usize + 1]))).collect();
            let name = self.name(n);
            out.push((name, self.lists.columns(&c)));
        }
        (out, im.u32s(R::DeclGone).iter().map(|&a| self.text(a)).collect())
    }
    /// The fact relations (`.about`'s flagged), each with its rows, and those this image removes.
    #[allow(clippy::type_complexity)]
    pub fn facts(&mut self) -> (Vec<(String, bool, Vec<super::front::List<Value>>)>, Vec<(String, bool)>) {
        let im = self.image;
        let (rows, cells, kinds, vals) = (im.u32s(R::FactRows), im.u32s(R::RowCells), im.u8s(R::CellKind), im.u64s(R::CellVal));
        let mut out = Vec::new();
        for (k, &r) in im.u32s(R::FactRel).iter().enumerate() {
            let mut list = Vec::with_capacity((rows[k + 1] - rows[k]) as usize);
            for row in rows[k]..rows[k + 1] {
                let vs: Vec<Value> = (cells[row as usize]..cells[row as usize + 1])
                    .map(|c| if kinds[c as usize] == 0 { Value::Int(vals[c as usize] as i64) } else { Value::Str(self.name(vals[c as usize] as u32)) })
                    .collect();
                list.push(self.lists.values(&vs));
            }
            out.push((self.text(r & !ABOUT_BIT), r & ABOUT_BIT != 0, list));
        }
        (out, im.u32s(R::FactGone).iter().map(|&a| (self.text(a & !ABOUT_BIT), a & ABOUT_BIT != 0)).collect())
    }
    pub fn names(&self, r: R) -> Vec<String> {
        self.image.u32s(r).iter().map(|&a| self.text(a)).collect()
    }
}

struct Cursor<'c> {
    at: usize,
    end: usize,
    kinds: &'c [u8],
    sizes: &'c [u32],
    vals: &'c [u32],
    ints: &'c [u64],
    vars: &'c [u32],
}
impl Cursor<'_> {
    fn peek(&self) -> Result<(u8, u32, u32), String> {
        if self.at >= self.end {
            return Err(bad("a rule ends early"));
        }
        let size = self.sizes[self.at];
        if size == 0 || self.at + size as usize > self.end {
            return Err(bad(format!("node {} overruns its rule", self.at)));
        }
        Ok((self.kinds[self.at], size, self.vals[self.at]))
    }
    fn next(&mut self, want: u8) -> Result<(u8, u32, u32), String> {
        let n = self.peek()?;
        if n.0 & kind::MASK != want || n.1 != 1 {
            return Err(bad(format!("node {}: kind {}, not {want}", self.at, n.0 & kind::MASK)));
        }
        self.at += 1;
        Ok(n)
    }
}

/// The program image `bytes` holds, its lists taken from the run's `lists`.
pub fn decode(bytes: &[u8], lists: &Lists) -> Result<Program, String> {
    let image = Arc::new(Image::open(bytes)?);
    if image.is_delta() {
        return Err(bad("a delta is read through its parent (`Version`)"));
    }
    Version::Base(image).program(lists)
}

/// The program of a base image's parts and rules, assembled as the text's reader assembles it.
pub(crate) fn assemble(rules: Vec<Rule>, decls: BTreeMap<Name, super::front::List<(Name, Name)>>, facts: BTreeMap<String, Vec<super::front::List<Value>>>, about: Facts, inputs: Vec<String>, outputs: Vec<String>) -> Result<Program, String> {
    let mut prog = program_facts::program(&about, ABOUT)?;
    for (rel, rows) in facts {
        prog.facts.entry(rel).or_default().extend(rows);
    }
    prog.decls = decls;
    prog.inputs = inputs;
    prog.outputs = outputs;
    prog.rules = rules;
    Ok(prog)
}
