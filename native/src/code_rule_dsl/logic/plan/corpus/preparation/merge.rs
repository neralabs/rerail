//! The merges before the split, on the version (decision 0281's path, step 5): `simplify::merge_identical` between
//! two `corpus::drop_repeated_rules`, as manager passes. The joins pushed into the chunks are made per rule, so two
//! programs' copies of one library rule made two: merged again, and so are the relations over them.
//!
//! Each is the tree pass it replaces, one for one: the classes are `congruence`'s (the coarsest partition of the
//! mergeable relations in which every two of a class have the same column types and the same rules once each
//! relation is named by its class), a rule compared as `front::template::canonical` writes it, and each class named
//! and written as `merge` names and writes it.

use crate::code_rule_dsl::logic::plan::fields::{Fields, FieldsData};
use crate::code_rule_dsl::logic::program_image::build::{Rewrite, T};
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{AggRef, AtomRef, Cell, Lit as L, RelId, RuleId, RuleRef, Slot, TermRef, Version};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Drops each rule equal to an earlier one (`corpus::drop_repeated_rules`): a relation is a set, so a second copy of
/// a rule derives nothing. How many it dropped, added up over its runs.
#[derive(Default)]
pub struct DropRepeated(pub usize);

impl ProgramPass for DropRepeated {
    const NAME: &'static str = "drop repeated rules";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let mut seen: HashMap<u64, Vec<RuleRef<'_>>> = HashMap::new();
        let mut drop: HashSet<RuleId> = HashSet::new();
        for r in v.rules() {
            let at = seen.entry(r.hash()).or_default();
            // Equal hashes are compared by text: a collision keeps both.
            if at.iter().any(|&o| v.show_rule(o) == v.show_rule(r)) {
                drop.insert(r.id());
            } else {
                at.push(r);
            }
        }
        if !drop.is_empty() {
            self.0 += drop.len();
            w.keep(|r| !drop.contains(&r.id()))?;
        }
        Ok(())
    }
}

/// Merges each class of `congruence` into one relation (`simplify::merge`): the relations merged, and how many
/// classes went to the shared namespace.
#[derive(Default)]
pub struct MergeIdentical {
    pub merged: usize,
    pub shared: usize,
}

impl ProgramPass for MergeIdentical {
    const NAME: &'static str = "merge identical relations";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let d = cx.get::<FieldsData>()?;
        let f = Fields::over(v, &d);
        let groups = congruence(v, f, false);
        if groups.is_empty() {
            return Ok(());
        }
        let read = read_rels(v, f);
        let mut taken: BTreeSet<String> = f.decls().iter().map(|d| v.text(d.name()).to_owned()).collect();
        let mut rename: BTreeMap<String, String> = BTreeMap::new();
        // Each class's rules are one member's: the member it is named after, or its first where its name is new.
        let mut firsts: BTreeSet<String> = BTreeSet::new();
        for g in &groups {
            let suffixes: BTreeSet<Option<&str>> = g.iter().map(|r| namespace(r).map(|(_, s)| s)).collect();
            let spaces: BTreeSet<Option<&str>> = g.iter().map(|r| namespace(r).map(|(p, _)| p)).collect();
            let unread = g.iter().find(|r| !read.contains(r.as_str()));
            // A relation nothing reads is read by name if at all: it names its class.
            let to = if let Some(u) = unread {
                u.clone()
            } else {
                match (suffixes.iter().next(), suffixes.len(), spaces.len()) {
                    (Some(Some(s)), 1, n) if n > 1 => {
                        let mut name = format!("s_{s}");
                        let mut k = 1;
                        while taken.contains(&name) {
                            name = format!("s{k}_{s}");
                            k += 1;
                        }
                        taken.insert(name.clone());
                        self.shared += 1;
                        name
                    }
                    // A class of outputs only, in one namespace: each output is the class, so a fresh name holds it.
                    _ if g.iter().all(|r| f.is_output(r)) => {
                        let mut name = format!("{}_class", g[0]);
                        let mut k = 1;
                        while taken.contains(&name) {
                            name = format!("{}_class{k}", g[0]);
                            k += 1;
                        }
                        taken.insert(name.clone());
                        name
                    }
                    // Named after its member with the shortest name (a lowering's copy of a relation is that
                    // relation's name with a prefix).
                    _ => g.iter().filter(|r| !f.is_output(r)).min_by_key(|r| r.len()).unwrap_or(&g[0]).clone(),
                }
            };
            if !g.contains(&to) {
                firsts.insert(g[0].clone());
            }
            for r in g {
                if *r != to {
                    rename.insert(r.clone(), to.clone());
                }
            }
        }
        self.merged += rename.len();
        // The program's rules in order, as the tree's: a renamed relation's rules dropped but for its class's first
        // (renamed); every rule reading a renamed relation rewritten; an output merged keeps its name, as a copy of
        // its class, appended.
        let input = w.input().clone();
        let renamed_rel: Vec<bool> = (0..input.rel_end()).map(|r| rename.contains_key(input.rel_name(RelId(r)))).collect();
        let renames = |a: AtomRef<'_>| renamed_rel[a.rel().0 as usize];
        let b = w.derive()?;
        let mut rw = Renaming { v: &input, rename: &rename };
        for r in input.rules() {
            let head = input.rel_name(r.head().rel());
            if rename.contains_key(head) && !firsts.contains(head) {
                continue;
            }
            if renames(r.head()) || r.body().filter_map(|l| l.atom()).any(renames) {
                b.rewrite(&input, r, &mut rw).finish()?;
            } else {
                b.keep(&input, r);
            }
        }
        for (from, to) in rename.iter().filter(|(from, _)| f.is_output(from)) {
            let cols: Vec<String> = (0..f.width(from)).map(|i| format!("c{i}")).collect();
            let args: Vec<T<'_>> = cols.iter().map(|c| T::Var(c.as_str())).collect();
            b.rule().head(from.as_str(), &args).atom(to.as_str(), &args).finish()?;
        }
        // The natives' reads (`front::Native::rename_reads`): every relation a native's rows name but its own.
        for (name, columns) in crate::code_rule_dsl::logic::program_image::about::RELATION_COLUMNS.iter().filter(|(n, _)| n.starts_with("native_")) {
            let rows = b.about_rows(name);
            let mut changed = false;
            let rows: Vec<Vec<Cell>> = rows
                .into_iter()
                .map(|mut row| {
                    for &c in columns.iter().filter(|&&c| c != 1) {
                        if let Some(Cell::Str(a)) = row.get(c).copied()
                            && let Some(t) = rename.get(b.text(a))
                        {
                            let t = b.atom(t);
                            row[c] = Cell::Str(t);
                            changed = true;
                        }
                    }
                    row
                })
                .collect();
            if changed {
                b.set_about_rows(name, rows);
            }
        }
        // The declarations and facts, each class's held by its name: the first member's where the name is new; an
        // output merged keeps its own declaration.
        let mut declared: HashSet<String> = taken.iter().filter(|t| f.declared(t)).cloned().collect();
        let mut with_facts: HashSet<String> = f.fact_rels().into_iter().filter(|r| !r.about()).map(|r| v.text(r.name()).to_owned()).collect();
        for (from, t) in &rename {
            if let Some(cols) = f.columns(from) {
                if !f.is_output(from) {
                    b.undeclare(from);
                    declared.remove(from);
                }
                if declared.insert(t.clone()) {
                    b.declare(t, &cols);
                }
            }
            if let Some(facts) = f.facts(from).filter(|r| !r.about()) {
                let rows: Vec<Vec<Cell>> = facts.rows().map(|r| r.collect()).collect();
                b.drop_facts(from);
                with_facts.remove(from);
                if with_facts.insert(t.clone()) {
                    b.set_fact_cells(t, rows);
                }
            }
        }
        Ok(())
    }
}

/// Renames the relations `rename` names, in heads and bodies.
struct Renaming<'a> {
    v: &'a Version,
    rename: &'a BTreeMap<String, String>,
}

impl Rewrite for Renaming<'_> {
    fn atom_as(&mut self, a: AtomRef<'_>) -> Option<(String, bool, bool)> {
        self.rename.get(self.v.rel_name(a.rel())).map(|t| (t.clone(), a.neg(), a.gated()))
    }
    fn head_as(&mut self, a: AtomRef<'_>) -> Option<String> {
        self.rename.get(self.v.rel_name(a.rel())).cloned()
    }
}

/// `simplify::namespace`: the program prefix `combine` gives a program's own relations (`p3_qn`) and the rest.
fn namespace(rel: &str) -> Option<(&str, &str)> {
    let rest = rel.strip_prefix('p')?;
    let digits = rest.find(|c: char| !c.is_ascii_digit())?;
    (digits > 0 && rest[digits..].starts_with('_')).then(|| (&rel[..digits + 1], &rest[digits + 1..]))
}

/// The relations the program reads: in a rule's body, by a native, or as an output (`simplify::read_rels`).
fn read_rels<'v>(v: &'v Version, f: Fields<'v>) -> BTreeSet<&'v str> {
    v.rules()
        .flat_map(|r| r.body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel())).collect::<Vec<_>>())
        .chain(f.natives().iter().flat_map(|n| n.reads.iter().map(|&r| v.rel_name(r))))
        .chain(f.outputs())
        .collect()
}

/// A token of a rule as `canonical` writes it: its text's parts, each variable by its first appearance, each
/// relation read a placeholder.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Tok<'v> {
    K(u8),
    I(i64),
    S(&'v str),
    V(u32),
}

/// A rule's template (`front::template::canonical` with every relation a placeholder): its tokens, and the
/// relations it reads, in the order it names them. Two rules of equal templates and equal labels of the relations
/// they read are equal under those labels, as their texts are.
struct Canon<'v> {
    v: &'v Version,
    /// Every constant the variable `?` (`simplify::congruence`'s `abstract_constants`).
    abstract_constants: bool,
    names: Vec<u32>,
    out: Vec<Tok<'v>>,
    read: Vec<RelId>,
}

impl<'v> Canon<'v> {
    fn of(v: &'v Version, r: RuleRef<'v>, abstract_constants: bool) -> (Vec<Tok<'v>>, Vec<RelId>) {
        let mut c = Canon { v, abstract_constants, names: Vec::new(), out: Vec::new(), read: Vec::new() };
        c.out.push(Tok::K(1));
        r.head().args().for_each(|t| c.term(t));
        for l in r.body() {
            c.out.push(Tok::K(2));
            match l.view() {
                L::Atom(a) => c.atom(a),
                L::Cmp { l, op, r } => {
                    c.out.push(Tok::K(3));
                    c.term(l);
                    c.out.push(Tok::I(op as i64));
                    c.term(r);
                }
                L::Agg { var, op, over, atom } => {
                    // Numbered as `canonical` names them (the value, `over`, a join's keys, then the atom's); written
                    // as `Literal`'s text writes them (a count's `over` not at all).
                    c.out.push(Tok::K(4));
                    let var = c.name(var);
                    c.out.push(Tok::V(var));
                    let over = over.map(|o| c.name(o));
                    let (k, count) = match op {
                        AggRef::Count => (0, true),
                        AggRef::Min => (1, false),
                        AggRef::Sum => (2, false),
                        AggRef::Product => (3, false),
                        AggRef::Max => (4, false),
                        AggRef::Join { .. } => (5, false),
                    };
                    c.out.push(Tok::I(k));
                    if !count {
                        c.out.push(over.map_or(Tok::K(5), Tok::V));
                    }
                    if let AggRef::Join { by, sep } = op {
                        for s in by {
                            let s = c.name(s);
                            c.out.push(Tok::V(s));
                        }
                        c.out.push(Tok::S(v.text(sep)));
                    }
                    c.atom(atom);
                }
                L::Nearest { n, a, kindset } => {
                    c.out.push(Tok::K(6));
                    let (n, a) = (c.name(n), c.name(a));
                    c.out.extend([Tok::V(n), Tok::V(a), Tok::S(v.text(kindset))]);
                }
                L::Ast { document, node, matcher, captures } => {
                    c.out.push(Tok::K(7));
                    let (d, n) = (c.name(document), c.name(node));
                    c.out.extend([Tok::V(d), Tok::V(n), Tok::S(v.text(matcher))]);
                    for (field, s) in captures {
                        let s = c.name(s);
                        c.out.extend([Tok::S(v.text(field)), Tok::V(s)]);
                    }
                }
            }
        }
        (c.out, c.read)
    }

    fn name(&mut self, s: Slot) -> u32 {
        match self.names.iter().position(|&n| n == s.0) {
            Some(k) => k as u32,
            None => {
                self.names.push(s.0);
                self.names.len() as u32 - 1
            }
        }
    }

    fn atom(&mut self, a: AtomRef<'v>) {
        self.out.push(Tok::K(if a.neg() { 9 } else { 8 }));
        self.read.push(a.rel());
        a.args().for_each(|t| self.term(t));
        self.out.push(Tok::K(10));
    }

    fn term(&mut self, t: TermRef<'v>) {
        if self.abstract_constants && matches!(t, TermRef::Int(_) | TermRef::Str(_)) {
            let k = self.name(Slot(u32::MAX));
            self.out.push(Tok::V(k));
            return;
        }
        match t {
            TermRef::Var(s) => {
                let k = self.name(s);
                self.out.push(Tok::V(k));
            }
            TermRef::Wild => self.out.push(Tok::K(11)),
            TermRef::Top => self.out.push(Tok::K(12)),
            TermRef::Int(n) => self.out.push(Tok::I(n)),
            TermRef::Str(a) => self.out.extend([Tok::K(13), Tok::S(self.v.text(a))]),
            TermRef::Call(f, args) => {
                self.out.extend([Tok::K(14), Tok::S(self.v.text(f))]);
                args.for_each(|a| self.term(a));
                self.out.push(Tok::K(15));
            }
        }
    }
}

/// `simplify::congruence` (constants kept) on the version: the classes of more than one mergeable relation, each
/// sorted, in the order of their first members by name.
pub(super) fn congruence(v: &Version, f: Fields<'_>, abstract_constants: bool) -> Vec<Vec<String>> {
    let mut rules_of: HashMap<RelId, Vec<RuleRef<'_>>> = HashMap::new();
    for r in v.rules() {
        rules_of.entry(r.head().rel()).or_default().push(r);
    }
    let facts_of = |rel: &str| -> Vec<String> {
        let mut rows: Vec<String> = f.facts(rel).filter(|r| !r.about()).into_iter().flat_map(|r| r.rows().map(|row| format!("{:?}", row.map(|c| f.cell_text(c)).collect::<Vec<_>>()))).collect();
        rows.sort();
        rows
    };
    // What keeps a relation its own (`simplify::fixed`).
    let fixed = |rel: &str| f.is_input(rel) || f.native(rel).is_some() || f.lattice(rel) || f.demanded(rel) || f.gate(rel) || !f.qualifiers_of(rel).is_empty();
    let read = read_rels(v, f);
    let cands: Vec<&str> = rules_of
        .keys()
        .map(|&r| v.rel_name(r))
        .chain(f.fact_rels().into_iter().filter(|r| !r.about() && r.rows().len() > 0).map(|r| v.text(r.name())))
        .collect::<BTreeSet<&str>>()
        .into_iter()
        .filter(|r| !fixed(r))
        .collect();
    // Each candidate's number, by relation; every other relation a label of its own (its id).
    let mut number: Vec<Option<u32>> = vec![None; v.rel_end() as usize];
    for (i, r) in cands.iter().enumerate() {
        if let Some(id) = v.rel(r) {
            number[id.0 as usize] = Some(i as u32);
        }
    }
    let rules = |rel: &str| v.rel(rel).and_then(|r| rules_of.get(&r)).map_or(&[][..], |rs| rs.as_slice());
    let mut class: Vec<usize> = Vec::with_capacity(cands.len());
    {
        let mut ids: HashMap<(Vec<&str>, usize, Vec<String>), usize> = HashMap::new();
        for &r in &cands {
            let distinct = rules(r).iter().map(|x| x.hash()).collect::<HashSet<u64>>().len();
            let n = ids.len();
            // An undeclared relation (a helper a pass made without one) has no types, as the tree's merge reads it.
            let types = f.columns(r).map_or_else(Vec::new, |c| c.into_iter().map(|(_, t)| t).collect());
            class.push(*ids.entry((types, distinct, facts_of(r))).or_insert(n));
        }
    }
    let mut templates: HashMap<Vec<Tok<'_>>, usize> = HashMap::new();
    let shapes: Vec<Vec<(usize, Vec<RelId>)>> = cands
        .iter()
        .map(|&r| {
            rules(r)
                .iter()
                .map(|&x| {
                    let (toks, read) = Canon::of(v, x, abstract_constants);
                    let n = templates.len();
                    (*templates.entry(toks).or_insert(n), read)
                })
                .collect()
        })
        .collect();
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Label {
        Class(usize),
        Own(u32),
    }
    let mut classes = class.iter().collect::<BTreeSet<_>>().len();
    loop {
        let mut ids: HashMap<(usize, Vec<(usize, Vec<Label>)>), usize> = HashMap::new();
        let mut next: Vec<usize> = Vec::with_capacity(cands.len());
        for (c, shaped) in class.iter().zip(&shapes) {
            let mut sig: Vec<(usize, Vec<Label>)> = shaped.iter().map(|(t, read)| (*t, read.iter().map(|r| number[r.0 as usize].map_or(Label::Own(r.0), |k| Label::Class(class[k as usize]))).collect())).collect();
            sig.sort();
            sig.dedup();
            let n = ids.len();
            next.push(*ids.entry((*c, sig)).or_insert(n));
        }
        let now = ids.len();
        class = next;
        if now == classes {
            break;
        }
        classes = now;
    }
    let mut groups: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (k, &r) in cands.iter().enumerate() {
        groups.entry(class[k]).or_default().push(r.to_owned());
    }
    // An unread relation joins a class only with a lowering's widened copy of it (`rr_wide_<attr>_<r>`, decision
    // 0174); any other stays apart.
    groups
        .into_values()
        .filter_map(|g| {
            let (mut kept, unread): (Vec<String>, Vec<String>) = g.into_iter().partition(|r| read.contains(r.as_str()));
            let copies = |u: &str| crate::code_rule_dsl::logic::rr::lower::ON_REQUEST.iter().flat_map(|round| round.iter()).any(|a| kept.iter().any(|k| *k == format!("rr_wide_{}_{u}", a.rel)));
            let own = unread.into_iter().find(|u| copies(u));
            kept.extend(own);
            kept.sort();
            (kept.len() > 1).then_some(kept)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::code_rule_dsl::logic::program_image::manager::Pipeline;
    use crate::code_rule_dsl::logic::program_image::Builder;

    /// The merges on the version are the tree's (`corpus::merge_and_drop_repeats`): two programs' copies of a relation
    /// go to the shared namespace (`s_q`) and their readers read it; an output merged keeps its name as a copy of its
    /// class; relations of equal facts merge; a repeated rule is dropped. The same program, the same counts.
    #[test]
    fn the_merges_on_a_version_are_the_trees() {
        let text = ".decl e(x: number, y: number)\n.input e\n\
                    .decl p0_q(x: number)\n.decl p1_q(x: number)\np0_q(x) :- e(x, _).\np1_q(x) :- e(x, _).\np1_q(x) :- e(x, _).\n\
                    .decl p0_r(x: number)\n.decl p1_r(x: number)\np0_r(x) :- p0_q(x), e(_, x).\np1_r(x) :- p1_q(x), e(_, x).\n\
                    .decl f1(k: symbol)\n.decl f2(k: symbol)\nf1(\"a\").\nf2(\"a\").\n\
                    .decl uses(x: number, k: symbol)\nuses(x, k) :- p0_r(x), f1(k).\nuses(x, k) :- p1_r(x), f2(k).\n\
                    .output uses, p0_r\n";
        let mut tree = crate::code_rule_dsl::logic::rr::parse(text).unwrap();
        let (v, _) = Builder::of_program(&tree).unwrap().finish().unwrap();
        let mut p = Pipeline::new("test", v);
        let (mut merge, mut repeats) = (super::MergeIdentical::default(), super::DropRepeated::default());
        p.program(&mut repeats).program(&mut merge).program(&mut repeats);
        let (out, _) = p.finish().unwrap();
        let (m, s, r) = crate::code_rule_dsl::logic::plan::corpus::merge_and_drop_repeats(&mut tree);
        assert!(m > 0 && s > 0 && r > 0, "the program merges, shares and repeats: {m} {s} {r}");
        assert_eq!((merge.merged, merge.shared, repeats.0), (m, s, r));
        let (want, _) = Builder::of_program(&tree).unwrap().finish().unwrap();
        assert_eq!((out.size(), out.digest()), (want.size(), want.digest()), "{}\n---\n{}", out.show(), want.show());
        assert!(out.rules().any(|r| out.rel_name(r.head().rel()) == "s_q"), "{}", out.show());
    }
}

/// Relations equal up to their constants (`simplify::similar_relations`): how many, in how many groups, and a few
/// groups. A congruence of its own over the whole program, so asked for only when the run is profiled.
pub fn similar_relations(v: &Version, f: Fields<'_>) -> (usize, usize, Vec<String>) {
    let similar = congruence(v, f, true);
    (similar.iter().map(Vec::len).sum(), similar.len(), similar.iter().take(3).map(|g| g.iter().take(4).cloned().collect::<Vec<_>>().join(" ~ ")).collect())
}
