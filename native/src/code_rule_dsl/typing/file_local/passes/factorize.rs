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

/// Per relation handed on as parts, per rule: its key and its parts (literal places).
type Plan = BTreeMap<String, Vec<(String, Vec<Vec<usize>>)>>;

/// Per relation factorized as an intermediate (decision 0477): its pair cache, where one was made (its guard, its
/// pairs, the witness allowance), else none.
pub type FactorCaches = BTreeMap<String, Option<(String, String, usize)>>;

/// Which relations are handed on as parts: the exports (file-local relations the cross-file part reads, `local`), or
/// the intermediates named (decision 0477's deferred joins: two factors that meet in each reader, in its own part).
#[derive(Clone, Copy)]
enum Scope<'a> {
    Exports(&'a BTreeSet<String>),
    Intermediates(&'a BTreeSet<String>),
}

/// `v`'s rules, each relation's reads (rule, literal) a part may stand in for, each relation's rules, and the plan
/// for each relation the scope hands on as parts.
#[allow(clippy::type_complexity)]
fn choose<'v>(v: &'v Version, f: Fields<'v>, scope: Scope<'_>, keys: &Keys) -> (Vec<RuleRef<'v>>, BTreeMap<&'v str, Vec<(usize, usize)>>, BTreeMap<&'v str, Vec<RuleRef<'v>>>, Plan) {
    let none = BTreeSet::new();
    let local = match scope {
        Scope::Exports(local) => local,
        Scope::Intermediates(_) => &none,
    };
    let head_of = |r: RuleRef<'v>| v.rel_name(r.head().rel());
    let natives_read: BTreeSet<&str> = f.natives().iter().filter(|n| !local.contains(v.rel_name(n.rel))).flat_map(|n| n.reads.iter()).map(|&r| v.rel_name(r)).collect();
    let rules: Vec<RuleRef<'v>> = v.rules().collect();
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
    let mut rules_of: BTreeMap<&str, Vec<RuleRef<'v>>> = BTreeMap::new();
    for r in &rules {
        rules_of.entry(head_of(*r)).or_default().push(*r);
    }
    let mut chosen: Plan = BTreeMap::new();
    for (rel, read) in &reads {
        let rel = *rel;
        let engine = rel.starts_with("magic_") || rel.starts_with("done_") || rel.contains("__");
        let named = match scope {
            Scope::Exports(local) => local.contains(rel),
            Scope::Intermediates(names) => names.contains(rel),
        };
        if !named
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
            let fans = parts.iter().filter(|p| !functional(v, *rule, &part_lits(p), &key, &head, keys)).count();
            if fans < 2 {
                break;
            }
            plan.push((key, parts));
        }
        if plan.len() != of.len() {
            continue;
        }
        // An export's every reader binds a column of it (its join starts from a bound part); an intermediate's
        // readers were chosen by the deferred joins pass, which checked each.
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
        if binds || matches!(scope, Scope::Intermediates(_)) {
            chosen.insert(rel.to_owned(), plan);
        }
    }
    (rules, reads, rules_of, chosen)
}

/// The witness allowance across a program's pair caches (decision 0477): `RERAIL_JOIN_CACHE_ROWS`, 4096 by default.
fn pair_allowance() -> usize {
    std::env::var_os("RERAIL_JOIN_CACHE_ROWS").map_or(4096, |s| s.to_str().and_then(|s| s.parse::<usize>().ok()).unwrap_or(0)).min(i64::MAX as usize)
}

/// What one round of the factorized store makes: its program, a line per relation, the parts made per relation, and
/// the pair caches.
struct Made2<'v> {
    round: Round<'v>,
    lines: Vec<String>,
    made: BTreeMap<String, Vec<String>>,
    caches: FactorCaches,
}

/// The round handing `chosen`'s relations on as parts (the module comment). As intermediates (`cache_reused`), a
/// relation of one two-part rule read by more than one rule, each reading no other chosen relation, has a pair cache
/// besides: its pairs stored while the factors' counts multiply to at most its share of the allowance
/// (`rr_admit_pairs`), each reader reading the pairs there and the parts where the guard does not hold.
#[allow(clippy::too_many_arguments)]
fn round<'v>(
    v: &'v Version,
    f: Fields<'v>,
    rules: &[RuleRef<'v>],
    reads: &BTreeMap<&'v str, Vec<(usize, usize)>>,
    rules_of: &BTreeMap<&'v str, Vec<RuleRef<'v>>>,
    chosen: &Plan,
    intermediates: bool,
    cache_reused: bool,
) -> Made2<'v> {
    let head_of = |r: RuleRef<'v>| v.rel_name(r.head().rel());
    let decl_names: HashSet<String> = f.decls().iter().map(|d| v.text(d.name()).to_owned()).collect();
    // Each part named from its content (`helper_name`).
    let mut used = HashMap::new();
    let mut decls: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let mut declare = |rel: String, cols: Vec<(String, String)>| {
        decls.retain(|(r, _)| *r != rel);
        decls.push((rel, cols));
    };
    let mut no_inline: Vec<String> = Vec::new();
    let mut nullary: Vec<String> = Vec::new();
    let mut lines = Vec::new();
    let mut made: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut program: Vec<Made<'v>> = rules.iter().map(|&r| Made { src: r, head: None, body: r.body().map(BodyItem::Orig).collect(), kept: true }).collect();
    // Per relation, per rule: the head's variables and the parts' atoms (relation, interface).
    let mut stands: BTreeMap<String, Vec<(Vec<String>, Vec<(String, Vec<String>)>)>> = BTreeMap::new();
    // A reader is split at most once, so a cached relation's readers read no other chosen relation.
    let cacheable: BTreeSet<&String> = chosen
        .iter()
        .filter(|(rel, plan)| {
            cache_reused
                && plan.len() == 1
                && plan[0].1.len() == 2
                && reads[rel.as_str()].len() > 1
                && reads[rel.as_str()].iter().all(|&(ri, _)| rules[ri].body().filter(|l| matches!(l.view(), L::Atom(a) if chosen.contains_key(v.rel_name(a.rel())))).count() == 1)
        })
        .map(|(rel, _)| rel)
        .collect();
    // Across the caches the joined witnesses are at most the allowance: both factors are sets, so even one shared
    // hot key contributes at most |left| * |right|.
    let pair_limit = if cacheable.is_empty() { 0 } else { pair_allowance() / cacheable.len() };
    // Per cached relation: its pairs' relation and its guard's.
    let mut caches: BTreeMap<String, (String, String)> = BTreeMap::new();
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
                // An intermediate's part is named apart from a key of the same digest too.
                let pname = super::super::helper_name("fz", rel, &content, |n| decl_names.contains(n) || (intermediates && decl_names.contains(&format!("fzk{}", &n[2..]))), &mut used);
                declare(pname.clone(), cols.clone());
                if intermediates && !no_inline.contains(&pname) {
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
            if pair_limit > 0 && cacheable.contains(&rel) {
                let (pairs, small) = cache(v, f, rule, &atoms, reads[rel.as_str()].len(), pair_limit, &decl_names, &mut program, &mut declare);
                nullary.push(small.clone());
                no_inline.extend([pairs.clone(), small.clone()]);
                caches.insert(rel.clone(), (pairs, small));
                shown.push(format!("cache if factor row counts multiply to at most {pair_limit}"));
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
            caches.remove(rel);
            continue;
        }
        let scope = if intermediates { "reader" } else { "cross-file" };
        lines.push(format!("{rel}: handed on as parts, each reduced by the others' keys, read in {} {scope} rules; {}", reads[rel.as_str()].len(), shown.join("; ")));
        stands.insert(rel.clone(), per_rule);
    }
    // The relation's rules go; each read becomes the parts of each of its rules, the key and the variables the read
    // leaves open named apart (the open ones by the copy's depth). A cached relation's read is read as its pairs where
    // the guard holds, and as the parts where it does not.
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
                _ => false,
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
                _ => unreachable!("a read"),
            };
            let mut cold = None;
            if let Some((pairs, small)) = caches.get(&rel) {
                let mut copy = m.clone();
                copy.kept = false;
                copy.body[at] = BodyItem::New(pairs.clone(), args.clone());
                out.push(copy);
                cold = Some(BodyItem::Not(small.clone(), vec![NOwned::Int(1)]));
            }
            // A variable of the reader may have a witness's spelling: the witnesses are named past it.
            let occupied: BTreeSet<String> = if intermediates { made_vars(v, &m) } else { BTreeSet::new() };
            for (head, atoms) in &stands[rel.as_str()] {
                let mut witness_depth = depth + 1;
                while atoms.iter().flat_map(|(_, iface)| iface).filter(|x| !head.contains(x)).any(|x| occupied.contains(&format!("fz{witness_depth}_{x}"))) {
                    witness_depth += 1;
                }
                let to = |x: &str| -> NOwned<'_> {
                    match head.iter().position(|h| h == x) {
                        Some(i) => args[i].clone(),
                        None => NOwned::V(format!("fz{witness_depth}_{x}")),
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
    let choices: FactorCaches = stands.keys().map(|rel| (rel.clone(), caches.get(rel).map(|(pairs, small)| (small.clone(), pairs.clone(), pair_limit)))).collect();
    Made2 { round: Round { decls, undeclared: stands.keys().cloned().collect(), rules: out, no_inline, nullary }, lines, made, caches: choices }
}

/// The variables a rule being made names: its source rule's, and those its made terms name.
fn made_vars(v: &Version, m: &Made<'_>) -> BTreeSet<String> {
    fn named(n: &NOwned<'_>, out: &mut BTreeSet<String>) {
        match n {
            NOwned::V(x) => {
                out.insert(x.clone());
            }
            NOwned::Call(_, args) => args.iter().for_each(|a| named(a, out)),
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    m.src.head().args().for_each(|t| out.extend(term_vars(v, m.src, t)));
    for it in &m.body {
        match it {
            BodyItem::Orig(l) => out.extend(lit_vars(v, m.src, *l)),
            BodyItem::New(_, args) | BodyItem::Not(_, args) => args.iter().for_each(|a| named(a, &mut out)),
            BodyItem::Count { var, .. } => {
                out.insert(var.clone());
            }
            BodyItem::Cmp(l, _, r) => {
                named(l, &mut out);
                named(r, &mut out);
            }
        }
    }
    if let Some((_, args)) = &m.head {
        args.iter().for_each(|a| named(a, &mut out));
    }
    out
}

/// A pair cache of `rule`'s two parts (`atoms`, its factors), read by `readers` rules (decision 0477): its guard
/// `small()` holds while the exact sizes of both factors fit `limit` (`rr_admit_pairs`, which an edit may cross
/// either way), and its pairs are the factors' join under the guard. The two rules join `program`; their relations
/// are declared. The guard is written in the physical unit encoding (a nullary relation's `rr_unit` column), as the
/// pass runs after the lowering made it. Its pairs' relation and its guard's.
#[allow(clippy::too_many_arguments)]
fn cache<'v>(
    v: &'v Version,
    f: Fields<'v>,
    rule: RuleRef<'v>,
    atoms: &[(String, Vec<String>)],
    readers: usize,
    limit: usize,
    decl_names: &HashSet<String>,
    program: &mut Vec<Made<'v>>,
    declare: &mut impl FnMut(String, Vec<(String, String)>),
) -> (String, String) {
    let base = super::super::helper_name("rr_pair_cache_", v.rel_name(rule.head().rel()), &v.show_rule(rule), |n| ["small", "pairs"].iter().any(|s| decl_names.contains(&format!("{n}_{s}"))), &mut HashMap::new());
    let (small, pairs) = (format!("{base}_small"), format!("{base}_pairs"));
    declare(small.clone(), vec![("rr_unit".to_owned(), "int".to_owned())]);
    declare(pairs.clone(), f.columns(v.rel_name(rule.head().rel())).unwrap_or_default().iter().map(|(c, t)| ((*c).to_owned(), (*t).to_owned())).collect());
    let mut guard: Vec<BodyItem<'v>> = atoms.iter().enumerate().map(|(i, (p, iface))| BodyItem::Count { var: format!("rr_count_{i}"), rel: p.clone(), width: iface.len() }).collect();
    let admit = NOwned::Call(
        "rr_admit_pairs".to_owned(),
        vec![NOwned::V("rr_count_0".to_owned()), NOwned::V("rr_count_1".to_owned()), NOwned::Int(readers as i64), NOwned::Int(limit as i64)],
    );
    guard.push(BodyItem::Cmp(admit, crate::code_rule_dsl::logic::program_image::CmpOp::Eq, NOwned::Int(1)));
    program.push(Made { src: rule, head: Some((small.clone(), vec![NOwned::Int(1)])), body: guard, kept: false });
    let mut body = vec![BodyItem::New(small.clone(), vec![NOwned::Int(1)])];
    body.extend(atoms.iter().map(|(p, iface)| BodyItem::New(p.clone(), iface.iter().map(|x| NOwned::V(x.clone())).collect())));
    program.push(Made { src: rule, head: Some((pairs.clone(), rule.head().args().map(NOwned::T).collect())), body, kept: false });
    (pairs, small)
}

/// The intermediates `names` of the version `v` (a derivation when not `first`) handed on as parts, each read in its
/// readers' own part (decision 0477, the deferred joins' factors), with pair caches where `cache_reused`: the version
/// made, a line per relation, and the caches; none where no relation splits so.
pub fn factorize_intermediates(v: &std::sync::Arc<Version>, first: bool, names: &BTreeSet<String>, cache_reused: bool) -> Result<Option<(std::sync::Arc<Version>, Vec<String>, FactorCaches)>, String> {
    if names.is_empty() {
        return Ok(None);
    }
    let d = manager::analysis::<FieldsData>(v)?;
    let f = Fields::over(v, &d);
    let keys = relation_keys(v, f);
    let (rules, reads, rules_of, chosen) = choose(v, f, Scope::Intermediates(names), &keys);
    if chosen.is_empty() {
        return Ok(None);
    }
    let made = round(v, f, &rules, &reads, &rules_of, &chosen, true, cache_reused);
    if made.lines.is_empty() {
        return Ok(None);
    }
    let mut b = Builder::derive(v);
    super::write_derived(&mut b, v, &made.round, first)?;
    Ok(Some((b.finish()?.0, made.lines, made.caches)))
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
        let keys = relation_keys(v, f);
        let (rules, reads, rules_of, mut chosen) = choose(v, f, Scope::Exports(&split.local), &keys);
        // Made, checked by the split, and made again without the relations whose parts it does not find file-local.
        loop {
            if chosen.is_empty() {
                return Ok(());
            }
            let Made2 { round, lines, made, .. } = round(v, f, &rules, &reads, &rules_of, &chosen, false, false);
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
