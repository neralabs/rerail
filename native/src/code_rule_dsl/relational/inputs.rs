//! Stream sealed projection columns into batched SQLite writes. Keep only diagnostic metadata.
use super::{FactQuery, Rows, insert};
use crate::code_rule_dsl::{
    eval,
    model::{CompiledClause, CompiledCodeRule, Metrics, Snapshot},
    violations::{Witness, anchored_span, projection::Row},
};
use napi::Result;
use rusqlite::{Connection, types::ValueRef};
use std::{
    cell::RefCell,
    sync::{Arc, atomic::AtomicUsize},
};

#[cfg(test)]
mod tests;

struct Pending {
    path: Arc<str>,
    start: i64,
    end: i64,
    projection: Option<Row>,
}
struct Sink<'a> {
    db: &'a Connection,
    table: &'a str,
    input: &'a FactQuery,
    work: &'a Arc<AtomicUsize>,
    pending: Vec<Pending>,
    path: Arc<str>,
    error: Option<napi::Error>,
    rows: usize,
    peak: usize,
    batches: usize,
}
impl Sink<'_> {
    fn flush(&mut self) -> Result<()> {
        let published = self
            .pending
            .iter()
            .take_while(|p| p.projection.as_ref().is_none_or(Row::published))
            .count();
        if published == 0 {
            return Ok(());
        }
        insert::insert_rows(
            self.db,
            self.table,
            self.input.project.len() + 3,
            self.pending[..published].iter().map(|p| {
                [
                    ValueRef::Text(p.path.as_bytes()),
                    ValueRef::Integer(p.start),
                    ValueRef::Integer(p.end),
                ]
                .into_iter()
                .chain((0..self.input.project.len()).map(|i| {
                    p.projection
                        .as_ref()
                        .map_or(ValueRef::Null, |row| row.sql(i))
                }))
                .map(insert::Borrowed)
            }),
            self.work,
            "relational input row budget exceeded",
        )?;
        self.pending.drain(..published);
        self.batches += 1;
        Ok(())
    }
    fn emit(&mut self, mut witness: Witness) -> Witness {
        // The matcher completes the current column chunk before calling this sink.
        // SQLite errors invalidate this entire session, including already inserted batches.
        let projection = witness.captures.take_projection();
        if self.error.is_some() {
            return witness;
        }
        if self.path.as_ref() != witness.path {
            self.path = Arc::from(witness.path.as_str());
        }
        let location = anchored_span(&self.input.anchor, &witness).0;
        self.pending.push(Pending {
            path: Arc::clone(&self.path),
            start: location.start_byte as i64,
            end: location.end_byte as i64,
            projection,
        });
        self.rows += 1;
        self.peak = self.peak.max(self.pending.len());
        if (self.pending.len() >= 256
            || self
                .pending
                .first()
                .is_some_and(|p| p.projection.as_ref().is_some_and(Row::published)))
            && let Err(e) = self.flush()
        {
            self.error = Some(e);
        }

        witness
    }
}
#[allow(clippy::too_many_arguments)]
pub(super) fn stream(
    db: &Connection,
    table: &str,
    input: &FactQuery,
    work: &Arc<AtomicUsize>,
    code_rule: &CompiledCodeRule,
    clause: &CompiledClause,
    snapshot: &Snapshot,
    metrics: &mut Metrics,
) -> Result<Rows> {
    #[cfg(feature = "code-rule-profile")]
    let mut trace = crate::code_rule_dsl::profile::trace::Span::start("input-stream", table);
    let mut sink = Sink {
        db,
        table,
        input,
        work,
        pending: Vec::new(),
        path: Arc::from(""),
        error: None,
        rows: 0,
        peak: 0,
        batches: 0,
    };
    let mut output = |w| sink.emit(w);
    let rows = eval::stateless_rows_to(
        code_rule,
        clause,
        (input, eval::evidence::Side::Counted),
        snapshot,
        metrics,
        Some(&RefCell::new(&mut output)),
    )?;
    if let Some(e) = sink.error.take() {
        return Err(e);
    }
    sink.flush()?;
    if !sink.pending.is_empty() {
        return Err(super::invalid("unpublished relational input columns"));
    }
    #[cfg(feature="code-rule-profile")]
    trace.detail(|| serde_json::json!({"rows":sink.rows,"peakBufferedRows":sink.peak,"columnBatches":sink.batches,"retainedProjectionRows":rows.witnesses.iter().filter(|w|w.captures.projection().is_some()).count(),"incompleteFiles":rows.incomplete.len()}));
    Ok(rows)
}
