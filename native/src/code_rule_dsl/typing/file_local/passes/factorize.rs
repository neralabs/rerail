//! The factorized store across the chunk boundary (decision 0099; 0281's path, step 5). An export (a file-local
//! relation only the cross-file part reads) whose every rule joins parts that meet only at a variable its head
//! projects away is handed on as the parts, not as their product: `fl853_p4_outer(f, x) :- rr_inside_ks5(f, p),
//! bound_in(p, x)` held 295 k rows over the 486 fixture files, and is handed on as `fz_a(f, p)` and `fz_b(p, x)`,
//! each about the size of its own part. The cross-file readers read the parts where they read the product
//! (`p4_outer(f, x) :- p4_read(_, x, f, _), fz_a(f, p), fz_b(p, x)`): their join is the cross part's ordinary join,
//! per key, from the columns they bind.
//!
//! - **Each part** keeps its own comparisons and negations (`keeps::factors_of`), and is reduced by the others' keys
//!   (`fz_a(f, p) :- …, fzk_b(p)`, `fzk_b(p) :- bound_in(p, x)`): every row stored then joins, so a part never holds
//!   more rows than the product it replaces.
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
use super::{BodyItem, Made, NOwned, Round, lit_vars, name, term_vars, write};
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

/// Which relations a factorization may hand on as parts: the exports of the chunk split (`local`, its file-local
/// relations), or the intermediates a deferred joins pass found (decision 0477: `names`, single-definition and
/// nonrecursive, every read of them checked by the caller), whose products may also be kept as a pair cache
/// (`cache_reused`) while their factors' sizes allow.
#[derive(Clone, Copy)]
pub(crate) enum Scope<'a> {
    Exports(&'a BTreeSet<String>),
    Intermediates { names: &'a BTreeSet<String>, cache_reused: bool },
}

impl Scope<'_> {
    fn intermediates(self) -> bool {
        matches!(self, Scope::Intermediates { .. })
    }
    /// Whether `rel` is one the factorization may hand on.
    fn takes(self, rel: &str) -> bool {
        match self {
            Scope::Exports(local) => local.contains(rel),
            Scope::Intermediates { names, .. } => names.contains(rel),
        }
    }
    /// Whether a rule deriving `head` reads in the chunk (an export's reads are the cross-file part's alone).
    fn in_chunk(self, head: &str) -> bool {
        match self {
            Scope::Exports(local) => local.contains(head),
            Scope::Intermediates { .. } => false,
        }
    }
}

/// The program's reads and rules, and the relations chosen to be handed on as parts: per relation, per rule, its
/// key and parts (literal places).
struct Survey<'v> {
    rules: Vec<RuleRef<'v>>,
    reads: BTreeMap<&'v str, Vec<(usize, usize)>>,
    rules_of: BTreeMap<&'v str, Vec<RuleRef<'v>>>,
    chosen: BTreeMap<String, Vec<(String, Vec<Vec<usize>>)>>,
}

fn survey<'v>(v: &'v Version, f: Fields<'v>, scope: Scope<'_>) -> Survey<'v> {
    let head_of = |r: RuleRef<'v>| v.rel_name(r.head().rel());
    let natives_read: BTreeSet<&str> = f.natives().iter().filter(|n| !scope.in_chunk(v.rel_name(n.rel))).flat_map(|n| n.reads.iter()).map(|&r| v.rel_name(r)).collect();
    let keys = relation_keys(v, f);
    let rules: Vec<RuleRef<'v>> = v.rules().collect();
    let mut reads: BTreeMap<&str, Vec<(usize, usize)>> = BTreeMap::new();
    let mut unfit: BTreeSet<&str> = BTreeSet::new();
    for (ri, r) in rules.iter().enumerate() {
        for (li, l) in r.body().enumerate() {
            match l.view() {
                L::Atom(a) => {
                    let rel = v.rel_name(a.rel());
                    if a.neg() || a.gated() || scope.in_chunk(head_of(*r)) || a.args().any(|t| matches!(t, TermRef::Call(..) | TermRef::Top)) {
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
        if !scope.takes(rel)
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
        // An export's every read binds a column of it, so the reader's join starts from a bound part; an
        // intermediate's readers were checked by the deferred joins pass.
        let binds = scope.intermediates()
            || read.iter().all(|&(ri, li)| {
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
    Survey { rules, reads, rules_of, chosen }
}

/// A product kept whole beside its parts while their sizes allow (decision 0477, `rr_admit_pairs`): its guard
/// `small` (nullary, true while the parts' row counts multiply to at most `limit` over its `readers`), its rows
/// `pairs`, and the parts it joins (relation, interface) under the head's variables `head`.
struct Cache {
    small: String,
    pairs: String,
    rel: String,
    head: Vec<String>,
    factors: Vec<(String, Vec<String>)>,
    readers: usize,
    limit: usize,
}

/// One round of a factorization: its program, a line per relation handed on, each relation's parts, and its caches.
struct Planned<'v> {
    round: Round<'v>,
    lines: Vec<String>,
    made: BTreeMap<String, Vec<String>>,
    caches: BTreeMap<String, Cache>,
}

/// The cross-file pair cache's allowance of joined witnesses over every cached product (`RERAIL_JOIN_CACHE_ROWS`,
/// 4096 by default; 0 turns the caches off).
fn cache_allowance() -> usize {
    crate::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_JOIN_CACHE_ROWS").map_or(4096, |s| s.to_str().and_then(|s| s.parse::<usize>().ok()).unwrap_or(0)).min(i64::MAX as usize))
}

/// The round that hands `s.chosen` on as parts: each part named from its content (`helper_name`), reduced by the
/// others' keys, and each read of a relation made a read of each of its rules' parts.
fn round_of<'v>(v: &'v Version, f: Fields<'v>, s: &Survey<'v>, scope: Scope<'_>) -> Planned<'v> {
    let head_of = |r: RuleRef<'v>| v.rel_name(r.head().rel());
    let (rules, reads, rules_of, chosen) = (&s.rules, &s.reads, &s.rules_of, &s.chosen);
    let decl_names: HashSet<String> = f.decls().iter().map(|d| v.text(d.name()).to_owned()).collect();
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
    // A reused product of two parts, each of whose readers reads no other relation handed on, may keep its pairs
    // too (an intermediate's, `cache_reused`): split a reader at most once.
    let cacheable: BTreeSet<&String> = match scope {
        Scope::Intermediates { cache_reused: true, .. } => chosen
            .iter()
            .filter(|(rel, plan)| {
                plan.len() == 1
                    && plan[0].1.len() == 2
                    && reads[rel.as_str()].len() > 1
                    && reads[rel.as_str()].iter().all(|&(ri, _)| rules[ri].body().filter(|l| matches!(l.view(), L::Atom(a) if chosen.contains_key(v.rel_name(a.rel())))).count() == 1)
            })
            .map(|(rel, _)| rel)
            .collect(),
        _ => BTreeSet::new(),
    };
    // Across the caches the joined witnesses are at most the allowance: both parts are sets, so even one shared key
    // holds at most |left| x |right|.
    let pair_limit = if cacheable.is_empty() { 0 } else { cache_allowance() / cacheable.len() };
    let mut caches: BTreeMap<String, Cache> = BTreeMap::new();
    // Per relation, per rule: the head's variables and the parts' atoms (relation, interface).
    let mut stands: BTreeMap<String, Vec<(Vec<String>, Vec<(String, Vec<String>)>)>> = BTreeMap::new();
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
                // An intermediate's part is named apart from every key relation too (the program may hold another
                // pass's `fzk`).
                let taken = |n: &str| decl_names.contains(n) || (scope.intermediates() && decl_names.contains(&format!("fzk{}", &n["fz".len()..])));
                let pname = super::super::helper_name("fz", rel, &content, taken, &mut used);
                declare(pname.clone(), cols.clone());
                let kname = format!("fzk{}", &pname["fz".len()..]);
                declare(kname.clone(), vec![cols.last().cloned().expect("the key")]);
                for n in [&kname, &pname] {
                    if (n == &kname || scope.intermediates()) && !no_inline.contains(n) {
                        no_inline.push(n.clone());
                    }
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
            if pair_limit > 0 && cacheable.contains(rel) {
                let taken = |n: &str| ["small", "pairs"].iter().any(|x| decl_names.contains(&format!("{n}_{x}")));
                let base = super::super::helper_name("rr_pair_cache_", rel, &v.show_rule(rule), taken, &mut Default::default());
                let (small, pairs) = (format!("{base}_small"), format!("{base}_pairs"));
                declare(small.clone(), vec![("rr_unit".to_owned(), "int".to_owned())]);
                declare(pairs.clone(), f.columns(rel).unwrap_or_default().iter().map(|(c, t)| ((*c).to_owned(), (*t).to_owned())).collect());
                for n in [&small, &pairs] {
                    no_inline.push(n.clone());
                }
                caches.insert(rel.clone(), Cache { small, pairs, rel: rel.clone(), head: head.clone(), factors: atoms.clone(), readers: reads[rel.as_str()].len(), limit: pair_limit });
                shown.push(format!("cache if factor row counts multiply to at most {pair_limit}"));
            }
            per_rule.push((head, atoms));
        }
        if !ok {
            caches.remove(rel);
            continue;
        }
        let where_read = if scope.intermediates() { "reader" } else { "cross-file" };
        lines.push(format!("{rel}: handed on as parts, each reduced by the others' keys, read in {} {where_read} rules; {}", reads[rel.as_str()].len(), shown.join("; ")));
        stands.insert(rel.clone(), per_rule);
    }
    // The relation's rules go; each read becomes the parts of each of its rules, the key and the variables the read
    // leaves open named apart (the open ones by the copy's depth). A cached product's reader reads its pairs too,
    // and its parts' copies only while the pairs are not kept (`!small(1)`).
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
                BodyItem::Not(..) => unreachable!("a read"),
            };
            let cold = caches.get(&rel).map(|c| {
                let mut eager = m.clone();
                eager.kept = false;
                eager.body[at] = BodyItem::New(c.pairs.clone(), args.clone());
                out.push(eager);
                BodyItem::Not(c.small.clone(), vec![NOwned::C(1)])
            });
            // A variable of the reader may already have a made witness's spelling.
            let mut witness = depth + 1;
            if scope.intermediates() {
                let occupied = m.names(v);
                while stands[rel.as_str()].iter().any(|(head, atoms)| {
                    atoms.iter().flat_map(|(_, iface)| iface).filter(|x| !head.contains(x)).any(|x| occupied.contains(&format!("fz{witness}_{x}")))
                }) {
                    witness += 1;
                }
            }
            for (head, atoms) in &stands[rel.as_str()] {
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
                if let Some(cold) = &cold {
                    copy.body.push(cold.clone());
                }
                work.push((copy, depth + 1));
            }
        }
    }
    let round = Round { decls, undeclared: stands.keys().cloned().collect(), rules: out, no_inline };
    Planned { round, lines, made, caches }
}

/// Each cache's rules (`Cache`): its guard, true while its parts' row counts admit their pairs, and the pairs.
fn write_caches(b: &mut Builder<'_>, caches: &BTreeMap<String, Cache>) -> Result<(), String> {
    use crate::code_rule_dsl::logic::front::ast::AggOp;
    use crate::code_rule_dsl::logic::program_image::T;
    for c in caches.values() {
        let mut rb = b.rule().head(c.small.as_str(), &[T::Int(1)]);
        let counts: Vec<String> = (0..c.factors.len()).map(|i| format!("rr_count_{i}")).collect();
        for ((rel, iface), count) in c.factors.iter().zip(&counts) {
            rb = rb.agg(T::Var(count), &AggOp::Count, None, rel.as_str(), &vec![T::Wild; iface.len()]);
        }
        let admit = T::Call("rr_admit_pairs", vec![T::Var(&counts[0]), T::Var(&counts[1]), T::Int(c.readers as i64), T::Int(c.limit as i64)]);
        rb.cmp(admit, CmpOp::Eq, T::Int(1)).finish()?;
        let head: Vec<T<'_>> = c.head.iter().map(|x| T::Var(x)).collect();
        let mut rb = b.rule().head(c.pairs.as_str(), &head).atom(c.small.as_str(), &[T::Int(1)]);
        for (rel, iface) in &c.factors {
            let args: Vec<T<'_>> = iface.iter().map(|x| T::Var(x)).collect();
            rb = rb.atom(rel.as_str(), &args);
        }
        rb.finish()?;
    }
    Ok(())
}

/// The intermediates `names` (decision 0477) handed on as reduced parts, also within a document or the cross-file
/// part, where every read of them is an atom a deferred joins pass checked: `None` when none is, else the version
/// made, a line per relation, and per relation its cache (guard, pairs, allowance), if it has one.
pub(crate) fn factorize_intermediates(
    v: &std::sync::Arc<Version>,
    names: &BTreeSet<String>,
    cache_reused: bool,
) -> Result<Option<(std::sync::Arc<Version>, Vec<String>, BTreeMap<String, Option<(String, String, usize)>>)>, String> {
    use crate::code_rule_dsl::logic::program_image::fields::{FieldsMut as _, ProgramFields as PF};
    if names.is_empty() {
        return Ok(None);
    }
    let d = manager::analysis::<FieldsData>(v)?;
    let f = Fields::over(v, &d);
    let scope = Scope::Intermediates { names, cache_reused };
    let s = survey(v, f, scope);
    if s.chosen.is_empty() {
        return Ok(None);
    }
    let planned = round_of(v, f, &s, scope);
    if planned.made.is_empty() {
        return Ok(None);
    }
    let mut b = Builder::derive(v);
    write(&mut b, v, &planned.round)?;
    write_caches(&mut b, &planned.caches)?;
    // The relations handed on lose their qualifiers with their declarations; each cache's guard is nullary.
    let mut qualifiers = PF::qualifiers(&**v);
    for rel in &planned.round.undeclared {
        qualifiers.remove(rel);
    }
    for n in &planned.round.no_inline {
        qualifiers.entry(n.clone()).or_default().insert("no_inline".to_owned());
    }
    b.fields().set_field(crate::code_rule_dsl::logic::program_facts::Field::Qualifiers(&qualifiers));
    if !planned.caches.is_empty() {
        let mut nullary = PF::nullary(&**v);
        nullary.extend(planned.caches.values().map(|c| c.small.clone()));
        b.fields().set_field(crate::code_rule_dsl::logic::program_facts::Field::Nullary(&nullary));
    }
    let made = b.finish()?.0;
    let mut caches = planned.caches;
    let choices = planned.made.keys().map(|rel| (rel.clone(), caches.remove(rel).map(|c| (c.small, c.pairs, c.limit)))).collect();
    Ok(Some((made, planned.lines, choices)))
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
        let scope = Scope::Exports(&split.local);
        let mut s = survey(v, f, scope);
        loop {
            if s.chosen.is_empty() {
                return Ok(());
            }
            let planned = round_of(v, f, &s, scope);
            // Each round's program is split again: a part the split does not find file-local leaves its relation whole.
            let (_, now) = super::candidate(v, &planned.round)?;
            let failed: Vec<String> = planned.made.iter().filter(|(_, parts)| parts.iter().any(|p| !now.local.contains(p))).map(|(r, _)| r.clone()).collect();
            if failed.is_empty() {
                self.lines = planned.lines;
                write(w.derive()?, v, &planned.round)?;
                return Ok(());
            }
            for r in failed {
                s.chosen.remove(&r);
            }
        }
    }
}
