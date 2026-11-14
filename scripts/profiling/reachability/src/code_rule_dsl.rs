//! The engine's `code_rule_dsl` paths the SQL tracer names, around the production source.
// The engine's source keeps its own layout.
#[rustfmt::skip]
#[allow(dead_code, unused_imports)]
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

/// The engine's admission ledger, as when no run admits statements: nothing observes the tracer's.
mod sql_admission {
    pub(in crate::code_rule_dsl) mod observer {
        use rusqlite::ffi::sqlite3_stmt;

        pub enum Observed {}

        impl Observed {
            pub(crate) fn start(&self, _statement: *mut sqlite3_stmt) {
                match *self {}
            }
            pub(crate) fn end(&self, _statement: *mut sqlite3_stmt) {
                match *self {}
            }
        }
    }
}
