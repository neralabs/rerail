//! A program read from its image (`body::Placed::of_program`, decision 0173): its other fields beside its rules, by
//! name, as the tree passes read `Program`'s (`keeps::choose`, `demand::split_negations`; t36 item 6), and as `plan::run`
//! reads them (t36 item 7). A maintained program's `again` and `together` are not here: they are `body::Maint`'s.

use super::super::program_image::{About, AtomId, Cell, DeclRef, FactsRef, GateRef, LatticeRef, NativeRef, RelId, ShapeRef, Version};
use super::body::Placed;
use std::collections::BTreeSet;

/// What a version's fields read once: its `About`, and its inputs and outputs. Held by its `Placed`, and shared by
/// every clone of it; or a pipeline's analysis (decision 0281 §2).
#[derive(Debug, PartialEq, Eq)]
pub struct FieldsData {
    pub about: About,
    inputs: Vec<AtomId>,
    outputs: Vec<AtomId>,
    input_set: BTreeSet<AtomId>,
    output_set: BTreeSet<AtomId>,
    /// The relations a rule derives (its head's).
    heads: BTreeSet<RelId>,
}

impl FieldsData {
    fn of(v: &Version) -> FieldsData {
        let inputs: Vec<AtomId> = v.inputs().collect();
        let outputs: Vec<AtomId> = v.outputs().collect();
        let heads = v.rules().map(|r| r.head().rel()).collect();
        FieldsData { about: About::of(v), input_set: inputs.iter().copied().collect(), output_set: outputs.iter().copied().collect(), inputs, outputs, heads }
    }
}

/// A version's fields, read once (at the first `of` of its `Placed`) and borrowed after.
#[derive(Clone, Copy)]
pub struct Fields<'v> {
    pub v: &'v Version,
    pub about: &'v About,
    d: &'v FieldsData,
}

impl super::super::program_image::manager::Analysis for FieldsData {
    const NAME: &'static str = "fields";
    const READS: super::super::program_image::manager::Reads = {
        use super::super::program_image::manager::Reads;
        Reads::RULES.or(Reads::ABOUT).or(Reads::INPUTS).or(Reads::OUTPUTS)
    };
    fn compute(cx: &super::super::program_image::manager::Cx<'_>) -> Result<FieldsData, String> {
        Ok(FieldsData::of(cx.v()))
    }
}

impl Placed {
    /// This version's fields (`Fields::of`), or an error where it is its rules alone: a program the image has no form
    /// for.
    pub fn fields(&self) -> super::Result<Fields<'_>> {
        Fields::of(self).ok_or_else(|| super::Error::Plan("the program has no image: its fields are read from its version".into()))
    }
}

impl<'v> Fields<'v> {
    /// `p`'s fields: `None` where its version is its rules alone (`Placed::whole`).
    pub fn of(p: &'v Placed) -> Option<Fields<'v>> {
        if !p.whole {
            return None;
        }
        let v: &Version = &p.v;
        // The version's (decision 0281 §2), so every `Placed` of it shares one.
        let d: &FieldsData = p.fields.get_or_init(|| super::super::program_image::manager::analysis::<FieldsData>(&p.v).expect("a version's fields are read"));
        Some(Fields { v, about: &d.about, d })
    }
    /// The fields of `v` as `d` read them: a pipeline's (`cx.get::<FieldsData>()`), for a whole version.
    pub fn over(v: &'v Version, d: &'v FieldsData) -> Fields<'v> {
        Fields { v, about: &d.about, d }
    }
    fn atom(&self, name: &str) -> Option<AtomId> {
        self.v.find(name)
    }
    fn rel(&self, rel: &str) -> Option<RelId> {
        self.v.rel(rel)
    }
    /// An atom's text.
    pub fn text(&self, a: AtomId) -> &'v str {
        self.v.text(a)
    }
    /// A relation's name.
    pub fn rel_name(&self, r: RelId) -> &'v str {
        self.v.rel_name(r)
    }

    // -- inputs and outputs --

    /// The inputs, in the program's order (`Program::inputs`).
    pub fn inputs(&self) -> impl ExactSizeIterator<Item = &'v str> + 'v {
        let v = self.v;
        self.d.inputs.iter().map(move |&a| v.text(a))
    }
    /// The outputs, in the program's order (`Program::outputs`).
    pub fn outputs(&self) -> impl ExactSizeIterator<Item = &'v str> + 'v {
        let v = self.v;
        self.d.outputs.iter().map(move |&a| v.text(a))
    }
    pub fn is_output(&self, rel: &str) -> bool {
        self.atom(rel).is_some_and(|a| self.d.output_set.contains(&a))
    }
    pub fn is_input(&self, rel: &str) -> bool {
        self.atom(rel).is_some_and(|a| self.d.input_set.contains(&a))
    }

    /// Whether a rule of the program derives `rel` (a rule's head).
    pub fn derives(&self, rel: &str) -> bool {
        self.rel(rel).is_some_and(|r| self.d.heads.contains(&r))
    }

    // -- declarations --

    /// `rel`'s declaration (`Program::decls`).
    pub fn decl(&self, rel: &str) -> Option<DeclRef<'v>> {
        self.v.decl_of(rel)
    }
    /// Every declaration, by atom.
    pub fn decls(&self) -> Vec<DeclRef<'v>> {
        self.v.decls()
    }
    pub fn declared(&self, rel: &str) -> bool {
        self.decl(rel).is_some()
    }
    /// `rel`'s declared columns' count, if it is declared.
    pub fn arity(&self, rel: &str) -> Option<usize> {
        self.decl(rel).map(|d| d.cols().len())
    }
    /// `rel`'s declared columns, (name, type) each.
    pub fn columns(&self, rel: &str) -> Option<Vec<(&'v str, &'v str)>> {
        let v = self.v;
        self.decl(rel).map(|d| d.cols().map(|(n, t)| (v.text(n), v.text(t))).collect())
    }
    /// `rel`'s declared columns' count, failing where it is not declared (as `Program::decls[rel]` does).
    pub fn width(&self, rel: &str) -> usize {
        self.arity(rel).unwrap_or_else(|| panic!("no declaration of {rel}"))
    }
    /// `rel`'s declared columns' types, in order, failing where it is not declared.
    pub fn types(&self, rel: &str) -> Vec<&'v str> {
        let v = self.v;
        self.decl(rel).unwrap_or_else(|| panic!("no declaration of {rel}")).cols().map(|(_, t)| v.text(t)).collect()
    }
    /// Every declaration's relation and its columns' types, by name (`Program::decls`' order).
    pub fn decls_by_name(&self) -> Vec<(&'v str, Vec<&'v str>)> {
        let v = self.v;
        let mut out: Vec<(&'v str, Vec<&'v str>)> = self.v.decls().into_iter().map(|d| (v.text(d.name()), d.cols().map(|(_, t)| v.text(t)).collect())).collect();
        out.sort_unstable_by(|a, b| a.0.cmp(b.0));
        out
    }
    /// `rel`'s column `c`'s type.
    pub fn column_type(&self, rel: &str, c: usize) -> Option<&'v str> {
        self.decl(rel).and_then(|d| d.cols().nth(c)).map(|(_, t)| self.v.text(t))
    }

    // -- facts --

    /// `rel`'s stated facts (`Program::facts`), if it has an entry.
    pub fn facts(&self, rel: &str) -> Option<FactsRef<'v>> {
        self.atom(rel).and_then(|a| self.v.facts(a))
    }
    /// Every relation with an entry of facts (some may have no rows), by atom.
    pub fn fact_rels(&self) -> Vec<FactsRef<'v>> {
        self.v.all_facts()
    }
    /// Whether `rel` has facts, one row or more.
    pub fn has_facts(&self, rel: &str) -> bool {
        self.facts(rel).is_some_and(|f| f.rows().len() > 0)
    }
    /// A fact cell's text, as `Value`'s `Display` writes it (a string quoted, a number bare).
    pub fn cell_text(&self, c: Cell) -> String {
        match c {
            Cell::Int(i) => i.to_string(),
            Cell::Str(a) => format!("{:?}", self.v.text(a)),
            Cell::Branded(b) => b.to_string(),
        }
    }

    // -- the other fields --

    /// `Program::qualified`.
    pub fn qualified(&self, rel: &str, word: &str) -> bool {
        self.atom(rel).zip(self.atom(word)).is_some_and(|(r, w)| self.about.qualified(r, w))
    }
    /// `rel`'s qualifiers (`Program::qualifiers`), in atom order.
    pub fn qualifiers_of(&self, rel: &str) -> Vec<&'v str> {
        let Some(r) = self.atom(rel) else { return Vec::new() };
        let about: &'v About = self.about;
        about.qualifiers.iter().filter(|q| q.0 == r).map(|q| self.v.text(q.1)).collect()
    }
    /// Whether `rel` is demanded (`Program::demand`) or a gate (`Program::gates`' keys).
    pub fn demanded_or_gate(&self, rel: &str) -> bool {
        self.demanded(rel) || self.gate(rel)
    }
    /// Whether `rel` is demanded (`Program::demand`).
    pub fn demanded(&self, rel: &str) -> bool {
        self.atom(rel).is_some_and(|a| self.about.demand.binary_search(&a).is_ok())
    }
    /// The demanded relations in the program's order (`Program::demand`).
    pub fn demand(&self) -> impl ExactSizeIterator<Item = &'v str> + 'v {
        let v = self.v;
        self.about.demand_list.iter().map(move |&a| v.text(a))
    }
    /// Whether `rel` is a gate (`Program::gates`' keys).
    pub fn gate(&self, rel: &str) -> bool {
        self.atom(rel).is_some_and(|a| self.about.gate_keys.binary_search(&a).is_ok())
    }
    /// The gates in the program's order (`Program::gates`).
    pub fn gates(&self) -> &'v [GateRef] {
        &self.about.gate_list
    }
    /// Whether `rel` is a request or a relation an answer produces (`Program::answers`).
    pub fn answer(&self, rel: &str) -> bool {
        self.rel(rel).is_some_and(|r| self.about.answers.iter().any(|&(q, p)| q == r || p == r))
    }
    /// Whether `rel` is a request (`Program::answers`' keys).
    pub fn request(&self, rel: &str) -> bool {
        self.rel(rel).is_some_and(|r| self.about.answers.iter().any(|&(q, _)| q == r))
    }
    /// The relations request `req` produces, in order (`Program::answers[req]`).
    pub fn answers_of(&self, req: &str) -> Vec<&'v str> {
        let Some(q) = self.rel(req) else { return Vec::new() };
        let about: &'v About = self.about;
        about.answers.iter().filter(|a| a.0 == q).map(|a| self.v.rel_name(a.1)).collect()
    }
    /// Whether `rel` is a relation an answer produces (`answers::produced`' keys).
    pub fn produced(&self, rel: &str) -> bool {
        self.rel(rel).is_some_and(|r| self.about.answers.iter().any(|&(_, p)| p == r))
    }
    /// The request that produces `rel`, as `answers::produced` maps it: of several, the last by name.
    pub fn request_of(&self, rel: &str) -> Option<&'v str> {
        let r = self.rel(rel)?;
        let v = self.v;
        self.about.answers.iter().filter(|&&(_, p)| p == r).map(|&(q, _)| v.rel_name(q)).max()
    }
    /// Every request and a relation it produces, by request then in order (`Program::answers`).
    pub fn answers(&self) -> &'v [(RelId, RelId)] {
        &self.about.answers
    }
    pub fn nullary(&self, rel: &str) -> bool {
        self.rel(rel).is_some_and(|r| self.about.nullary.binary_search(&r).is_ok())
    }
    /// The relations of no columns (`Program::nullary`).
    pub fn nullaries(&self) -> impl Iterator<Item = &'v str> + 'v {
        let v = self.v;
        self.about.nullary.iter().map(move |&r| v.rel_name(r))
    }
    pub fn lattice(&self, rel: &str) -> bool {
        self.lattice_decl(rel).is_some()
    }
    /// `rel`'s lattice declaration (`Program::lattice[rel]`).
    pub fn lattice_decl(&self, rel: &str) -> Option<&'v LatticeRef> {
        let about: &'v About = self.about;
        self.rel(rel).and_then(|r| about.lattice_decl(r))
    }
    /// `LatticeDecl::lattice`: `rel`'s merge's name, else its value column's type.
    pub fn lattice_name(&self, rel: &str) -> Option<&'v str> {
        self.lattice_decl(rel).map(|l| self.v.text(l.lattice))
    }
    /// Every lattice relation, by relation id.
    pub fn lattices(&self) -> &'v [LatticeRef] {
        &self.about.lattice
    }
    pub fn native(&self, rel: &str) -> Option<&'v NativeRef> {
        let about: &'v About = self.about;
        self.rel(rel).and_then(|r| about.native(r))
    }
    /// Every native relation, by relation id (`Program::natives`).
    pub fn natives(&self) -> &'v [NativeRef] {
        &self.about.natives
    }
    /// The relations natives read (`Native::reads`), by name.
    pub fn natives_read(&self) -> BTreeSet<&'v str> {
        self.about.natives.iter().flat_map(|n| n.reads.iter()).map(|&r| self.v.rel_name(r)).collect()
    }
    /// `rel`'s columns declared `tape` (`Program::tapes[rel]`), in order.
    pub fn tapes_of(&self, rel: &str) -> Vec<usize> {
        let Some(r) = self.rel(rel) else { return Vec::new() };
        self.about.tapes.iter().filter(|t| t.0 == r).map(|t| t.1).collect()
    }
    /// Every relation's columns declared `tape`, sorted.
    pub fn tapes(&self) -> &'v [(RelId, usize)] {
        &self.about.tapes
    }
    /// Kind set `set`'s kinds, in order (`Program::kindsets[set]`).
    pub fn kindset(&self, set: &str) -> Option<&'v [AtomId]> {
        let about: &'v About = self.about;
        about.kindsets.iter().find(|k| self.v.text(k.0) == set).map(|k| k.1.as_slice())
    }
    /// Every kind set and its kinds, by name.
    pub fn kindsets(&self) -> &'v [(AtomId, Vec<AtomId>)] {
        &self.about.kindsets
    }
    /// The join shape directives, in the program's order (`Program::shapes`).
    pub fn shapes(&self) -> &'v [ShapeRef] {
        &self.about.shapes
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::front::{Atom, LatticeDecl, Literal, Native, Program, Rule, Term};
    use super::super::body::{Placed, Versions};
    use super::Fields;

    /// A version's fields are read once, shared by its clones, and say the tree's fields.
    #[test]
    fn fields_are_read_once_and_say_the_trees() {
        let atom = |rel: &str, args: &[&str]| Atom { rel: rel.into(), args: args.iter().map(|a| Term::Var((*a).into())).collect::<Vec<_>>().into(), neg: false, gated: false };
        let mut p = Program::default();
        p.decls.insert("e".into(), [("a".into(), "int".into()), ("b".into(), "sym".into())].into());
        p.inputs = vec!["e".into(), "s".into()];
        p.outputs = vec!["best".into(), "far".into()];
        p.rules = vec![Rule { head: atom("best", &["x", "y"]), body: vec![Literal::Atom(atom("e", &["x", "y"]))].into() }];
        p.lattice.insert("best".into(), LatticeDecl { keys: 1, ty: "sym".into(), merge: Some("type_join".into()), widen: Some(2) });
        p.natives.insert("far".into(), Native::DfsFirst { edge: "e".into(), target: "s".into(), roots: Some("s".into()) });
        let placed = Placed::of_program(&p).unwrap();
        let f = Fields::of(&placed).unwrap();
        assert!(std::ptr::eq(f.about, Fields::of(&placed.clone()).unwrap().about));
        assert_eq!(f.inputs().collect::<Vec<_>>(), ["e", "s"]);
        assert_eq!(f.outputs().collect::<Vec<_>>(), ["best", "far"]);
        assert_eq!(f.columns("e"), Some(vec![("a", "int"), ("b", "sym")]));
        let l = f.lattice_decl("best").unwrap();
        assert_eq!((l.keys, f.text(l.ty), l.merge.map(|m| f.text(m)), l.widen, f.lattice_name("best")), (1, "sym", Some("type_join"), Some(2), Some("type_join")));
        match &f.native("far").unwrap().spec {
            super::super::super::program_image::NativeSpec::DfsFirst { edge, target, roots } => {
                assert_eq!((f.rel_name(*edge), f.rel_name(*target), roots.map(|r| f.rel_name(r))), ("e", "s", Some("s")));
            }
            s => panic!("{s:?}"),
        }
        // The run's version 0 is the program's.
        let vs = Versions::with(&p, placed.clone());
        assert!(std::ptr::eq(vs.fields(0).unwrap().about, f.about));
        assert!(vs.fields(1).is_none());
        // A whole program added (a maintained form) is a whole version of its own.
        let mut vs = vs;
        let mut form = Program::default();
        form.decls.insert("e".into(), [("a".into(), "int".into())].into());
        form.inputs = vec!["e".into()];
        let k = vs.push(Placed::of_program(&form).unwrap());
        assert_eq!(k, 1);
        assert_eq!((vs.whole(k).inputs().collect::<Vec<_>>(), vs.whole(k).columns("e")), (vec!["e"], Some(vec![("a", "int")])));
        assert!(vs.fields(2).is_none());
        // A version of rules alone has none.
        assert!(Fields::of(&Placed::made(&p.rules).unwrap()).is_none());
    }
}
