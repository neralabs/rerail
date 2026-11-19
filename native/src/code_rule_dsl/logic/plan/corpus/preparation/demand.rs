//! The reach form and the demand for negated tests, on the version (decision 0281's path, step 6d):
//!
//! - `ReachForm`: the closures held as each start's bitmap, and the relations through them (decision 0119,
//!   `reach_form`);
//! - `NegatedDemand`: a relation some rule negates computed for the keys its readers test (decision 0038);
//! - `UndoInCycles`: a closure a later rewrite put in a cycle of other relations made rows again.
//!
//! The first and the last edit the program one relation at a time, each edit read by the next relation's shape: they
//! keep a model of what each relation reads, edited as they go, and write the program once.

use crate::code_rule_dsl::logic::front::AXIS;
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{About, AtomRef, Builder, Cell, Lit, NativeKind, NativeSpec, Rewrite, RuleId, RuleRef, T, TermRef, Version, t_of};
use crate::code_rule_dsl::logic::reach_form::{QUALIFIER, SEED};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

type Result<T> = std::result::Result<T, String>;

/// Native about rows `name` with the rows of `drop`'s relations left out and `add` added, in `program_facts::
/// write_field`'s order: by the native's name (column 1), then by column 2 where it is a number.
pub(super) fn edit_native_rows(b: &mut Builder<'_>, name: &str, drop: &HashSet<String>, add: Vec<Vec<Cell>>) {
    let mut rows = b.about_rows(name);
    let rel_of = |b: &Builder<'_>, r: &[Cell]| match r.get(1) {
        Some(Cell::Str(a)) => b.text(*a).to_owned(),
        _ => String::new(),
    };
    rows.retain(|r| !drop.contains(&rel_of(b, r)));
    if add.is_empty() && rows.len() == b.about_rows(name).len() {
        return;
    }
    rows.extend(add);
    rows.sort_by_cached_key(|r| {
        let n = match r.get(2) {
            Some(Cell::Int(n)) => *n,
            _ => 0,
        };
        (rel_of(b, r), n)
    });
    b.set_about_rows(name, rows);
}

/// The program's name cell of an about row (column 0).
pub(super) fn about_p(b: &mut Builder<'_>) -> Cell {
    Cell::Str(b.atom(crate::code_rule_dsl::logic::program_image::ABOUT))
}

pub(super) fn qualified(v: &Version, about: &About, rel: &str, word: &str) -> bool {
    about.qualifiers.iter().any(|(r, w)| v.text(*r) == rel && v.text(*w) == word)
}

fn facts_nonempty(v: &Version, rel: &str) -> bool {
    v.find(rel).and_then(|a| v.facts(a)).is_some_and(|f| f.rows().len() > 0)
}

pub(super) fn is_input(v: &Version, rel: &str) -> bool {
    v.inputs().any(|a| v.text(a) == rel)
}

fn var(t: Option<TermRef<'_>>) -> Option<crate::code_rule_dsl::logic::program_image::Slot> {
    t.and_then(|t| t.var())
}

/// The two positive, ungated atoms of a body of exactly two literals.
fn two_atoms<'v>(r: RuleRef<'v>) -> Option<(AtomRef<'v>, AtomRef<'v>)> {
    let body: Vec<Lit<'v>> = r.body().map(|l| l.view()).collect();
    match body.as_slice() {
        [Lit::Atom(a), Lit::Atom(b)] if !a.neg() && !b.neg() && !a.gated() && !b.gated() => Some((*a, *b)),
        _ => None,
    }
}

/// What each relation's rules and natives read, as the passes edit it.
struct Model {
    /// Each rule's head and the relations its body reads, by id, in the version's order; a rule made anew has none.
    rules: Vec<(Option<RuleId>, String, Vec<String>)>,
    /// Each native and the relations it reads.
    natives: BTreeMap<String, (NativeKind, Vec<String>)>,
}

impl Model {
    fn of(v: &Version, about: &About) -> Model {
        let rules = v.rules().map(|r| (Some(r.id()), v.rel_name(r.head().rel()).to_owned(), r.body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel()).to_owned()).collect())).collect();
        let natives = about.natives.iter().map(|n| (v.rel_name(n.rel).to_owned(), (n.kind, n.reads.iter().map(|&r| v.rel_name(r).to_owned()).collect()))).collect();
        Model { rules, natives }
    }
    /// The relations each relation's rules and native read.
    fn reads(&self) -> BTreeMap<&str, BTreeSet<&str>> {
        let mut out: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for (_, head, body) in &self.rules {
            out.entry(head.as_str()).or_default().extend(body.iter().map(String::as_str));
        }
        for (rel, (_, reads)) in &self.natives {
            out.entry(rel.as_str()).or_default().extend(reads.iter().map(String::as_str));
        }
        out
    }
    /// A relation `rel`'s rules read (but `rel` itself) that reads `rel` back: `rel` is in a cycle of others.
    fn cycle_through(&self, rel: &str) -> Option<String> {
        let reads = self.reads();
        // Whether `from` reads `rel`, directly or through other relations.
        let depends = |from: &str| {
            let mut seen = BTreeSet::new();
            let mut todo = vec![from];
            while let Some(r) = todo.pop() {
                if r == rel {
                    return true;
                }
                if seen.insert(r) {
                    todo.extend(reads.get(r).into_iter().flatten().copied());
                }
            }
            false
        };
        let direct: BTreeSet<&str> = reads.get(rel).into_iter().flatten().copied().filter(|r| *r != rel).collect();
        direct.into_iter().find(|r| depends(r)).map(str::to_owned)
    }
}

// ---- the reach form ----

/// Each relation found in the reach form (`reach_form`, decision 0119), rewritten: the closures and the relations
/// through them, how many of each.
#[derive(Default)]
pub struct ReachForm {
    pub reaches: usize,
    pub throughs: usize,
}

impl ProgramPass for ReachForm {
    const NAME: &'static str = "the reach form";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let v = cx.v();
        let about = About::of(v);
        let auto = !std::env::var("RERAIL_REACH_INDEX").is_ok_and(|x| x == "off");
        let words = |word: &str| -> BTreeSet<String> { about.qualifiers.iter().filter(|(_, w)| v.text(*w) == word).map(|(r, _)| v.text(*r).to_owned()).collect() };
        let (asked, refused) = (words(QUALIFIER), words("no_reach"));
        let mut candidates: Vec<String> = v.decls().iter().filter(|d| d.arity() == 2).map(|d| v.text(d.name()).to_owned()).filter(|r| !refused.contains(r)).collect();
        candidates.sort();
        let lattice: HashSet<&str> = about.lattice.iter().map(|l| v.rel_name(l.rel)).collect();
        let demand: HashSet<&str> = about.demand_list.iter().map(|a| v.text(*a)).collect();
        let gates: HashSet<&str> = about.gate_keys.iter().map(|a| v.text(*a)).collect();
        let requests: HashSet<&str> = about.answers.iter().map(|(q, _)| v.rel_name(*q)).collect();
        let mut model = Model::of(v, &about);
        // Why `rel` may not be held otherwise than as its rules make it: what the planner reads of it by name.
        let held_by_name = |model: &Model, rel: &str| -> Option<&'static str> {
            if model.natives.contains_key(rel) {
                Some("made natively already")
            } else if is_input(v, rel) {
                Some("an input")
            } else if facts_nonempty(v, rel) {
                Some("it has facts")
            } else if lattice.contains(rel) {
                Some("a lattice")
            } else if demand.contains(rel) || gates.contains(rel) || requests.contains(rel) {
                Some("demanded, a gate or an answer")
            } else {
                None
            }
        };
        let heads = |rel: &str| -> Vec<RuleRef<'_>> { v.rel(rel).map(|id| v.heads(id).collect()).unwrap_or_default() };
        // Whether `rel`'s rules could be a relation through a reach (its one rule reads one), for the directive's error.
        let is_through = |rel: &str| {
            let rules = heads(rel);
            rules.len() == 1 && rules[0].body().count() == 2
        };
        // Each reach: its recursive rule (replaced), its seeds' rules (renamed) and its edge.
        let mut reaches: BTreeMap<String, (RuleId, String)> = BTreeMap::new();
        // The recursive rule and its edge relation, or why `rel` is not a reach.
        let shape = |rel: &str, model: &Model| -> std::result::Result<(RuleId, String), String> {
            if let Some(why) = held_by_name(model, rel) {
                return Err(why.into());
            }
            let mut recursive = None;
            let mut seeds = 0;
            for r in heads(rel) {
                if !r.body().filter_map(|l| l.atom()).any(|a| v.rel_name(a.rel()) == rel) {
                    seeds += 1;
                    continue;
                }
                if recursive.is_some() {
                    return Err("more than one rule reads it".into());
                }
                let Some((a, b)) = two_atoms(r) else { return Err("its recursive rule is not `r(x, z) :- r(x, y), edge(y, z)`".into()) };
                let (step, edge) = if v.rel_name(a.rel()) == rel { (a, b) } else { (b, a) };
                let h: Vec<TermRef<'_>> = r.head().args().collect();
                let (Some(hx), Some(hz)) = (var(h.first().copied()), var(h.get(1).copied())) else { return Err("its recursive rule's head is not two variables".into()) };
                let (s, e): (Vec<TermRef<'_>>, Vec<TermRef<'_>>) = (step.args().collect(), edge.args().collect());
                let ok = s.len() == 2
                    && e.len() == 2
                    && v.rel_name(edge.rel()) != rel
                    && var(Some(s[0])) == Some(hx)
                    && var(Some(s[1])).is_some_and(|y| var(Some(e[0])) == Some(y) && y != hx && y != hz)
                    && var(Some(e[1])) == Some(hz)
                    && hx != hz;
                if !ok {
                    return Err("its recursive rule is not `r(x, z) :- r(x, y), edge(y, z)`".into());
                }
                recursive = Some((r.id(), v.rel_name(edge.rel()).to_owned()));
            }
            let Some((at, edge)) = recursive else { return Err("no rule reads it".into()) };
            if seeds == 0 {
                return Err("no rule seeds it".into());
            }
            // Its only cycle is its own rule: nothing its rules read (the seeds' bodies and the edges) reads it back.
            if let Some(back) = model.cycle_through(rel) {
                let why = format!("`{back}` reads it and it reads `{back}`");
                // A closure kept as rows by a cycle through other relations, in the explain output.
                crate::code_rule_dsl::logic::profile::decision("reach index: cycles and connected graphs (decision 0119)", || format!("{rel}: a closure left as rows, in a cycle of relations: {why}"));
                return Err(why);
            }
            Ok((at, edge))
        };
        for rel in &candidates {
            if !auto && !asked.contains(rel) {
                continue;
            }
            match shape(rel, &model) {
                // `rel`'s seeds as `rr_reach_seed_<rel>`'s rules, and `rel` made natively from them and `edge`.
                Ok((at, edge)) => {
                    let seed = format!("{SEED}{rel}");
                    for (id, head, body) in &mut model.rules {
                        if *id == Some(at) {
                            *body = vec![seed.clone(), edge.clone()];
                        } else if head == rel {
                            *head = seed.clone();
                        }
                    }
                    model.natives.insert(rel.clone(), (NativeKind::Reach, vec![seed, edge.clone()]));
                    reaches.insert(rel.clone(), (at, edge));
                }
                Err(why) if asked.contains(rel) && !is_through(rel) => return Err(format!(".decl {rel} reach: {why}")),
                Err(_) => {}
            }
        }
        let mut throughs: BTreeMap<String, (String, String)> = BTreeMap::new();
        for rel in &candidates {
            if model.natives.contains_key(rel) || (!auto && !asked.contains(rel)) {
                continue;
            }
            // The reach and the map `rel` is made through, or why it is not.
            let through = (|| -> std::result::Result<(String, String), String> {
                if let Some(why) = held_by_name(&model, rel) {
                    return Err(why.into());
                }
                let rules = heads(rel);
                let [r] = rules.as_slice() else { return Err("not one rule".into()) };
                let Some((a, b)) = two_atoms(*r) else { return Err("its rule is not `s(x, q) :- r(x, e), map(e, q)`".into()) };
                let is_reach = |x: AtomRef<'_>| matches!(model.natives.get(v.rel_name(x.rel())), Some((NativeKind::Reach, _)));
                let (reach, map) = if is_reach(a) { (a, b) } else if is_reach(b) { (b, a) } else { return Err("its rule reads no reach".into()) };
                let h: Vec<TermRef<'_>> = r.head().args().collect();
                let (Some(hx), Some(hq)) = (var(h.first().copied()), var(h.get(1).copied())) else { return Err("its head is not two variables".into()) };
                let (ra, ma): (Vec<TermRef<'_>>, Vec<TermRef<'_>>) = (reach.args().collect(), map.args().collect());
                let ok = ma.len() == 2
                    && v.rel_name(map.rel()) != rel
                    && !is_reach(map)
                    && var(ra.first().copied()) == Some(hx)
                    && var(ra.get(1).copied()).is_some_and(|e| var(Some(ma[0])) == Some(e) && e != hx && e != hq)
                    && var(Some(ma[1])) == Some(hq)
                    && hx != hq;
                if !ok {
                    return Err("its rule is not `s(x, q) :- r(x, e), map(e, q)`".into());
                }
                if let Some(back) = model.cycle_through(rel) {
                    return Err(format!("`{back}` reads it and it reads `{back}`"));
                }
                Ok((v.rel_name(reach.rel()).to_owned(), v.rel_name(map.rel()).to_owned()))
            })();
            match through {
                Ok((reach, map)) => {
                    model.natives.insert(rel.clone(), (NativeKind::ReachThrough, vec![reach.clone(), map.clone()]));
                    throughs.insert(rel.clone(), (reach, map));
                }
                Err(why) if asked.contains(rel) => return Err(format!(".decl {rel} reach: {why}")),
                Err(_) => {}
            }
        }
        self.reaches += reaches.len();
        self.throughs += throughs.len();
        if reaches.is_empty() && throughs.is_empty() {
            return Ok(());
        }
        let input = w.input().clone();
        let b = w.derive()?;
        let recursive: HashMap<RuleId, (&str, &str)> = reaches.iter().map(|(rel, (at, edge))| (*at, (rel.as_str(), edge.as_str()))).collect();
        let mut rw = Seeds { v: &input, reaches: &reaches };
        for r in input.rules() {
            let head = input.rel_name(r.head().rel());
            if let Some((rel, edge)) = recursive.get(&r.id()) {
                // Its one rule only orders it after the seeds and the edges: the evaluator makes its rows.
                let seed = format!("{SEED}{rel}");
                b.rule().head(*rel, &[T::Var("x"), T::Var("z")]).atom(seed.as_str(), &[T::Var("x"), T::Var("y")]).atom(*edge, &[T::Var("y"), T::Var("z")]).finish()?;
            } else if reaches.contains_key(head) {
                b.rewrite(&input, r, &mut rw).finish()?;
            } else {
                b.keep(&input, r);
            }
        }
        let p = about_p(b);
        let mut rows = Vec::new();
        for (rel, (_, edge)) in &reaches {
            let seed = format!("{SEED}{rel}");
            let cols: Vec<(String, String)> = input.decl_of(rel).ok_or_else(|| format!("{rel} is not declared"))?.cols().map(|(n, t)| (input.text(n).to_owned(), input.text(t).to_owned())).collect();
            let cols: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
            b.declare(&seed, &cols);
            let (r, s, e) = (b.atom(rel), b.atom(&seed), b.atom(edge));
            rows.push(vec![p, Cell::Str(r), Cell::Str(s), Cell::Str(e)]);
        }
        edit_native_rows(b, "native_reach", &HashSet::new(), rows);
        let mut rows = Vec::new();
        for (rel, (reach, map)) in &throughs {
            let (r, x, m) = (b.atom(rel), b.atom(reach), b.atom(map));
            rows.push(vec![p, Cell::Str(r), Cell::Str(x), Cell::Str(m)]);
        }
        edit_native_rows(b, "native_reach_through", &HashSet::new(), rows);
        Ok(())
    }
}

/// A reach's seeds: its other rules' heads as `rr_reach_seed_<r>`.
struct Seeds<'a> {
    v: &'a Version,
    reaches: &'a BTreeMap<String, (RuleId, String)>,
}

impl Rewrite for Seeds<'_> {
    fn head_as(&mut self, a: AtomRef<'_>) -> Option<String> {
        let rel = self.v.rel_name(a.rel());
        self.reaches.contains_key(rel).then(|| format!("{SEED}{rel}"))
    }
}

// ---- reaches in cycles ----

/// The reaches a later rewrite put in a cycle of other relations, made rows again. `ReachForm` takes a closure only
/// when nothing it reads reads it back, but the rewrites after it (magic sets' demand, decision 0145) add readers: at
/// O125, `magic_found_bbf` reads `anc` and feeds `found`, which `base`, `anc`'s edge, reads. A native operator's
/// stratum has no maintained form (`plan`, decision 0119), so the 44 relations of that component were derived from
/// scratch at every edit. Each such reach's rules are its own again, and a relation made through it is rows too.
/// `undone`: those made rows; one declared `reach` is an error.
#[derive(Default)]
pub struct UndoInCycles {
    pub undone: Vec<String>,
}

impl ProgramPass for UndoInCycles {
    const NAME: &'static str = "reaches in cycles made rows";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let v = cx.v();
        let about = About::of(v);
        let mut reaches: Vec<(String, String)> = about
            .natives
            .iter()
            .filter_map(|n| match n.spec {
                NativeSpec::Reach { seed, .. } => Some((v.rel_name(n.rel).to_owned(), v.rel_name(seed).to_owned())),
                _ => None,
            })
            .collect();
        reaches.sort();
        let mut model = Model::of(v, &about);
        // Each reach undone: its seed's name.
        let mut seeds: BTreeMap<String, String> = BTreeMap::new();
        let mut gone: HashSet<String> = HashSet::new();
        for (rel, seed) in reaches {
            let Some(back) = model.cycle_through(&rel) else { continue };
            if qualified(v, &about, &rel, QUALIFIER) {
                return Err(format!(".decl {rel} reach: after demand, `{back}` reads it and it reads `{back}`"));
            }
            crate::code_rule_dsl::logic::profile::decision("reach index: cycles and connected graphs (decision 0119)", || format!("{rel}: made rows again, in a cycle a later rewrite made: `{back}` reads it and it reads `{back}`"));
            model.natives.remove(&rel);
            for (_, head, body) in &mut model.rules {
                if *head == seed {
                    *head = rel.clone();
                } else if *head == rel {
                    for b in body.iter_mut().filter(|b| **b == seed) {
                        *b = rel.clone();
                    }
                }
            }
            let through: Vec<String> = model.natives.iter().filter(|(_, (k, reads))| *k == NativeKind::ReachThrough && reads.first() == Some(&rel)).map(|(r, _)| r.clone()).collect();
            for t in through {
                model.natives.remove(&t);
                gone.insert(t.clone());
                self.undone.push(t);
            }
            gone.insert(rel.clone());
            seeds.insert(seed, rel.clone());
            self.undone.push(rel);
        }
        if seeds.is_empty() {
            return Ok(());
        }
        let input = w.input().clone();
        let undone: HashSet<&str> = seeds.values().map(String::as_str).collect();
        let b = w.derive()?;
        let mut rw = Unseed { v: &input, seeds: &seeds, in_reach: false };
        for r in input.rules() {
            let head = input.rel_name(r.head().rel());
            if seeds.contains_key(head) || undone.contains(head) {
                rw.in_reach = undone.contains(head);
                b.rewrite(&input, r, &mut rw).finish()?;
            } else {
                b.keep(&input, r);
            }
        }
        for seed in seeds.keys() {
            b.undeclare(seed);
        }
        edit_native_rows(b, "native_reach", &gone, Vec::new());
        edit_native_rows(b, "native_reach_through", &gone, Vec::new());
        Ok(())
    }
}

/// A reach's rules its own again: a seed's head is the reach, and the reach's own rule reads it in its seed's place.
struct Unseed<'a> {
    v: &'a Version,
    seeds: &'a BTreeMap<String, String>,
    /// Whether the rule written is the reach's own (its body's seed atoms renamed).
    in_reach: bool,
}

impl Rewrite for Unseed<'_> {
    fn head_as(&mut self, a: AtomRef<'_>) -> Option<String> {
        self.seeds.get(self.v.rel_name(a.rel())).cloned()
    }
    fn atom_as(&mut self, a: AtomRef<'_>) -> Option<(String, bool, bool)> {
        // A reach's own rule is two atoms (`rr_reach_seed_<r>(x, y), edge(y, z)`).
        if !self.in_reach {
            return None;
        }
        self.seeds.get(self.v.rel_name(a.rel())).map(|r| (r.clone(), a.neg(), a.gated()))
    }
}

// ---- demand for negated tests ----

/// Demand for the tests a rule negates (decision 0038's corpus path): a relation whose rules each start from every
/// node (`nearest($n, $a, set)`, `inside_<set>($n, $a)` on its key) and that some rule negates is computed for every
/// node of a chunk, though its readers ask about the nodes they have bound (`!m($n)`). Demanded, it is computed for
/// the keys `demand_<r>` holds only: each reader's tests of the key alone (its kind, say). Not `KeyDemand` (T's
/// `magic_sets`), whose demand is a reader's whole positive prefix: without the reader's negations that can be far
/// more than the test. A relation qualifies when it is derived and not kept (an output, input, lattice, relation with
/// facts, gate or already demanded), not recursive, negated somewhere, read only by atoms with a variable key, and
/// every rule reads its key from such an atom. A relation some reader gives no test of the key alone stays whole.
/// `demanded`: the relations demanded (`Prepared::demanded`).
#[derive(Default)]
pub struct NegatedDemand {
    pub demanded: Vec<String>,
}

impl ProgramPass for NegatedDemand {
    const NAME: &'static str = "demand for negated tests";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let v = cx.v();
        let about = About::of(v);
        let outputs: HashSet<&str> = v.outputs().map(|a| v.text(a)).collect();
        let lattice: HashSet<&str> = about.lattice.iter().map(|l| v.rel_name(l.rel)).collect();
        let natives: HashSet<&str> = about.natives.iter().map(|n| v.rel_name(n.rel)).collect();
        let demand_list: HashSet<&str> = about.demand_list.iter().map(|a| v.text(*a)).collect();
        let gates: HashSet<&str> = about.gate_list.iter().map(|g| v.text(g.key)).collect();
        let kept = |rel: &str| {
            outputs.contains(rel)
                || is_input(v, rel)
                || lattice.contains(rel)
                || natives.contains(rel)
                || facts_nonempty(v, rel)
                || demand_list.contains(rel)
                || gates.contains(rel)
                // A scope's documents (decision 0091's guards): a row per document, never worth a demand.
                || rel.contains("rr_scope_")
                // `.decl r(…) no_magic` (decision 0099): the program keeps it whole.
                || qualified(v, &about, rel, "no_magic")
        };
        let mut by_rel: BTreeMap<&str, Vec<RuleRef<'_>>> = BTreeMap::new();
        for r in v.rules() {
            by_rel.entry(v.rel_name(r.head().rel())).or_default().push(r);
        }
        let (mut negated, mut refused): (BTreeSet<&str>, BTreeSet<&str>) = Default::default();
        for r in v.rules() {
            let head = v.rel_name(r.head().rel());
            for l in r.body() {
                match l.view() {
                    Lit::Atom(a) if matches!(a.args().next(), Some(TermRef::Var(_))) => {
                        let rel = v.rel_name(a.rel());
                        if a.neg() {
                            negated.insert(rel);
                        }
                        if rel == head {
                            refused.insert(rel);
                        }
                    }
                    Lit::Atom(a) | Lit::Agg { atom: a, .. } => {
                        refused.insert(v.rel_name(a.rel()));
                    }
                    _ => {}
                }
            }
        }
        let per_node = |r: &RuleRef<'_>| -> bool {
            let Some(TermRef::Var(key)) = r.head().args().next() else { return false };
            r.body().any(|l| match l.view() {
                Lit::Nearest { n, .. } => n == key,
                Lit::Atom(a) => !a.neg() && v.rel_name(a.rel()).starts_with(AXIS) && var(a.args().next()) == Some(key),
                _ => false,
            })
        };
        let mut demand: Vec<String> = by_rel.iter().filter(|(rel, rules)| negated.contains(*rel) && !refused.contains(*rel) && !kept(rel) && rules.iter().all(per_node)).map(|(rel, _)| (*rel).to_owned()).collect();
        // Each reader's demand: its positive atoms of input relations over the key alone (`node(_, $n, "call", _, _)`),
        // a superset of the keys it asks about, cheap to read, the key renamed `k`. A relation one reader gives no such
        // atom stays whole.
        let mut wanted: BTreeMap<String, Vec<Vec<(String, Vec<TermRef<'_>>, RuleRef<'_>, crate::code_rule_dsl::logic::program_image::Slot)>>> = BTreeMap::new();
        let mut unbounded: BTreeSet<String> = BTreeSet::new();
        for r in v.rules() {
            for l in r.body() {
                let Lit::Atom(a) = l.view() else { continue };
                let rel = v.rel_name(a.rel());
                if !demand.iter().any(|x| x == rel) {
                    continue;
                }
                let Some(TermRef::Var(key)) = a.args().next() else { continue };
                let only_key = |b: AtomRef<'_>| {
                    let mut vars = BTreeSet::new();
                    b.args().for_each(|t| term_vars(t, &mut vars));
                    !b.neg() && is_input(v, v.rel_name(b.rel())) && vars.iter().all(|x| *x == key) && vars.contains(&key)
                };
                let atoms: Vec<_> = r.body().filter_map(|m| match m.view() {
                    Lit::Atom(b) if only_key(b) => Some((v.rel_name(b.rel()).to_owned(), b.args().collect(), r, key)),
                    _ => None,
                }).collect();
                if atoms.is_empty() {
                    unbounded.insert(rel.to_owned());
                } else {
                    wanted.entry(rel.to_owned()).or_default().push(atoms);
                }
            }
        }
        demand.retain(|d| !unbounded.contains(d) && wanted.contains_key(d));
        if demand.is_empty() {
            return Ok(());
        }
        let input = w.input().clone();
        let names: BTreeMap<&str, String> = demand.iter().map(|rel| (rel.as_str(), format!("demand_{rel}"))).collect();
        let b = w.derive()?;
        for r in input.rules() {
            match names.get(input.rel_name(r.head().rel())) {
                Some(name) => {
                    let key = r.head().args().next().ok_or("a demanded relation of no columns")?;
                    let mut wr = b.rule().head_from(&input, r, r.head()).atom(name.as_str(), &[t_of(&input, r, key)]);
                    for l in r.body() {
                        wr = wr.lit(&input, r, l);
                    }
                    wr.finish()?;
                }
                None => {
                    b.keep(&input, r);
                }
            }
        }
        for rel in &demand {
            let name = &names[rel.as_str()];
            let ty = input.text(input.decl_of(rel).ok_or_else(|| format!("{rel} is not declared"))?.cols().next().ok_or("a demanded relation of no columns")?.1).to_owned();
            b.declare(name, &[("k", ty.as_str())]);
            // Each distinct body once: an atom's text with its key as `k`.
            let mut bodies: BTreeMap<String, &Vec<(String, Vec<TermRef<'_>>, RuleRef<'_>, crate::code_rule_dsl::logic::program_image::Slot)>> = BTreeMap::new();
            for body in wanted.get(rel).into_iter().flatten() {
                let text = body.iter().map(|(a, args, r, key)| format!("{a}({})", args.iter().map(|t| if t.var() == Some(*key) { "k".to_owned() } else { input.show_term(*r, *t) }).collect::<Vec<_>>().join(", "))).collect::<Vec<_>>().join(", ");
                bodies.entry(text).or_insert(body);
            }
            for body in bodies.into_values() {
                let mut wr = b.rule().head(name.as_str(), &[T::Var("k")]);
                for (a, args, r, key) in body {
                    let terms: Vec<T<'_>> = args.iter().map(|t| if t.var() == Some(*key) { T::Var("k") } else { t_of(&input, *r, *t) }).collect();
                    wr = wr.atom(a.as_str(), &terms);
                }
                wr.finish()?;
            }
        }
        self.demanded = demand;
        Ok(())
    }
}

fn term_vars(t: TermRef<'_>, out: &mut BTreeSet<crate::code_rule_dsl::logic::program_image::Slot>) {
    match t {
        TermRef::Var(s) => {
            out.insert(s);
        }
        TermRef::Call(_, args) => args.for_each(|a| term_vars(a, out)),
        _ => {}
    }
}
