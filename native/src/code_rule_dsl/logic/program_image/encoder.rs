//! Writes one image (decision 0173): its strings, relations, rules' nodes, parts and indexes. A delta's encoder
//! numbers what it adds after its parent's id spaces, and finds what its parent holds by its parent's tables, so a
//! delta holds only what it adds. Hashes are of texts, never of ids, so a rule or part hashes the same in any image.
use super::{ABOUT_BIT, AtomId, Cell, R, RelId, Version, bad, csr, extents::Extents, kind, meta, strings::{self, Strings}};
use std::collections::HashMap;
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

/// A fact relation's hash: its name, whether it is of the `.about` rows, and its rows.
pub fn facts_hash<'a, Row: Iterator<Item = CellText<'a>>>(rel: &str, about: bool, rows: impl Iterator<Item = Row>) -> u64 {
    let mut h = hasher();
    h.write_u8(2);
    put_text(&mut h, rel);
    h.write_u8(about as u8);
    for row in rows {
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
    h.finish()
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

/// A part, by ids, with its hash.
#[derive(Clone)]
pub struct Decl {
    pub cols: Vec<(u32, u32)>,
    pub hash: u64,
}
#[derive(Clone)]
pub struct FactRel {
    pub rows: Vec<Vec<Cell>>,
    pub hash: u64,
}

/// The parts an image holds besides its rules, by ids. A delta's are what it replaces.
#[derive(Clone, Default)]
pub struct Parts {
    pub decls: std::collections::BTreeMap<u32, Decl>,
    pub decls_gone: std::collections::BTreeSet<u32>,
    /// By (atom | `ABOUT_BIT` for an `.about` relation).
    pub facts: std::collections::BTreeMap<u32, FactRel>,
    pub facts_gone: std::collections::BTreeSet<u32>,
    pub inputs: Option<(Vec<u32>, u64)>,
    pub outputs: Option<(Vec<u32>, u64)>,
    /// Whether the image's `.about` rows (those in `facts`) are all the program's.
    pub owns_about: bool,
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
    rels: Vec<u32>,
    /// The relations this image numbers, by their atoms.
    rel_of: HashMap<u32, u32>,
    /// The parent's atoms this image makes relations, and their relation numbers.
    promoted: Vec<(u32, u32)>,
    kind: Extents<u8>,
    size: Extents<u32>,
    val: Extents<u32>,
    ints: Vec<i64>,
    rule_node: Vec<u32>,
    rule_vars: Vec<u32>,
    vars: Vec<u32>,
    rule_hash: Vec<u64>,
    /// How many big integers there were when the open rule began: what `abandon` keeps.
    ints_at_rule: usize,
    heads: Vec<(u32, u32)>,
    /// (relation, read entry), with the entries' rules and nodes beside.
    reads: Vec<(u32, u32)>,
    read_rules: Vec<u32>,
    read_nodes: Vec<u32>,
    /// Each rule's source (`R::From`), `NO_RULE` for none.
    from: Vec<u32>,
    /// The rule being written: its variables' atoms by slot, and the slot of each atom.
    slots: HashMap<u32, u32>,
    slot_names: Vec<u32>,
    /// The parent's atoms' text hashes, as rules' hashes need them.
    parent_hashes: HashMap<u32, u64>,
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
            rels: Vec::new(),
            rel_of: HashMap::new(),
            promoted: Vec::new(),
            kind: Extents::new(12),
            size: Extents::new(12),
            val: Extents::new(12),
            ints: Vec::new(),
            rule_node: Vec::new(),
            rule_vars: vec![0],
            vars: Vec::new(),
            rule_hash: Vec::new(),
            ints_at_rule: 0,
            heads: Vec::new(),
            reads: Vec::new(),
            read_rules: Vec::new(),
            read_nodes: Vec::new(),
            from: Vec::new(),
            slots: HashMap::new(),
            slot_names: Vec::new(),
            parent_hashes: HashMap::new(),
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
            self.promoted.push((a, r));
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
                    self.heads.push((val, k));
                } else if n >= head_end {
                    let e = self.read_rules.len() as u32;
                    self.reads.push((val, e));
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
    /// rules. Written once, in place (`Layout`, decision 0480), from the encoder as it is: nothing it holds is copied
    /// or consumed first, so a writer can write its image and go on. A base states every part, what `parts` leaves
    /// unstated empty: no inputs, no outputs, no `.about` rows.
    pub fn finish(&self, parts: &Parts, removed: &[u32]) -> Result<super::Image, String> {
        let delta = self.parent.is_some();
        if !delta && !removed.is_empty() {
            return Err(super::bad("a base removes nothing"));
        }
        let (empty_in, empty_out) = ((Vec::new(), names_hash(0, std::iter::empty())), (Vec::new(), names_hash(1, std::iter::empty())));
        let inputs = parts.inputs.as_ref().or((!delta).then_some(&empty_in));
        let outputs = parts.outputs.as_ref().or((!delta).then_some(&empty_out));
        // A base that states no `.about` rows has none (`set_about` of none).
        let owns_about = parts.owns_about || !delta;
        let keep_fact = |k: u32| parts.owns_about || delta || k & ABOUT_BIT == 0;
        let facts: Vec<(u32, &FactRel)> = parts.facts.iter().filter(|(k, _)| keep_fact(**k)).map(|(k, f)| (*k, f)).collect();
        let (str_bytes, str_ends, table) = self.strings.parts();
        let atoms = self.strings.len();
        let rules = self.rule_hash.len();
        let nodes: usize = self.kind.extents().map(<[u8]>::len).sum();
        let mut promoted: Vec<u64> = self.promoted.iter().map(|&(a, r)| (a as u64) << 32 | r as u64).collect();
        promoted.sort_unstable();
        // Indexes: rules by head, and reads; dense over the relations in a base, keyed in a delta. Each is
        // (keys, starts, the pairs' values in place order).
        let index = |pairs: &[(u32, u32)]| -> (Vec<u32>, Vec<u32>, Vec<u32>) {
            if delta {
                return csr::keyed(pairs.to_vec());
            }
            let n = self.rels.len();
            let mut starts = vec![0u32; n + 1];
            for &(r, _) in pairs {
                starts[r as usize + 1] += 1;
            }
            for i in 0..n {
                starts[i + 1] += starts[i];
            }
            let mut at = starts.clone();
            let mut values = vec![0u32; pairs.len()];
            for &(r, x) in pairs {
                values[at[r as usize] as usize] = x;
                at[r as usize] += 1;
            }
            (Vec::new(), starts, values)
        };
        let (head_keys, head_starts, head_rules) = index(&self.heads);
        let (read_keys, read_starts, read_entries) = index(&self.reads);
        let has_from = self.from.iter().any(|&f| f != super::NO_RULE);
        let mut from_index: Vec<u64> = if has_from { self.from.iter().enumerate().filter(|(_, f)| **f != super::NO_RULE).map(|(k, &f)| (f as u64) << 32 | k as u64).collect() } else { Vec::new() };
        from_index.sort_unstable();
        let rows: usize = facts.iter().map(|(_, f)| f.rows.len()).sum();
        let cells: usize = facts.iter().flat_map(|(_, f)| &f.rows).map(|r| r.len()).sum();
        let cols: usize = parts.decls.values().map(|d| d.cols.len()).sum();

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
            (R::DeclName, parts.decls.len()),
            (R::DeclHash, parts.decls.len()),
            (R::DeclCols, parts.decls.len() + 1),
            (R::Cols, 2 * cols),
            (R::DeclGone, parts.decls_gone.len()),
            (R::FactRel, facts.len()),
            (R::FactHash, facts.len()),
            (R::FactRows, facts.len() + 1),
            (R::RowCells, rows + 1),
            (R::CellKind, cells),
            (R::CellVal, cells),
            (R::FactGone, parts.facts_gone.len()),
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
        for (slot, (&a, d)) in w.u32s(R::DeclName).iter_mut().zip(&parts.decls) {
            *slot = a;
            own_sum = own_sum.wrapping_add(d.hash);
        }
        for (slot, d) in w.u64s(R::DeclHash).iter_mut().zip(parts.decls.values()) {
            *slot = d.hash;
        }
        {
            let (mut end, ends) = (0u32, w.u32s(R::DeclCols));
            for (k, d) in parts.decls.values().enumerate() {
                end += d.cols.len() as u32;
                ends[k + 1] = end;
            }
        }
        for (slot, x) in w.u32s(R::Cols).iter_mut().zip(parts.decls.values().flat_map(|d| d.cols.iter().flat_map(|&(n, t)| [n, t]))) {
            *slot = x;
        }
        for (slot, &a) in w.u32s(R::DeclGone).iter_mut().zip(&parts.decls_gone) {
            *slot = a;
        }
        for (slot, &(key, f)) in w.u32s(R::FactRel).iter_mut().zip(&facts) {
            *slot = key;
            own_sum = own_sum.wrapping_add(f.hash);
        }
        for (slot, (_, f)) in w.u64s(R::FactHash).iter_mut().zip(&facts) {
            *slot = f.hash;
        }
        {
            let fact_rows = w.u32s(R::FactRows);
            let mut n = 0u32;
            for (k, (_, f)) in facts.iter().enumerate() {
                n += f.rows.len() as u32;
                fact_rows[k + 1] = n;
            }
            let row_cells = w.u32s(R::RowCells);
            let mut n = 0u32;
            for (k, row) in facts.iter().flat_map(|(_, f)| &f.rows).enumerate() {
                n += row.len() as u32;
                row_cells[k + 1] = n;
            }
        }
        let each_cell = || facts.iter().flat_map(|(_, f)| &f.rows).flatten();
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
        for (slot, &k) in w.u32s(R::FactGone).iter_mut().zip(&parts.facts_gone) {
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
