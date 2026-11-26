//! SPIKE B (throw-away, shared strata work): a relation written by several strata, as segments.
//!
//! Today a stratum is a component of relations: all of a relation's rules run in its stratum, once every relation
//! any of them reads is complete. Here each rule has its own level (one more than the deepest level it reads), and a
//! relation whose rules land at different levels is written as one segment a level, `R__b<L>`, by the rules of that
//! level. Its positive readers are copied, one copy a segment, so a copy runs at its segment's level (semi-naive
//! evaluation carried across strata); its negated and aggregated readers, and everything outside the program's
//! rules, read `R`, the segments' union. A spike: the union is a relation of its own here (`R :- R__b<L>`), where the
//! design reads the segments in place.
//!
//! `RERAIL_SPIKE_SEGMENTS=1` turns it on.

use super::super::program_image::build::Rewrite;
use super::super::program_image::{AtomRef, Builder, Lit, T};
use super::body::{Freeze as _, Placed};
use super::fields::Fields;
use std::collections::{BTreeMap, HashMap, HashSet};

/// The most copies one rule may be made into; a rule whose segmented reads would make more reads the unions.
const MAX_COPIES: usize = 4;

/// Whether the spike is on.
pub fn on() -> bool {
    std::env::var("RERAIL_SPIKE_SEGMENTS").as_deref() == Ok("1")
}

/// Relations the demand and deferred-join machinery read by name: never split, and a read of one never copied.
fn machinery(rel: &str) -> bool {
    ["demand_", "rr_deferred_pairs_", "rr_demand_agg_", "rr_pair_cache_", "rr_product_demand"].iter().any(|p| rel.starts_with(p))
}

struct Rename<'a> {
    atoms: &'a HashMap<u32, String>,
    head: Option<String>,
}

impl Rewrite for Rename<'_> {
    fn atom_as(&mut self, a: AtomRef<'_>) -> Option<(String, bool, bool)> {
        self.atoms.get(&a.node()).map(|n| (n.clone(), a.neg(), a.gated()))
    }
    fn head_as(&mut self, _a: AtomRef<'_>) -> Option<String> {
        self.head.clone()
    }
}

/// What the pass did: the relations split, each with its levels.
pub struct Split {
    pub lines: Vec<String>,
}

/// `placed` with its relations split into segments where their rules' levels differ; `None` where none is.
pub fn apply(placed: &Placed) -> Result<Option<(Split, Placed)>, String> {
    let v = &placed.v;
    let Some(f) = Fields::of(placed) else { return Ok(None) };
    let comps = placed.components().map_err(|e| format!("{e:?}"))?;
    // The program's rules by head.
    let mut by_head: HashMap<&str, Vec<usize>> = HashMap::new();
    for k in 0..placed.len() {
        by_head.entry(v.rel_name(placed.rule(k).head().rel())).or_default().push(k);
    }
    // Each relation's level (inputs 0), and the segments of each split relation: (name, level).
    let mut level: HashMap<String, usize> = HashMap::new();
    let mut segments: HashMap<String, Vec<(String, usize)>> = HashMap::new();
    // The rules written: each program rule's replacement (copies with their heads and renamed atoms).
    struct Copy {
        head: Option<String>,
        atoms: HashMap<u32, String>,
    }
    let mut copies: BTreeMap<usize, Vec<Copy>> = BTreeMap::new();
    let mut unions: Vec<(String, Vec<String>)> = Vec::new();
    let mut lines = Vec::new();
    let lvl = |level: &HashMap<String, usize>, r: &str| level.get(r).copied().unwrap_or(0);
    for comp in comps.iter() {
        let members: Vec<&str> = comp.iter().collect();
        let recursive = members.len() > 1
            || members.iter().any(|&m| by_head.get(m).is_some_and(|ks| ks.iter().any(|&k| placed.rule(k).reads().any(|a| v.rel_name(a.rel()) == m))));
        let splittable = !recursive && {
            let m = members[0];
            !machinery(m)
                && !f.is_input(m)
                && !f.gate(m)
                && !f.request(m)
                && f.lattice_decl(m).is_none()
                && v.decl_of(m).is_some()
                && !by_head.get(m).is_some_and(|ks| ks.iter().any(|&k| placed.rule(k).reads().any(|a| { let r = v.rel_name(a.rel()); machinery(r) && r.ends_with(m) })))
        };
        // Each rule's copies: the segment each positive read of a split relation reads, and the copy's level.
        let mut ruled: Vec<(usize, Vec<(HashMap<u32, String>, usize)>)> = Vec::new();
        for &m in &members {
            for &k in by_head.get(m).map(Vec::as_slice).unwrap_or(&[]) {
                let r = placed.rule(k);
                // The positive plain atoms reading a split relation, each with its segments.
                let mut split_reads: Vec<(u32, &Vec<(String, usize)>)> = Vec::new();
                let mut base = 0usize;
                for l in r.body() {
                    match l.view() {
                        Lit::Atom(a) if !a.neg() && !recursive && segments.contains_key(v.rel_name(a.rel())) => split_reads.push((a.node(), &segments[v.rel_name(a.rel())])),
                        _ => {
                            if let Some(a) = l.atom() {
                                base = base.max(lvl(&level, v.rel_name(a.rel())));
                            }
                        }
                    }
                }
                let n: usize = split_reads.iter().map(|(_, s)| s.len()).product();
                if n > MAX_COPIES {
                    for (node, _) in split_reads.drain(..) {
                        let a = r.reads().find(|a| a.node() == node).expect("a read of the rule");
                        base = base.max(lvl(&level, v.rel_name(a.rel())));
                    }
                }
                let mut out: Vec<(HashMap<u32, String>, usize)> = vec![(HashMap::new(), base)];
                for (node, segs) in &split_reads {
                    out = out.into_iter().flat_map(|(atoms, l)| segs.iter().map(move |(s, sl)| { let mut a = atoms.clone(); a.insert(*node, s.clone()); (a, l.max(*sl)) }).collect::<Vec<_>>()).collect();
                }
                ruled.push((k, out.into_iter().map(|(a, l)| (a, l + 1)).collect()));
            }
        }
        let levels: Vec<usize> = { let mut l: Vec<usize> = ruled.iter().flat_map(|(_, c)| c.iter().map(|(_, l)| *l)).collect(); l.sort_unstable(); l.dedup(); l };
        let top = levels.last().copied().unwrap_or(1);
        if splittable && levels.len() > 1 {
            let m = members[0];
            let segs: Vec<(String, usize)> = levels.iter().map(|&l| (format!("{m}__b{l}"), l)).collect();
            for (k, cs) in ruled {
                copies.insert(k, cs.into_iter().map(|(atoms, l)| Copy { head: Some(format!("{m}__b{l}")), atoms }).collect());
            }
            lines.push(format!("{m}: {} rules at levels {levels:?}", by_head[m].len()));
            unions.push((m.to_owned(), segs.iter().map(|(s, _)| s.clone()).collect()));
            level.insert(m.to_owned(), top + 1);
            for (s, l) in &segs {
                level.insert(s.clone(), *l);
            }
            segments.insert(m.to_owned(), segs);
        } else {
            for (k, cs) in ruled {
                if cs.len() > 1 || cs.iter().any(|(a, _)| !a.is_empty()) {
                    copies.insert(k, cs.into_iter().map(|(atoms, _)| Copy { head: None, atoms }).collect());
                }
            }
            // A component's members share its level (the deepest of its rules).
            for &m in &members {
                level.insert(m.to_owned(), top);
            }
        }
    }
    if unions.is_empty() {
        return Ok(None);
    }
    let mut b = Builder::on(&placed.v);
    for (m, segs) in &unions {
        let d = v.decl_of(m).expect("a split relation is declared");
        let cols: Vec<(String, String)> = d.cols().map(|(n, t)| (v.text(n).to_owned(), v.text(t).to_owned())).collect();
        let cols: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        for s in segs {
            b.fields().declare(s, &cols);
        }
    }
    let mut seq = Vec::new();
    for k in 0..placed.len() {
        let id = placed.id(k);
        let Some(cs) = copies.get(&k) else {
            seq.push(id);
            continue;
        };
        b.remove(id);
        let r = placed.rule(k);
        for c in cs {
            let mut rw = Rename { atoms: &c.atoms, head: c.head.clone() };
            let mut rb = b.rule().source(id).head_with(v, r, r.head(), &mut rw);
            for l in r.body() {
                rb = rb.lit_with(v, r, l, &mut rw);
            }
            seq.push(rb.finish()?);
        }
    }
    let names: HashSet<&str> = unions.iter().map(|(m, _)| m.as_str()).collect();
    for (m, segs) in &unions {
        let arity = v.decl_of(m).expect("declared").cols().len();
        let vars: Vec<String> = (0..arity).map(|c| format!("rr_seg{c}")).collect();
        let args: Vec<T<'_>> = vars.iter().map(|x| T::Var(x)).collect();
        for s in segs {
            seq.push(b.rule().head(m.as_str(), &args).atom(s.as_str(), &args).finish()?);
        }
    }
    let _ = names;
    let out = b.freeze_placed(seq, placed.whole)?;
    Ok(Some((Split { lines }, out)))
}
