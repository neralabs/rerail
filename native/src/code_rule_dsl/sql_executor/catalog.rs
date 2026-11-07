//! The session's rules and relations as SQL rows (`docs/sql-access/01-design.md` §1-2). After every
//! change to the retained programs, the `rr_*` catalog tables are rebuilt from them, and each
//! declared relation that has rows here gets a typed view `rr_rel_<name>` whose columns are its
//! `.decl` names. The physical relations keep their positional `c0, c1, …` columns.
use super::*;

/// Every object this module owns is named with this prefix, which `reserved` keeps from programs.
pub(super) const PREFIX: &str = "rr_";

const TABLES: &[&str] = &[
    "CREATE TEMP TABLE rr_relation(execution INTEGER NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL, arity INTEGER NOT NULL, stratum INTEGER, output INTEGER NOT NULL, file_local INTEGER NOT NULL, view TEXT, PRIMARY KEY (execution, name)) WITHOUT ROWID",
    "CREATE TEMP TABLE rr_column(execution INTEGER NOT NULL, relation TEXT NOT NULL, ord INTEGER NOT NULL, name TEXT NOT NULL, type TEXT NOT NULL, PRIMARY KEY (execution, relation, ord)) WITHOUT ROWID",
    "CREATE TEMP TABLE rr_rule(rule INTEGER PRIMARY KEY, execution INTEGER NOT NULL, ord INTEGER NOT NULL, head TEXT NOT NULL, text TEXT NOT NULL)",
    "CREATE TEMP TABLE rr_atom(rule INTEGER NOT NULL, ord INTEGER NOT NULL, kind TEXT NOT NULL, relation TEXT, negated INTEGER NOT NULL, aggregate TEXT, text TEXT NOT NULL, PRIMARY KEY (rule, ord)) WITHOUT ROWID",
    "CREATE TEMP TABLE rr_plan(execution INTEGER NOT NULL, step INTEGER NOT NULL, stratum INTEGER NOT NULL, output TEXT NOT NULL, reads TEXT NOT NULL, sql TEXT NOT NULL, PRIMARY KEY (execution, step)) WITHOUT ROWID",
];

/// A relation's role in its program: read from outside, made natively, a lattice, or derived.
fn kind(program: &front::Program, name: &str) -> &'static str {
    if program.inputs.iter().any(|i| i == name) {
        "input"
    } else if program.natives.contains_key(name) {
        "native"
    } else if program.lattice.contains_key(name) {
        "lattice"
    } else {
        "derived"
    }
}

fn literal(l: &front::Literal) -> (&'static str, Option<&str>, bool, Option<String>) {
    match l {
        front::Literal::Atom(a) => ("atom", Some(a.rel.as_str()), a.neg, None),
        front::Literal::Agg { op, atom, .. } => {
            let op = match op {
                front::AggOp::Count => "count",
                front::AggOp::Min => "min",
                front::AggOp::Max => "max",
                front::AggOp::Sum => "sum",
                front::AggOp::Join { .. } => "join",
            };
            ("aggregate", Some(atom.rel.as_str()), atom.neg, Some(op.into()))
        }
        front::Literal::Cmp { .. } => ("compare", None, false, None),
        front::Literal::Nearest { .. } => ("nearest", None, false, None),
        front::Literal::Ast { .. } => ("ast", None, false, None),
    }
}

/// The column names of a typed view: the `.decl` names, unique ignoring case, or `present` for a
/// logically nullary relation's unit column.
fn view_columns(decl: &[(String, String)]) -> Vec<String> {
    if decl.is_empty() {
        return vec!["present".into()];
    }
    let mut seen = BTreeSet::new();
    decl.iter()
        .enumerate()
        .map(|(i, (name, _))| {
            let name = if identifier(name) { name.clone() } else { format!("col{i}") };
            if seen.insert(name.to_ascii_lowercase()) { name } else { format!("{name}_{i}") }
        })
        .collect()
}

fn width(db: &rusqlite::Connection, relation: &str) -> Option<usize> {
    db.prepare(&format!("SELECT * FROM {} LIMIT 0", quote(relation))).ok().map(|s| s.column_count())
}

impl CodeRuleSqlExecutor {
    /// Rebuild the catalog and the typed views from the retained programs.
    pub(super) fn refresh_catalog(&self) -> Result<()> {
        let db = &self.db;
        let owned: Vec<(String, String)> = db
            .prepare("SELECT name, type FROM temp.sqlite_schema WHERE type IN ('table', 'view') AND substr(name, 1, 3) = 'rr_'")
            .map_err(error)?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(error)?
            .collect::<rusqlite::Result<_>>()
            .map_err(error)?;
        for kind in ["view", "table"] {
            for (name, _) in owned.iter().filter(|(_, t)| t == kind) {
                db.execute_batch(&format!("DROP {} IF EXISTS temp.{}", kind.to_ascii_uppercase(), quote(name)))
                    .map_err(error)?;
            }
        }
        for table in TABLES {
            db.execute_batch(table).map_err(error)?;
        }
        let mut relation = db.prepare("INSERT INTO temp.rr_relation VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)").map_err(error)?;
        let mut column = db.prepare("INSERT INTO temp.rr_column VALUES (?1, ?2, ?3, ?4, ?5)").map_err(error)?;
        let mut rule = db.prepare("INSERT INTO temp.rr_rule VALUES (?1, ?2, ?3, ?4, ?5)").map_err(error)?;
        let mut atom = db.prepare("INSERT INTO temp.rr_atom VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)").map_err(error)?;
        let mut plan = db.prepare("INSERT INTO temp.rr_plan VALUES (?1, ?2, ?3, ?4, ?5, ?6)").map_err(error)?;
        // One typed view per relation name: the execution that derives it, else the first that reads it.
        let mut views: BTreeMap<String, (bool, Vec<String>)> = BTreeMap::new();
        let mut rule_id = 0i64;
        for (e, run) in self.executions.iter().enumerate() {
            let program = &run.program;
            let steps = &run.emitted.program.steps;
            for (name, decl) in &program.decls {
                let kind = kind(program, name);
                let stratum = steps.iter().filter(|s| &s.output == name).map(|s| s.stratum).min();
                let columns = view_columns(decl);
                let physical = if program.nullary.contains(name) { 1 } else { decl.len() };
                let view = (width(db, name) == Some(physical)).then(|| format!("{PREFIX}rel_{name}"));
                relation
                    .execute(rusqlite::params![
                        e as i64,
                        name,
                        kind,
                        decl.len() as i64,
                        stratum,
                        run.outputs.contains(name),
                        run.emitted.program.file_local,
                        view
                    ])
                    .map_err(error)?;
                for (i, (col, ty)) in decl.iter().enumerate() {
                    column.execute(rusqlite::params![e as i64, name, i as i64, col, ty]).map_err(error)?;
                }
                if view.is_some() {
                    let derives = kind != "input";
                    match views.get(name) {
                        Some((true, _)) => {}
                        Some((false, _)) if !derives => {}
                        _ => {
                            views.insert(name.clone(), (derives, columns));
                        }
                    }
                }
            }
            for (ord, r) in program.rules.iter().enumerate() {
                rule.execute(rusqlite::params![rule_id, e as i64, ord as i64, r.head.rel, r.to_string()]).map_err(error)?;
                for (i, l) in r.body.iter().enumerate() {
                    let (kind, rel, negated, aggregate) = literal(l);
                    atom.execute(rusqlite::params![rule_id, i as i64, kind, rel, negated, aggregate, l.to_string()])
                        .map_err(error)?;
                }
                rule_id += 1;
            }
            for (i, s) in steps.iter().enumerate() {
                let reads = serde_json::to_string(&s.reads).map_err(error)?;
                plan.execute(rusqlite::params![e as i64, i as i64, s.stratum, s.output, reads, s.sql]).map_err(error)?;
            }
        }
        for (name, (_, columns)) in views {
            let names = columns.iter().map(|c| quote(c)).collect::<Vec<_>>().join(", ");
            db.execute_batch(&format!(
                "CREATE TEMP VIEW {}({names}) AS SELECT * FROM {}",
                quote(&format!("{PREFIX}rel_{name}")),
                quote(&name)
            ))
            .map_err(error)?;
        }
        Ok(())
    }
}
