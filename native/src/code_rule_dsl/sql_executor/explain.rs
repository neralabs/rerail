//! Logical proofs are available with profiling disabled and without scanning facts.
use super::*;
impl CodeRuleSqlExecutor {
    pub(super) fn explain_value(&self) -> Value {
        json!({"format":"rerail.logical-analysis/v1","native_publication":{"empty":"Validated bulk roots are published directly","replacement":"Canonical bulk roots preserve row IDs; identical ordered rows reuse complete roots; unique primary prefixes need no second sort","sparse":"Persistent patches preserve untouched rows"},"executions":self.executions.iter().enumerate().map(|(i,run)|json!({"execution":i,"logical":run.emitted.analysis.as_ref(),"retained":run.reductions.iter().flat_map(|states|states.values()).map(reductions::Retained::explain).collect::<Vec<_>>(),"recursive":run.temporal.values().map(temporal::Trace::explain).collect::<Vec<_>>(),"opaque_sql_producers":run.producers.keys().collect::<Vec<_>>()})).collect::<Vec<_>>()})
    }
}
