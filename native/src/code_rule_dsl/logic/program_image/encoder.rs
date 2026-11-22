//! Writes one image (decision 0173): its strings, relations, rules' nodes, parts and indexes. A delta's encoder
//! numbers what it adds after its parent's id spaces, and finds what its parent holds by its parent's tables, so a
//! delta holds only what it adds. Hashes are of texts, never of ids, so a rule or part hashes the same in any image.
use rerail_blocks::vector::Vector;
use super::{ABOUT_BIT, AtomId, Cell, R, RelId, Version, bad, csr, extents::Extents, kind, meta, strings::{self, Strings}};
use rerail_blocks::maps::BlockHash;
use std::hash::{BuildHasher, Hasher};

fn hasher() -> impl Hasher {
    foldhash::quality::FixedState::with_seed(0x0173).build_hasher()
}
fn put_text(h: &mut impl Hasher, s: &str) {
    h.write(s.as_bytes());
    h.write_u8(0xff);
}

/// A declaration's hash: its relation and its columns' names and types.
pub fn decl_hash<'a>(rel: &str, cols: impl Iterator<Item = (&'a str, &'a str)>) -> u64 {
    let mut h = hasher();
    h.write_u8(1);
    put_text(&mut h, rel);
    for (c, t) in cols {
        put_text(&mut h, c);
        put_text(&mut h, t);
    }
    h.finish()
}

/// A fact's cell, by text.
#[derive(Clone, Copy)]
pub enum CellText<'a> {
    Int(i64),
    Str(&'a str),
    Branded(crate::code_rule_dsl::logic::front::ast::Branded),
}

/// A fact relation's hash: its name, whether it is of the `.about` rows, and its rows. A relation's facts are a set,
/// so their hash is of the rows whatever their order (each row hashed alone, the hashes summed): the same facts in
/// another order are the same program (`Version::digest`), as a phase's export maintained from kept rows (read in
/// their canonical order, decision 0456 K3) gives the same rows as one derived anew, in another order. The `.about`
/// rows are hashed in their order.
pub fn facts_hash<'a, Row: Iterator<Item = CellText<'a>>>(rel: &str, about: bool, rows: impl Iterator<Item = Row>) -> u64 {
    let mut h = hasher();
    h.write_u8(2);
    put_text(&mut h, rel);
    h.write_u8(about as u8);
    if !about {
        let (mut sum, mut n) = (0u64, 0u64);
        for row in rows {
            let mut r = hasher();
            put_row(&mut r, row);
            sum = sum.wrapping_add(r.finish());
            n += 1;
        }
        h.write_u64(n);
        h.write_u64(sum);
        return h.finish();
    }
    for row in rows {
        put_row(&mut h, row);
    }
    h.finish()
}

/// A fact row into `h`.
fn put_row<'a>(mut h: &mut impl std::hash::Hasher, row: impl Iterator<Item = CellText<'a>>) {
    {
        h.write_u8(3);
        for c in row {
            match c {
                CellText::Int(i) => {
                    h.write_u8(4);
                    h.write_i64(i);
                }
                CellText::Str(s) => {
                    h.write_u8(5);
                    put_text(&mut h, s);
                }
                CellText::Branded(b) => {
                    h.write_u8(6);
                    h.write_u8(super::brand_code(b.class));
                    h.write_u32(b.atom);
                }
            }
        }
    }
}

/// The hash of a list of names (inputs, outputs); `tag` tells the lists apart.
pub fn names_hash<'a>(tag: u8, names: impl Iterator<Item = &'a str>) -> u64 {
    let mut h = hasher();
    h.write_u8(6 + tag);
    for n in names {
        put_text(&mut h, n);
    }
    h.finish()
}

/// A fact relation's rows of cells, in a block (decision 0569 §4): each row's end in `cells`, then the cells.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Rows {
    ends: Vector<u32>,
    cells: Vector<Cell>,
}

/// Rows read where they lie: a `Rows`', a builder's part's.
#[derive(Clone, Copy)]
pub struct RowsRef<'a> {
    ends: &'a [u32],
    cells: &'a [Cell],
}

impl<'a> RowsRef<'a> {
    pub fn len(&self) -> usize {
        self.ends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    pub fn get(self, i: usize) -> Option<&'a [Cell]> {
        let end = *self.ends.get(i)? as usize;
        let start = if i == 0 { 0 } else { self.ends[i - 1] as usize };
        Some(&self.cells[start..end])
    }

    pub fn iter(self) -> impl Iterator<Item = &'a [Cell]> + Clone + 'a {
        let (ends, cells) = (self.ends, self.cells);
        (0..ends.len()).map(move |i| &cells[if i == 0 { 0 } else { ends[i - 1] as usize }..ends[i] as usize])
    }

    /// Every cell, row after row.
    pub fn cells(self) -> &'a [Cell] {
        self.cells
    }

    /// Its rows as a `Rows` of their own, to be edited and stated again.
    pub fn to_rows(self) -> Rows {
        // copy: derived: the rows an edit is made from, stated again by the builder (`set_about_rows`).
        Rows { ends: Vector::from(self.ends), cells: Vector::from(self.cells) }
    }
}

impl Rows {
    pub fn new() -> Rows {
        Rows::default()
    }

    pub fn as_ref(&self) -> RowsRef<'_> {
        RowsRef { ends: &self.ends, cells: &self.cells }
    }

    pub fn len(&self) -> usize {
        self.ends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    /// Rows of cells read one by one (a version's fact relation's).
    pub fn of_cells<R: IntoIterator<Item = Cell>>(rows: impl Iterator<Item = R>) -> Rows {
        let mut out = Rows::new();
        for r in rows {
            out.push_cells(r);
        }
        out
    }

    /// A row of cells read one by one.
    pub fn push_cells(&mut self, row: impl IntoIterator<Item = Cell>) {
        self.cells.extend(row);
        self.ends.push(u32::try_from(self.cells.len()).expect("4G cells"));
    }

    pub fn push(&mut self, row: impl AsRef<[Cell]>) {
        self.cells.extend_from_slice(row.as_ref());
        self.ends.push(u32::try_from(self.cells.len()).expect("4G cells"));
    }

    pub fn get(&self, i: usize) -> Option<&[Cell]> {
        self.as_ref().get(i)
    }

    pub fn iter(&self) -> impl Iterator<Item = &[Cell]> + Clone + '_ {
        self.as_ref().iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut [Cell]> + '_ {
        let Rows { ends, cells } = self;
        let mut rest: &mut [Cell] = cells;
        let mut prev = 0usize;
        ends.iter().map(move |&e| {
            let (row, after) = std::mem::take(&mut rest).split_at_mut(e as usize - prev);
            prev = e as usize;
            rest = after;
            row
        })
    }

    /// Keeps the rows `keep` takes, in order.
    pub fn retain(&mut self, mut keep: impl FnMut(&[Cell]) -> bool) {
        let (mut w, mut start, mut kept) = (0usize, 0usize, 0usize);
        for i in 0..self.ends.len() {
            let end = self.ends[i] as usize;
            if keep(&self.cells[start..end]) {
                let n = end - start;
                self.cells.copy_within(start..end, w);
                w += n;
                self.ends[kept] = w as u32;
                kept += 1;
            }
            start = end;
        }
        self.cells.truncate(w);
        self.ends.truncate(kept);
    }

    pub fn first(&self) -> Option<&[Cell]> {
        self.get(0)
    }

    /// Orders the rows by `cmp`, stably: a permutation of the rows, then the rows laid again in its order.
    pub fn sort_by(&mut self, mut cmp: impl FnMut(&[Cell], &[Cell]) -> std::cmp::Ordering) {
        let mut order: Vector<u32> = (0..self.ends.len() as u32).collect();
        let r = self.as_ref();
        order.sort_by(|&a, &b| cmp(r.get(a as usize).expect("row"), r.get(b as usize).expect("row")));
        let mut out = Rows { ends: Vector::with_capacity(self.ends.len()), cells: Vector::with_capacity(self.cells.len()) };
        for &i in order.iter() {
            out.push(r.get(i as usize).expect("row"));
        }
        *self = out;
    }

    /// Orders the rows by the key `key` gives each, stably.
    pub fn sort_by_key<K: Ord>(&mut self, mut key: impl FnMut(&[Cell]) -> K) {
        self.sort_by(|a, b| key(a).cmp(&key(b)));
    }

    /// Orders the rows by the key `key` gives each, computed once a row, stably.
    pub fn sort_by_cached_key<K: Ord>(&mut self, key: impl FnMut(&[Cell]) -> K) {
        // copy: derived: each row's key, computed once for the sort, dropped after it.
        let keys: Vec<K> = self.iter().map(key).collect();
        let mut order: Vector<u32> = (0..self.ends.len() as u32).collect();
        order.sort_by(|&a, &b| keys[a as usize].cmp(&keys[b as usize]));
        let r = self.as_ref();
        let mut out = Rows { ends: Vector::with_capacity(self.ends.len()), cells: Vector::with_capacity(self.cells.len()) };
        for &i in order.iter() {
            out.push(r.get(i as usize).expect("row"));
        }
        *self = out;
    }
}

impl<'a> IntoIterator for &'a Rows {
    type Item = &'a [Cell];
    type IntoIter = Box<dyn Iterator<Item = &'a [Cell]> + 'a>;
    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

impl<R: AsRef<[Cell]>> FromIterator<R> for Rows {
    fn from_iter<I: IntoIterator<Item = R>>(it: I) -> Rows {
        let mut rows = Rows::new();
        for r in it {
            rows.push(r);
        }
        rows
    }
}

impl<R: AsRef<[Cell]>> Extend<R> for Rows {
    fn extend<I: IntoIterator<Item = R>>(&mut self, it: I) {
        for r in it {
            self.push(r);
        }
    }
}

/// A declaration of a builder's parts: its columns' (name, type) atoms in `Parts::cols`, and its hash.
#[repr(C)]
#[derive(Clone, Copy)]
struct DeclAt {
    at: u32,
    len: u32,
    hash: u64,
}

// SAFETY: plain numbers.
unsafe impl rerail_blocks::Pod for DeclAt {}

/// A fact relation of a builder's parts: its rows' ends in `Parts::ends` (from `cells`, its first cell in
/// `Parts::cells`), and its hash.
#[repr(C)]
#[derive(Clone, Copy)]
struct FactAt {
    ends: u32,
    rows: u32,
    cells: u32,
    _pad: u32,
    hash: u64,
}

// SAFETY: plain numbers.
unsafe impl rerail_blocks::Pod for FactAt {}

/// The parts an image holds besides its rules, by ids, as a builder states them (a delta's are what it replaces), in
/// blocks: each declaration and fact relation found by its atom, its columns and cells in arenas a replacement
/// appends to. `finish` writes them into the image's sections, sorted by atom.
#[derive(Clone, Default)]
pub struct Parts {
    decls: BlockHash<u32, DeclAt>,
    cols: Vector<[u32; 2]>,
    pub decls_gone: rerail_blocks::maps::BlockSet<u32>,
    /// By (atom | `ABOUT_BIT` for an `.about` relation).
    facts: BlockHash<u32, FactAt>,
    ends: Vector<u32>,
    cells: Vector<Cell>,
    pub facts_gone: rerail_blocks::maps::BlockSet<u32>,
    pub inputs: Option<(Vector<u32>, u64)>,
    pub outputs: Option<(Vector<u32>, u64)>,
    /// Whether the image's `.about` rows (those in `facts`) are all the program's.
    pub owns_about: bool,
}

impl Parts {
    /// Declaration `a`'s columns and hash.
    pub fn decl(&self, a: u32) -> Option<(&[[u32; 2]], u64)> {
        self.decls.get(&a).map(|d| (&self.cols[d.at as usize..(d.at + d.len) as usize], d.hash))
    }

    pub fn has_decl(&self, a: u32) -> bool {
        self.decls.contains_key(&a)
    }

    pub fn set_decl(&mut self, a: u32, cols: &[[u32; 2]], hash: u64) {
        let at = u32::try_from(self.cols.len()).expect("4G columns");
        self.cols.extend_from_slice(cols);
        self.decls.insert(a, DeclAt { at, len: cols.len() as u32, hash });
    }

    pub fn remove_decl(&mut self, a: u32) {
        self.decls.remove(&a);
    }

    pub fn decl_count(&self) -> usize {
        self.decls.len()
    }

    /// The declared atoms, in order.
    pub fn decl_atoms(&self) -> Vector<u32> {
        let mut v: Vector<u32> = self.decls.keys_iter().collect();
        v.sort_unstable();
        v
    }

    /// Fact relation `k`'s rows and hash.
    pub fn fact(&self, k: u32) -> Option<(RowsRef<'_>, u64)> {
        self.facts.get(&k).map(|f| {
            let ends = &self.ends[f.ends as usize..(f.ends + f.rows) as usize];
            let n = ends.last().map_or(0, |&e| e as usize);
            (RowsRef { ends, cells: &self.cells[f.cells as usize..f.cells as usize + n] }, f.hash)
        })
    }

    pub fn has_fact(&self, k: u32) -> bool {
        self.facts.contains_key(&k)
    }

    pub fn set_fact(&mut self, k: u32, rows: RowsRef<'_>, hash: u64) {
        let ends = u32::try_from(self.ends.len()).expect("4G rows");
        let cells = u32::try_from(self.cells.len()).expect("4G cells");
        self.ends.extend_from_slice(rows.ends);
        self.cells.extend_from_slice(rows.cells);
        self.facts.insert(k, FactAt { ends, rows: rows.len() as u32, cells, _pad: 0, hash });
    }

    pub fn remove_fact(&mut self, k: u32) {
        self.facts.remove(&k);
    }

    /// The fact relations' keys, in order.
    pub fn fact_keys(&self) -> Vector<u32> {
        let mut v: Vector<u32> = self.facts.keys_iter().collect();
        v.sort_unstable();
        v
    }

    pub fn retain_facts(&mut self, mut keep: impl FnMut(u32) -> bool) {
        self.facts.retain(|k, _| keep(*k));
    }
}

#[derive(Clone)]
pub struct Encoder<'p> {
    /// The version whose ids this image's extend (a delta's parent), if any.
    parent: Option<&'p Version>,
    atom_base: u32,
    rel_base: u32,
    rule_base: u32,
    strings: Strings,
    /// This image's relations: the atom naming each.
    rels: Vector<u32>,
    /// The relations this image numbers, by their atoms.
    rel_of: BlockHash<u32, u32>,
    /// The parent's atoms this image makes relations, and their relation numbers.
    promoted: Vector<[u32; 2]>,
    kind: Extents<u8>,
    size: Extents<u32>,
    val: Extents<u32>,
    ints: Vector<i64>,
    rule_node: Vector<u32>,
    rule_vars: Vector<u32>,
    vars: Vector<u32>,
    rule_hash: Vector<u64>,
    /// How many big integers there were when the open rule began: what `abandon` keeps.
    ints_at_rule: usize,
    heads: Vector<[u32; 2]>,
    /// (relation, read entry), with the entries' rules and nodes beside.
    reads: Vector<[u32; 2]>,
    read_rules: Vector<u32>,
    read_nodes: Vector<u32>,
    /// Each rule's source (`R::From`), `NO_RULE` for none.
    from: Vector<u32>,
    /// The rule being written: its variables' atoms by slot, and the slot of each atom.
    slots: BlockHash<u32, u32>,
    slot_names: Vector<u32>,
    /// The parent's atoms' text hashes, as rules' hashes need them.
    parent_hashes: BlockHash<u32, u64>,
}

impl<'p> Encoder<'p> {
    /// An encoder for a base (no parent), or a delta over `parent`.
    pub fn new(parent: Option<&'p Version>) -> Encoder<'p> {
        let (atom_base, rel_base, rule_base) = parent.map_or((0, 0, 0), |p| (p.atom_end(), p.rel_end(), p.rule_end()));
        Encoder {
            parent,
            atom_base,
            rel_base,
            rule_base,
            strings: Strings::default(),
            rels: rerail_blocks::vector::Vector::new(),
            rel_of: BlockHash::default(),
            promoted: Vector::new(),
            kind: Extents::new(12),
            size: Extents::new(12),
            val: Extents::new(12),
            ints: rerail_blocks::vector::Vector::new(),
            rule_node: rerail_blocks::vector::Vector::new(),
            rule_vars: rerail_blocks::vector![0],
            vars: rerail_blocks::vector::Vector::new(),
            rule_hash: rerail_blocks::vector::Vector::new(),
            ints_at_rule: 0,
            heads: Vector::new(),
            reads: Vector::new(),
            read_rules: rerail_blocks::vector::Vector::new(),
            read_nodes: rerail_blocks::vector::Vector::new(),
            from: rerail_blocks::vector::Vector::new(),
            slots: BlockHash::default(),
            slot_names: rerail_blocks::vector::Vector::new(),
            parent_hashes: BlockHash::default(),
        }
    }

    /// A base holding `v`'s atoms and relations under the same ids (a fold): every text and relation of `v`'s
    /// stack, in id order.
    pub fn with_ids_of(v: &Version) -> Encoder<'static> {
        let mut e = Encoder::new(None);
        for a in 0..v.atom_end() {
            let got = e.strings.intern(v.text(AtomId(a)));
            debug_assert_eq!(got, a, "a text twice in a stack");
        }
        for r in 0..v.rel_end() {
            let a = v.rel_atom(RelId(r)).0;
            e.rel_of.insert(a, r);
            e.rels.push(a);
        }
        e
    }

    /// The rules written so far.
    pub fn rules(&self) -> u32 {
        self.rule_node.len() as u32
    }
    pub fn rule_base(&self) -> u32 {
        self.rule_base
    }
    /// The hash of local rule `k`.
    pub fn rule_hash_of(&self, k: u32) -> u64 {
        self.rule_hash[k as usize]
    }

    // -- names --

    /// The atom of `text`: the parent's, or this image's (added if new).
    pub fn atom(&mut self, text: &str) -> u32 {
        if let Some(p) = self.parent
            && let Some(a) = p.find(text)
        {
            return a.0;
        }
        self.atom_base + self.strings.intern(text)
    }
    /// The atom of `text`, where the parent or this image has one; none is added.
    pub fn find(&self, text: &str) -> Option<u32> {
        if let Some(p) = self.parent
            && let Some(a) = p.find(text)
        {
            return Some(a.0);
        }
        self.strings.find(text).map(|i| self.atom_base + i)
    }
    pub fn text(&self, a: u32) -> &str {
        if a >= self.atom_base { self.strings.text(a - self.atom_base) } else { self.parent.expect("an atom below the base has a parent").text(AtomId(a)) }
    }
    fn text_hash(&mut self, a: u32) -> u64 {
        if a >= self.atom_base {
            return self.strings.hash_of(a - self.atom_base);
        }
        let p = self.parent.expect("an atom below the base has a parent");
        *self.parent_hashes.entry(a).or_insert_with(|| strings::hash(p.text(AtomId(a)).as_bytes()))
    }
    /// The relation named by atom `a`: the parent's, or numbered here.
    pub fn rel_of_atom(&mut self, a: u32) -> u32 {
        if let Some(&r) = self.rel_of.get(&a) {
            return r;
        }
        if a < self.atom_base
            && let Some(r) = self.parent.and_then(|p| p.rel_of(AtomId(a)))
        {
            return r.0;
        }
        let r = self.rel_base + self.rels.len() as u32;
        self.rels.push(a);
        self.rel_of.insert(a, r);
        if a < self.atom_base {
            self.promoted.push([a, r]);
        }
        r
    }
    /// The atom of `name`, made a relation.
    pub fn rel_atom_of(&mut self, name: &str) -> u32 {
        let a = self.atom(name);
        self.rel_of_atom(a);
        a
    }
    pub fn rel(&mut self, name: &str) -> u32 {
        let a = self.atom(name);
        self.rel_of_atom(a)
    }
    fn rel_atom(&self, r: u32) -> u32 {
        if r >= self.rel_base { self.rels[(r - self.rel_base) as usize] } else { self.parent.expect("a relation below the base has a parent").rel_atom(RelId(r)).0 }
    }

    // -- nodes --

    pub fn open(&mut self, k: u8, val: u32) -> usize {
        let at = self.kind.len();
        self.kind.push(k);
        self.size.push(0);
        self.val.push(val);
        at
    }
    pub fn close(&mut self, at: usize) {
        self.size.set(at, (self.kind.len() - at) as u32);
    }
    pub fn leaf(&mut self, k: u8, val: u32) {
        self.kind.push(k);
        self.size.push(1);
        self.val.push(val);
    }
    /// The slot of the variable named by atom `a` in the rule being written.
    pub fn slot_of(&mut self, a: u32) -> u32 {
        if let Some(&s) = self.slots.get(&a) {
            return s;
        }
        let s = self.slot_names.len() as u32;
        self.slot_names.push(a);
        self.slots.insert(a, s);
        s
    }
    pub fn var(&mut self, name: &str) {
        let a = self.atom(name);
        let s = self.slot_of(a);
        self.leaf(kind::VAR, s);
    }
    pub fn int(&mut self, i: i64) {
        if (0..1 << 31).contains(&i) {
            self.leaf(kind::INT, i as u32);
        } else {
            let at = self.ints.len() as u32;
            self.ints.push(i);
            self.leaf(kind::INT | kind::BIG, at);
        }
    }

    /// Starts a rule: its node.
    pub fn begin_rule(&mut self) -> usize {
        self.slots.clear();
        self.slot_names.clear();
        self.ints_at_rule = self.ints.len();
        let at = self.open(kind::RULE, 0);
        self.rule_node.push(at as u32);
        at
    }
    /// Ends the rule begun at `at`: its size, variables, index entries and hash. Its local index.
    pub fn end_rule(&mut self, at: usize) -> u32 {
        self.close(at);
        let k = self.rule_node.len() as u32 - 1;
        self.vars.extend_from_slice(&self.slot_names);
        self.rule_vars.push(self.vars.len() as u32);
        self.index_rule(k, at, self.kind.len());
        let h = self.hash_closed(at);
        self.rule_hash.push(h);
        self.from.push(super::NO_RULE);
        k
    }
    /// Records local rule `k`'s head and reads (nodes `at..end`) in the relation index.
    fn index_rule(&mut self, k: u32, at: usize, end: usize) {
        let head_at = at + 1;
        let head_end = head_at + self.size.get(head_at) as usize;
        for n in at..end {
            let (kd, val) = (self.kind.get(n), self.val.get(n));
            if kd & kind::MASK == kind::ATOM {
                if n == head_at {
                    self.heads.push([val, k]);
                } else if n >= head_end {
                    let e = self.read_rules.len() as u32;
                    self.reads.push([val, e]);
                    self.read_rules.push(k);
                    self.read_nodes.push(n as u32);
                }
            }
        }
    }
    /// Copies rules `ks` of `src` (consecutive locals, so one run of nodes) whole, when `src`'s ids are this image's:
    /// their nodes as one range, each rule's variables and hash as `src` holds them (the hash is of texts and values,
    /// so the same over the same ids), a big integer to this image's table. The local index of the first; 0276 §3's
    /// range copy, as 0266 §3 copies a stratum.
    pub fn copy_run(&mut self, src: &super::Image, ks: std::ops::Range<u32>) -> u32 {
        let first = self.rule_node.len() as u32;
        if ks.is_empty() {
            return first;
        }
        let (a, b) = {
            let last = ks.end - 1;
            let at = src.rule_node(last);
            (src.rule_node(ks.start), at + src.node(at).1)
        };
        let base = self.kind.len() as u32;
        let (kinds, sizes, vals) = (&src.u8s(super::R::Kind)[a as usize..b as usize], &src.u32s(super::R::Size)[a as usize..b as usize], &src.u32s(super::R::Val)[a as usize..b as usize]);
        self.kind.extend(kinds);
        self.size.extend(sizes);
        self.val.extend(vals);
        // A big integer's value indexes `src`'s table: written to this one's.
        for (i, &kd) in kinds.iter().enumerate() {
            if kd & kind::MASK == kind::INT && kd & kind::BIG != 0 {
                self.ints.push(src.big(vals[i]));
                self.val.set(base as usize + i, self.ints.len() as u32 - 1);
            }
        }
        for k in ks {
            let at = (src.rule_node(k) - a + base) as usize;
            let end = at + src.node(src.rule_node(k)).1 as usize;
            let local = self.rule_node.len() as u32;
            self.rule_node.push(at as u32);
            self.vars.extend_from_slice(src.rule_vars(k));
            self.rule_vars.push(self.vars.len() as u32);
            self.index_rule(local, at, end);
            self.rule_hash.push(src.rule_hash(k));
            self.from.push(super::NO_RULE);
        }
        first
    }
    /// The hash of the rule begun at `at`, its nodes closed (`close`) and not yet ended: what `end_rule` records,
    /// so a writer can leave out a rule it wrote before (`Builder::close_rule_once`).
    pub fn hash_closed(&mut self, at: usize) -> u64 {
        let mut h = hasher();
        for n in at..self.kind.len() {
            let (kd, size, val) = (self.kind.get(n), self.size.get(n), self.val.get(n));
            h.write_u8(kd);
            h.write_u32(size);
            match kd & kind::MASK {
                kind::ATOM => {
                    let a = self.rel_atom(val);
                    h.write_u64(self.text_hash(a));
                }
                kind::INT if kd & kind::BIG != 0 => h.write_i64(self.ints[val as usize]),
                _ if kind::val_is_atom(kd) => h.write_u64(self.text_hash(val)),
                _ => h.write_u32(val),
            }
        }
        for s in 0..self.slot_names.len() {
            let a = self.slot_names[s];
            h.write_u64(self.text_hash(a));
        }
        h.finish()
    }
    /// Records that local rule `k` came from rule `src` of the input (§9).
    pub fn set_from(&mut self, k: u32, src: super::RuleId) {
        self.from[k as usize] = src.0;
    }

    /// Drops the rule begun at `at` (a rule that failed to encode).
    pub fn abandon(&mut self, at: usize) {
        self.kind.truncate(at);
        self.size.truncate(at);
        self.val.truncate(at);
        self.ints.truncate(self.ints_at_rule);
        self.rule_node.pop();
    }

    // -- nodes copied from an image --

    /// Copies nodes `range` of `src` (in a rule whose variables' atoms are `src_vars`), each id mapped: unchanged
    /// when `src`'s ids are this image's (`same`), else by text, through `from`.
    pub fn copy_nodes(&mut self, from: &Version, src: &super::Image, range: std::ops::Range<u32>, src_vars: &[u32], same: bool) {
        for n in range {
            let (kd, size, _) = src.node(n);
            let v = self.map_val(from, src, n, src_vars, same);
            self.kind.push(kd);
            self.size.push(size);
            self.val.push(v);
        }
    }
    /// Node `n` of `src`'s value, mapped into this image's ids (`copy_nodes`).
    pub fn map_val(&mut self, from: &Version, src: &super::Image, n: u32, src_vars: &[u32], same: bool) -> u32 {
        let (kd, _, val) = src.node(n);
        match kd & kind::MASK {
            kind::ATOM if same => val,
            kind::ATOM => {
                let t = from.rel_name(RelId(val));
                self.rel(t)
            }
            kind::VAR | kind::AGG => {
                let a = src_vars[val as usize];
                let a = if same { a } else { self.atom(from.text(AtomId(a))) };
                self.slot_of(a)
            }
            kind::INT if kd & kind::BIG != 0 => {
                self.ints.push(src.big(val));
                self.ints.len() as u32 - 1
            }
            _ if kind::val_is_atom(kd) && !same => self.atom(from.text(AtomId(val))),
            _ => val,
        }
    }

    /// The image: the rules written, and `parts`; a delta when it has a parent, removing `removed` (sorted) of its
    /// rules. Written once, in place (`Layout`, decision 0464), from the encoder as it is: nothing it holds is copied
    /// or consumed first, so a writer can write its image and go on. A base states every part, what `parts` leaves
    /// unstated empty: no inputs, no outputs, no `.about` rows.
    pub fn finish(&self, parts: &Parts, removed: &[u32]) -> Result<super::Image, String> {
        let delta = self.parent.is_some();
        if !delta && !removed.is_empty() {
            return Err(super::bad("a base removes nothing"));
        }
        let (empty_in, empty_out) = ((Vector::new(), names_hash(0, std::iter::empty())), (Vector::new(), names_hash(1, std::iter::empty())));
        let inputs = parts.inputs.as_ref().or((!delta).then_some(&empty_in));
        let outputs = parts.outputs.as_ref().or((!delta).then_some(&empty_out));
        // A base that states no `.about` rows has none (`set_about` of none).
        let owns_about = parts.owns_about || !delta;
        let keep_fact = |k: u32| parts.owns_about || delta || k & ABOUT_BIT == 0;
        let fact_keys: Vector<u32> = parts.fact_keys().into_iter().filter(|&k| keep_fact(k)).collect();
        let facts: Vec<(u32, RowsRef<'_>, u64)> = fact_keys.iter().map(|&k| {
            let (rows, hash) = parts.fact(k).expect("a fact relation of the parts");
            (k, rows, hash)
        }).collect();
        let decls: Vector<u32> = parts.decl_atoms();
        let decl = |a: u32| parts.decl(a).expect("a declaration of the parts");
        let gone_decls: Vector<u32> = { let mut v: Vector<u32> = parts.decls_gone.keys_iter().collect(); v.sort_unstable(); v };
        let gone_facts: Vector<u32> = { let mut v: Vector<u32> = parts.facts_gone.keys_iter().collect(); v.sort_unstable(); v };
        let (str_bytes, str_ends, table) = self.strings.parts();
        let atoms = self.strings.len();
        let rules = self.rule_hash.len();
        let nodes: usize = self.kind.extents().map(<[u8]>::len).sum();
        let mut promoted: Vector<u64> = self.promoted.iter().map(|&[a, r]| (a as u64) << 32 | r as u64).collect();
        promoted.sort_unstable();
        // Indexes: rules by head, and reads; dense over the relations in a base, keyed in a delta. Each is
        // (keys, starts, the pairs' values in place order).
        let index = |pairs: &[[u32; 2]]| -> (Vector<u32>, Vector<u32>, Vector<u32>) {
            if delta {
                return csr::keyed(pairs);
            }
            let n = self.rels.len();
            let mut starts = rerail_blocks::vector![0u32; n + 1];
            for &[r, _] in pairs {
                starts[r as usize + 1] += 1;
            }
            for i in 0..n {
                starts[i + 1] += starts[i];
            }
            let mut at = starts.clone();
            let mut values = rerail_blocks::vector![0u32; pairs.len()];
            for &[r, x] in pairs {
                values[at[r as usize] as usize] = x;
                at[r as usize] += 1;
            }
            (rerail_blocks::vector::Vector::new(), starts, values)
        };
        let (head_keys, head_starts, head_rules) = index(&self.heads);
        let (read_keys, read_starts, read_entries) = index(&self.reads);
        let has_from = self.from.iter().any(|&f| f != super::NO_RULE);
        let mut from_index: Vector<u64> = if has_from { self.from.iter().enumerate().filter(|(_, f)| **f != super::NO_RULE).map(|(k, &f)| (f as u64) << 32 | k as u64).collect() } else { rerail_blocks::vector::Vector::new() };
        from_index.sort_unstable();
        let rows: usize = facts.iter().map(|(_, f, _)| f.len()).sum();
        let cells: usize = facts.iter().map(|(_, f, _)| f.cells().len()).sum();
        let cols: usize = decls.iter().map(|&a| decl(a).0.len()).sum();

        let mut l = super::Layout::default();
        for (r, n) in [
            (R::StrBytes, str_bytes.len()),
            (R::StrEnds, str_ends.len()),
            (R::StrBuckets, table.buckets.len()),
            (R::StrOver, table.over.len()),
            (R::Rels, self.rels.len()),
            (R::AtomRel, atoms),
            (R::Promoted, promoted.len()),
            (R::Kind, nodes),
            (R::Size, nodes),
            (R::Val, nodes),
            (R::Ints, self.ints.len()),
            (R::RuleNode, self.rule_node.len()),
            (R::RuleVars, self.rule_vars.len()),
            (R::RuleHash, rules),
            (R::Vars, self.vars.len()),
            (R::DeclName, decls.len()),
            (R::DeclHash, decls.len()),
            (R::DeclCols, decls.len() + 1),
            (R::Cols, 2 * cols),
            (R::DeclGone, gone_decls.len()),
            (R::FactRel, facts.len()),
            (R::FactHash, facts.len()),
            (R::FactRows, facts.len() + 1),
            (R::RowCells, rows + 1),
            (R::CellKind, cells),
            (R::CellVal, cells),
            (R::FactGone, gone_facts.len()),
            (R::Inputs, inputs.map_or(0, |i| i.0.len())),
            (R::Outputs, outputs.map_or(0, |o| o.0.len())),
            (R::HeadKeys, head_keys.len()),
            (R::HeadStart, head_starts.len()),
            (R::HeadRules, head_rules.len()),
            (R::ReadKeys, read_keys.len()),
            (R::ReadStart, read_starts.len()),
            (R::ReadRule, read_entries.len()),
            (R::ReadNode, read_entries.len()),
            (R::Removed, removed.len()),
            (R::From, if has_from { self.from.len() } else { 0 }),
            (R::FromIndex, from_index.len()),
            (R::Meta, meta::WORDS),
        ] {
            l.set(r, n);
        }
        let mut w = l.place();
        // Strings, and each of this image's atoms' relation.
        w.u8s(R::StrBytes).copy_from_slice(str_bytes);
        w.u32s(R::StrEnds).copy_from_slice(str_ends);
        w.buckets(R::StrBuckets).copy_from_slice(&table.buckets);
        w.buckets(R::StrOver).copy_from_slice(&table.over);
        w.u32s(R::Rels).copy_from_slice(&self.rels);
        let atom_rel = w.u32s(R::AtomRel);
        for (k, &a) in self.rels.iter().enumerate() {
            if a >= self.atom_base {
                atom_rel[(a - self.atom_base) as usize] = self.rel_base + k as u32 + 1;
            }
        }
        w.u64s(R::Promoted).copy_from_slice(&promoted);
        // Nodes, one region a column, each extent in turn.
        let mut at = 0;
        for e in self.kind.extents() {
            w.u8s(R::Kind)[at..at + e.len()].copy_from_slice(e);
            at += e.len();
        }
        for (column, r) in [(&self.size, R::Size), (&self.val, R::Val)] {
            let mut at = 0;
            for e in column.extents() {
                w.u32s(r)[at..at + e.len()].copy_from_slice(e);
                at += e.len();
            }
        }
        for (slot, &i) in w.u64s(R::Ints).iter_mut().zip(&self.ints) {
            *slot = i as u64;
        }
        w.u32s(R::RuleNode).copy_from_slice(&self.rule_node);
        w.u32s(R::RuleVars).copy_from_slice(&self.rule_vars);
        w.u64s(R::RuleHash).copy_from_slice(&self.rule_hash);
        w.u32s(R::Vars).copy_from_slice(&self.vars);
        // Parts, sorted by atom so they are found by search.
        let mut own_sum = self.rule_hash.iter().fold(0u64, |a, &h| a.wrapping_add(h));
        for (slot, &a) in w.u32s(R::DeclName).iter_mut().zip(&decls) {
            *slot = a;
            own_sum = own_sum.wrapping_add(decl(a).1);
        }
        for (slot, &a) in w.u64s(R::DeclHash).iter_mut().zip(&decls) {
            *slot = decl(a).1;
        }
        {
            let (mut end, ends) = (0u32, w.u32s(R::DeclCols));
            for (k, &a) in decls.iter().enumerate() {
                end += decl(a).0.len() as u32;
                ends[k + 1] = end;
            }
        }
        for (slot, x) in w.u32s(R::Cols).iter_mut().zip(decls.iter().flat_map(|&a| decl(a).0.iter().flat_map(|&[n, t]| [n, t]))) {
            *slot = x;
        }
        for (slot, &a) in w.u32s(R::DeclGone).iter_mut().zip(&gone_decls) {
            *slot = a;
        }
        for (slot, &(key, _, hash)) in w.u32s(R::FactRel).iter_mut().zip(&facts) {
            *slot = key;
            own_sum = own_sum.wrapping_add(hash);
        }
        for (slot, (_, _, hash)) in w.u64s(R::FactHash).iter_mut().zip(&facts) {
            *slot = *hash;
        }
        {
            let fact_rows = w.u32s(R::FactRows);
            let mut n = 0u32;
            for (k, (_, f, _)) in facts.iter().enumerate() {
                n += f.len() as u32;
                fact_rows[k + 1] = n;
            }
            let row_cells = w.u32s(R::RowCells);
            let mut n = 0u32;
            for (k, row) in facts.iter().flat_map(|(_, f, _)| f.iter()).enumerate() {
                n += row.len() as u32;
                row_cells[k + 1] = n;
            }
        }
        let each_cell = || facts.iter().flat_map(|(_, f, _)| f.cells().iter());
        for (slot, c) in w.u8s(R::CellKind).iter_mut().zip(each_cell()) {
            *slot = match *c {
                Cell::Int(_) => 0,
                Cell::Str(_) => 1,
                Cell::Branded(b) => 2 + super::brand_code(b.class),
            };
        }
        for (slot, c) in w.u64s(R::CellVal).iter_mut().zip(each_cell()) {
            *slot = match *c {
                Cell::Int(i) => i as u64,
                Cell::Str(a) => a.0 as u64,
                Cell::Branded(b) => u64::from(b.atom),
            };
        }
        for (slot, &k) in w.u32s(R::FactGone).iter_mut().zip(&gone_facts) {
            *slot = k;
        }
        let mut owns = 0u64;
        let (mut ih, mut oh) = (0, 0);
        if let Some((names, h)) = inputs {
            owns |= 1;
            ih = *h;
            own_sum = own_sum.wrapping_add(*h);
            w.u32s(R::Inputs).copy_from_slice(names);
        }
        if let Some((names, h)) = outputs {
            owns |= 2;
            oh = *h;
            own_sum = own_sum.wrapping_add(*h);
            w.u32s(R::Outputs).copy_from_slice(names);
        }
        if owns_about {
            owns |= 4;
        }
        w.u32s(R::HeadKeys).copy_from_slice(&head_keys);
        w.u32s(R::HeadStart).copy_from_slice(&head_starts);
        w.u32s(R::HeadRules).copy_from_slice(&head_rules);
        w.u32s(R::ReadKeys).copy_from_slice(&read_keys);
        w.u32s(R::ReadStart).copy_from_slice(&read_starts);
        for (slot, &e) in w.u32s(R::ReadRule).iter_mut().zip(&read_entries) {
            *slot = self.read_rules[e as usize];
        }
        for (slot, &e) in w.u32s(R::ReadNode).iter_mut().zip(&read_entries) {
            *slot = self.read_nodes[e as usize];
        }
        w.u32s(R::Removed).copy_from_slice(removed);
        if has_from {
            w.u32s(R::From).copy_from_slice(&self.from);
            w.u64s(R::FromIndex).copy_from_slice(&from_index);
        }
        let m = w.u64s(R::Meta);
        m[meta::KIND] = delta as u64;
        m[meta::RULE_BASE] = self.rule_base as u64;
        m[meta::ATOM_BASE] = self.atom_base as u64;
        m[meta::REL_BASE] = self.rel_base as u64;
        m[meta::HASH_LEVEL] = table.level as u64;
        m[meta::HASH_SPLIT] = table.split as u64;
        m[meta::HASH_COUNT] = table.count;
        m[meta::OWNS] = owns;
        m[meta::INPUTS_HASH] = ih;
        m[meta::OUTPUTS_HASH] = oh;
        m[meta::OWN_SUM] = own_sum;
        w.finish()
    }
}
