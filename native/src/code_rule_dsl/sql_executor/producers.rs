//! SQL producers (`docs/sql-access/01-design.md` §3): `.input r(IO=sql, query="…")` makes `r` a
//! relation whose rows come from a SQL query. Rules see only `r`. The query may read ordinary
//! tables and the program's own relations, through their typed views `rr_rel_<name>` or by name;
//! SQLite resolves which ones it reads.
//!
//! `r` becomes a derived relation of the wave program with one step of its own stratum:
//! `INSERT OR IGNORE INTO new_r WITH q(c0, …) AS (<query>) SELECT … FROM q`. Its `symbol`
//! columns are interned on the way in. The strata are then renumbered so that each one's number
//! is above those it reads from (`renumber`): the step runs after every relation it reads is
//! complete, and its readers run after it. A cycle through a producer is refused: SQL is not monotone, so iterating it need not
//! settle.
//!
//! Tuple maintenance recomputes a producer's stratum whole when anything its query reads changes:
//! a relation of the program, or a session input it reads by name (`maintenance::maintain`).
use super::*;

#[derive(Clone, Debug)]
pub(super) struct Producer {
    pub rel: String,
    pub query: String,
}

/// `front::prune`, keeping what SQL producers' queries may read: every relation a query names,
/// directly or as `rr_rel_<name>`. Naming is a superset of reading, so nothing read is dropped.
pub(super) fn prune(mut program: front::Program) -> front::Program {
    let named: BTreeSet<String> = program
        .settings
        .iter()
        .filter(|(k, t)| k == "input" && t.contains("IO=sql"))
        .flat_map(|(_, t)| words(t))
        .collect();
    let keep: Vec<String> = program
        .decls
        .keys()
        .filter(|r| {
            let r = r.to_ascii_lowercase();
            named.contains(&r) || named.contains(&format!("{}rel_{r}", catalog::PREFIX))
        })
        .cloned()
        .collect();
    let outputs = program.outputs.clone();
    program.outputs.extend(keep);
    let mut program = front::prune(program);
    program.outputs = outputs;
    program
}

/// The words of a text, lowercased: its identifiers and anything else that looks like one.
fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
}

/// Take the program's SQL producers out of its inputs: each becomes a derived relation.
pub(super) fn take(program: &mut front::Program) -> Result<Vec<Producer>> {
    let mut producers = Vec::new();
    let mut kept = Vec::new();
    for (key, text) in std::mem::take(&mut program.settings) {
        let parsed = (key == "input")
            .then(|| text.strip_suffix(')').and_then(|s| s.split_once('(')))
            .flatten();
        let Some((rel, params)) = parsed else {
            kept.push((key, text));
            continue;
        };
        let params = plan::corpus::input_parameters(params).map_err(error)?;
        if params.get("IO").map(String::as_str) != Some("sql") {
            kept.push((key, text));
            continue;
        }
        if params.keys().any(|k| k != "IO" && k != "query") {
            return Err(error(format!("{rel}: IO=sql takes only a query")));
        }
        let query = params
            .get("query")
            .filter(|q| !q.trim().is_empty())
            .ok_or_else(|| error(format!("{rel}: IO=sql needs a query")))?;
        let decl = program
            .decls
            .get(rel)
            .ok_or_else(|| error(format!("{rel}: a SQL producer needs a declaration")))?;
        if decl.is_empty() {
            return Err(error(format!("{rel}: a SQL producer needs at least one column")));
        }
        if program.rules.iter().any(|r| r.head.rel == rel) || program.facts.contains_key(rel) {
            return Err(error(format!("{rel}: a SQL producer's rows come only from its query")));
        }
        program.inputs.retain(|i| i != rel);
        producers.push(Producer { rel: rel.to_owned(), query: query.trim().trim_end_matches(';').to_owned() });
    }
    program.settings = kept;
    Ok(producers)
}

/// The program relations each producer reads, as SQLite resolves its query. Inside a savepoint
/// that is rolled back, every wave relation gets a placeholder table and its typed view; an
/// authorizer records the tables the prepared query reads. A relation the emitter made a view
/// counts as the relations that view reads. Each query must give its declaration's columns.
/// Per producer: the wave relations it reads, and every other table or view it reads (lowercased),
/// which tuple maintenance watches for changes.
fn resolve(db: &Database, emitted: &plan::emit::Emitted, program: &front::Program, producers: &[Producer]) -> Result<Vec<(Vec<String>, Vec<String>)>> {
    use rusqlite::hooks::{AuthAction, Authorization};
    let tables: BTreeMap<String, String> =
        emitted.program.relations.iter().map(|(r, _)| (r.to_ascii_lowercase(), r.clone())).collect();
    let views: BTreeMap<String, &select::Query> =
        emitted.views.iter().map(|(r, q)| (r.to_ascii_lowercase(), q)).collect();
    db.execute_batch("SAVEPOINT rr_producer_reads").map_err(error)?;
    let result = (|| -> Result<Vec<(Vec<String>, Vec<String>)>> {
        for (rel, arity) in &emitted.program.relations {
            let cols = (0..*arity).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", ");
            db.execute_batch(&format!("CREATE TEMP TABLE IF NOT EXISTS {}({cols})", quote(rel))).map_err(error)?;
            if let Some(decl) = program.decls.get(rel) {
                let view = quote(&format!("{}rel_{rel}", catalog::PREFIX));
                let names = catalog::view_columns(decl).iter().map(|c| quote(c)).collect::<Vec<_>>().join(", ");
                db.execute_batch(&format!("DROP VIEW IF EXISTS temp.{view}; CREATE TEMP VIEW {view}({names}) AS SELECT * FROM {}", quote(rel)))
                    .map_err(error)?;
            }
        }
        for statement in &emitted.setup {
            if statement.starts_with("CREATE TEMP TABLE rdl_int(") {
                constants::integer_domain(db)?;
                continue;
            }
            // The emitter's views, so a query may name them; anything already present is kept.
            let _ = db.execute_batch(statement);
        }
        let mut out = Vec::new();
        for p in producers {
            let seen = Arc::new(parking_lot::Mutex::new(BTreeSet::<String>::new()));
            let record = Arc::clone(&seen);
            db.authorizer(Some(move |context: rusqlite::hooks::AuthContext<'_>| {
                if let AuthAction::Read { table_name, .. } = context.action {
                    record.lock().insert(table_name.to_ascii_lowercase());
                }
                Authorization::Allow
            }))
            .map_err(error)?;
            let prepared = db.prepare(&format!("SELECT * FROM ({})", p.query)).map(|s| s.column_count());
            db.authorizer(None::<fn(rusqlite::hooks::AuthContext<'_>) -> Authorization>).map_err(error)?;
            let width = prepared.map_err(|e| error(format!("{}: SQL producer query: {e}", p.rel)))?;
            if width != program.decls[&p.rel].len() {
                return Err(error(format!("{}: the query gives {width} columns, the declaration {}", p.rel, program.decls[&p.rel].len())));
            }
            let mut reads = BTreeSet::new();
            let mut other = BTreeSet::new();
            let mut pending: Vec<String> = seen.lock().iter().cloned().collect();
            let mut visited = BTreeSet::new();
            while let Some(name) = pending.pop() {
                let name = name.strip_prefix(&format!("{}rel_", catalog::PREFIX)).map(str::to_owned).unwrap_or(name);
                if !visited.insert(name.clone()) {
                    continue;
                }
                if let Some(rel) = tables.get(&name) {
                    reads.insert(rel.clone());
                } else if let Some(q) = views.get(&name) {
                    pending.extend(q.reads().into_iter().filter_map(|r| r.program_relation().map(str::to_ascii_lowercase)));
                } else if !name.starts_with("sqlite_") {
                    other.insert(name);
                }
            }
            out.push((reads.into_iter().collect(), other.into_iter().collect()));
        }
        Ok(out)
    })();
    db.execute_batch("ROLLBACK TO rr_producer_reads; RELEASE rr_producer_reads").map_err(error)?;
    result
}

/// The relations whose rows depend on `rel` through the program's rules and producers, `rel`
/// included.
fn downstream(program: &front::Program, producers: &[Producer], reads: &[Vec<String>], rel: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::from([rel.to_owned()]);
    loop {
        let before = out.len();
        for r in &program.rules {
            if r.body.iter().filter_map(front::Literal::atom).any(|a| out.contains(&a.rel)) {
                out.insert(r.head.rel.clone());
            }
        }
        for (p, reads) in producers.iter().zip(reads) {
            if reads.iter().any(|r| out.contains(r)) {
                out.insert(p.rel.clone());
            }
        }
        if out.len() == before {
            return out;
        }
    }
}

/// Add each producer's step to the emitted program, and the typed views its query may read.
/// Returns what each producer reads beyond the wave relations (`resolve`), by producer.
pub(super) fn steps(db: &Database, emitted: &mut plan::emit::Emitted, program: &front::Program, producers: &[Producer]) -> Result<BTreeMap<String, Vec<String>>> {
    if producers.is_empty() {
        return Ok(BTreeMap::new());
    }
    for p in producers {
        if !emitted.program.relations.iter().any(|(r, _)| *r == p.rel) {
            return Err(error(format!("{}: the SQL producer's relation is not a wave relation", p.rel)));
        }
    }
    let resolved = resolve(db, emitted, program, producers)?;
    let all_reads: Vec<Vec<String>> = resolved.iter().map(|(reads, _)| reads.clone()).collect();
    let mut stratum = emitted.program.steps.iter().map(|s| s.stratum).max().unwrap_or(0);
    for (p, reads) in producers.iter().zip(&all_reads) {
        let decl = &program.decls[&p.rel];
        let below = downstream(program, producers, &all_reads, &p.rel);
        if let Some(cycle) = reads.iter().find(|r| below.contains(*r)) {
            return Err(error(format!(
                "{}: a SQL producer cannot read {cycle}, which depends on it (no recursion through SQL)",
                p.rel
            )));
        }
        let columns: Vec<String> = (0..decl.len()).map(|i| format!("c{i}")).collect();
        let values: Vec<String> = decl
            .iter()
            .zip(&columns)
            .map(|((_, ty), c)| if ty == "sym" { format!("sym_atom({c})") } else { c.clone() })
            .collect();
        stratum += 1;
        emitted.program.steps.push(schedule::Step {
            output: p.rel.clone(),
            stratum,
            reads: reads.clone(),
            sql: format!(
                "INSERT OR IGNORE INTO new_{} WITH q({}) AS ({}) SELECT {} FROM q",
                p.rel,
                columns.join(", "),
                p.query,
                values.join(", ")
            ),
        });
        emitted.queries.push(None);
        for rel in reads {
            if let Some(decl) = program.decls.get(rel) {
                let view = quote(&format!("{}rel_{rel}", catalog::PREFIX));
                let names = catalog::view_columns(decl).iter().map(|c| quote(c)).collect::<Vec<_>>().join(", ");
                emitted.setup.push(format!("DROP VIEW IF EXISTS temp.{view}"));
                emitted.setup.push(format!("CREATE TEMP VIEW {view}({names}) AS SELECT * FROM {}", quote(rel)));
            }
        }
    }
    renumber(&mut emitted.program.steps)?;
    Ok(producers.iter().zip(resolved).map(|(p, (_, other))| (p.rel.clone(), other)).collect())
}

/// Renumber the strata so that a stratum's number is above every stratum it reads from. A
/// producer's stratum starts above all the rules', though rules may read it; maintenance and
/// `Program::strata` walk strata by number. The steps keep their order; ties keep the old order.
fn renumber(steps: &mut [schedule::Step]) -> Result<()> {
    let mut makes = BTreeMap::<&str, BTreeSet<u32>>::new();
    for s in steps.iter() {
        makes.entry(s.output.as_str()).or_default().insert(s.stratum);
    }
    let mut after = BTreeMap::<u32, BTreeSet<u32>>::new();
    for s in steps.iter() {
        let before = after.entry(s.stratum).or_default();
        for r in &s.reads {
            let r = r.strip_prefix("delta_").unwrap_or(r);
            before.extend(makes.get(r).into_iter().flatten().filter(|&&b| b != s.stratum));
        }
    }
    let first = after.keys().next().copied().unwrap_or(0);
    let mut placed = BTreeMap::<u32, u32>::new();
    while placed.len() < after.len() {
        let next = after
            .iter()
            .find(|(s, before)| !placed.contains_key(s) && before.iter().all(|b| placed.contains_key(b)))
            .map(|(s, _)| *s)
            .ok_or_else(|| error("SQL producers: the strata read each other in a cycle"))?;
        placed.insert(next, first + placed.len() as u32);
    }
    for s in steps.iter_mut() {
        s.stratum = placed[&s.stratum];
    }
    Ok(())
}
