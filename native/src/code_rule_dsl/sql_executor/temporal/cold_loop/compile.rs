//! Compile a bounded retained SQL transition against explicit iteration roots.
use super::*;

fn slot(
    name: &str,
    root: versions::Version,
    slots: &mut BTreeMap<String, usize>,
    roots: &mut Vec<versions::Version>,
) -> usize {
    *slots.entry(name.into()).or_insert_with(|| {
        let id = roots.len();
        roots.push(root);
        id
    })
}
pub(super) fn build(
    trace: &Trace,
    frame: &mut Frame,
    w: &mut Work<'_>,
    store: &versions::Store,
    limit: u32,
) -> Result<Option<(Arc<Plan>, Vec<versions::Version>)>> {
    if frame.operators.iter().any(|op| {
        !op.phase_eligible
            || op.sql_kernel
            || op.opaque.is_some()
            || op
                .run
                .reductions
                .as_ref()
                .is_none_or(|states| states.values().any(|s| !s.version_loop_eligible()))
    }) {
        return Ok(None);
    }
    let _scope = store.activate(frame.scope())?;
    for op in &mut frame.operators {
        let state = op
            .run
            .reductions
            .as_mut()
            .unwrap()
            .get_mut(&op.result)
            .unwrap();
        if !op.ready {
            state.declare_phase(w.db, &mut op.run.objects, store)?;
        }
    }
    let mut slots = BTreeMap::new();
    let mut roots = vec![];
    for (name, _) in frame.objects() {
        slot(&name, store.version(&name), &mut slots, &mut roots);
    }
    let targets = slots.iter().map(|(name, &id)| (id, name.clone())).collect();
    let mut current = slots.clone();
    let mut inputs = vec![];
    for (logical, name) in &frame.inputs {
        let source = if let Some(r) = logical.strip_prefix("delta_") {
            &frame.frontiers[r]
        } else {
            &frame.published[logical]
        };
        current.insert(name.clone(), slots[source]);
        inputs.push((slots[name], slots[source]));
    }
    let mut aliases = slots
        .iter()
        .map(|(name, &id)| (name.clone(), current[name], roots[id].columns()))
        .collect::<Vec<_>>();
    let mut stages = vec![];
    let mut differences = BTreeMap::new();
    let contexts = frame
        .operators
        .iter()
        .flat_map(|op| op.run.reductions.as_ref().unwrap().values().cloned())
        .collect();
    let mut empty_guards = vec![];
    let mut operators = vec![];
    let mut improvements = BTreeMap::<String, Vec<String>>::new();
    for (id, op) in frame.operators.iter_mut().enumerate() {
        let mut phase = phase::Phase::default();
        let gate = format!("{GATE}(__rr_parent.state,{id},0)");
        let first = format!("{GATE}(__rr_parent.state,{id},1)");
        let mut changed = BTreeMap::new();
        let mut seen = vec![];
        for input in &op.run.inputs {
            // During a cold positive set loop, base relations only gain rows and
            // external facts stay frozen. Generated frontier inputs can lose rows.
            let removable = frame
                .inputs
                .iter()
                .any(|(logical, table)| table == input && logical.starts_with("delta_"));
            let version = store.available(input);
            let columns = if let Some(v) = &version {
                v.columns()
            } else {
                w.db.prepare(&format!("SELECT * FROM {} LIMIT 0", quote(input)))
                    .map_err(error)?
                    .column_names()
                    .map(str::to_owned)
                    .collect()
            };
            let fields = columns
                .iter()
                .map(|c| quote(c))
                .collect::<Vec<_>>()
                .join(",");
            let (plus, minus) = if let Some(version) = version {
                let now = slot(input, version.clone(), &mut slots, &mut roots);
                let current_slot = *current.entry(input.clone()).or_insert(now);
                if !aliases.iter().any(|(name, _, _)| name == input) {
                    aliases.push((input.clone(), current_slot, columns.clone()));
                }
                let name = format!("__rr_loop_seen_{id}_{}", seen.len());
                let previous = slot(
                    &name,
                    op.seen
                        .get(input)
                        .cloned()
                        .unwrap_or_else(|| version.empty_like()),
                    &mut slots,
                    &mut roots,
                );
                aliases.push((name.clone(), previous, columns.clone()));
                seen.push((input.clone(), previous, current_slot));
                let mut sources = vec![];
                for negative in [false, true] {
                    let source =
                        format!("__rr_loop_diff_{id}_{}_{}", seen.len(), u8::from(negative));
                    let difference = slot(&source, version.empty_like(), &mut slots, &mut roots);
                    differences.insert(difference, (current_slot, previous, negative));
                    aliases.push((source.clone(), difference, columns.clone()));
                    sources.push(format!(
                        "SELECT {fields} FROM {} WHERE {gate}",
                        quote(&source)
                    ));
                }
                (sources.remove(0), sources.remove(0))
            } else {
                (
                    format!("SELECT {fields} FROM {} WHERE {first}", quote(input)),
                    format!("SELECT {} WHERE 0", vec!["NULL"; columns.len()].join(",")),
                )
            };
            changed.insert(
                input.clone(),
                Delta {
                    plus: phase.relation(columns.len(), &plus),
                    minus: phase.relation(columns.len(), &minus),
                    inserted: 1,
                    deleted: u64::from(removable),
                },
            );
        }
        let (step, _) = &trace.steps[id];
        let width = trace.relations[&step.output].width;
        let state = &op.run.reductions.as_ref().unwrap()[&op.result];
        let empty = state
            .empty_positive_inputs()
            .iter()
            .map(|source| {
                if let Some(slot) = current.get(source) {
                    format!("{EMPTY}(__rr_parent.state,{slot})")
                } else {
                    format!("NOT EXISTS(SELECT 1 FROM {})", quote(source))
                }
            })
            .collect::<Vec<_>>();
        empty_guards.push(if empty.is_empty() {
            "0".to_owned()
        } else {
            empty.join(" OR ")
        });
        let (_, next) = state.append_phase(w, &changed, &mut phase, false)?;
        let improved = phase.relation(
            width,
            &format!(
                "SELECT n.* FROM {next} n WHERE {gate} AND NOT EXISTS(SELECT 1 FROM {} p WHERE {})",
                quote(&frame.inputs[&step.output]),
                same("p", "n", width)
            ),
        );
        phase.replace_all(store, &op.improvements, &improved)?;
        improvements
            .entry(step.output.clone())
            .or_default()
            .push(format!(
                "SELECT * FROM {} WHERE {gate}",
                quote(&op.improvements)
            ));
        let comparison = slot(
            &format!("__rr_loop_comparison_{id}"),
            op.comparison.clone(),
            &mut slots,
            &mut roots,
        );
        stages.push(compiled(phase, &slots)?);
        operators.push(Operator {
            seen,
            comparison: (comparison, current[&frame.inputs[&step.output]]),
        });
    }
    let mut phase = phase::Phase::default();
    for (r, spec) in &trace.relations {
        let queries = improvements
            .get(r)
            .into_iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        let query = if queries.is_empty() {
            format!("SELECT {} WHERE 0", vec!["NULL"; spec.width].join(","))
        } else {
            queries.join(" UNION ")
        };
        let delta = phase.relation(spec.width, &query);
        phase.replace_all(store, &frame.frontiers[r], &delta)?;
        phase.insert(store, &frame.published[r], &delta, true)?;
    }
    let fold = compiled(phase, &slots)?;
    let expansion = stages
        .iter()
        .chain([&fold])
        .map(|s| s.expansion)
        .sum::<usize>();
    let arms = stages.iter().chain([&fold]).map(|s| s.arms).sum::<usize>();
    if arms > 400 || expansion > phase::Phase::EXPANSION_BUDGET {
        return Ok(None);
    }
    let width = roots
        .iter()
        .map(|v| v.columns().len())
        .max()
        .unwrap_or(0)
        .max(
            stages
                .iter()
                .chain([&fold])
                .map(|s| s.template.width())
                .max()
                .unwrap(),
        );
    if width > 120 {
        return Ok(None);
    }
    let bindings = |parent: &str| {
        aliases.iter().map(|(name,id,columns)|format!("{}({}) AS NOT MATERIALIZED (SELECT {} FROM {ROWS} WHERE rr_state={parent} AND rr_slot={id})",quote(name),columns.iter().map(|c|quote(c)).collect::<Vec<_>>().join(","),names("",columns.len()))).collect::<Vec<_>>()
    };
    let mut work=stages.iter().enumerate().map(|(id,stage)|format!("__rr_work_{id} AS MATERIALIZED (SELECT CASE WHEN ({}) THEN {CLEAR}(__rr_parent.state,{id}) WHEN {GATE}(__rr_parent.state,{id},0) THEN {} ELSE __rr_parent.state END AS state)",empty_guards[id],transition(stage,id as i64,"__rr_parent.state",&bindings("__rr_parent.state")))).collect::<Vec<_>>();
    let handles = (0..stages.len())
        .map(|id| format!("(SELECT state FROM __rr_work_{id})"))
        .collect::<Vec<_>>()
        .join(",");
    work.push(format!(
        "__rr_merged AS MATERIALIZED (SELECT {MERGE}(__rr_parent.state,{handles}) AS state)"
    ));
    let transition = format!(
        "(WITH {} SELECT {} FROM __rr_merged)",
        work.join(","),
        transition(
            &fold,
            -1,
            "__rr_merged.state",
            &bindings("__rr_merged.state")
        )
    );
    let sql=format!("WITH RECURSIVE __rr_retained_states(state) AS (VALUES(0) UNION ALL SELECT {transition} FROM __rr_retained_states __rr_parent WHERE {LIVE}(__rr_parent.state)) SELECT state FROM __rr_retained_states");
    let frontiers = frame
        .frontiers
        .iter()
        .map(|(r, name)| (r.clone(), slots[name]))
        .collect();
    let members = trace.relations.keys().cloned().collect();
    let analysis = plan::acyclic::version_loop(&trace.steps, &members, &BTreeSet::new());
    let explain = json!({"strategy":"retained SQL version loop","executor":"sql","planned_submissions":1,
        "placement":"owner SQL connection",
        "rule_stages":stages.iter().zip(trace.steps.iter()).enumerate().map(|(id,(stage,(step,_)))|json!({"id":id,"relation":step.output,"reads":step.reads,"output_arms":stage.arms,"dependency_expansion_cost":stage.expansion})).collect::<Vec<_>>(),
        "conditional_rule_stages":stages.len(),"root_slots":roots.len(),"width":width,"output_arms":arms,"dependency_expansion_cost":expansion,
        "logical_analysis":analysis,"proof":"Positive set caches with total checked expressions; SQL maintains contexts, contributions, support counts and results; frozen gates retain every intermediate frontier",
        "storage_callbacks":"Root construction, indexed access, immutable version differences, and frontier-length scheduling only; no nested SQL",
        "input_differences":"Shared persistent-index differences, cached per current and seen root pair within each iteration",
        "empty_context_proof":"SQL tests positive input emptiness before cache work; storage-only clearing advances its unprotected cursors",
        "cold_input_proof":"Positive set outputs only grow and external facts stay frozen; only generated frontier inputs require deletion branches",
        "scope":"Cold initialization; warm history replay retains its existing transitions"});
    Ok(Some((
        Arc::new(Plan {
            stages,
            fold,
            differences,
            targets,
            inputs,
            operators,
            contexts,
            frontiers,
            steps: trace.steps.clone(),
            after: trace.after.clone(),
            limit,
            sql,
            explain,
        }),
        roots,
    )))
}

fn compiled(phase: phase::Phase, slots: &BTreeMap<String, usize>) -> Result<Arc<Stage>> {
    let expansion = phase.expansion_cost().saturating_mul(4);
    let arms = phase.arm_bound();
    let (template, _) = phase.compile()?;
    let snapshots = template.inputs().iter().map(|name| slots[*name]).collect();
    let outputs = template.names().iter().map(|name| slots[*name]).collect();
    Ok(Arc::new(Stage {
        template,
        snapshots,
        outputs,
        expansion,
        arms,
    }))
}
fn transition(stage: &Stage, id: i64, parent: &str, bindings: &[String]) -> String {
    let width = stage.template.width();
    let protocol = std::iter::once("rr_tag".into())
        .chain((0..width).map(|c| format!("c{c}")))
        .collect::<Vec<_>>()
        .join(",");
    let sentinel = format!("SELECT -1,{}", vec!["NULL"; width].join(","));
    format!("(WITH __rr_retained_feed({protocol}) AS ({}) SELECT {COLLECT}({id},{parent},rr_tag,{}) FROM (SELECT * FROM __rr_retained_feed UNION ALL {sentinel}))",stage.template.sql_with(bindings),names("",width))
}
