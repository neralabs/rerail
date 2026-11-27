//! Joins deferred to their readers' requests (decision 0488). A binary positive join over a key its head projects
//! away (`wide(a, b) :- lhs(a, k), rhs(b, k)`) is driven by what its readers request: the pairs, or one endpoint
//! (`DEFERRED_PAIRS`, a demand relation as `upstream`'s), or else kept as its two factors, which meet in each reader
//! (`file_local::factorize_intermediates`), with a pair cache where the factors are small. A frequently requested
//! endpoint shares its side's witnesses across the pairs requested of it (`share`).
//!
//! On the version, as `product_demand`'s other steps (decision 0276): each rewrite is a derivation of the program
//! (`Program`), and the pass publishes the last as an edit of its input.

use rerail_blocks::vector::Vector;
use super::{DEFERRED_PAIRS, PREFIX, Program, plain, rewritten, same, upstream};
use crate::code_rule_dsl::logic::plan::body::{VarSet, atom_vars, lit_vars, term_vars};
use crate::code_rule_dsl::logic::plan::corpus::preparation::Deferral;
use crate::code_rule_dsl::logic::profile::author_joins::{Access, Join, TOPIC};
use crate::code_rule_dsl::logic::program_facts::Field;
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{CmpOp, Lit, RuleRef, T, TermRef, Version};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// An endpoint requested by this many pairs shares its side's witnesses (`share`).
const SHARE_AFTER: i64 = 8;

/// What the pass did: the relations driven by their requested pairs, by one requested endpoint (`rel column c`), the
/// lines of those kept as factors, the endpoints shared, whether a pair cache was made; and the report of each join
/// (`profile::author_joins`).
#[derive(Default)]
pub struct Plans {
    pub pairs: Vec<String>,
    pub one_sided: Vec<String>,
    pub factors: Vec<String>,
    pub shared: Vec<String>,
    /// The joins left as written: the chunks make them, and a cross-file reader would state their requests.
    pub cross: Vec<String>,
    /// The joins left as written because the split holds them constants (decision 0665): of facts and settings alone,
    /// the same for every file and made once. Read by a file's rule, the requests would be the files', and the
    /// constant would read every file's rows. A constant only constants read is driven as any join is.
    pub constants: Vec<String>,
    pub cached: bool,
    pub author: Vec<Join>,
}

/// The place in `rule`'s body of its requests guard: a positive atom of a demand relation of its head (`PREFIX`,
/// `upstream::PREFIX`, `DEFERRED_PAIRS`) over the head's own terms.
fn guard(v: &Version, rule: RuleRef<'_>) -> Option<usize> {
    let head = v.rel_name(rule.head().rel());
    let args: Vec<TermRef<'_>> = rule.head().args().collect();
    rule.body().position(|l| match l.view() {
        Lit::Atom(a) => {
            !a.neg()
                && !a.gated()
                && a.args().count() == args.len()
                && a.args().zip(&args).all(|(x, y)| same(x, *y))
                && [PREFIX, upstream::PREFIX, DEFERRED_PAIRS].iter().any(|p| v.rel_name(a.rel()).strip_prefix(p) == Some(head))
        }
        _ => false,
    })
}

/// `rule` split into the two parts that meet at a key its head projects away (`keeps::factors_without`, its requests
/// guard left out), each part reading the key: its key's name and the parts by place. `None` for a rule whose head
/// is not two variables, whose body holds anything but positive atoms of plain terms and comparisons other than `=`,
/// or that splits otherwise.
fn split(v: &Version, rule: RuleRef<'_>) -> Option<(String, Vec<Vector<usize>>)> {
    let head: Vec<TermRef<'_>> = rule.head().args().collect();
    let [TermRef::Var(a), TermRef::Var(b)] = head.as_slice() else {
        return None;
    };
    let fits = rule.body().all(|l| match l.view() {
        Lit::Atom(x) => !x.neg() && !x.gated() && x.args().all(|t| plain(t) || matches!(t, TermRef::Wild)),
        Lit::Cmp { l, op, r } => op != CmpOp::Eq && plain(l) && plain(r),
        _ => false,
    });
    if a == b || !fits {
        return None;
    }
    let (key, parts) = crate::code_rule_dsl::logic::plan::keeps::factors_without(v, rule, guard(v, rule))?;
    let lits: Vec<_> = rule.body().collect();
    let keyed = |part: &Vector<usize>| part.iter().any(|&i| matches!(lits[i].view(), Lit::Atom(x) if atom_vars(x).iter().any(|s| v.text(rule.var(s)) == key)));
    (parts.len() == 2 && parts.iter().all(keyed)).then_some((key, parts))
}

/// Whether `rule`'s read at `j` has its columns `cols` bound by the rest of the body: each a plain term, and their
/// variables all read by one positive atom of another relation (or none).
fn reader_binds(v: &Version, rule: RuleRef<'_>, j: usize, cols: &[usize]) -> bool {
    let lits: Vec<_> = rule.body().collect();
    let Some(read) = lits[j].atom() else { return false };
    let args: Vec<TermRef<'_>> = read.args().collect();
    let mut bound = VarSet::new();
    for &c in cols {
        let Some(&t) = args.get(c) else { return false };
        if !plain(t) {
            return false;
        }
        term_vars(t, &mut bound);
    }
    let rel = v.rel_name(read.rel());
    bound.is_empty() || lits.iter().any(|l| matches!(l.view(), Lit::Atom(a) if v.rel_name(a.rel()) != rel && !a.neg() && !a.gated() && bound.is_subset(&atom_vars(a))))
}

/// Whether a reader's request (`proof`, of a read of `rel` at its key columns `cols`) asks for every key the
/// producer makes, so the request, a union over the readers, restricts nothing and costs a join of its own: each atom
/// naming a key appears in the producer's rule too, at the key's columns and with its constants (the producer may
/// hold more constants), and every other literal is a positive atom of no constant, reached from those. `PT010(g, x)
/// :- exc_call(c, "pytest.raises"), .., field(c, "function", g0), unparen(g0, g), text(g, x)` asks for `exc_call`'s
/// every call through `field(c, "function", _)`, which `exc_call`'s rule reads: corpus0's request made 1.23 M rows of
/// work to guard a join of 42 k (decision 0488). A cost guess, never a matter of rows: a join left as written is
/// always right.
fn void_request(v: &Version, rel: &str, cols: &[usize], proof: &upstream::Proof) -> bool {
    use super::Part;
    use crate::code_rule_dsl::logic::program_image::AtomRef;
    let producers: Vec<RuleRef<'_>> = v.rules().filter(|r| v.rel_name(r.head().rel()) == rel).collect();
    let [producer] = producers.as_slice() else { return false };
    let producer = *producer;
    let Some(reader) = v.rule(proof.rule) else { return false };
    let Some(read) = reader.body().nth(proof.read).and_then(|l| l.atom()) else { return false };
    let name = |rule: RuleRef<'_>, t: TermRef<'_>| match t {
        TermRef::Var(s) => Some(v.text(rule.var(s)).to_owned()),
        _ => None,
    };
    // The reader's variable, and the producer's, at each of the key's columns.
    let at = |rule: RuleRef<'_>, args: Vec<TermRef<'_>>| -> Option<Vec<String>> { cols.iter().map(|&c| args.get(c).and_then(|t| name(rule, *t))).collect() };
    let (Some(key), Some(own)) = (at(reader, read.args().collect()), at(producer, producer.head().args().collect())) else { return false };
    let constant = |t: TermRef<'_>| matches!(t, TermRef::Int(_) | TermRef::Str(_) | TermRef::Branded(_));
    let keyed_at = |t: TermRef<'_>| name(reader, t).and_then(|x| key.iter().position(|k| *k == x));
    let covered = |a: AtomRef<'_>| {
        producer.body().filter_map(|l| l.atom()).any(|b| {
            !b.neg()
                && b.rel() == a.rel()
                && b.args().count() == a.args().count()
                && a.args().zip(b.args()).all(|(x, y)| match keyed_at(x) {
                    Some(i) => name(producer, y).is_some_and(|w| w == own[i]),
                    None if constant(x) => same(x, y),
                    None => true,
                })
        })
    };
    let mut keyed = false;
    for part in &proof.parts {
        let Part::Lit(j) = *part else { return false };
        let Some(Lit::Atom(a)) = reader.body().nth(j).map(|l| l.view()) else { return false };
        if a.neg() || a.gated() {
            return false;
        }
        if a.args().any(|t| keyed_at(t).is_some()) {
            if !covered(a) {
                return false;
            }
            keyed = true;
        } else if a.args().any(|t| constant(t) || matches!(t, TermRef::Call(..))) {
            return false;
        }
    }
    keyed
}

/// Whether a reader's request (`proof`) of `rel`, a context's instance (decision 0597), reads a relation no rule derives:
/// a scan of the batch, which lists no tape value's nodes (decision 0431 §8: a value's nodes are reached by lookup),
/// so the request holds none of the context's nodes and the join it guards finds nothing. A join left as written is
/// always right. vue's `getter(f, x) :- vcomputed(_, x, f), docof(f, d)` in the context `script` asked
/// `rr_deferred_pairs_…_vcomputed(f) :- docof(f, _)` and found no computed getter of a component's script.
fn blind_in_a_context(v: &Version, rel: &str, proof: &upstream::Proof) -> bool {
    if !crate::code_rule_dsl::logic::passes_made::of_a_context(rel) {
        return false;
    }
    let Some(reader) = v.rule(proof.rule) else { return true };
    let derived = |a: crate::code_rule_dsl::logic::program_image::AtomRef<'_>| v.heads(a.rel()).next().is_some();
    proof.parts.iter().any(|part| match *part {
        super::Part::Lit(j) => reader.body().nth(j).and_then(|l| l.atom()).is_some_and(|a| !a.neg() && !derived(a)),
        _ => false,
    })
}

/// The pass over `p` (its input `input`): which restrictions the readers give each binary join (the module comment),
/// and the rewrites made. `adaptive`: endpoints shared and reduced factors for broad readers; `cache_pairs`: pair
/// caches admitted besides.
pub fn rewrite(p: &mut Program<'_>, input: &Arc<Version>, adaptive: bool, cache_pairs: bool) -> Result<Plans, String> {
    let mut plans = Plans::default();
    // The join relations by the readers they could be driven by, read off the program before any rewrite (a guard
    // inserted moves the places).
    // Each relation one document's in every row, with its location columns: a request keeps their tie.
    let mut same_doc: BTreeMap<String, rerail_blocks::vector::Vector<usize>> = BTreeMap::new();
    let (pairs, mut guarded, mut broad, mut open_readers, recursive) = {
        let v = p.v();
        let rules: Vec<RuleRef<'_>> = v.rules().collect();
        let mut definitions = BTreeMap::<&str, usize>::new();
        let mut readers = BTreeMap::<&str, Vec<(usize, usize)>>::new();
        for (i, r) in rules.iter().enumerate() {
            *definitions.entry(p.head_rel(*r)).or_default() += 1;
            for (j, l) in r.body().enumerate() {
                if let Some(a) = l.atom() {
                    readers.entry(v.rel_name(a.rel())).or_default().push((i, j));
                }
            }
        }
        // A relation the connected demand already restricted is reported with its guard, whatever this pass finds.
        let mut explained = BTreeSet::new();
        for rule in &rules {
            if let Some(at) = guard(v, *rule)
                && explained.insert(p.head_rel(*rule))
            {
                let request = rule.body().nth(at).and_then(|l| l.atom()).map(|a| v.rel_name(a.rel()).to_owned()).expect("the guard");
                let columns = p.columns(p.head_rel(*rule)).unwrap_or_default().into_iter().map(|(c, _)| c).collect();
                plans.author.push(Join { relation: p.head_rel(*rule).to_owned(), access: Access::Requests { table: request, columns }, readers: Vec::new(), placement: None });
            }
        }
        let tapes = v.tape_columns()?;
        let shapes = v.shape_list()?;
        let mut candidates = BTreeSet::new();
        let mut guarded: Vec<(String, usize)> = Vec::new();
        let mut nth: BTreeMap<&str, usize> = BTreeMap::new();
        for rule in &rules {
            let rel = p.head_rel(*rule);
            let n = {
                let k = nth.entry(rel).or_default();
                *k += 1;
                *k - 1
            };
            if rule.head().args().count() != 2
                || rule.body().count() < 2
                || !readers.contains_key(rel)
                || !upstream::eligible(p, rel)
                || p.f.qualifiers.get(rel).is_some_and(|q| !q.is_empty())
                || tapes.contains_key(rel)
                || p.f.demand.iter().any(|r| r == rel)
                || shapes.iter().any(|s| s.head == rel || s.lit == rel)
                || p.f.asked_by.values().flatten().any(|refs| refs.contains(rel))
            {
                continue;
            }
            if split(v, *rule).is_some() {
                if guard(v, *rule).is_some() {
                    if adaptive {
                        guarded.push((rel.to_owned(), n));
                    }
                } else if definitions[rel] == 1 {
                    candidates.insert(rel);
                }
            }
        }
        if candidates.is_empty() && guarded.is_empty() {
            return Ok(plans);
        }
        let Ok(sccs) = Program::components(v) else {
            return Ok(plans);
        };
        let recursive: BTreeSet<String> = sccs
            .iter()
            .filter(|c| c.len() > 1)
            .flatten()
            .map(str::to_owned)
            .chain(rules.iter().filter(|r| r.body().any(|l| l.atom().is_some_and(|a| a.rel() == r.head().rel()))).map(|r| p.head_rel(*r).to_owned()))
            .collect();
        let mut pairs: Vec<(String, &'static [usize], bool)> = Vec::new();
        let mut broad = BTreeSet::new();
        let mut open_readers: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for rel in candidates {
            if recursive.contains(rel) {
                continue;
            }
            let uses = &readers[rel];
            let columns = p.columns(rel).unwrap_or_default();
            let mut descriptions = BTreeSet::new();
            for &(i, j) in uses {
                let open: Vec<&str> = (0..2).filter(|c| !reader_binds(v, rules[i], j, &[*c])).map(|c| columns[c].0.as_str()).collect();
                if !open.is_empty() {
                    descriptions.insert((p.head_rel(rules[i]).to_owned(), format!("supplies no lookup value for {}", open.join(", "))));
                }
            }
            open_readers.insert(rel.to_owned(), descriptions.into_iter().collect());
            let lit = |i: usize, j: usize| rules[i].body().nth(j).expect("a read");
            if uses.iter().any(|&(i, j)| !matches!(lit(i, j).view(), Lit::Atom(a) if !a.gated())) {
                continue;
            }
            let positive = uses.iter().all(|&(i, j)| lit(i, j).atom().is_some_and(|a| !a.neg()));
            const MASKS: [&[usize]; 3] = [&[0, 1], &[0], &[1]];
            if let Some(cols) = MASKS.into_iter().find(|cols| uses.iter().all(|&(i, j)| reader_binds(v, rules[i], j, cols))) {
                pairs.push((rel.to_owned(), cols, positive));
            } else if positive {
                broad.insert(rel.to_owned());
            }
        }
        // A producer the chunks make (`file_local::split`) restricted by a request a cross-file reader states: the
        // request's rows are the chunk's own, so a reader joining another file's rows asks nothing of this one
        // (`remark(x, ..) :- noted(c, x), defined(_, x), ..`: b.py's comment names a.py's function, and a.py's chunk,
        // asked only by its own comments, made no `defined` row). Such a join is left as written.
        if !pairs.is_empty() {
            let current = p.made.clone().unwrap_or_else(|| Arc::clone(input));
            let split = crate::code_rule_dsl::logic::program_image::manager::analysis::<crate::code_rule_dsl::typing::file_local::FileLocal>(&current)?;
            same_doc = split.row_documents.clone();
            pairs.retain(|(rel, _, _)| {
                // A constant a file's rule reads (decision 0665): its requests would be the files'.
                if split.constants.contains(rel) && readers[rel.as_str()].iter().any(|&(i, _)| !split.constants.contains(p.head_rel(rules[i]))) {
                    plans.constants.push(rel.clone());
                    return false;
                }
                let cross = split.local.contains(rel) && readers[rel.as_str()].iter().any(|&(i, _)| !split.local.contains(p.head_rel(rules[i])));
                if cross {
                    plans.cross.push(rel.clone());
                }
                !cross
            });
        }
        (pairs, guarded, broad, open_readers, recursive)
    };
    // The requests installed, once every read is classified: in rounds, each one derivation of the program.
    let requests: Vec<(String, &[usize])> = pairs.iter().map(|(rel, cols, _)| (rel.clone(), *cols)).collect();
    let installed = upstream::install_requests(p, &requests, DEFERRED_PAIRS, &same_doc, &|rel, cols, v, proofs| !proofs.iter().any(|r| void_request(v, rel, cols, r) || blind_in_a_context(v, rel, r)))?;
    let mut no_inline = Vec::new();
    for ((rel, cols, positive), installed) in pairs.into_iter().zip(installed) {
        if !installed {
            continue;
        }
        no_inline.push(rel.clone());
        let columns = p.columns(&rel).unwrap_or_default();
        plans.author.push(Join {
            relation: rel.clone(),
            access: Access::Requests { table: format!("{DEFERRED_PAIRS}{rel}"), columns: cols.iter().map(|&c| columns[c].0.clone()).collect() },
            readers: open_readers.remove(&rel).unwrap_or_default(),
            placement: None,
        });
        if cols.len() == 1 {
            plans.one_sided.push(format!("{rel} column {}", cols[0]));
            // A request for one endpoint can still match the whole opposite factor: its guard stays on that factor,
            // and the positive readers read the join's factors directly.
            if adaptive && positive {
                broad.insert(rel);
            }
            continue;
        }
        plans.pairs.push(rel.clone());
        if adaptive {
            let v = p.v();
            if v.rules().find(|r| p.head_rel(*r) == rel).is_some_and(|r| split(v, r).is_some()) {
                guarded.push((rel, 0));
            }
        }
    }
    for (rel, n) in guarded {
        if !recursive.contains(&rel)
            && let Some(label) = share(p, &rel, n)?
        {
            plans.shared.push(label);
        }
    }
    let current = p.made.clone().unwrap_or_else(|| Arc::clone(input));
    if let Some((made, lines, choices)) = crate::code_rule_dsl::typing::file_local::factorize_intermediates(&current, p.first(), &broad, adaptive && cache_pairs)? {
        p.set(made);
        plans.factors = lines;
        plans.cached = choices.values().any(Option::is_some);
        for (relation, cache) in choices {
            let readers = open_readers.remove(&relation).unwrap_or_default();
            plans.author.push(Join { relation, access: Access::Factors { cache }, readers, placement: None });
        }
    }
    for (relation, readers) in open_readers {
        if !readers.is_empty() {
            plans.author.push(Join { relation, access: Access::Full, readers, placement: None });
        }
    }
    // A relation its requests drive is read whole by each reader, not inlined into them; one handed on as its factors
    // is gone.
    no_inline.retain(|rel| p.columns(rel).is_some());
    if !no_inline.is_empty() {
        let mut qualifiers = p.f.qualifiers.clone();
        for rel in &no_inline {
            qualifiers.entry(rel.clone()).or_default().insert("no_inline".to_owned());
        }
        let made = p.derive(&mut |_, _, _, _| Ok(false), &mut |_, b| {
            use crate::code_rule_dsl::logic::program_image::fields::FieldsMut as _;
            b.fields().set_field(Field::Qualifiers(&qualifiers));
            Ok(())
        })?;
        p.set(made);
    }
    Ok(plans)
}

/// The `n`th rule of `rel`, driven by its requests, sharing an endpoint's witnesses (`rel column c`): where a
/// requested endpoint is asked for by at least `SHARE_AFTER` pairs (`hot`), its larger part's rows for it are made
/// once (`factor(endpoint, witness)`), and each requested pair reads them with the other part; the rule itself
/// answers the endpoints that are not hot. None where the larger part is a single literal (an indexed input answers
/// the probe in place) or the rule does not split so.
fn share(p: &mut Program<'_>, rel: &str, n: usize) -> Result<Option<String>, String> {
    use crate::code_rule_dsl::logic::front::ast::AggOp;
    let v = p.v();
    let Some(rule) = v.rules().filter(|r| p.head_rel(*r) == rel).nth(n) else { return Ok(None) };
    let Some((key, parts)) = split(v, rule) else { return Ok(None) };
    let Some((side, part)) = parts.iter().enumerate().max_by_key(|(_, part)| part.len()) else { return Ok(None) };
    if part.len() < 2 {
        return Ok(None);
    }
    let lits: Vec<_> = rule.body().collect();
    let name = |s| v.text(rule.var(s)).to_owned();
    let vars: BTreeSet<String> = part.iter().flat_map(|&i| lit_vars(lits[i]).iter().map(name).collect::<Vec<_>>()).collect();
    let head: Vec<TermRef<'_>> = rule.head().args().collect();
    let Some(col) = head.iter().position(|t| matches!(t, TermRef::Var(s) if vars.contains(&name(*s)))) else { return Ok(None) };
    let TermRef::Var(at) = head[col] else { return Ok(None) };
    let endpoint = name(at);
    let key_ty = part.iter().find_map(|&i| {
        let a = lits[i].atom()?;
        let c = a.args().position(|t| matches!(t, TermRef::Var(s) if name(s) == key))?;
        p.columns(v.rel_name(a.rel()))?.get(c).map(|(_, ty)| ty.clone())
    });
    let (Some(key_ty), Some(endpoint_ty), Some(request_at)) = (key_ty, p.columns(rel).and_then(|c| c.get(col).map(|(_, t)| t.clone())), guard(v, rule)) else {
        return Ok(None);
    };
    let request = lits[request_at].atom().map(|a| v.rel_name(a.rel()).to_owned()).expect("the guard");
    let base = crate::code_rule_dsl::typing::file_local::helper_name("rr_deferred_", rel, &v.show_rule(rule), |n| ["hot", "factor"].iter().any(|s| p.columns(&format!("{n}_{s}")).is_some()), &mut Default::default());
    let (hot, factor) = (format!("{base}_hot"), format!("{base}_factor"));
    let mut count = "rr_fanout".to_owned();
    while count == endpoint {
        count.push('_');
    }
    let label = format!("{rel} column {col} ({} literals)", part.len());
    let (part, other, id) = (part.clone(), parts[1 - side].clone(), rule.id());
    let mut qualifiers = p.f.qualifiers.clone();
    for r in [&hot, &factor] {
        qualifiers.entry(r.clone()).or_default().insert("no_inline".to_owned());
    }
    qualifiers.entry(hot.clone()).or_default().insert(crate::code_rule_dsl::typing::file_local::STRATEGY.to_owned());
    let made = p.derive(
        &mut |first, v, r, b| {
            if r.id() != id {
                return Ok(false);
            }
            // The rule answers the cold endpoints.
            let mut rb = rewritten(first, v, r, b);
            for l in r.body() {
                rb = rb.lit(v, r, l);
            }
            rb.not(hot.as_str(), &[T::Var(&endpoint)]).finish().map(|_| true)
        },
        &mut |v, b| {
            use crate::code_rule_dsl::logic::program_image::fields::FieldsMut as _;
            let r = v.rule(id).expect("the shared rule");
            let lits: Vec<_> = r.body().collect();
            b.fields().declare(&hot, &[("endpoint", endpoint_ty.as_str())]);
            b.fields().declare(&factor, &[("endpoint", endpoint_ty.as_str()), ("witness", key_ty.as_str())]);
            b.fields().set_field(Field::Qualifiers(&qualifiers));
            let mut group = [T::Wild, T::Wild];
            group[col] = T::Var(&endpoint);
            // hot(e) :- request(…e…), n = count : { request(…e…) }, n >= SHARE_AFTER.
            b.rule()
                .head(hot.as_str(), &[T::Var(&endpoint)])
                .atom(request.as_str(), &group)
                .agg(T::Var(&count), &AggOp::Count, None, request.as_str(), &group)
                .cmp(T::Var(&count), CmpOp::Ge, T::Int(SHARE_AFTER))
                .finish()?;
            // factor(e, k) :- hot(e), the larger part.
            let mut rb = b.rule().head(factor.as_str(), &[T::Var(&endpoint), T::Var(&key)]).atom(hot.as_str(), &[T::Var(&endpoint)]);
            for &i in &part {
                rb = rb.lit(v, r, lits[i]);
            }
            rb.finish()?;
            // The hot endpoints' pairs: the request, the factor, the other part.
            let mut rb = b.rule().head_from(v, r, r.head()).lit(v, r, lits[request_at]).atom(factor.as_str(), &[T::Var(&endpoint), T::Var(&key)]);
            for &i in &other {
                rb = rb.lit(v, r, lits[i]);
            }
            rb.finish().map(drop)
        },
    )?;
    p.set(made);
    Ok(Some(label))
}

/// The deferred joins pass (decision 0488) under a deferral (`preparation::Deferral`): its decisions, and the report
/// of each join (`profile::author_joins::TOPIC`, which `preparation::front` reads back for the pair caches).
pub(in crate::code_rule_dsl::logic::plan::corpus) struct DeferJoins {
    deferral: Deferral,
}

impl DeferJoins {
    pub(in crate::code_rule_dsl::logic::plan::corpus) fn new(deferral: &Deferral) -> DeferJoins {
        DeferJoins { deferral: deferral.clone() }
    }
}

impl ProgramPass for DeferJoins {
    const NAME: &'static str = "joins deferred to their readers' requests";

    fn settings(&self) -> Vec<u8> {
        self.deferral.settings()
    }

    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let version: &Arc<Version> = cx.v();
        let mut program = Program::of(version);
        let plans = rewrite(&mut program, version, self.deferral.adaptive, self.deferral.cache_pairs)?;
        cx.decide("deferred joins", || {
            format!("pair demand: {:?}; endpoint demand: {:?}; reduced factors: {:?}; fanout sharing: {:?}; left as written, read across files: {:?}; left as written, constants: {:?}", plans.pairs, plans.one_sided, plans.factors, plans.shared, plans.cross, plans.constants)
        });
        for mut join in plans.author {
            if let Access::Factors { cache: None } = &join.access {
                join.placement = self.deferral.placement.get(&join.relation).cloned();
            }
            cx.decide(TOPIC, || serde_json::to_string(&join).expect("a join's report"));
        }
        if program.first() {
            return Ok(());
        }
        super::publish(version, program.v(), w)
    }
}

// cratify: `tests` is rerail-logic's (it mounts it: it reads the language, `rr::parse_version`, or the corpus's root)
