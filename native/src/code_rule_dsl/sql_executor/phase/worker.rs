//! Turn total checked transitions into read-only groups; publish their roots on the owner.
use super::super::work_graph::{self, Node, Request};
use super::*;
pub mod references;

pub fn evaluate(template: &Template, w: &Work<'_>) -> Result<Option<Buffers>> {
    if !template.checked_total
        || template.owner_only
        || runtime::limited()
        || rerail_accel::exec::threads() < 2
    {
        return Ok(None);
    }
    if template.width > 400 {
        return Ok(None);
    }
    let mut nodes = vec![];
    let mut sources = BTreeMap::<String, Arc<dyn Source>>::new();
    let definitions = (0..template.definitions.len())
        .map(|i| format!("__rr_phase_{i}"))
        .collect::<BTreeSet<_>>();
    let mut add = |name: String, sql: String, width, materialized| -> Option<()> {
        let refs = references::read(&sql)?;
        let mut reads = vec![];
        for name in refs {
            if !definitions.contains(&name) && !sources.contains_key(&name) {
                let source = super::super::super::facts::vtab::bound_source(w.db, &name)?;
                sources.insert(name.clone(), source.frozen().unwrap_or(source));
            }
            reads.push(name);
        }
        nodes.push(Node {
            name,
            sql,
            width,
            reads,
            materialized,
        });
        Some(())
    };
    for (id, d) in template.definitions.iter().enumerate() {
        if add(
            format!("__rr_phase_{id}"),
            d.query().to_owned(),
            d.width,
            d.materialized,
        )
        .is_none()
        {
            return Ok(None);
        }
    }
    let mut outputs = vec![];
    for (tag, query) in template.reads.queries.iter().enumerate() {
        let name = format!("__rr_read_{tag}");
        if add(name.clone(), query.clone(), template.width, true).is_none() {
            return Ok(None);
        }
        outputs.push(name);
    }
    // The existing bulk-publication threshold also bounds the minimum work that amortizes
    // connection setup and result transport. Small transitions keep cached owner preparation.
    if sources
        .values()
        .map(|s| s.estimated_len())
        .max()
        .unwrap_or(0)
        < 512
    {
        return Ok(None);
    }
    let allowance = super::super::super::sql_admission::current()
        .map_or(4, |l| l.available().saturating_sub(16).min(4));
    if allowance < 2 {
        return Ok(None);
    }
    let request = Request {
        nodes,
        outputs,
        max_submissions: allowance,
    };
    let Ok(plan) =
        work_graph::planner::Plan::new(&request, &sources, rerail_accel::exec::threads())
    else {
        return Ok(None);
    };
    if plan.groups.len() < 2 {
        return Ok(None);
    }
    let _context=w.profile_scope(||json!({"stage":"checked rule SQL workers","proof":"Total checked support transitions; immutable native bindings; atomic owner publication"}));
    let result = work_graph::execute_planned(
        request,
        sources,
        &runtime::Limits::default(),
        w.profiler,
        w.pool.worker_accounting.clone(),
        plan,
    )?;
    let mut explain = result.explain;
    explain["submissions"] = json!(result.submissions);
    explain["vm_steps"] = json!(result.vm_steps);
    *w.pool.worker_plan.lock() = Some(explain);
    let mut buffers = template.buffers();
    for (name, rows) in result.rows {
        let tag = name
            .strip_prefix("__rr_read_")
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or_else(|| error("invalid checked SQL output"))?;
        for cells in &rows.rows {
            template.collect(tag, cells.clone(), &mut buffers)?;
        }
    }
    Ok(Some(buffers))
}
