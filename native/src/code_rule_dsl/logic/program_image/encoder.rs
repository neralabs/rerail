//! Writes one image (decision 0173): its strings, relations, rules' nodes, parts and indexes. A delta's encoder
//! numbers what it adds after its parent's id spaces, and finds what its parent holds by its parent's tables, so a
//! delta holds only what it adds. Hashes are of texts, never of ids, so a rule or part hashes the same in any image.
use super::{AtomId, Cell, R, RelId, Version, Writer, bad, csr, extents::Extents, kind, meta, strings::{self, Strings}};
use crate::code_rule_dsl::logic::front::{AggOp, Atom, Literal, Rule, Term};
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
pub(crate) fn decl_hash<'a>(rel: &str, cols: impl Iterator<Item = (&'a str, &'a str)>) -> u64 {
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
pub(crate) enum CellText<'a> {
    Int(i64),
    Str(&'a str),
}

/// A fact relation's hash: its name, whether it is of the `.about` rows, and its rows.
pub(crate) fn facts_hash<'a, Row: Iterator<Item = CellText<'a>>>(rel: &str, about: bool, rows: impl Iterator<Item = Row>) -> u64 {
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
            }
        }
    }
    h.finish()
}

/// The hash of a list of names (inputs, outputs); `tag` tells the lists apart.
pub(crate) fn names_hash<'a>(tag: u8, names: impl Iterator<Item = &'a str>) -> u64 {
    let mut h = hasher();
    h.write_u8(6 + tag);
    for n in names {
        put_text(&mut h, n);
    }
    h.finish()
}

/// A part, by ids, with its hash.
pub(crate) struct Decl {
    pub cols: Vec<(u32, u32)>,
    pub hash: u64,
}
pub(crate) struct FactRel {
    pub rows: Vec<Vec<Cell>>,
    pub hash: u64,
}

/// The parts an image holds besides its rules, by ids. A delta's are what it replaces.
#[derive(Default)]
pub(crate) struct Parts {
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

pub(crate) struct Encoder<'p> {
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
        let end = self.kind.len();
        let mut h = hasher();
        let head_at = at + 1;
        let head_end = head_at + self.size.get(head_at) as usize;
        for n in at..end {
            let (kd, size, val) = (self.kind.get(n), self.size.get(n), self.val.get(n));
            h.write_u8(kd);
            h.write_u32(size);
            match kd & kind::MASK {
                kind::ATOM => {
                    if n == head_at {
                        self.heads.push((val, k));
                    } else if n >= head_end {
                        let e = self.read_rules.len() as u32;
                        self.reads.push((val, e));
                        self.read_rules.push(k);
                        self.read_nodes.push(n as u32);
                    }
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
        self.rule_hash.push(h.finish());
        self.from.push(super::NO_RULE);
        k
    }
    /// Records that local rule `k` came from rule `src` of the input (§9).
    pub fn set_from(&mut self, k: u32, src: super::RuleId) {
        self.from[k as usize] = src.0;
    }

    // -- a rule from the tree --

    fn term(&mut self, t: &Term) {
        match t {
            Term::Var(v) => self.var(v),
            Term::Wild => self.leaf(kind::WILD, 0),
            Term::Top => self.leaf(kind::TOP, 0),
            Term::Int(i) => self.int(*i),
            Term::Str(s) => {
                let a = self.atom(s);
                self.leaf(kind::STR, a)
            }
            Term::Call(f, args) => {
                let a = self.atom(f);
                let at = self.open(kind::CALL, a);
                for x in args.iter() {
                    self.term(x);
                }
                self.close(at);
            }
        }
    }
    fn tree_atom(&mut self, a: &Atom) {
        let flags = if a.neg { kind::NEG } else { 0 } | if a.gated { kind::GATED } else { 0 };
        let r = self.rel(&a.rel);
        let at = self.open(kind::ATOM | flags, r);
        for x in a.args.iter() {
            self.term(x);
        }
        self.close(at);
    }
    fn literal(&mut self, l: &Literal) -> Result<(), String> {
        match l {
            Literal::Atom(a) => self.tree_atom(a),
            Literal::Cmp { l, op, r } => {
                let o = super::CmpOp::of(op).ok_or_else(|| bad(format!("comparison {op:?}")))? as u32;
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
                let a = self.atom(var);
                let s = self.slot_of(a);
                let at = self.open(kind::AGG | code << 5, s);
                match over {
                    Some(v) => self.var(v),
                    None => self.leaf(kind::WILD, 0),
                }
                if let AggOp::Join { by, sep } = op {
                    for b in by {
                        self.var(b);
                    }
                    let a = self.atom(sep);
                    self.leaf(kind::STR, a);
                }
                self.tree_atom(atom);
                self.close(at);
            }
            Literal::Nearest { n, a, kindset } => {
                let k = self.atom(kindset);
                let at = self.open(kind::NEAREST, k);
                self.var(n);
                self.var(a);
                self.close(at);
            }
            Literal::Ast { document, node, matcher, captures } => {
                let m = self.atom(matcher);
                let at = self.open(kind::AST, m);
                self.var(document);
                self.var(node);
                for (n, v) in captures {
                    let a = self.atom(n);
                    self.leaf(kind::STR, a);
                    self.var(v);
                }
                self.close(at);
            }
        }
        Ok(())
    }
    /// Appends `rule`, from the tree. Its local index.
    pub fn tree_rule(&mut self, rule: &Rule) -> Result<u32, String> {
        let at = self.begin_rule();
        self.tree_atom(&rule.head);
        for l in rule.body.iter() {
            if let Err(e) = self.literal(l) {
                self.abandon(at);
                return Err(e);
            }
        }
        Ok(self.end_rule(at))
    }
    /// Drops the rule begun at `at` (a rule that failed to encode).
    pub fn abandon(&mut self, at: usize) {
        self.kind.truncate(at);
        self.size.truncate(at);
        self.val.truncate(at);
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

    /// The image: the rules written, and `parts`; a delta when it has a parent, removing `removed` of its rules.
    pub fn finish(self, parts: &Parts, removed: &[u32]) -> Vec<u8> {
        let Encoder { parent, atom_base, rel_base, rule_base, strings, rels, mut promoted, kind, size, val, ints, rule_node, rule_vars, vars, rule_hash, heads, reads, read_rules, read_nodes, from, .. } = self;
        let delta = parent.is_some();
        let mut w = Writer::default();
        // Strings, and each of this image's atoms' relation.
        let atoms = strings.len();
        let mut atom_rel = vec![0u32; atoms];
        for (k, &a) in rels.iter().enumerate() {
            if a >= atom_base {
                atom_rel[(a - atom_base) as usize] = rel_base + k as u32 + 1;
            }
        }
        let (bytes, ends, table) = strings.into_parts();
        w.raw(R::StrBytes, &bytes);
        w.u32s(R::StrEnds, &ends);
        for b in &table.buckets {
            w.u64s(R::StrBuckets, b);
        }
        for b in &table.over {
            w.u64s(R::StrOver, b);
        }
        w.u32s(R::Rels, &rels);
        w.u32s(R::AtomRel, &atom_rel);
        promoted.sort_unstable();
        let promoted: Vec<u64> = promoted.iter().map(|&(a, r)| (a as u64) << 32 | r as u64).collect();
        w.u64s(R::Promoted, &promoted);
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
        w.u64s(R::RuleHash, &rule_hash);
        w.u32s(R::Vars, &vars);
        // Parts, sorted by atom so they are found by search.
        let mut own_sum = rule_hash.iter().fold(0u64, |a, &h| a.wrapping_add(h));
        let mut decl_ends = vec![0u32];
        let mut cols = Vec::new();
        for (&a, d) in &parts.decls {
            w.u32(R::DeclName, a);
            w.u64(R::DeclHash, d.hash);
            own_sum = own_sum.wrapping_add(d.hash);
            for &(n, t) in &d.cols {
                cols.push(n);
                cols.push(t);
            }
            decl_ends.push(cols.len() as u32 / 2);
        }
        w.u32s(R::DeclCols, &decl_ends);
        w.u32s(R::Cols, &cols);
        for &a in &parts.decls_gone {
            w.u32(R::DeclGone, a);
        }
        let (mut fact_ends, mut row_ends, mut cells) = (vec![0u32], vec![0u32], 0u32);
        for (&key, f) in &parts.facts {
            w.u32(R::FactRel, key);
            w.u64(R::FactHash, f.hash);
            own_sum = own_sum.wrapping_add(f.hash);
            for row in &f.rows {
                for c in row {
                    match *c {
                        Cell::Int(i) => {
                            w.u8(R::CellKind, 0);
                            w.u64(R::CellVal, i as u64);
                        }
                        Cell::Str(a) => {
                            w.u8(R::CellKind, 1);
                            w.u64(R::CellVal, a.0 as u64);
                        }
                    }
                    cells += 1;
                }
                row_ends.push(cells);
            }
            fact_ends.push(row_ends.len() as u32 - 1);
        }
        w.u32s(R::FactRows, &fact_ends);
        w.u32s(R::RowCells, &row_ends);
        for &k in &parts.facts_gone {
            w.u32(R::FactGone, k);
        }
        let mut owns = 0u64;
        let (mut ih, mut oh) = (0, 0);
        if let Some((names, h)) = &parts.inputs {
            owns |= 1;
            ih = *h;
            own_sum = own_sum.wrapping_add(*h);
            w.u32s(R::Inputs, names);
        }
        if let Some((names, h)) = &parts.outputs {
            owns |= 2;
            oh = *h;
            own_sum = own_sum.wrapping_add(*h);
            w.u32s(R::Outputs, names);
        }
        if parts.owns_about {
            owns |= 4;
        }
        // Indexes: rules by head, and reads; dense over the relations in a base, keyed in a delta.
        let index = |w: &mut Writer, pairs: Vec<(u32, u32)>, keys: R, starts: R| -> Vec<u32> {
            if delta {
                let (k, s, v) = csr::keyed(pairs);
                w.u32s(keys, &k);
                w.u32s(starts, &s);
                v
            } else {
                let n = rels.len();
                let mut counts = vec![0u32; n + 1];
                for &(r, _) in &pairs {
                    counts[r as usize + 1] += 1;
                }
                for i in 0..n {
                    counts[i + 1] += counts[i];
                }
                let mut at = counts.clone();
                let mut v = vec![0u32; pairs.len()];
                for &(r, x) in &pairs {
                    v[at[r as usize] as usize] = x;
                    at[r as usize] += 1;
                }
                w.u32s(starts, &counts);
                v
            }
        };
        let head_rules = index(&mut w, heads, R::HeadKeys, R::HeadStart);
        w.u32s(R::HeadRules, &head_rules);
        let entries = index(&mut w, reads, R::ReadKeys, R::ReadStart);
        let (rr, rn): (Vec<u32>, Vec<u32>) = entries.iter().map(|&e| (read_rules[e as usize], read_nodes[e as usize])).unzip();
        w.u32s(R::ReadRule, &rr);
        w.u32s(R::ReadNode, &rn);
        w.u32s(R::Removed, removed);
        if from.iter().any(|&f| f != super::NO_RULE) {
            w.u32s(R::From, &from);
            let mut index: Vec<u64> = from.iter().enumerate().filter(|(_, f)| **f != super::NO_RULE).map(|(k, &f)| (f as u64) << 32 | k as u64).collect();
            index.sort_unstable();
            w.u64s(R::FromIndex, &index);
        }
        let mut m = [0u64; meta::WORDS];
        m[meta::KIND] = delta as u64;
        m[meta::RULE_BASE] = rule_base as u64;
        m[meta::ATOM_BASE] = atom_base as u64;
        m[meta::REL_BASE] = rel_base as u64;
        m[meta::HASH_LEVEL] = table.level as u64;
        m[meta::HASH_SPLIT] = table.split as u64;
        m[meta::HASH_COUNT] = table.count;
        m[meta::OWNS] = owns;
        m[meta::INPUTS_HASH] = ih;
        m[meta::OUTPUTS_HASH] = oh;
        m[meta::OWN_SUM] = own_sum;
        w.u64s(R::Meta, &m);
        w.finish()
    }
}
