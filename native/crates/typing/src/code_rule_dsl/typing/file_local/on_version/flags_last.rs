//! A flag over every file that only gates per-file work is applied last (decision 0686).
//!
//! A flag is a cross-file relation of no column (`plp_anycand()`: is any file a candidate) or of one document's id
//! (`plp_docin(d)`: is `d` analysed), which every file-local rule reading it reads at its own document. Read so, it has
//! one value in each file. Each file derives the relations above it (its cone: those per-file once the flag is
//! placeless) twice, once under each value (`flag_copy`), and the cross-file part takes each row read outside the cone
//! from the copy of its document's value. A negated read of a copy, an aggregate over one and a recursion through one
//! are a copy's own, as the file's rows are under that value: the rewrite is exact, whatever the cone does.

use super::*;
use crate::code_rule_dsl::logic::program_image::build::T;
use crate::code_rule_dsl::logic::program_image::{AtomRef, Builder, LitRef, Rewrite};
use std::sync::Arc;

/// The prefix of a flag's copies (`flag_copy`).
pub const FLAG_COPY: &str = "rr_flag";

/// Relation `rel` derived per file under a flag's value `value` (decision 0686).
pub fn flag_copy(rel: &str, value: bool) -> String {
    format!("{FLAG_COPY}{}_{rel}", u8::from(value))
}

/// The flags applied last (`FileLocal::flags_applied_last`): each, with its cone, and the flags not applied, with why.
#[derive(Debug, Default)]
pub struct LastFlags {
    pub flags: BTreeMap<String, BTreeSet<String>>,
    pub refused: BTreeMap<String, String>,
}

/// One flag applied last in a round: its cone, and the cone's relations read outside it, with the column of each that
/// places its row's document (none for a flag of no column).
struct Applied {
    flag: String,
    cone: BTreeSet<String>,
    selected: Vec<(String, Option<usize>)>,
}

impl FileLocal {
    /// `v` with each flag over every file that only gates per-file work applied last (decision 0686), if one is; and
    /// which were, with their cones. Found again on the program each round leaves, so a cone's copy reading another
    /// flag is applied too (a few rounds at most: each applies flags the last could not).
    pub fn flags_applied_last(v: &Arc<Version>) -> Result<(Option<Arc<Version>>, LastFlags), String> {
        let mut last = LastFlags::default();
        let mut cur = Arc::clone(v);
        let mut changed = false;
        // The relations a round made the cross-file part's choice: never a cone's again.
        let mut chosen: BTreeSet<String> = BTreeSet::new();
        for _ in 0..4 {
            let applied = {
                let d = crate::code_rule_dsl::logic::program_image::manager::analysis::<FieldsData>(&cur)?;
                let f = Fields::over(&cur, &d);
                Program::of(&cur, f).flags_last(&chosen, &mut last)
            };
            if applied.is_empty() {
                break;
            }
            for a in &applied {
                chosen.extend(a.selected.iter().map(|(r, _)| r.clone()));
                last.flags.entry(a.flag.clone()).or_default().extend(a.cone.iter().cloned());
                last.refused.remove(&a.flag);
            }
            cur = rewrite(&cur, &applied)?;
            changed = true;
        }
        Ok((changed.then_some(cur), last))
    }
}

impl<'v> Program<'v> {
    /// The flags this program applies last, with disjoint cones (a cone another flag's overlaps waits for the next
    /// round), and why each other flag read by a rule the split leaves cross-file is not.
    fn flags_last(&self, chosen: &BTreeSet<String>, last: &mut LastFlags) -> Vec<Applied> {
        let rels = &self.rels;
        let n = rels.n;
        let whole = self.split(None, None);
        let flags: Vec<u32> = (0..n as u32)
            .filter(|&r| {
                let rel = self.name(r);
                rels.is_derived(r)
                    && !rels.rules[r as usize].is_empty()
                    && !whole.local.contains(rel)
                    && !whole.constants.contains(rel)
                    && !chosen.contains(rel)
                    && rels.candidate[r as usize]
                    && !rels.output[r as usize]
                    && self.of_documents(&whole, rel)
                    && rels.reading[r as usize].iter().any(|&k| !whole.local.contains(self.name(rels.head[k])))
            })
            .collect();
        if flags.is_empty() {
            return Vec::new();
        }
        let names: BTreeSet<String> = flags.iter().map(|&r| self.name(r).to_owned()).collect();
        let q = Program::without(self.v, self.f, &names);
        let placeless = q.split(None, None);
        let mut applied: Vec<Applied> = Vec::new();
        let mut taken: BTreeSet<String> = BTreeSet::new();
        for &g in &flags {
            let flag = self.name(g);
            match self.cone(g, &whole, &placeless, chosen) {
                Ok(None) => {}
                Ok(Some(a)) if a.cone.iter().any(|r| taken.contains(r)) => drop(a),
                Ok(Some(a)) => {
                    // Taken alone, it makes no crossing the split refuses.
                    let alone = Program::without(self.v, self.f, &BTreeSet::from([flag.to_owned()])).split(None, None);
                    let made: Vec<&String> = alone.crossings.keys().filter(|c| !whole.crossings.contains_key(*c)).collect();
                    if !made.is_empty() {
                        last.refused.insert(flag.to_owned(), format!("not applied last: {} would join documents undeclared", made.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
                        continue;
                    }
                    taken.extend(a.cone.iter().cloned());
                    applied.push(a);
                }
                Err(why) => drop(last.refused.insert(flag.to_owned(), format!("not applied last: {why}"))),
            }
        }
        applied
    }

    /// Whether `rel`'s rows each name one document by their one column, or it has none: a flag's shape.
    fn of_documents(&self, whole: &FileLocal, rel: &str) -> bool {
        if self.f.nullary(rel) || self.f.arity(rel) == Some(0) {
            return true;
        }
        match self.f.columns(rel).as_deref() {
            Some([(_, t)]) => *t != "node" && (whole.placing(rel) == [0] || whole.row_documents.get(rel).is_some_and(|c| c == &[0])),
            _ => false,
        }
    }

    /// Flag `g`'s cone: the relations per-file once it is placeless (`placeless`) and not before, which read it, or a
    /// relation of the cone. `None` where none does; `Err` where it cannot be applied last.
    fn cone(&self, g: u32, whole: &FileLocal, placeless: &FileLocal, chosen: &BTreeSet<String>) -> Result<Option<Applied>, String> {
        let rels = &self.rels;
        let flag = self.name(g);
        let newly = |r: u32| rels.is_derived(r) && placeless.local.contains(self.name(r)) && !whole.local.contains(self.name(r)) && !chosen.contains(self.name(r));
        let mut cone: BTreeSet<u32> = BTreeSet::new();
        let mut todo: Vec<u32> = vec![g];
        while let Some(x) = todo.pop() {
            for &k in &rels.reading[x as usize] {
                let h = rels.head[k];
                if h != g && newly(h) && cone.insert(h) {
                    todo.push(h);
                }
            }
        }
        if cone.is_empty() {
            return Ok(None);
        }
        let in_cone = |r: u32| cone.contains(&r);
        // What the flag reads holds none of the cone: its value is no copy's.
        let mut below: BTreeSet<u32> = BTreeSet::new();
        let mut todo: Vec<u32> = vec![g];
        while let Some(x) = todo.pop() {
            for &k in &rels.rules[x as usize] {
                for a in self.rules[k].reads() {
                    let y = a.rel().0;
                    if in_cone(y) {
                        return Err(format!("it reads {}, which reads it", self.name(y)));
                    }
                    if rels.is_derived(y) && below.insert(y) {
                        todo.push(y);
                    }
                }
            }
        }
        let natives_read = self.f.natives_read();
        // A nullary flag is declared with one unit column (`rr_unit`), read `any(1)`: no document's.
        let one_column = !self.f.nullary(flag) && self.f.arity(flag) == Some(1);
        for &r in &cone {
            let rel = self.name(r);
            let i = r as usize;
            // Each copy is an ordinary relation of the program: one the engine reads by its name is not copied.
            let reason = if !rels.candidate[i] || rels.functional[i] || rels.test[i] {
                Some("a native, lattice, function or test")
            } else if natives_read.contains(rel) {
                Some("a native reads it")
            } else if self.f.demanded_or_gate(rel) || self.f.answer(rel) || self.f.produced(rel) {
                Some("demanded, a gate, or a load's")
            } else if crate::code_rule_dsl::logic::rr::lower::made_on_request(rel) {
                Some("the on-request rewrite's (decision 0174)")
            } else if self.f.has_facts(rel) {
                Some("it has facts stated")
            } else {
                None
            };
            if let Some(c) = reason {
                return Err(format!("{rel} in its cone is {c}"));
            }
            // A rule of the cone reads the flag where it has one value in the file: at the rule's own document (a
            // reader at another, `remote(n) :- …, docin(0)`, is per-file with the flag placeless, so in the cone).
            for &k in &rels.rules[i] {
                for l in self.rules[k].body() {
                    match l.view() {
                        L::Agg { atom, .. } if atom.rel().0 == g => return Err(format!("{rel} aggregates over it")),
                        L::Atom(a) if a.rel().0 == g && one_column && !self.at_own_document(self.rules[k], a, placeless) => {
                            return Err(format!("{rel} reads it at another document than its own"));
                        }
                        L::Nearest { kindset, .. } if cone.iter().any(|&c| self.name(c) == self.v.text(kindset)) => {
                            return Err(format!("{rel} names a relation of its cone as a kind set"));
                        }
                        _ => {}
                    }
                }
            }
        }
        // The cone's relations read outside it, or written out: the cross-file part's choice, each by a column placing
        // its row's document.
        let mut selected: Vec<(String, Option<usize>)> = Vec::new();
        for &r in &cone {
            let rel = self.name(r);
            let outside = rels.output[r as usize] || rels.reading[r as usize].iter().any(|&k| !in_cone(rels.head[k]));
            if !outside {
                continue;
            }
            let at = if one_column {
                match placeless.locations.get(rel).and_then(|c| c.first()) {
                    Some(&c) if self.f.columns(rel).is_some_and(|t| t[c].1 == "number") || self.f.declared("docof") => Some(c),
                    Some(_) => return Err(format!("{rel}, read outside its cone, is placed by a node, and no `docof` names its document")),
                    None => return Err(format!("{rel}, read outside its cone, has no location")),
                }
            } else {
                None
            };
            selected.push((rel.to_owned(), at));
        }
        Ok(Some(Applied { flag: flag.to_owned(), cone: cone.iter().map(|&r| self.name(r).to_owned()).collect(), selected }))
    }

    /// Whether rule `rule`'s read `a` of a one-column flag is at the rule's own document: its argument a variable that
    /// a positive read of another relation holds at a column placing its row's document.
    fn at_own_document(&self, rule: RuleRef<'_>, a: AtomRef<'_>, placeless: &FileLocal) -> bool {
        let Some(TermRef::Var(d)) = a.args().next() else { return false };
        rule.body().any(|l| match l.view() {
            L::Atom(b) if !b.neg() && b.rel() != a.rel() => {
                let rel = self.v.rel_name(b.rel());
                let placing = if self.rels.is_derived(b.rel().0) { placeless.placing(rel) } else { self.base_placing(rel) };
                b.args().enumerate().any(|(c, t)| t.var() == Some(d) && placing.contains(&c))
            }
            _ => false,
        })
    }
}

/// `v` with each of `applied` applied last: the cone's rules copied under each value, the flag's reads dropped where
/// they hold and the rule left out where they fail; a copy no rule derives stated empty; each relation read outside the
/// cone the cross-file part's choice between its copies by the flag's value at its row's document.
fn rewrite(v: &Arc<Version>, applied: &[Applied]) -> Result<Arc<Version>, String> {
    use crate::code_rule_dsl::logic::program_image::fields::FieldsMut;
    let cone_of: BTreeMap<&str, &Applied> = applied.iter().flat_map(|a| a.cone.iter().map(move |r| (r.as_str(), a))).collect();
    let mut b = Builder::on(v);
    b.retain(|r| !cone_of.contains_key(v.rel_name(r.head().rel())));
    // The copies' declarations and qualifiers.
    let mut words: Vec<(String, String)> = v.qualifier_map()?.into_iter().flat_map(|(rel, ws)| ws.into_iter().map(move |w| (rel.clone(), w))).collect();
    for a in applied {
        for rel in &a.cone {
            let cols = v.columns_of(rel).ok_or_else(|| format!("{rel} is not declared"))?;
            let cols: Vec<(&str, &str)> = cols.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
            for value in [false, true] {
                let copy = flag_copy(rel, value);
                b.fields().declare(&copy, &cols);
                let own: Vec<(String, String)> = words.iter().filter(|(r, _)| r == rel).map(|(_, w)| (copy.clone(), w.clone())).collect();
                words.extend(own);
            }
        }
    }
    b.fields().set_qualifiers(&words);
    struct Copied<'a> {
        v: &'a Version,
        flag: &'a str,
        cone: &'a BTreeSet<String>,
        value: bool,
    }
    impl Rewrite for Copied<'_> {
        fn head_as(&mut self, a: AtomRef<'_>) -> Option<String> {
            let rel = self.v.rel_name(a.rel());
            self.cone.contains(rel).then(|| flag_copy(rel, self.value))
        }
        fn atom_as(&mut self, a: AtomRef<'_>) -> Option<(String, bool, bool)> {
            let rel = self.v.rel_name(a.rel());
            self.cone.contains(rel).then(|| (flag_copy(rel, self.value), a.neg(), a.gated()))
        }
        fn keep(&mut self, l: LitRef<'_>) -> bool {
            !matches!(l.view(), L::Atom(a) if self.v.rel_name(a.rel()) == self.flag)
        }
    }
    for a in applied {
        let mut derived: BTreeSet<String> = BTreeSet::new();
        for value in [false, true] {
            for r in v.rules().filter(|r| a.cone.contains(v.rel_name(r.head().rel()))) {
                // A read of the flag that fails under this value leaves the rule out.
                let fails = r.body().any(|l| matches!(l.view(), L::Atom(x) if v.rel_name(x.rel()) == a.flag && x.neg() == value));
                if fails {
                    continue;
                }
                let mut rw = Copied { v, flag: &a.flag, cone: &a.cone, value };
                b.rewrite(v, r, &mut rw).finish()?;
                derived.insert(flag_copy(v.rel_name(r.head().rel()), value));
            }
            for rel in &a.cone {
                let copy = flag_copy(rel, value);
                if !derived.contains(&copy) {
                    b.fields().set_facts(&copy, &[]);
                }
            }
        }
        // The cross-file part's choice: `r(x) :- rr_flag1_r(x), [docof(x_c, d),] flag(d).` and its negation's. A
        // nullary flag is read at its unit, where it is declared with one.
        let unit: Vec<T<'_>> = if v.columns_of(&a.flag).is_some_and(|c| c.len() == 1) { vec![T::Int(1)] } else { Vec::new() };
        for (rel, at) in &a.selected {
            let types = v.columns_of(rel).ok_or_else(|| format!("{rel} is not declared"))?;
            let vars: Vec<String> = (0..types.len()).map(|c| format!("c{c}")).collect();
            let args: Vec<T<'_>> = vars.iter().map(|x| T::Var(x.as_str())).collect();
            for value in [false, true] {
                let copy = flag_copy(rel, value);
                let mut rb = b.rule().head(rel.as_str(), &args).atom(copy.as_str(), &args);
                rb = match at {
                    None => rb.atom_with(a.flag.as_str(), &unit, !value, false),
                    Some(c) if types[*c].1 == "number" => rb.atom_with(a.flag.as_str(), &[T::Var(vars[*c].as_str())], !value, false),
                    Some(c) => rb.atom("docof", &[T::Var(vars[*c].as_str()), T::Var("rr_flag_d")]).atom_with(a.flag.as_str(), &[T::Var("rr_flag_d")], !value, false),
                };
                rb.finish()?;
            }
        }
    }
    b.finish().map(|(v, _)| v)
}
