//! The merges before the split, on the version (decision 0281's path, step 5): `MergeIdentical` between two
//! `DropRepeated`, as manager passes. The joins pushed into the chunks are made per rule, so two
//! programs' copies of one library rule made two: merged again, and so are the relations over them.
//!
//! The classes are `congruence`'s (the coarsest partition of the
//! mergeable relations in which every two of a class have the same column types and the same rules once each
//! relation is named by its class), a rule compared by its template (`Canon` below), and each class named
//! and written as `merge` names and writes it.

use crate::code_rule_dsl::logic::plan::fields::{Fields, FieldsData};
use crate::code_rule_dsl::logic::program_image::build::{Rewrite, T};
use crate::code_rule_dsl::logic::program_image::manager::{Cx, Last, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{AggRef, AtomRef, Cell, Lit as L, RelId, RuleId, RuleRef, Slot, TermRef, Version};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Drops each rule equal to an earlier one: a relation is a set, so a second copy of a rule derives nothing, yet it
/// was planned and run as a rule of its own. Hiding's clones made them (at O125 step 07, 407 of the cross part's
/// 1,483 rules, `rr_in_annotation_0_bv`'s most), and the merge's renaming does. The rule kept serves its repeats'
/// readers too (the request record, decision 0331). How many it dropped, added up over its runs.
#[derive(Default)]
pub struct DropRepeated(pub usize);

impl ProgramPass for DropRepeated {
    const NAME: &'static str = "drop repeated rules";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let mut seen: HashMap<u64, Vec<RuleRef<'_>>> = HashMap::new();
        // Each rule dropped, and the earlier equal rule kept for it.
        let mut drop: HashMap<RuleId, RuleId> = HashMap::new();
        for r in v.rules() {
            let at = seen.entry(r.hash()).or_default();
            // Equal hashes are compared by text: a collision keeps both.
            match at.iter().find(|&&o| v.show_rule(o) == v.show_rule(r)) {
                Some(o) => {
                    drop.insert(r.id(), o.id());
                }
                None => at.push(r),
            }
        }
        if !drop.is_empty() {
            self.0 += drop.len();
            w.keep(|r| !drop.contains_key(&r.id()))?;
            // The request record by rule, as the tree's `set_rules_from` states it (decision 0331): a rule kept serves
            // its repeats' readers too. Left by place over the rules dropped, a later step's edit of it (the merge's
            // renamed readers) fell on other rules, and the pipeline's record at its end lost it.
            let was = v.asked();
            let mut asked = was.clone();
            for (req, by) in asked.iter_mut() {
                let Some(rel) = v.rel(req) else { continue };
                let heads: Vec<RuleId> = v.heads(rel).map(|r| r.id()).collect();
                if heads.len() != by.len() || !heads.iter().any(|h| drop.contains_key(h)) {
                    continue;
                }
                let mut serves: HashMap<RuleId, std::collections::BTreeSet<String>> = HashMap::new();
                for (h, readers) in heads.iter().zip(by.iter()) {
                    serves.entry(drop.get(h).copied().unwrap_or(*h)).or_default().extend(readers.iter().cloned());
                }
                *by = heads.iter().filter(|h| !drop.contains_key(h)).map(|h| serves.remove(h).unwrap_or_default()).collect();
            }
            if asked != was {
                w.on()?.fields().set_asked(&asked);
            }
        }
        Ok(())
    }
    /// Over an edit (decision 0393): a removed rule that repeated none is gone from the last output, and an added rule
    /// is written unless a rule of the input before it repeats it. A removed rule that another repeated (the other
    /// would now be the first) runs it whole.
    fn again(&mut self, cx: &Cx<'_>, last: &Last<'_>, w: &mut ProgramW<'_>) -> Result<bool, String> {
        let v = cx.v();
        let repeats = |of: &Version, r: RuleRef<'_>| of.heads(r.head().rel()).any(|o| o.id() != r.id() && o.hash() == r.hash() && of.show_rule(o) == of.show_rule(r));
        let removed: Vec<RuleRef<'_>> = last.delta.removed.iter().filter_map(|id| last.input.rule(id)).collect();
        if removed.len() != last.delta.removed.len() || removed.iter().any(|&r| repeats(last.input, r)) {
            return Ok(false);
        }
        // Each removed rule in the last output: by its id where the output numbers it as the input does, else the
        // output's one rule of its text (an output written onto an earlier output numbers its rules its own way).
        let mut gone = Vec::new();
        for r in removed {
            match crate::code_rule_dsl::logic::program_image::manager::held_as(last.output, last.input, r) {
                Some(o) => gone.push(o),
                None => return Ok(false),
            }
        }
        let b = w.on()?;
        for id in gone {
            b.remove(id);
        }
        for &id in &last.delta.added {
            let Some(r) = v.rule(id) else { return Ok(false) };
            // An earlier rule of the input repeats it: the input's own rules come before the edit's added ones, and of
            // two added ones the first is written.
            let earlier = v.heads(r.head().rel()).take_while(|o| o.id() != r.id()).any(|o| o.hash() == r.hash() && v.show_rule(o) == v.show_rule(r));
            if earlier {
                self.0 += 1;
            } else {
                b.copy(v, r);
            }
        }
        Ok(true)
    }
}

/// Merges each class of `congruence` into one relation: the relations merged, and how many
/// classes went to the shared namespace.
///
/// One class's rules are kept, renamed: its namesake's (not renamed), or its first member's. An output merged
/// keeps its name, as a copy of the class.
///
/// A request's copy merged into it (`congruence`): the copy's rules are the request's, so each of the request's
/// serves the readers of the copy's equal rule too (decision 0331), before the copy's rules and record go.
///
/// A request's copy merged into it (`congruence`) leaves its record with its rules.
///
/// A request rule's gate (`Program::asked_by`) goes with its rules: kept where the class keeps this member's
/// (its first, under a new name), else dropped with them.
///
/// namespaces (a library relation every program that includes the library has) is named in the shared namespace,
/// `s_<name>`. Whether anything merged.
#[derive(Default)]
pub struct MergeIdentical {
    pub merged: usize,
    pub shared: usize,
    /// What `again` reads back (decision 0393): the renames and the classes' firsts, where it merged any.
    kept: Option<(BTreeMap<String, String>, BTreeSet<String>)>,
}

impl MergeIdentical {
    /// Each class's name, and the relations renamed to it (`run`'s decisions): the renames, the members whose
    /// rules a class keeps, and how many classes went to the shared namespace. `None` where there is no class.
    fn renames(v: &Version, f: Fields<'_>, groups: &[Vec<String>]) -> Option<(BTreeMap<String, String>, BTreeSet<String>, usize)> {
        if groups.is_empty() {
            return None;
        }
        let mut shared = 0;
        let read = read_rels(v, f);
        let mut taken: BTreeSet<String> = f.decls().iter().map(|d| v.text(d.name()).to_owned()).collect();
        let mut rename: BTreeMap<String, String> = BTreeMap::new();
        // Each class's rules are one member's: the member it is named after, or its first where its name is new.
        let mut firsts: BTreeSet<String> = BTreeSet::new();
        for g in groups {
            let suffixes: BTreeSet<Option<&str>> = g.iter().map(|r| namespace(r).map(|(_, s)| s)).collect();
            let spaces: BTreeSet<Option<&str>> = g.iter().map(|r| namespace(r).map(|(p, _)| p)).collect();
            let unread = g.iter().find(|r| !read.contains(r.as_str()));
            // A relation nothing reads is read by name if at all: it names its class.
            let to = if let Some(u) = unread {
                u.clone()
            } else {
                // A relation nothing reads is read by name if at all: it names its class (`congruence` gives it one), as a
                // request does (the class of its own widened copies).
                match (suffixes.iter().next(), suffixes.len(), spaces.len()) {
                    (Some(Some(s)), 1, n) if n > 1 => {
                        let mut name = format!("s_{s}");
                        let mut k = 1;
                        while taken.contains(&name) {
                            name = format!("s{k}_{s}");
                            k += 1;
                        }
                        taken.insert(name.clone());
                        shared += 1;
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
                    // Named after its member with the shortest name: a lowering's copy of a relation (`rr_wide_<attr>_<r>`)
                    // is that relation's name with a prefix, so the relation names it.
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
        Some((rename, firsts, shared))
    }
}

impl ProgramPass for MergeIdentical {
    const NAME: &'static str = "merge identical relations";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let d = cx.get::<FieldsData>()?;
        let f = Fields::over(v, &d);
        let classes = cx.get::<Congruence>()?;
        let Some((rename, firsts, shared)) = Self::renames(v, f, &classes.groups) else { return Ok(()) };
        self.shared += shared;
        let taken: BTreeSet<String> = f.decls().iter().map(|d| v.text(d.name()).to_owned()).chain(rename.values().cloned()).collect();
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
                b.fields().set_about_rows(name, rows);
            }
        }
        // The request record (decision 0331) after the merge: a renamed reader is read by its
        // class's name, and a request's copy merged into it leaves the record with its rules.
        {
            let mut rows = b.about_rows("asked");
            // A request's copy merged into it: each of the request's rules serves the readers of the copy's equal
            // rule too (equal as `canonical` compares, constants and all), before the
            // copy's rows go.
            let asked = input.asked();
            let class = |r: RelId| { let n = input.rel_name(r); rename.get(n).map_or(n, String::as_str) };
            for (from, to) in &rename {
                let (Some(copy), Some(_)) = (asked.get(from), asked.get(to)) else { continue };
                let canon = |rel: &str| -> Vec<(Vec<Tok<'_>>, Vec<&str>)> {
                    input.rel(rel).into_iter().flat_map(|id| input.heads(id)).map(|r| { let (t, read) = Canon::of(&input, r, false); (t, read.into_iter().map(class).collect()) }).collect()
                };
                let (theirs, ours) = (canon(from), canon(to));
                if theirs.len() != copy.len() {
                    continue;
                }
                let p = rows.iter().find_map(|r| r.first().copied());
                for (t, by) in theirs.iter().zip(copy) {
                    let (Some(j), Some(p)) = (ours.iter().position(|o| o == t), p) else { continue };
                    for reader in by {
                        rows.push(vec![p, Cell::Str(b.atom(to)), Cell::Int(j as i64), Cell::Str(b.atom(reader))]);
                    }
                }
            }
            let n = rows.len();
            let mut seen = HashSet::new();
            let key = |row: &[Cell]| format!("{row:?}");
            let mut changed = false;
            let mut kept = Vec::with_capacity(n);
            for mut row in rows {
                if let Some(Cell::Str(a)) = row.get(1).copied()
                    && rename.contains_key(b.text(a))
                {
                    changed = true;
                    continue;
                }
                if let Some(Cell::Str(a)) = row.get(3).copied()
                    && let Some(t) = rename.get(b.text(a))
                {
                    row[3] = Cell::Str(b.atom(t));
                    changed = true;
                }
                if seen.insert(key(&row)) {
                    kept.push(row);
                } else {
                    changed = true;
                }
            }
            if changed || kept.len() != n {
                // In the order the tree's record writes them (`about_of`): by request, place and reader.
                let text = |b: &crate::code_rule_dsl::logic::program_image::Builder<'_>, c: Option<&Cell>| match c {
                    Some(&Cell::Str(a)) => b.text(a).to_owned(),
                    _ => String::new(),
                };
                let num = |c: Option<&Cell>| match c {
                    Some(&Cell::Int(k)) => k,
                    _ => 0,
                };
                let mut keyed: Vec<((String, i64, String), Vec<Cell>)> = kept.into_iter().map(|r| ((text(b, r.get(1)), num(r.get(2)), text(b, r.get(3))), r)).collect();
                keyed.sort_by(|x, y| x.0.cmp(&y.0));
                b.fields().set_about_rows("asked", keyed.into_iter().map(|(_, r)| r).collect());
            }
        }
        // The declarations and facts, each class's held by its name: the first member's where the name is new; an
        // output merged keeps its own declaration.
        let mut declared: HashSet<String> = taken.iter().filter(|t| f.declared(t)).cloned().collect();
        let mut with_facts: HashSet<String> = f.fact_rels().into_iter().filter(|r| !r.about()).map(|r| v.text(r.name()).to_owned()).collect();
        for (from, t) in &rename {
            if let Some(cols) = f.columns(from) {
                if !f.is_output(from) {
                    b.fields().undeclare(from);
                    declared.remove(from);
                }
                if declared.insert(t.clone()) {
                    b.fields().declare(t, &cols);
                }
            }
            if let Some(facts) = f.facts(from).filter(|r| !r.about()) {
                let rows: Vec<Vec<Cell>> = facts.rows().map(|r| r.collect()).collect();
                b.fields().drop_facts(from);
                with_facts.remove(from);
                if with_facts.insert(t.clone()) {
                    b.fields().set_fact_cells(t, rows);
                }
            }
        }
        self.kept = Some((rename, firsts));
        Ok(())
    }

    fn plan(&self) -> Vec<crate::code_rule_dsl::logic::program_image::manager::Row> {
        use crate::code_rule_dsl::logic::front::Value;
        let Some((rename, firsts)) = &self.kept else { return Vec::new() };
        let s = |x: &str| Value::Str(x.to_owned().into());
        let mut rows = vec![vec![s("ran")]];
        rows.extend(rename.iter().map(|(a, b)| vec![s("rename"), s(a), s(b)]));
        rows.extend(firsts.iter().map(|r| vec![s("first"), s(r)]));
        rows
    }

    /// Over an edit (decision 0393): the edit's added rules renamed alone, onto the last output, where the classes
    /// come out named as the last run's (the same renames and firsts). Otherwise it runs whole.
    fn again(&mut self, cx: &Cx<'_>, last: &Last<'_>, w: &mut ProgramW<'_>) -> Result<bool, String> {
        use crate::code_rule_dsl::logic::front::Value;
        use crate::code_rule_dsl::logic::program_image::manager::Reads;
        if last.delta.touched != Reads::RULES {
            return Ok(false);
        }
        let text = |x: &Value| if let Value::Str(s) = x { Some(s.to_string()) } else { None };
        let (mut was, mut was_firsts, mut ran) = (BTreeMap::new(), BTreeSet::new(), false);
        for r in last.plan {
            match (r.first().and_then(text).as_deref(), &r[1..]) {
                (Some("ran"), []) => ran = true,
                (Some("rename"), [a, b]) => drop(was.insert(text(a).ok_or("a merge plan's relation")?, text(b).ok_or("a merge plan's relation")?)),
                (Some("first"), [a]) => drop(was_firsts.insert(text(a).ok_or("a merge plan's relation")?)),
                _ => return Ok(false),
            }
        }
        if !ran {
            return Ok(false);
        }
        let v = cx.v();
        let d = cx.get::<FieldsData>()?;
        let f = Fields::over(v, &d);
        let classes = cx.get::<Congruence>()?;
        let Some((rename, firsts, shared)) = Self::renames(v, f, &classes.groups) else { return Ok(false) };
        if rename != was || firsts != was_firsts {
            return Ok(false);
        }
        // The request record (decision 0331) reads a merged request's and copy's rules: an edit of either writes it
        // again, whole.
        let asked = v.asked();
        let recorded = |of: &Version, r: RuleRef<'_>| {
            let h = of.rel_name(r.head().rel());
            asked.contains_key(h) && (rename.contains_key(h) || rename.values().any(|t| t == h))
        };
        if last.delta.removed.iter().any(|id| last.input.rule(id).is_none_or(|r| recorded(last.input, r))) || last.delta.added.iter().any(|&id| v.rule(id).is_none_or(|r| recorded(v, r))) {
            return Ok(false);
        }
        let renamed_rel: Vec<bool> = (0..v.rel_end()).map(|r| rename.contains_key(v.rel_name(RelId(r)))).collect();
        let renames = |a: AtomRef<'_>| renamed_rel[a.rel().0 as usize];
        let b = w.on()?;
        for id in last.delta.removed.iter() {
            let came: Vec<RuleId> = last.output.derived(id).map(|r| r.id()).collect();
            for c in came {
                b.remove(c);
            }
        }
        let mut rw = Renaming { v, rename: &rename };
        for &id in &last.delta.added {
            let r = v.rule(id).ok_or("an added rule is not its input's")?;
            let head = v.rel_name(r.head().rel());
            if rename.contains_key(head) && !firsts.contains(head) {
                continue;
            }
            if renames(r.head()) || r.body().filter_map(|l| l.atom()).any(renames) {
                b.rewrite(v, r, &mut rw).finish()?;
            } else {
                b.keep_into(v, r);
            }
        }
        self.merged += rename.len();
        self.shared += shared;
        self.kept = Some((rename, firsts));
        Ok(true)
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

/// The program prefix `combine` gives a program's own relations (`p3_qn`) and the rest.
fn namespace(rel: &str) -> Option<(&str, &str)> {
    let rest = rel.strip_prefix('p')?;
    let digits = rest.find(|c: char| !c.is_ascii_digit())?;
    (digits > 0 && rest[digits..].starts_with('_')).then(|| (&rel[..digits + 1], &rest[digits + 1..]))
}

/// The relations the program reads: in a rule's body, by a native, or as an output.
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

/// A rule's template (every variable named by its place, every relation a placeholder): its tokens, and the
/// relations it reads, in the order it names them. Two rules of equal templates and equal labels of the relations
/// they read are equal under those labels, as their texts are.
struct Canon<'v> {
    v: &'v Version,
    /// Every constant the variable `?` (`congruence`'s `abstract_constants`).
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
                        AggRef::Bounded { max: false, strict: false, .. } => (6, false),
                        AggRef::Bounded { max: false, strict: true, .. } => (7, false),
                        AggRef::Bounded { max: true, strict: false, .. } => (8, false),
                        AggRef::Bounded { max: true, strict: true, .. } => (9, false),
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
                    if let Some(b) = op.bound() {
                        let b = c.name(b);
                        c.out.push(Tok::V(b));
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
            // A branded atom (decision 0270), as `canonical` keeps it: its class and atom.
            TermRef::Branded(b) => {
                self.out.extend([Tok::K(16), Tok::I(i64::from(crate::code_rule_dsl::logic::program_image::brand_code(b.class))), Tok::I(i64::from(b.atom))])
            }
        }
    }
}

/// The classes of more than one mergeable relation (constants kept, unless `abstract_constants`), each
/// sorted, in the order of their first members by name.
///
/// Relations with facts merge too, when their facts are equal (a library's `cdef("class_definition").`).
///
/// What keeps a relation its own (`fixed`), as sets made once.
///
/// The candidates by number, in `cands`' order; every other relation a rule names numbered after them, each a
/// label of its own in every round.
///
/// A relation's rules are a set: a second copy of a rule derives nothing, so it is counted once (and
/// below, a relation's signature holds each rule once), and a repeated rule does not keep a relation
/// apart from its twin.
///
/// A candidate's rules as `canonical` reads them: the program's own, or with every constant `?`.
///
/// Each rule's canonical text is fixed but for the labels of the relations it reads: it is made once, with every
/// relation a placeholder (its template), and the relations read kept in the order it labels them. A round's
/// signature of a rule is its template and those relations' labels, equal exactly when the texts `canonical`
/// gives under the round's labels are (a relation's name has no character of the text around it).
/// The texts made, numbered in `cands`' order, each relation read kept by its number.
///
/// The head's label is replaced by `@`.
///
/// The coarsest partition of the mergeable relations in which every two of a class have the same column types and
/// the same rules once each relation is named by its class (partition refinement: start from one class per type
/// signature and rule count, split until stable). It is the greatest such congruence, so it holds recursive
/// relations that only equal each other (`p0_qn(…) :- …, p0_qn(…)` and `p1_qn`): their least models are equal,
/// since at each step of the fixpoint every member of a class derives from equal classes by equal rules.
/// `abstract_constants` names every constant `?` too: the classes are then relations equal up to their constants.
pub fn congruence(v: &Version, f: Fields<'_>, abstract_constants: bool) -> Vec<Vec<String>> {
    let r = refine(v, f, abstract_constants);
    r.groups(v, f)
}

/// `congruence`'s partition, before its classes are filtered (`Refined::groups`): the candidates, each one's number
/// (by relation), its class, the hash of its first key (its types, its rules' count and its facts), and each of its
/// rules' template hash with the relations it reads.
struct Refined<'v> {
    cands: Vec<&'v str>,
    number: Vec<Option<u32>>,
    class: Vec<usize>,
    initial: Vec<u64>,
    shapes: Vec<Vec<(u64, Vec<RelId>)>>,
}

fn fx<T: std::hash::Hash>(t: &T) -> u64 {
    use std::hash::{BuildHasher as _, BuildHasherDefault};
    BuildHasherDefault::<rustc_hash::FxHasher>::default().hash_one(t)
}

/// What keeps a relation its own (`simplify::fixed`): its kind, or being a request (decision 0174) with no widened
/// copy of its own to join (`rr_wide_<attr>_<req>`, the tree's `simplify::congruence`); a request is read by name by
/// its answer.
fn fixed(v: &Version, f: Fields<'_>, rel: &str) -> bool {
    let own = f.is_input(rel) || f.native(rel).is_some() || f.lattice(rel) || f.demanded(rel) || f.gate(rel) || !f.qualifiers_of(rel).is_empty();
    own || (request(rel) && !copied(v, rel))
}

/// A request's name (decision 0174).
fn request(rel: &str) -> bool {
    crate::code_rule_dsl::logic::rr::lower::ON_REQUEST.iter().flat_map(|round| round.iter()).any(|a| a.req == rel)
}

/// `copy` is a lowering's widened copy of `of` (`rr_wide_<attr>_<of>`).
fn copy_of(copy: &str, of: &str) -> bool {
    crate::code_rule_dsl::logic::rr::lower::ON_REQUEST.iter().flat_map(|round| round.iter()).any(|a| copy == format!("rr_wide_{}_{of}", a.rel))
}

/// A request has a widened copy of its own with rules.
fn copied(v: &Version, rel: &str) -> bool {
    crate::code_rule_dsl::logic::rr::lower::ON_REQUEST.iter().flat_map(|round| round.iter()).any(|a| v.rel(&format!("rr_wide_{}_{rel}", a.rel)).is_some_and(|id| v.heads(id).next().is_some()))
}

/// A candidate's first key (`refine`): its column types, its distinct rules' count, and its facts, by text.
fn first_key(v: &Version, f: Fields<'_>, rel: &str, rules: &[RuleRef<'_>]) -> (Vec<String>, usize, Vec<String>) {
    let distinct = rules.iter().map(|x| x.hash()).collect::<HashSet<u64>>().len();
    // An undeclared relation (a helper a pass made without one, or a library's facts, `param_slot(…).`) has no
    // types, as the tree's merge reads it.
    let types = f.columns(rel).map_or_else(Vec::new, |c| c.into_iter().map(|(_, t)| t.to_owned()).collect());
    let mut rows: Vec<String> = f.facts(rel).filter(|r| !r.about()).into_iter().flat_map(|r| r.rows().map(|row| format!("{:?}", row.map(|c| f.cell_text(c)).collect::<Vec<_>>()))).collect();
    rows.sort();
    let _ = v;
    (types, distinct, rows)
}

impl<'v> Refined<'v> {
    /// `congruence`'s classes: each of more than one member, sorted, in the order of its first member by name; an
    /// unread relation in one only with a lowering's widened copy of it.
    fn groups(&self, v: &Version, f: Fields<'_>) -> Vec<Vec<String>> {
        let mut groups: BTreeMap<usize, Vec<String>> = BTreeMap::new();
        for (k, &r) in self.cands.iter().enumerate() {
            groups.entry(self.class[k]).or_default().push(r.to_owned());
        }
        filter_classes(groups.into_values(), &read_rels(v, f))
    }
    /// Each candidate's class, by its least member, and its signature: its first key and its rules' templates over
    /// the classes of what they read (an `Own` relation by its name).
    fn classed(&self, v: &Version) -> BTreeMap<String, (String, u64)> {
        let mut least: HashMap<usize, &str> = HashMap::new();
        for (k, &r) in self.cands.iter().enumerate() {
            let e = least.entry(self.class[k]).or_insert(r);
            if r < *e {
                *e = r;
            }
        }
        let label = |r: RelId| -> &str { self.number[r.0 as usize].map_or_else(|| v.rel_name(r), |k| least[&self.class[k as usize]]) };
        self.cands
            .iter()
            .enumerate()
            .map(|(k, &r)| {
                let mut sig: Vec<(u64, Vec<&str>)> = self.shapes[k].iter().map(|(t, read)| (*t, read.iter().map(|&x| label(x)).collect())).collect();
                sig.sort();
                sig.dedup();
                (r.to_owned(), (least[&self.class[k]].to_owned(), fx(&(self.initial[k], sig))))
            })
            .collect()
    }
}

/// `congruence` (constants kept) as an analysis of the version (decision 0393): its classes, each candidate's class
/// and signature (`Refined::classed`), and each head's rules' key. Maintained over an edit of rules: a relation that
/// reads no changed head, through any others, keeps its class; one that does (the edit's cone, up) is classed again
/// from its rules over the classes of what it reads, and joins the class of that signature. Where the cone is
/// recursive, or a class's least member would change, it is computed again.
#[derive(Debug, PartialEq)]
pub(in crate::code_rule_dsl::logic::plan::corpus) struct Congruence {
    pub(in crate::code_rule_dsl::logic::plan::corpus) groups: Vec<Vec<String>>,
    classed: BTreeMap<String, (String, u64)>,
    keys: BTreeMap<String, u64>,
}

/// Each head's key: the hash of its rules' hashes, sorted.
fn head_keys(v: &Version) -> BTreeMap<String, u64> {
    let mut of: HashMap<RelId, Vec<u64>> = HashMap::new();
    for r in v.rules() {
        of.entry(r.head().rel()).or_default().push(r.hash());
    }
    of.into_iter()
        .map(|(rel, mut hs)| {
            hs.sort_unstable();
            (v.rel_name(rel).to_owned(), fx(&hs))
        })
        .collect()
}

impl crate::code_rule_dsl::logic::program_image::manager::Analysis for Congruence {
    const NAME: &'static str = "congruence classes";
    const READS: crate::code_rule_dsl::logic::program_image::manager::Reads = crate::code_rule_dsl::logic::program_image::manager::Reads::ALL;
    fn compute(cx: &Cx<'_>) -> Result<Self, String> {
        let v = cx.v();
        let d = cx.get::<FieldsData>()?;
        let f = Fields::over(v, &d);
        let r = refine(v, f, false);
        Ok(Congruence { groups: r.groups(v, f), classed: r.classed(v), keys: head_keys(v) })
    }
    fn maintain(&self, cx: &Cx<'_>, delta: &crate::code_rule_dsl::logic::program_image::manager::Delta) -> Option<Self> {
        use crate::code_rule_dsl::logic::program_image::manager::Reads;
        if delta.touched != Reads::RULES || delta.rebuilt {
            return None;
        }
        let v = cx.v();
        let d = cx.get::<FieldsData>().ok()?;
        let f = Fields::over(v, &d);
        let keys = head_keys(v);
        let mut changed: BTreeSet<String> = keys.iter().filter(|(r, k)| self.keys.get(*r) != Some(*k)).map(|(r, _)| r.clone()).collect();
        changed.extend(self.keys.keys().filter(|r| !keys.contains_key(*r)).cloned());
        if changed.is_empty() {
            return Some(Congruence { groups: self.groups.clone(), classed: self.classed.clone(), keys });
        }
        // A request's copy or the request changed: whether the request is its own (`fixed`) may change, though it
        // reads no changed head.
        if changed.iter().any(|r| request(r) || crate::code_rule_dsl::logic::rr::lower::ON_REQUEST.iter().flat_map(|round| round.iter()).any(|a| r.starts_with(&format!("rr_wide_{}_", a.rel)))) {
            return None;
        }
        // The cone: the changed heads and every relation reading one, through any others.
        let mut cone: BTreeSet<String> = changed.clone();
        let mut todo: Vec<String> = changed.iter().cloned().collect();
        while let Some(rel) = todo.pop() {
            let Some(id) = v.rel(&rel) else { continue };
            for (r, _) in v.readers(id) {
                let h = v.rel_name(r.head().rel());
                if cone.insert(h.to_owned()) {
                    todo.push(h.to_owned());
                }
            }
            for n in f.natives().iter().filter(|n| n.reads.contains(&id)) {
                let h = v.rel_name(n.rel);
                if cone.insert(h.to_owned()) {
                    todo.push(h.to_owned());
                }
            }
        }
        if cone.len() * 4 > self.classed.len().max(1) {
            return None;
        }
        // The cone's relations in order, each after what it reads of the cone; a cycle in it computes it again.
        let reads_of = |rel: &str| -> BTreeSet<String> {
            v.rel(rel).into_iter().flat_map(|id| v.heads(id)).flat_map(|r| r.body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel()).to_owned()).collect::<Vec<_>>()).filter(|x| cone.contains(x)).collect()
        };
        let mut order: Vec<String> = Vec::new();
        let mut placed: BTreeSet<String> = BTreeSet::new();
        let mut pending: BTreeMap<String, BTreeSet<String>> = cone.iter().map(|r| (r.clone(), reads_of(r))).collect();
        while !pending.is_empty() {
            let ready: Vec<String> = pending.iter().filter(|(_, rs)| rs.iter().all(|x| placed.contains(x))).map(|(r, _)| r.clone()).collect();
            if ready.is_empty() {
                return None;
            }
            for r in ready {
                pending.remove(&r);
                placed.insert(r.clone());
                order.push(r);
            }
        }
        // The classes outside the cone, kept: by signature, its least member.
        let mut classed: BTreeMap<String, (String, u64)> = self.classed.iter().filter(|(r, _)| !cone.contains(*r)).map(|(r, c)| (r.clone(), c.clone())).collect();
        // A class whose least member leaves it: its other members' readers' labels change.
        for (r, (least, _)) in &self.classed {
            if cone.contains(r) && r == least && self.classed.iter().any(|(o, (l, _))| o != r && l == r && !cone.contains(o)) {
                return None;
            }
        }
        let mut by_sig: HashMap<u64, String> = classed.values().map(|(l, s)| (*s, l.clone())).collect();
        let mut rules_of: HashMap<RelId, Vec<RuleRef<'_>>> = HashMap::new();
        for rel in &order {
            if let Some(id) = v.rel(rel) {
                rules_of.insert(id, v.heads(id).collect());
            }
        }
        let with_facts: BTreeSet<&str> = f.fact_rels().into_iter().filter(|r| !r.about() && r.rows().len() > 0).map(|r| v.text(r.name())).collect();
        for rel in &order {
            let id = v.rel(rel);
            let rules: &[RuleRef<'_>] = id.and_then(|id| rules_of.get(&id)).map_or(&[][..], |rs| rs.as_slice());
            if (rules.is_empty() && !with_facts.contains(rel.as_str())) || fixed(v, f, rel) {
                continue;
            }
            let label = |x: RelId| -> String {
                let name = v.rel_name(x);
                classed.get(name).map_or_else(|| name.to_owned(), |(l, _)| l.clone())
            };
            let initial = fx(&first_key(v, f, rel, rules));
            let mut sig: Vec<(u64, Vec<String>)> = rules
                .iter()
                .map(|&x| {
                    let (toks, read) = Canon::of(v, x, false);
                    (fx(&toks), read.into_iter().map(label).collect())
                })
                .collect();
            sig.sort();
            sig.dedup();
            // The signature as `classed` hashes it: the labels by text.
            let sig: Vec<(u64, Vec<&str>)> = sig.iter().map(|(t, ls)| (*t, ls.iter().map(String::as_str).collect())).collect();
            let s = fx(&(initial, sig));
            let least = match by_sig.get(&s) {
                Some(l) if l.as_str() < rel.as_str() => l.clone(),
                Some(_) => return None,
                None => {
                    by_sig.insert(s, rel.clone());
                    rel.clone()
                }
            };
            classed.insert(rel.clone(), (least, s));
        }
        // A relation outside the cone whose label is a cone member's: its signature reads that member's class.
        if self.classed.iter().any(|(r, (l, _))| !cone.contains(r) && cone.contains(l)) {
            return None;
        }
        let mut groups: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for (r, (l, _)) in &classed {
            groups.entry(l.as_str()).or_default().push(r.clone());
        }
        // In the order of their least members, as `congruence` numbers its classes.
        let groups = filter_classes(groups.into_values(), &read_rels(v, f));
        Some(Congruence { groups, classed, keys })
    }
    fn rows(&self, _v: &Version) -> Option<Vec<crate::code_rule_dsl::logic::program_image::manager::Row>> {
        use crate::code_rule_dsl::logic::front::Value;
        let s = |x: &str| Value::Str(x.to_owned().into());
        let n = |k: u64| Value::Int(k as i64);
        let mut rows: Vec<Vec<Value>> = self.classed.iter().map(|(r, (l, sig))| vec![s("c"), s(r), s(l), n(*sig)]).collect();
        rows.extend(self.keys.iter().map(|(r, k)| vec![s("k"), s(r), n(*k)]));
        for (i, g) in self.groups.iter().enumerate() {
            rows.extend(g.iter().map(|r| vec![s("g"), n(i as u64), s(r)]));
        }
        Some(rows)
    }
    fn of_rows(_v: &Version, rows: &[crate::code_rule_dsl::logic::program_image::manager::Row]) -> Option<Self> {
        use crate::code_rule_dsl::logic::front::Value;
        let text = |x: &Value| if let Value::Str(s) = x { Some(s.to_string()) } else { None };
        let int = |x: &Value| if let Value::Int(k) = x { Some(*k as u64) } else { None };
        let (mut classed, mut keys, mut groups) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::<u64, Vec<String>>::new());
        for r in rows {
            match (text(r.first()?)?.as_str(), &r[1..]) {
                ("c", [x, l, sig]) => drop(classed.insert(text(x)?, (text(l)?, int(sig)?))),
                ("k", [x, k]) => drop(keys.insert(text(x)?, int(k)?)),
                ("g", [i, x]) => groups.entry(int(i)?).or_default().push(text(x)?),
                _ => return None,
            }
        }
        Some(Congruence { groups: groups.into_values().collect(), classed, keys })
    }
}

/// `congruence`'s last filter: a request's class is the request and its copies alone, the rest a class without it;
/// an unread relation joins a class only with a lowering's widened copy of it (`rr_wide_<attr>_<r>`, decision 0174);
/// any other stays apart.
fn filter_classes(groups: impl Iterator<Item = Vec<String>>, read: &BTreeSet<&str>) -> Vec<Vec<String>> {
    groups
        .flat_map(|g| {
            let (with_request, g): (Vec<String>, Vec<String>) = match g.iter().find(|r| request(r)).cloned() {
                Some(req) => g.into_iter().partition(|r| *r == req || copy_of(r, &req)),
                None => (Vec::new(), g),
            };
            [(with_request.len() > 1).then_some(with_request), (!g.is_empty()).then_some(g)].into_iter().flatten()
        })
        .filter_map(|g| {
            if g.iter().any(|r| request(r)) {
                let mut g = g;
                g.sort();
                return Some(g);
            }
            let (mut kept, unread): (Vec<String>, Vec<String>) = g.into_iter().partition(|r| read.contains(r.as_str()));
            let copies = |u: &str| crate::code_rule_dsl::logic::rr::lower::ON_REQUEST.iter().flat_map(|round| round.iter()).any(|a| kept.iter().any(|k| *k == format!("rr_wide_{}_{u}", a.rel)));
            let own = unread.into_iter().find(|u| copies(u));
            kept.extend(own);
            kept.sort();
            (kept.len() > 1).then_some(kept)
        })
        .collect()
}

fn refine<'v>(v: &'v Version, f: Fields<'v>, abstract_constants: bool) -> Refined<'v> {
    let mut rules_of: HashMap<RelId, Vec<RuleRef<'_>>> = HashMap::new();
    for r in v.rules() {
        rules_of.entry(r.head().rel()).or_default().push(r);
    }
    let cands: Vec<&str> = rules_of
        .keys()
        .map(|&r| v.rel_name(r))
        .chain(f.fact_rels().into_iter().filter(|r| !r.about() && r.rows().len() > 0).map(|r| v.text(r.name())))
        .collect::<BTreeSet<&str>>()
        .into_iter()
        .filter(|r| !fixed(v, f, r))
        .collect();
    // Each candidate's number, by relation; every other relation a label of its own (its id).
    let mut number: Vec<Option<u32>> = vec![None; v.rel_end() as usize];
    for (i, r) in cands.iter().enumerate() {
        if let Some(id) = v.rel(r) {
            number[id.0 as usize] = Some(i as u32);
        }
    }
    let rules = |rel: &str| v.rel(rel).and_then(|r| rules_of.get(&r)).map_or(&[][..], |rs| rs.as_slice());
    // Each relation's signatures are its own; the numbering that follows them is in
    // `cands`' order, as one thread gives it, so the classes are the same whatever the threads.
    let mut class: Vec<usize> = Vec::with_capacity(cands.len());
    let mut initial: Vec<u64> = Vec::with_capacity(cands.len());
    {
        let mut ids: HashMap<(Vec<String>, usize, Vec<String>), usize> = HashMap::new();
        for &r in &cands {
            let key = first_key(v, f, r, rules(r));
            initial.push(fx(&key));
            let n = ids.len();
            class.push(*ids.entry(key).or_insert(n));
        }
    }
    let mut templates: HashMap<Vec<Tok<'_>>, (usize, u64)> = HashMap::new();
    let both: Vec<Vec<((usize, u64), Vec<RelId>)>> = cands
        .iter()
        .map(|&r| {
            rules(r)
                .iter()
                .map(|&x| {
                    let (toks, read) = Canon::of(v, x, abstract_constants);
                    let n = templates.len();
                    let h = fx(&toks);
                    (*templates.entry(toks).or_insert((n, h)), read)
                })
                .collect()
        })
        .collect();
    let shapes: Vec<Vec<(usize, Vec<RelId>)>> = both.iter().map(|s| s.iter().map(|((t, _), read)| (*t, read.clone())).collect()).collect();
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
    let shapes = both.into_iter().map(|s| s.into_iter().map(|((_, h), read)| (h, read)).collect()).collect();
    Refined { cands, number, class, initial, shapes }
}


/// Relations equal up to their constants: how many, in how many groups, and a few
/// groups. A congruence of its own over the whole program, so asked for only when the run is profiled.
pub fn similar_relations(v: &Version, f: Fields<'_>) -> (usize, usize, Vec<String>) {
    let similar = congruence(v, f, true);
    (similar.iter().map(Vec::len).sum(), similar.len(), similar.iter().take(3).map(|g| g.iter().take(4).cloned().collect::<Vec<_>>().join(" ~ ")).collect())
}

#[cfg(test)]
mod tests {
    use crate::code_rule_dsl::logic::program_image::manager::Pipeline;
    use crate::code_rule_dsl::logic::program_image::{Builder, T, Version};
    use std::sync::Arc;

    fn read(text: &str) -> Arc<Version> {
        crate::code_rule_dsl::logic::rr::parse_version(text).unwrap()
    }

    fn heads(v: &Version) -> Vec<String> {
        v.rules().map(|r| v.rel_name(r.head().rel()).to_owned()).collect()
    }

    /// Two programs' copies of a relation go to the shared namespace (`s_q`) and their readers read it; an output
    /// merged keeps its name as a copy of its class; relations of equal facts merge; a repeated rule is dropped
    /// (`DropRepeated` around `MergeIdentical`, as the preparation runs them).
    #[test]
    fn the_merges_on_a_version() {
        let text = ".decl e(x: number, y: number)\n.input e\n\
                    .decl p0_q(x: number)\n.decl p1_q(x: number)\np0_q(x) :- e(x, _).\np1_q(x) :- e(x, _).\np1_q(x) :- e(x, _).\n\
                    .decl p0_r(x: number)\n.decl p1_r(x: number)\np0_r(x) :- p0_q(x), e(_, x).\np1_r(x) :- p1_q(x), e(_, x).\n\
                    .decl f1(k: symbol)\n.decl f2(k: symbol)\nf1(\"a\").\nf2(\"a\").\n\
                    .decl uses(x: number, k: symbol)\nuses(x, k) :- p0_r(x), f1(k).\nuses(x, k) :- p1_r(x), f2(k).\n\
                    .output uses, p0_r\n";
        let mut p = Pipeline::new("test", read(text));
        let (mut merge, mut repeats) = (super::MergeIdentical::default(), super::DropRepeated::default());
        p.program(&mut repeats).program(&mut merge).program(&mut repeats);
        let (out, _) = p.finish().unwrap();
        assert!(merge.merged > 0 && merge.shared > 0 && repeats.0 > 0, "the program merges, shares and repeats: {} {} {}", merge.merged, merge.shared, repeats.0);
        let heads = heads(&out);
        assert!(heads.iter().any(|h| h == "s_q"), "{}", out.show());
        assert!(heads.iter().any(|h| h == "p0_r"), "the output keeps its name:\n{}", out.show());
        assert_eq!(heads.iter().filter(|h| *h == "uses").count(), 1, "uses' two rules are one once their reads merge:\n{}", out.show());
    }

    /// A request relation (decision 0174, `rr_node_text_req`) keeps its own: equal rules do not merge it with another
    /// relation (`fixed`). PR #508's c80b57f39 (t47): the version's merge took it as any relation, merged 1 where the
    /// tree's merged none, and a replay sample failed the debug check "the version's merges are the tree's". The
    /// tree's answer, stated: nothing merged or shared, both relations keep their rules.
    #[test]
    fn a_request_relation_keeps_its_own_on_a_version_as_on_the_tree() {
        let text = ".decl e(x: number, y: number)\n.input e\n\
                    .decl rr_node_text_req(n: number)\nrr_node_text_req(n) :- e(n, _).\n\
                    .decl other(n: number)\nother(n) :- e(n, _).\n\
                    .decl uses(n: number)\nuses(n) :- other(n), rr_node_text_req(n).\n.output uses\n";
        let mut p = Pipeline::new("test", read(text));
        let (mut merge, mut repeats) = (super::MergeIdentical::default(), super::DropRepeated::default());
        p.program(&mut repeats).program(&mut merge).program(&mut repeats);
        let (out, _) = p.finish().unwrap();
        assert_eq!((merge.merged, merge.shared, repeats.0), (0, 0, 0), "{}", out.show());
        for rel in ["rr_node_text_req", "other", "uses"] {
            assert_eq!(out.rel(rel).map(|id| out.heads(id).count()), Some(1), "{rel}: {}", out.show());
        }
    }

    /// A request relation's repeated rule dropped: the rule kept serves both copies' readers, stated by rule
    /// (decision 0331). Left by place over the rules dropped, the merge's
    /// renamed readers after it fell on other rules (the combine's tail on corpus0 x B-now).
    #[test]
    fn a_repeated_request_rule_dropped_keeps_both_readers_by_rule() {
        use std::collections::BTreeSet;
        let text = ".decl a(x: number)\n.input a\n.decl b(x: number)\n.input b\n.decl q(x: number)\n.output q\n\
                    q(x) :- a(x).\nq(x) :- b(x).\n";
        let by = |rs: &[&str]| rs.iter().map(|r| (*r).to_owned()).collect::<BTreeSet<String>>();
        // `text`'s version with record `asked` for `q`, and `q`'s first rule repeated after its others when `repeat`,
        // as one base image: a text's repeated rule is read as one (the version lowering, 0276 step 6), and a repeat
        // reaches the pass from a merge of two programs' rules.
        let image = |repeat: bool, asked: Vec<BTreeSet<String>>| {
            let v = read(text);
            let mut b = Builder::on(&v);
            if repeat {
                b.rule().head("q", &[T::Var("x")]).atom("a", &[T::Var("x")]).finish().unwrap();
            }
            b.fields().set_asked(&std::collections::BTreeMap::from([("q".to_owned(), asked)]));
            let (o, _) = b.finish().unwrap();
            Version::open(o.fold().unwrap().base_bytes().unwrap()).unwrap()
        };
        let v = image(true, vec![by(&["r1"]), by(&["r2"]), by(&["r3"])]);
        assert_eq!(v.rel("q").map(|q| v.heads(q).count()), Some(3));
        let mut p = Pipeline::new("test", v);
        let mut repeats = super::DropRepeated::default();
        p.program(&mut repeats);
        let out = p.current().unwrap().clone();
        assert_eq!((repeats.0, out.rules().count()), (1, 2));
        let want = image(false, vec![by(&["r1", "r3"]), by(&["r2"])]);
        assert_eq!(out.asked().get("q"), Some(&vec![BTreeSet::from(["r1".to_owned(), "r3".to_owned()]), BTreeSet::from(["r2".to_owned()])]), "the step's own record, by rule");
        assert!(crate::code_rule_dsl::logic::program_image::same_program(&want, &out), "{}", crate::code_rule_dsl::logic::program_image::differences(&want, &out));
    }

    /// A library's facts may go undeclared (`param_slot("datetime.datetime", "tzinfo", 7).`): such relations merge by
    /// their facts, and the merge does not ask for a declaration they lack.
    #[test]
    fn undeclared_fact_relations_merge() {
        let text = ".decl e(x: number)\n.input e\nf1(\"a\", 1).\nf2(\"a\", 1).\n\
                    .decl uses(x: number, k: symbol)\nuses(x, k) :- e(x), f1(k, _).\nuses(x, k) :- e(x), f2(k, _).\n\
                    .output uses\n";
        let mut p = Pipeline::new("test", read(text));
        let mut merge = super::MergeIdentical::default();
        p.program(&mut merge);
        p.finish().unwrap();
        assert_eq!(merge.merged, 1);
    }

    /// A request and its widened copy left with the request's own rules merge into the request (decision 0174's copy,
    /// Q3): the request's rules serve the copy's readers too (decision 0331). Another relation equal to the request
    /// stays apart.
    #[test]
    fn a_request_merges_with_its_own_copy_only() {
        let text = ".decl k(x: number)\n.input k\n.decl rr_token_req(x: number)\n.decl rr_wide_node_text_rr_token_req(x: number)\n\
                    .decl other(x: number)\n.decl a(x: number)\n.decl b(x: number)\n\
                    rr_token_req(x) :- k(x), x < 3.\nrr_wide_node_text_rr_token_req(x) :- k(x), x < 3.\nother(x) :- k(x), x < 3.\n\
                    a(x) :- rr_token_req(x).\nb(x) :- rr_wide_node_text_rr_token_req(x), other(x).\n.output a, b\n";
        let by = |r: &[&str]| r.iter().map(|s| (*s).to_owned()).collect::<std::collections::BTreeSet<String>>();
        let read = read(text);
        let mut b = Builder::on(&read);
        let asked = std::collections::BTreeMap::from([("rr_token_req".to_owned(), vec![by(&["a"])]), ("rr_wide_node_text_rr_token_req".to_owned(), vec![by(&["b"])])]);
        b.fields().set_asked(&asked);
        let (v, _) = b.finish().unwrap();
        let mut p = Pipeline::new("test", v);
        let mut merge = super::MergeIdentical::default();
        p.program(&mut merge);
        let (out, _) = p.finish().unwrap();
        assert_eq!(merge.merged, 1, "the copy merges into the request, `other` stays apart");
        assert!(heads(&out).iter().any(|h| h == "other"), "{}", out.show());
        assert_eq!(out.asked(), std::collections::BTreeMap::from([("rr_token_req".to_owned(), vec![by(&["a", "b"])])]));
    }

    /// A rule's branded atoms (decision 0270, a lowered program's names) compare as `canonical` writes them, the atom
    /// itself: `a1` and `a2` read the same branded atom and merge, `b` reads another and stays apart, though the three
    /// rules differ in nothing else.
    #[test]
    fn rules_merge_only_on_the_same_branded_atom() {
        use crate::code_rule_dsl::logic::front::Branded;
        let mut b = Builder::base();
        b.fields().declare("e", &[("x", "number"), ("k", "symbol")]);
        for rel in ["a1", "a2", "b", "uses"] {
            b.fields().declare(rel, &[("x", "number")]);
        }
        b.fields().set_inputs(&["e"]);
        b.fields().set_outputs(&["uses"]);
        for (rel, atom) in [("a1", 1), ("a2", 1), ("b", 2)] {
            b.rule().head(rel, &[T::Var("x")]).atom("e", &[T::Var("x"), T::Branded(Branded::name(atom))]).finish().unwrap();
        }
        b.rule().head("uses", &[T::Var("x")]).atom("a1", &[T::Var("x")]).atom("a2", &[T::Var("x")]).atom("b", &[T::Var("x")]).finish().unwrap();
        let (v, _) = b.finish().unwrap();
        assert!(v.show().matches("e($x, ").count() == 3 && !v.show().contains("\"p\""), "branded: {}", v.show());
        let mut p = Pipeline::new("test", v);
        let mut merge = super::MergeIdentical::default();
        p.program(&mut merge);
        let (out, _) = p.finish().unwrap();
        assert_eq!(merge.merged, 1, "a1 and a2 alone: {}", out.show());
        let heads = heads(&out);
        assert!(heads.iter().any(|h| h == "b") && !(heads.iter().any(|h| h == "a1") && heads.iter().any(|h| h == "a2")), "{}", out.show());
    }
}
