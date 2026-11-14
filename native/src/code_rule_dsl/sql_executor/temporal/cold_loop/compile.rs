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
pub fn build(
    trace: &Trace,
    frame: &mut Frame,
    w: &mut Work<'_>,
    store: &versions::Store,
    selected: &BTreeSet<usize>,
    limit: u32,
    preparing: bool,
) -> Result<Option<(Arc<Plan>, Vec<versions::Version>)>> {
    let _scope = store.activate(frame.scope())?;
    for &id in selected {
        let op = &mut frame.operators[id];
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
    for (r, name) in &frame.input_counters {
        let source = slots[&frame.counters[r]];
        current.insert(name.clone(), source);
        inputs.push((slots[name], source));
    }
    let mut aliases = slots
        .iter()
        .map(|(name, &id)| (name.clone(), current[name], roots[id].columns()))
        .collect::<Vec<_>>();
    let mut stages = BTreeMap::new();
    let mut differences = BTreeMap::new();
    let contexts = selected
        .iter()
        .flat_map(|&id| {
            frame.operators[id]
                .run
                .reductions
                .as_ref()
                .unwrap()
                .values()
                .cloned()
        })
        .collect();
    let mut empty_guards = BTreeMap::new();
    let mut operators = BTreeMap::new();
    let mut improvements = BTreeMap::<String, Vec<String>>::new();
    for &id in selected {
        let op = &frame.operators[id];
        let mut phase = phase::Phase::default();
        let gate = format!("{GATE}(__rr_parent.state,{id},0)");
        let first = format!("{GATE}(__rr_parent.state,{id},1)");
        let mut changed = BTreeMap::new();
        let mut seen = vec![];
        let mut journals = vec![];
        for input in &op.run.inputs {
            // During a cold positive set loop, base relations only gain rows and
            // external facts stay frozen. Generated frontier inputs can lose rows.
            let mut removable = !preparing
                || frame
                    .inputs
                    .iter()
                    .any(|(logical, table)| table == input && logical.starts_with("delta_"));
            let journal = op.pending.get(input);
            let version = if journal.is_none() {
                store.available(input)
            } else {
                None
            };
            let columns = if let Some(v) = &version {
                v.columns()
            } else if let Some(external) = store.available(input) {
                external.columns()
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
                let mut plus = format!("SELECT {fields} FROM {} WHERE {first}", quote(input));
                let mut minus = format!("SELECT {} WHERE 0", vec!["NULL"; columns.len()].join(","));
                removable = false;
                if let Some(journal) = journal {
                    journals.extend([slots[&journal.plus], slots[&journal.minus]]);
                    // A new cache starts from the current facts. A ready cache consumes
                    // only its own signed journal; subsequent iterations see it empty.
                    let journal_fields = names("", columns.len());
                    if roots[slots[&journal.plus]].len() > 0 {
                        plus.push_str(&format!(" UNION ALL SELECT {journal_fields} FROM {} WHERE {gate} AND NOT {first}", quote(&journal.plus)));
                    }
                    if roots[slots[&journal.minus]].len() > 0 {
                        minus = format!(
                            "SELECT {journal_fields} FROM {} WHERE {gate} AND NOT {first}",
                            quote(&journal.minus)
                        );
                        removable = true;
                    }
                }
                (plus, minus)
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
        empty_guards.insert(
            id,
            if empty.is_empty() {
                "0".to_owned()
            } else {
                empty.join(" OR ")
            },
        );
        let (_, next) = if state.version_loop_eligible() {
            state.append_phase(w, &changed, &mut phase, false)?
        } else {
            state.append_frozen_scalar_loop(w, &changed, &mut phase)?
        };
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
        stages.insert(id, compiled(phase, &slots)?);
        operators.insert(
            id,
            Operator {
                seen,
                journals,
                comparison: (comparison, current[&frame.inputs[&step.output]]),
            },
        );
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
        if !queries.is_empty() {
            phase.insert(store, &frame.published[r], &delta, true)?;
        }
    }
    let fold = compiled(phase, &slots)?;
    let expansion = stages
        .values()
        .chain([&fold])
        .map(|s| s.expansion)
        .sum::<usize>();
    let arms = stages
        .values()
        .chain([&fold])
        .map(|s| s.arms)
        .sum::<usize>();
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
                .values()
                .chain([&fold])
                .map(|s| s.template.width())
                .max()
                .unwrap(),
        );
    if width > 120 {
        return Ok(None);
    }
    let bindings = |stage: &Stage, parent: &str| {
        let body = stage.template.sql();
        let words = body
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .collect::<BTreeSet<_>>();
        // Frame and journal aliases are generated identifiers. Exclude unused aliases
        // before SQLite expands each separately compiled stage.
        aliases.iter().filter(|(name,_,_)| words.contains(name.as_str())).map(|(name,id,columns)|format!("{}({}) AS NOT MATERIALIZED (SELECT {} FROM {ROWS} WHERE rr_state={parent} AND rr_slot={id})",quote(name),columns.iter().map(|c|quote(c)).collect::<Vec<_>>().join(","),names("",columns.len()))).collect::<Vec<_>>()
    };
    let mut work=stages.iter().map(|(&id,stage)|format!("__rr_work_{id} AS MATERIALIZED (SELECT CASE WHEN ({}) THEN {CLEAR}(__rr_parent.state,{id}) WHEN {GATE}(__rr_parent.state,{id},0) THEN {} ELSE __rr_parent.state END AS state)",empty_guards[&id],transition(stage,id as i64,"__rr_parent.state",&bindings(stage,"__rr_parent.state")))).collect::<Vec<_>>();
    let handles = stages
        .keys()
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
            &bindings(&fold, "__rr_merged.state")
        )
    );
    let sql = format!(
        "WITH RECURSIVE __rr_retained_states(state) AS (VALUES(0) UNION ALL SELECT {transition} FROM __rr_retained_states __rr_parent WHERE {LIVE}(__rr_parent.state)) SELECT state FROM __rr_retained_states"
    );
    let frontiers = frame
        .frontiers
        .iter()
        .map(|(r, name)| (r.clone(), slots[name]))
        .collect();
    let members = selected
        .iter()
        .map(|&id| trace.steps[id].0.output.clone())
        .collect();
    let analysis = plan::acyclic::version_loop(&trace.steps, &members, &BTreeSet::new());
    let explain = json!({"strategy":"retained SQL version loop","executor":"sql","planned_submissions":1,
        "placement":"owner SQL connection",
        "rule_stages":stages.iter().map(|(&id,stage)| {let (step,_) = &trace.steps[id]; json!({"id":id,"relation":step.output,"reads":step.reads,"output_arms":stage.arms,"dependency_expansion_cost":stage.expansion,"frozen_scalar_inputs":frame.operators[id].run.reductions.as_ref().unwrap().values().flat_map(|s|s.reducer_inputs()).collect::<BTreeSet<_>>()})}).collect::<Vec<_>>(),
        "sql_bytes":sql.len(),"conditional_rule_stages":stages.len(),"root_slots":roots.len(),"width":width,"output_arms":arms,"dependency_expansion_cost":expansion,
        "logical_analysis":analysis,"proof":"Positive set caches with total checked expressions; SQL maintains contexts, contributions, support counts and results; frozen gates retain every intermediate frontier",
        "storage_callbacks":"Root construction, indexed access, immutable version differences, and frontier-length scheduling only; no nested SQL",
        "input_differences":"Shared persistent-index differences, cached per current and seen root pair within each iteration",
        "empty_context_proof":"SQL tests positive input emptiness before cache work; storage-only clearing advances its unprotected cursors",
        "input_proof":if preparing {"Positive set roots only grow; generated frontiers can shrink"}else{"Version differences include both signs; external journals are consumed once"},
        "scope":if preparing {"cold"}else{"warm"},
        "owner_gates":trace.steps.iter().enumerate().filter(|(id,_)|!selected.contains(id)).map(|(_, (step,_))|step.output.as_str()).collect::<BTreeSet<_>>()});
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
            pipeline: frame.pipeline.clone(),
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
    format!(
        "(WITH __rr_retained_feed({protocol}) AS ({}) SELECT {COLLECT}({id},{parent},rr_tag,{}) FROM (SELECT * FROM __rr_retained_feed UNION ALL {sentinel}))",
        stage.template.sql_with(bindings),
        names("", width)
    )
}
