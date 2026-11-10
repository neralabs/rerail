//! Execute a bounded SQL suffix and install its retained trajectory.
use super::*;
use crate::code_rule_dsl::facts::vtab::Modules;
use rusqlite::functions::FunctionFlags;

pub(super) fn run(
    trace: &mut Trace,
    w: &mut Work<'_>,
    store: &versions::Store,
    limit: u32,
    mut frame: Frame,
    pending: BTreeSet<usize>,
    waves: u32,
    diagnostic_rows: &mut Vec<Vec<SqlValue>>,
) -> Result<Option<u32>> {
    let prefix_frames = trace.frames.len();
    let prefix_phases = trace.frames.iter().map(|f| f.phase_batches).sum::<usize>();
    let initialized = frame.operators.iter().map(|op| op.ready).collect();
    let ran = frame
        .operators
        .iter()
        .enumerate()
        .filter(|(_, op)| op.active)
        .map(|(id, _)| id)
        .collect();
    frame.composed = false;
    let uncompiled = frame.clone();
    let compiled = compile::build(trace, &mut frame, w, store, limit)?;
    let Some((plan, roots)) = compiled else {
        // Compilation can declare empty support roots before its size is known.
        // Keep the original ownership list, so normal initialization owns each root once.
        trace.frames.push(uncompiled);
        return Ok(None);
    };
    trace.frames.push(frame);
    let active = plan.ready(&pending);
    let width = roots.iter().map(|v| v.columns().len()).max().unwrap().max(
        plan.stages
            .iter()
            .chain([&plan.fold])
            .map(|s| s.template.width())
            .max()
            .unwrap(),
    );
    let arena = Arc::new(parking_lot::Mutex::new(Arena {
        plan: plan.clone(),
        states: vec![State {
            roots,
            changed: vec![],
            initialized,
            empty_body: vec![false; trace.steps.len()],
            pending,
            active,
            ran,
            waves,
            publication: json!([]),
        }],
        private: vec![],
        failure: None,
        storage_cpu_ns: Some(0),
        shared_roots: 0,
        retained_roots: 0,
        difference_state: None,
        differences: BTreeMap::new(),
        version_nodes: 0,
        context_rows: 0,
        difference_cpu_ns: Some(0),
    }));
    let modules = Modules::default();
    let mut vm_steps = 0u64;
    let result = (|| {
        modules
            .bind(w.db, ROWS, Arc::new(rows::Rows(arena.clone(), width)))
            .map_err(error)?;
        w.db.create_aggregate_function(
            COLLECT,
            -1,
            FunctionFlags::SQLITE_UTF8,
            storage::Collect(arena.clone()),
        )
        .map_err(error)?;
        let gates = arena.clone();
        w.db.create_scalar_function(GATE, 3, FunctionFlags::SQLITE_UTF8, move |c| {
            let arena = gates.lock();
            let s = arena.state(c.get(0)?)?;
            let id = usize::try_from(c.get::<i64>(1)?)
                .map_err(|_| sql_error("invalid retained operator"))?;
            Ok(s.active.contains(&id)
                && (c.get::<i64>(2)? == 0
                    || !s
                        .initialized
                        .get(id)
                        .copied()
                        .ok_or_else(|| sql_error("invalid retained operator"))?))
        })
        .map_err(error)?;
        let empty = arena.clone();
        w.db.create_scalar_function(EMPTY, 2, FunctionFlags::SQLITE_UTF8, move |c| {
            let slot = usize::try_from(c.get::<i64>(1)?)
                .map_err(|_| sql_error("invalid retained input"))?;
            Ok(empty.lock().root(c.get(0)?, slot)?.len() == 0)
        })
        .map_err(error)?;
        let clear = arena.clone();
        w.db.create_scalar_function(CLEAR, 2, FunctionFlags::SQLITE_UTF8, move |c| {
            let id = usize::try_from(c.get::<i64>(1)?)
                .map_err(|_| sql_error("invalid retained operator"))?;
            clear.lock().clear(c.get(0)?, id)
        })
        .map_err(error)?;
        let merge = arena.clone();
        w.db.create_scalar_function(MERGE, -1, FunctionFlags::SQLITE_UTF8, move |c| {
            let parent = c.get::<i64>(0)?;
            let handles = (1..c.len())
                .map(|i| c.get::<i64>(i))
                .collect::<rusqlite::Result<Vec<_>>>()?;
            merge.lock().merge(parent, &handles)
        })
        .map_err(error)?;
        let live = arena.clone();
        w.db.create_scalar_function(LIVE, 1, FunctionFlags::SQLITE_UTF8, move |c| {
            live.lock().live(c.get(0)?)
        })
        .map_err(error)?;
        let _context = w.profile_scope(
            || json!({"stage":"retained SQL version loop","sqlWorkPlan":plan.explain}),
        );
        crate::code_rule_dsl::logic::lanes::sql_executor().local(
            rerail_accel::Lane::Sql,
            "SQL retained version loop",
            0.0,
            || {
                let mut statement = w.db.prepare(&plan.sql).map_err(error)?;
                let mut rows = statement.query([]).map_err(error)?;
                while let Some(row) = rows.next().map_err(error)? {
                    let _: i64 = row.get(0).map_err(error)?;
                }
                drop(rows);
                vm_steps = statement
                    .get_status(rusqlite::StatementStatus::VmStep)
                    .max(0) as u64;
                Ok::<_, napi::Error>(())
            },
        )?;
        Ok::<_, napi::Error>(())
    })();
    if let Some(profiler) = w.profiler {
        profiler.explain_pending(w.db);
    }
    let cleanup = [
        modules.unregister(w.db, ROWS),
        w.db.remove_function(COLLECT, -1),
        w.db.remove_function(GATE, 3),
        w.db.remove_function(LIVE, 1),
        w.db.remove_function(MERGE, -1),
        w.db.remove_function(EMPTY, 2),
        w.db.remove_function(CLEAR, 2),
    ]
    .into_iter()
    .collect::<rusqlite::Result<Vec<_>>>()
    .map_err(error);
    result?;
    cleanup?;
    let arena = arena.lock();
    if let Some(failure) = &arena.failure {
        return Err(error(failure));
    }
    let mut explanation = plan.explain.clone();
    explanation["prefix_frames"] = json!(prefix_frames);
    explanation["prefix_sql_phases"] = json!(prefix_phases);
    explanation["host_phase_limit"] = json!(HOST_PHASE_LIMIT);
    explanation["vm_steps"] = json!(vm_steps);
    explanation["iterations_including_closing"] = json!(arena.states.len() - 1);
    explanation["storage_cpu_ns"] = json!(arena.storage_cpu_ns);
    w.context_rows += arena.context_rows;
    explanation["context_rows"] = json!(arena.context_rows);
    explanation["version_nodes"] = json!(arena.version_nodes);
    explanation["difference_cpu_ns"] = json!(arena.difference_cpu_ns);
    w.version_nodes += arena.version_nodes;
    explanation["shared_roots"] = json!(arena.shared_roots);
    explanation["retained_nonempty_roots"] = json!(arena.retained_roots);
    let base = trace.frames.pop().unwrap();
    for (iteration, state) in arena.states.iter().enumerate().skip(1) {
        let mut frame = if iteration == 1 {
            base.clone()
        } else {
            trace.frames.last().unwrap().fork(store)?
        };
        frame.pipeline.clear();
        let _scope = store.activate(frame.scope())?;
        for &slot in &state.changed {
            if let Some(name) = plan.targets.get(&slot) {
                store.replace(name, state.roots[slot].clone())?;
            }
        }
        for (id, op) in frame.operators.iter_mut().enumerate() {
            op.ready = state.initialized[id];
            op.active = state.ran.contains(&id);
            op.empty_body = state.empty_body[id];
            op.comparison = state.roots[plan.operators[id].comparison.0].clone();
            for (input, slot, _) in &plan.operators[id].seen {
                op.seen.insert(input.clone(), state.roots[*slot].clone());
            }
            if let Some(states) = &mut op.run.reductions {
                for state in states.values_mut() {
                    state.ready = true;
                }
            }
            op.run
                .counts
                .insert(op.result.clone(), store.version(&op.result).len() as u64);
        }
        frame.composed = true;
        frame.phase_batches = usize::from(iteration == 1);
        frame.publication_decisions = state.publication.as_array().unwrap().clone();
        frame.observation = json!({"purpose":if state.ran.is_empty(){"closing"}else{"SQL recursive trajectory"},
            "scheduled_steps":state.ran.len(),"reused":false,"sql_phases":frame.phase_batches,
            "sql_version_loop":if iteration==1 {Some(&explanation)}else{None},"iteration":iteration,"recursive_components":[]});
        for (id, template) in frame.prepared_missing(store, &trace.templates) {
            Arc::make_mut(&mut trace.templates)
                .entry(id)
                .or_insert(template);
        }
        let previous = &arena.states[iteration - 1];
        if !state.ran.is_empty() {
            let written = state
                .ran
                .iter()
                .map(|&id| trace.steps[id].0.output.as_str())
                .collect::<BTreeSet<_>>();
            for (r, &slot) in &plan.frontiers {
                if written.contains(r.as_str()) || previous.roots[slot].len() > 0 {
                    diagnostic_rows.push(vec![
                        SqlValue::Integer(state.waves as i64),
                        SqlValue::Text(r.clone()),
                        SqlValue::Integer(state.roots[slot].len() as i64),
                    ]);
                }
            }
        }
        trace.frames.push(frame);
    }
    Ok(Some(arena.states.last().unwrap().waves))
}
