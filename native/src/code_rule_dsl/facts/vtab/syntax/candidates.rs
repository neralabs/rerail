//! Candidate demands carry document identity before any scalar AST relation is materialized.
use super::*;
use crate::code_rule_dsl::facts::producers::python::families as py;
use crate::code_rule_dsl::facts::producers::python::{
    families::candidates, python_facts::Fact,
};
impl Batch {
    pub(crate) fn python_candidates(
        &self,
        scope: &[i64],
        targets: &BTreeSet<String>,
        context: &str,
        member: &str,
    ) -> Result<Vec<Vec<rusqlite::types::Value>>, String> {
        use rusqlite::types::Value;
        let _span =
            crate::code_rule_dsl::logic::profile::span(|| "native Python candidates".into());
        let kind = py::CONTEXTS
            .iter()
            .position(|&v| v == context)
            .ok_or_else(|| format!("unknown Python context {context}"))? as u32;
        let Some(py) = self.bound::<super::py_families::Py>() else {
            return Ok(Vec::new());
        };
        let scope: BTreeSet<_> = scope.iter().copied().collect();
        let wanted = targets
            .iter()
            .filter(|s| !s.starts_with("builtins."))
            .cloned()
            .collect();
        let globals = targets
            .iter()
            .filter_map(|s| s.strip_prefix("builtins.").map(str::to_owned))
            .collect();
        let members = BTreeSet::from([crate::code_rule_dsl::identifier::nfkc(member).into_owned()]);
        let mut output = Vec::new();
        let mut total = candidates::Work::default();
        for p in self
            .docs
            .iter()
            .filter(|p| p.tag == py.grammar.tag() && scope.contains(&i64::from(p.key)))
        {
            let (rows, work) = candidates::collect(&py.grammar, &p.facts, kind, &members, |n| {
                match candidates::eligibility(&py.grammar, &p.facts, n) {
                    Fact::Known(chain) => py
                        .imports(self, p)
                        .may_qualify_parts(
                            chain.head,
                            chain.attributes,
                            &wanted,
                            &globals,
                            p.module.as_ref(),
                        ),
                    Fact::NotApplicable(_) => false,
                    Fact::Unknown(_) | Fact::Unsupported(_) => true,
                }
            });
            total.calls += work.calls;
            total.posting_calls += work.posting_calls;
            total.context_ranges += work.context_ranges;
            total.contexts += work.contexts;
            total.screens += work.screens;
            for [call, report, reference, receiver, flags, owner] in rows {
                let id = |n: u32| {
                    if n == NONE {
                        Some(-1)
                    } else {
                        p.local
                            .get(n as usize)
                            .copied()
                            .filter(|&v| v != NONE)
                            .map(|v| p.global(v))
                    }
                };
                if let (Some(call), Some(report), Some(reference), Some(receiver), Some(_)) =
                    (id(call), id(report), id(reference), id(receiver), id(owner))
                {
                    output.push(vec![
                        Value::Integer(call),
                        Value::Integer(report),
                        Value::Integer(if flags & 1 != 0 { reference } else { -1 }),
                        Value::Integer(receiver),
                    ]);
                    total.emitted += 1;
                    total.direct += usize::from(flags & 1 != 0);
                    total.receivers += usize::from(flags & 2 != 0);
                }
            }
        }
        crate::code_rule_dsl::logic::profile::decision("Python candidate work", || {
            serde_json::to_string(&total).unwrap()
        });
        Ok(output)
    }
}
