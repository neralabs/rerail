//! The SQL profiler (`sql_profile`: a statement's trace, plans and counts, `RERAIL_SQL_PROFILE_DIR`) and the SQL
//! admission ledger (`sql_admission`), crate split step 10c. They were facts-base's; only the crates that run SQL owners
//! use them (setfix's waves, the executor), so they sit above the facts, and an edit to them rebuilds those alone. The
//! counters the facts' virtual tables add to are rerail-prof's (`sql_frame`), which `sql_profile` re-exports.
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    pub mod logic {
        pub use rerail_prof::code_rule_dsl::logic::profile;
    }
    pub mod sql_admission;
    pub mod sql_profile;
}
