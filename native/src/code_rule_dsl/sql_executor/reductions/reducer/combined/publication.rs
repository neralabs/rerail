//! Stage SQL-computed patches for compact SQLite summaries. The caller owns the batch transaction.
//! Host code publishes rows; it never evaluates a rule or aggregate.
use super::*;
use crate::code_rule_dsl::sql_executor::phase::{Counts, Phase};

pub(super) struct Publication {
    table: String,
    keys: String,
    rows: String,
    key_width: usize,
}

/// Both queries have unique prefix keys; every returned row belongs to the selected key set.
pub(super) fn append(
    w: &mut Work<'_>,
    phase: &mut Phase,
    pending: &mut Vec<Publication>,
    table: &str,
    keys: &str,
    rows: &str,
    key_width: usize,
    width: usize,
) -> Result<()> {
    let store = w.versions.unwrap();
    if store.available(table).is_some() {
        return phase.patch_computed(store, table, keys, rows);
    }
    let key_rows = w.output_table(key_width)?;
    let values = w.allocate_readonly(format!(
        "{},PRIMARY KEY({})",
        schema(width),
        names("", key_width)
    ))?;
    let selected = phase.view(
        key_width,
        &format!("SELECT {} FROM {keys}", names("", key_width)),
    );
    phase.load_empty(store, &key_rows, &selected)?;
    phase.load_empty(store, &values, rows)?;
    pending.push(Publication {
        table: table.into(),
        keys: key_rows,
        rows: values,
        key_width,
    });
    Ok(())
}

pub(super) fn publish(
    w: &Work<'_>,
    pending: &[Publication],
    counts: &BTreeMap<String, Counts>,
) -> Result<()> {
    let _context = w.profile_scope(|| json!({"stage":"publish shared SQL summaries"}));
    for p in pending {
        let key_count = counts[&p.keys].rows;
        let row_count = counts[&p.rows].rows;
        // Uniqueness plus containment proves that a complete patch needs no deletion.
        if key_count > row_count {
            execute(
                w.db,
                &format!(
                    "DELETE FROM {} AS t WHERE ({}) IN (SELECT {} FROM {}) AND NOT EXISTS(SELECT 1 FROM {} s WHERE {})",
                    quote(&p.table),
                    names("t", p.key_width),
                    names("", p.key_width),
                    quote(&p.keys),
                    quote(&p.rows),
                    equal("s", "t", p.key_width)
                ),
            )?;
        }
        if row_count > 0 {
            execute(
                w.db,
                &format!(
                    "INSERT OR REPLACE INTO {} SELECT * FROM {}",
                    quote(&p.table),
                    quote(&p.rows)
                ),
            )?;
        }
    }
    Ok(())
}
