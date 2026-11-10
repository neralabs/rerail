//! Compile relational graphs and linear closures to one query, including batched native leaves.
//! Compilation appends definitions. Only `execute` steps a statement.
use super::super::sql::ident;
use super::*;

thread_local! {static CACHE:RefCell<HashMap<[u8;32],Compiled>>=RefCell::new(HashMap::new());}
/// Whether a run may be one composed query: only when asked (`RERAIL_COMPOSE=1`). The integration branch runs its
/// strata natively, with SQL only where SQL reads a relation (decision 0104), and a composed run passes by its
/// sessions, kept strata and maintenance; the composed form stays for comparison (async-family-integration).
pub(super) fn enabled() -> bool {
    #[cfg(test)]
    if experiments::native_only() {
        return false;
    }
    std::env::var("RERAIL_COMPOSE").is_ok_and(|v| v == "1")
}

pub(super) fn clear() {
    CACHE.with(|c| c.borrow_mut().clear());
}
pub(super) fn planned(prog: &Program, db: &Connection, inputs: Inputs) -> Result<Compiled> {
    use sha2::{Digest, Sha256};
    let key: [u8; 32] = Sha256::digest(
        format!(
            "compose-v2\0{prog:?}\0{}\0{:?}\0{:?}\0{:?}",
            inputs.facts, inputs.atoms, inputs.kept, inputs.batch
        )
        .as_bytes(),
    )
    .into();
    if let Some(plan) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return Ok(plan);
    }
    let _span = super::super::profile::span(|| "compile composed query".into());
    let mut model = make_plan(prog, db, inputs)?;
    let plan = compile(prog, &mut model)?;
    if super::super::profile::on() {
        let mut explain = db.prepare(&format!("EXPLAIN QUERY PLAN {}", plan.sql))?;
        let rows = explain
            .query_map([], |r| r.get::<_, String>(3))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        super::super::profile::decision("composed query plan", || rows.join("\n"));
        super::super::profile::decision("composed query shape", || {
            format!(
                "{} SQL bytes; {} outputs; one evaluation statement",
                plan.sql.len(),
                plan.outputs.len()
            )
        });
    }
    CACHE.with(|c| c.borrow_mut().insert(key, plan.clone()));
    Ok(plan)
}

#[derive(Clone)]
pub(super) struct Compiled {
    pub sql: String,
    outputs: Vec<String>,
    kept: BTreeSet<String>,
}

pub(super) fn accepts(prog: &Program) -> Result<bool> {
    if !prog.lattice.is_empty()
        || !prog.answers.is_empty()
        || !prog.gates.is_empty()
        || prog
            .natives
            .values()
            .any(|n| !matches!(n, super::super::front::Native::Python { .. } | super::super::front::Native::PythonCandidates {..}))
        || prog.outputs.iter().any(|r| r == "finding")
    {
        return Ok(false);
    }
    for scc in strata(prog)?.0 {
        if scc.len() != 1 {
            return Ok(false);
        }
        let rel = scc.iter().next().unwrap();
        for rule in prog.rules.iter().filter(|r| &r.head.rel == rel) {
            let reads = rule
                .body
                .iter()
                .filter_map(Literal::atom)
                .filter(|a| &a.rel == rel)
                .count();
            if reads > 1
                || (reads == 1
                    && (prog.natives.contains_key(rel)
                        || rule.body.iter().any(|l| {
                            matches!(
                                l,
                                Literal::Agg { .. } | Literal::Nearest { .. } | Literal::Ast { .. }
                            )
                        })))
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

pub(super) fn compile(prog: &Program, model: &mut Model) -> Result<Compiled> {
    let outputs: Vec<String> = prog
        .outputs
        .iter()
        .chain(&model.inputs.kept)
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let needed = super::super::front::needed(prog, outputs.clone());
    let mut uses = HashMap::<String, usize>::new();
    for r in prog.rules.iter().filter(|r| needed.contains(&r.head.rel)) {
        for a in r.body.iter().filter_map(Literal::atom) {
            *uses.entry(a.rel.clone()).or_default() += 1;
        }
    }
    for r in &outputs {
        *uses.entry(r.clone()).or_default() += 1;
    }
    let mut frame_counts = BTreeMap::<String, usize>::new();
    for (rel, op) in &prog.natives {
        if needed.contains(rel) {
            if let super::super::front::Native::Python { input, .. } | super::super::front::Native::PythonCandidates {input,..} = op {
                *frame_counts.entry(input.clone()).or_default() += 1;
            }
        }
    }
    let mut frames = BTreeMap::new();
    let mut frame_defs = Vec::new();
    for (input, count) in frame_counts {
        let mut name = format!("__rr_demand_{}", frames.len());
        while prog.decls.contains_key(&name) {
            name.push('_');
        }
        frame_defs.push(format!(
            "{name}(handle) AS MATERIALIZED (SELECT rr_demand_frame(c0) FROM {} HAVING count(*) > 0)",
            ident(&input)
        ));
        if let Some(n) = uses.get_mut(&input) {
            *n = n.saturating_sub(count - 1);
        }
        frames.insert(input, name);
    }
    let keys_only = |rel: &str| {
        frames.contains_key(rel) && !outputs.iter().any(|r|r==rel) && prog.rules.iter().filter(|r|r.body.iter().filter_map(Literal::atom).any(|a|a.rel==rel)).all(|r|matches!(prog.natives.get(&r.head.rel),Some(super::super::front::Native::Python{input,targets,..}) if input==rel && targets.as_deref()!=Some(rel)))
    };
    let mut defs = Vec::new();
    let mut defined = BTreeSet::new();
    let mut add = |rel: &str, query: String, materialized: bool| {
        let columns = (0..prog.decls[rel].len())
            .map(|c| format!("c{c}"))
            .collect::<Vec<_>>()
            .join(",");
        defs.push(format!(
            "{}({columns}) AS {} ({query})",
            ident(rel),
            if materialized {
                "MATERIALIZED"
            } else {
                "NOT MATERIALIZED"
            }
        ));
        defined.insert(rel.to_owned());
    };
    // Constants can be read by any component. A head with rules folds its facts below.
    let has_rules: BTreeSet<_> = prog.rules.iter().map(|r| r.head.rel.as_str()).collect();
    let constants = |rel: &str| -> Vec<String> {
        prog.facts
            .get(rel)
            .into_iter()
            .flatten()
            .map(|row| {
                format!(
                    "SELECT {}",
                    row.iter()
                        .map(|v| match v {
                            Value::Int(n) => n.to_string(),
                            Value::Str(s) => format!("'{}'", s.replace('\'', "''")),
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
            prog.decls[rel]
                .iter()
                .map(|(_, t)| format!(
                    "CAST(NULL AS {})",
                    if t == "sym" && !atoms {
                        "TEXT"
                    } else {
                        "INTEGER"
                    }
                ))
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    for rel in prog.decls.keys().filter(|r| {
        needed.contains(*r) && !prog.inputs.contains(*r) && !has_rules.contains(r.as_str())
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
        );
    }
    for scc in strata(prog)?.0 {
        let rel = scc.iter().next().unwrap();
        if !needed.contains(rel) || prog.inputs.contains(rel) {
            continue;
        }
        let mut rules: Vec<_> = prog.rules.iter().filter(|r| r.head.rel == *rel).collect();
        // SQLite requires every seed before every recursive arm. UNION provides the visited set.
        rules.sort_by_key(|r| {
            r.body
                .iter()
                .filter_map(Literal::atom)
                .any(|a| a.rel == *rel)
        });
        if rules.is_empty() {
            continue;
        }
        let mut arms = constants(rel);
        let producer=match prog.natives.get(rel) {
            Some(super::super::front::Native::Python{kind,input,targets})=>Some((kind.as_str(),input,targets.as_ref(),String::new())),
            Some(super::super::front::Native::PythonCandidates{input,targets,context,member})=>Some(("candidates",input,Some(targets),format!(" AND context='{}' AND member='{}'",context.replace(char::from(39),"''"),member.replace(char::from(39),"''")))),
            _=>None,
        };
        if let Some((kind,input,targets,parameters))=producer {
            let cols = prog.decls[rel]
                .iter()
                .enumerate()
                .map(|(c, (_, ty))| {
                    if ty == "sym" && model.inputs.atoms.is_some() {
                        format!("sym_atom(c{c})")
                    } else {
                        format!("c{c}")
                    }
                })
                .collect::<Vec<_>>()
                .join(",");
            let target = targets
                .as_ref()
                .map(|t| {
                    format!(
                        " AND targets=(SELECT json_group_array({}) FROM {})",
                        if model.inputs.atoms.is_some() {
                            "sym_text(c0)"
                        } else {
                            "c0"
                        },
                        ident(t)
                    )
                })
                .unwrap_or_default();
            arms.push(format!("SELECT DISTINCT {cols} FROM {} AS demanded CROSS JOIN rr_batch_python_{kind} WHERE frame=demanded.handle{target}{parameters}",frames[input]));
        } else {
            for rule in rules {
                let ((order, _, _), _) =
                    model.plan_rule_reused(prog, &rule.head, &rule.body, None)?;
                let mut q = model.checked_query(prog, &rule.head, &order, None, None)?;
                // A frame owns key deduplication. No other consumer observes these projection rows.
                if keys_only(rel) {
                    q.body.distinct = false;
                }
                // Keep the statistics-driven plan's demand anchors. The session-table
                // sqlite_order policy loses native posting selectivity behind these CTEs.
                if !q.ctes.is_empty()
                    && rule
                        .body
                        .iter()
                        .filter_map(Literal::atom)
                        .any(|a| a.rel == *rel)
                {
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
            prog.natives.contains_key(rel) || uses.get(rel).copied().unwrap_or(0) > 1,
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
        .map(|r| prog.decls[r].len())
        .max()
        .unwrap_or(1);
    let arms = outputs
        .iter()
        .enumerate()
        .map(|(id, rel)| {
            let cols = (0..width)
                .map(|c| {
                    if c >= prog.decls[rel].len() {
                        "NULL".into()
                    } else {
                        format!("c{c}")
                    }
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("SELECT {id} AS __output,{cols} FROM {}", ident(rel))
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
            #[cfg(test)]
            let sql = experiments::physical_sql(sql, prog);
            sql
        },
        outputs,
        kept: model.inputs.kept.clone(),
    })
}

impl Compiled {
    pub(super) fn execute(
        &self,
        db: &Connection,
        prog: &Program,
        lowered: Option<&lower::Lowered>,
        hooks: &mut Hooks<'_>,
    ) -> Result<Outcome> {
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
        #[cfg(test)]
        query.reset_status(rusqlite::StatementStatus::RePrepare);
        let mut cursor = query.query([])?;
        while let Some(row) = cursor.next()? {
            let rel = &self.outputs[row.get::<_, i64>(0)? as usize];
            values.get_mut(rel).unwrap().push(
                (0..prog.decls[rel].len())
                    .map(|c| row.get(c + 1))
                    .collect::<rusqlite::Result<_>>()?,
            );
        }
        drop(cursor);
        #[cfg(test)]
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
        #[cfg(test)]
        {
            experiments::replay(self, db, &values)?;
            let work = crate::code_rule_dsl::facts::vtab::lookup_spike::take_work();
            if work.requests > 0 { super::super::profile::decision("lookup spike", || serde_json::to_string(&work).unwrap()); }
        }
        let mut out = Outcome::default();
        for (rel, rows) in values {
            if self.kept.contains(&rel) {
                let ints = rows
                    .into_iter()
                    .map(|row| {
                        row.into_iter()
                            .map(|v| {
                                if let SqlValue::Integer(n) = v {
                                    Ok(n)
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
                    vec![b'i'; prog.decls[&rel].len()],
                )?;
                out.kept.insert(rel.clone());
                if let Some(hand) = hooks.hand_off.as_deref_mut() {
                    hand(&rel)?;
                    out.handed.insert(rel);
                }
                continue;
            }
            let domains = lowered.and_then(|l| l.domains.get(&rel));
            let mut decoded = Vec::new();
            for row in rows {
                decoded.push(
                    row.into_iter()
                        .enumerate()
                        .map(|(c, v)| match v {
                            SqlValue::Integer(n) => match domains.and_then(|d| d.get(c)) {
                                Some(lower::Domain::Name) => {
                                    super::super::native::symbol_text(db, n)
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
                                        .map(str::to_owned)
                                        .unwrap_or_else(|| n.to_string()))
                                }
                                Some(lower::Domain::Field) => {
                                    let dialect = crate::code_rule_dsl::language::from_tape_tag(
                                        (n >> 16) as u8,
                                    )
                                    .unwrap_or(lowered.unwrap().dialect);
                                    Ok(dialect
                                        .kinds()
                                        .field_name((n & 0xffff) as u16)
                                        .map(str::to_owned)
                                        .unwrap_or_else(|| n.to_string()))
                                }
                                _ => Ok(n.to_string()),
                            },
                            SqlValue::Text(s) => Ok(s),
                            SqlValue::Null => Ok("None".into()),
                            SqlValue::Real(f) => Ok(format!("{f:?}")),
                            SqlValue::Blob(b) => Ok(format!("{b:?}")),
                        })
                        .collect::<rusqlite::Result<Vec<_>>>()?
                        .join("\t"),
                );
            }
            decoded.sort();
            decoded.dedup();
            out.nodes.insert(
                rel.clone(),
                domains
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .filter_map(|(i, d)| (*d == lower::Domain::Node).then_some(i))
                    .collect(),
            );
            out.outputs.insert(rel, decoded);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) mod experiments;
