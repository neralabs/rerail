//! Apply logical proofs without treating SQLite's chosen access plan as evidence.
use super::*;
impl Retained {
    pub(super) fn support_uniqueness(&self) -> crate::code_rule_dsl::logic::analysis::Decision {
        if self.rules.len() != 1 {
            return crate::code_rule_dsl::logic::analysis::Decision::new(
                false,
                "Multiple rules can support the same head; disjointness is not proven",
            );
        }
        self.rules[0].uniqueness.decision.clone()
    }
    pub(super) fn head_query(&self, columns: &str, source: &str) -> String {
        if self.support_uniqueness().proven {
            format!("SELECT {columns},1 FROM {source}")
        } else {
            format!("SELECT {columns},count(*) FROM {source} GROUP BY {columns}")
        }
    }
    pub(in crate::code_rule_dsl::sql_executor) fn explain(&self) -> Value {
        json!({"relation":self.output,"support_uniqueness":self.support_uniqueness(),"full_rebuild_grouping_elided":self.support_uniqueness().proven,"incremental_support":"Signed old/new contributions still require reconciliation","cold_reuse":self.cold_reuse,"cold_initialized":self.cold_initialized,"rules":self.rules.iter().map(|r|json!({"uniqueness":r.uniqueness.as_ref(),"scalar_summaries":r.scalar_analysis.as_ref(),"initial_evaluation":r.initial_evaluation,"shared_summary_lookups":r.reducers.iter().filter(|s|s.shared_lookup_owner()).count()})).collect::<Vec<_>>()})
    }
}
