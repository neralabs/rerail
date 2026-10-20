//! A compiler proof for progressive packed pattern walks. Unknown-producing operations,
//! recursive reads, semantic effects, field walks and rule-based stops are barriers.
use super::{Emitter, Live, Op, Reads, Stop, Walk};
use crate::code_rule_dsl::sql::{plan::SlotKind, terminals::scan::Scan};

fn body<'a>(
    op: &'a Op,
    pattern: &mut Option<&'a Op>,
    kinds: &mut Option<Vec<u16>>,
    regexes: &mut Vec<usize>,
) -> bool {
    match op {
        Op::Pattern { .. } if pattern.is_none() => {
            *pattern = Some(op);
            true
        }
        Op::Kinds(ks) => {
            *kinds = Some(kinds.take().map_or_else(
                || ks.clone(),
                |old| old.into_iter().filter(|k| ks.contains(k)).collect(),
            ));
            true
        }
        Op::Regex(id) => {
            regexes.push(*id);
            true
        }
        Op::All(items) => items.iter().all(|op| body(op, pattern, kinds, regexes)),
        Op::AllOpen(items) => items
            .iter()
            .all(|term| body(&term.op, pattern, kinds, regexes)),
        _ => false,
    }
}
impl Emitter<'_> {
    pub(super) fn scan(&mut self, op: &Op, input: &str, _live: &Live) -> Option<String> {
        let (walk, sub, count, bind) = match op {
            Op::Relation { walk, sub, .. } | Op::Search { walk, sub, .. } => {
                (*walk, &**sub, None, None)
            }
            Op::Count {
                walk,
                sub,
                min,
                max,
                bind,
                ..
            } => (*walk, &**sub, Some((*min, *max, bind.is_some())), *bind),
            _ => return None,
        };
        let walk = match walk {
            Walk::Has(Stop::End) => 0,
            Walk::Has(Stop::Neighbor) => 1,
            Walk::Inside(Stop::End) => 2,
            Walk::Inside(Stop::Neighbor) => 3,
            Walk::Precedes(Stop::End) => 4,
            Walk::Precedes(Stop::Neighbor) => 5,
            Walk::Follows(Stop::End) => 6,
            Walk::Follows(Stop::Neighbor) => 7,
            _ => return None,
        };
        let (mut pattern, mut kinds, mut regexes) = (None, None, Vec::new());
        if !body(sub, &mut pattern, &mut kinds, &mut regexes) {
            return None;
        }
        let Op::Pattern {
            term, vars, reads, ..
        } = pattern?
        else {
            return None;
        };
        let columns: Vec<_> = reads
            .iter()
            .flat_map(|&slot| match self.slots[slot].kind {
                SlotKind::List => vec![format!("s{slot}a"), format!("s{slot}b")],
                _ => vec![format!("s{slot}")],
            })
            .collect();
        let descriptor = std::iter::once(input.to_owned())
            .chain(columns)
            .collect::<Vec<_>>()
            .join(",");
        let scan = serde_json::to_string(&Scan {
            walk,
            kinds,
            regexes,
            count,
        })
        .expect("scan serializes");
        let mut overrides = Vec::new();
        if count.is_some() {
            if let Some((slot, offset)) = bind {
                overrides.push((format!("s{slot}"), format!("p.v0 + ({offset})")));
            }
        } else {
            for &slot in vars {
                for column in self.columns(slot) {
                    overrides.push((column, format!("p.v{}", overrides.len())));
                }
            }
        }
        let values = (0..if count.is_some() {
            1
        } else {
            vars.iter().map(|&s| self.columns(s).len()).sum()
        })
            .map(|i| format!(", v{i}"))
            .collect::<String>();
        let with = format!(
            "WITH p AS MATERIALIZED (SELECT ek{values} FROM pattern WHERE term = {term} AND frame = '{descriptor}' AND scan = '{scan}' AND ek IN (SELECT k FROM {input})) "
        );
        let selected = if self.three {
            self.verdict("f", &overrides, "0")
        } else {
            self.select("f", &overrides)
        };
        Some(self.frame_with(
            "x",
            &with,
            &format!("SELECT {selected} FROM {input} f JOIN p ON p.ek = f.k"),
            Reads::join(&[input]),
        ))
    }
}
