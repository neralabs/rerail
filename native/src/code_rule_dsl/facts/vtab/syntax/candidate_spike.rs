//! Test-only bridge from immutable core facts to typed candidate rows.
use super::*;
use crate::code_rule_dsl::facts::producers::python::families as py;
use crate::code_rule_dsl::facts::producers::python::{families::candidate_spike, import_screen::Imports};
impl Batch {
    pub(crate) fn candidate_spike(&self, targets: &std::collections::BTreeSet<String>) -> Result<Vec<Vec<rusqlite::types::Value>>, String> {
        use rusqlite::types::Value;
        let _span = crate::code_rule_dsl::logic::profile::span(|| "native candidate spike".into());
        let mode = std::env::var("RERAIL_CANDIDATE_SPIKE").unwrap();
        let collector_mode=if mode=="typed_ranges" {"context_ranges"} else {&mode};
        let py = self.bound::<super::py_families::Py>().ok_or("candidate producer requires Python")?;
        let wanted = targets.iter().filter(|s| !s.starts_with("builtins.")).cloned().collect();
        let globals = targets.iter().filter_map(|s| s.strip_prefix("builtins.").map(str::to_owned)).collect();
        let members = std::collections::BTreeSet::from(["open".to_owned()]);
        let mut output = Vec::new();
        let mut total = candidate_spike::Work::default();
        for p in self.docs.iter().filter(|p| p.tag == py.grammar.tag()) {
            let (rows, work) = candidate_spike::experiment(&py.grammar, &p.facts, 2, &members, collector_mode, |n| {
                let imports = p.python_imports.get_or_init(|| Imports::from_core(&p.facts, &self.dialects[&p.tag].catalog, p.module.as_ref()));
                if mode=="typed_ranges" {
                    use crate::code_rule_dsl::facts::producers::python::python_facts::Fact;
                    match candidate_spike::eligibility(&py.grammar,&p.facts,n) {
                        Fact::Known(chain)=>imports.may_qualify_parts(chain.head,chain.attributes,&wanted,&globals,p.module.as_ref()),
                        Fact::NotApplicable(_)=>false,
                        Fact::Unknown(_)|Fact::Unsupported(_)=>true,
                    }
                }else {py::qualification_chain(&py.grammar, &p.facts, n).is_none_or(|(head, tail)| imports.may_qualify_parts(head, tail, &wanted, &globals, p.module.as_ref()))}
            });
            total.posting_calls += work.posting_calls; total.context_ranges += work.context_ranges; total.calls += work.calls; total.contexts += work.contexts; total.screens += work.screens;
            for [call, report, reference, receiver, flags, owner] in rows {
                let id = |n: u32| -> Option<i64> {
                    if n == NONE { Some(-1) } else { p.local.get(n as usize).copied().filter(|&v| v != NONE).map(|v| p.global(v)) }
                };
                // Match projection eligibility of the existing site/context/attribute relations.
                if let (Some(call), Some(report), Some(reference), Some(receiver), Some(_)) = (id(call), id(report), id(reference), id(receiver), id(owner)) {
                    output.push(vec![Value::Integer(call), Value::Integer(report), Value::Integer(reference), Value::Integer(receiver), Value::Integer(flags as i64)]);
                    total.emitted += 1; total.direct += usize::from(flags & 1 != 0); total.receivers += usize::from(flags & 2 != 0);
                }
            }
        }
        crate::code_rule_dsl::logic::profile::decision("candidate spike", || serde_json::to_string(&total).unwrap());
        Ok(output)
    }
}
