//! SQL transport for a file-first native batch. Factoring lives in matcher/factor and
//! producers/prefix, independent of SQLite, rule names, and catalog grammars.
use super::*;
use crate::code_rule_dsl::matcher::{factor::Prefix, wave};

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
    pub wave: wave::Metrics,
    pub wave_resource_replays: usize,
    pub wave_emitted_rows: usize,
    pub wave_guard_roots: usize,
    pub wave_guard_rejections: usize,
    pub cohorts: Vec<serde_json::Value>,
    pub cohort_compile_cpu_ns: u64,
    pub cohort_compile_wall_ns: u64,
}
#[derive(Clone)]
pub(in crate::code_rule_dsl::sql) struct Member {
    pub id: usize,
    pub active: Active,
    pub prefix: Arc<Prefix>,
}
#[derive(Clone, Copy)]
struct Entry {
    program: u32,
    exact: bool,
}
#[derive(Clone)]
struct Aux {
    members: Vec<Member>,
    programs: Arc<wave::Programs>,
    entries: BTreeMap<usize, Entry>,
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
    let mut programs = wave::Programs::default();
    let entries = members
        .iter()
        .filter_map(|m| {
            let rest = matcher::AstMatcher::All(m.prefix.rest.clone());
            if !m.prefix.rest.is_empty() && programs.admission(&rest).is_some() {
                programs.admission(&m.prefix.residual(0)).map(|program| {
                    (
                        m.id,
                        Entry {
                            program,
                            exact: false,
                        },
                    )
                })
            } else if !m.prefix.terms.is_empty() {
                programs
                    .lower(&matcher::AstMatcher::All(m.prefix.terms.clone()))
                    .map(|program| {
                        (
                            m.id,
                            Entry {
                                program,
                                exact: true,
                            },
                        )
                    })
            } else {
                None
            }
        })
        .collect();
    let programs = Arc::new(programs);
    db.create_module(
        name,
        &MODULE,
        Some(Aux {
            members,
            programs,
            entries,
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
    // Stable matcher addresses are part of the existing native memo's ownership keys.
    let residuals = members
        .iter()
        .map(|(member, f)| {
            let mut grammar = f.grammar.clone();
            grammar.matcher = member.prefix.residual(member.prefix.terms.len());
            Fragment {
                grammar,
                names: f.names.clone(),
                requests: vec![],
                work: f.work,
                each: f.each.clone(),
                metrics: f.metrics.clone(),
                scope: None,
                allowance: f.allowance,
            }
        })
        .collect::<Vec<_>>();
    let (mut admitted, wave_metrics) = prepare_waves(aux, &batch, &members, &by_file, &supplied)?;
    let compile_started = Started::now();
    let mut programs = matcher::cohort::Programs::default();
    for (_, fragment) in &members {
        programs.lower(&fragment.grammar.matcher);
    }
    for fragment in &residuals {
        programs.lower(&fragment.grammar.matcher);
    }
    let compile_time = compile_started.elapsed();
    {
        let mut metrics = aux.metrics.lock().unwrap();
        metrics.cohort_compile_cpu_ns += compile_time.cpu;
        metrics.cohort_compile_wall_ns += compile_time.wall;
    }
    let mut output = vec![];
    let file_groups = by_file.iter().collect::<Vec<_>>();
    // Retain source memos and input capture rows only for this document tile.
    for files in file_groups.chunks(16) {
        let mut tasks = vec![];
        let mut task_members = vec![];
        for &(file, scopes) in files {
            let file = *file;
            for (index, selected) in scopes {
                let index = *index;
                let (member, f) = &members[index];
                let gate = admitted.remove(&(member.id, file as u32));
                let had_admission = gate.is_some();
                let (requests, remaining, fragment) = if let Some(gate) = gate.filter(|g| !g.replay)
                {
                    if gate.exact && member.prefix.rest.is_empty() && f.names.is_empty() {
                        let count = gate.nodes.len();
                        output.extend(gate.nodes.into_iter().enumerate().map(|(request, node)| {
                            (
                                member.id,
                                Row {
                                    request: request as i64,
                                    file: file as u32,
                                    node,
                                    truth: 1,
                                    ordinal: 0,
                                    values: vec![],
                                },
                            )
                        }));
                        aux.metrics.lock().unwrap().wave_emitted_rows += count;
                        f.metrics.lock().unwrap().rows += count;
                        continue;
                    }
                    (
                        gate.nodes
                            .into_iter()
                            .map(|node| Request {
                                file: file as u32,
                                node,
                                inputs: vec![],
                            })
                            .collect::<Vec<_>>(),
                        Some(gate.remaining),
                        if gate.exact {
                            Some(&residuals[index])
                        } else {
                            Some(f.as_ref())
                        },
                    )
                } else {
                    let requests = match supplied.remove(&(member.id, file as u32)) {
                        Some(r) => r,
                        None => f.requests_for(&batch, &[(file, selected.clone())])?,
                    };
                    if !had_admission {
                        aux.metrics.lock().unwrap().candidate_roots += requests.len();
                    }
                    (requests, None, None)
                };
                let mut requests = requests;
                requests.sort_by_key(|r| r.node);
                let runner = fragment.unwrap_or(f);
                task_members.push(member.id);
                tasks.push(super::cohorts::Task {
                    fragment: runner,
                    file: file as u32,
                    requests,
                    remaining,
                });
            }
        }
        let (rows, cohort_metrics) = super::cohorts::execute_compiled(&batch, &tasks, &programs)?;
        for (member, rows) in task_members.into_iter().zip(rows) {
            output.extend(rows.into_iter().map(|row| (member, row)));
        }
        aux.metrics
            .lock()
            .unwrap()
            .cohorts
            .push(serde_json::to_value(cohort_metrics).unwrap());
        for &(file, _) in files {
            crate::code_rule_dsl::sql::terminals::BatchScopes::release(*file);
        }
    }
    let memo_after = matcher::memo_metrics();
    let mut m = aux.metrics.lock().unwrap();
    m.memo_hits += memo_after.hits - memo_before.hits;
    m.memo_misses += memo_after.misses - memo_before.misses;
    m.memo_files += memo_after.files - memo_before.files;
    m.memo_metrics_enabled = cfg!(any(test, feature = "code-rule-profile"));
    m.wave = wave_metrics;
    Ok(output)
}

struct Admission {
    exact: bool,
    nodes: Vec<u32>,
    remaining: usize,
    replay: bool,
}
struct Pending {
    member: usize,
    file: u32,
    program: u32,
    flags: u32,
    nodes: Vec<u32>,
    at: usize,
}
fn prepare_waves(
    aux: &Aux,
    batch: &Batch,
    members: &[(Member, Arc<Fragment>)],
    files: &BTreeMap<usize, Vec<(usize, Option<Arc<[u32]>>)>>,
    supplied: &BTreeMap<(usize, u32), Vec<Request>>,
) -> Result<(BTreeMap<(usize, u32), Admission>, wave::Metrics)> {
    let started = Started::now();
    let mut tasks = files
        .iter()
        .flat_map(|(&file, scopes)| {
            scopes.iter().filter_map(move |(index, selected)| {
                aux.entries
                    .get(&members[*index].0.id)
                    .map(|&program| (file, *index, program, selected.clone()))
            })
        })
        .collect::<Vec<_>>();
    // Tile both axes: files remain resident; a dispatch contains several programs and files.
    tasks.sort_by_key(|(file, index, _, _)| (file / 8, index / 4, file % 8, index % 4));
    let mut tasks = tasks.into_iter();
    let mut ready = std::collections::VecDeque::new();
    let mut admitted = BTreeMap::new();
    let docs = batch
        .files
        .iter()
        .map(|f| f.root.root().get_doc())
        .collect();
    let cancel = |file: u32| {
        batch.files[file as usize]
            .context
            .budget
            .as_ref()
            .is_some_and(|b| b.cancelled())
    };
    let mut executor = wave::Executor::new(&aux.programs, docs);
    executor.cancellation(&cancel);
    let kind_index = |file: u32, kinds: &[u16]| batch.files[file as usize].facts.kind_ids(kinds);
    executor.kind_index(&kind_index);
    let mut candidate_roots = 0;
    let mut stages = 0;
    let mut replays = 0;
    let mut guard_roots = 0;
    let mut guard_rejections = 0;
    loop {
        while ready.len() < 32 {
            let Some((file, index, program, selected)) = tasks.next() else {
                break;
            };
            let (member, f) = &members[index];
            if f.each.is_some() {
                continue;
            }
            let nodes = if let Some(requests) = supplied.get(&(member.id, file as u32)) {
                if requests.iter().any(|r| !r.inputs.is_empty()) {
                    continue;
                }
                if requests
                    .iter()
                    .any(|r| r.node >= batch.files[file].facts.len())
                {
                    return Err(invalid("scope root outside document"));
                }
                requests.iter().map(|r| r.node).collect()
            } else {
                f.roots_for(batch, file, selected.as_deref())?
            };
            candidate_roots += nodes.len();
            stages += usize::from(!nodes.is_empty());
            let remaining = f.allowance.map_or(f.work, |(base, per)| {
                base.max(per.saturating_mul(batch.files[file].facts.len() as usize))
            });
            admitted.insert(
                (member.id, file as u32),
                Admission {
                    nodes: vec![],
                    exact: program.exact,
                    remaining,
                    replay: false,
                },
            );
            if !nodes.is_empty() {
                ready.push_back(Pending {
                    member: member.id,
                    file: file as u32,
                    program: program.program,
                    flags: u32::from(f.grammar.three_valued),
                    nodes,
                    at: 0,
                });
            }
        }
        if ready.is_empty() {
            break;
        }
        let mut lanes = vec![];
        let mut owners = vec![];
        while lanes.len() < wave::TILE && !ready.is_empty() {
            let mut task = ready.pop_front().unwrap();
            let gate = admitted.get(&(task.member, task.file)).unwrap();
            if gate.replay {
                continue;
            }
            let end = (task.at + 32)
                .min(task.nodes.len())
                .min(task.at + wave::TILE - lanes.len());
            for &node in &task.nodes[task.at..end] {
                lanes.push(wave::Lane {
                    program: task.program,
                    document: task.file,
                    node,
                    flags: task.flags,
                });
                owners.push((task.member, task.file));
            }
            task.at = end;
            if task.at < task.nodes.len() {
                ready.push_back(task);
            }
        }
        let answers = executor.execute(&lanes);
        for ((lane, key), answer) in lanes.into_iter().zip(owners).zip(answers) {
            let gate = admitted.get_mut(&key).unwrap();
            if gate.replay {
                continue;
            }
            if !gate.exact {
                guard_roots += 1;
            }
            let charge = if gate.exact || !answer.matched {
                answer.steps
            } else {
                0
            };
            if answer.incomplete || charge > gate.remaining {
                gate.replay = true;
                gate.nodes.clear();
                replays += 1;
                continue;
            }
            gate.remaining -= charge;
            if !gate.exact && !answer.matched {
                guard_rejections += 1;
            }
            if answer.matched {
                gate.nodes.push(lane.node)
            }
        }
    }
    let elapsed = started.elapsed();
    let mut m = aux.metrics.lock().unwrap();
    m.candidate_roots += candidate_roots;
    m.prefix_stages += stages;
    m.prefix_roots += executor.metrics.unique_lanes as usize;
    m.prefix_rows += admitted.values().map(|g| g.nodes.len()).sum::<usize>();
    m.prefix_cpu_ns += elapsed.cpu;
    m.prefix_wall_ns += elapsed.wall;
    m.wave_resource_replays += replays;
    m.wave_guard_roots += guard_roots;
    m.wave_guard_rejections += guard_rejections;
    Ok((admitted, executor.metrics))
}
