//! Making a version (decision 0173 §6, §7): a `Builder` states an edit of a parent (or a new base) and makes the
//! version that costs it, never a copy of the parent.
//!
//! ```text
//! let mut b = Builder::on(&parent);
//! b.remove(rule.id());                                    // a rule of the parent
//! b.copy(&other, other_rule);                             // a rule of another version, by its handles
//! b.rule().head("p", &[T::Var("x")]).atom("e", &[T::Var("x"), T::Wild]).finish()?;
//! b.declare("p", &[("x", "int")]);  b.set_inputs(&["e"]);
//! let (v, made) = b.finish()?;                            // a view, an overlay, or folded
//! ```
//!
//! What it makes:
//! - **a view** when the edit only removes rules, and what is left is few ranges (`max_ranges`: a quarter of the
//!   parent's rules, decision 0244); a view of a view is of that view's parent;
//! - **an overlay** otherwise: a delta holding only what it adds, numbered after the parent's ids;
//! - **a fold** when the edit reaches half its parent: a new base of the result, keeping its atom and relation ids.
//!
//! A version's digest is made with it, from the parent's and the edit's hashes.
use super::encoder::{self, CellText, Decl, Encoder, FactRel, Parts};
use super::handles::{AtomRef, CmpOp, Lit, LitRef, RuleRef, TermRef};
use super::version::{Made, Version, max_ranges};
use super::{ABOUT_BIT, AtomId, Cell, RelId, RuleId, Slot, kind};
use crate::code_rule_dsl::logic::front::{ast::Value, name::List};
use crate::code_rule_dsl::logic::program_facts::Facts;
use std::sync::Arc;

/// A term to write, by text or by id.
#[derive(Clone, Debug, PartialEq)]
pub enum T<'a> {
    Var(&'a str),
    /// A variable of the rule being written, by the atom naming it.
    VarAtom(AtomId),
    Wild,
    Top,
    Int(i64),
    /// A branded atom (decision 0270).
    Branded(crate::code_rule_dsl::logic::front::ast::Branded),
    Str(&'a str),
    StrAtom(AtomId),
    Call(&'a str, Vec<T<'a>>),
    /// A call by the atom naming its function (a version's, or the builder's).
    CallAtom(AtomId, Vec<T<'a>>),
}

/// A term of `v`'s rule `r`, to write into a builder under `v`'s ids (one on `v`, or derived from it).
pub fn t_of<'v>(v: &'v Version, r: RuleRef<'v>, t: TermRef<'v>) -> T<'v> {
    match t {
        TermRef::Var(s) => T::VarAtom(r.var(s)),
        TermRef::Wild => T::Wild,
        TermRef::Top => T::Top,
        TermRef::Int(i) => T::Int(i),
        TermRef::Branded(b) => T::Branded(b),
        TermRef::Str(a) => T::StrAtom(a),
        TermRef::Call(f, args) => T::CallAtom(f, args.map(|a| t_of(v, r, a)).collect()),
    }
}

/// A relation to write, by its id (the builder's parent's, or one it made) or by name.
#[derive(Clone, Copy, Debug)]
pub enum Rel<'a> {
    Id(RelId),
    Name(&'a str),
}
impl From<RelId> for Rel<'_> {
    fn from(r: RelId) -> Self {
        Rel::Id(r)
    }
}
impl<'a> From<&'a str> for Rel<'a> {
    fn from(r: &'a str) -> Self {
        Rel::Name(r)
    }
}

#[derive(Clone)]
pub struct Builder<'p> {
    parent: Option<&'p Arc<Version>>,
    /// The version whose ids this image's are: the parent, or the version a fold is of.
    ids: Option<&'p Version>,
    enc: Encoder<'p>,
    removed: Vec<u32>,
    parts: Parts,
    /// Whether the edit states any part.
    parts_changed: bool,
    /// Rows of facts stated, for the edit's size.
    rows: usize,
}

impl Builder<'static> {
    /// A new base.
    pub fn base() -> Builder<'static> {
        Builder { parent: None, ids: None, enc: Encoder::new(None), removed: Vec::new(), parts: Parts::default(), parts_changed: false, rows: 0 }
    }

}

/// A new base (`Builder::base`): a lowerer's header before it states anything.
impl Default for Builder<'static> {
    fn default() -> Self {
        Builder::base()
    }
}

impl<'p> Builder<'p> {
    /// An edit of `parent`.
    pub fn on(parent: &'p Arc<Version>) -> Builder<'p> {
        Builder { parent: Some(parent), ids: Some(parent), enc: Encoder::new(Some(parent)), removed: Vec::new(), parts: Parts::default(), parts_changed: false, rows: 0 }
    }
    /// A derivation of `input` (§7, §9): a new base under `input`'s atom and relation ids, holding all of `input`'s
    /// parts and none of its rules. A pass adds each rule it makes from one of `input`'s by `rewrite`, which records
    /// where the rule came from, and states the parts it changes.
    pub fn derive(input: &'p Version) -> Builder<'p> {
        let mut b = Builder { parent: None, ids: Some(input), enc: Encoder::with_ids_of(input), removed: Vec::new(), parts: Parts::default(), parts_changed: false, rows: 0 };
        for d in input.decls() {
            let cols = d.cols().map(|(n, t)| (n.0, t.0)).collect();
            b.parts.decls.insert(d.name().0, Decl { cols, hash: d.hash() });
        }
        for f in input.all_facts().into_iter().chain(input.about_facts()) {
            let rows = f.rows().map(|r| r.collect()).collect();
            b.parts.facts.insert(f.name().0 | if f.about() { ABOUT_BIT } else { 0 }, FactRel { rows, hash: f.hash() });
        }
        b.parts.inputs = Some((input.inputs().map(|a| a.0).collect(), input.inputs_hash()));
        b.parts.outputs = Some((input.outputs().map(|a| a.0).collect(), input.outputs_hash()));
        b.parts.owns_about = true;
        b
    }
    /// A derivation of `input` that states each of its parts (decision 0241): a new base under `input`'s atom and
    /// relation ids, holding none of `input`'s rules or parts. What is not stated is empty. Finished with nothing
    /// stated, it is `input`'s names alone, which a maintained form overlays.
    pub fn under(input: &'p Version) -> Builder<'p> {
        Builder { parent: None, ids: Some(input), enc: Encoder::with_ids_of(input), removed: Vec::new(), parts: Parts::default(), parts_changed: false, rows: 0 }
    }
    /// A base of all of `v`, under `v`'s atom and relation ids: each rule copied as it is, its source kept.
    pub fn folding(v: &'p Version) -> Builder<'p> {
        let mut b = Builder::derive(v);
        for r in v.rules() {
            let id = b.copy(v, r);
            if let Some(src) = r.source() {
                b.enc.set_from(id.0 - b.enc.rule_base(), src);
            }
        }
        b
    }

    /// `v` with no stack (decision 0385): a view of one new base holding every rule `v`'s stack numbers, by its id,
    /// its own and those an overlay removed, under `v`'s atom and relation ids and with its parts, each rule's source
    /// kept; the view leaves out the removed. Its ids, size and digest are `v`'s, so an edit of it reads as an edit of
    /// `v`. `None` where the removed rules are more ranges than a view holds.
    pub fn flat(v: &Arc<Version>) -> Result<Option<Arc<Version>>, String> {
        let mut b = Builder::derive(v);
        for r in v.stack_rules() {
            let id = b.copy(v, r);
            if id != r.id() {
                return Err(format!("a flat version numbers rule {} as {}", r.id().0, id.0));
            }
            if let Some(src) = r.source() {
                b.enc.set_from(id.0 - b.enc.rule_base(), src);
            }
        }
        let base = Version::of_image(b.image()?)?;
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        for r in v.rules() {
            let k = r.id().0;
            match ranges.last_mut() {
                Some((_, end)) if *end == k => *end = k + 1,
                _ => ranges.push((k, k + 1)),
            }
        }
        if ranges.len() > super::version::max_ranges(base.size()) {
            return Ok(None);
        }
        Ok(Some(Version::view(&base, ranges, v.size(), v.digest())))
    }

    /// A base of all of `edit`, an overlay of `parent` (its derivation's input), under its atom and relation ids: each
    /// rule copied with the source the derivation records for it: a rule of `parent` with a source its own (as `keep`
    /// records it), a rule the edit added its source in `parent`; a rule with none stays without (a base's digest). A pass's output folded so is the same edit of its input as the
    /// overlay was (`manager::Delta`, the request record by rule, decision 0331); `folding` keeps the older sources.
    fn folding_edit(edit: &'p Version, parent: &Version) -> Builder<'p> {
        let mut b = Builder::derive(edit);
        let added = match edit {
            Version::Overlay { delta, .. } => delta.rule_base(),
            _ => u32::MAX,
        };
        for r in edit.rules() {
            let id = b.copy(edit, r);
            let src = r.source().map(|s| if r.id().0 < added && parent.contains(r.id()) { r.id() } else { s });
            if let Some(src) = src {
                b.enc.set_from(id.0 - b.enc.rule_base(), src);
            }
        }
        b
    }

    /// The edit's size: rules removed and added, and part rows stated.
    pub fn size(&self) -> usize {
        self.removed.len() + self.enc.rules() as usize + self.parts.decls.len() + self.parts.decls_gone.len() + self.rows + self.parts.facts_gone.len()
    }

    /// Whether the edit states nothing: no rule removed or added, no part stated.
    pub fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.enc.rules() == 0 && !self.parts_changed
    }

    // -- names --

    /// The atom of `text`: the parent's, or a new one.
    pub fn atom(&mut self, text: &str) -> AtomId {
        AtomId(self.enc.atom(text))
    }
    /// The text of atom `a`, one of this builder's.
    pub fn text(&self, a: AtomId) -> &str {
        self.enc.text(a.0)
    }
    /// The relation named `name`: the parent's, or a new one.
    pub fn rel(&mut self, name: &str) -> RelId {
        RelId(self.enc.rel(name))
    }
    fn rel_id(&mut self, r: Rel<'_>) -> u32 {
        match r {
            Rel::Id(r) => r.0,
            Rel::Name(n) => self.enc.rel(n),
        }
    }

    // -- rules --

    /// Removes the parent's rule `id`.
    pub fn remove(&mut self, id: RuleId) {
        self.removed.push(id.0);
    }
    /// Removes each of the parent's rules `keep` refuses.
    pub fn retain(&mut self, mut keep: impl FnMut(RuleRef<'_>) -> bool) {
        let p = self.parent.expect("a builder on a parent");
        for r in p.rules() {
            if !keep(r) {
                self.removed.push(r.id().0);
            }
        }
    }
    /// The id the next added rule gets.
    pub fn next_id(&self) -> RuleId {
        RuleId(self.enc.rule_base() + self.enc.rules())
    }
    /// Adds a copy of `rule`, one of `from`'s rules: when `from`'s ids are this builder's, its nodes, variables and
    /// hash as they are, one range; else each name by its text.
    pub fn copy(&mut self, from: &Version, rule: RuleRef<'_>) -> RuleId {
        let id = self.next_id();
        let same = self.same(rule);
        if same {
            // Its ids are this builder's: its nodes, variables and hash as they are, one range (`Encoder::copy_run`).
            self.enc.copy_run(rule.image(), rule.local()..rule.local() + 1);
            return id;
        }
        let at = self.enc.begin_rule();
        let n = rule.nodes();
        let vars: Vec<u32> = rule.vars().map(|a| a.0).collect();
        self.enc.copy_nodes(from, rule.image(), n.start + 1..n.end, &vars, same);
        self.enc.end_rule(at);
        id
    }
    /// Adds copies of `rules`, `from`'s, in order, as `keep` adds each: a run of consecutive rules of one image,
    /// when `from`'s ids are this builder's, is copied as one range of nodes (`Encoder::copy_run`, 0276 §3), as a
    /// stratum is (0266 §3); any other rule as `keep` copies it.
    pub fn keep_run(&mut self, from: &Version, rules: &[RuleRef<'_>]) {
        let mut i = 0;
        while i < rules.len() {
            let r = rules[i];
            if !self.same(r) {
                self.keep(from, r);
                i += 1;
                continue;
            }
            let mut j = i + 1;
            while j < rules.len() && std::ptr::eq(rules[j].image(), r.image()) && rules[j].local() == rules[j - 1].local() + 1 {
                j += 1;
            }
            let first = self.enc.copy_run(r.image(), r.local()..r.local() + (j - i) as u32);
            if self.parent.is_none() {
                for (n, kept) in rules[i..j].iter().enumerate() {
                    self.enc.set_from(first + n as u32, kept.id());
                }
            }
            i = j;
        }
    }
    /// Adds a copy of `rule`, one of a derivation's input's, recording it as the copy's source (§9): a rule a pass
    /// kept as it was (`manager`'s derivations).
    pub fn keep(&mut self, from: &Version, rule: RuleRef<'_>) -> RuleId {
        let id = self.copy(from, rule);
        if self.parent.is_none() {
            self.enc.set_from(id.0 - self.enc.rule_base(), rule.id());
        }
        id
    }
    /// Adds a copy of `rule`, one of `from`'s, recording it as the copy's source in an edit too: a rule a pass kept,
    /// when a pipeline runs it again over an edit of its input onto its last output (`manager`, 0281 §3).
    pub fn keep_into(&mut self, from: &Version, rule: RuleRef<'_>) -> RuleId {
        let id = self.copy(from, rule);
        self.enc.set_from(id.0 - self.enc.rule_base(), rule.id());
        id
    }
    /// Adds a copy of `rule`, one of `from`'s, with the source `from` records for it (`RuleRef::source`): a rule
    /// carried into a new base as it was, as a kept link splices the parts it did not link again.
    pub fn copy_as_is(&mut self, from: &Version, rule: RuleRef<'_>) -> RuleId {
        let id = self.copy(from, rule);
        if let Some(src) = rule.source() {
            self.enc.set_from(id.0 - self.enc.rule_base(), src);
        }
        id
    }
    /// Adds a copy of `rule`, one of `from`'s, recording `source` as its source: a rule a pass rewrote elsewhere (a
    /// derivation of its own), replacing its input's rule `source` in an edit (`product_demand`).
    pub fn copy_from(&mut self, from: &Version, rule: RuleRef<'_>, source: RuleId) -> RuleId {
        let id = self.copy(from, rule);
        self.enc.set_from(id.0 - self.enc.rule_base(), source);
        id
    }
    fn same(&self, rule: RuleRef<'_>) -> bool {
        self.ids.is_some_and(|v| v.holds_image(rule.image()))
    }
    /// Starts a rule written term by term.
    pub fn rule(&mut self) -> RuleBuilder<'_, 'p> {
        let at = self.enc.begin_rule();
        RuleBuilder { b: self, at, head: false, failed: None, done: false, from: None }
    }

    /// Opens a rule written literal by literal between other reads and edits of this builder's fields (the
    /// lowering reads declarations while it writes a rule, decision 0276 A3): its head first, then its body, then
    /// `close_rule`. One rule is open at a time.
    pub fn open_rule(&mut self) -> Open {
        Open { at: self.enc.begin_rule(), head: false, failed: None }
    }
    fn put_atom_node(&mut self, rel: Rel<'_>, args: &[T<'_>], flags: u8) {
        let r = self.rel_id(rel);
        let at = self.enc.open(kind::ATOM | flags, r);
        for a in args {
            write_t(&mut self.enc, a);
        }
        self.enc.close(at);
    }
    fn put_body(o: &mut Open) {
        if !o.head && o.failed.is_none() {
            o.failed = Some(super::bad("a rule's body before its head"));
        }
    }
    /// Open rule `o`'s head.
    pub fn put_head<'r>(&mut self, o: &mut Open, rel: impl Into<Rel<'r>>, args: &[T<'_>]) {
        if o.head {
            o.failed = Some(super::bad("a rule with two heads"));
        }
        self.put_atom_node(rel.into(), args, 0);
        o.head = true;
    }
    /// An atom of open rule `o`'s body: negated, gated.
    pub fn put_atom<'r>(&mut self, o: &mut Open, rel: impl Into<Rel<'r>>, args: &[T<'_>], neg: bool, gated: bool) {
        Self::put_body(o);
        self.put_atom_node(rel.into(), args, if neg { kind::NEG } else { 0 } | if gated { kind::GATED } else { 0 });
    }
    /// A comparison of open rule `o`'s body.
    pub fn put_cmp(&mut self, o: &mut Open, l: T<'_>, op: CmpOp, r: T<'_>) {
        Self::put_body(o);
        let at = self.enc.open(kind::CMP, op as u32);
        write_t(&mut self.enc, &l);
        write_t(&mut self.enc, &r);
        self.enc.close(at);
    }
    /// `nearest(n, a, kindset)` in open rule `o`'s body: `a` the nearest ancestor of `n` in the kind set.
    pub fn put_nearest(&mut self, o: &mut Open, n: &str, a: &str, kindset: &str) {
        Self::put_body(o);
        let k = self.enc.atom(kindset);
        let at = self.enc.open(kind::NEAREST, k);
        self.enc.var(n);
        self.enc.var(a);
        self.enc.close(at);
    }
    /// An `ast(document, node, matcher)` literal of open rule `o`'s body, its captures (name, variable) in order.
    pub fn put_ast(&mut self, o: &mut Open, document: &str, node: &str, matcher: &str, captures: &[(&str, &str)]) {
        Self::put_body(o);
        let m = self.enc.atom(matcher);
        let at = self.enc.open(kind::AST, m);
        self.enc.var(document);
        self.enc.var(node);
        for (name, var) in captures {
            let a = self.enc.atom(name);
            self.enc.leaf(kind::STR, a);
            self.enc.var(var);
        }
        self.enc.close(at);
    }
    /// An aggregate `var = op over : { rel(args) }` of open rule `o`'s body (`over` `None` for a count); a join's
    /// keys follow `over`, then its separator.
    pub fn put_agg<'r>(&mut self, o: &mut Open, var: &str, op: &crate::code_rule_dsl::logic::front::ast::AggOp, over: Option<&str>, rel: impl Into<Rel<'r>>, args: &[T<'_>]) {
        use crate::code_rule_dsl::logic::front::ast::AggOp;
        Self::put_body(o);
        let code: u8 = match op {
            AggOp::Count => 0,
            AggOp::Min => 1,
            AggOp::Max => 2,
            AggOp::Sum => 3,
            AggOp::Product => 4,
            AggOp::Join { .. } => 5,
        };
        let a = self.enc.atom(var);
        let s = self.enc.slot_of(a);
        let at = self.enc.open(kind::AGG | code << 5, s);
        match over {
            Some(v) => self.enc.var(v),
            None => self.enc.leaf(kind::WILD, 0),
        }
        if let AggOp::Join { by, sep } = op {
            for k in by {
                self.enc.var(k);
            }
            let a = self.enc.atom(sep);
            self.enc.leaf(kind::STR, a);
        }
        self.put_atom_node(rel.into(), args, 0);
        self.enc.close(at);
    }
    /// Ends open rule `o`: its id, or why it is refused (then it is not written).
    pub fn close_rule(&mut self, mut o: Open) -> Result<RuleId, String> {
        if let Some(e) = o.failed.take().or_else(|| (!o.head).then(|| super::bad("a rule with no head"))) {
            self.enc.abandon(o.at);
            return Err(e);
        }
        Ok(RuleId(self.enc.rule_base() + self.enc.end_rule(o.at)))
    }
    /// Ends open rule `o` unless a rule of the same hash was ended before (its hash in `seen`): then it is left out
    /// and `None`.
    pub fn close_rule_once(&mut self, o: Open, seen: &mut std::collections::HashSet<u64>) -> Result<Option<RuleId>, String> {
        if o.failed.is_none() && o.head {
            self.enc.close(o.at);
            let h = self.enc.hash_closed(o.at);
            if !seen.insert(h) {
                self.enc.abandon(o.at);
                return Ok(None);
            }
        }
        self.close_rule(o).map(Some)
    }
    /// Leaves open rule `o` out.
    pub fn abandon_rule(&mut self, o: Open) {
        self.enc.abandon(o.at);
    }

    /// Starts a rule that is `from`'s rule `rule` rewritten by `rw` as it is copied: its head, then each body literal
    /// `rw` keeps. More literals may follow; `finish` writes it, recording `rule` as its source (§9).
    /// Every rule of `from` through `rw` (0276 §3), in order: a rule `rw.begin` picks is rewritten, the rest are kept
    /// (`keep_run`). The rewrite of the whole program, one call.
    pub fn rewrite_all(&mut self, from: &Version, rw: &mut impl Rewrite) -> Result<(), String> {
        let mut kept: Vec<RuleRef<'_>> = Vec::new();
        for r in from.rules() {
            if rw.begin(r) {
                self.keep_run(from, &std::mem::take(&mut kept));
                self.rewrite(from, r, rw).finish()?;
            } else {
                kept.push(r);
            }
        }
        self.keep_run(from, &kept);
        Ok(())
    }
    pub fn rewrite(&mut self, from: &Version, rule: RuleRef<'_>, rw: &mut impl Rewrite) -> RuleBuilder<'_, 'p> {
        let same = self.same(rule);
        let vars: Vec<u32> = rule.vars().map(|a| a.0).collect();
        let at = self.enc.begin_rule();
        let mut w = Walk { enc: &mut self.enc, from, im: rule.image(), vars: &vars, same, prefix: Vec::new() };
        w.atom(rule.head(), Site::Head(rule.head()), rw);
        for l in rule.body() {
            if rw.keep(l) {
                w.lit(l, rw);
            }
        }
        RuleBuilder { b: self, at, head: true, failed: None, done: false, from: Some(rule.id()) }
    }

    // -- parts --

    fn declare_names(&mut self, rel: &str, cols: &[(crate::code_rule_dsl::logic::front::name::Name, crate::code_rule_dsl::logic::front::name::Name)]) {
        let pairs: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        self.declare(rel, &pairs);
    }
    /// Declares relation `rel`'s columns (names and types), replacing the parent's.
    pub(in crate::code_rule_dsl::logic::program_image) fn declare(&mut self, rel: &str, cols: &[(&str, &str)]) {
        let a = self.enc.atom(rel);
        self.enc.rel_of_atom(a);
        let ids = cols.iter().map(|&(n, t)| (self.enc.atom(n), self.enc.atom(t))).collect();
        self.parts.decls_gone.remove(&a);
        self.parts.decls.insert(a, Decl { cols: ids, hash: encoder::decl_hash(rel, cols.iter().copied()) });
        self.parts_changed = true;
    }
    /// Removes the parent's declaration of `rel`.
    pub(in crate::code_rule_dsl::logic::program_image) fn undeclare(&mut self, rel: &str) {
        let a = self.enc.atom(rel);
        self.parts.decls.remove(&a);
        if self.parent.is_some_and(|p| p.decl(AtomId(a)).is_some()) {
            self.parts.decls_gone.insert(a);
        }
        self.parts_changed = true;
    }
    fn rows_of(&mut self, rows: &[List<Value>]) -> Vec<Vec<Cell>> {
        rows.iter().map(|row| row.iter().map(|v| match v {
            Value::Int(i) => Cell::Int(*i),
            Value::Str(s) => Cell::Str(AtomId(self.enc.atom(s))),
            Value::Branded(b) => Cell::Branded(*b),
        }).collect()).collect()
    }
    fn hash_rows(rel: &str, about: bool, rows: &[List<Value>]) -> u64 {
        encoder::facts_hash(rel, about, rows.iter().map(|row| row.iter().map(|v| match v {
            Value::Int(i) => CellText::Int(*i),
            Value::Str(s) => CellText::Str(s.as_str()),
            Value::Branded(b) => CellText::Branded(*b),
        })))
    }
    /// States relation `rel`'s facts, replacing the parent's.
    pub(in crate::code_rule_dsl::logic::program_image) fn set_facts(&mut self, rel: &str, rows: &[List<Value>]) {
        let a = self.enc.atom(rel);
        self.enc.rel_of_atom(a);
        let cells = self.rows_of(rows);
        self.parts.facts_gone.remove(&a);
        self.parts.facts.insert(a, FactRel { rows: cells, hash: Self::hash_rows(rel, false, rows) });
        self.rows += rows.len().max(1);
        self.parts_changed = true;
    }
    /// Removes the parent's facts of `rel`.
    pub(in crate::code_rule_dsl::logic::program_image) fn drop_facts(&mut self, rel: &str) {
        let a = self.enc.atom(rel);
        self.parts.facts.remove(&a);
        if self.parent.is_some_and(|p| p.facts(AtomId(a)).is_some()) {
            self.parts.facts_gone.insert(a);
        }
        self.parts_changed = true;
    }
    /// States relation `rel`'s facts as cells of this builder's atoms, replacing the parent's.
    pub(in crate::code_rule_dsl::logic::program_image) fn set_fact_cells(&mut self, rel: &str, rows: Vec<Vec<Cell>>) {
        let a = self.enc.atom(rel);
        self.enc.rel_of_atom(a);
        let hash = self.hash_cells(rel, false, &rows);
        self.parts.facts_gone.remove(&a);
        self.rows += rows.len().max(1);
        self.parts.facts.insert(a, FactRel { rows, hash });
        self.parts_changed = true;
    }
    fn hash_cells(&self, rel: &str, about: bool, rows: &[Vec<Cell>]) -> u64 {
        let enc = &self.enc;
        encoder::facts_hash(rel, about, rows.iter().map(|row| row.iter().map(|c| match *c {
            Cell::Int(i) => CellText::Int(i),
            Cell::Str(x) => CellText::Str(enc.text(x.0)),
            Cell::Branded(b) => CellText::Branded(b),
        })))
    }
    /// The rows of about relation `name` this edit states, or its parent's (`about.rs` reads and writes them by type).
    pub fn about_rows(&mut self, name: &str) -> Vec<Vec<Cell>> {
        self.own_about();
        let a = self.enc.atom(name);
        self.parts.facts.get(&(a | ABOUT_BIT)).map(|f| f.rows.clone()).unwrap_or_default()
    }
    /// States about relation `name`'s rows, every other about relation kept.
    pub(in crate::code_rule_dsl::logic::program_image) fn set_about_rows(&mut self, name: &str, rows: Vec<Vec<Cell>>) {
        self.own_about();
        let a = self.enc.atom(name);
        for &c in super::about::relation_columns(name) {
            for row in &rows {
                if let Some(Cell::Str(x)) = row.get(c)
                    && !self.enc.text(x.0).is_empty()
                {
                    self.enc.rel_of_atom(x.0);
                }
            }
        }
        if rows.is_empty() {
            self.parts.facts.remove(&(a | ABOUT_BIT));
        } else {
            let hash = self.hash_cells(name, true, &rows);
            self.parts.facts.insert(a | ABOUT_BIT, FactRel { rows, hash });
        }
        self.parts_changed = true;
    }
    /// Takes the parent's about rows as this edit's own, so one of them can be stated (they are owned wholesale).
    fn own_about(&mut self) {
        if self.parts.owns_about {
            return;
        }
        if let Some(p) = self.parent {
            for f in p.about_facts() {
                let rows = f.rows().map(|r| r.collect()).collect();
                self.parts.facts.insert(f.name().0 | ABOUT_BIT, FactRel { rows, hash: f.hash() });
            }
        }
        self.parts.owns_about = true;
    }
    pub(in crate::code_rule_dsl::logic::program_image) fn set_inputs<S: AsRef<str>>(&mut self, names: &[S]) {
        let ids = names.iter().map(|n| self.enc.rel_atom_of(n.as_ref())).collect();
        self.parts.inputs = Some((ids, encoder::names_hash(0, names.iter().map(|n| n.as_ref()))));
        self.parts_changed = true;
    }
    pub(in crate::code_rule_dsl::logic::program_image) fn set_outputs<S: AsRef<str>>(&mut self, names: &[S]) {
        let ids = names.iter().map(|n| self.enc.rel_atom_of(n.as_ref())).collect();
        self.parts.outputs = Some((ids, encoder::names_hash(1, names.iter().map(|n| n.as_ref()))));
        self.parts_changed = true;
    }
    /// The inputs this edit states, or its parent's.
    pub fn input_names(&self) -> Vec<String> {
        match &self.parts.inputs {
            Some((ids, _)) => ids.iter().map(|&a| self.enc.text(a).to_owned()).collect(),
            None => self.parent.map(|p| p.input_names()).unwrap_or_default(),
        }
    }
    /// Whether `rel` is an input this edit states, or its parent's: `input_names` with nothing copied.
    pub fn has_input(&self, rel: &str) -> bool {
        match &self.parts.inputs {
            Some((ids, _)) => self.enc.find(rel).is_some_and(|a| ids.contains(&a)),
            None => self.parent.is_some_and(|p| p.inputs().any(|a| p.text(a) == rel)),
        }
    }
    /// The outputs this edit states, or its parent's.
    pub fn output_names(&self) -> Vec<String> {
        match &self.parts.outputs {
            Some((ids, _)) => ids.iter().map(|&a| self.enc.text(a).to_owned()).collect(),
            None => self.parent.map(|p| p.output_names()).unwrap_or_default(),
        }
    }
    /// Relation `rel`'s fact rows as this edit states them, or its parent's (`None` where it has none).
    pub fn fact_cells(&self, rel: &str) -> Option<Vec<Vec<Cell>>> {
        let a = self.enc.find(rel)?;
        match self.parts.facts.get(&a) {
            Some(f) => Some(f.rows.clone()),
            None if self.parts.facts_gone.contains(&a) => None,
            None => self.parent.and_then(|p| p.facts(AtomId(a))).map(|f| f.rows().map(|r| r.collect()).collect()),
        }
    }
    /// Whether relation `rel` is declared, by this edit or its parent.
    pub fn declares(&self, rel: &str) -> bool {
        let Some(a) = self.enc.find(rel) else { return false };
        self.parts.decls.contains_key(&a) || (!self.parts.decls_gone.contains(&a) && self.parent.is_some_and(|p| p.decl(AtomId(a)).is_some()))
    }
    /// States the fields edit `e` states (its declarations, facts, inputs, outputs and `.about` rows, by name), as
    /// this builder's: an edit staged against the same input (`fields::Edit`), its rules this builder's own.
    pub(in crate::code_rule_dsl::logic::program_image) fn state_fields_of(&mut self, e: &Builder<'_>) {
        let text = |a: u32| e.enc.text(a).to_owned();
        let value = |c: &Cell| match *c {
            Cell::Int(i) => Value::Int(i),
            Cell::Str(a) => Value::Str(e.enc.text(a.0).to_owned().into()),
            Cell::Branded(b) => Value::Branded(b),
        };
        for (&a, d) in &e.parts.decls {
            let cols: Vec<(String, String)> = d.cols.iter().map(|&(n, t)| (text(n), text(t))).collect();
            let pairs: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
            self.declare(&text(a), &pairs);
        }
        for &a in &e.parts.decls_gone {
            self.undeclare(&text(a));
        }
        for (&k, f) in e.parts.facts.iter().filter(|(k, _)| *k & ABOUT_BIT == 0) {
            let rows: Vec<List<Value>> = f.rows.iter().map(|row| row.iter().map(value).collect::<Vec<_>>().into()).collect();
            self.set_facts(&text(k), &rows);
        }
        for &a in &e.parts.facts_gone {
            self.drop_facts(&text(a));
        }
        if let Some((ids, _)) = &e.parts.inputs {
            self.set_inputs(&ids.iter().map(|&a| text(a)).collect::<Vec<_>>());
        }
        if let Some((ids, _)) = &e.parts.outputs {
            self.set_outputs(&ids.iter().map(|&a| text(a)).collect::<Vec<_>>());
        }
        if e.parts.owns_about {
            let about: Vec<(String, Vec<List<Value>>)> = e
                .parts
                .facts
                .iter()
                .filter(|(k, _)| *k & ABOUT_BIT != 0)
                .map(|(&k, f)| (text(k & !ABOUT_BIT), f.rows.iter().map(|row| row.iter().map(value).collect::<Vec<_>>().into()).collect()))
                .collect();
            self.own_about();
            self.parts.facts.retain(|k, _| k & ABOUT_BIT == 0);
            for (name, rows) in about {
                let cells: Vec<Vec<Cell>> = rows.iter().map(|row| row.iter().map(|v| self.cell(v)).collect()).collect();
                self.set_about_rows(&name, cells);
            }
        }
    }
    /// Relation `rel`'s declared columns (name, type), as this edit leaves them.
    pub fn columns_of(&self, rel: &str) -> Option<Vec<(String, String)>> {
        let a = self.enc.find(rel)?;
        if let Some(d) = self.parts.decls.get(&a) {
            return Some(d.cols.iter().map(|&(n, t)| (self.enc.text(n).to_owned(), self.enc.text(t).to_owned())).collect());
        }
        if self.parts.decls_gone.contains(&a) {
            return None;
        }
        self.parent.and_then(|p| p.columns_of(rel))
    }
    /// The declared relations' names, in name order, as this edit leaves them.
    pub fn declared_names(&self) -> Vec<String> {
        let mut names: std::collections::BTreeSet<String> = self.parent.map(|p| p.declared_names()).unwrap_or_default().into_iter().collect();
        for a in &self.parts.decls_gone {
            names.remove(self.enc.text(*a));
        }
        names.extend(self.parts.decls.keys().map(|&a| self.enc.text(a).to_owned()));
        names.into_iter().collect()
    }
    /// The relations with facts stated, in name order, as this edit leaves them.
    pub fn fact_names(&self) -> Vec<String> {
        let mut names: std::collections::BTreeSet<String> = self.parent.map(|p| p.fact_names()).unwrap_or_default().into_iter().collect();
        for a in &self.parts.facts_gone {
            names.remove(self.enc.text(*a));
        }
        names.extend(self.parts.facts.keys().filter(|&&k| k & ABOUT_BIT == 0).map(|&a| self.enc.text(a).to_owned()));
        names.into_iter().collect()
    }
    /// The rows of about relation `name` as this edit leaves them, read without taking them over (`about_rows`).
    pub fn about_cells(&self, name: &str) -> Vec<Vec<Cell>> {
        if self.parts.owns_about {
            let Some(a) = self.enc.find(name) else { return Vec::new() };
            return self.parts.facts.get(&(a | ABOUT_BIT)).map(|f| f.rows.clone()).unwrap_or_default();
        }
        let Some(p) = self.parent else { return Vec::new() };
        p.about_facts().into_iter().filter(|f| p.text(f.name()) == name).flat_map(|f| f.rows().map(|r| r.collect::<Vec<Cell>>()).collect::<Vec<_>>()).collect()
    }
    /// Adds rows `rows` to relation `rel`'s facts, after the parent's.
    pub(in crate::code_rule_dsl::logic::program_image) fn add_facts(&mut self, rel: &str, rows: &[List<Value>]) {
        if rows.is_empty() {
            return;
        }
        let a = self.atom(rel);
        let mut cells: Vec<Vec<Cell>> = match self.parts.facts.get(&a.0) {
            Some(f) => f.rows.clone(),
            None if self.parts.facts_gone.contains(&a.0) => Vec::new(),
            None => self.parent.and_then(|p| p.facts(a)).map(|f| f.rows().map(|r| r.collect()).collect()).unwrap_or_default(),
        };
        cells.extend(rows.iter().map(|row| row.iter().map(|v| self.cell(v)).collect::<Vec<Cell>>()));
        self.set_fact_cells(rel, cells);
    }
    /// States the program's other fields (`about_of`), all of them.
    pub(in crate::code_rule_dsl::logic::program_image) fn set_about(&mut self, about: &Facts) {
        self.parts.facts.retain(|k, _| k & ABOUT_BIT == 0);
        for (rel, rows) in about {
            let a = self.enc.atom(rel);
            let cells = self.rows_of(rows);
            // Every relation a row names has an id (`about::RELATION_COLUMNS`).
            for &c in super::about::relation_columns(rel) {
                for row in rows {
                    if let Some(Value::Str(s)) = row.get(c)
                        && !s.is_empty()
                    {
                        self.enc.rel(s);
                    }
                }
            }
            self.parts.facts.insert(a | ABOUT_BIT, FactRel { rows: cells, hash: Self::hash_rows(rel, true, rows) });
        }
        self.parts.owns_about = true;
        self.parts_changed = true;
    }

    // -- the version --

    /// The image written so far, once, in place (decision 0464): a base's, or a delta's over the parent. The builder
    /// is left as it is, to write on: nothing is cloned to write it.
    pub fn image(&self) -> Result<super::Image, String> {
        if self.removed.windows(2).all(|w| w[0] < w[1]) {
            return self.enc.finish(&self.parts, &self.removed);
        }
        let mut removed = self.removed.clone();
        removed.sort_unstable();
        removed.dedup();
        self.enc.finish(&self.parts, &removed)
    }

    /// The base written (`image`), as a version read where it was written.
    pub fn base_version(&self) -> Result<Arc<Version>, String> {
        if self.parent.is_some() {
            return Err(super::bad("an edit of a parent is no base"));
        }
        Version::of_image(self.image()?)
    }

    /// The edit as an overlay of its parent, never folded: for a version whose ids must stay as written (a
    /// maintained form over its run's names, decision 0241, and its `Maint`), so that its cost is its own size.
    pub fn finish_overlay(mut self) -> Result<Arc<Version>, String> {
        let parent = self.parent.ok_or("an overlay of no parent")?;
        self.removed.sort_unstable();
        self.removed.dedup();
        Version::over_image(parent, self.image()?)
    }
    /// The version the edit makes: a view, an overlay, or folded (`Builder`'s doc).
    pub fn finish(self) -> Result<(Arc<Version>, Made), String> {
        self.finish_as(false).map(|(v, made, _)| (v, made))
    }
    /// The version the edit makes (`finish`), and where it folded, each rule's id in the folded version by its id in
    /// the edit: a fold numbers the rules from 0, in the edit's order (the parent's kept, then the added). A pass that
    /// keeps its rules in an order of its own (`Placed`'s) reads its order back through it.
    pub fn finish_placing(self) -> Result<(Arc<Version>, Made, Option<std::collections::BTreeMap<RuleId, RuleId>>), String> {
        self.finish_as(true)
    }
    fn finish_as(mut self, placing: bool) -> Result<(Arc<Version>, Made, Option<std::collections::BTreeMap<RuleId, RuleId>>), String> {
        let Some(parent) = self.parent else {
            return Ok((Version::of_image(self.image()?)?, Made::Folded, None));
        };
        self.removed.sort_unstable();
        self.removed.dedup();
        if self.enc.rules() == 0 && !self.parts_changed
            && let Some(v) = view(parent, &self.removed)
        {
            return Ok((v, Made::View, None));
        }
        let size = self.size();
        let overlay = Version::over_image(parent, self.image()?)?;
        if 2 * size >= parent.size().max(1) {
            // `folding_edit` copies the overlay's rules in its order, so the k-th is rule k.
            let order = placing.then(|| overlay.rules().enumerate().map(|(k, r)| (r.id(), RuleId(k as u32))).collect());
            return Ok((Builder::folding_edit(&overlay, parent).finish()?.0, Made::Folded, order));
        }
        Ok((overlay, Made::Overlay, None))
    }
}

/// The view of `parent` less rules `removed` (its rules), if it is few enough ranges (`max_ranges`). A view of a view
/// is of that view's parent, its ranges that view's less `removed`.
pub fn view(parent: &Arc<Version>, removed: &[u32]) -> Option<Arc<Version>> {
    let (over, ranges): (&Arc<Version>, Vec<(u32, u32)>) = match &**parent {
        Version::View { parent: over, ranges, .. } => (over, ranges.clone()),
        _ => (parent, vec![(0, parent.rule_end())]),
    };
    let limit = max_ranges(over.size());
    let mut out = Vec::new();
    let mut gone = removed.iter().peekable();
    let mut digest = parent.digest();
    let mut size = parent.size();
    for (s, e) in ranges {
        let mut start = s;
        while let Some(&&g) = gone.peek() {
            if g >= e {
                break;
            }
            gone.next();
            if g < start {
                continue;
            }
            // A rule `parent` does not have removes nothing (an overlay's parent's removed ids are in its ranges).
            let Some(rule) = parent.rule(RuleId(g)) else { continue };
            digest = digest.wrapping_sub(rule.hash());
            size -= 1;
            if g > start {
                out.push((start, g));
            }
            start = g + 1;
        }
        if start < e {
            out.push((start, e));
        }
        if out.len() > limit {
            return None;
        }
    }
    Some(Version::view(over, out, size, digest))
}

/// A rule open on a builder (`Builder::open_rule`), written by the builder's `put_` writes.
#[must_use]
pub struct Open {
    at: usize,
    head: bool,
    failed: Option<String>,
}

/// A rule written term by term: its head, then its body's literals, in order. A rule refused (`finish`'s error) or
/// dropped unfinished is not written.
pub struct RuleBuilder<'b, 'p> {
    b: &'b mut Builder<'p>,
    at: usize,
    head: bool,
    failed: Option<String>,
    done: bool,
    /// The rule it is rewritten from (`Builder::rewrite`).
    from: Option<RuleId>,
}

impl Drop for RuleBuilder<'_, '_> {
    fn drop(&mut self) {
        if !self.done {
            self.b.enc.abandon(self.at);
        }
    }
}

impl<'b, 'p> RuleBuilder<'b, 'p> {
    fn term(&mut self, t: &T<'_>) {
        write_t(&mut self.b.enc, t);
    }
    fn atom_node(&mut self, rel: Rel<'_>, args: &[T<'_>], flags: u8) {
        let r = self.b.rel_id(rel);
        let at = self.b.enc.open(kind::ATOM | flags, r);
        for a in args {
            self.term(a);
        }
        self.b.enc.close(at);
    }
    fn body(&mut self) {
        if !self.head && self.failed.is_none() {
            self.failed = Some(super::bad("a rule's body before its head"));
        }
    }
    pub fn head(mut self, rel: impl Into<Rel<'b>>, args: &[T<'_>]) -> Self {
        if self.head {
            self.failed = Some(super::bad("a rule with two heads"));
        }
        self.atom_node(rel.into(), args, 0);
        self.head = true;
        self
    }
    pub fn atom(mut self, rel: impl Into<Rel<'b>>, args: &[T<'_>]) -> Self {
        self.body();
        self.atom_node(rel.into(), args, 0);
        self
    }
    /// An atom with flags: negated, gated.
    pub fn atom_with(mut self, rel: impl Into<Rel<'b>>, args: &[T<'_>], neg: bool, gated: bool) -> Self {
        self.body();
        self.atom_node(rel.into(), args, if neg { kind::NEG } else { 0 } | if gated { kind::GATED } else { 0 });
        self
    }
    /// A negated atom.
    pub fn not(mut self, rel: impl Into<Rel<'b>>, args: &[T<'_>]) -> Self {
        self.body();
        self.atom_node(rel.into(), args, kind::NEG);
        self
    }
    /// An `ast(document, node, matcher)` literal, its captures (name, variable) in order.
    pub fn ast(mut self, document: T<'_>, node: T<'_>, matcher: &str, captures: &[(&str, T<'_>)]) -> Self {
        self.body();
        let m = self.b.enc.atom(matcher);
        let at = self.b.enc.open(kind::AST, m);
        self.term(&document);
        self.term(&node);
        for (name, var) in captures {
            let a = self.b.enc.atom(name);
            self.b.enc.leaf(kind::STR, a);
            self.term(var);
        }
        self.b.enc.close(at);
        self
    }
    /// An aggregate `var = op over : { rel(args) }`, written as the encoder writes a tree's (`over` `None` for a
    /// count); `var` a variable. A join's keys follow `over`, a variable each, then its separator.
    pub fn agg(mut self, var: T<'_>, op: &crate::code_rule_dsl::logic::front::ast::AggOp, over: Option<T<'_>>, rel: impl Into<Rel<'b>>, args: &[T<'_>]) -> Self {
        use crate::code_rule_dsl::logic::front::ast::AggOp;
        self.body();
        let code: u8 = match op {
            AggOp::Count => 0,
            AggOp::Min => 1,
            AggOp::Max => 2,
            AggOp::Sum => 3,
            AggOp::Product => 4,
            AggOp::Join { .. } => 5,
        };
        let s = match var {
            T::Var(v) => {
                let a = self.b.enc.atom(v);
                self.b.enc.slot_of(a)
            }
            T::VarAtom(a) => self.b.enc.slot_of(a.0),
            _ => {
                self.failed = Some(super::bad("an aggregate's result that is not a variable"));
                return self;
            }
        };
        let at = self.b.enc.open(kind::AGG | code << 5, s);
        match over {
            Some(t) => self.term(&t),
            None => self.b.enc.leaf(kind::WILD, 0),
        }
        if let AggOp::Join { by, sep } = op {
            for k in by {
                self.term(&T::Var(k));
            }
            let a = self.b.enc.atom(sep);
            self.b.enc.leaf(kind::STR, a);
        }
        self.atom_node(rel.into(), args, 0);
        self.b.enc.close(at);
        self
    }
    pub fn cmp(mut self, l: T<'_>, op: CmpOp, r: T<'_>) -> Self {
        self.body();
        let at = self.b.enc.open(kind::CMP, op as u32);
        self.term(&l);
        self.term(&r);
        self.b.enc.close(at);
        self
    }
    /// A literal of `from`'s rule `rule`, copied: its variables by their names.
    pub fn lit(mut self, from: &Version, rule: RuleRef<'_>, lit: LitRef<'_>) -> Self {
        self.body();
        let same = self.b.same(rule);
        let vars: Vec<u32> = rule.vars().map(|a| a.0).collect();
        let n = lit.node();
        let size = rule.image().node(n).1;
        self.b.enc.copy_nodes(from, rule.image(), n..n + size, &vars, same);
        self
    }
    /// A literal of `from`'s rule `rule`, copied through `rw` (its variables renamed, terms replaced): a literal of one
    /// rule written into another (an unfolded read, a demand split's part).
    pub fn lit_with(mut self, from: &Version, rule: RuleRef<'_>, lit: LitRef<'_>, rw: &mut impl Rewrite) -> Self {
        self.body();
        let same = self.b.same(rule);
        let vars: Vec<u32> = rule.vars().map(|a| a.0).collect();
        let mut w = Walk { enc: &mut self.b.enc, from, im: rule.image(), vars: &vars, same, prefix: Vec::new() };
        w.lit(lit, rw);
        self
    }
    /// An atom of `from`'s rule `rule`, copied as this rule's head.
    pub fn head_from(mut self, from: &Version, rule: RuleRef<'_>, atom: super::AtomRef<'_>) -> Self {
        if self.head {
            self.failed = Some(super::bad("a rule with two heads"));
        }
        let same = self.b.same(rule);
        let vars: Vec<u32> = rule.vars().map(|a| a.0).collect();
        let n = atom.node();
        let size = rule.image().node(n).1;
        self.b.enc.copy_nodes(from, rule.image(), n..n + size, &vars, same);
        self.head = true;
        self
    }
    /// An atom of `from`'s rule `rule`, copied through `rw` as this rule's head (its variables renamed, terms replaced).
    pub fn head_with(mut self, from: &Version, rule: RuleRef<'_>, atom: super::AtomRef<'_>, rw: &mut impl Rewrite) -> Self {
        if self.head {
            self.failed = Some(super::bad("a rule with two heads"));
        }
        let same = self.b.same(rule);
        let vars: Vec<u32> = rule.vars().map(|a| a.0).collect();
        let mut w = Walk { enc: &mut self.b.enc, from, im: rule.image(), vars: &vars, same, prefix: Vec::new() };
        w.atom(atom, Site::Head(atom), rw);
        self.head = true;
        self
    }
    /// The builder the rule is written into, for its names (`atom`, `rel`).
    pub fn builder(&mut self) -> &mut Builder<'p> {
        self.b
    }
    /// Records rule `id` of the derivation's input as this rule's source (§9), as `Builder::rewrite` does: for a rule
    /// written term by term from one of the input's (a maintenance rule of a source rule, decision 0258).
    pub fn source(mut self, id: RuleId) -> Self {
        self.from = Some(id);
        self
    }
    /// The slot this rule gives the variable named `name` (new variables are numbered as they first appear).
    pub fn slot(&mut self, name: &str) -> Slot {
        let a = self.b.enc.atom(name);
        Slot(self.b.enc.slot_of(a))
    }
    /// The rule's id, once its head is written.
    pub fn finish(mut self) -> Result<RuleId, String> {
        if let Some(e) = self.failed.take().or_else(|| (!self.head).then(|| super::bad("a rule with no head"))) {
            return Err(e);
        }
        let k = self.b.enc.end_rule(self.at);
        if let Some(src) = self.from {
            self.b.enc.set_from(k, src);
        }
        self.done = true;
        Ok(RuleId(self.b.enc.rule_base() + k))
    }
}

impl Version {
    /// An edit of this version (`Builder::on`).
    pub fn builder(self: &Arc<Self>) -> Builder<'_> {
        Builder::on(self)
    }

}

/// Which relations' fields a part keeps (`Version::part`): its natives', its lattices' and its gates' (by key).
pub struct PartFields<'f> {
    pub natives: &'f dyn Fn(&str) -> bool,
    pub lattice: &'f dyn Fn(&str) -> bool,
    pub gates: &'f dyn Fn(&str) -> bool,
}

/// Writes term `t`.
fn write_t(e: &mut Encoder<'_>, t: &T<'_>) {
    match t {
        T::Var(v) => e.var(v),
        T::VarAtom(a) => {
            let s = e.slot_of(a.0);
            e.leaf(kind::VAR, s);
        }
        T::Wild => e.leaf(kind::WILD, 0),
        T::Top => e.leaf(kind::TOP, 0),
        T::Int(i) => e.int(*i),
        T::Branded(b) => e.leaf(kind::BRANDED | super::brand_code(b.class) << 5, b.atom),
        T::Str(s) => {
            let a = e.atom(s);
            e.leaf(kind::STR, a);
        }
        T::StrAtom(a) => e.leaf(kind::STR, a.0),
        T::CallAtom(a, args) => {
            let at = e.open(kind::CALL, a.0);
            for x in args {
                write_t(e, x);
            }
            e.close(at);
        }
        T::Call(f, args) => {
            let a = e.atom(f);
            let at = e.open(kind::CALL, a);
            for x in args {
                write_t(e, x);
            }
            e.close(at);
        }
    }
}

/// Where a term is, in the rule being rewritten: an argument of its head, of a body atom (an aggregate's included),
/// or a side of a comparison (a call's arguments are where the call is).
#[derive(Clone, Copy)]
pub enum Site<'v> {
    Head(AtomRef<'v>),
    Atom(AtomRef<'v>),
    Cmp(LitRef<'v>),
}

/// A rule rewritten as it is copied (`Builder::rewrite`). Each hook's default keeps what it is given, so a pass states
/// only what it changes, and never walks a rule's nodes itself.
pub trait Rewrite {
    /// Starts rule `r` of a rewrite of the whole program (`Builder::rewrite_all`): whether it is rewritten. A rule it
    /// leaves is kept as it is, copied with its neighbours as a range.
    fn begin(&mut self, _r: RuleRef<'_>) -> bool {
        true
    }
    /// Terms to write before atom `a`'s own arguments (a presence column, say), into `out`.
    fn prefix(&mut self, _a: AtomRef<'_>, _out: &mut Vec<T<'static>>) {}
    /// A term to write in place of `t` at `site`; `None` keeps it (and asks again of each of a call's arguments).
    fn term(&mut self, _site: Site<'_>, _t: TermRef<'_>) -> Option<T<'static>> {
        None
    }
    /// The relation (by name) and flags (`neg`, `gated`) to write atom `a` (a body atom, or an aggregate's) with;
    /// `None` keeps its own.
    fn atom_as(&mut self, _a: AtomRef<'_>) -> Option<(String, bool, bool)> {
        None
    }
    /// The relation (by name) to write the head `a` with; `None` keeps its own (a relation merged into another,
    /// `preparation::merge`).
    fn head_as(&mut self, _a: AtomRef<'_>) -> Option<String> {
        None
    }
    /// Whether to keep body literal `l`.
    fn keep(&mut self, _l: LitRef<'_>) -> bool {
        true
    }
    /// A term to write in place of argument `c` of atom `a` (a body atom, or an aggregate's), asked before `term`;
    /// `None` leaves it to `term`.
    fn arg(&mut self, _a: AtomRef<'_>, _c: usize, _t: TermRef<'_>) -> Option<T<'static>> {
        None
    }
    /// The kind set (by name) to write a `nearest` literal over set `set` with; `None` keeps its own.
    fn kindset(&mut self, _set: AtomId) -> Option<String> {
        None
    }
    /// Whether to leave out argument `c` of atom `a` (`match_all`'s regex, which its relation names instead).
    fn drop_arg(&mut self, _a: AtomRef<'_>, _c: usize) -> bool {
        false
    }
    /// The variable (by the atom naming it, in the builder's ids) to write in place of variable `s` where no term is
    /// asked for it (`nearest`'s variables), and where `term` keeps a variable term; `None` keeps its name.
    fn var(&mut self, _s: Slot) -> Option<AtomId> {
        None
    }
}

/// The copy a `Rewrite` steers: a rule's nodes, from `im` (of version `from`) into `enc`.
struct Walk<'a, 'e> {
    enc: &'a mut Encoder<'e>,
    from: &'a Version,
    im: &'a super::Image,
    vars: &'a [u32],
    same: bool,
    prefix: Vec<T<'static>>,
}

impl Walk<'_, '_> {
    fn open_mapped(&mut self, n: u32) -> usize {
        let kd = self.im.node(n).0;
        let v = self.enc.map_val(self.from, self.im, n, self.vars, self.same);
        self.enc.open(kd, v)
    }
    fn copy(&mut self, n: u32) {
        let size = self.im.node(n).1;
        self.enc.copy_nodes(self.from, self.im, n..n + size, self.vars, self.same);
    }
    /// `copy`, each variable leaf renamed where `rw.var` names it.
    fn copy_renamed(&mut self, n: u32, rw: &mut impl Rewrite) {
        let size = self.im.node(n).1;
        for m in n..n + size {
            let (kd, _, val) = self.im.node(m);
            match (kd & kind::MASK == kind::VAR).then(|| rw.var(Slot(val))).flatten() {
                Some(a) => {
                    let s = self.enc.slot_of(a.0);
                    self.enc.leaf(kind::VAR, s);
                }
                None => self.enc.copy_nodes(self.from, self.im, m..m + 1, self.vars, self.same),
            }
        }
    }
    fn atom(&mut self, a: AtomRef<'_>, site: Site<'_>, rw: &mut impl Rewrite) {
        let renamed = if matches!(site, Site::Head(_)) { rw.head_as(a).map(|rel| (rel, a.neg(), a.gated())) } else { rw.atom_as(a) };
        let at = match renamed {
            Some((rel, neg, gated)) => {
                let r = self.enc.rel(&rel);
                self.enc.open(kind::ATOM | if neg { kind::NEG } else { 0 } | if gated { kind::GATED } else { 0 }, r)
            }
            None => self.open_mapped(a.node()),
        };
        self.prefix.clear();
        rw.prefix(a, &mut self.prefix);
        for t in std::mem::take(&mut self.prefix) {
            write_t(self.enc, &t);
        }
        let n = a.node();
        let end = n + self.im.node(n).1;
        let mut c = n + 1;
        let mut k = 0;
        while c < end {
            if !rw.drop_arg(a, k) {
                match (!matches!(site, Site::Head(_))).then(|| rw.arg(a, k, super::handles::term(self.im, c))).flatten() {
                    Some(t) => write_t(self.enc, &t),
                    None => self.term(c, site, rw),
                }
            }
            k += 1;
            c += self.im.node(c).1;
        }
        self.enc.close(at);
    }
    fn term(&mut self, n: u32, site: Site<'_>, rw: &mut impl Rewrite) {
        let t = super::handles::term(self.im, n);
        if let Some(t) = rw.term(site, t) {
            write_t(self.enc, &t);
            return;
        }
        if let TermRef::Call(..) = t {
            let at = self.open_mapped(n);
            let end = n + self.im.node(n).1;
            let mut c = n + 1;
            while c < end {
                self.term(c, site, rw);
                c += self.im.node(c).1;
            }
            self.enc.close(at);
        } else {
            self.copy_renamed(n, rw);
        }
    }
    fn lit(&mut self, l: LitRef<'_>, rw: &mut impl Rewrite) {
        let n = l.node();
        let (kd, size, _) = self.im.node(n);
        match kd & kind::MASK {
            kind::ATOM => {
                let a = l.atom().expect("an atom literal");
                self.atom(a, Site::Atom(a), rw);
            }
            kind::CMP => {
                let at = self.open_mapped(n);
                let mut c = n + 1;
                while c < n + size {
                    self.term(c, Site::Cmp(l), rw);
                    c += self.im.node(c).1;
                }
                self.enc.close(at);
            }
            kind::AGG => {
                // Its atom rewritten; its variables (the result, `over`) renamed as everywhere else in the rule
                // (`Rewrite::var`), so a variable is one name throughout. The result is the node's own value.
                let at = match rw.var(Slot(self.im.node(n).2)) {
                    Some(a) => {
                        let s = self.enc.slot_of(a.0);
                        self.enc.open(kd, s)
                    }
                    None => self.open_mapped(n),
                };
                let mut c = n + 1;
                while c < n + size {
                    if self.im.node(c).0 & kind::MASK == kind::ATOM {
                        let a = AtomRef::new(self.im, c);
                        self.atom(a, Site::Atom(a), rw);
                    } else {
                        self.copy_renamed(c, rw);
                    }
                    c += self.im.node(c).1;
                }
                self.enc.close(at);
            }
            kind::NEAREST => {
                let Lit::Nearest { kindset, .. } = l.view() else { unreachable!("a nearest literal") };
                match rw.kindset(kindset) {
                    Some(set) => {
                        let a = self.enc.atom(&set);
                        let at = self.enc.open(kd, a);
                        let mut c = n + 1;
                        while c < n + size {
                            self.copy_renamed(c, rw);
                            c += self.im.node(c).1;
                        }
                        self.enc.close(at);
                    }
                    None => self.copy_renamed(n, rw),
                }
            }
            _ => self.copy_renamed(n, rw),
        }
    }
}
