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
//!
//! A frame that numbers its rows afresh inserts NULL as `k`, so SQLite assigns the next rowid and
//! appends: a `ROW_NUMBER() OVER ()` would buffer every row in an ephemeral table first and
//! insert them by seeking.

mod each;
mod keyed;
mod leaves;
mod three;

use super::lower::{INTERNAL, ORIGIN};
use super::nearest;
use super::plan::{
    CallReturn, CallSlot, OfRule, Op, Slot, SlotId, SlotKind, Source, Statement, Stop, Unit, Walk,
};
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write;
use std::rc::Rc;

/// Where the driver substitutes the statement's whole files (`n.file IN (…)` of the root frame).
pub(super) const FILES: &str = "/*FILES*/";
/// Where the driver substitutes the statement's selected roots, `(file, node)` rows, in files
/// whose roots an anchor index narrowed (the root frame's second branch).
pub(super) const ROOTS: &str = "/*ROOTS*/";
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
/// A recursion component's loop is emitted once, as a block: `BLOCK<scc>` … `END` at its first
/// reference site, which runs it there, and `RUN<scc>` at every later one, which runs the same
/// statements again over the demand rows that site added (`Emitter::recur`). The loop's
/// statements read only its units' demand and answer tables and its own frames, which it empties
/// before it writes them, so a run computes what a copy of the loop emitted at the site would.
pub(super) const BLOCK: &str = "/*BLOCK*/";
pub(super) const END: &str = "/*END*/";
pub(super) const RUN: &str = "/*RUN*/";

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

/// When a statement's output is empty, read from the rows its `Effect`'s inputs hold for the
/// batch (`skip`, which documents how the session applies each). The emitter records it as it
/// writes the statement, from the operator and the frames it reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Rule {
    /// Never passed over: the statement reads no counted frame (a root frame, a seed), or its
    /// output does not follow from its inputs' emptiness.
    #[default]
    AlwaysRun,
    /// Empty where any input is: each output row stems from a row of every input (joins,
    /// semi-joins, relations, terminals over an input set, or a frame derived from them).
    EmptyIfAnyInputEmpty,
    /// Empty where every input is: a union whose branches each read one of them.
    EmptyIfAllInputsEmpty,
    /// The rows of the first input the others do not remove (an anti-join, `not`, `any`'s
    /// remainder): empty where it is; where every other input is empty, exactly its rows, so the
    /// statement still runs.
    EmptyIfPrimaryEmpty,
    /// Exactly the rows of its one input.
    CopyOf,
}

/// How a statement changes the rows of a counted table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum Writes<T> {
    /// No table's rows (a query, an `UPDATE`, a marker).
    #[default]
    None,
    /// Adds the rows it changes.
    Insert(T),
    /// Removes the rows it changes.
    Delete(T),
    /// Empties it.
    Clear(T),
}

impl<T> Writes<T> {
    fn map<U>(self, mut table: impl FnMut(T) -> U) -> Writes<U> {
        match self {
            Self::None => Writes::None,
            Self::Insert(name) => Writes::Insert(table(name)),
            Self::Delete(name) => Writes::Delete(table(name)),
            Self::Clear(name) => Writes::Clear(table(name)),
        }
    }
}

/// What one statement reads and writes, recorded at emission: its emptiness rule over `inputs`
/// (the first is the primary input of `EmptyIfPrimaryEmpty` and `CopyOf`), and its output. Tables
/// are named as the statement names them (`NAMESPACE`, `EACH` unsubstituted); the session maps
/// them to its counted tables. A `BLOCK` marker's inputs are its units' demand tables.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Effect<T = String> {
    pub(super) rule: Rule,
    pub(super) inputs: Vec<T>,
    pub(super) writes: Writes<T>,
}

impl<T> Effect<T> {
    /// The effect over other table names (or ids).
    pub(super) fn map<U>(self, mut table: impl FnMut(T) -> U) -> Effect<U> {
        Effect {
            rule: self.rule,
            inputs: self.inputs.into_iter().map(&mut table).collect(),
            writes: self.writes.map(table),
        }
    }
}

/// A frame statement's rule over the frames it reads; `Reads::writes` adds its output.
pub(super) struct Reads {
    rule: Rule,
    inputs: Vec<String>,
}

impl Reads {
    fn new(rule: Rule, inputs: &[&str]) -> Self {
        Self {
            rule,
            inputs: inputs.iter().map(|&input| input.to_owned()).collect(),
        }
    }

    /// `Rule::EmptyIfAnyInputEmpty`.
    pub(super) fn join(inputs: &[&str]) -> Self {
        Self::new(Rule::EmptyIfAnyInputEmpty, inputs)
    }

    /// `Rule::EmptyIfAllInputsEmpty`.
    pub(super) fn union(inputs: &[&str]) -> Self {
        Self::new(Rule::EmptyIfAllInputsEmpty, inputs)
    }

    /// `Rule::EmptyIfPrimaryEmpty`: `primary`'s rows that no row of `probes` removes.
    pub(super) fn anti(primary: &str, probes: &[&str]) -> Self {
        let mut reads = Self::new(Rule::EmptyIfPrimaryEmpty, probes);
        reads.inputs.insert(0, primary.to_owned());
        reads
    }

    /// `Rule::CopyOf`.
    pub(super) fn copy(input: &str) -> Self {
        Self::new(Rule::CopyOf, &[input])
    }

    /// `Rule::AlwaysRun`.
    pub(super) fn always() -> Self {
        Self::new(Rule::AlwaysRun, &[])
    }

    pub(super) fn writes(self, writes: Writes<String>) -> Effect {
        Effect {
            rule: self.rule,
            inputs: self.inputs,
            writes,
        }
    }
}

/// A stage's or clause's statements, each with its `Effect`, and its root frame: the frame of
/// the roots it runs on, which the session reads to pass over the rest where it is empty.
#[derive(Clone, Debug, Default)]
pub(super) struct Emitted {
    pub(super) statements: Vec<String>,
    pub(super) effects: Vec<Effect>,
    pub(super) root: Option<String>,
}

impl Emitted {
    fn push(&mut self, sql: String, effect: Effect) {
        self.statements.push(sql);
        self.effects.push(effect);
    }
}

/// A clause's statements: the last selects its matching roots and the clause-scope bindings of
/// each, `(file, node, columns of every scope-0 slot)`, ordered by file and root.
pub(super) fn clause(statement: &Statement) -> Emitted {
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
    // An unknown root does not match.
    let known = if emitter.three { " WHERE unk = 0" } else { "" };
    emitter.push(
        format!("SELECT file, node{columns} FROM {out}{known} ORDER BY file, node"),
        Reads::join(&[&out]).writes(Writes::None),
    );
    emitter.out
}

/// A clause's statements under `violation.each` (the enumerated capture's scope-0 node slot is
/// `name`), for one round of the enumeration: the statement's matches, the fresh ranges they bind
/// recorded per root (`EACH` `x`) and the roots that bound one kept for the next round (`a`). The
/// last selects what the round reports, as `clause`'s does: in round 0 every match, in a later
/// round the matches that bound a fresh range.
pub(super) fn clause_each(statement: &Statement, name: Option<SlotId>) -> Emitted {
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
    // A root whose verdict is unknown does not match (`run.rs` `enumerate`): it is neither
    // reported nor enumerated further.
    let known = if emitter.three {
        " WHERE o.unk = 0"
    } else {
        ""
    };
    let (eo, ef, x, a) = (
        format!("{NAMESPACE}eo"),
        format!("{NAMESPACE}ef"),
        format!("{EACH}x"),
        format!("{EACH}a"),
    );
    emitter.push(
        format!("CREATE TEMP TABLE {eo}(file, node{bare}, es, ee)"),
        Effect::default(),
    );
    emitter.push(
        format!(
            "INSERT INTO {eo} SELECT o.file, o.node{}, {range} FROM {out} o{join}{known}",
            of("o")
        ),
        Reads::join(&[&out]).writes(Writes::Insert(eo.clone())),
    );
    emitter.push(
        format!("CREATE TEMP TABLE {ef}(file, node, es, ee)"),
        Effect::default(),
    );
    // The ranges not reported yet: an anti-join with `x`, but filtered too (`es IS NOT NULL`).
    emitter.push(
        format!(
            "INSERT INTO {ef} SELECT e.file, e.node, e.es, e.ee FROM {eo} e \
             LEFT JOIN {x} x ON x.file = e.file AND x.root = e.node AND x.start = e.es AND x.\"end\" = e.ee \
             WHERE e.es IS NOT NULL AND x.root IS NULL"
        ),
        Reads::join(&[&eo]).writes(Writes::Insert(ef.clone())),
    );
    emitter.push(
        format!("INSERT INTO {x} SELECT file, node, es, ee FROM {ef}"),
        Reads::copy(&ef).writes(Writes::Insert(x.clone())),
    );
    emitter.push(
        format!("DELETE FROM {a}"),
        Reads::join(&[&a]).writes(Writes::Clear(a.clone())),
    );
    emitter.push(
        format!("INSERT INTO {a} SELECT file, node FROM {ef}"),
        Reads::join(&[&ef]).writes(Writes::Insert(a.clone())),
    );
    emitter.push(
        format!(
            "SELECT e.file, e.node{} FROM {eo} e WHERE (SELECT round FROM {EACH}r) = 0 \
             OR (e.file, e.node) IN (SELECT file, node FROM {ef}) \
             ORDER BY e.file, e.node",
            of("e")
        ),
        Reads::join(&[&eo]).writes(Writes::None),
    );
    emitter.out
}

/// A stage's statements: the last selects the distinct `(file, node)` its rule matches; in a
/// three-valued statement `(file, node, unk)`, where it is true or unknown.
pub(super) fn stage(statement: &Statement) -> Emitted {
    let mut emitter = Emitter::new(statement);
    let out = emitter.statement(statement);
    let select = if emitter.three {
        format!("SELECT file, node, MIN(unk) FROM {out} GROUP BY file, node")
    } else {
        format!("SELECT DISTINCT file, node FROM {out}")
    };
    emitter.push(select, Reads::join(&[&out]).writes(Writes::None));
    emitter.out
}

struct Emitter<'s> {
    slots: &'s [Slot],
    units: &'s [Unit],
    /// The terminals' related-node walks (`Walk::Related`).
    sources: &'s [Source],
    /// Every slot's physical columns, in order.
    physical: Vec<String>,
    /// The statements so far, each with its effect.
    out: Emitted,
    next: usize,
    /// Whether the statement is three-valued (`Op::Unknowable`): frames then carry `unk` and
    /// `masks` withheld-mask columns.
    three: bool,
    masks: usize,
    /// The stop-rule stages whose verdict may be unknown.
    open: Vec<usize>,
    /// How many loops the statements being emitted run in: their tables are dropped first.
    looping: usize,
    /// The unit components whose loops are being emitted, innermost last.
    emitting: Vec<usize>,
    /// The unit components whose loop is emitted already, as a block later sites run (`RUN`).
    blocks: BTreeSet<usize>,
    /// The units whose bodies are being emitted, innermost last.
    evaluating: Vec<usize>,
    /// The slot holding the demand row a unit's row is evaluated for.
    origin: Option<SlotId>,
    /// The frames known to hold one row per `(file, node, slots of a set)`, with each such set
    /// (`keyed`).
    unique: HashMap<String, Vec<BTreeSet<SlotId>>>,
    /// The slots each operator names, by its address (`keyed::touched`).
    touched_slots: HashMap<usize, Rc<BTreeSet<SlotId>>>,
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
            sources: &statement.sources,
            physical,
            out: Emitted::default(),
            next: 0,
            three: false,
            masks: 0,
            open: Vec::new(),
            looping: 0,
            emitting: Vec::new(),
            blocks: BTreeSet::new(),
            evaluating: Vec::new(),
            origin: slots
                .iter()
                .position(|slot| slot.scope == INTERNAL && slot.name == ORIGIN),
            unique: HashMap::new(),
            touched_slots: HashMap::new(),
        }
    }

    /// Emits `sql`, which does `effect`.
    fn push(&mut self, sql: String, effect: Effect) {
        self.out.push(sql, effect);
    }

    /// Creates the table `name(columns)`; inside a loop, once, emptying it on every pass: the
    /// schema stays as it is, so the loop's statements are prepared once (`exec::execute`).
    fn create(&mut self, name: &str, columns: &str) {
        if self.looping > 0 {
            self.push(
                format!("CREATE TEMP TABLE IF NOT EXISTS {name}({columns})"),
                Effect::default(),
            );
            self.push(
                format!("DELETE FROM {name}"),
                Reads::join(&[name]).writes(Writes::Clear(name.to_owned())),
            );
        } else {
            self.push(
                format!("CREATE TEMP TABLE {name}({columns})"),
                Effect::default(),
            );
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

    /// Emits `sql`, an `INSERT` into `table`, empty as `reads` says.
    fn insert(&mut self, table: &str, sql: String, reads: Reads) {
        self.push(sql, reads.writes(Writes::Insert(table.to_owned())));
    }

    /// A frame table filled by `select` (canonical columns, in order), with CTEs `with`; empty
    /// as `reads` says.
    fn frame_with(&mut self, prefix: &str, with: &str, select: &str, reads: Reads) -> String {
        let name = self.name(prefix);
        let columns = format!(
            "k INTEGER PRIMARY KEY, file, node{}{}",
            self.physical_list(),
            self.extra_list()
        );
        self.create(&name, &columns);
        self.insert(&name, format!("{with}INSERT INTO {name} {select}"), reads);
        name
    }

    fn frame(&mut self, prefix: &str, select: &str, reads: Reads) -> String {
        self.frame_with(prefix, "", select, reads)
    }

    /// A table of per-row results keyed by the row's `k` (`pk INTEGER PRIMARY KEY`, then
    /// `columns`), filled by `select`: a frame joins it on `k` through its primary key, whatever
    /// side of the join the frame is on (a grouped CTE has no index, and a `LEFT JOIN` to one
    /// rescans it per row).
    fn grouped(&mut self, columns: &str, select: &str, reads: Reads) -> String {
        let name = self.name("h");
        self.create(&name, &format!("pk INTEGER PRIMARY KEY, {columns}"));
        self.insert(&name, format!("INSERT INTO {name} {select}"), reads);
        name
    }

    /// The frame columns of `alias`.
    fn cols(&self, alias: &str) -> String {
        format!(
            "{alias}.k, {alias}.file, {alias}.node{}{}",
            self.physical_of(alias),
            self.zeros()
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
        select.push_str(&self.zeros());
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

    /// A three-valued frame's columns after the slots, comma-led: `, unk, wh0…`; empty in a
    /// two-valued statement.
    fn extra_list(&self) -> String {
        if !self.three {
            return String::new();
        }
        let mut list = String::from(", unk");
        for word in 0..self.masks {
            write!(list, ", wh{word}").expect("write to a string");
        }
        list
    }

    /// The values of `extra_list` for a row an operator emits as true: `, 0, 0…`.
    fn zeros(&self) -> String {
        if !self.three {
            return String::new();
        }
        ", 0".repeat(1 + self.masks)
    }

    /// `select` with the verdict `unk` (an expression) and no withheld capture: the row of a
    /// three-valued operator.
    fn verdict(&self, alias: &str, overrides: &[(String, String)], unk: &str) -> String {
        let mut select = format!("{alias}.k, {alias}.file, {alias}.node");
        for column in &self.physical {
            match overrides.iter().find(|(name, _)| name == column) {
                Some((_, expr)) => write!(select, ", {expr}"),
                None => write!(select, ", {alias}.{column}"),
            }
            .expect("write to a string");
        }
        write!(select, ", {unk}{}", ", 0".repeat(self.masks)).expect("write to a string");
        select
    }

    /// Reads whether `statement` is three-valued (`Op::Unknowable`, with its mask columns and
    /// open stop stages); returns the operator under it.
    fn configure<'o>(&mut self, statement: &'o Statement) -> &'o Op {
        match &statement.op {
            Op::Unknowable { op, masks, open } => {
                self.three = true;
                self.masks = *masks;
                self.open.clone_from(open);
                op
            }
            op => op,
        }
    }

    fn statement(&mut self, statement: &Statement) -> String {
        let op = self.configure(statement);
        let nulls: String = self.physical.iter().map(|_| ", NULL").collect();
        let kinds = match &statement.kinds {
            None => String::new(),
            Some(kinds) if kinds.is_empty() => " AND 0".to_owned(),
            Some(kinds) => format!(" AND n.kind IN ({})", list(kinds)),
        };
        // Every root of the clause's kinds in the whole files, then the selected roots of those
        // kinds in the narrowed files. It reads the batch's own rows (`FILES`, `ROOTS`), which the
        // session does not count: it always runs, and every later statement reads what it holds.
        let root = self.frame(
            "r",
            &format!(
                "SELECT NULL, file, id{nulls}{} FROM (\
                 SELECT n.file AS file, n.id AS id FROM node n WHERE n.file IN ({FILES}){kinds} \
                 UNION ALL SELECT n.file, n.id FROM ({ROOTS}) r CROSS JOIN node n \
                 ON n.file = r.file AND n.id = r.node WHERE 1{kinds})",
                self.zeros()
            ),
            Reads::always(),
        );
        self.out.root = Some(root.clone());
        // One row per root: a file's roots are either all its nodes or the selected ones.
        self.keyed_by(&root, vec![BTreeSet::new()]);
        self.op(op, &root)
    }

    /// `op` on `input`: per distinct key where it walks over a frame that may repeat a key
    /// (`keyed`), else directly.
    fn op(&mut self, op: &Op, input: &str) -> String {
        let out = match self.keyed(op, input) {
            Some(out) => out,
            None => self.apply(op, input),
        };
        let changed = self.touched(op);
        self.inherit(input, &out, &changed);
        out
    }

    fn apply(&mut self, op: &Op, input: &str) -> String {
        if self.three
            && let Some(out) = self.op3(op, input)
        {
            return out;
        }
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
                    Reads::join(&[input]),
                )
            }
            Op::Regex(term) => self.frame(
                "x",
                &format!(
                    "SELECT {} FROM {input} f WHERE (f.file * 4294967296 + f.node) IN \
                     (SELECT ek FROM text_match WHERE term = {term} AND ek IN (SELECT file * 4294967296 + node FROM {input}))",
                    self.cols("f")
                ),
                Reads::join(&[input]),
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
                        // `rest` without the rows the branch matched: all of it where none did.
                        let next = self.frame(
                            "y",
                            &format!(
                                "SELECT {} FROM {rest} f WHERE f.k NOT IN (SELECT k FROM {out})",
                                self.cols("f")
                            ),
                            Reads::anti(&rest, &[&out]),
                        );
                        self.inherit(&rest, &next, &BTreeSet::new());
                        rest = next;
                    }
                    outs.push(out);
                }
                let union = outs
                    .iter()
                    .map(|out| format!("SELECT {} FROM {out} f", self.cols("f")))
                    .collect::<Vec<_>>()
                    .join(" UNION ALL ");
                let branches: Vec<&str> = outs.iter().map(String::as_str).collect();
                self.frame("x", &union, Reads::union(&branches))
            }
            Op::Not(inner) => {
                let out = self.op(inner, input);
                self.frame(
                    "x",
                    &format!(
                        "SELECT {} FROM {input} f WHERE f.k NOT IN (SELECT k FROM {out})",
                        self.cols("f")
                    ),
                    Reads::anti(input, &[&out]),
                )
            }
            Op::Never => self.frame(
                "x",
                &format!("SELECT {} FROM {input} f WHERE 0", self.cols("f")),
                Reads::join(&[input]),
            ),
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
                    self.frame_with("x", &with, &select, Reads::join(&[input, &matched, &candidates]))
                } else {
                    self.frame(
                        "x",
                        &format!(
                            "SELECT {} FROM {input} f WHERE f.k IN (SELECT g.pk FROM {matched} s JOIN {candidates} g ON g.k = s.k)",
                            self.cols("f")
                        ),
                        Reads::join(&[input, &matched, &candidates]),
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
                let counts = self.grouped(
                    "n",
                    &format!(
                        "SELECT g.pk, COUNT(*) FROM {matched} s JOIN {candidates} g ON g.k = s.k GROUP BY g.pk"
                    ),
                    Reads::join(&[&matched, &candidates]),
                );
                let overrides: Vec<(String, String)> = bind
                    .iter()
                    .map(|(slot, offset)| {
                        (format!("s{slot}"), format!("(COALESCE(c.n, 0) + ({offset}))"))
                    })
                    .collect();
                let upper = max.map_or(String::new(), |max| format!(" AND COALESCE(c.n, 0) <= {max}"));
                let select = format!(
                    "SELECT {} FROM {input} f LEFT JOIN {counts} c ON c.pk = f.k WHERE COALESCE(c.n, 0) >= {min}{upper}",
                    self.select("f", &overrides)
                );
                // A row with no matching candidate counts 0, which `min` may accept.
                self.frame("x", &select, Reads::join(&[input]))
            }
            Op::NthChild {
                step,
                offset,
                reverse,
                of_rule,
            } => self.nth_child(*step, *offset, *reverse, of_rule.as_ref(), input),
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
                        Reads::join(&[&current]),
                    );
                    let captured = self.frame(
                        "g",
                        &format!(
                            "SELECT f.k, f.file, f.{column}{} FROM {current} f WHERE f.{column} IS NOT NULL",
                            self.physical_of("f")
                        ),
                        Reads::join(&[&current]),
                    );
                    let checked = self.op(constraint, &captured);
                    current = self.frame(
                        "x",
                        &format!(
                            "SELECT {} FROM {checked} c JOIN {current} f ON f.k = c.k UNION ALL SELECT {} FROM {passed} p",
                            self.select("f", &self.all_from("c")),
                            self.cols("p")
                        ),
                        Reads::union(&[&checked, &passed]),
                    );
                }
                current
            }
            Op::Fact {
                term,
                reads,
                at_node,
                binds,
            } => self.fact(*term, reads, *at_node, binds, input),
            Op::Seq(items) => self.seq(items, input),
            Op::Gate(slot) => self.gate(*slot, input),
            Op::Every { walk, sub } => self.every(*walk, sub, input),
            Op::Parse {
                term,
                inner,
                host,
                tree,
                body,
                exports,
            } => self.parse(*term, inner, *host, *tree, body, exports, input),
            Op::Host {
                host,
                tree,
                rule,
                exports,
            } => self.host(*host, *tree, rule, exports, input),
            Op::Exports { tree, exports } => self.exports(*tree, exports, input),
            Op::Unknowable { .. } | Op::AllOpen(_) | Op::AnyOpen(_) | Op::Search { .. } => {
                unreachable!("three-valued operators lower under `Unknowable`, at the root only")
            }
            Op::Recur { unit, slots } => self.recur(*unit, slots.as_deref(), input),
            // The root is bound only where some range is reported for it: elsewhere `Excluded`
            // drops nothing whatever the root, so leaving it unbound keeps the roots out of the
            // distinct keys of the sub-rules (`keyed`) and recursive units, which would otherwise
            // evaluate the same answer once per root.
            Op::EachRoot { root } => self.frame(
                "x",
                &format!(
                    "SELECT {} FROM {input} f WHERE (SELECT round FROM {EACH}r) = 0 \
                     OR EXISTS (SELECT 1 FROM {EACH}a a WHERE a.file = f.file AND a.root = f.node)",
                    self.select(
                        "f",
                        &[(
                            format!("s{root}"),
                            format!(
                                "CASE WHEN EXISTS (SELECT 1 FROM {EACH}x x WHERE x.file = f.file AND x.root = f.node) \
                                 THEN f.node END"
                            )
                        )]
                    )
                ),
                Reads::join(&[input]),
            ),
            Op::Excluded { name, root } => self.frame(
                "x",
                &format!(
                    "SELECT {} FROM {input} f LEFT JOIN node b ON b.file = f.file AND b.id = f.s{name} \
                     LEFT JOIN {EACH}x e ON e.file = f.file AND e.root = f.s{root} AND e.start = b.start AND e.\"end\" = b.\"end\" \
                     WHERE e.root IS NULL",
                    self.cols("f")
                ),
                Reads::anti(input, &[&format!("{EACH}x")]),
            ),
            Op::NthCommit { nth, commit } => {
                let positioned = self.op(nth, input);
                let passed = self.frame(
                    "y",
                    &format!(
                        "SELECT {} FROM {input} f WHERE f.k IN (SELECT k FROM {positioned})",
                        self.cols("f")
                    ),
                    Reads::join(&[input, &positioned]),
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
                    Reads::join(&[&passed]),
                )
            }
        }
    }

    /// The physical columns of `slots`, in order.
    fn key_columns(&self, slots: &[SlotId]) -> Vec<String> {
        slots.iter().flat_map(|&slot| self.columns(slot)).collect()
    }

    /// Creates `unit`'s demand and answer tables where they do not exist yet. A demanded key is
    /// `dirty` until its body is evaluated, and again when a key it read changed its answer. A
    /// three-valued unit's answer holds its verdict too, `unk`: the key matched or is unknown.
    fn unit_tables(&mut self, unit: usize) {
        let width = self.key_columns(&self.units[unit].key).len();
        let columns: String = (0..width).map(|index| format!(", c{index}")).collect();
        let demand = demand_table(unit);
        let answer = answer_table(unit);
        let unk = if self.three { ", unk" } else { "" };
        self.push(
            format!(
                "CREATE TEMP TABLE IF NOT EXISTS {demand}(id INTEGER PRIMARY KEY, file, node{columns}, dirty INTEGER DEFAULT 1)"
            ),
            Effect::default(),
        );
        // The whole key: a reference finds its row by `(file, node)` and every key column, and a
        // node can be demanded under many bindings.
        self.push(
            format!("CREATE INDEX IF NOT EXISTS {demand}_at ON {demand}(file, node{columns})"),
            Effect::default(),
        );
        self.push(
            format!(
                "CREATE TEMP TABLE IF NOT EXISTS {answer}(id INTEGER PRIMARY KEY{columns}{unk})"
            ),
            Effect::default(),
        );
    }

    /// A recursive unit's answers at the keys of `input`'s rows (`site`: the frame's slots for the
    /// unit's key; `None`, the key's own): the keys are demanded, the loop of the unit's component
    /// runs where the unit is not being evaluated already, and each row whose key matched takes
    /// the key's answer. Rows whose key did not match are dropped. Three-valued, a row whose key
    /// is unknown is unknown (its answer's bindings are the key's, so the row's own).
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
        let columns: String = (0..site_columns.len())
            .map(|index| format!(", c{index}"))
            .collect();
        let values: String = site_columns
            .iter()
            .map(|column| format!(", f.{column}"))
            .collect();
        self.insert(
            &demand,
            format!(
                "INSERT INTO {demand}(file, node{columns}) SELECT DISTINCT f.file, f.node{values} FROM {input} f \
                 WHERE NOT EXISTS (SELECT 1 FROM {demand} d WHERE d.file = f.file AND d.node = f.node{matched})"
            ),
            Reads::join(&[input]),
        );
        let scc = units[unit].scc;
        if !self.emitting.contains(&scc) {
            self.shared_loop(scc);
        } else if let (Some(&from), Some(origin)) = (self.evaluating.last(), self.origin)
            && units[from].scc == scc
        {
            // A read inside the component's own loop: recorded for the acyclicity check.
            let reads = format!("{NAMESPACE}e{scc}");
            self.insert(
                &reads,
                format!(
                    "INSERT INTO {reads} SELECT DISTINCT {from}, f.s{origin}, {unit}, d.id FROM {input} f \
                     JOIN {demand} d ON d.file = f.file AND d.node = f.node{matched}"
                ),
                Reads::join(&[input, &demand]),
            );
        }
        let overrides: Vec<(String, String)> = site_columns
            .iter()
            .enumerate()
            .map(|(index, column)| (column.clone(), format!("r.c{index}")))
            .collect();
        let select = if self.three {
            self.verdict("f", &overrides, "r.unk")
        } else {
            self.select("f", &overrides)
        };
        self.frame(
            "x",
            &format!(
                "SELECT {select} FROM {input} f JOIN {demand} d ON d.file = f.file AND d.node = f.node{matched} \
                 JOIN {answer} r ON r.id = d.id"
            ),
            Reads::join(&[input, &demand, &answer]),
        )
    }

    /// Runs component `scc`'s loop: emitted where it is first run, as a block (`BLOCK`), and run
    /// again by name (`RUN`) everywhere after. Its statements do not depend on the site: they read
    /// the units' demand and answer tables and frames of their own, and every unit they reach
    /// lies outside the loops being emitted (one that reached back would share the component).
    /// Both markers' inputs are the members' demand tables: a run that finds no new demand row in
    /// them has no dirty key, and is passed over (`skip`).
    fn shared_loop(&mut self, scc: usize) {
        let units = self.units;
        let demand: Vec<String> = (0..units.len())
            .filter(|&unit| units[unit].scc == scc)
            .map(demand_table)
            .collect();
        let marker = Effect {
            inputs: demand,
            ..Effect::default()
        };
        if self.blocks.insert(scc) {
            self.push(format!("{BLOCK}{scc}"), marker);
            self.recursion_loop(scc);
            self.push(END.to_owned(), Effect::default());
        } else {
            self.push(format!("{RUN}{scc}"), marker);
        }
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
        for sql in [
            format!("CREATE TEMP TABLE IF NOT EXISTS {reads}(su, s, du, d)"),
            format!("CREATE INDEX IF NOT EXISTS {reads}_to ON {reads}(du, d)"),
            format!("CREATE INDEX IF NOT EXISTS {reads}_from ON {reads}(su, s)"),
            format!("CREATE TEMP TABLE IF NOT EXISTS {changed}(u, id)"),
            format!("CREATE INDEX IF NOT EXISTS {changed}_at ON {changed}(u, id)"),
        ] {
            self.push(sql, Effect::default());
        }
        self.emitting.push(scc);
        self.looping += 1;
        self.push(LOOP.to_owned(), Effect::default());
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
            select.push_str(&self.zeros());
            let keys = self.frame(
                "u",
                &format!("SELECT {select} FROM {demand} d WHERE d.dirty = 1"),
                Reads::join(&[&demand]),
            );
            // One row per demanded key, and one per demand row (its origin).
            let mut sets = vec![key.iter().copied().collect::<BTreeSet<SlotId>>()];
            sets.extend(self.origin.map(|origin| BTreeSet::from([origin])));
            self.keyed_by(&keys, sets);
            self.push(
                format!("UPDATE {demand} SET dirty = 0 WHERE id IN (SELECT k FROM {keys})"),
                Reads::join(&[&keys]).writes(Writes::None),
            );
            self.push(
                format!("DELETE FROM {reads} WHERE su = {unit} AND s IN (SELECT k FROM {keys})"),
                Reads::join(&[&reads, &keys]).writes(Writes::Delete(reads.clone())),
            );
            self.evaluating.push(unit);
            let out = self.op(&units[unit].body, &keys);
            self.evaluating.pop();
            let fresh = format!("{NAMESPACE}n{unit}");
            let width = key_columns.len();
            let columns: String = (0..width).map(|index| format!(", c{index}")).collect();
            let (unk, verdict) = if self.three {
                (", unk", ", o.unk")
            } else {
                ("", "")
            };
            self.create(&fresh, &format!("id INTEGER PRIMARY KEY{columns}{unk}"));
            let values: String = key_columns
                .iter()
                .map(|column| format!(", o.{column}"))
                .collect();
            // `out` is a frame, one row per `k`: `fresh` holds as many.
            self.insert(
                &fresh,
                format!("INSERT INTO {fresh} SELECT o.k{values}{verdict} FROM {out} o"),
                Reads::copy(&out),
            );
            // The evaluated keys whose answer changed: added, dropped or rebound.
            let before = format!("SELECT * FROM {answer} WHERE id IN (SELECT k FROM {keys})");
            self.insert(
                &changed,
                format!(
                    "INSERT INTO {changed} SELECT {unit}, id FROM \
                     (SELECT * FROM {fresh} EXCEPT {before} UNION SELECT * FROM ({before} EXCEPT SELECT * FROM {fresh}))"
                ),
                Reads::union(&[&fresh, &answer]),
            );
            self.push(
                format!("DELETE FROM {answer} WHERE id IN (SELECT k FROM {keys})"),
                Reads::join(&[&answer, &keys]).writes(Writes::Delete(answer.clone())),
            );
            // The evaluated keys' answers were just deleted: no `id` of `fresh` is left in it.
            self.insert(
                &answer,
                format!("INSERT INTO {answer} SELECT * FROM {fresh}"),
                Reads::copy(&fresh),
            );
        }
        for &unit in &members {
            let demand = demand_table(unit);
            self.push(
                format!(
                    "UPDATE {demand} SET dirty = 1 WHERE id IN (SELECT e.s FROM {reads} e JOIN {changed} c \
                     ON c.u = e.du AND c.id = e.d WHERE e.su = {unit})"
                ),
                Reads::join(&[&reads, &changed]).writes(Writes::None),
            );
        }
        self.push(
            format!("DELETE FROM {changed}"),
            Reads::join(&[&changed]).writes(Writes::Clear(changed.clone())),
        );
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
        self.push(format!("{UNTIL}SELECT {dirty}"), Effect::default());
        self.push(
            format!("{ACYCLIC}SELECT su, s, du, d FROM {reads}"),
            Effect::default(),
        );
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
            return self.frame_with("x", &with, &select, Reads::join(&[input]));
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
        self.frame_with("x", &with, &select, Reads::join(&[input]))
    }

    /// A relation's candidates from each row of `input`: a frame table with a fresh `k` per
    /// candidate and the row's bindings, plus `pk` (the row's `k`) and `ord` (the walk order:
    /// ids, negated for descending walks).
    fn candidates(&mut self, walk: Walk, input: &str) -> String {
        if let Some(candidates) = self.leaf_candidates(walk, input) {
            return candidates;
        }
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
            // A rule stop's walks read the stop set `S` through `nearest` (no recursion). `has`
            // visits a descendant `c` of `f` where no node strictly between them is in `S`: where
            // `c`'s nearest proper ancestor in `S` is `f`'s nearest ancestor-or-self in `S`.
            Walk::Has(Stop::Rule(stage)) => simple(&format!(
                "CROSS JOIN nearest nf ON {} AND nf.id = f.node \
                 CROSS JOIN nearest c ON {} AND c.anc = nf.base AND c.id > f.node AND c.id <= nf.\"last\"",
                nearest::set_of("nf", stage, "f.file"),
                nearest::set_of("c", stage, "f.file")
            )),
            Walk::Inside(Stop::Neighbor) => simple(&format!(
                "{at} JOIN node c ON c.file = f.file AND c.id = fn.parent"
            )),
            Walk::Inside(Stop::End) => {
                simple("JOIN node c ON c.file = f.file AND c.id < f.node AND c.\"last\" >= f.node")
            }
            // `inside` ends at `f`'s nearest proper ancestor in `S`, inclusive.
            Walk::Inside(Stop::Rule(stage)) => simple(&format!(
                "CROSS JOIN nearest nf ON {} AND nf.id = f.node \
                 CROSS JOIN node c ON c.file = f.file AND c.id < f.node AND c.\"last\" >= f.node AND c.id >= nf.anc",
                nearest::set_of("nf", stage, "f.file")
            )),
            Walk::Precedes(Stop::Neighbor) => simple(&format!(
                "{at} JOIN node c ON c.file = f.file AND c.id = fn.next"
            )),
            Walk::Precedes(Stop::End) => simple(&format!(
                "{at} JOIN node c ON c.file = f.file AND c.parent = fn.wpar AND c.id >= fn.anext"
            )),
            // The sibling walks end at the first node in `S` of their chain, inclusive.
            Walk::Precedes(Stop::Rule(stage)) => simple(&format!(
                "{at} CROSS JOIN nearest na ON {} AND na.id = fn.anext \
                 CROSS JOIN node c ON c.file = f.file AND c.parent = fn.wpar AND c.id >= fn.anext \
                 AND c.id <= COALESCE(na.nxt, 4294967295)",
                nearest::set_of("na", stage, "f.file")
            )),
            Walk::Follows(Stop::Neighbor) => simple(&format!(
                "{at} JOIN node c ON c.file = f.file AND c.id = fn.prev"
            )),
            Walk::Follows(Stop::End) => simple(&format!(
                "{at} JOIN node c ON c.file = f.file AND c.parent = fn.wpar AND c.id <= fn.aprev"
            )),
            Walk::Follows(Stop::Rule(stage)) => simple(&format!(
                "{at} CROSS JOIN nearest na ON {} AND na.id = fn.aprev \
                 CROSS JOIN node c ON c.file = f.file AND c.parent = fn.wpar AND c.id <= fn.aprev \
                 AND c.id >= COALESCE(na.prv, 0)",
                nearest::set_of("na", stage, "f.file")
            )),
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
            // The ancestors `inside` walks to, through a field child `w` of each: `w` an
            // ancestor-or-self of `f` below its nearest proper ancestor in `S`.
            Walk::FieldInside(field, Stop::Rule(stage)) => simple(&format!(
                "CROSS JOIN nearest nf ON {} AND nf.id = f.node \
                 CROSS JOIN node w ON w.file = f.file AND w.id <= f.node AND w.\"last\" >= f.node AND w.id > nf.anc \
                 CROSS JOIN node c ON c.file = w.file AND c.id = w.parent WHERE w.field = {field} AND w.ffirst = 1",
                nearest::set_of("nf", stage, "f.file")
            )),
            Walk::Slot(_) | Walk::Related(_) => {
                unreachable!("a terminal's walk (`leaf_candidates`)")
            }
        };
        let order = if walk.descending() {
            "-c.node"
        } else {
            "c.node"
        };
        let name = self.name("g");
        let columns = format!(
            "k INTEGER PRIMARY KEY, file, node{}, pk, ord{}",
            self.physical_list(),
            self.extra_list()
        );
        self.create(&name, &columns);
        self.insert(
            &name,
            format!(
                "{with}INSERT INTO {name} SELECT NULL, c.file, c.node{}, c.pk, {order}{} \
                 FROM ({source}) c JOIN {input} f ON f.k = c.pk",
                self.physical_of("f"),
                self.zeros()
            ),
            Reads::join(&[input]),
        );
        // A node has one parent and its field's first child one: from a frame unique on a key,
        // the children (or field children) of its rows are unique on it too.
        if matches!(
            walk,
            Walk::Has(Stop::Neighbor) | Walk::FieldHas(_, Stop::Neighbor)
        ) {
            self.inherit(input, &name, &BTreeSet::new());
        }
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
                    Reads::join(&[input]),
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
                self.frame_with("x", &with, &select, Reads::join(&[input, &table]))
            }
            Some(OfRule::Frame(rule)) => {
                let siblings = self.name("g");
                let columns = format!(
                    "k INTEGER PRIMARY KEY, file, node{}, pk, ord{}",
                    self.physical_list(),
                    self.extra_list()
                );
                self.create(&siblings, &columns);
                self.insert(
                    &siblings,
                    format!(
                        "INSERT INTO {siblings} SELECT NULL, c.file, c.id{}, f.k, c.id{} FROM {input} f \
                         JOIN node fn ON fn.file = f.file AND fn.id = f.node \
                         JOIN node c ON c.file = f.file AND c.parent = fn.parent WHERE c.named = 1",
                        self.physical_of("f"),
                        self.zeros()
                    ),
                    Reads::join(&[input]),
                );
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
                self.frame_with("x", &with, &select, Reads::join(&[input, &matched]))
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
                _ => {
                    for (to, from) in callee.iter().zip(&caller) {
                        overrides.push((to.clone(), format!("f.{from}")));
                    }
                }
            }
        }
        // The input's rows, each under its own `k`, the parameters bound.
        let framed = self.frame(
            "i",
            &format!("SELECT {} FROM {input} f", self.select("f", &overrides)),
            Reads::copy(input),
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
                // An exported text: its text and range unify as `unify_capture`'s transformed
                // variables do, each with the caller's or the first parameter's.
                SlotKind::Text => {
                    for &param in &ret.params {
                        returned.push(format!("(b.s{param} IS NOT NULL AND b.s{caller} IS NULL)"));
                    }
                    for suffix in ["", "r"] {
                        let reference = format!(
                            "COALESCE(b.s{caller}{suffix}{})",
                            ret.params
                                .iter()
                                .map(|param| format!(", b.s{param}{suffix}"))
                                .collect::<String>()
                        );
                        for &param in &ret.params {
                            ok.push(format!(
                                "(b.s{param}{suffix} IS NULL OR b.s{param}{suffix} = {reference})"
                            ));
                        }
                        overrides.push((format!("s{caller}{suffix}"), reference));
                    }
                }
            }
        }
        let returned = format!("({})", returned.join(" OR "));
        let ok = format!("({})", ok.join(" AND "));
        // Three-valued: a body whose verdict is unknown returns nothing; the call is unknown.
        let (returned, verdict) = if self.three {
            (format!("(b.unk = 0 AND {returned})"), "b.unk")
        } else {
            (returned, "")
        };
        let overrides: Vec<(String, String)> = overrides
            .into_iter()
            .map(|(column, value)| {
                let kept = format!("CASE WHEN {returned} THEN {value} ELSE b.{column} END");
                (column, kept)
            })
            .collect();
        let select = if self.three {
            self.verdict("b", &overrides, verdict)
        } else {
            self.select("b", &overrides)
        };
        self.frame(
            "x",
            &format!("SELECT {select} FROM {bound} b{joins} WHERE NOT {returned} OR {ok}"),
            Reads::join(&[&bound]),
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
