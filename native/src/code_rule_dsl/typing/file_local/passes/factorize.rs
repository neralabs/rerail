//! `file_local::factorize_exports` on the version (decision 0099; 0281's path, step 5): the tree pass one for one,
//! reading the split from the version (`FileLocal`). Variables are read by name, as the tree reads them, so each
//! choice, counter and name is the tree's; each round's program is a version of its own, split again to check the
//! parts stay file-local.

use super::super::FileLocal;
use super::{BodyItem, Made, NOwned, Round, lit_vars, name, term_vars, write};
use crate::code_rule_dsl::logic::plan::body;
use crate::code_rule_dsl::logic::plan::fields::{Fields, FieldsData};
use crate::code_rule_dsl::logic::program_image::Builder;
use crate::code_rule_dsl::logic::program_image::manager::{self, Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{CmpOp, Lit as L, LitRef, RuleRef, TermRef, Version};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Each export whose rules join parts meeting at a key handed on as the parts (`factorize_exports`): the lines it
/// reports. Off (`on` false) where the run's options turn the factorized store off (`factorize_on`).
pub struct FactorizeExports {
    pub on: bool,
    pub lines: Vec<String>,
}

impl FactorizeExports {
    pub fn new(on: bool) -> FactorizeExports {
        FactorizeExports { on, lines: Vec::new() }
    }
}

type Keys = BTreeMap<String, Vec<Vec<usize>>>;

/// `file_local::determined`: the variables `lits` of `rule` determine from `known`.
fn determined(v: &Version, rule: RuleRef<'_>, lits: &[LitRef<'_>], mut known: BTreeSet<String>, keys: &Keys) -> BTreeSet<String> {
    loop {
        let before = known.len();
        for l in lits {
            match l.view() {
                L::Atom(a) if !a.neg() => {
                    let args: Vec<TermRef<'_>> = a.args().collect();
                    let fixed = |c: &usize| match args.get(*c) {
                        Some(TermRef::Var(s)) => known.contains(&name(v, rule, *s)),
                        Some(TermRef::Int(_) | TermRef::Branded(_) | TermRef::Str(_)) => true,
                        _ => false,
                    };
                    if keys.get(v.rel_name(a.rel())).is_some_and(|ks| ks.iter().any(|k| k.iter().all(fixed))) {
                        known.extend(body::atom_vars(a).iter().map(|s| name(v, rule, s)));
                    }
                }
                L::Cmp { l, op: CmpOp::Eq, r } => {
                    for (x, y) in [(l, r), (r, l)] {
                        if let TermRef::Var(x) = x
                            && term_vars(v, rule, y).is_subset(&known)
                        {
                            known.insert(name(v, rule, x));
                        }
                    }
                }
                L::Nearest { n, a, .. } if known.contains(&name(v, rule, n)) => {
                    known.insert(name(v, rule, a));
                }
                _ => {}
            }
        }
        if known.len() == before {
            return known;
        }
    }
}

/// `file_local::functional`: whether `part` holds at most one row per value of `key`.
fn functional(v: &Version, rule: RuleRef<'_>, part: &[LitRef<'_>], key: &str, head: &[String], keys: &Keys) -> bool {
    let mut vars = BTreeSet::new();
    part.iter().for_each(|l| vars.extend(lit_vars(v, rule, *l)));
    let known = determined(v, rule, part, BTreeSet::from([key.to_owned()]), keys);
    head.iter().filter(|h| vars.contains(h.as_str())).all(|h| known.contains(h.as_str()))
}

/// `file_local::relation_keys` on the version.
fn relation_keys(v: &Version, f: Fields<'_>) -> Keys {
    let mut keys: Keys = BTreeMap::new();
    for rel in f.inputs() {
        let (Some(columns), Some(types)) = (crate::code_rule_dsl::logic::sql::select::input_columns(rel), f.columns(rel)) else { continue };
        if columns.len() != types.len() {
            continue;
        }
        let at = |name: &str| columns.iter().position(|c| *c == name);
        if let ("child" | "field", Some(p), Some(c)) = (rel, at("p"), at("c")) {
            let other = (0..columns.len()).find(|&i| i != p && i != c);
            keys.insert(rel.to_owned(), std::iter::once(vec![c]).chain(other.map(|o| vec![p, o])).collect());
            continue;
        }
        let Some(k) = at("n") else { continue };
        let others_nodes = types.iter().enumerate().any(|(i, (_, t))| i != k && (matches!(*t, "node" | "loc" | "span") || matches!(columns.get(i), Some(&("par" | "a" | "p" | "h")))));
        if !others_nodes {
            keys.insert(rel.to_owned(), vec![vec![k]]);
        }
    }
    for l in f.lattices() {
        keys.insert(v.rel_name(l.rel).to_owned(), vec![(0..l.keys).collect()]);
    }
    let derived: BTreeSet<&str> = v.rules().map(|r| v.rel_name(r.head().rel())).filter(|r| !keys.contains_key(*r)).collect();
    let mut rules_of: BTreeMap<&str, Vec<RuleRef<'_>>> = BTreeMap::new();
    for r in v.rules() {
        let h = v.rel_name(r.head().rel());
        if derived.contains(h) {
            rules_of.entry(h).or_default().push(r);
        }
    }
    let mut step = 0usize;
    let mut gained: HashMap<String, usize> = HashMap::new();
    let mut tried: HashMap<(&str, usize), usize> = HashMap::new();
    for _ in 0..6 {
        let mut found = false;
        for (rel, rules) in &rules_of {
            let width = rules[0].head().args().count();
            for c in 0..width {
                if keys.get(*rel).is_some_and(|ks| ks.contains(&vec![c])) {
                    continue;
                }
                if let Some(&at) = tried.get(&(*rel, c))
                    && !rules.iter().flat_map(|r| r.body()).any(|l| matches!(l.view(), L::Atom(a) if gained.get(v.rel_name(a.rel())).is_some_and(|&g| g > at)))
                {
                    continue;
                }
                tried.insert((*rel, c), step);
                let proven = rules.iter().all(|r| {
                    let mut head = BTreeSet::new();
                    r.head().args().for_each(|t| head.extend(term_vars(v, *r, t)));
                    let start = match r.head().args().nth(c) {
                        Some(TermRef::Var(s)) => BTreeSet::from([name(v, *r, s)]),
                        _ => return false,
                    };
                    let lits: Vec<LitRef<'_>> = r.body().collect();
                    lits.iter().all(|l| matches!(l.view(), L::Atom(_) | L::Cmp { .. } | L::Nearest { .. })) && head.is_subset(&determined(v, *r, &lits, start, &keys))
                });
                if proven {
                    keys.entry((*rel).to_owned()).or_default().push(vec![c]);
                    found = true;
                    step += 1;
                    gained.insert((*rel).to_owned(), step);
                }
            }
        }
        if !found {
            break;
        }
    }
    keys
}

impl ProgramPass for FactorizeExports {
    const NAME: &'static str = "factorize the exports";
    fn settings(&self) -> Vec<u8> {
        vec![u8::from(self.on)]
    }
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        if !self.on {
            return Ok(());
        }
        let v: &std::sync::Arc<Version> = cx.v();
        let d = cx.get::<FieldsData>()?;
        let f = Fields::over(v, &d);
        let split = cx.get::<FileLocal>()?;
        let local = &split.local;
        let head_of = |r: RuleRef<'_>| v.rel_name(r.head().rel());
        let natives_read: BTreeSet<&str> = f.natives().iter().filter(|n| !local.contains(v.rel_name(n.rel))).flat_map(|n| n.reads.iter()).map(|&r| v.rel_name(r)).collect();
        let keys = relation_keys(v, f);
        let rules: Vec<RuleRef<'_>> = v.rules().collect();
        let mut reads: BTreeMap<&str, Vec<(usize, usize)>> = BTreeMap::new();
        let mut unfit: BTreeSet<&str> = BTreeSet::new();
        for (ri, r) in rules.iter().enumerate() {
            for (li, l) in r.body().enumerate() {
                match l.view() {
                    L::Atom(a) => {
                        let rel = v.rel_name(a.rel());
                        if a.neg() || a.gated() || local.contains(head_of(*r)) || a.args().any(|t| matches!(t, TermRef::Call(..) | TermRef::Top)) {
                            unfit.insert(rel);
                        } else {
                            reads.entry(rel).or_default().push((ri, li));
                        }
                    }
                    L::Agg { atom, .. } => {
                        unfit.insert(v.rel_name(atom.rel()));
                    }
                    _ => {}
                }
            }
        }
        let mut rules_of: BTreeMap<&str, Vec<RuleRef<'_>>> = BTreeMap::new();
        for r in &rules {
            rules_of.entry(head_of(*r)).or_default().push(*r);
        }
        // A plan per relation: per rule, its key and parts (literal places).
        let mut chosen: BTreeMap<String, Vec<(String, Vec<Vec<usize>>)>> = BTreeMap::new();
        for (rel, read) in &reads {
            let rel = *rel;
            let engine = rel.starts_with("magic_") || rel.starts_with("done_") || rel.contains("__");
            if !local.contains(rel)
                || unfit.contains(rel)
                || engine
                || f.is_output(rel)
                || f.is_input(rel)
                || f.lattice(rel)
                || f.native(rel).is_some()
                || natives_read.contains(rel)
                || f.has_facts(rel)
                || f.gate(rel)
                || f.answer(rel)
            {
                continue;
            }
            let Some(of) = rules_of.get(rel) else { continue };
            if of.len() > super::super::FACTOR_RULES || read.len() * of.len() > super::super::FACTOR_COPIES {
                continue;
            }
            let mut plan = Vec::new();
            for rule in of {
                let head: Option<Vec<String>> = rule.head().args().map(|t| t.var().map(|s| name(v, *rule, s))).collect();
                let Some(head) = head.filter(|h| h.iter().collect::<BTreeSet<_>>().len() == h.len()) else { break };
                let Some((key, parts)) = crate::code_rule_dsl::logic::plan::keeps::factors_of(v, *rule) else { break };
                let lits: Vec<LitRef<'_>> = rule.body().collect();
                let part_lits = |p: &Vec<usize>| p.iter().map(|&i| lits[i]).collect::<Vec<_>>();
                let keyed = |p: &Vec<usize>| part_lits(p).iter().any(|l| matches!(l.view(), L::Atom(a) if !a.neg() && body::atom_vars(a).iter().any(|s| name(v, *rule, s) == key)));
                if !parts.iter().all(keyed) {
                    break;
                }
                let fans = parts.iter().filter(|p| !functional(v, *rule, &part_lits(p), &key, &head, &keys)).count();
                if fans < 2 {
                    break;
                }
                // Decision 0677: no node crosses into the cross-file part. Its readers (an export's, whose guard counts
                // the whole factors) join the parts in the cross part, so a key that is a node carries it there. A
                // relation that holds no node is handed on whole, its rows symbols; one that does holds a node already,
                // a crossing the check names, and is factorized as before until its family is keyed by symbols.
                let node_at = |rel: &str, c: usize| f.columns(rel).and_then(|cols| cols.get(c).map(|(_, t)| super::super::node_crossings::NODE_TYPES.contains(t))).unwrap_or(false);
                let key_is_node = lits.iter().any(|l| match l.view() {
                    L::Atom(a) if !a.neg() => a.args().enumerate().any(|(c, t)| matches!(t, TermRef::Var(s) if name(v, *rule, s) == key) && node_at(v.rel_name(a.rel()), c)),
                    _ => false,
                });
                let head_holds_node = (0..rule.head().args().count()).any(|c| node_at(rel, c));
                if key_is_node && !head_holds_node {
                    break;
                }
                plan.push((key, parts));
            }
            if plan.len() != of.len() {
                continue;
            }
            let binds = read.iter().all(|&(ri, li)| {
                let reader = rules[ri];
                let lits: Vec<LitRef<'_>> = reader.body().collect();
                let L::Atom(a) = lits[li].view() else { return false };
                a.args().any(|t| match t {
                    TermRef::Int(_) | TermRef::Branded(_) | TermRef::Str(_) => true,
                    TermRef::Var(s) => lits.iter().enumerate().any(|(j, l)| j != li && matches!(l.view(), L::Atom(b) if !b.neg() && body::atom_vars(b).contains(s))),
                    _ => false,
                })
            });
            if binds {
                chosen.insert(rel.to_owned(), plan);
            }
        }
        let decl_names: HashSet<String> = f.decls().iter().map(|d| v.text(d.name()).to_owned()).collect();
        loop {
            if chosen.is_empty() {
                return Ok(());
            }
            // Each part named from its content (`helper_name`), as the tree names it.
            let mut used = HashMap::new();
            let mut decls: Vec<(String, Vec<(String, String)>)> = Vec::new();
            let mut declare = |rel: String, cols: Vec<(String, String)>| {
                decls.retain(|(r, _)| *r != rel);
                decls.push((rel, cols));
            };
            let mut no_inline: Vec<String> = Vec::new();
            let mut lines = Vec::new();
            let mut made: BTreeMap<String, Vec<String>> = BTreeMap::new();
            let mut program: Vec<Made<'_>> = rules.iter().map(|&r| Made { src: r, head: None, body: r.body().map(BodyItem::Orig).collect(), kept: true }).collect();
            // Per relation, per rule: the head's variables and the parts' atoms (relation, interface).
            let mut stands: BTreeMap<String, Vec<(Vec<String>, Vec<(String, Vec<String>)>)>> = BTreeMap::new();
            for (rel, plan) in &chosen {
                let of = &rules_of[rel.as_str()];
                let mut shown = Vec::new();
                let mut per_rule = Vec::new();
                let mut ok = true;
                for (rule, (key, parts)) in of.iter().zip(plan) {
                    let rule = *rule;
                    let lits: Vec<LitRef<'_>> = rule.body().collect();
                    let head: Vec<String> = rule.head().args().filter_map(|t| t.var()).map(|s| name(v, rule, s)).collect();
                    let typed = |part: &[LitRef<'_>], x: &str| {
                        part.iter().find_map(|l| match l.view() {
                            L::Atom(a) if !a.neg() => {
                                let c = a.args().position(|t| matches!(t, TermRef::Var(s) if name(v, rule, s) == x))?;
                                f.column_type(v.rel_name(a.rel()), c).map(|t| (x.to_owned(), t.to_owned()))
                            }
                            _ => None,
                        })
                    };
                    let mut atoms: Vec<(String, Vec<String>)> = Vec::new();
                    let mut knames: Vec<String> = Vec::new();
                    let mut bodies: Vec<Vec<LitRef<'_>>> = Vec::new();
                    for part in parts {
                        let part: Vec<LitRef<'_>> = part.iter().map(|&i| lits[i]).collect();
                        let mut vars = BTreeSet::new();
                        part.iter().for_each(|l| vars.extend(lit_vars(v, rule, *l)));
                        let mut iface: Vec<String> = head.iter().filter(|x| vars.contains(x.as_str())).cloned().collect();
                        iface.push(key.clone());
                        let Some(cols) = iface.iter().map(|x| typed(&part, x)).collect::<Option<Vec<_>>>() else {
                            ok = false;
                            break;
                        };
                        let content = format!("{rel}({}) :- {}", iface.join(", "), part.iter().map(|l| v.show_lit(rule, *l)).collect::<Vec<_>>().join(", "));
                        let pname = super::super::helper_name("fz", rel, &content, |n| decl_names.contains(n), &mut used);
                        declare(pname.clone(), cols.clone());
                        let kname = format!("fzk{}", &pname["fz".len()..]);
                        declare(kname.clone(), vec![cols.last().cloned().expect("the key")]);
                        if !no_inline.contains(&kname) {
                            no_inline.push(kname.clone());
                        }
                        atoms.push((pname, iface));
                        knames.push(kname);
                        bodies.push(part);
                    }
                    if !ok {
                        break;
                    }
                    let var = |x: &String| NOwned::V(x.clone());
                    for (i, ((pname, iface), body)) in atoms.iter().zip(&bodies).enumerate() {
                        let orig: Vec<BodyItem<'_>> = body.iter().map(|l| BodyItem::Orig(*l)).collect();
                        program.push(Made { src: rule, head: Some((knames[i].clone(), vec![var(key)])), body: orig.clone(), kept: false });
                        let mut reduced = orig;
                        reduced.extend(knames.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, k)| BodyItem::New(k.clone(), vec![var(key)])));
                        program.push(Made { src: rule, head: Some((pname.clone(), iface.iter().map(var).collect())), body: reduced, kept: false });
                        made.entry(rel.clone()).or_default().extend([pname.clone(), knames[i].clone()]);
                    }
                    shown.push(format!(
                        "by ${key}: {}",
                        atoms
                            .iter()
                            .zip(&bodies)
                            .map(|((p, iface), b)| format!(
                                "{p}({}) :- {}",
                                iface.iter().map(|x| format!("${x}")).collect::<Vec<_>>().join(", "),
                                b.iter().map(|l| v.show_lit(rule, *l)).collect::<Vec<_>>().join(", ")
                            ))
                            .collect::<Vec<_>>()
                            .join(" | ")
                    ));
                    per_rule.push((head, atoms));
                }
                if !ok {
                    continue;
                }
                lines.push(format!("{rel}: handed on as parts, each reduced by the others' keys, read in {} cross-file rules; {}", reads[rel.as_str()].len(), shown.join("; ")));
                stands.insert(rel.clone(), per_rule);
            }
            // The relation's rules go; each read becomes the parts of each of its rules, the key and the variables
            // the read leaves open named apart (the open ones by the copy's depth, as the tree names them).
            let mut out: Vec<Made<'_>> = Vec::with_capacity(program.len());
            for m in program {
                let head = match &m.head {
                    None => head_of(m.src),
                    Some((rel, _)) => rel.as_str(),
                };
                if stands.contains_key(head) {
                    continue;
                }
                let mut work = vec![(m, 0usize)];
                while let Some((m, depth)) = work.pop() {
                    let read_at = m.body.iter().position(|it| match it {
                        BodyItem::Orig(l) => matches!(l.view(), L::Atom(a) if !a.neg() && stands.contains_key(v.rel_name(a.rel()))),
                        BodyItem::New(rel, _) => stands.contains_key(rel),
                    });
                    let Some(at) = read_at else {
                        out.push(m);
                        continue;
                    };
                    let (rel, args): (String, Vec<NOwned<'_>>) = match &m.body[at] {
                        BodyItem::Orig(l) => {
                            let L::Atom(a) = l.view() else { unreachable!() };
                            (v.rel_name(a.rel()).to_owned(), a.args().map(NOwned::T).collect())
                        }
                        BodyItem::New(rel, args) => (rel.clone(), args.clone()),
                    };
                    for (head, atoms) in &stands[rel.as_str()] {
                        let to = |x: &str| -> NOwned<'_> {
                            match head.iter().position(|h| h == x) {
                                Some(i) => args[i].clone(),
                                None => NOwned::V(format!("fz{}_{x}", depth + 1)),
                            }
                        };
                        let parts = atoms.iter().map(|(p, iface)| BodyItem::New(p.clone(), iface.iter().map(|x| to(x)).collect()));
                        let mut copy = m.clone();
                        copy.kept = false;
                        copy.body.splice(at..=at, parts);
                        work.push((copy, depth + 1));
                    }
                }
            }
            let round = Round { decls, undeclared: stands.keys().cloned().collect(), rules: out, no_inline };
            let (_, now) = super::candidate(v, &round)?;
            let failed: Vec<String> = made.iter().filter(|(_, parts)| parts.iter().any(|p| !now.local.contains(p))).map(|(r, _)| r.clone()).collect();
            if failed.is_empty() {
                self.lines = lines;
                write(w.derive()?, v, &round)?;
                return Ok(());
            }
            for r in failed {
                chosen.remove(&r);
            }
        }
    }
}
