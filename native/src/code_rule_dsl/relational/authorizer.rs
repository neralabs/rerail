//! The relational query authorizer, shared by compilation and the scoped-reader contract test.
use rusqlite::{
    Connection, Result,
    hooks::{AuthAction, Authorization},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) struct Reads {
    pub fields: Arc<parking_lot::Mutex<BTreeMap<String, BTreeSet<String>>>>,
    pub source: Arc<AtomicBool>,
}
pub(super) fn install(db: &Connection, tables: &BTreeSet<String>) -> Result<Reads> {
    let tables = tables.clone();
    let reads = Arc::new(parking_lot::Mutex::new(
        BTreeMap::<String, BTreeSet<String>>::new(),
    ));
    let observed = Arc::clone(&reads);
    let source_read = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let source_observed = Arc::clone(&source_read);
    db.authorizer(Some(
        move |context: rusqlite::hooks::AuthContext<'_>| match context.action {
            AuthAction::Select | AuthAction::Recursive => Authorization::Allow,
            AuthAction::Read {
                table_name,
                column_name,
                ..
            } if tables.contains(table_name) || matches!(table_name, "json_each" | "json_tree") => {
                if tables.contains(table_name) {
                    observed
                        .lock()
                        .entry(table_name.to_owned())
                        .or_default()
                        .insert(column_name.to_owned());
                }
                Authorization::Allow
            }
            // A generated reachability view exposes only its public columns. SQLite
            // reports its backing reads with an accessor label. CTEs can receive
            // the same label: do not extend this legacy exception to shared owners.
            AuthAction::Read { table_name, .. }
                if context.accessor.is_some_and(|view| {
                    tables.contains(view)
                        && (table_name == format!("__stage_{view}")
                            || ["keys", "pairs", "ranges", "positions"]
                                .iter()
                                .any(|suffix| table_name == format!("__reach_{view}_{suffix}")))
                }) =>
            {
                Authorization::Allow
            }
            AuthAction::Function { function_name }
                if matches!(
                    function_name.to_ascii_lowercase().as_str(),
                    "abs"
                        | "char"
                        | "cast"
                        | "coalesce"
                        | "count"
                        | "dense_rank"
                        | "group_concat"
                        | "hex"
                        | "ifnull"
                        | "iif"
                        | "instr"
                        | "json"
                        | "json_array"
                        | "json_array_length"
                        | "json_each"
                        | "json_extract"
                        | "json_group_array"
                        | "json_group_object"
                        | "json_object"
                        | "json_quote"
                        | "json_tree"
                        | "json_type"
                        | "json_valid"
                        | "length"
                        | "like"
                        | "glob"
                        | "lower"
                        | "ltrim"
                        | "max"
                        | "min"
                        | "nullif"
                        | "printf"
                        | "quote"
                        | "rank"
                        | "replace"
                        | "row_number"
                        | "rtrim"
                        | "substr"
                        | "substring"
                        | "sum"
                        | "total"
                        | "trim"
                        | "typeof"
                        | "unicode"
                        | "upper"
                        | "rx_matches"
                        | "rx_extract"
                        | "rx_replace"
                        | "source_text"
                ) =>
            {
                if function_name.eq_ignore_ascii_case("source_text") {
                    source_observed.store(true, Ordering::Relaxed);
                }
                Authorization::Allow
            }
            _ => Authorization::Deny,
        },
    ))?;
    Ok(Reads {
        fields: reads,
        source: source_read,
    })
}
