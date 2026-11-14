//! Making a version (decision 0173 §6, §7): a `Builder` states an edit of a parent (or a new base) and makes the
//! version that costs it, never a copy of the parent.
//!
//! ```text
//! let mut b = Builder::on(&parent);
//! b.remove(rule.id());                                    // a rule of the parent
//! b.add(&tree_rule)?;                                     // a rule from the tree
//! b.copy(&other, other_rule);                             // a rule of another version, by its handles
//! b.rule().head("p", &[T::Var("x")]).atom("e", &[T::Var("x"), T::Wild]).finish()?;
//! b.declare("p", &[("x", "int")]);  b.set_inputs(&["e"]);
//! let (v, made) = b.finish()?;                            // a view, an overlay, or folded
//! ```
//!
//! What it makes:
//! - **a view** when the edit only removes rules of a base or a view, and what is left is few ranges;
//! - **an overlay** otherwise: a delta holding only what it adds, numbered after the parent's ids;
//! - **a fold** when the edit reaches half its parent: a new base of the result, keeping its atom and relation ids.
//!
//! A version's digest is made with it, from the parent's and the edit's hashes.
use super::encoder::{self, CellText, Decl, Encoder, FactRel, Parts};
use super::handles::{CmpOp, LitRef, RuleRef};
use super::version::{MAX_RANGES, Made, Version};
use super::{ABOUT_BIT, AtomId, Cell, RelId, RuleId, Slot, kind};
use crate::code_rule_dsl::logic::front::{List, Program, Rule, Value};
use crate::code_rule_dsl::logic::program_facts::Facts;
use std::sync::Arc;

/// A term to write, by text or by id.
#[derive(Clone, Debug)]
pub enum T<'a> {
    Var(&'a str),
    /// A variable of the rule being written, by the atom naming it.
    VarAtom(AtomId),
    Wild,
    Top,
    Int(i64),
    Str(&'a str),
    StrAtom(AtomId),
    Call(&'a str, Vec<T<'a>>),
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
    /// A new base of `prog`, every part its own. Refuses what the image has no form for (maintenance's parts, which
    /// the text refuses too) and a comparison it does not know.
    pub fn of_program(prog: &Program) -> Result<Builder<'static>, String> {
        if !prog.again.is_empty() || !prog.together.is_empty() || !prog.shapes.is_empty() {
            return Err(super::bad("maintenance's parts (again, together, shapes) have no image"));
        }
        let mut b = Builder::base();
        for r in &prog.rules {
            b.add(r)?;
        }
        for (n, c) in &prog.decls {
            b.declare_names(n, c.as_slice());
        }
        for (r, rows) in &prog.facts {
            b.set_facts(r, rows);
        }
        b.set_inputs(&prog.inputs);
        b.set_outputs(&prog.outputs);
        b.set_about(&super::about_of(prog));
        Ok(b)
    }
}

impl<'p> Builder<'p> {
    /// An edit of `parent`.
    pub fn on(parent: &'p Arc<Version>) -> Builder<'p> {
        Builder { parent: Some(parent), ids: Some(parent), enc: Encoder::new(Some(parent)), removed: Vec::new(), parts: Parts::default(), parts_changed: false, rows: 0 }
    }
    /// A base of all of `v`, under `v`'s atom and relation ids.
    pub(crate) fn folding(v: &'p Version) -> Builder<'p> {
        let mut b = Builder { parent: None, ids: Some(v), enc: Encoder::with_ids_of(v), removed: Vec::new(), parts: Parts::default(), parts_changed: false, rows: 0 };
        for r in v.rules() {
            b.copy(v, r);
        }
        for d in v.decls() {
            let cols = d.cols().map(|(n, t)| (n.0, t.0)).collect();
            b.parts.decls.insert(d.name().0, Decl { cols, hash: d.hash() });
        }
        for f in v.all_facts().into_iter().chain(v.about_facts()) {
            let rows = f.rows().map(|r| r.collect()).collect();
            b.parts.facts.insert(f.name().0 | if f.about() { ABOUT_BIT } else { 0 }, FactRel { rows, hash: f.hash() });
        }
        b.parts.inputs = Some((v.inputs().map(|a| a.0).collect(), v.inputs_hash()));
        b.parts.outputs = Some((v.outputs().map(|a| a.0).collect(), v.outputs_hash()));
        b.parts.owns_about = true;
        b
    }

    /// The edit's size: rules removed and added, and part rows stated.
    pub fn size(&self) -> usize {
        self.removed.len() + self.enc.rules() as usize + self.parts.decls.len() + self.parts.decls_gone.len() + self.rows + self.parts.facts_gone.len()
    }

    // -- names --

    /// The atom of `text`: the parent's, or a new one.
    pub fn atom(&mut self, text: &str) -> AtomId {
        AtomId(self.enc.atom(text))
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
    fn next_id(&self) -> RuleId {
        RuleId(self.enc.rule_base() + self.enc.rules())
    }
    /// Adds `rule`, from the tree.
    pub fn add(&mut self, rule: &Rule) -> Result<RuleId, String> {
        let id = self.next_id();
        self.enc.tree_rule(rule)?;
        Ok(id)
    }
    /// Adds a copy of `rule`, one of `from`'s rules: its nodes as they are when `from`'s ids are this builder's,
    /// else each name by its text.
    pub fn copy(&mut self, from: &Version, rule: RuleRef<'_>) -> RuleId {
        let id = self.next_id();
        let same = self.same(rule);
        let at = self.enc.begin_rule();
        let n = rule.nodes();
        let vars: Vec<u32> = rule.vars().map(|a| a.0).collect();
        self.enc.copy_nodes(from, rule.image(), n.start + 1..n.end, &vars, same);
        self.enc.end_rule(at);
        id
    }
    fn same(&self, rule: RuleRef<'_>) -> bool {
        self.ids.is_some_and(|v| v.holds_image(rule.image()))
    }
    /// Starts a rule written term by term.
    pub fn rule(&mut self) -> RuleBuilder<'_, 'p> {
        let at = self.enc.begin_rule();
        RuleBuilder { b: self, at, head: false, failed: None, done: false }
    }

    // -- parts --

    fn declare_names(&mut self, rel: &str, cols: &[(crate::code_rule_dsl::logic::front::Name, crate::code_rule_dsl::logic::front::Name)]) {
        let pairs: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        self.declare(rel, &pairs);
    }
    /// Declares relation `rel`'s columns (names and types), replacing the parent's.
    pub fn declare(&mut self, rel: &str, cols: &[(&str, &str)]) {
        let a = self.enc.atom(rel);
        let ids = cols.iter().map(|&(n, t)| (self.enc.atom(n), self.enc.atom(t))).collect();
        self.parts.decls_gone.remove(&a);
        self.parts.decls.insert(a, Decl { cols: ids, hash: encoder::decl_hash(rel, cols.iter().copied()) });
        self.parts_changed = true;
    }
    /// Removes the parent's declaration of `rel`.
    pub fn undeclare(&mut self, rel: &str) {
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
        }).collect()).collect()
    }
    fn hash_rows(rel: &str, about: bool, rows: &[List<Value>]) -> u64 {
        encoder::facts_hash(rel, about, rows.iter().map(|row| row.iter().map(|v| match v {
            Value::Int(i) => CellText::Int(*i),
            Value::Str(s) => CellText::Str(s.as_str()),
        })))
    }
    /// States relation `rel`'s facts, replacing the parent's.
    pub fn set_facts(&mut self, rel: &str, rows: &[List<Value>]) {
        let a = self.enc.atom(rel);
        let cells = self.rows_of(rows);
        self.parts.facts_gone.remove(&a);
        self.parts.facts.insert(a, FactRel { rows: cells, hash: Self::hash_rows(rel, false, rows) });
        self.rows += rows.len().max(1);
        self.parts_changed = true;
    }
    /// Removes the parent's facts of `rel`.
    pub fn drop_facts(&mut self, rel: &str) {
        let a = self.enc.atom(rel);
        self.parts.facts.remove(&a);
        if self.parent.is_some_and(|p| p.facts(AtomId(a)).is_some()) {
            self.parts.facts_gone.insert(a);
        }
        self.parts_changed = true;
    }
    pub fn set_inputs<S: AsRef<str>>(&mut self, names: &[S]) {
        let ids = names.iter().map(|n| self.enc.atom(n.as_ref())).collect();
        self.parts.inputs = Some((ids, encoder::names_hash(0, names.iter().map(|n| n.as_ref()))));
        self.parts_changed = true;
    }
    pub fn set_outputs<S: AsRef<str>>(&mut self, names: &[S]) {
        let ids = names.iter().map(|n| self.enc.atom(n.as_ref())).collect();
        self.parts.outputs = Some((ids, encoder::names_hash(1, names.iter().map(|n| n.as_ref()))));
        self.parts_changed = true;
    }
    /// States the program's other fields (`about_of`), all of them.
    pub fn set_about(&mut self, about: &Facts) {
        self.parts.facts.retain(|k, _| k & ABOUT_BIT == 0);
        for (rel, rows) in about {
            let a = self.enc.atom(rel);
            let cells = self.rows_of(rows);
            self.parts.facts.insert(a | ABOUT_BIT, FactRel { rows: cells, hash: Self::hash_rows(rel, true, rows) });
        }
        self.parts.owns_about = true;
        self.parts_changed = true;
    }

    // -- the version --

    /// The image's bytes: a base's, or a delta's over the parent.
    pub fn finish_bytes(mut self) -> Result<Vec<u8>, String> {
        self.removed.sort_unstable();
        self.removed.dedup();
        if self.parent.is_none() && (!self.removed.is_empty() || self.parts.inputs.is_none() || self.parts.outputs.is_none() || !self.parts.owns_about) {
            if !self.removed.is_empty() {
                return Err(super::bad("a base removes nothing"));
            }
            // A base owns every part: what was not stated is empty.
            if self.parts.inputs.is_none() {
                self.set_inputs::<&str>(&[]);
            }
            if self.parts.outputs.is_none() {
                self.set_outputs::<&str>(&[]);
            }
            if !self.parts.owns_about {
                self.set_about(&Facts::new());
            }
        }
        Ok(self.enc.finish(&self.parts, &self.removed))
    }

    /// The version the edit makes: a view, an overlay, or folded (`Builder`'s doc).
    pub fn finish(mut self) -> Result<(Arc<Version>, Made), String> {
        let Some(parent) = self.parent else {
            return Ok((Version::open(&self.finish_bytes()?)?, Made::Folded));
        };
        self.removed.sort_unstable();
        self.removed.dedup();
        if self.enc.rules() == 0 && !self.parts_changed
            && let Some(v) = view(parent, &self.removed)
        {
            return Ok((v, Made::View));
        }
        let size = self.size();
        let overlay = Version::over(parent, &self.finish_bytes()?)?;
        if 2 * size >= parent.size().max(1) {
            return Ok((overlay.fold()?, Made::Folded));
        }
        Ok((overlay, Made::Overlay))
    }
}

/// The view of `parent` (a base or a view) less rules `removed`, if it is few enough ranges.
fn view(parent: &Arc<Version>, removed: &[u32]) -> Option<Arc<Version>> {
    let (base, ranges): (_, Vec<(u32, u32)>) = match &**parent {
        Version::Base { image, .. } => (image, vec![(0, image.rules() as u32)]),
        Version::View { base, ranges, .. } => (base, ranges.clone()),
        Version::Overlay { .. } => return None,
    };
    let mut out = Vec::new();
    let mut gone = removed.iter().peekable();
    let mut digest = parent.digest();
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
            digest = digest.wrapping_sub(base.rule_hash(g));
            if g > start {
                out.push((start, g));
            }
            start = g + 1;
        }
        if start < e {
            out.push((start, e));
        }
        if out.len() > MAX_RANGES {
            return None;
        }
    }
    Some(Version::view(base, out, digest))
}

/// A rule written term by term: its head, then its body's literals, in order. A rule refused (`finish`'s error) or
/// dropped unfinished is not written.
pub struct RuleBuilder<'b, 'p> {
    b: &'b mut Builder<'p>,
    at: usize,
    head: bool,
    failed: Option<String>,
    done: bool,
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
        let e = &mut self.b.enc;
        match t {
            T::Var(v) => e.var(v),
            T::VarAtom(a) => {
                let s = e.slot_of(a.0);
                e.leaf(kind::VAR, s);
            }
            T::Wild => e.leaf(kind::WILD, 0),
            T::Top => e.leaf(kind::TOP, 0),
            T::Int(i) => e.int(*i),
            T::Str(s) => {
                let a = e.atom(s);
                e.leaf(kind::STR, a);
            }
            T::StrAtom(a) => e.leaf(kind::STR, a.0),
            T::Call(f, args) => {
                let a = e.atom(f);
                let at = e.open(kind::CALL, a);
                for x in args {
                    self.term(x);
                }
                self.b.enc.close(at);
            }
        }
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
    /// A negated atom.
    pub fn not(mut self, rel: impl Into<Rel<'b>>, args: &[T<'_>]) -> Self {
        self.body();
        self.atom_node(rel.into(), args, kind::NEG);
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
        self.done = true;
        Ok(RuleId(self.b.enc.rule_base() + k))
    }
}

impl Version {
    /// An edit of this version (`Builder::on`).
    pub fn builder(self: &Arc<Self>) -> Builder<'_> {
        Builder::on(self)
    }

    /// `to` as an edit of this version: its rules matched by hash, its parts by relation. For a caller that holds
    /// both programs (a test, the trace's replay); a pass states its edit instead.
    pub fn diff<'p>(self: &'p Arc<Self>, to: &Program) -> Result<Builder<'p>, String> {
        let mut want: std::collections::HashMap<u64, Vec<usize>> = std::collections::HashMap::new();
        let mut scratch = Encoder::new(None);
        for (k, r) in to.rules.iter().enumerate() {
            let at = scratch.tree_rule(r)?;
            want.entry(scratch.rule_hash_of(at)).or_default().push(k);
        }
        let mut b = Builder::on(self);
        let mut kept = vec![false; to.rules.len()];
        for r in self.rules() {
            match want.get_mut(&r.hash()).and_then(Vec::pop) {
                Some(k) => kept[k] = true,
                None => b.remove(r.id()),
            }
        }
        for (r, k) in to.rules.iter().zip(&kept) {
            if !k {
                b.add(r)?;
            }
        }
        for (n, c) in &to.decls {
            let cols: Vec<(&str, &str)> = c.iter().map(|(a, t)| (a.as_str(), t.as_str())).collect();
            if self.decl_of(n).is_none_or(|d| d.hash() != encoder::decl_hash(n, cols.iter().copied())) {
                b.declare(n, &cols);
            }
        }
        for d in self.decls() {
            if !to.decls.contains_key(self.text(d.name())) {
                b.undeclare(self.text(d.name()));
            }
        }
        for (rel, rows) in &to.facts {
            if self.find(rel).and_then(|a| self.facts(a)).is_none_or(|f| f.hash() != Builder::hash_rows(rel, false, rows)) {
                b.set_facts(rel, rows);
            }
        }
        for f in self.all_facts() {
            if !to.facts.contains_key(self.text(f.name())) {
                b.drop_facts(self.text(f.name()));
            }
        }
        let about = super::about_of(to);
        let old: std::collections::BTreeMap<&str, u64> = self.about_facts().into_iter().map(|f| (self.text(f.name()), f.hash())).collect();
        let new: std::collections::BTreeMap<&str, u64> = about.iter().map(|(r, rows)| (r.as_str(), Builder::hash_rows(r, true, rows))).collect();
        if old != new {
            b.set_about(&about);
        }
        if self.inputs_hash() != encoder::names_hash(0, to.inputs.iter().map(String::as_str)) {
            b.set_inputs(&to.inputs);
        }
        if self.outputs_hash() != encoder::names_hash(1, to.outputs.iter().map(String::as_str)) {
            b.set_outputs(&to.outputs);
        }
        Ok(b)
    }
}
