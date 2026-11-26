//! SPIKE C2 (throw-away, shared strata work): a body walked once, read by every rule that walks it.
//!
//! (a) A rule whose body holds, up to its variables' names, the whole body of another relation's one rule reads that
//! relation instead: `rf_site(i) :- nameref(i, _), !bind_only(i), !rf_deleted(i), !guest_of(i, _), !rf_object(i),
//! rr_ast_35(i)` walks the 2.4 M names `pre_386` already walked (`pre_386(i) :- nameref(i, _), !rf_deleted(i), …`),
//! and becomes `rf_site(i) :- pre_386(i), !bind_only(i)`. No relation is made that was not made before.
//!
//! `RERAIL_SPIKE_PREFIX=1` turns it on.

use super::super::program_image::{Builder, RuleRef, Slot, T, TermRef};
use super::body::{Freeze as _, Placed};
use super::fields::Fields;
use std::collections::HashMap;

pub fn on() -> bool {
    std::env::var("RERAIL_SPIKE_PREFIX").as_deref() == Ok("1")
}

/// A literal as matched: its shape with its variables left out, and its variables in order.
struct Shape {
    skel: String,
    vars: Vec<u32>,
}

fn shapes(v: &super::super::program_image::Version, r: RuleRef<'_>) -> Vec<Shape> {
    r.body()
        .map(|l| {
            let text = v.show_lit_named(r, l, &|s: Slot| format!("\u{1}{}\u{2}", s.0));
            let (mut skel, mut vars) = (String::new(), Vec::new());
            let mut rest = text.as_str();
            while let Some(i) = rest.find('\u{1}') {
                skel.push_str(&rest[..i]);
                skel.push('\u{1}');
                let j = rest[i..].find('\u{2}').expect("a named variable ends") + i;
                vars.push(rest[i + 1..j].parse().expect("a slot"));
                rest = &rest[j + 1..];
            }
            skel.push_str(rest);
            Shape { skel, vars }
        })
        .collect()
}

/// A match of `q`'s body into `t`'s: for each of `q`'s literals the literal of `t`'s it is, and `q`'s variables
/// as `t`'s. One to one, each literal once.
fn matched(q: &[Shape], t: &[Shape]) -> Option<(Vec<usize>, HashMap<u32, u32>)> {
    fn go(i: usize, q: &[Shape], t: &[Shape], used: &mut Vec<bool>, at: &mut Vec<usize>, m: &mut HashMap<u32, u32>, back: &mut HashMap<u32, u32>) -> bool {
        if i == q.len() {
            return true;
        }
        for j in 0..t.len() {
            if used[j] || t[j].skel != q[i].skel {
                continue;
            }
            let mut added = Vec::new();
            let ok = q[i].vars.iter().zip(&t[j].vars).all(|(&a, &b)| match (m.get(&a), back.get(&b)) {
                (Some(&x), _) => x == b,
                (None, Some(_)) => false,
                (None, None) => {
                    m.insert(a, b);
                    back.insert(b, a);
                    added.push(a);
                    true
                }
            });
            if ok {
                used[j] = true;
                at.push(j);
                if go(i + 1, q, t, used, at, m, back) {
                    return true;
                }
                at.pop();
                used[j] = false;
            }
            for a in added {
                if let Some(b) = m.remove(&a) {
                    back.remove(&b);
                }
            }
        }
        false
    }
    let (mut used, mut at, mut m, mut back) = (vec![false; t.len()], Vec::new(), HashMap::new(), HashMap::new());
    go(0, q, t, &mut used, &mut at, &mut m, &mut back).then_some((at, m))
}

pub struct Shared {
    pub lines: Vec<String>,
}

pub fn apply(placed: &Placed) -> Result<Option<(Shared, Placed)>, String> {
    let v = &placed.v;
    let Some(f) = Fields::of(placed) else { return Ok(None) };
    let comps = placed.components().map_err(|e| format!("{e:?}"))?;
    let n = placed.len();
    let mut by_head: HashMap<&str, Vec<usize>> = HashMap::new();
    for k in 0..n {
        by_head.entry(v.rel_name(placed.rule(k).head().rel())).or_default().push(k);
    }
    // Relations in components of one relation, not reading themselves.
    let mut lone: HashMap<&str, bool> = HashMap::new();
    for c in comps.iter() {
        let ms: Vec<&str> = c.iter().collect();
        let rec = ms.len() > 1 || by_head.get(ms[0]).is_some_and(|ks| ks.iter().any(|&k| placed.rule(k).reads().any(|a| v.rel_name(a.rel()) == ms[0])));
        for m in ms {
            lone.insert(m, !rec);
        }
    }
    let shapes_of: Vec<Vec<Shape>> = (0..n).map(|k| shapes(v, placed.rule(k))).collect();
    // The patterns: a relation's one rule of two literals or more, its head distinct variables.
    let patterns: Vec<(usize, &str, Vec<u32>)> = by_head
        .iter()
        .filter(|(rel, ks)| ks.len() == 1 && lone.get(**rel).copied().unwrap_or(false) && !f.gate(rel) && !f.request(rel) && f.lattice_decl(rel).is_none())
        .filter_map(|(rel, ks)| {
            let k = ks[0];
            let r = placed.rule(k);
            if shapes_of[k].len() < 2 {
                return None;
            }
            let hv: Option<Vec<u32>> = r.head().args().map(|t| match t {
                TermRef::Var(s) => Some(s.0),
                _ => None,
            }).collect();
            let hv = hv?;
            let mut seen = hv.clone();
            seen.sort_unstable();
            seen.dedup();
            (seen.len() == hv.len()).then_some((k, *rel, hv))
        })
        .collect();
    let mut lines = Vec::new();
    // Each rewritten rule: the pattern read and the target's literals it stands for.
    let mut rewrites: HashMap<usize, (usize, Vec<usize>, Vec<u32>)> = HashMap::new();
    for t in 0..n {
        let r = placed.rule(t);
        let head = v.rel_name(r.head().rel());
        if !lone.get(head).copied().unwrap_or(false) {
            continue;
        }
        // Each variable's uses in the head.
        let head_vars: Vec<u32> = r.head().args().filter_map(|x| x.var().map(|s| s.0)).collect();
        let mut best: Option<(usize, Vec<usize>, Vec<u32>)> = None;
        for (q, rel, hv) in &patterns {
            if *q == t || *rel == head || shapes_of[*q].len() > shapes_of[t].len() || best.as_ref().is_some_and(|b| shapes_of[b.0].len() >= shapes_of[*q].len()) {
                continue;
            }
            let Some((at, m)) = matched(&shapes_of[*q], &shapes_of[t]) else { continue };
            // A variable the pattern leaves out of its head is read by none of the target's other literals or its head.
            let inside: std::collections::HashSet<u32> = m.iter().filter(|(a, _)| !hv.contains(a)).map(|(_, &b)| b).collect();
            let outside_uses = head_vars.iter().chain((0..shapes_of[t].len()).filter(|j| !at.contains(j)).flat_map(|j| shapes_of[t][j].vars.iter())).any(|x| inside.contains(x));
            if outside_uses {
                continue;
            }
            best = Some((*q, at, hv.iter().map(|a| m[a]).collect()));
        }
        if let Some(b) = best {
            lines.push(format!("{head} reads {} for {} literals", v.rel_name(placed.rule(b.0).head().rel()), b.1.len()));
            rewrites.insert(t, b);
        }
    }
    if rewrites.is_empty() {
        return Ok(None);
    }
    let mut b = Builder::on(&placed.v);
    let mut seq = Vec::new();
    for k in 0..n {
        let id = placed.id(k);
        let Some((q, at, args)) = rewrites.get(&k) else {
            seq.push(id);
            continue;
        };
        b.remove(id);
        let r = placed.rule(k);
        let rel = v.rel_name(placed.rule(*q).head().rel()).to_owned();
        let names: Vec<String> = args.iter().map(|&s| v.text(r.var(Slot(s))).to_owned()).collect();
        let targs: Vec<T<'_>> = names.iter().map(|x| T::Var(x)).collect();
        let mut rb = b.rule().source(id).head_from(v, r, r.head()).atom(rel.as_str(), &targs);
        for (j, l) in r.body().enumerate() {
            if !at.contains(&j) {
                rb = rb.lit(v, r, l);
            }
        }
        seq.push(rb.finish()?);
    }
    let out = b.freeze_placed(seq, placed.whole)?;
    Ok(Some((Shared { lines }, out)))
}
