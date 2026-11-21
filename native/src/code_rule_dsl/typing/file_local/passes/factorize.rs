//! The factorized store across the chunk boundary (decision 0099; 0281's path, step 5). An export (a file-local
//! relation only the cross-file part reads) whose every rule joins parts that meet only at a variable its head
//! projects away is handed on as the parts, not as their product: `fl853_p4_outer(f, x) :- rr_inside_ks5(f, p),
//! bound_in(p, x)` held 295 k rows over the 486 fixture files, and is handed on as `fz_a(f, p)` and `fz_b(p, x)`,
//! each about the size of its own part. The cross-file readers read the parts where they read the product
//! (`p4_outer(f, x) :- p4_read(_, x, f, _), fz_a(f, p), fz_b(p, x)`): their join is the cross part's ordinary join,
//! per key, from the columns they bind.
//!
//! - **Each part** keeps its own comparisons and negations (`keeps::factors_of`), and is reduced by the keys every part
//!   holds (`fz_a(f, p) :- …, fzk_b(p)`): every row stored then joins, so a part never holds more rows than the
//!   product it replaces. The keys are chained, the parts of fewest atoms first: `fzk_a(p) :- …`, then
//!   `fzk_b(p) :- bound_in(p, x), fzk_a(p)`, so a part that is itself a join is made only for the keys the parts
//!   before it hold, and the last key is every part's.
//! - **When:** decided from the program alone, before the first chunk: the per-file memo's key is a digest of the
//!   file-local part (`corpus::memo::program_digest`), which names the exports, so the choice must not depend on a
//!   chunk's rows. A changed choice (another engine, `RERAIL_FACTORIZE=off`) is another key, a memo miss.
//!   The relation is factorized when at least two of its parts may hold many rows per key (a part that is a node's
//!   attributes, `parent(n, p), text(p, t)`, holds one, and the product is no larger than the other part); when every
//!   read of it is positive, in a cross-file rule that binds a column of it (so the reader's join starts from a
//!   bound part, not the whole product again); and when the readers' copies stay few (one per rule of the relation,
//!   at most `FACTOR_COPIES`).
//!
//! A relation whose parts the split does not find file-local is left whole. The split is read from the version
//! (`FileLocal`); variables are read by name, so each choice, counter and name is fixed by the program; each round's
//! program is a version of its own, split again to check the parts stay file-local.

use super::super::FileLocal;
use super::{BodyItem, Counts, Made, NOwned, Round, lit_vars, name, term_vars, write};
use crate::code_rule_dsl::logic::plan::body;
use crate::code_rule_dsl::logic::plan::fields::{Fields, FieldsData};
use crate::code_rule_dsl::logic::program_image::Builder;
use crate::code_rule_dsl::logic::program_image::manager::{self, Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{CmpOp, Lit as L, LitRef, RuleRef, TermRef, Version};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Each export whose rules join parts meeting at a key handed on as the parts (the module comment): a line per
/// relation factorized. Off (`on` false) where the run's options turn the factorized store off (`factorize_on`).
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

/// The variables `lits` of `rule` determine from `known`: an atom whose relation has a key all of whose columns are
/// known (or constants) determines its other variables; so does an equality, and a nearest ancestor its node.
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

/// Whether `part` holds at most one row per value of `key`: its head variables (`head`) follow from the key through
/// atoms read by a key of their relation (`keys`, `relation_keys`), equalities and nearest ancestors.
fn functional(v: &Version, rule: RuleRef<'_>, part: &[LitRef<'_>], key: &str, head: &[String], keys: &Keys) -> bool {
    let mut vars = BTreeSet::new();
    part.iter().for_each(|l| vars.extend(lit_vars(v, rule, *l)));
    let known = determined(v, rule, part, BTreeSet::from([key.to_owned()]), keys);
    head.iter().filter(|h| vars.contains(h.as_str())).all(|h| known.contains(h.as_str()))
}

/// Keys of the program's relations, as far as the program shows them: a node's attributes by the node (`docof`,
/// `text`, `node` by its node, and a view of one); the tree's edges by the child, and by the parent with the field or
/// the index (`child(p, c, i)`, `field(p, f, c)`); a lattice by its key columns; and a derived relation by a column
/// that, in every rule, determines the head (`determined`), found to a fixpoint. They price the factorized store's
/// test only (a part of one row a key holds no more rows than the product): a wrong key costs rows, never findings.
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

/// Per relation factorized by `factorize_intermediates`: its pair cache, where it was given one (the guard, the
/// pairs' relation, the pairs it holds at most).
pub(crate) type FactorCaches = BTreeMap<String, Option<(String, String, usize)>>;

/// What `Scan::round` made: the round, a line per relation, the relations each relation's parts are, and the caches.
struct Built<'v> {
    round: Round<'v>,
    lines: Vec<String>,
    made: BTreeMap<String, Vec<String>>,
    caches: FactorCaches,
}

/// The program as the factorizing reads it: its rules, each relation's reads and rules, and the plans chosen.
struct Scan<'v> {
    v: &'v Version,
    f: Fields<'v>,
    rules: Vec<RuleRef<'v>>,
    reads: BTreeMap<&'v str, Vec<(usize, usize)>>,
    rules_of: BTreeMap<&'v str, Vec<RuleRef<'v>>>,
    /// A plan per relation: per rule, its key and parts (literal places).
    chosen: BTreeMap<String, Vec<(String, Vec<Vec<usize>>)>>,
    /// The intermediates asked for (`factorize_intermediates`), or none: the exports, `local` the file-local part.
    intermediates: bool,
}

impl<'v> Scan<'v> {
    /// The plans for `v`'s relations: its exports (`local`, the file-local relations), or the `intermediates`.
    fn of(v: &'v Version, f: Fields<'v>, local: &BTreeSet<String>, intermediates: Option<&BTreeSet<String>>) -> Scan<'v> {
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
        let mut chosen: BTreeMap<String, Vec<(String, Vec<Vec<usize>>)>> = BTreeMap::new();
        for (rel, read) in &reads {
            let rel = *rel;
            let engine = rel.starts_with("magic_") || rel.starts_with("done_") || rel.contains("__");
            if !intermediates.map_or_else(|| local.contains(rel), |names| names.contains(rel))
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
                plan.push((key, parts));
            }
            if plan.len() != of.len() {
                continue;
            }
            // An export's every read binds a column of it; an intermediate's readers are the deferred joins pass's
            // to check (`product_demand::deferred`).
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
            if binds || intermediates.is_some() {
                chosen.insert(rel.to_owned(), plan);
            }
        }
        Scan { v, f, rules, reads, rules_of, chosen, intermediates: intermediates.is_some() }
    }

    /// The round of the plans chosen. With `cache_reused` (intermediates only), a relation of one rule of two parts
    /// that several rules each read once, as their only relation factorized, is given a pair cache: its pairs made
    /// eagerly while its factors' rows multiply to at most its share of `RERAIL_JOIN_CACHE_ROWS` (4,096; 0 for none),
    /// read where its readers read it, and its factors read in the readers' copies otherwise.
    fn round(&self, cache_reused: bool) -> Result<Built<'v>, String> {
        let (v, f, rules, reads, rules_of, chosen) = (self.v, self.f, &self.rules, &self.reads, &self.rules_of, &self.chosen);
        let head_of = |r: RuleRef<'_>| v.rel_name(r.head().rel());
        let decl_names: HashSet<String> = f.decls().iter().map(|d| v.text(d.name()).to_owned()).collect();
        // Each part named from its content (`helper_name`).
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
        // A reader is split at most once: a cached relation's every reader reads it as its only relation factorized.
        let cacheable: BTreeSet<&String> = chosen
            .keys()
            .filter(|rel| {
                cache_reused
                    && chosen[*rel].len() == 1
                    && chosen[*rel][0].1.len() == 2
                    && reads[rel.as_str()].len() > 1
                    && reads[rel.as_str()].iter().all(|&(ri, _)| rules[ri].body().filter(|l| matches!(l.view(), L::Atom(a) if chosen.contains_key(v.rel_name(a.rel())))).count() == 1)
            })
            .collect();
        // Across the caches, the pairs joined are at most the allowance: both factors are sets, so even one key all
        // rows share joins at most |left| × |right|.
        let allowance = std::env::var_os("RERAIL_JOIN_CACHE_ROWS").map_or(4096, |s| s.to_str().and_then(|s| s.parse::<usize>().ok()).unwrap_or(0)).min(i64::MAX as usize);
        let pair_limit = allowance / cacheable.len().max(1);
        // Per relation cached: its pairs' relation and its guard.
        let mut caches: BTreeMap<String, (String, String)> = BTreeMap::new();
        let mut counts = Vec::new();
        let mut nullary = Vec::new();
        for (rel, plan) in chosen {
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
                // The parts in the order their keys are chained (below): the fewest positive atoms first, so a
                // part that is itself a join (`mf_def($c, $f), mf_def($a, $f)`) has its keys found within the
                // keys of the parts before it, never on its own.
                let positive = |part: &Vec<usize>| part.iter().filter(|&&i| matches!(lits[i].view(), L::Atom(a) if !a.neg())).count();
                let mut parts: Vec<&Vec<usize>> = parts.iter().collect();
                parts.sort_by_key(|p| positive(p));
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
                    // An intermediate's part is named apart from the keys' names too: both are made here.
                    let pname = super::super::helper_name("fz", rel, &content, |n| decl_names.contains(n) || (self.intermediates && decl_names.contains(&format!("fzk{}", &n["fz".len()..]))), &mut used);
                    declare(pname.clone(), cols.clone());
                    if self.intermediates && !no_inline.contains(&pname) {
                        no_inline.push(pname.clone());
                    }
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
                if pair_limit > 0 && cacheable.contains(rel) {
                    // The pair cache (decision 0488): `small(1)` while the factors' rows multiply to at most the
                    // limit (`rr_admit_pairs`, the machine's pricer's choice past that), and the pairs made then.
                    let base = super::super::helper_name("rr_pair_cache_", rel, &v.show_rule(rule), |n| ["small", "pairs"].iter().any(|s| decl_names.contains(&format!("{n}_{s}"))), &mut HashMap::new());
                    let (small, pairs) = (format!("{base}_small"), format!("{base}_pairs"));
                    declare(small.clone(), vec![("rr_unit".to_owned(), "int".to_owned())]);
                    declare(pairs.clone(), f.columns(rel).unwrap_or_default().into_iter().map(|(c, t)| (c.to_owned(), t.to_owned())).collect());
                    nullary.push(small.clone());
                    no_inline.extend([small.clone(), pairs.clone()]);
                    counts.push(Counts {
                        head: small.clone(),
                        counted: atoms.iter().map(|(p, iface)| (p.clone(), iface.len())).collect(),
                        test: "rr_admit_pairs",
                        args: vec![reads[rel.as_str()].len() as i64, pair_limit as i64],
                    });
                    let mut body = vec![BodyItem::New(small.clone(), vec![NOwned::Int(1)])];
                    body.extend(atoms.iter().map(|(p, iface)| BodyItem::New(p.clone(), iface.iter().map(var).collect())));
                    program.push(Made { src: rule, head: Some((pairs.clone(), head.iter().map(var).collect())), body, kept: false });
                    caches.insert(rel.clone(), (pairs, small));
                    shown.push(format!("cache if factor row counts multiply to at most {pair_limit}"));
                }
                for (i, ((pname, iface), body)) in atoms.iter().zip(&bodies).enumerate() {
                    let orig: Vec<BodyItem<'_>> = body.iter().map(|l| BodyItem::Orig(*l)).collect();
                    // The keys chained: a part's keys are those it holds among the keys of the parts before it,
                    // so the last is the keys every part holds. Each made apart from its own part alone, a part
                    // that is a join was made whole for its keys: every pair of nodes in a def, 4 s of one
                    // stratum over 91 files, where the product it replaced held a few rows.
                    let mut keyed = orig.clone();
                    if i > 0 {
                        keyed.push(BodyItem::New(knames[i - 1].clone(), vec![var(key)]));
                    }
                    program.push(Made { src: rule, head: Some((knames[i].clone(), vec![var(key)])), body: keyed, kept: false });
                    // Reduced by the keys every part holds: the others' keys, as before, and its own.
                    let mut reduced = orig;
                    reduced.push(BodyItem::New(knames.last().expect("a part").clone(), vec![var(key)]));
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
            let scope = if self.intermediates { "reader" } else { "cross-file" };
            lines.push(format!("{rel}: handed on as parts, each reduced by the others' keys, read in {} {scope} rules; {}", reads[rel.as_str()].len(), shown.join("; ")));
            stands.insert(rel.clone(), per_rule);
        }
        // The relation's rules go; each read becomes the parts of each of its rules, the key and the variables
        // the read leaves open named apart (the open ones by the copy's depth). A cached relation's read is also
        // copied reading its pairs, and its parts' copies are then read while the guard does not hold.
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
                    BodyItem::Not(..) => false,
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
                    BodyItem::Not(..) => unreachable!("a positive read"),
                };
                let mut cold = None;
                if let Some((pairs, small)) = caches.get(&rel) {
                    let mut eager = m.clone();
                    eager.kept = false;
                    eager.body[at] = BodyItem::New(pairs.clone(), args.clone());
                    out.push(eager);
                    cold = Some(BodyItem::Not(small.clone(), vec![NOwned::Int(1)]));
                }
                // The reader's own variables, which an intermediate's open variables are named apart from.
                let occupied: BTreeSet<String> = if self.intermediates {
                    let mut vars = BTreeSet::new();
                    m.src.head().args().for_each(|t| vars.extend(term_vars(v, m.src, t)));
                    m.src.body().for_each(|l| vars.extend(lit_vars(v, m.src, l)));
                    vars
                } else {
                    BTreeSet::new()
                };
                for (head, atoms) in &stands[rel.as_str()] {
                    let mut witness = depth + 1;
                    while atoms.iter().flat_map(|(_, iface)| iface).filter(|x| !head.contains(x)).any(|x| occupied.contains(&format!("fz{witness}_{x}"))) {
                        witness += 1;
                    }
                    let to = |x: &str| -> NOwned<'_> {
                        match head.iter().position(|h| h == x) {
                            Some(i) => args[i].clone(),
                            None => NOwned::V(format!("fz{witness}_{x}")),
                        }
                    };
                    let parts = atoms.iter().map(|(p, iface)| BodyItem::New(p.clone(), iface.iter().map(|x| to(x)).collect()));
                    let mut copy = m.clone();
                    copy.kept = false;
                    copy.body.splice(at..=at, parts);
                    if let Some(gate) = &cold {
                        copy.body.push(gate.clone());
                    }
                    work.push((copy, depth + 1));
                }
            }
        }
        let undeclared: Vec<String> = stands.keys().cloned().collect();
        let mut round = Round::of(decls, undeclared.clone(), out, no_inline);
        round.nullary = nullary;
        round.counts = counts;
        if self.intermediates {
            round.unqualified = undeclared;
        }
        let caches = stands.into_keys().map(|rel| {
            let cache = caches.remove(&rel).map(|(pairs, small)| (small, pairs, pair_limit));
            (rel, cache)
        }).collect();
        Ok(Built { round, lines, made, caches })
    }
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
        let mut scan = Scan::of(v, f, &split.local, None);
        loop {
            if scan.chosen.is_empty() {
                return Ok(());
            }
            let Built { round, lines, made, .. } = scan.round(false)?;
            let (_, now) = super::candidate(v, &round)?;
            let failed: Vec<String> = made.iter().filter(|(_, parts)| parts.iter().any(|p| !now.local.contains(p))).map(|(r, _)| r.clone()).collect();
            if failed.is_empty() {
                self.lines = lines;
                write(w.derive()?, v, &round)?;
                return Ok(());
            }
            for r in failed {
                scan.chosen.remove(&r);
            }
        }
    }
}

/// The deferred joins pass's reduced factors (`product_demand::deferred`, decision 0488): each of `candidates` (a
/// relation of one definition, not recursive, its every read the caller's to check) whose rules join parts meeting at
/// a key its head projects away, held as its parts, each reduced by the others' keys, within a document or across
/// them, read where its readers read it (`Scan::round`, its pair caches with `cache_reused`). The version made (none
/// where nothing was), a line per relation, and each relation's cache.
pub(crate) fn factorize_intermediates(v: &std::sync::Arc<Version>, candidates: &BTreeSet<String>, cache_reused: bool) -> Result<(Option<std::sync::Arc<Version>>, Vec<String>, FactorCaches), String> {
    if candidates.is_empty() {
        return Ok((None, Vec::new(), FactorCaches::new()));
    }
    let d = manager::analysis::<FieldsData>(v)?;
    let f = Fields::over(v, &d);
    let scan = Scan::of(v, f, &BTreeSet::new(), Some(candidates));
    if scan.chosen.is_empty() {
        return Ok((None, Vec::new(), FactorCaches::new()));
    }
    let Built { round, lines, caches, .. } = scan.round(cache_reused)?;
    if caches.is_empty() {
        return Ok((None, Vec::new(), FactorCaches::new()));
    }
    let mut b = Builder::derive(v);
    write(&mut b, v, &round)?;
    Ok((Some(b.finish()?.0), lines, caches))
}
