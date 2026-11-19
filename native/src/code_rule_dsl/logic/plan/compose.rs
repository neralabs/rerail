//! Compile relational graphs and linear closures to one query, including batched native leaves.
//! Compilation appends definitions. Only `execute` steps a statement.
//!
//! The composed path reads a program's version (t36 item 7, step 6): its rules by their handles (`Placed::body`), its
//! other fields through `Fields`, and its components in the tree's order (`analyses::components_by_name`). Where it
//! edits a program (`stages`: relations made inputs, rules proved empty, outputs chosen) the edit is an overlay of the
//! version it was given.
use super::super::program_image::{Builder, Lit, NativeKind, NativeSpec, RelId, analyses};
use super::super::sql::ident;
use super::body::{Placed, Versions};
use super::*;

thread_local! {static CACHE:RefCell<HashMap<[u8;32],Compiled>>=RefCell::new(HashMap::new());}
/// Whether a run may be one composed query: only when asked (`RERAIL_COMPOSE=1`). The integration branch runs its
/// strata natively, with SQL only where SQL reads a relation (decision 0104). Composition retains ordinary facts
/// for sessions and persistence; it stays opt-in for comparison (async-family-integration).
pub(super) fn enabled() -> bool {
    #[cfg(any(test, feature = "test-hooks"))]
    if experiments::native_only() {
        return false;
    }
    #[cfg(any(test, feature = "test-hooks"))]
    if ASKED.get() {
        return true;
    }
    std::env::var("RERAIL_COMPOSE").is_ok_and(|v| v == "1")
}

// A test of the composed form asks for it on its own thread (`RERAIL_COMPOSE` would ask for every test's runs).
#[cfg(any(test, feature = "test-hooks"))]
thread_local! {pub(super) static ASKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };}

pub(super) fn clear() {
    CACHE.with(|c| c.borrow_mut().clear());
}

/// `p`'s composed query, keyed by its program's digest (`Placed::digest`, decision 0173 §7: its version's and its
/// maintained parts', so a version the composed path edited, `stages`, needs no tree) and the inputs' fields.
pub(super) fn planned(p: &Placed, db: &Connection, inputs: &mut Inputs) -> Result<Compiled> {
    use sha2::{Digest, Sha256};
    let key: [u8; 32] = Sha256::digest(
        format!(
            "compose-v4\0{:016x}\0{}\0{}\0{:?}\0{:?}\0{:?}",
            p.digest(), p.whole, inputs.facts, inputs.atoms, inputs.kept, inputs.batch
        )
        .as_bytes(),
    )
    .into();
    if let Some(plan) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return Ok(plan);
    }
    let _span = super::super::profile::span(|| "compile composed query".into());
    let given = inputs.given.take();
    let mut owned = inputs.clone();
    owned.given = given;
    let mut model = make_plan(p, db, owned)?;
    let plan = compile(p, &mut model)?;
    inputs.given = Some(model.stats);
    CACHE.with(|c| c.borrow_mut().insert(key, plan.clone()));
    Ok(plan)
}

#[derive(Clone)]
pub(super) struct Compiled {
    pub sql: String,
    outputs: Vec<String>,
    kept: BTreeSet<String>,
    reads: Vec<(String, BTreeMap<String, usize>)>,
    aliases: HashMap<String, String>,
    resident: BTreeSet<String>,
    prepares: Arc<std::sync::OnceLock<bool>>,
}

/// Whether the composed path takes `p`: no lattices, answers, gates or natives but Python's, no `finding` output, and
/// every component one relation that reads itself at most once, linearly (not where it is native, nor through an
/// aggregate, `nearest` or `ast` literal).
pub fn accepts(p: &Placed) -> Result<bool> {
    let f = p.fields()?;
    if !f.lattices().is_empty()
        || !f.answers().is_empty()
        || f.about.gates != 0
        || f.natives().iter().any(|n| !matches!(n.kind, NativeKind::Python | NativeKind::PythonCandidates))
        || f.outputs().any(|r| r == "finding")
    {
        return Ok(false);
    }
    for scc in p.components()?.iter() {
        if scc.len() != 1 {
            return Ok(false);
        }
        let rel = scc.iter().next().unwrap();
        let Some(id) = f.v.rel(rel) else { continue };
        for rule in f.v.heads(id) {
            let reads = rule.reads().filter(|a| a.rel() == id).count();
            if reads > 1
                || (reads == 1
                    && (f.native(rel).is_some()
                        || rule.body().any(|l| matches!(l.view(), Lit::Agg { .. } | Lit::Nearest { .. } | Lit::Ast { .. }))))
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// What `roots` read, directly or through rules or natives, the roots included (`analyses::needed`), over `f`'s version
/// (`analyses::needed`).
pub(super) fn needed<'a>(f: Fields<'_>, roots: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut ids = Vec::new();
    for r in roots {
        match f.v.rel(r) {
            Some(id) => ids.push(id),
            None => {
                out.insert(r.to_owned());
            }
        }
    }
    let need = analyses::needed(f.v, f.about, ids);
    out.extend(need.iter().enumerate().filter(|&(_, &n)| n).map(|(i, _)| f.v.rel_name(RelId(i as u32)).to_owned()));
    out
}

/// The relation rule `k` of `p` derives (its head's).
fn head(p: &Placed, k: usize) -> &str {
    p.v.rel_name(p.rule(k).head().rel())
}

/// `p`'s composed query: its rules read by their handles, its fields from its version (`Placed::of_program`, or an
/// edit of one).
pub(super) fn compile(p: &Placed, model: &mut Model) -> Result<Compiled> {
    let pf = p.fields()?;
    let decls: Vec<&str> = pf.decls_by_name().into_iter().map(|(r, _)| r).collect();
    // SQLite folds identifier case, whereas rule relations are case-sensitive.
    // Bind physical names on the typed query; never replace substrings in rendered SQL.
    let mut counts = HashMap::<String, usize>::new();
    for rel in &decls { *counts.entry(rel.to_ascii_lowercase()).or_default() += 1; }
    let mut taken: BTreeSet<_> = decls.iter().map(|r|r.to_ascii_lowercase()).collect();
    let mut names = HashMap::new();
    for rel in decls.iter().filter(|r|counts[&r.to_ascii_lowercase()] > 1) {
        let mut name = format!("__rr_relation_{}", names.len());
        while !taken.insert(name.to_ascii_lowercase()) { name.push('_'); }
        names.insert(rel.to_string(), name);
    }
    let physical = |rel: &str| ident(names.get(rel).map(String::as_str).unwrap_or(rel)).into_owned();
    let outputs: Vec<String> = pf
        .outputs()
        .map(str::to_owned)
        .chain(model.inputs.kept.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let needed = needed(pf, outputs.iter().map(String::as_str));
    // The program's rules by the relation they derive, in the program's order.
    let mut by_head = HashMap::<&str, Vec<usize>>::new();
    for k in 0..p.len() {
        by_head.entry(head(p, k)).or_default().push(k);
    }
    let mut uses = HashMap::<String, usize>::new();
    for k in (0..p.len()).filter(|&k| needed.contains(head(p, k))) {
        for a in p.rule(k).reads() {
            *uses.entry(pf.rel_name(a.rel()).to_owned()).or_default() += 1;
        }
    }
    for r in &outputs {
        *uses.entry(r.clone()).or_default() += 1;
    }
    let python = |rel: &str| -> Option<(&str, &str, Option<&str>)> {
        match &pf.native(rel)?.spec {
            NativeSpec::Python { kind, input, targets } => Some((pf.text(*kind), pf.rel_name(*input), targets.map(|t| pf.rel_name(t)))),
            _ => None,
        }
    };
    let mut frame_counts = BTreeMap::<String, usize>::new();
    for n in pf.natives() {
        if needed.contains(pf.rel_name(n.rel)) {
            if let NativeSpec::Python { input, .. } | NativeSpec::PythonCandidates { input, .. } = &n.spec {
                *frame_counts.entry(pf.rel_name(*input).to_owned()).or_default() += 1;
            }
        }
    }
    let mut frames = BTreeMap::new();
    let mut frame_defs = Vec::new();
    for (input, count) in frame_counts {
        let mut name = format!("__rr_demand_{}", frames.len());
        while pf.declared(name.as_str()) {
            name.push('_');
        }
        frame_defs.push(format!(
            "{name}(handle) AS MATERIALIZED (SELECT rr_demand_frame(c0) FROM {} HAVING count(*) > 0)",
            physical(&input)
        ));
        if let Some(n) = uses.get_mut(&input) {
            *n = n.saturating_sub(count - 1);
        }
        frames.insert(input, name);
    }
    let keys_only = |rel: &str| {
        frames.contains_key(rel) && !outputs.iter().any(|r|r==rel) && (0..p.len()).filter(|&k|p.rule(k).reads().any(|a|pf.rel_name(a.rel())==rel)).all(|k|matches!(python(head(p, k)),Some((_,input,targets)) if input==rel && targets!=Some(rel)))
    };
    let mut reads = Vec::new();
    let mut defs = Vec::new();
    let mut defined = BTreeSet::new();
    let mut add = |rel: &str, query: String, materialized: bool, dependencies: BTreeMap<String, usize>| {
        reads.push((rel.to_owned(), dependencies));
        let columns = (0..pf.width(rel))
            .map(|c| format!("c{c}"))
            .collect::<Vec<_>>()
            .join(",");
        defs.push(format!(
            "{}({columns}) AS {} ({query})",
            physical(rel),
            if materialized {
                "MATERIALIZED"
            } else {
                "NOT MATERIALIZED"
            }
        ));
        defined.insert(rel.to_owned());
    };
    // Constants can be read by any component. A head with rules folds its facts below.
    let constants = |rel: &str| -> Vec<String> {
        pf.facts(rel)
            .into_iter()
            .flat_map(|f| f.rows())
            .map(|row| {
                format!(
                    "SELECT {}",
                    row.map(|v| match v {
                            super::super::program_image::Cell::Int(n) => n.to_string(),
                            super::super::program_image::Cell::Str(s) => format!("'{}'", pf.text(s).replace('\'', "''")),
                            super::super::program_image::Cell::Branded(b) => b.value().to_string(),
                        })
                        .collect::<Vec<_>>()
                        .join(",")
                )
            })
            .collect()
    };
    let atoms = model.inputs.atoms.is_some();
    let empty = |rel: &str| {
        format!(
            "SELECT {} WHERE 0",
            pf.types(rel)
                .iter()
                .map(|t| format!(
                    "CAST(NULL AS {})",
                    if *t == "sym" && !atoms {
                        "TEXT"
                    } else {
                        "INTEGER"
                    }
                ))
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    for rel in decls.iter().copied().filter(|r| {
        needed.contains(*r) && !pf.is_input(r) && !pf.derives(r)
    }) {
        let arms = constants(rel);
        add(
            rel,
            if arms.is_empty() {
                empty(rel)
            } else {
                arms.join(" UNION ")
            },
            false,
            BTreeMap::new(),
        );
    }
    for scc in p.components()?.iter() {
        let rel = scc.iter().next().unwrap();
        if !needed.contains(rel) || pf.is_input(rel) {
            continue;
        }
        // The relation's rules, read by their handles.
        let reads_itself = |b: &super::body::Body<'_>| (0..b.len()).filter_map(|i| b.atom(i)).any(|a| b.rel(a) == rel.as_str());
        let mut rules: Vec<super::body::Body<'_>> = by_head.get(rel.as_str()).into_iter().flatten().map(|&k| p.body(k)).collect();
        // SQLite requires every seed before every recursive arm. UNION provides the visited set.
        rules.sort_by_key(|b| reads_itself(b));
        if rules.is_empty() {
            continue;
        }
        let mut arms = constants(rel);
        let mut dependencies = BTreeMap::new();
        let producer=match pf.native(rel).map(|n| &n.spec) {
            Some(NativeSpec::Python{kind,input,targets})=>Some((pf.text(*kind),pf.rel_name(*input),targets.map(|t|pf.rel_name(t)),String::new())),
            Some(NativeSpec::PythonCandidates{input,targets,context,member})=>Some(("candidates",pf.rel_name(*input),Some(pf.rel_name(*targets)),format!(" AND context='{}' AND member='{}'",pf.text(*context).replace(char::from(39),"''"),pf.text(*member).replace(char::from(39),"''")))),
            _=>None,
        };
        if let Some((kind,input,targets,parameters))=producer {
            *dependencies.entry(input.to_owned()).or_default() += 1;
            if let Some(target) = targets { *dependencies.entry(target.to_owned()).or_default() += 1; }
            let cols = pf.types(rel)
                .iter()
                .enumerate()
                .map(|(c, ty)| {
                    if *ty == "sym" && model.inputs.atoms.is_some() {
                        format!("sym_atom(c{c})")
                    } else {
                        format!("c{c}")
                    }
                })
                .collect::<Vec<_>>()
                .join(",");
            let target = targets
                .map(|t| {
                    format!(
                        " AND targets=(SELECT json_group_array({}) FROM {})",
                        if model.inputs.atoms.is_some() {
                            "sym_text(c0)"
                        } else {
                            "c0"
                        },
                        physical(t)
                    )
                })
                .unwrap_or_default();
            arms.push(format!("SELECT DISTINCT {cols} FROM {} AS demanded CROSS JOIN rr_batch_python_{kind} WHERE frame=demanded.handle{target}{parameters}",frames[input]));
        } else {
            for b in rules {
                let ((order, _, _), _) = model.plan_rule_reused(pf, &b, None)?;
                let mut q = model.checked_query(pf, &b, &order, None, None)?;
                count_reads(&mut q.body, &mut dependencies);
                for cte in &mut q.ctes { for arm in &mut cte.arms { count_reads(arm, &mut dependencies); } }
                bind_names(&mut q.body, &names);
                for cte in &mut q.ctes { for arm in &mut cte.arms { bind_names(arm, &names); } }

                // A frame owns key deduplication. No other consumer observes these projection rows.
                if keys_only(rel) {
                    q.body.distinct = false;
                }
                // Keep the statistics-driven plan's demand anchors. The session-table
                // sqlite_order policy loses native posting selectivity behind these CTEs.
                if !q.ctes.is_empty() && reads_itself(&b) {
                    return Err(Error::Plan(format!(
                        "{rel}: recursive walk needs a separate physical plan"
                    )));
                }
                // Keep a walk's local WITH scope. Different rules can use the same walk names.
                arms.push(if q.ctes.is_empty() {
                    q.render()
                } else {
                    format!("SELECT * FROM ({})", q.render())
                });
            }
        }
        add(
            rel,
            if arms.is_empty() {
                empty(rel)
            } else {
                arms.join(" UNION ")
            },
            pf.native(rel).is_some() || uses.get(rel).copied().unwrap_or(0) > 1,
            dependencies,
        );
    }
    drop(add);
    defs.extend(frame_defs);
    if defs.iter().any(|d| d.contains("rdl_int")) {
        defs.insert(
            0,
            "rdl_int(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM rdl_int WHERE i<4096)".into(),
        );
    }
    let width = outputs
        .iter()
        .map(|r| pf.width(r))
        .max()
        .unwrap_or(1);
    let arms = outputs
        .iter()
        .enumerate()
        .map(|(id, rel)| {
            let cols = (0..width)
                .map(|c| {
                    if c >= pf.width(rel) {
                        "NULL".into()
                    } else {
                        format!("c{c}")
                    }
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("SELECT {id} AS __output,{cols} FROM {}", physical(rel))
        })
        .collect::<Vec<_>>();
    let body = if arms.is_empty() {
        "SELECT 0 WHERE 0".into()
    } else {
        arms.join(" UNION ALL ")
    };
    Ok(Compiled {
        sql: {
            let sql = if defs.is_empty() {body} else {format!("WITH RECURSIVE {} {body}", defs.join(",\n"))};
            #[cfg(any(test, feature = "test-hooks"))]
            let sql = experiments::physical_sql(sql, pf);
            sql
        },
        outputs,
        kept: model.inputs.kept.clone(),
        reads,
        resident: BTreeSet::new(),
        prepares: Default::default(),
        aliases: names.into_iter().filter(|(r,_)|pf.is_input(r)).collect(),
    })
}

/// Count typed reads, including correlated subqueries, before SQL rendering.
fn count_reads(block: &mut Select, reads: &mut BTreeMap<String, usize>) {
    for item in &block.from {
        if let Relation::Derived { rel, .. } | Relation::Input { rel, .. } | Relation::Bound {rel,..} | Relation::Batch {rel,..} = &item.relation {
            *reads.entry(rel.clone()).or_default() += 1;
        }
    }
    let ons = block.from.iter_mut().flat_map(|item| match &mut item.join {
        Join::On(on) => on.iter_mut().collect::<Vec<_>>(), Join::Cross => Vec::new(),
    });
    for e in block.columns.iter_mut().map(|(e,_)|e).chain(&mut block.predicates).chain(ons) {
        e.visit_mut(&mut |e| if let Expr::Exists { query, .. } | Expr::Scalar(query) = e { count_reads(query,reads); });
    }
}

/// Apply physical relation names through joins, aggregates and correlated subqueries.
fn bind_names(block: &mut Select, names: &HashMap<String, String>) {
    for item in &mut block.from {
        if let Relation::Derived { rel, delta: false } | Relation::Input { rel, .. } = &item.relation {
            if let Some(table) = names.get(rel) {
                item.relation = Relation::Bound { rel: rel.clone(), table: ident(table).into_owned() };
            }
        }
    }
    let ons = block.from.iter_mut().flat_map(|item| match &mut item.join {
        Join::On(on) => on.iter_mut().collect::<Vec<_>>(), Join::Cross => Vec::new(),
    });
    for e in block.columns.iter_mut().map(|(e,_)|e).chain(&mut block.predicates).chain(ons) {
        e.visit_mut(&mut |e| if let Expr::Exists { query, .. } | Expr::Scalar(query) = e { bind_names(query,names); });
    }
}

impl Compiled {
    fn bind_aliases(&self, db: &Connection) -> Result<()> {
        for (logical,physical) in &self.aliases {
            let source = crate::code_rule_dsl::facts::vtab::bound_source(db,logical)
                .or_else(||super::super::native::component::derived(db,logical))
                .ok_or_else(||Error::Plan(format!("missing composed input {logical}")))?;
            connection::bind(db,physical,source)?;
        }
        Ok(())
    }

    /// Expanded-read counts identify risky graphs; SQLite determines whether they fit.
    /// Cache structural refusal so subsequent chunks do not repeat a failed preparation.
    fn prepares(&self, db: &Connection) -> Result<bool> {
        if let Some(fits)=self.prepares.get() { return Ok(*fits); }
        self.bind_aliases(db)?;
        let _span=super::super::profile::span(|| "composed query limit check".into());
        super::super::profile::decision("composed query SQL",||self.sql.clone());
        let fits=match db.prepare_cached(&self.sql) {
            Ok(_) => true,
            Err(e) if e.to_string().contains("too many references to") || e.to_string().contains("VIEWs and/or subqueries nested too deep") => {
                super::super::profile::decision("composed preparation limit",||e.to_string());
                false
            }
            Err(e) => return Err(e.into()),
        };
        let _=self.prepares.set(fits);
        Ok(fits)
    }

    pub(super) fn execute(
        &self,
        db: &Connection,
        f: Fields<'_>,
        lowered: Option<lower::Lowering<'_>>,
        hooks: &mut Hooks<'_>,
    ) -> Result<Outcome> {
        self.bind_aliases(db)?;
        if super::super::profile::on() {
            // Publish the generated text even if preparation fails.
            super::super::profile::decision("composed query SQL", || self.sql.clone());
            let mut explain = db.prepare(&format!("EXPLAIN QUERY PLAN {}", self.sql))?;
            let rows = explain.query_map([], |r| r.get::<_, String>(3))?.collect::<rusqlite::Result<Vec<_>>>()?;
            super::super::profile::decision("composed query plan", || rows.join("\n"));
            super::super::profile::decision("composed query shape", || format!("{} SQL bytes; {} outputs; one evaluation statement",self.sql.len(),self.outputs.len()));
        }
        let _span = super::super::profile::span(|| "composed relational query".into());
        let _frames = crate::code_rule_dsl::facts::vtab::demand::Scope::enter();
        let counters =
            super::super::profile::on().then(|| crate::code_rule_dsl::facts::vtab::counters(db));
        let mut values: BTreeMap<String, Vec<Vec<SqlValue>>> = self
            .outputs
            .iter()
            .map(|r| (r.clone(), Vec::new()))
            .collect();
        {
            let _preparing = super::super::profile::span(|| "composed query preparation".into());
            db.prepare_cached(&self.sql)?;
        }
        let mut query = db.prepare_cached(&self.sql)?;
        #[cfg(any(test, feature = "test-hooks"))]
        query.reset_status(rusqlite::StatementStatus::RePrepare);
        let mut cursor = query.query([])?;
        while let Some(row) = cursor.next()? {
            let rel = &self.outputs[row.get::<_, i64>(0)? as usize];
            values.get_mut(rel).unwrap().push(
                (0..f.width(rel))
                    .map(|c| row.get(c + 1))
                    .collect::<rusqlite::Result<_>>()?,
            );
        }
        drop(cursor);
        #[cfg(any(test, feature = "test-hooks"))]
        super::super::profile::decision("composed automatic reprepare",||query.get_status(rusqlite::StatementStatus::RePrepare).to_string());
        drop(query);
        if let Some(before) = counters {
            for (rel, after) in crate::code_rule_dsl::facts::vtab::counters(db) {
                let old = before.get(&rel).copied().unwrap_or_default();
                let delta = std::array::from_fn::<_, 4, _>(|i| after[i].saturating_sub(old[i]));
                if delta.iter().any(|&n| n > 0) {
                    super::super::profile::decision("composed input work", || {
                        format!(
                            "{rel}: {} filters, {} batches, {} candidate rows, {} columns",
                            delta[0], delta[1], delta[2], delta[3]
                        )
                    });
                }
            }
        }
        #[cfg(any(test, feature = "test-hooks"))]
        {
            experiments::replay(self, db, &values)?;
            let work = crate::code_rule_dsl::facts::vtab::lookup_spike::take_work();
            if work.requests > 0 { super::super::profile::decision("lookup spike", || serde_json::to_string(&work).unwrap()); }
        }
        self.finish(db,f,lowered,hooks,values)
    }

    fn finish(&self, db: &Connection, f: Fields<'_>, lowered: Option<lower::Lowering<'_>>, hooks: &mut Hooks<'_>,
        values: BTreeMap<String,Vec<Vec<SqlValue>>>) -> Result<Outcome> {
        let mut out = Outcome::default();
        for (rel, rows) in values {
            // Session maintenance reads ordinary resident facts, including decoded outputs.
            // Retain their atom rows before decoding instead of expecting a physical SQL table.
            if self.kept.contains(&rel) || lowered.is_some() {
                if !self.resident.contains(&rel) {
                let ints = rows
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|v| {
                                if let SqlValue::Integer(n) = v {
                                    Ok(*n)
                                } else {
                                    Err(Error::Plan(
                                        "retained composed output must contain atoms".into(),
                                    ))
                                }
                            })
                            .collect::<Result<Vec<_>>>()
                    })
                    .collect::<Result<Vec<_>>>()?;
                super::super::native::component::retain(
                    db,
                    &rel,
                    ints,
                    vec![b'i'; f.width(&rel)],
                )?;
                }
                out.kept.insert(rel.clone());
                if let Some(hand) = hooks.hand_off.as_deref_mut().filter(|_|self.kept.contains(&rel)) {
                    if hand(&rel)? {
                        out.handed.insert(rel.clone());
                    }
                }
                if self.kept.contains(&rel) { continue; }
            }
            let domains = lowered.and_then(|l| l.domains.get(&rel));
            // The rows held as their values (decision 0239): a name, kind or field as its text.
            let mut decoded = super::output::Rows::default();
            for row in rows {
                let shown: Vec<super::Shown> =
                    row.into_iter()
                        .enumerate()
                        .map(|(c, v)| match v {
                            SqlValue::Integer(n) => match domains.and_then(|d| d.get(c)) {
                                Some(lower::Domain::Name) => {
                                    super::super::native::symbol_text(db, n).map(super::Shown::Text)
                                }
                                Some(lower::Domain::Kind) => {
                                    let (tag, code) = (
                                        crate::code_rule_dsl::facts::core::kind_tag(n as u32),
                                        crate::code_rule_dsl::facts::core::kind_code(n as u32),
                                    );
                                    let dialect =
                                        crate::code_rule_dsl::language::from_tape_tag(tag)
                                            .unwrap_or(lowered.unwrap().dialect);
                                    Ok(dialect
                                        .kinds()
                                        .kind_name(code)
                                        .map_or(super::Shown::Int(n), |k| super::Shown::Text(k.to_owned())))
                                }
                                Some(lower::Domain::Field) => {
                                    let dialect = crate::code_rule_dsl::language::from_tape_tag(
                                        (n >> 16) as u8,
                                    )
                                    .unwrap_or(lowered.unwrap().dialect);
                                    Ok(dialect
                                        .kinds()
                                        .field_name((n & 0xffff) as u16)
                                        .map_or(super::Shown::Int(n), |f| super::Shown::Text(f.to_owned())))
                                }
                                _ => Ok(super::Shown::Int(n)),
                            },
                            SqlValue::Text(s) => Ok(super::Shown::Text(s)),
                            SqlValue::Null => Ok(super::Shown::Text("None".into())),
                            SqlValue::Real(f) => Ok(super::Shown::Text(format!("{f:?}"))),
                            SqlValue::Blob(b) => Ok(super::Shown::Text(format!("{b:?}"))),
                        })
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                decoded.push(&shown.iter().map(super::Shown::cell).collect::<Vec<_>>());
            }
            out.nodes.insert(
                rel.clone(),
                domains
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .filter_map(|(i, d)| (*d == lower::Domain::Node).then_some(i))
                    .collect(),
            );
            out.outputs.insert(rel, std::sync::Arc::new(decoded));
        }
        Ok(out)
    }
}

#[cfg(test)]
pub mod tests;

#[cfg(any(test, feature = "test-hooks"))]
pub mod experiments;

pub mod stages;
pub mod retained;
pub use retained::run;
