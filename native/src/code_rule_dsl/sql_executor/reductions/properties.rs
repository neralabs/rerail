//! Apply logical proofs without treating SQLite's chosen access plan as evidence.
use super::*;
impl Retained {
    pub(super) fn support_uniqueness(&self) -> crate::code_rule_dsl::logic::analysis::Decision {
        self.support_uniqueness.clone()
    }
    pub(super) fn head_query(&self, columns: &str, source: &str) -> String {
        if self.support_uniqueness().proven {
            format!("SELECT {columns},1 FROM {source}")
        } else {
            format!("SELECT {columns},count(*) FROM {source} GROUP BY {columns}")
        }
    }
    fn contribution_choice(&self, r: &Rule) -> crate::code_rule_dsl::logic::analysis::Decision {
        use crate::code_rule_dsl::logic::analysis::Decision;
        if r.contribution_view || !r.contribution_storage.decision.proven {
            return r.contribution_storage.decision.clone();
        }
        let reason = if self.lattice.as_ref().is_some_and(|l| l.tree.is_some()) {
            "Registered lattice support keeps its materialized transition path"
        } else if self.rules.len() > 64 || r.width + self.width >= 2000 {
            "The native support phase exceeds its checked SQL size bound"
        } else {
            "Reconstruction is enabled for native positive phases; this transition retains materialized support"
        };
        Decision::new(false, reason)
    }
    pub(in crate::code_rule_dsl::sql_executor) fn explain(&self) -> Value {
        json!({"relation":self.output,"head_support_store":self.heads,"lattice_support_index":self.lattice.as_ref().and_then(|l|l.tree.as_ref()).map(|t|json!({"nodes":t.nodes,"roots":t.roots,"merge":t.merge,"strategy":"Ordered AVL support index; SQL evaluates registered merges"})),"support_uniqueness":self.support_uniqueness(),"full_rebuild_grouping_elided":self.support_uniqueness().proven,"incremental_support":if self.lattice.is_none() && self.support_uniqueness().proven {"The unique witness proof permits old/new set reconciliation without grouping in SQL phases; fallback paths retain signed counts"} else {"Signed old/new contributions require aggregation"},"cold_reuse":self.cold_reuse,"scalar_support_phase":self.scalar_phase_analysis(),"common_wave_phase":self.common_phase(),"isolated_checked_support_phase":self.isolated_phase(),"cold_initialized":self.cold_initialized,"rules":self.rules.iter().map(|r|json!({"uniqueness":r.uniqueness.as_ref(),"contribution_storage":r.contribution_storage,"contributions_reconstructed":r.contribution_view,"contribution_choice":self.contribution_choice(r),"context_store":r.contexts,"context_insertion_coverage":{"if_insertions_equal_current_input":crate::code_rule_dsl::logic::analysis::deltas::complete_positive_input(crate::code_rule_dsl::logic::plan::acyclic::total(&r.context_query.body),true),"requires":"Exact immutable input and insertion root identity","application":"Actual anchor and proof appear in SQL phase logicalAnalyses; deletion and scalar invalidation branches remain"},"contribution_store":if r.contribution_view {None}else{Some(&r.contributions)},"scalar_summaries":r.scalar_analysis.as_ref(),"initial_evaluation":r.initial_evaluation,"evaluation_order":r.evaluation_order,"summary_storage":r.reducers.iter().map(|s|s.storage_explain()).collect::<Vec<_>>(),"shared_summary_lookups":r.reducers.iter().filter(|s|s.shared_lookup_owner()).count()})).collect::<Vec<_>>()})
    }
}
