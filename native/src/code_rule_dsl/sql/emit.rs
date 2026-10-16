//! Emission from the frame IR to SQLite statements. Every frame is a TEMP table with the columns
//! `k, file, node` and then each slot's physical columns, in slot order; an operator is one
//! `INSERT … SELECT` from its input frame's table (with CTEs of its own), so frames combine with
//! `UNION ALL`. Relations are joins with `node` on the axis; a first-match binding is an argmin
//! per `k` (`MIN` with bare columns); terminals are read once per statement through
//! `MATERIALIZED` CTEs or an uncorrelated `IN` subquery.
//!
//! Frames are tables rather than one statement's CTEs because SQLite copies a CTE's whole
//! definition into every reference (`resolveFromTermToCte`), CTEs it references included: a rule
//! whose frames are each read twice would expand exponentially with its depth.

use super::lower::{INTERNAL, ORIGIN};
use super::plan::{
    CallReturn, CallSlot, OfRule, Op, Slot, SlotId, SlotKind, Statement, Stop, Unit, Walk,
};
use std::collections::BTreeSet;
use std::fmt::Write;

/// Where the driver substitutes the statement's file list (`n.file IN (…)` of the root frame).
pub(super) const FILES: &str = "/*FILES*/";
/// Where the driver substitutes the prefix of the statement's own tables.
pub(super) const NAMESPACE: &str = "/*NS*/";
/// Where the driver substitutes the prefix of a `violation.each` enumeration's tables: `x`, the
/// ranges reported per root `(file, root, start, end)`; `a`, the roots still enumerated; `r`, the
/// round.
pub(super) const EACH: &str = "/*EACH*/";
/// A statement that opens a loop: the statements up to the matching `UNTIL` run again while its
/// query (the rest of the `UNTIL` statement) counts a change (`exec`).
pub(super) const LOOP: &str = "/*LOOP*/";
pub(super) const UNTIL: &str = "/*UNTIL*/";
/// A query after a loop: the recursive reads of its last pass, `(unit, key, unit, key)`, which
/// the driver accepts only where they form no cycle (`recursion`).
pub(super) const ACYCLIC: &str = "/*ACYCLIC*/";

/// The prefix of a stage's own tables.
pub(super) fn stage_namespace(stage: usize) -> String {
    format!("s{stage}_")
}

/// A unit's demand table: `(id, file, node, c0…)`, the keys asked for.
fn demand_table(unit: usize) -> String {
    format!("{NAMESPACE}d{unit}")
}

/// A unit's answer table: `(id, c0…)`, the bindings after the body for the keys it matched.
fn answer_table(unit: usize) -> String {
    format!("{NAMESPACE}a{unit}")
}

/// The table a stage's rows are stored in for the batch.
pub(super) fn stage_table(stage: usize) -> String {
    format!("stage_{stage}")
}

/// A clause's statements: the last selects its matching roots and the clause-scope bindings of
/// each, `(file, node, columns of every scope-0 slot)`, ordered by file and root.
pub(super) fn clause(statement: &Statement) -> Vec<String> {
    let mut emitter = Emitter::new(statement);
    let out = emitter.statement(statement);
    let columns: String = statement
        .slots
        .iter()
        .enumerate()
        .filter(|(_, slot)| slot.scope == 0)
        .flat_map(|(id, slot)| slot.columns(id))
        .map(|column| format!(", {column}"))
        .collect();
    emitter.statements.push(format!(
        "SELECT file, node{columns} FROM {out} ORDER BY file, node"
    ));
    emitter.statements
}

/// A clause's statements under `violation.each` (the enumerated capture's scope-0 node slot is
/// `name`), for one round of the enumeration: the statement's matches, the fresh ranges they bind
/// recorded per root (`EACH` `x`) and the roots that bound one kept for the next round (`a`). The
/// last selects what the round reports, as `clause`'s does: in round 0 every match, in a later
/// round the matches that bound a fresh range.
pub(super) fn clause_each(statement: &Statement, name: Option<SlotId>) -> Vec<String> {
    let mut emitter = Emitter::new(statement);
    let out = emitter.statement(statement);
    let columns: Vec<String> = statement
        .slots
        .iter()
        .enumerate()
        .filter(|(_, slot)| slot.scope == 0)
        .flat_map(|(id, slot)| slot.columns(id))
        .collect();
    let of = |alias: &str| -> String {
        columns
            .iter()
            .map(|column| format!(", {alias}.{column}"))
            .collect()
    };
    let bare: String = columns.iter().map(|column| format!(", {column}")).collect();
    let (range, join) = match name {
        Some(slot) => (
            "x.start, x.\"end\"".to_owned(),
            format!(" LEFT JOIN node x ON x.file = o.file AND x.id = o.s{slot}"),
        ),
        None => ("NULL, NULL".to_owned(), String::new()),
    };
    let statements = &mut emitter.statements;
    statements.push(format!(
        "CREATE TEMP TABLE {NAMESPACE}eo(file, node{bare}, es, ee)"
    ));
    statements.push(format!(
        "INSERT INTO {NAMESPACE}eo SELECT o.file, o.node{}, {range} FROM {out} o{join}",
        of("o")
    ));
    statements.push(format!(
        "CREATE TEMP TABLE {NAMESPACE}ef(file, node, es, ee)"
    ));
    statements.push(format!(
        "INSERT INTO {NAMESPACE}ef SELECT e.file, e.node, e.es, e.ee FROM {NAMESPACE}eo e \
         LEFT JOIN {EACH}x x ON x.file = e.file AND x.root = e.node AND x.start = e.es AND x.\"end\" = e.ee \
         WHERE e.es IS NOT NULL AND x.root IS NULL"
    ));
    statements.push(format!(
        "INSERT INTO {EACH}x SELECT file, node, es, ee FROM {NAMESPACE}ef"
    ));
    statements.push(format!("DELETE FROM {EACH}a"));
    statements.push(format!(
        "INSERT INTO {EACH}a SELECT file, node FROM {NAMESPACE}ef"
    ));
    statements.push(format!(
        "SELECT e.file, e.node{} FROM {NAMESPACE}eo e WHERE (SELECT round FROM {EACH}r) = 0 \
         OR EXISTS (SELECT 1 FROM {NAMESPACE}ef f WHERE f.file = e.file AND f.node = e.node) \
         ORDER BY e.file, e.node",
        of("e")
    ));
    emitter.statements
}

/// A capture-free cycle's stage (its units, every one seeded with the nodes of its root kinds):
/// the members' loop, then the first member's matches. `recursive_member` selects another's.
pub(super) fn recursive_stage(statement: &Statement) -> Vec<String> {
    let mut emitter = Emitter::new(statement);
    for (index, unit) in statement.units.iter().enumerate() {
        emitter.unit_tables(index);
        if let Some(seed) = &unit.seed {
            let kinds = match seed {
                None => String::new(),
                Some(kinds) if kinds.is_empty() => " AND 0".to_owned(),
                Some(kinds) => format!(" AND n.kind IN ({})", list(kinds)),
            };
            emitter.statements.push(format!(
                "INSERT INTO {}(file, node) SELECT n.file, n.id FROM node n WHERE n.file IN ({FILES}){kinds}",
                demand_table(index)
            ));
        }
    }
    let sccs: BTreeSet<usize> = statement.units.iter().map(|unit| unit.scc).collect();
    for scc in sccs {
        if statement.units.iter().any(|unit| unit.scc == scc && unit.seed.is_some()) {
            emitter.recursion_loop(scc);
        }
    }
    emitter.statements.push(recursive_member(NAMESPACE, 0));
    emitter.statements
}

/// The matches of a capture-free cycle's member `unit`, from the tables of its stage's loop
/// (`namespace`).
pub(super) fn recursive_member(namespace: &str, unit: usize) -> String {
    let demand = demand_table(unit).replace(NAMESPACE, namespace);
    let answer = answer_table(unit).replace(NAMESPACE, namespace);
    format!("SELECT d.file, d.node FROM {demand} d JOIN {answer} r ON r.id = d.id")
}

/// A stage's statements: the last selects the distinct `(file, node)` its rule matches.
pub(super) fn stage(statement: &Statement) -> Vec<String> {
    let mut emitter = Emitter::new(statement);
    let out = emitter.statement(statement);
    emitter
        .statements
        .push(format!("SELECT DISTINCT file, node FROM {out}"));
    emitter.statements
}

struct Emitter<'s> {
    slots: &'s [Slot],
    units: &'s [Unit],
    /// Every slot's physical columns, in order.
    physical: Vec<String>,
    statements: Vec<String>,
    next: usize,
    /// How many loops the statements being emitted run in: their tables are dropped first.
    looping: usize,
    /// The unit components whose loops are being emitted, innermost last.
    emitting: Vec<usize>,
    /// The units whose bodies are being emitted, innermost last.
    evaluating: Vec<usize>,
    /// The slot holding the demand row a unit's row is evaluated for.
    origin: Option<SlotId>,
}

/// `An+B` at a 1-based position `index`, as `nth_child::Position::holds` computes it.
fn holds(index: &str, step: i32, offset: i32) -> String {
    if step == 0 {
        format!("{index} = {offset}")
    } else {
        format!(
            "(({index}) - ({offset})) / ({step}) >= 0 AND (({index}) - ({offset})) % ({step}) = 0"
        )
    }
}

fn list<T: ToString>(items: &[T]) -> String {
    items
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

impl<'s> Emitter<'s> {
    fn new(statement: &'s Statement) -> Self {
        let slots = &statement.slots;
        let physical = slots
            .iter()
            .enumerate()
            .flat_map(|(id, slot)| slot.columns(id))
            .collect();
        Self {
            slots,
            units: &statement.units,
            physical,
            statements: Vec::new(),
            next: 0,
            looping: 0,
            emitting: Vec::new(),
            evaluating: Vec::new(),
            origin: slots
                .iter()
                .position(|slot| slot.scope == INTERNAL && slot.name == ORIGIN),
        }
    }

    /// Creates the table `name(columns)`; inside a loop, once, emptying it on every pass: the
    /// schema stays as it is, so the loop's statements are prepared once (`exec::execute`).
    fn create(&mut self, name: &str, columns: &str) {
        if self.looping > 0 {
            self.statements.push(format!(
                "CREATE TEMP TABLE IF NOT EXISTS {name}({columns})"
            ));
            self.statements.push(format!("DELETE FROM {name}"));
        } else {
            self.statements
                .push(format!("CREATE TEMP TABLE {name}({columns})"));
        }
    }

    fn name(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{NAMESPACE}{prefix}{}", self.next)
    }

    /// The physical columns, comma-led: `, s0, s1a…`.
    fn physical_list(&self) -> String {
        self.physical
            .iter()
            .map(|column| format!(", {column}"))
            .collect()
    }

    /// The physical columns of `alias`, comma-led: `, alias.s0…`.
    fn physical_of(&self, alias: &str) -> String {
        self.physical
            .iter()
            .map(|column| format!(", {alias}.{column}"))
            .collect()
    }

    /// A frame table filled by `select` (canonical columns, in order), with CTEs `with`.
    fn frame_with(&mut self, prefix: &str, with: &str, select: &str) -> String {
        let name = self.name(prefix);
        let columns = format!("k INTEGER PRIMARY KEY, file, node{}", self.physical_list());
        self.create(&name, &columns);
        self.statements
            .push(format!("{with}INSERT INTO {name} {select}"));
        name
    }

    fn frame(&mut self, prefix: &str, select: &str) -> String {
        self.frame_with(prefix, "", select)
    }

    /// The frame columns of `alias`.
    fn cols(&self, alias: &str) -> String {
        format!(
            "{alias}.k, {alias}.file, {alias}.node{}",
            self.physical_of(alias)
        )
    }

    /// The frame columns of `alias`, with some physical columns replaced by expressions.
    fn select(&self, alias: &str, overrides: &[(String, String)]) -> String {
        let mut select = format!("{alias}.k, {alias}.file, {alias}.node");
        for column in &self.physical {
            match overrides.iter().find(|(name, _)| name == column) {
                Some((_, expr)) => write!(select, ", {expr}"),
                None => write!(select, ", {alias}.{column}"),
            }
            .expect("write to a string");
        }
        select
    }

    /// Every physical column replaced by `alias`'s.
    fn all_from(&self, alias: &str) -> Vec<(String, String)> {
        self.physical
            .iter()
            .map(|column| (column.clone(), format!("{alias}.{column}")))
            .collect()
    }

    fn columns(&self, slot: usize) -> Vec<String> {
        self.slots[slot].columns(slot)
    }

    fn statement(&mut self, statement: &Statement) -> String {
        let nulls: String = self.physical.iter().map(|_| ", NULL").collect();
        let kinds = match &statement.kinds {
            None => String::new(),
            Some(kinds) if kinds.is_empty() => " AND 0".to_owned(),
            Some(kinds) => format!(" AND n.kind IN ({})", list(kinds)),
        };
        let root = self.frame(
            "r",
            &format!(
                "SELECT ROW_NUMBER() OVER (), n.file, n.id{nulls} FROM node n WHERE n.file IN ({FILES}){kinds}"
            ),
        );
        self.op(&statement.op, &root)
    }

    fn op(&mut self, op: &Op, input: &str) -> String {
        match op {
            Op::Kinds(kinds) => {
                let test = match kinds.as_slice() {
                    [kind] => format!("n.kind = {kind}"),
                    kinds => format!("n.kind IN ({})", list(kinds)),
                };
                self.frame(
                    "x",
                    &format!(
                        "SELECT {} FROM {input} f JOIN node n ON n.file = f.file AND n.id = f.node WHERE {test}",
                        self.cols("f")
                    ),
                )
            }
            Op::Regex(term) => self.frame(
                "x",
                &format!(
                    "SELECT {} FROM {input} f WHERE (f.file * 4294967296 + f.node) IN \
                     (SELECT ek FROM text_match WHERE term = {term} AND ek IN (SELECT file * 4294967296 + node FROM {input}))",
                    self.cols("f")
                ),
            ),
            Op::Pattern {
                term,
                env,
                vars,
                reads,
            } => self.pattern(*term, *env, vars, reads, input),
            Op::All(items) => {
                let mut current = input.to_owned();
                for item in items {
                    current = self.op(item, &current);
                }
                current
            }
            Op::Any(branches) => {
                let mut rest = input.to_owned();
                let mut outs = Vec::new();
                for (index, branch) in branches.iter().enumerate() {
                    let out = self.op(branch, &rest);
                    if index + 1 < branches.len() {
                        rest = self.frame(
                            "y",
                            &format!(
                                "SELECT {} FROM {rest} f WHERE f.k NOT IN (SELECT k FROM {out})",
                                self.cols("f")
                            ),
                        );
                    }
                    outs.push(out);
                }
                let union = outs
                    .iter()
                    .map(|out| format!("SELECT {} FROM {out} f", self.cols("f")))
                    .collect::<Vec<_>>()
                    .join(" UNION ALL ");
                self.frame("x", &union)
            }
            Op::Not(inner) => {
                let out = self.op(inner, input);
                self.frame(
                    "x",
                    &format!(
                        "SELECT {} FROM {input} f WHERE f.k NOT IN (SELECT k FROM {out})",
                        self.cols("f")
                    ),
                )
            }
            Op::Never => self.frame("x", &format!("SELECT {} FROM {input} f WHERE 0", self.cols("f"))),
            Op::Relation { walk, sub, binds } => {
                let candidates = self.candidates(*walk, input);
                let matched = self.op(sub, &candidates);
                if *binds {
                    // `ord` is the walk order (negated ids for descending walks): its minimum
                    // is the first match.
                    let with = format!(
                        "WITH w AS (SELECT g.pk AS pk, MIN(g.ord) AS o{} FROM {matched} s JOIN {candidates} g ON g.k = s.k GROUP BY g.pk) ",
                        self.physical_of("s")
                    );
                    let select = format!(
                        "SELECT {} FROM {input} f JOIN w ON w.pk = f.k",
                        self.select("f", &self.all_from("w"))
                    );
                    self.frame_with("x", &with, &select)
                } else {
                    self.frame(
                        "x",
                        &format!(
                            "SELECT {} FROM {input} f WHERE f.k IN (SELECT g.pk FROM {matched} s JOIN {candidates} g ON g.k = s.k)",
                            self.cols("f")
                        ),
                    )
                }
            }
            Op::Count {
                walk,
                sub,
                min,
                max,
                bind,
            } => {
                let candidates = self.candidates(*walk, input);
                let matched = self.op(sub, &candidates);
                let with = format!(
                    "WITH c AS (SELECT g.pk AS pk, COUNT(*) AS n FROM {matched} s JOIN {candidates} g ON g.k = s.k GROUP BY g.pk) "
                );
                let overrides: Vec<(String, String)> = bind
                    .iter()
                    .map(|(slot, offset)| {
                        (format!("s{slot}"), format!("(COALESCE(c.n, 0) + ({offset}))"))
                    })
                    .collect();
                let upper = max.map_or(String::new(), |max| format!(" AND COALESCE(c.n, 0) <= {max}"));
                let select = format!(
                    "SELECT {} FROM {input} f LEFT JOIN c ON c.pk = f.k WHERE COALESCE(c.n, 0) >= {min}{upper}",
                    self.select("f", &overrides)
                );
                self.frame_with("x", &with, &select)
            }
            Op::NthChild {
                step,
                offset,
                reverse,
                of_rule,
            } => self.nth_child(*step, *offset, *reverse, of_rule.as_ref(), input),
            Op::Stage(stage) => {
                let table = stage_table(*stage);
                self.frame(
                    "x",
                    &format!(
                        "SELECT {} FROM {input} f JOIN {table} st ON st.file = f.file AND st.node = f.node",
                        self.cols("f")
                    ),
                )
            }
            Op::Call {
                entry,
                body,
                returns,
            } => self.call(entry, body, returns, input),
            Op::Constrained { rule, constraints } => {
                let mut current = self.op(rule, input);
                for (slot, constraint) in constraints {
                    let column = format!("s{slot}");
                    let passed = self.frame(
                        "y",
                        &format!(
                            "SELECT {} FROM {current} f WHERE f.{column} IS NULL",
                            self.cols("f")
                        ),
                    );
                    let captured = self.frame(
                        "g",
                        &format!(
                            "SELECT f.k, f.file, f.{column}{} FROM {current} f WHERE f.{column} IS NOT NULL",
                            self.physical_of("f")
                        ),
                    );
                    let checked = self.op(constraint, &captured);
                    current = self.frame(
                        "x",
                        &format!(
                            "SELECT {} FROM {checked} c JOIN {current} f ON f.k = c.k UNION ALL SELECT {} FROM {passed} p",
                            self.select("f", &self.all_from("c")),
                            self.cols("p")
                        ),
                    );
                }
                current
            }
            Op::Recur { unit, slots } => self.recur(*unit, slots.as_deref(), input),
            Op::EachRoot { root } => self.frame(
                "x",
                &format!(
                    "SELECT {} FROM {input} f WHERE (SELECT round FROM {EACH}r) = 0 \
                     OR EXISTS (SELECT 1 FROM {EACH}a a WHERE a.file = f.file AND a.root = f.node)",
                    self.select("f", &[(format!("s{root}"), "f.node".to_owned())])
                ),
            ),
            Op::Excluded { name, root } => self.frame(
                "x",
                &format!(
                    "SELECT {} FROM {input} f LEFT JOIN node b ON b.file = f.file AND b.id = f.s{name} \
                     LEFT JOIN {EACH}x e ON e.file = f.file AND e.root = f.s{root} AND e.start = b.start AND e.\"end\" = b.\"end\" \
                     WHERE e.root IS NULL",
                    self.cols("f")
                ),
            ),
            Op::NthCommit { nth, commit } => {
                let positioned = self.op(nth, input);
                let passed = self.frame(
                    "y",
                    &format!(
                        "SELECT {} FROM {input} f WHERE f.k IN (SELECT k FROM {positioned})",
                        self.cols("f")
                    ),
                );
                let committed = self.op(commit, &passed);
                let overrides: Vec<(String, String)> = self
                    .physical
                    .iter()
                    .map(|column| {
                        (
                            column.clone(),
                            format!("CASE WHEN c.k IS NOT NULL THEN c.{column} ELSE f.{column} END"),
                        )
                    })
                    .collect();
                self.frame(
                    "x",
                    &format!(
                        "SELECT {} FROM {passed} f LEFT JOIN {committed} c ON c.k = f.k",
                        self.select("f", &overrides)
                    ),
                )
            }
        }
    }

    /// The physical columns of `slots`, in order.
    fn key_columns(&self, slots: &[SlotId]) -> Vec<String> {
        slots.iter().flat_map(|&slot| self.columns(slot)).collect()
    }

    /// Creates `unit`'s demand and answer tables where they do not exist yet. A demanded key is
    /// `dirty` until its body is evaluated, and again when a key it read changed its answer.
    fn unit_tables(&mut self, unit: usize) {
        let width = self.key_columns(&self.units[unit].key).len();
        let columns: String = (0..width).map(|index| format!(", c{index}")).collect();
        let demand = demand_table(unit);
        let answer = answer_table(unit);
        self.statements.push(format!(
            "CREATE TEMP TABLE IF NOT EXISTS {demand}(id INTEGER PRIMARY KEY, file, node{columns}, dirty INTEGER DEFAULT 1)"
        ));
        self.statements.push(format!(
            "CREATE INDEX IF NOT EXISTS {demand}_at ON {demand}(file, node)"
        ));
        self.statements.push(format!(
            "CREATE TEMP TABLE IF NOT EXISTS {answer}(id INTEGER PRIMARY KEY{columns})"
        ));
    }

    /// A recursive unit's answers at the keys of `input`'s rows (`site`: the frame's slots for the
    /// unit's key; `None`, the key's own): the keys are demanded, the loop of the unit's component
    /// runs where the unit is not being evaluated already, and each row whose key matched takes
    /// the key's answer. Rows whose key did not match are dropped.
    fn recur(&mut self, unit: usize, site: Option<&[SlotId]>, input: &str) -> String {
        let units = self.units;
        let key = &units[unit].key;
        let unit_columns = self.key_columns(key);
        let site_columns = match site {
            Some(site) => self.key_columns(site),
            None => unit_columns.clone(),
        };
        self.unit_tables(unit);
        let demand = demand_table(unit);
        let answer = answer_table(unit);
        let matched: String = site_columns
            .iter()
            .enumerate()
            .map(|(index, column)| format!(" AND d.c{index} IS f.{column}"))
            .collect();
        if units[unit].seed.is_none() {
            let columns: String = (0..site_columns.len())
                .map(|index| format!(", c{index}"))
                .collect();
            let values: String = site_columns
                .iter()
                .map(|column| format!(", f.{column}"))
                .collect();
            self.statements.push(format!(
                "INSERT INTO {demand}(file, node{columns}) SELECT DISTINCT f.file, f.node{values} FROM {input} f \
                 WHERE NOT EXISTS (SELECT 1 FROM {demand} d WHERE d.file = f.file AND d.node = f.node{matched})"
            ));
        }
        let scc = units[unit].scc;
        if !self.emitting.contains(&scc) {
            self.recursion_loop(scc);
        } else if let (Some(&from), Some(origin)) = (self.evaluating.last(), self.origin)
            && units[from].scc == scc
        {
            // A read inside the component's own loop: recorded for the acyclicity check.
            self.statements.push(format!(
                "INSERT INTO {NAMESPACE}e{scc} SELECT DISTINCT {from}, f.s{origin}, {unit}, d.id FROM {input} f \
                 JOIN {demand} d ON d.file = f.file AND d.node = f.node{matched}"
            ));
        }
        let overrides: Vec<(String, String)> = site_columns
            .iter()
            .enumerate()
            .map(|(index, column)| (column.clone(), format!("r.c{index}")))
            .collect();
        self.frame(
            "x",
            &format!(
                "SELECT {} FROM {input} f JOIN {demand} d ON d.file = f.file AND d.node = f.node{matched} \
                 JOIN {answer} r ON r.id = d.id",
                self.select("f", &overrides)
            ),
        )
    }

    /// The loop of the units of component `scc`, semi-naive: each pass evaluates every unit's body
    /// over its dirty keys (new ones, and those that read a key whose answer changed), reading the
    /// answers so far, records what each read and replaces their answers; the loop ends after a
    /// pass that leaves no key dirty. A key that read no changed answer would read the same keys
    /// and answer the same again, so the answers are those of evaluating every key on every pass
    /// (`recursion` proves they are then the matcher's). The reads kept are each key's latest.
    fn recursion_loop(&mut self, scc: usize) {
        let units = self.units;
        let origin = self.origin.map(|slot| format!("s{slot}"));
        let members: Vec<usize> = (0..units.len())
            .filter(|&unit| units[unit].scc == scc)
            .collect();
        for &unit in &members {
            self.unit_tables(unit);
        }
        let reads = format!("{NAMESPACE}e{scc}");
        let changed = format!("{NAMESPACE}c{scc}");
        self.statements.push(format!(
            "CREATE TEMP TABLE IF NOT EXISTS {reads}(su, s, du, d)"
        ));
        self.statements.push(format!(
            "CREATE INDEX IF NOT EXISTS {reads}_to ON {reads}(du, d)"
        ));
        self.statements.push(format!(
            "CREATE INDEX IF NOT EXISTS {reads}_from ON {reads}(su, s)"
        ));
        self.statements.push(format!(
            "CREATE TEMP TABLE IF NOT EXISTS {changed}(u, id)"
        ));
        self.emitting.push(scc);
        self.looping += 1;
        self.statements.push(LOOP.to_owned());
        for &unit in &members {
            let key = &units[unit].key;
            let key_columns = self.key_columns(key);
            let demand = demand_table(unit);
            let answer = answer_table(unit);
            let mut select = String::from("d.id, d.file, d.node");
            for column in &self.physical {
                let value = match key_columns.iter().position(|key| key == column) {
                    Some(index) => format!("d.c{index}"),
                    None if Some(column) == origin.as_ref() => "d.id".to_owned(),
                    None => "NULL".to_owned(),
                };
                write!(select, ", {value}").expect("write to a string");
            }
            let keys = self.frame(
                "u",
                &format!("SELECT {select} FROM {demand} d WHERE d.dirty = 1"),
            );
            self.statements.push(format!(
                "UPDATE {demand} SET dirty = 0 WHERE id IN (SELECT k FROM {keys})"
            ));
            self.statements.push(format!(
                "DELETE FROM {reads} WHERE su = {unit} AND s IN (SELECT k FROM {keys})"
            ));
            self.evaluating.push(unit);
            let out = self.op(&units[unit].body, &keys);
            self.evaluating.pop();
            let fresh = format!("{NAMESPACE}n{unit}");
            let width = key_columns.len();
            let columns: String = (0..width).map(|index| format!(", c{index}")).collect();
            self.create(&fresh, &format!("id INTEGER PRIMARY KEY{columns}"));
            let values: String = key_columns
                .iter()
                .map(|column| format!(", o.{column}"))
                .collect();
            self.statements.push(format!(
                "INSERT INTO {fresh} SELECT o.k{values} FROM {out} o"
            ));
            // The evaluated keys whose answer changed: added, dropped or rebound.
            let before = format!("SELECT * FROM {answer} WHERE id IN (SELECT k FROM {keys})");
            self.statements.push(format!(
                "INSERT INTO {changed} SELECT {unit}, id FROM \
                 (SELECT * FROM {fresh} EXCEPT {before} UNION SELECT * FROM ({before} EXCEPT SELECT * FROM {fresh}))"
            ));
            self.statements.push(format!(
                "DELETE FROM {answer} WHERE id IN (SELECT k FROM {keys})"
            ));
            self.statements
                .push(format!("INSERT INTO {answer} SELECT * FROM {fresh}"));
        }
        for &unit in &members {
            self.statements.push(format!(
                "UPDATE {} SET dirty = 1 WHERE id IN (SELECT e.s FROM {reads} e JOIN {changed} c \
                 ON c.u = e.du AND c.id = e.d WHERE e.su = {unit})",
                demand_table(unit)
            ));
        }
        self.statements.push(format!("DELETE FROM {changed}"));
        let dirty = members
            .iter()
            .map(|&unit| {
                format!(
                    "(SELECT count(*) FROM {} WHERE dirty = 1)",
                    demand_table(unit)
                )
            })
            .collect::<Vec<_>>()
            .join(" + ");
        self.statements.push(format!("{UNTIL}SELECT {dirty}"));
        self.statements
            .push(format!("{ACYCLIC}SELECT su, s, du, d FROM {reads}"));
        self.looping -= 1;
        self.emitting.pop();
    }

    fn pattern(
        &mut self,
        term: usize,
        env: bool,
        vars: &[usize],
        reads: &[usize],
        input: &str,
    ) -> String {
        let mut overrides = Vec::new();
        let mut values = Vec::new();
        for &slot in vars {
            for column in self.columns(slot) {
                overrides.push((column, format!("p.v{}", values.len())));
                values.push(format!("v{}", values.len()));
            }
        }
        let values = values
            .iter()
            .map(|value| format!(", {value}"))
            .collect::<String>();
        if !env {
            let with = format!(
                "WITH p AS MATERIALIZED (SELECT file, node{values} FROM pattern WHERE term = {term}) "
            );
            let select = format!(
                "SELECT {} FROM {input} f JOIN p ON p.file = f.file AND p.node = f.node",
                self.select("f", &overrides)
            );
            return self.frame_with("x", &with, &select);
        }
        let mut key = String::from("f.file || ',' || f.node");
        for &slot in reads {
            let value = match self.slots[slot].kind {
                SlotKind::List => format!("f.s{slot}a || ':' || f.s{slot}b"),
                _ => format!("f.s{slot}"),
            };
            write!(key, " || ',' || COALESCE({value}, '')").expect("write to a string");
        }
        let with = format!(
            "WITH e AS MATERIALIZED (SELECT f.k AS k, {key} AS ek FROM {input} f), \
             p AS MATERIALIZED (SELECT ek{values} FROM pattern WHERE term = {term} AND ek IN (SELECT ek FROM e)) "
        );
        let select = format!(
            "SELECT {} FROM {input} f JOIN e ON e.k = f.k JOIN p ON p.ek = e.ek",
            self.select("f", &overrides)
        );
        self.frame_with("x", &with, &select)
    }

    /// A relation's candidates from each row of `input`: a frame table with a fresh `k` per
    /// candidate and the row's bindings, plus `pk` (the row's `k`) and `ord` (the walk order:
    /// ids, negated for descending walks).
    fn candidates(&mut self, walk: Walk, input: &str) -> String {
        let at = "JOIN node fn ON fn.file = f.file AND fn.id = f.node";
        let simple = |from: &str| {
            format!("SELECT f.k AS pk, c.file AS file, c.id AS node FROM {input} f {from}")
        };
        let mut with = String::new();
        let source = match walk {
            Walk::Has(Stop::Neighbor) => {
                simple("JOIN node c ON c.file = f.file AND c.parent = f.node")
            }
            Walk::Has(Stop::End) => simple(&format!(
                "{at} JOIN node c ON c.file = f.file AND c.id > f.node AND c.id <= fn.\"last\""
            )),
            Walk::Has(Stop::Rule(stage)) => {
                let table = stage_table(stage);
                with = format!(
                    "WITH RECURSIVE v(pk, file, node) AS (\
                     SELECT f.k, c.file, c.id FROM {input} f JOIN node c ON c.file = f.file AND c.parent = f.node \
                     UNION ALL SELECT v.pk, c.file, c.id FROM v LEFT JOIN {table} st ON st.file = v.file AND st.node = v.node \
                     JOIN node c ON c.file = v.file AND c.parent = v.node WHERE st.node IS NULL) "
                );
                "SELECT pk, file, node FROM v".to_owned()
            }
            Walk::Inside(Stop::Neighbor) => simple(&format!(
                "{at} JOIN node c ON c.file = f.file AND c.id = fn.parent"
            )),
            Walk::Inside(Stop::End) => {
                simple("JOIN node c ON c.file = f.file AND c.id < f.node AND c.\"last\" >= f.node")
            }
            Walk::Inside(Stop::Rule(stage)) => {
                with = chain(input, stage, "fn.parent", "parent", false);
                "SELECT pk, file, node FROM v".to_owned()
            }
            Walk::Precedes(Stop::Neighbor) => simple(&format!(
                "{at} JOIN node c ON c.file = f.file AND c.id = fn.next"
            )),
            Walk::Precedes(Stop::End) => simple(&format!(
                "{at} JOIN node c ON c.file = f.file AND c.parent = fn.wpar AND c.id >= fn.anext"
            )),
            Walk::Precedes(Stop::Rule(stage)) => {
                with = chain(input, stage, "fn.anext", "nsib", false);
                "SELECT pk, file, node FROM v".to_owned()
            }
            Walk::Follows(Stop::Neighbor) => simple(&format!(
                "{at} JOIN node c ON c.file = f.file AND c.id = fn.prev"
            )),
            Walk::Follows(Stop::End) => simple(&format!(
                "{at} JOIN node c ON c.file = f.file AND c.parent = fn.wpar AND c.id <= fn.aprev"
            )),
            Walk::Follows(Stop::Rule(stage)) => {
                with = chain(input, stage, "fn.aprev", "prev", false);
                "SELECT pk, file, node FROM v".to_owned()
            }
            Walk::FieldHas(field, Stop::Neighbor) => simple(&format!(
                "JOIN node c ON c.file = f.file AND c.parent = f.node WHERE c.field = {field} AND c.ffirst = 1"
            )),
            Walk::FieldHas(field, Stop::End) => simple(&format!(
                "JOIN node fc ON fc.file = f.file AND fc.parent = f.node \
                 JOIN node c ON c.file = fc.file AND c.id >= fc.id AND c.id <= fc.\"last\" \
                 WHERE fc.field = {field} AND fc.ffirst = 1"
            )),
            Walk::FieldHas(field, Stop::Rule(stage)) => {
                let table = stage_table(stage);
                with = format!(
                    "WITH q AS (SELECT f.k AS pk, c.file AS file, c.id AS node FROM {input} f \
                     JOIN node c ON c.file = f.file AND c.parent = f.node WHERE c.field = {field} AND c.ffirst = 1) "
                );
                format!(
                    "SELECT pk, file, node FROM q UNION ALL SELECT x.pk, c.file, c.id FROM q x \
                     LEFT JOIN {table} st ON st.file = x.file AND st.node = x.node \
                     JOIN node c ON c.file = x.file AND c.parent = x.node WHERE st.node IS NULL"
                )
            }
            Walk::FieldInside(field, Stop::Neighbor) => simple(&format!(
                "{at} JOIN node c ON c.file = f.file AND c.id = fn.parent WHERE fn.field = {field} AND fn.ffirst = 1"
            )),
            Walk::FieldInside(field, Stop::End) => simple(&format!(
                "JOIN node w ON w.file = f.file AND w.id <= f.node AND w.\"last\" >= f.node \
                 JOIN node c ON c.file = w.file AND c.id = w.parent WHERE w.field = {field} AND w.ffirst = 1"
            )),
            Walk::FieldInside(field, Stop::Rule(stage)) => {
                with = chain(input, stage, "fn.parent", "parent", true);
                format!(
                    "SELECT v.pk AS pk, v.file AS file, v.node AS node FROM v \
                     JOIN node w ON w.file = v.file AND w.id = v.src WHERE w.field = {field} AND w.ffirst = 1"
                )
            }
        };
        let order = if walk.descending() {
            "-c.node"
        } else {
            "c.node"
        };
        let name = self.name("g");
        let columns = format!(
            "k INTEGER PRIMARY KEY, file, node{}, pk, ord",
            self.physical_list()
        );
        self.create(&name, &columns);
        self.statements.push(format!(
            "{with}INSERT INTO {name} SELECT ROW_NUMBER() OVER (), c.file, c.node{}, c.pk, {order} \
             FROM ({source}) c JOIN {input} f ON f.k = c.pk",
            self.physical_of("f")
        ));
        name
    }

    fn nth_child(
        &mut self,
        step: i32,
        offset: i32,
        reverse: bool,
        of_rule: Option<&OfRule>,
        input: &str,
    ) -> String {
        match of_rule {
            None => {
                let index = if reverse {
                    "(n.named_n - n.named_ix + 1)"
                } else {
                    "n.named_ix"
                };
                self.frame(
                    "x",
                    &format!(
                        "SELECT {} FROM {input} f JOIN node n ON n.file = f.file AND n.id = f.node \
                         WHERE n.parent IS NOT NULL AND n.named = 1 AND {}",
                        self.cols("f"),
                        holds(index, step, offset)
                    ),
                )
            }
            Some(OfRule::Stage(stage)) => {
                let table = stage_table(*stage);
                let with = format!(
                    "WITH r AS (SELECT n.file AS file, n.id AS node, \
                     ROW_NUMBER() OVER (PARTITION BY n.file, n.parent ORDER BY n.id) AS r, \
                     COUNT(*) OVER (PARTITION BY n.file, n.parent) AS cnt FROM {table} st \
                     JOIN node n ON n.file = st.file AND n.id = st.node WHERE n.named = 1 AND n.parent IS NOT NULL) "
                );
                let index = if reverse { "(r.cnt - r.r + 1)" } else { "r.r" };
                let select = format!(
                    "SELECT {} FROM {input} f JOIN r ON r.file = f.file AND r.node = f.node WHERE {}",
                    self.cols("f"),
                    holds(index, step, offset)
                );
                self.frame_with("x", &with, &select)
            }
            Some(OfRule::Frame(rule)) => {
                let siblings = self.name("g");
                let columns = format!(
                    "k INTEGER PRIMARY KEY, file, node{}, pk, ord",
                    self.physical_list()
                );
                self.create(&siblings, &columns);
                self.statements.push(format!(
                    "INSERT INTO {siblings} SELECT ROW_NUMBER() OVER (), c.file, c.id{}, f.k, c.id FROM {input} f \
                     JOIN node fn ON fn.file = f.file AND fn.id = f.node \
                     JOIN node c ON c.file = f.file AND c.parent = fn.parent WHERE c.named = 1",
                    self.physical_of("f")
                ));
                let matched = self.op(rule, &siblings);
                let with = format!(
                    "WITH r AS (SELECT g.pk AS pk, g.node AS node, \
                     ROW_NUMBER() OVER (PARTITION BY g.pk ORDER BY g.node) AS r, \
                     COUNT(*) OVER (PARTITION BY g.pk) AS cnt{} FROM {matched} s JOIN {siblings} g ON g.k = s.k) ",
                    self.physical_of("s")
                );
                let index = if reverse { "(r.cnt - r.r + 1)" } else { "r.r" };
                let select = format!(
                    "SELECT {} FROM {input} f JOIN r ON r.pk = f.k AND r.node = f.node WHERE {}",
                    self.select("f", &self.all_from("r")),
                    holds(index, step, offset)
                );
                self.frame_with("x", &with, &select)
            }
        }
    }

    /// A parameterized call: the entry frame copies the caller's bindings into the parameters,
    /// the body runs on it, and the return applies `call::call`'s rules: where some parameter the
    /// caller left unbound was bound, every caller name takes its parameters' bindings, which must
    /// have equal texts (an existing binding kept, else the first in the source), and the call
    /// fails where they differ.
    fn call(
        &mut self,
        entry: &[CallSlot],
        body: &Op,
        returns: &[CallReturn],
        input: &str,
    ) -> String {
        let mut overrides = Vec::new();
        for copy in entry {
            let callee = self.columns(copy.callee);
            let caller = self.columns(copy.caller);
            match copy.kind {
                SlotKind::List => {
                    for (to, from) in callee.iter().zip(&caller) {
                        overrides.push((
                            to.clone(),
                            format!("CASE WHEN f.s{}a <> -1 THEN f.{from} END", copy.caller),
                        ));
                    }
                }
                _ => overrides.push((callee[0].clone(), format!("f.{}", caller[0]))),
            }
        }
        let framed = self.frame(
            "i",
            &format!("SELECT {} FROM {input} f", self.select("f", &overrides)),
        );
        let bound = self.op(body, &framed);
        if returns.is_empty() {
            return bound;
        }
        let nonempty = |slot: usize| format!("COALESCE(b.s{slot}a <> -1, 0)");
        let mut joins = String::new();
        let join = |slot: usize, column: &str, joins: &mut String| -> String {
            let alias = format!("j{slot}");
            write!(
                joins,
                " LEFT JOIN node {alias} ON {alias}.file = b.file AND {alias}.id = b.{column}"
            )
            .expect("write to a string");
            alias
        };
        let mut returned = Vec::new();
        let mut ok = Vec::new();
        let mut overrides = Vec::new();
        for ret in returns {
            let caller = ret.caller;
            match ret.kind {
                SlotKind::Node => {
                    let held = join(caller, &format!("s{caller}"), &mut joins);
                    let params: Vec<(usize, String)> = ret
                        .params
                        .iter()
                        .map(|&param| (param, join(param, &format!("s{param}"), &mut joins)))
                        .collect();
                    for (param, _) in &params {
                        returned.push(format!("(b.s{param} IS NOT NULL AND b.s{caller} IS NULL)"));
                    }
                    let reference = format!(
                        "COALESCE({held}.thash{})",
                        params
                            .iter()
                            .map(|(_, alias)| format!(", {alias}.thash"))
                            .collect::<String>()
                    );
                    for (param, alias) in &params {
                        ok.push(format!(
                            "(b.s{param} IS NULL OR {alias}.thash = {reference})"
                        ));
                    }
                    // The first in the source of the bound parameters (by range; ties keep the
                    // earlier parameter).
                    let positions: Vec<(String, String)> = params
                        .iter()
                        .map(|(param, alias)| {
                            (
                                position(alias, &format!("b.s{param} IS NOT NULL")),
                                format!("b.s{param}"),
                            )
                        })
                        .collect();
                    let best = first_of(&positions);
                    overrides.push((
                        format!("s{caller}"),
                        format!("COALESCE(b.s{caller}, {best})"),
                    ));
                }
                SlotKind::List => {
                    for &param in &ret.params {
                        returned.push(format!(
                            "({} AND NOT {})",
                            nonempty(param),
                            nonempty(caller)
                        ));
                    }
                    let chosen_hash = format!(
                        "COALESCE({})",
                        ret.params
                            .iter()
                            .map(|&param| format!(
                                "CASE WHEN {} THEN b.s{param}h END",
                                nonempty(param)
                            ))
                            .chain(["NULL".to_owned()])
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    let reference = format!(
                        "CASE WHEN {} THEN b.s{caller}h ELSE {chosen_hash} END",
                        nonempty(caller)
                    );
                    for &param in &ret.params {
                        ok.push(format!(
                            "(NOT {} OR b.s{param}h = {reference})",
                            nonempty(param)
                        ));
                    }
                    let params: Vec<(usize, String)> = ret
                        .params
                        .iter()
                        .map(|&param| (param, join(param, &format!("s{param}a"), &mut joins)))
                        .collect();
                    // The chosen list: the first in the source by its first node's range.
                    let positions: Vec<(String, String)> = params
                        .iter()
                        .map(|(param, alias)| {
                            (position(alias, &nonempty(*param)), param.to_string())
                        })
                        .collect();
                    let best = first_of(&positions);
                    // A caller name bound to no node takes only a list without named nodes.
                    let chosen_named = params
                        .iter()
                        .map(|(param, _)| format!("WHEN {param} THEN b.s{param}n"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    ok.push(format!(
                        "NOT (COALESCE(b.s{caller}a = -1, 0) AND COALESCE((CASE {best} {chosen_named} END) > 0, 0))"
                    ));
                    for suffix in ["a", "b", "h", "n"] {
                        let picks = params
                            .iter()
                            .map(|(param, _)| format!("WHEN {param} THEN b.s{param}{suffix}"))
                            .collect::<Vec<_>>()
                            .join(" ");
                        overrides.push((
                            format!("s{caller}{suffix}"),
                            format!(
                                "CASE WHEN {} THEN b.s{caller}{suffix} ELSE COALESCE(CASE {best} {picks} END, b.s{caller}{suffix}) END",
                                nonempty(caller)
                            ),
                        ));
                    }
                }
                SlotKind::Count => {
                    for &param in &ret.params {
                        returned.push(format!("(b.s{param} IS NOT NULL AND b.s{caller} IS NULL)"));
                    }
                    let reference = format!(
                        "COALESCE(b.s{caller}{})",
                        ret.params
                            .iter()
                            .map(|param| format!(", b.s{param}"))
                            .collect::<String>()
                    );
                    for &param in &ret.params {
                        ok.push(format!("(b.s{param} IS NULL OR b.s{param} = {reference})"));
                    }
                    overrides.push((format!("s{caller}"), reference));
                }
            }
        }
        let returned = format!("({})", returned.join(" OR "));
        let ok = format!("({})", ok.join(" AND "));
        let overrides: Vec<(String, String)> = overrides
            .into_iter()
            .map(|(column, value)| {
                let kept = format!("CASE WHEN {returned} THEN {value} ELSE b.{column} END");
                (column, kept)
            })
            .collect();
        self.frame(
            "x",
            &format!(
                "SELECT {} FROM {bound} b{joins} WHERE NOT {returned} OR {ok}",
                self.select("b", &overrides)
            ),
        )
    }
}

/// A node's range (`alias`'s) as one integer ordered as `(start, end)` (byte offsets fit in 32
/// bits), where `present`; else NULL.
fn position(alias: &str, present: &str) -> String {
    format!("CASE WHEN {present} THEN {alias}.start * 4294967296 + {alias}.\"end\" END")
}

/// Of `(position, value)` pairs, the value of the least position (the earliest pair on ties), or
/// NULL where no position is present: one expression whose size grows with the square of the
/// pairs, where nesting a comparison per pair would grow exponentially.
fn first_of(pairs: &[(String, String)]) -> String {
    let least = match pairs {
        [(only, _)] => only.clone(),
        _ => format!(
            "min({})",
            pairs
                .iter()
                .map(|(position, _)| format!("COALESCE({position}, 9223372036854775807)"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let arms: String = pairs
        .iter()
        .map(|(position, value)| format!(" WHEN {position} = {least} THEN {value}"))
        .collect();
    format!("(CASE{arms} END)")
}

/// A walk along one link (`parent`, `nsib`, `prev`) from `start`, up to and including the first
/// node of `stage`: the recursive CTE `v(pk, file, node[, src])`, `src` the node each step came
/// from.
fn chain(input: &str, stage: usize, start: &str, link: &str, src: bool) -> String {
    let table = stage_table(stage);
    let (columns, first_src, step_src) = if src {
        ("pk, file, node, src", ", f.node", ", v.node")
    } else {
        ("pk, file, node", "", "")
    };
    format!(
        "WITH RECURSIVE v({columns}) AS (\
         SELECT f.k, fn.file, {start}{first_src} FROM {input} f JOIN node fn ON fn.file = f.file AND fn.id = f.node \
         WHERE {start} IS NOT NULL \
         UNION ALL SELECT v.pk, v.file, n.{link}{step_src} FROM v \
         LEFT JOIN {table} st ON st.file = v.file AND st.node = v.node \
         JOIN node n ON n.file = v.file AND n.id = v.node WHERE st.node IS NULL AND n.{link} IS NOT NULL) "
    )
}
