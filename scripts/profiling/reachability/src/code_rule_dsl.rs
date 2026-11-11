//! The engine's `code_rule_dsl` paths the SQL tracer names, around the production source.
// The engine's source keeps its own layout.
#[rustfmt::skip]
#[allow(dead_code)]
#[path = "../../../../native/src/code_rule_dsl/profile/sql.rs"]
mod sql;

/// The engine's run profile, as when it is off: the tracer writes only its own capture.
mod logic {
    pub(in crate::code_rule_dsl) mod profile {
        pub(in crate::code_rule_dsl) fn on() -> bool {
            false
        }
        pub(in crate::code_rule_dsl) fn directory() -> Option<std::path::PathBuf> {
            None
        }
        pub(in crate::code_rule_dsl) fn note(_key: String, _text: impl FnOnce() -> String) {}
        pub(in crate::code_rule_dsl) fn normal(sql: &str) -> String {
            sql.split_whitespace().collect::<Vec<_>>().join(" ")
        }
        pub(in crate::code_rule_dsl) fn sql(
            _text: String,
            _ns: u64,
            _vm: u64,
            _scan: u64,
            _sort: u64,
            _autoindex: u64,
        ) {
        }
    }
}
