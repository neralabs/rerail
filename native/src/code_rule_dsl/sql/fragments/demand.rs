//! SQL constructs one immutable demand relation. Only its handle crosses the matcher boundary.
use super::{Cell, Request, invalid};
use crate::code_rule_dsl::sql::plan::SlotKind;
use rusqlite::{
    Connection, Result,
    functions::{Aggregate, Context, FunctionFlags},
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, Ordering},
    },
};

#[derive(Default)]
pub(super) struct Demand {
    pub requests: Vec<Request>,
    pub identities: Vec<i64>,
    bytes: usize,
}
#[derive(Default)]
pub(in crate::code_rule_dsl::sql) struct DemandStore {
    next: AtomicI64,
    values: Mutex<BTreeMap<i64, Arc<Demand>>>,
}
impl DemandStore {
    pub(super) fn take(&self, handle: i64) -> Result<Arc<Demand>> {
        self.values
            .lock()
            .unwrap()
            .remove(&handle)
            .ok_or_else(|| invalid("expired or foreign demand handle"))
    }
    pub(in crate::code_rule_dsl::sql) fn clear(&self) {
        self.values.lock().unwrap().clear();
    }
    pub(in crate::code_rule_dsl::sql) fn register(
        self: &Arc<Self>,
        db: &Connection,
        name: &str,
        inputs: Vec<(String, SlotKind)>,
    ) -> Result<()> {
        db.create_aggregate_function(
            name,
            (3 + inputs.len()) as i32,
            FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DIRECTONLY,
            Collect {
                store: self.clone(),
                inputs,
            },
        )
    }
}
struct Collect {
    store: Arc<DemandStore>,
    inputs: Vec<(String, SlotKind)>,
}
impl Aggregate<Demand, i64> for Collect {
    fn init(&self, _: &mut Context<'_>) -> Result<Demand> {
        Ok(Demand::default())
    }
    fn step(&self, ctx: &mut Context<'_>, demand: &mut Demand) -> Result<()> {
        let identity = ctx.get(0)?;
        let file = ctx.get(1)?;
        let node = ctx.get(2)?;
        let mut inputs = Vec::with_capacity(self.inputs.len());
        for (i, (name, kind)) in self.inputs.iter().enumerate() {
            let value: Cell = ctx.get(i + 3)?;
            demand.bytes += match &value {
                Cell::List(v) | Cell::Span(v) => v.len(),
                _ => 8,
            } + name.len();
            inputs.push((name.clone(), *kind, value));
        }
        if demand.requests.len() >= 1_000_000 || demand.bytes > 128 * 1024 * 1024 {
            return Err(invalid("capture demand exceeds transport budget"));
        }
        demand.identities.push(identity);
        demand.requests.push(Request { file, node, inputs });
        Ok(())
    }
    fn finalize(&self, _: &mut Context<'_>, demand: Option<Demand>) -> Result<i64> {
        let demand = demand.unwrap_or_default(); // Empty aggregate is an empty batch, never a scan.
        let mut unique = std::collections::BTreeSet::new();
        if demand.identities.iter().any(|id| !unique.insert(*id)) {
            return Err(invalid("duplicate request identity"));
        }
        let handle = self
            .store
            .next
            .fetch_add(1, Ordering::Relaxed)
            .checked_add(1)
            .ok_or_else(|| invalid("demand handle overflow"))?;
        self.store
            .values
            .lock()
            .unwrap()
            .insert(handle, Arc::new(demand));
        Ok(handle)
    }
}
