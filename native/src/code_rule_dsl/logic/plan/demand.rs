//! Demand for negated relations (decision 0048): a relation computed for every node that rules only negate is
//! computed for the keys those rules reach, not for every node.
//!
//! An imperfect conversion leaves relations like
//!
//! ```text
//! blocked($low, $high) :- nearest_k11($low, $s), inside_k2($s, $high).
//! reader(…) :- <body binding $low, $high>, !blocked($low, $high).
//! ```
//!
//! where `blocked` holds a row for every node (1.8M rows on 1000 files) and the reader asks about a few. Each reader
//! is split into its body less the negations it demands, and the negations:
//!
//! ```text
//! pre_<i>(vars) :- <the reader's body without its negations of demanded relations>.
//! demand_blocked($low, $high) :- pre_<i>(vars).
//! blocked($low, $high) :- demand_blocked($low, $high), nearest_k11($low, $s), inside_k2($s, $high).
//! reader(…) :- pre_<i>(vars), !blocked($low, $high).
//! ```
//!
//! Still a keyed anti-join, run set at a time (the spike: a per-row probe re-runs the body per outer row, S203/S064;
//! a keyed anti-join beats a subplan rescan, S254/S272). The guard is structural, not statistics (history index
//! §4): `pre_<i>` is the reader's own work less some anti-joins, planned freely in every chunk, so the demand never
//! exceeds what the reader binds. No plan is frozen into the split (a plan taken from one chunk ordered the body
//! badly in others: 320M rows), and a relation negated twice in one rule needs no copy, since every demand comes from
//! the one `pre_<i>`.
//!
//! A relation is demanded when it is derived and not kept, not recursive, read, and read only by atoms (negated or
//! positive) with variable or constant arguments, and when it is large:
//!
//! - every rule of it starts from every node (a `nearest_<set>`, `inside_<set>` or kind-free `node` atom on its key),
//!   or
//! - its scans (rules where a base atom binds its key) reach at least 1/32 of the chunk's nodes together, by the
//!   measured counts of the base relations and of the program's constants, never by estimates of derived relations.
//!   Its other rules take the demand too. The rows not built must outweigh the strata the split adds, opened in
//!   every chunk.
//!
//! The program overrides the size test (decision 0099): `.decl r(…) magic` demands r whenever it can be, and
//! `.decl r(…) no_magic` never.
//!
//! A positive read is cut like a negation, with a guard of its own. A negation only filters, so the reader's body less
//! it is no larger than the reader's work. A positive read can be what keeps the body small, so it is cut only where
//!
//! - the kept anchors bind all its variables, directly or through lookups of bounded fan-out (`reached`), and
//! - every kept positive atom is so reached, or the reader is itself demanded (its `pre_<i>` then starts from that
//!   demand).
//!
//! An anchor is a derived relation, or a base atom whose measured rows are fewer than 1/32 of the nodes. A text test is
//! not one (`*[a-z]*` matched 7.2M of 7.2M names), nor a common kind (`node(_, $n, 1025, _, _)`: 6M identifiers), nor
//! an axis (`inside_ks5` from the rare kind 1132 reached 18.4M nodes). Where only another demanded read anchors a read,
//! that one is kept in the body instead. For example
//!
//! ```text
//! m131($n) :- child($n, $c, _), node(_, $c, 1025, _, _), text($c, _).   // 6.0M rows on the full corpus
//! m132($n) :- m126($n), m131($n).                                          // m126: 242k rows
//! ```
//!
//! becomes `pre_i($n) :- m126($n)`, `demand_m131($n) :- pre_i($n)`, `m131` over `demand_m131`, and
//! `m132($n) :- pre_i($n), m131($n)`. Corpus runs only, so the golden plans are unchanged.

use super::super::program_image::{Builder, CmpOp, RuleId, RuleRef, T, TermRef, Version};
use super::keeps::inline::named;
use std::collections::{BTreeMap, BTreeSet};
use super::body::{Freeze as _, Placed};
use super::fields::Fields;

mod over;

/// Bound while a session evaluates its cross-file part (decision 0077): the rows it keeps are each relation's whole
/// rows, which delete-and-rederive maintains, so no relation is computed only for the keys its readers reach. A
/// relation kept for some keys would miss a key demanded later whose derivation no delta touches (spike O125).
#[derive(Clone, Copy, Default)]
struct Whole(bool);

/// While the guard lives, `split_negations` splits nothing (`Whole`).
pub(in crate::code_rule_dsl) fn whole_relations() -> crate::code_rule_dsl::logic::profile::Bound {
    crate::code_rule_dsl::logic::profile::bind(Whole(true))
}

/// What was demanded, and the program's version `placed` with each demandable relation computed for the keys its
/// readers reach (the module's split, an overlay of `placed`); `None` where nothing is, or where the split would not
/// stratify or type. `placed` must be the whole program (t36 item 6: `over`).
pub(super) fn split_negations(placed: &Placed, model: &super::Model) -> Option<(Vec<String>, Placed)> {
    if crate::code_rule_dsl::logic::profile::bound::<Whole>().0 {
        return None;
    }
    let Some(f) = Fields::of(placed) else {
        super::super::profile::decision("demand (decision 0048)", || "a version of rules alone: nothing demanded".to_owned());
        return None;
    };
    let mut candidates = over::candidates(&f, placed, model);
    // A relation that depends on an answer whose rows are a part of the chunk's inputs (decision 0431 §8: a tape
    // value's `parse_of` or `parsed`, its nodes and text admitted with it) is not demanded: its readers' bodies less it
    // (`pre_<i>`) would read those inputs before the answer, and find nothing of the value (Q325).
    let grown: BTreeSet<&str> = f.answers().iter().map(|&(_, p)| f.rel_name(p)).filter(|p| f.is_input(p)).collect();
    if !grown.is_empty() {
        let v = &placed.v;
        let mut above: BTreeSet<&str> = grown;
        loop {
            let before = above.len();
            for r in v.rules() {
                let head = v.rel_name(r.head().rel());
                if !above.contains(head) && r.reads().any(|a| above.contains(v.rel_name(a.rel()))) {
                    above.insert(head);
                }
            }
            if above.len() == before {
                break;
            }
        }
        candidates.retain(|c| !above.contains(*c));
    }
    if candidates.is_empty() {
        return None;
    }
    // A candidate whose split fails (a variable of no known type, or a cycle through a negation) is left as it was,
    // and the rest are split again: one rule must not keep every other from its demand (in a combined program, one
    // component closing a cycle left all 100 rules without one).
    let mut rounds = 0;
    loop {
        rounds += 1;
        match split_with(&f, placed, model, &candidates) {
            Ok(split) => {
                super::super::profile::decision("demand (decision 0048)", || format!("split in {rounds} rounds"));
                return Some(split);
            }
            Err(failed) => {
                let before = candidates.len();
                candidates.retain(|c| !failed.contains(*c));
                if candidates.is_empty() || candidates.len() == before {
                    return None;
                }
            }
        }
    }
}

/// The program's version `base` split for `candidates`, or the candidates whose split failed: the readings `over::analyse`'s, the split an
/// overlay of the version (`overlaid`), its strata checked over that overlay.
fn split_with(f: &Fields<'_>, base: &Placed, model: &super::Model, candidates: &BTreeSet<&str>) -> Result<(Vec<String>, Placed), BTreeSet<String>> {
    let (splits, decls, nullary) = over::analyse(f, base, model, candidates)?;
    let demanded: Vec<String> = candidates.iter().map(|r| (*r).to_owned()).collect();
    // The split as an overlay of the program's version (t36 item 6, step 3). A demanded relation's rules read its
    // demand first (`overlaid`).
    let placed = overlaid(base, candidates, &splits, &decls, &nullary).map_err(|e| BTreeSet::from([e]))?;
    // A split that closes a cycle through a negation (a reader's body reading what the negated relation depends on)
    // leaves the program as it was.
    // Every failing component at once, not the first: a round per component was a split and strata per round.
    let about = super::super::program_image::manager::analysis::<super::super::program_image::About>(&placed.v).map_err(|e| BTreeSet::from([e]))?;
    let errors = super::super::program_image::analyses::unstratified(&placed.v, &about);
    if errors.is_empty() {
        return Ok((demanded, placed));
    }
    let mut failed: BTreeSet<String> = BTreeSet::new();
    for e in &errors {
        // The candidates in the failing component (a `demand_<c>` there names `c`).
        let mine: BTreeSet<String> = match e {
            super::super::front::Error::NotStratifiable { component, .. } | super::super::front::Error::AggregationInRecursion { component, .. } => component
                .iter()
                .map(|r| r.strip_prefix("demand_").unwrap_or(r))
                .filter(|r| candidates.contains(r))
                .map(str::to_owned)
                .collect(),
            _ => BTreeSet::new(),
        };
        super::super::profile::decision("demand (decision 0048)", || format!("not demanded: {} (their split closes a cycle through a negation or an aggregate)", mine.iter().cloned().collect::<Vec<_>>().join(", ")));
        failed.extend(mine);
    }
    Err(failed)
}

/// A reader's split (`split_with`): its `pre_<i>`, the variables it hands on, and the places of the reads it cuts.
struct Split {
    pre: String,
    vars: Vec<String>,
    cut: BTreeSet<usize>,
}

/// The split as an overlay of `base`, the program's version (its rule `k` the program's rule `k`), its rules in the
/// tree's order: each reader replaced by its `pre_<i>` and itself over that, each demanded relation's rules reading
/// its demand first, then the demands.
fn overlaid(base: &Placed, candidates: &BTreeSet<&str>, splits: &BTreeMap<usize, Split>, decls: &[(String, Vec<(String, String)>)], nullary: &[String]) -> Result<Placed, String> {
    let v: &Version = &base.v;
    let mut b = Builder::on(&base.v);
    // The splits' and the demands' declarations, and the flags, so the version stays the whole program.
    for (rel, cols) in decls {
        b.fields().declare(rel, &cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect::<Vec<_>>());
    }
    for rel in candidates {
        if let Some(d) = v.decl_of(rel) {
            let cols: Vec<(&str, &str)> = d.cols().map(|(n, t)| (v.text(n), v.text(t))).collect();
            b.fields().declare(&format!("demand_{rel}"), &cols);
        }
    }
    b.fields().add_nullary(nullary);
    let (mut seq, mut demands) = (Vec::new(), Vec::new());
    for ri in 0..base.len() {
        let id = base.id(ri);
        let r = v.rule(id).expect("a program rule");
        let head = v.rel_name(r.head().rel());
        let own = candidates.contains(head);
        let demand = format!("demand_{head}");
        let fresh: Vec<String> = (0..r.head().arity()).map(|i| format!("rr_demanded{i}")).collect();
        let (own_args, computed) = demand_terms(v, r, &fresh);
        let Some(s) = splits.get(&ri) else {
            if !own {
                seq.push(id);
                continue;
            }
            b.remove(id);
            let mut rb = b.rule().head_from(v, r, r.head()).atom(demand.as_str(), &own_args);
            for l in r.body() {
                rb = rb.lit(v, r, l);
            }
            for (x, t) in &computed {
                rb = rb.cmp(T::Var(x), CmpOp::Eq, t.clone());
            }
            let n = rb.finish()?;
            seq.push(n);
            continue;
        };
        b.remove(id);
        // A split sharing no variable with its reads is a flag: one unit column.
        let pre_args: Vec<T<'_>> = if s.vars.is_empty() { vec![T::Int(1)] } else { s.vars.iter().map(|x| T::Var(x)).collect() };
        let mut rb = b.rule().head(s.pre.as_str(), &pre_args);
        if own {
            rb = rb.atom(demand.as_str(), &own_args);
        }
        let body = super::body::Body::of(v, r);
        let kept: Vec<usize> = (0..body.len()).filter(|i| !s.cut.contains(i)).collect();
        for &i in &kept {
            if !body.implied_read(i, &kept) {
                rb = rb.lit(v, r, body.lit(i));
            }
        }
        if own {
            for (x, t) in &computed {
                rb = rb.cmp(T::Var(x), CmpOp::Eq, t.clone());
            }
        }
        let pre = rb.finish()?;
        let mut rb = b.rule().head_from(v, r, r.head());
        if own {
            rb = rb.atom(demand.as_str(), &own_args);
        }
        rb = rb.atom(s.pre.as_str(), &pre_args);
        for (i, l) in r.body().enumerate() {
            if s.cut.contains(&i) {
                rb = rb.lit(v, r, l);
            }
        }
        if own {
            for (x, t) in &computed {
                rb = rb.cmp(T::Var(x), CmpOp::Eq, t.clone());
            }
        }
        let reader = rb.finish()?;
        seq.extend([pre, reader]);
        for &i in &s.cut {
            let read = r.body().nth(i).and_then(|l| l.atom()).expect("a cut read");
            let name = format!("demand_{}", v.rel_name(read.rel()));
            let args: Vec<T<'_>> = read.args().map(|t| named(v, t, &|x| v.text(r.var(x)))).collect();
            let d = b.rule().head(name.as_str(), &args).atom(s.pre.as_str(), &pre_args).finish()?;
            demands.push(d);
        }
    }
    seq.extend(demands);
    b.freeze_placed(seq, base.whole)
}

/// `demand_atom`'s arguments over the version: the head's, each computed one a fresh variable, and the checks that
/// each equals its term.
fn demand_terms<'a>(v: &'a Version, r: RuleRef<'a>, fresh: &'a [String]) -> (Vec<T<'a>>, Vec<(&'a str, T<'a>)>) {
    let mut computed = Vec::new();
    let args = r
        .head()
        .args()
        .enumerate()
        .map(|(i, t)| match t {
            TermRef::Call(..) => {
                computed.push((fresh[i].as_str(), named(v, t, &|s| v.text(r.var(s)))));
                T::Var(&fresh[i])
            }
            t => named(v, t, &|s| v.text(r.var(s))),
        })
        .collect::<Vec<_>>();
    (args, computed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_demand_prefix_keeps_only_reads_not_already_proved() {
        let program = crate::code_rule_dsl::logic::rr::parse_version(
            "
            .decl relation(k: number, value: number, tag: number)
            .decl veto(value: number)
            .decl result(k: number, value: number)
            .input relation, veto
            .output result
            result(k, value) :- relation(k, _, _), relation(k, value, _), relation(k, _, 1), !veto(value).
            ",
        )
        .unwrap();
        let base = Placed::whole(program);
        let split = Split { pre: "pre_0".into(), vars: vec!["k".into(), "value".into()], cut: BTreeSet::from([3]) };
        let decls = [("pre_0".into(), vec![("k".into(), "int".into()), ("value".into(), "int".into())])];
        let actual = overlaid(&base, &BTreeSet::from(["veto"]), &BTreeMap::from([(0, split)]), &decls, &[]).unwrap();
        let pre = actual.v.rules().find(|r| actual.v.rel_name(r.head().rel()) == "pre_0").map(|r| actual.v.show_rule(r)).unwrap();
        assert_eq!(pre, "pre_0($k, $value) :- relation($k, $value, _), relation($k, _, 1)");

        let body = super::super::body::Body::of(&base.v, base.v.rule(base.id(0)).unwrap());
        assert!(!body.implied_read(0, &[0, 3]), "a cut read cannot prove a kept read");
    }
}
