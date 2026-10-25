//! Partial evaluation of pure control on node kind, before producing any rows.
//! Only complete, metadata-proved executions qualify. Structural/text probes
//! and cycles retain the original reached interpreter and failure behavior.
use super::super::ir::Set;
use super::lower::pure_recursive_lower::{Instruction, Program};
use std::collections::BTreeSet;
#[derive(Clone)]
pub(super) struct Answer {
    pub truth: bool,
    pub cost: u64,
    pub entries: Vec<u32>,
}
pub(super) fn predicate(s: &Set, kind: Option<u16>) -> Option<bool> {
    match s {
        Set::AllNodes => Some(true),
        Set::Empty => Some(false),
        Set::Kinds(v) => Some(kind.is_some_and(|k| v.contains(&k))),
        Set::Complement(s) => predicate(s, kind).map(|v| !v),
        Set::Intersect(v) | Set::Union(v) => {
            let all = matches!(s, Set::Intersect(_));
            let mut unknown = false;
            for s in v {
                match predicate(s, kind) {
                    Some(x) if x != all => return Some(x),
                    None => unknown = true,
                    _ => {}
                }
            }
            (!unknown).then_some(all)
        }
        _ => None,
    }
}
fn evaluate(
    p: &Program,
    pc: usize,
    kind: Option<u16>,
    active: &mut BTreeSet<usize>,
    visits: &mut usize,
) -> Option<Answer> {
    *visits += 1;
    if *visits > 65536 || !active.insert(pc) {
        return None;
    }
    let result = (|| match p.instructions.get(pc)? {
        Instruction::Return(truth) => Some(Answer {
            truth: *truth,
            cost: 0,
            entries: vec![],
        }),
        Instruction::Test {
            predicate: s,
            yes,
            no,
        } => evaluate(
            p,
            if predicate(s, kind)? { *yes } else { *no },
            kind,
            active,
            visits,
        ),
        Instruction::Call { utility, yes, no } => {
            let body = evaluate(p, p.functions[*utility], kind, active, visits)?;
            let rest = evaluate(p, if body.truth { *yes } else { *no }, kind, active, visits)?;
            let entries = std::iter::once(0)
                .chain(body.entries.iter().map(|d| d + 1))
                .chain(rest.entries.iter().copied())
                .collect::<Vec<_>>();
            if entries.len() > 256 {
                return None;
            }
            Some(Answer {
                truth: rest.truth,
                cost: entries.len() as u64,
                entries,
            })
        }
        _ => None,
    })();
    active.remove(&pc);
    result
}
pub(super) fn kinds(s: &Set, out: &mut BTreeSet<u16>) {
    match s {
        Set::Kinds(v) => out.extend(v),
        Set::Intersect(v) | Set::Union(v) => {
            for s in v {
                kinds(s, out)
            }
        }
        Set::Complement(s) => kinds(s, out),
        _ => {}
    }
}
pub(super) fn compile(p: &Program) -> Vec<(Option<u16>, Option<Answer>)> {
    let mut palette = BTreeSet::new();
    for i in &p.instructions {
        if let Instruction::Test { predicate, .. } = i {
            kinds(predicate, &mut palette)
        }
    }
    palette
        .into_iter()
        .map(Some)
        .chain(std::iter::once(None))
        .map(|k| (k, evaluate(p, p.entry, k, &mut BTreeSet::new(), &mut 0)))
        .collect()
}

/// Compile exact failure/peak prefixes of a proved constant entry trace.
pub(super) fn fields(a: &Answer, depth: &str, work: &str) -> [String; 3] {
    let mut maxima = Vec::new();
    let mut greatest = None;
    for (i, d) in a.entries.iter().copied().enumerate() {
        if greatest.is_none_or(|m| d > m) {
            maxima.push((i, d));
            greatest = Some(d);
        }
    }
    let breach = if maxima.is_empty() {
        "0".into()
    } else {
        format!(
            "CASE {} ELSE {} END",
            maxima
                .iter()
                .map(|(i, d)| format!("WHEN ({depth})>={} THEN {i}", 256 - d))
                .collect::<Vec<_>>()
                .join(" "),
            a.cost
        )
    };
    let truth = format!(
        "CASE WHEN ({breach})<{} AND {work}>=({breach}) THEN -1 WHEN {work}<{} THEN -2 ELSE {} END",
        a.cost,
        a.cost,
        usize::from(a.truth)
    );
    let cost = format!("min({work},({breach}))");
    let peak = if maxima.is_empty() {
        "0".into()
    } else {
        format!(
            "max(0,{})",
            maxima
                .iter()
                .map(|(i, d)| format!(
                    "CASE WHEN {work}>={i} AND ({breach})>{i} THEN {} ELSE 0 END",
                    d + 1
                ))
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    [truth, cost, peak]
}
