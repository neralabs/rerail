//! SQL transport for a file-first native batch. Factoring lives in matcher/factor and
//! producers/prefix, independent of SQLite, rule names, and catalog grammars.
use super::*;
use crate::code_rule_dsl::{matcher::factor::Prefix, producers::prefix::Plan};

#[derive(Default, serde::Serialize)]
pub(in crate::code_rule_dsl::sql) struct Metrics {
    pub filters: usize,
    pub prefix_stages: usize,
    pub prefix_roots: usize,
    pub prefix_rows: usize,
    pub prefix_cpu_ns: u64,
    pub prefix_wall_ns: u64,
    pub candidate_roots: usize,
    pub grouping_roots: usize,
    pub shared_consumers: usize,
    pub peak_domain_rows: usize,
    pub memo_hits: u64,
    pub memo_misses: u64,
    pub memo_files: u64,
    pub memo_metrics_enabled: bool,
    pub producers: Vec<serde_json::Value>,
}
#[derive(Clone)]
pub(in crate::code_rule_dsl::sql) struct Member {
    pub id: usize,
    pub active: Active,
    pub prefix: Arc<Prefix>,
}
#[derive(Clone)]
struct Aux {
    members: Vec<Member>,
    eligible: std::collections::BTreeSet<usize>,
    demands: Arc<DemandStore>,
    metrics: Arc<Mutex<Metrics>>,
    width: usize,
}
pub(in crate::code_rule_dsl::sql) fn register(
    db: &Connection,
    name: &CStr,
    members: Vec<Member>,
    demands: Arc<DemandStore>,
    metrics: Arc<Mutex<Metrics>>,
    width: usize,
) -> Result<()> {
    const MODULE: Module<'static, Table> = Module::eponymous_only_module();
    // Static eligibility avoids building domain keys for predicates with no possible consumer.
    // This does not authorize sharing: execution still compares the actual domains and options.
    let potential = Plan::new(
        &members
            .iter()
            .map(|m| (m.id, m.prefix.keys.as_slice()))
            .collect::<Vec<_>>(),
    );
    let eligible = potential
        .stages
        .iter()
        .flat_map(|s| s.consumers.iter().copied())
        .collect();
    db.create_module(
        name,
        &MODULE,
        Some(Aux {
            members,
            eligible,
            demands,
            metrics,
            width,
        }),
    )
}
#[repr(C)]
struct Table {
    base: sqlite3_vtab,
    aux: Aux,
}
// SAFETY: SQLite base is first; all batch inputs are owned.
unsafe impl<'v> VTab<'v> for Table {
    type Aux = Aux;
    type Cursor = Cursor<'v>;
    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Aux>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        let aux = aux.ok_or_else(|| invalid("missing native batch"))?.clone();
        let captures = (0..aux.width).map(|i| format!(",c{i}")).collect::<String>();
        Ok((Cow::Owned(CString::new(format!("CREATE TABLE x(clause INTEGER,file INTEGER,node INTEGER,truth INTEGER,ordinal INTEGER{captures},demand INTEGER HIDDEN)")).unwrap()),Self {base:sqlite3_vtab::default(),aux}))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let demand = info
            .constraints()
            .enumerate()
            .find(|(_, c)| {
                c.is_usable()
                    && c.column() == (5 + self.aux.width) as c_int
                    && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            })
            .map(|(i, _)| i);
        let Some(i) = demand else { return Ok(false) };
        info.constraint_usage(i).set_argv_index(1);
        info.constraint_usage(i).set_omit(true);
        info.set_estimated_rows(i64::MAX);
        info.set_estimated_cost(i64::MAX as f64);
        Ok(true)
    }
    fn open(&'v mut self) -> Result<Cursor<'v>> {
        Ok(Cursor {
            base: sqlite3_vtab_cursor::default(),
            aux: self.aux.clone(),
            rows: vec![],
            at: 0,
            phantom: PhantomData,
        })
    }
}
#[repr(C)]
struct Cursor<'v> {
    base: sqlite3_vtab_cursor,
    aux: Aux,
    rows: Vec<(usize, Row)>,
    at: usize,
    phantom: PhantomData<&'v Table>,
}
// SAFETY: SQLite base is first; rows retain all output values.
unsafe impl VTabCursor for Cursor<'_> {
    fn filter(&mut self, _: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.aux.metrics.lock().unwrap().filters += 1;
        self.rows = execute(&self.aux, self.aux.demands.take_group(args.get(0)?)?)?;
        self.at = 0;
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        self.at += 1;
        if self.at == self.rows.len() {
            self.rows = vec![];
        }
        Ok(())
    }
    fn eof(&self) -> bool {
        self.at >= self.rows.len()
    }
    fn column(&self, ctx: &mut Context, col: c_int) -> Result<()> {
        let (id, r) = &self.rows[self.at];
        match col {
            0 => ctx.set_result(&(*id as i64)),
            1 => ctx.set_result(&r.file),
            2 => ctx.set_result(&r.node),
            3 => ctx.set_result(&r.truth),
            4 => ctx.set_result(&r.ordinal),
            c if c >= 5 && (c as usize) < 5 + self.aux.width => {
                ctx.set_result(r.values.get(c as usize - 5).unwrap_or(&Cell::Null))
            }
            _ => ctx.set_result(&rusqlite::types::Null),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.at as i64)
    }
}
struct Input {
    member: Member,
    fragment: Arc<Fragment>,
    requests: Arc<[Request]>,
}
fn run(
    f: &Fragment,
    batch: &Batch,
    file: u32,
    requests: &[Request],
    remaining: Option<usize>,
) -> Result<Vec<Row>> {
    f.evaluate_file(batch, requests, file, 0..requests.len(), remaining)
}
fn execute(
    aux: &Aux,
    mut demands: BTreeMap<usize, Arc<super::demand::Demand>>,
) -> Result<Vec<(usize, Row)>> {
    let memo_before = matcher::memo_metrics();
    let mut batch = None;
    let mut members = vec![];
    let mut by_file = BTreeMap::<usize, Vec<(usize, Option<Arc<[u32]>>)>>::new();
    let mut supplied = BTreeMap::<(usize, u32), Vec<Request>>::new();
    for member in &aux.members {
        let (b, f) = member
            .active
            .read()
            .unwrap()
            .clone()
            .ok_or_else(|| invalid("missing active fragment"))?;
        batch.get_or_insert(b);
        let scope = f
            .scope
            .as_ref()
            .ok_or_else(|| invalid("batch requires explicit scope"))?;
        let demand = demands.remove(&member.id);
        for (file, roots) in scope {
            by_file
                .entry(*file)
                .or_default()
                .push((members.len(), roots.clone()));
            if demand.is_some() {
                supplied.insert((member.id, *file as u32), vec![]);
            }
        }
        if let Some(demand) = demand {
            for r in &demand.requests {
                supplied
                    .get_mut(&(member.id, r.file))
                    .ok_or_else(|| invalid("demand escaped scope"))?
                    .push(r.clone());
            }
        }
        members.push((member.clone(), f));
    }
    if !demands.is_empty() {
        return Err(invalid("demand for foreign batch member"));
    }
    let Some(batch) = batch else {
        return Ok(vec![]);
    };
    let mut output = vec![];
    let mut plans = BTreeMap::new();
    for (file, scopes) in by_file {
        let entry = &batch.files[file];
        matcher::with_file_memo(&entry.root, || -> Result<()> {
            // Equality is the complete ordered root domain plus resource options and dialect.
            // Capture-bearing requests and enumeration remain private, but still share file memo.
            let mut groups =
                BTreeMap::<(String, usize, Option<(usize, usize)>, Vec<u32>), Vec<Input>>::new();
            for (index, selected) in scopes {
                let (member, f) = &members[index];
                let mut requests = match supplied.remove(&(member.id, file as u32)) {
                    Some(r) => r,
                    None => f.requests_for(&batch, &[(file, selected)])?,
                };
                requests.sort_by_key(|r| r.node);
                aux.metrics.lock().unwrap().candidate_roots += requests.len();
                if requests.is_empty() {
                    continue;
                }
                if f.each.is_some()
                    || requests.iter().any(|r| !r.inputs.is_empty())
                    || !aux.eligible.contains(&member.id)
                {
                    output.extend(
                        run(f, &batch, file as u32, &requests, None)?
                            .into_iter()
                            .map(|r| (member.id, r)),
                    );
                    continue;
                }
                aux.metrics.lock().unwrap().grouping_roots += requests.len();
                let key = (
                    f.grammar.language.to_string(),
                    f.work,
                    f.allowance,
                    requests.iter().map(|r| r.node).collect(),
                );
                groups.entry(key).or_default().push(Input {
                    member: member.clone(),
                    fragment: f.clone(),
                    requests: requests.into(),
                });
            }
            for inputs in groups.into_values() {
                let ids = inputs.iter().map(|i| i.member.id).collect::<Vec<_>>();
                if inputs.len() == 1 {
                    let i = &inputs[0];
                    output.extend(
                        run(&i.fragment, &batch, file as u32, &i.requests, None)?
                            .into_iter()
                            .map(|r| (i.member.id, r)),
                    );
                    continue;
                }
                let plan = plans.entry(ids).or_insert_with(|| Prepared::new(&inputs));
                evaluate_group(aux, &batch, file as u32, &inputs, plan, &mut output)?;
            }
            Ok(())
        })?;
        // All consumers of this file in the batch are complete; release parked semantic caches.
        crate::code_rule_dsl::sql::terminals::BatchScopes::release(file);
    }
    let memo_after = matcher::memo_metrics();
    {
        let mut m = aux.metrics.lock().unwrap();
        m.memo_hits += memo_after.hits - memo_before.hits;
        m.memo_misses += memo_after.misses - memo_before.misses;
        m.memo_files += memo_after.files - memo_before.files;
        m.memo_metrics_enabled = cfg!(any(test, feature = "code-rule-profile"));
        m.producers = plans.values().flat_map(Prepared::profile).collect();
    }
    Ok(output)
}
struct Completed {
    requests: Arc<[Request]>,
    remaining: Option<usize>,
    failure: Option<Row>,
}
// Keep compiled matcher objects alive and at stable addresses for the entire memo lifetime.
// Native semantic and relation caches can key their entries by those object addresses.
struct Prepared {
    plan: Plan<matcher::factor::Key>,
    stages: Vec<Arc<Fragment>>,
    tails: Vec<Arc<Fragment>>,
}
impl Prepared {
    fn profile(&self) -> Vec<serde_json::Value> {
        self.plan.stages.iter().zip(&self.stages).map(|(stage,f)| {
            let m=f.metrics.lock().unwrap();
            let terms=stage.terms.iter().map(|key|match key {
                matcher::factor::Key::Utility(body)=>serde_json::from_slice::<serde_json::Value>(body).ok().and_then(|v|v[1].as_str().map(str::to_owned)).unwrap_or_else(||"utility".into()),
                matcher::factor::Key::Kinds(k)=>format!("kinds {k:?}"),
                matcher::factor::Key::Regex(r)=>format!("regex {r}"),
                _=>"closed structural predicate".into(),
            }).collect::<Vec<_>>();
            serde_json::json!({"contract":"matcher.closed_predicate.v1","consumers":stage.consumers,"terms":terms,"roots":m.roots,"rows":m.rows,"cpuNs":m.native_cpu_ns,"wallNs":m.native_wall_ns,"filesWithWork":m.remaining_work.len()})
        }).collect()
    }
    fn new(inputs: &[Input]) -> Self {
        let plan = Plan::new(
            &inputs
                .iter()
                .map(|i| (i.member.id, i.member.prefix.keys.as_slice()))
                .collect::<Vec<_>>(),
        );
        let input = |id| {
            inputs
                .iter()
                .find(|i| i.member.id == id)
                .expect("planned consumer")
        };
        let stages = plan
            .stages
            .iter()
            .map(|stage| {
                let prototype = input(stage.consumers[0]);
                let mut grammar = prototype.fragment.grammar.clone();
                let depth = stage.parent.map_or(0, |mut p| {
                    let mut n = 0;
                    loop {
                        n += plan.stages[p].terms.len();
                        match plan.stages[p].parent {
                            Some(parent) => p = parent,
                            None => break,
                        }
                    }
                    n
                });
                debug_assert_eq!(
                    &prototype.member.prefix.keys[depth..depth + stage.terms.len()],
                    &stage.terms
                );
                grammar.matcher = matcher::AstMatcher::All(
                    prototype.member.prefix.terms[depth..depth + stage.terms.len()].to_vec(),
                );
                grammar.three_valued = false;
                grammar.transform = None;
                Arc::new(Fragment {
                    grammar,
                    names: vec![],
                    requests: vec![],
                    work: prototype.fragment.work,
                    each: None,
                    metrics: Arc::default(),
                    scope: None,
                    allowance: prototype.fragment.allowance,
                })
            })
            .collect();
        let tails = plan
            .tails
            .iter()
            .map(|tail| {
                let i = input(tail.consumer);
                if tail.consumed == 0 {
                    return i.fragment.clone();
                }
                let mut grammar = i.fragment.grammar.clone();
                grammar.matcher = i.member.prefix.residual(tail.consumed);
                Arc::new(Fragment {
                    grammar,
                    names: i.fragment.names.clone(),
                    requests: vec![],
                    work: i.fragment.work,
                    each: i.fragment.each.clone(),
                    metrics: i.fragment.metrics.clone(),
                    scope: None,
                    allowance: i.fragment.allowance,
                })
            })
            .collect();
        Self {
            plan,
            stages,
            tails,
        }
    }
}
fn evaluate_group(
    aux: &Aux,
    batch: &Batch,
    file: u32,
    inputs: &[Input],
    prepared: &Prepared,
    output: &mut Vec<(usize, Row)>,
) -> Result<()> {
    let original = &inputs[0].requests;
    let mut completed: Vec<Completed> = vec![];
    let plan = &prepared.plan;
    for (stage, f) in plan.stages.iter().zip(&prepared.stages) {
        let source = stage.parent.map(|p| &completed[p]);
        if let Some(failure) = source.and_then(|s| s.failure.clone()) {
            completed.push(Completed {
                requests: Arc::from([]),
                remaining: None,
                failure: Some(failure),
            });
            continue;
        }
        let requests = source.map_or(original, |s| &s.requests);
        let before = {
            let m = f.metrics.lock().unwrap();
            (m.roots, m.native_cpu_ns, m.native_wall_ns)
        };
        let rows = run(f, batch, file, requests, source.and_then(|s| s.remaining))?;
        let m = f.metrics.lock().unwrap();
        {
            let mut metrics = aux.metrics.lock().unwrap();
            metrics.prefix_stages += 1;
            metrics.prefix_roots += m.roots - before.0;
            metrics.prefix_rows += rows.len();
            metrics.prefix_cpu_ns += m.native_cpu_ns - before.1;
            metrics.prefix_wall_ns += m.native_wall_ns - before.2;
            metrics.shared_consumers += stage.consumers.len();
            metrics.peak_domain_rows = metrics.peak_domain_rows.max(requests.len());
        }
        let failure = rows.iter().find(|r| r.truth < 0).cloned();
        if rows.iter().any(|r| r.truth == 2) {
            return Err(invalid("closed prefix produced logical unknown"));
        }
        let selected = rows
            .iter()
            .filter(|r| r.truth == 1)
            .map(|r| requests[r.request as usize].clone())
            .collect::<Vec<_>>();
        completed.push(Completed {
            requests: selected.into(),
            remaining: m.remaining_work.get(&file).copied(),
            failure,
        });
    }
    for (tail, f) in plan.tails.iter().zip(&prepared.tails) {
        let source = tail.parent.map(|p| &completed[p]);
        if let Some(mut failure) = source.and_then(|s| s.failure.clone()) {
            failure.values = vec![Cell::Null; f.names.len()];
            output.push((tail.consumer, failure));
            continue;
        }
        let requests = source.map_or(original, |s| &s.requests);
        output.extend(
            run(f, batch, file, requests, source.and_then(|s| s.remaining))?
                .into_iter()
                .map(|r| (tail.consumer, r)),
        );
    }
    Ok(())
}
