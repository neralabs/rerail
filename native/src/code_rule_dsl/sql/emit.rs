//! Emission from the frame IR to SQLite statements. Every frame is a TEMP table with the columns
//! `k, file, node` and then the physical columns of the slots it holds, in slot order: the slots
//! a later operator reads or the statement emits (`Emitter::op`'s `live`), so a frame carries no
//! slot nothing reads after it. An operator is one `INSERT … SELECT` from its input frame's table
//! (with CTEs of its own), so frames combine with `UNION ALL`. Relations are joins with `node` on
//! the axis; a first-match binding is an argmin per `k` (`MIN` with bare columns); terminals are
//! read once per statement through `MATERIALIZED` CTEs or an uncorrelated `IN` subquery.
//!
//! Frames are tables rather than one statement's CTEs because SQLite copies a CTE's whole
//! definition into every reference (`resolveFromTermToCte`), CTEs it references included: a rule
//! whose frames are each read twice would expand exponentially with its depth.
//!
//! A frame that numbers its rows afresh inserts NULL as `k`, so SQLite assigns the next rowid and
//! appends: a `ROW_NUMBER() OVER ()` would buffer every row in an ephemeral table first and
//! insert them by seeking.

mod each;
mod following;
mod keyed;
mod leaves;
mod lineage;
mod three;

use super::lower::{INTERNAL, ORIGIN};
use super::nearest;
use super::plan::{
    CallReturn, CallSlot, Need, NodeSet, OfRule, Op, Shape, Slot, SlotId, SlotKind, Source,
    Statement, Stop, Unit, Walk,
};
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write;
use std::rc::Rc;

/// Where the driver substitutes the statement's whole files (`n.file IN (…)` of the root frame).
pub(super) const FILES: &str = "/*FILES*/";
/// Where the driver substitutes the statement's selected roots, `(file, node)` rows, in files
/// whose roots an anchor index narrowed (the root frame's second branch).
pub(super) const ROOTS: &str = "/*ROOTS*/";
/// Where the driver substitutes the prefix of the statement's own tables that live across its
/// operators: a recursion unit's tables, the enumeration's frames.
pub(super) const NAMESPACE: &str = "/*NS*/";
/// Before an operator's temporary table (`Emitter::name`: a prefix of letters, then digits):
/// the driver allocates each that is a frame a register, a frame name shared across clauses and
/// reused once the temporary is dead (`exec::registers`); one that stays a B-tree takes the
/// statement's namespace.
pub(super) const TEMP: &str = "/*T*/";
/// Where the driver substitutes the prefix of a `violation.each` enumeration's tables: `x`, the
/// ranges reported per root `(file, root, start, end)`; `a`, the roots still enumerated; `r`, the
/// round.
pub(super) const EACH: &str = "/*EACH*/";
/// A statement that opens a loop: the statements up to the matching `UNTIL` run while its query
/// (the rest of the `UNTIL` statement) counts a dirty key, checked before every pass (`exec`).
pub(super) const LOOP: &str = "/*LOOP*/";
pub(super) const UNTIL: &str = "/*UNTIL*/";
/// A query after a statement's operator: the recursive reads every loop's last pass and the
/// statement's own rows recorded from their needed rows, `(unit, key, unit, key, inner)`, which
/// the driver accepts only where those it reaches from the statement form no cycle (`exec`).
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

/// A statement's recursive reads from its own rows, outside every unit's body: `(-1, 0, unit,
/// key)`, the keys the recursion check starts from (`Emitter::acyclic`).
fn start_table() -> String {
    format!("{NAMESPACE}ex")
}

/// The table a stage's rows are stored in for the batch.
pub(super) fn stage_table(stage: usize) -> String {
    format!("stage_{stage}")
}

/// Whether node `node` of file `file` (SQL expressions) is in `set`, as an SQL condition: a row of
/// its stage, or (a complement) none.
pub(super) fn in_set(set: NodeSet, file: &str, node: &str) -> String {
    format!(
        "{}EXISTS (SELECT 1 FROM {} st WHERE st.file = {file} AND st.node = {node})",
        if set.complement { "NOT " } else { "" },
        stage_table(set.stage)
    )
}

/// `shape` at node `node` of file `file` (SQL expressions), as an SQL condition read from the
/// `node` table: a kind test by id, a descendant's or a child's from the kind postings within
/// the node's subtree or among its children. `alias` numbers the nested tables.
pub(super) fn shape_test(shape: &Shape, file: &str, node: &str, alias: &mut usize) -> String {
    fn fresh(alias: &mut usize) -> String {
        *alias += 1;
        format!("sh{alias}")
    }
    let list_of = |kinds: &[u16]| list(kinds);
    match shape {
        // No node has none of no kinds: false, with no table read.
        Shape::Kinds(own) | Shape::Contains(own) | Shape::Child(own) if own.is_empty() => {
            "0".to_owned()
        }
        Shape::Kinds(own) => {
            let n = fresh(alias);
            format!(
                "EXISTS (SELECT 1 FROM node {n} WHERE {n}.file = {file} AND {n}.id = {node} AND {n}.kind IN ({}))",
                list_of(own)
            )
        }
        Shape::Contains(below) => {
            let (n, d) = (fresh(alias), fresh(alias));
            format!(
                "EXISTS (SELECT 1 FROM node {n} JOIN node {d} ON {d}.file = {n}.file AND {d}.id > {n}.id \
                 AND {d}.id <= {n}.\"last\" AND {d}.kind IN ({}) WHERE {n}.file = {file} AND {n}.id = {node})",
                list_of(below)
            )
        }
        Shape::Child(below) => {
            let d = fresh(alias);
            format!(
                "EXISTS (SELECT 1 FROM node {d} WHERE {d}.file = {file} AND {d}.parent = {node} AND {d}.kind IN ({}))",
                list_of(below)
            )
        }
        Shape::All(shapes) | Shape::Any(shapes) => {
            let joiner = if matches!(shape, Shape::All(_)) {
                " AND "
            } else {
                " OR "
            };
            let parts: Vec<String> = shapes
                .iter()
                .map(|part| shape_test(part, file, node, alias))
                .collect();
            format!("({})", parts.join(joiner))
        }
    }
}

/// The tables a statement reading `set` for its members only reads whose emptiness empties it
/// (`Reads::join`): its stage's, unless a complement, which an empty stage leaves whole.
pub(super) fn set_inputs(set: NodeSet) -> Option<String> {
    (!set.complement).then(|| stage_table(set.stage))
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

/// The slots of the clause's own scope: those a clause's last statement selects.
fn clause_slots(statement: &Statement) -> BTreeSet<SlotId> {
    (0..statement.slots.len())
        .filter(|&id| statement.slots[id].scope == 0)
        .collect()
}

/// A clause's statements: the last selects its matching roots and the clause-scope bindings of
/// each, `(file, node, columns of every scope-0 slot)`, ordered by file and root.
pub(super) fn clause(statement: &Statement) -> Emitted {
    let mut emitter = Emitter::new(statement);
    let out = emitter.statement(statement, clause_slots(statement));
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
    let mut emitted = clause_slots(statement);
    emitted.extend(name);
    let out = emitter.statement(statement, emitted);
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
    // Mutant 10: a match that repeats a range is reported too.
    let reported = if super::mutant(10) {
        "e.es IS NOT NULL".to_owned()
    } else {
        format!("(e.file, e.node) IN (SELECT file, node FROM {ef})")
    };
    emitter.push(
        format!(
            "SELECT e.file, e.node{} FROM {eo} e WHERE (SELECT round FROM {EACH}r) = 0 \
             OR {reported} \
             ORDER BY e.file, e.node",
            of("e")
        ),
        Reads::join(&[&eo]).writes(Writes::None),
    );
    emitter.out
}

/// A stage's statements: the last selects the distinct `(file, node)` its rule matches; in a
/// three-valued statement `(file, node, unk)`, where it is true or unknown. `three`: the stage's
/// table has `unk` (its readers are three-valued), which a two-valued statement (a rule that
/// cannot be unknown) fills with 0.
pub(super) fn stage(statement: &Statement, three: bool) -> Emitted {
    let mut emitter = Emitter::new(statement);
    let out = emitter.statement(statement, BTreeSet::new());
    let select = if emitter.three {
        format!("SELECT file, node, MIN(unk) FROM {out} GROUP BY file, node")
    } else if three {
        format!("SELECT DISTINCT file, node, 0 FROM {out}")
    } else {
        format!("SELECT DISTINCT file, node FROM {out}")
    };
    emitter.push(select, Reads::join(&[&out]).writes(Writes::None));
    emitter.out
}

/// The slots a frame must hold for what reads it after an operator (`Emitter::op`): the slots
/// later operators read or bind, and those the statement emits.
pub(super) type Live = Rc<BTreeSet<SlotId>>;

/// A frame's physical columns: those of the slots it holds, in slot order (`Emitter::layout`).
#[derive(Clone, Default)]
struct Layout {
    columns: Rc<[String]>,
    /// The columns comma-led: `, s0, s1a…`.
    list: Rc<str>,
}

struct Emitter<'s> {
    slots: &'s [Slot],
    units: &'s [Unit],
    /// The terminals' related-node walks (`Walk::Related`).
    sources: &'s [Source],
    /// The layout of the frames being written: the output frame of the operator being emitted
    /// (`Emitter::op`), or of an inner frame (`Emitter::within`). Every row a statement selects
    /// lists these columns (`cols`, `select`…), read from frames that hold them all.
    layout: Layout,
    /// Each slot set's layout, made once.
    layouts: HashMap<BTreeSet<SlotId>, Layout>,
    /// What every frame of the statement lists after its slots, fixed once it is configured
    /// (`configure`): the three-valued columns (`, unk, wh0…`) and their values in a true row
    /// (`, 0, 0…`).
    extra_list: String,
    zeros: String,
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
    /// The k-space of each frame in a unit's body (`lineage`), by name.
    space_of: HashMap<String, usize>,
    spaces: Vec<lineage::Space>,
    /// The unit bodies being emitted, innermost last, and the next one's id.
    members: Vec<lineage::Member>,
    next_member: usize,
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
        let mut emitter = Self {
            slots,
            units: &statement.units,
            sources: &statement.sources,
            layout: Layout::default(),
            layouts: HashMap::new(),
            extra_list: String::new(),
            zeros: String::new(),
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
            space_of: HashMap::new(),
            spaces: Vec::new(),
            members: Vec::new(),
            next_member: 0,
        };
        emitter.fix_columns();
        emitter
    }

    /// Fixes the columns every frame lists after its slots (`extra_list`, `zeros`) for the
    /// statement's valuedness.
    fn fix_columns(&mut self) {
        let mut extra = String::new();
        let mut zeros = String::new();
        if self.three {
            extra.push_str(", unk");
            for word in 0..self.masks {
                write!(extra, ", wh{word}").expect("write to a string");
            }
            zeros = ", 0".repeat(1 + self.masks);
        }
        self.extra_list = extra;
        self.zeros = zeros;
    }

    /// The layout of a frame holding `slots`.
    fn layout_of(&mut self, slots: &BTreeSet<SlotId>) -> Layout {
        if let Some(layout) = self.layouts.get(slots) {
            return layout.clone();
        }
        let columns: Vec<String> = slots
            .iter()
            .flat_map(|&slot| self.slots[slot].columns(slot))
            .collect();
        let mut list = String::with_capacity(columns.iter().map(|c| c.len() + 2).sum());
        for column in &columns {
            list.push_str(", ");
            list.push_str(column);
        }
        let layout = Layout {
            columns: columns.into(),
            list: list.into(),
        };
        self.layouts.insert(slots.clone(), layout.clone());
        layout
    }

    /// Runs `emit` with the frames it writes holding `slots` (`layout`).
    fn within<R>(&mut self, slots: &BTreeSet<SlotId>, emit: impl FnOnce(&mut Self) -> R) -> R {
        let layout = self.layout_of(slots);
        let outer = std::mem::replace(&mut self.layout, layout);
        let result = emit(self);
        self.layout = outer;
        result
    }

    /// `live` and the slots `op` names (`keyed::touched`, which holds every slot it reads or
    /// binds): what `op`'s input must hold for its output to hold `live`. No slot an operator
    /// binds is dropped from its input: it may leave a row's binding as it was.
    fn live_before(&mut self, op: &Op, live: &Live) -> Live {
        let touched = self.touched(op);
        if touched.is_subset(live) {
            return Rc::clone(live);
        }
        Rc::new(live.union(&touched).copied().collect())
    }

    /// The slots of `live` that `op` names: those of its output it may have rebound, which a
    /// walk's sub-rule returns to the row it walked from.
    fn rebound(&mut self, op: &Op, live: &BTreeSet<SlotId>) -> Live {
        let touched = self.touched(op);
        Rc::new(live.intersection(&touched).copied().collect())
    }

    /// What each of `ops`, run one after another, must leave in its output for the rest and
    /// `live` after them: the `live` of each, in order.
    fn pipeline<'o>(
        &mut self,
        ops: impl DoubleEndedIterator<Item = &'o Op>,
        live: &Live,
    ) -> Vec<Live> {
        let mut lives = Vec::new();
        let mut after = Rc::clone(live);
        for op in ops.rev() {
            lives.push(Rc::clone(&after));
            after = self.live_before(op, &after);
        }
        lives.reverse();
        lives
    }

    /// What the rows left to each of an `any`'s `branches`, in the order they run, hold: what it
    /// and every later branch reads, and `live`, which each branch's output holds.
    fn remainders<'o>(
        &mut self,
        branches: impl DoubleEndedIterator<Item = &'o Op>,
        live: &Live,
    ) -> Vec<Live> {
        let mut left = Vec::new();
        let mut after = Rc::clone(live);
        for op in branches.rev() {
            after = self.live_before(op, &after);
            left.push(Rc::clone(&after));
        }
        left.reverse();
        left
    }

    /// What the rows of a query's `constraints` hold, with `live` after the last: before the
    /// first, and after each, in order. A constraint reads its capture's slot.
    fn constrained_lives(
        &mut self,
        constraints: &[(SlotId, Op)],
        live: &Live,
    ) -> (Live, Vec<Live>) {
        let mut lives = Vec::with_capacity(constraints.len());
        let mut before = Rc::clone(live);
        for (slot, constraint) in constraints.iter().rev() {
            lives.push(Rc::clone(&before));
            let mut held = (*self.live_before(constraint, &before)).clone();
            held.insert(*slot);
            before = Rc::new(held);
        }
        lives.reverse();
        (before, lives)
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
        format!("{TEMP}{prefix}{}", self.next)
    }

    /// The physical columns of the layout, comma-led: `, s0, s1a…`.
    fn physical_list(&self) -> &str {
        &self.layout.list
    }

    /// Appends the physical columns of `alias` to `out`, comma-led: `, alias.s0…`.
    fn push_physical_of(&self, out: &mut String, alias: &str) {
        for column in self.layout.columns.iter() {
            out.push_str(", ");
            out.push_str(alias);
            out.push('.');
            out.push_str(column);
        }
    }

    /// The physical columns of `alias`, comma-led: `, alias.s0…`.
    fn physical_of(&self, alias: &str) -> String {
        let mut out = String::with_capacity(self.layout.list.len() * 2);
        self.push_physical_of(&mut out, alias);
        out
    }

    /// The declared columns of a frame of the layout.
    fn frame_columns(&self) -> String {
        format!(
            "k INTEGER PRIMARY KEY, file, node{}{}",
            self.layout.list, self.extra_list
        )
    }

    /// Emits `sql`, an `INSERT` into `table`, empty as `reads` says.
    fn insert(&mut self, table: &str, sql: String, reads: Reads) {
        self.push(sql, reads.writes(Writes::Insert(table.to_owned())));
    }

    /// A frame table filled by `select` (canonical columns, in order), with CTEs `with`; empty
    /// as `reads` says.
    fn frame_with(&mut self, prefix: &str, with: &str, select: &str, reads: Reads) -> String {
        let name = self.name(prefix);
        let columns = self.frame_columns();
        self.create(&name, &columns);
        let mut sql = String::with_capacity(with.len() + name.len() + select.len() + 13);
        sql.push_str(with);
        sql.push_str("INSERT INTO ");
        sql.push_str(&name);
        sql.push(' ');
        sql.push_str(select);
        self.insert(&name, sql, reads);
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
        let mut out = String::with_capacity(
            3 * alias.len() + 16 + self.layout.list.len() * 2 + self.zeros.len(),
        );
        out.push_str(alias);
        out.push_str(".k, ");
        out.push_str(alias);
        out.push_str(".file, ");
        out.push_str(alias);
        out.push_str(".node");
        self.push_physical_of(&mut out, alias);
        out.push_str(&self.zeros);
        out
    }

    /// The canonical columns of `alias` (`k`, `file`, `node`), then each physical column of
    /// `alias` or its expression in `overrides`.
    fn overridden(&self, alias: &str, overrides: &[(String, String)]) -> String {
        let mut select = String::with_capacity(
            3 * alias.len() + 16 + self.layout.list.len() * 2 + self.zeros.len(),
        );
        select.push_str(alias);
        select.push_str(".k, ");
        select.push_str(alias);
        select.push_str(".file, ");
        select.push_str(alias);
        select.push_str(".node");
        for column in self.layout.columns.iter() {
            select.push_str(", ");
            match overrides.iter().find(|(name, _)| name == column) {
                Some((_, expr)) => select.push_str(expr),
                None => {
                    select.push_str(alias);
                    select.push('.');
                    select.push_str(column);
                }
            }
        }
        select
    }

    /// The frame columns of `alias`, with some physical columns replaced by expressions.
    fn select(&self, alias: &str, overrides: &[(String, String)]) -> String {
        let mut select = self.overridden(alias, overrides);
        select.push_str(&self.zeros);
        select
    }

    /// The physical columns of `slots` replaced by `alias`'s.
    fn all_from(&mut self, slots: &BTreeSet<SlotId>, alias: &str) -> Vec<(String, String)> {
        self.layout_of(slots)
            .columns
            .iter()
            .map(|column| (column.clone(), format!("{alias}.{column}")))
            .collect()
    }

    fn columns(&self, slot: usize) -> Vec<String> {
        self.slots[slot].columns(slot)
    }

    /// A three-valued frame's columns after the slots, comma-led: `, unk, wh0…`; empty in a
    /// two-valued statement.
    fn extra_list(&self) -> &str {
        &self.extra_list
    }

    /// The values of `extra_list` for a row an operator emits as true: `, 0, 0…`.
    fn zeros(&self) -> &str {
        &self.zeros
    }

    /// `select` with the verdict `unk` (an expression) and no withheld capture: the row of a
    /// three-valued operator.
    fn verdict(&self, alias: &str, overrides: &[(String, String)], unk: &str) -> String {
        let mut select = self.overridden(alias, overrides);
        select.push_str(", ");
        select.push_str(unk);
        for _ in 0..self.masks {
            select.push_str(", 0");
        }
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
                self.fix_columns();
                op
            }
            op => op,
        }
    }

    /// The statement's operator on its roots, its output holding `emitted`, the slots its last
    /// statement selects.
    fn statement(&mut self, statement: &Statement, emitted: BTreeSet<SlotId>) -> String {
        let op = self.configure(statement);
        let live = Rc::new(emitted);
        let held = self.live_before(op, &live);
        let layout = self.layout_of(&held);
        self.layout = layout;
        let nulls: String = self.layout.columns.iter().map(|_| ", NULL").collect();
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
        if self.units.is_empty() {
            return self.op(op, &root, &live);
        }
        // The keys the clause's own needed rows demand: where the recursion check starts.
        self.create(&start_table(), "su, s, du, d");
        self.member_start(&root);
        let out = self.op(op, &root, &live);
        self.member_end();
        self.acyclic();
        out
    }

    /// Accepts the statement's recursion (`exec::acyclic`): every read recorded from a needed row,
    /// `(unit, key, unit, key, inner)`, `inner` where both units are of one component, the
    /// clause's own as from unit `-1`'s key `0` (`start_table`).
    fn acyclic(&mut self) {
        let mut query = format!("{ACYCLIC}SELECT su, s, du, d, 0 FROM {}", start_table());
        for &scc in &self.blocks {
            let members: Vec<String> = (0..self.units.len())
                .filter(|&unit| self.units[unit].scc == scc)
                .map(|unit| unit.to_string())
                .collect();
            write!(
                query,
                " UNION ALL SELECT su, s, du, d, du IN ({}) FROM {NAMESPACE}e{scc}",
                members.join(", ")
            )
            .expect("write to a string");
        }
        self.push(query, Effect::default());
    }

    /// `op` on `input`, its output holding `live` (every frame of it holds those slots, `input`
    /// those and every slot `op` names): per distinct key where it walks over a frame that may
    /// repeat a key (`keyed`), else directly.
    fn op(&mut self, op: &Op, input: &str, live: &Live) -> String {
        let out = self.within(live, |this| match this.keyed(op, input, live) {
            Some(out) => out,
            None => this.apply(op, input, live),
        });
        let changed = self.touched(op);
        self.inherit(input, &out, &changed);
        self.same_space(input, &out);
        out
    }

    /// `op` on a tree walk's `candidates`, which hold only `need`'s kinds (`tree_candidates`): a
    /// leading kind test those kinds pass is not run again (a walk to `kind: call` would
    /// otherwise copy every candidate row once more to re-test its kind).
    fn on_candidates(
        &mut self,
        op: &Op,
        walk: Walk,
        candidates: &str,
        need: &Need,
        live: &Live,
    ) -> String {
        let tree = !matches!(walk, Walk::Slot(_) | Walk::Related(_));
        let implied = |kinds: &[u16]| {
            tree && need
                .kinds
                .as_deref()
                .is_some_and(|need| need.iter().all(|kind| kinds.contains(kind)))
        };
        match op {
            Op::Kinds(kinds) if implied(kinds) => candidates.to_owned(),
            Op::All(items) if matches!(items.first(), Some(Op::Kinds(kinds)) if implied(kinds)) => {
                let lives = self.pipeline(items[1..].iter(), live);
                let mut current = candidates.to_owned();
                for (item, live) in items[1..].iter().zip(&lives) {
                    current = self.op(item, &current, live);
                }
                let changed = self.touched(op);
                self.inherit(candidates, &current, &changed);
                self.same_space(candidates, &current);
                current
            }
            _ => self.op(op, candidates, live),
        }
    }

    /// `op` on `input` (`Emitter::op`), in the output frame's layout.
    fn apply(&mut self, op: &Op, input: &str, live: &Live) -> String {
        if self.three
            && let Some(out) = self.op3(op, input, live)
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
                let lives = self.pipeline(items.iter(), live);
                let mut current = input.to_owned();
                for (item, live) in items.iter().zip(&lives) {
                    current = self.op(item, &current, live);
                }
                current
            }
            // One branch is that branch: no union copies its rows.
            Op::Any(branches) if branches.len() == 1 => self.op(&branches[0], input, live),
            Op::Any(branches) => {
                // Mutant 2: the branches last first.
                let flip = super::mutant(2);
                let order: Vec<&Op> = (0..branches.len())
                    .map(|index| &branches[if flip { branches.len() - 1 - index } else { index }])
                    .collect();
                let left = self.remainders(order.iter().copied(), live);
                let mut rest = input.to_owned();
                let mut outs = Vec::new();
                for (index, branch) in order.iter().enumerate() {
                    let out = self.op(branch, &rest, live);
                    if index + 1 < order.len() {
                        // `rest` without the rows the branch matched: all of it where none did.
                        let next = self.within(&left[index + 1], |this| {
                            this.frame(
                                "y",
                                &format!(
                                    "SELECT {} FROM {rest} f WHERE f.k NOT IN (SELECT k FROM {out})",
                                    this.cols("f")
                                ),
                                Reads::anti(&rest, &[&out]),
                            )
                        });
                        self.inherit(&rest, &next, &BTreeSet::new());
                        self.same_space(&rest, &next);
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
                if let Op::Not(rule) = &**inner {
                    return self.holds_at(rule, input);
                }
                let out = self.op(inner, input, &Live::default());
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
            Op::Relation {
                walk,
                sub,
                binds,
                need,
            } => {
                // A binding first match returns the slots of `live` the walk may rebind; a
                // semi-join, none.
                let back = if *binds {
                    self.rebound(op, live)
                } else {
                    Live::default()
                };
                let held = self.live_before(op, &back);
                let candidates = self.candidates(*walk, need, input, &held);
                self.first_match(&candidates, *walk, false);
                let matched = self.on_candidates(sub, *walk, &candidates, need, &back);
                self.first_matched(&candidates, &matched);
                if *binds {
                    // `ord` is the walk order (negated ids for descending walks): its minimum
                    // is the first match.
                    // Mutant 1: any match, not the first.
                    let first = if super::mutant(1) { "g.ord" } else { "MIN(g.ord)" };
                    let returned = self.within(&back, |this| this.physical_of("s"));
                    let with = format!(
                        "WITH w AS (SELECT g.pk AS pk, {first} AS o{returned} FROM {matched} s JOIN {candidates} g ON g.k = s.k GROUP BY g.pk) "
                    );
                    let overrides = self.all_from(&back, "w");
                    let select = format!(
                        "SELECT {} FROM {input} f JOIN w ON w.pk = f.k",
                        self.select("f", &overrides)
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
                need,
            } => {
                let held = self.live_before(op, &Live::default());
                let candidates = self.candidates(*walk, need, input, &held);
                let matched = self.op(sub, &candidates, &Live::default());
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
            } => self.nth_child(op, *step, *offset, *reverse, of_rule.as_ref(), input, live),
            Op::Call {
                entry,
                body,
                returns,
            } => self.call(entry, body, returns, input, live),
            Op::Constrained { rule, constraints } => {
                let (before, lives) = self.constrained_lives(constraints, live);
                let mut current = self.op(rule, input, &before);
                for ((slot, constraint), after) in constraints.iter().zip(&lives) {
                    let column = format!("s{slot}");
                    let passed = self.within(after, |this| {
                        this.frame(
                            "y",
                            &format!(
                                "SELECT {} FROM {current} f WHERE f.{column} IS NULL",
                                this.cols("f")
                            ),
                            Reads::join(&[&current]),
                        )
                    });
                    let held = self.live_before(constraint, after);
                    let captured = self.within(&held, |this| {
                        this.frame(
                            "g",
                            &format!(
                                "SELECT f.k, f.file, f.{column}{} FROM {current} f WHERE f.{column} IS NOT NULL",
                                this.physical_of("f")
                            ),
                            Reads::join(&[&current]),
                        )
                    });
                    self.same_space(&current, &passed);
                    self.same_space(&current, &captured);
                    let checked = self.op(constraint, &captured, after);
                    let overrides = self.all_from(after, "c");
                    let next = self.within(after, |this| {
                        this.frame(
                            "x",
                            &format!(
                                "SELECT {} FROM {checked} c JOIN {current} f ON f.k = c.k UNION ALL SELECT {} FROM {passed} p",
                                this.select("f", &overrides),
                                this.cols("p")
                            ),
                            Reads::union(&[&checked, &passed]),
                        )
                    });
                    self.same_space(&current, &next);
                    current = next;
                }
                current
            }
            Op::Fact {
                term,
                reads,
                at_node,
                binds,
            } => self.fact(*term, reads, *at_node, binds, input),
            Op::Seq(items) => self.seq(items, input, live),
            Op::Gate(slot) => self.gate(*slot, input),
            Op::Every { walk, sub } => self.every(op, *walk, sub, input),
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
            Op::Recur {
                unit,
                slots,
                context,
            } => self.recur(*unit, slots.as_deref(), *context, input),
            Op::Follow {
                rule,
                base,
                out,
                sub,
            } => self.follow(op, *rule, *base, *out, sub, input, live),
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
                let positioned = self.op(nth, input, &Live::default());
                let held = self.live_before(commit, live);
                let passed = self.within(&held, |this| {
                    this.frame(
                        "y",
                        &format!(
                            "SELECT {} FROM {input} f WHERE f.k IN (SELECT k FROM {positioned})",
                            this.cols("f")
                        ),
                        Reads::join(&[input, &positioned]),
                    )
                });
                self.same_space(input, &passed);
                let committed = self.op(commit, &passed, live);
                let overrides: Vec<(String, String)> = self
                    .layout
                    .columns
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
    /// three-valued unit's answer holds its verdict too, `unk`: the key matched or is unknown. A
    /// unit keyed by its following context (`Unit::following`) demands it too, in `cf`.
    fn unit_tables(&mut self, unit: usize) {
        let width = self.key_columns(&self.units[unit].key).len();
        let columns: String = (0..width).map(|index| format!(", c{index}")).collect();
        let context = if self.units[unit].following.is_some() {
            ", cf"
        } else {
            ""
        };
        let demand = demand_table(unit);
        let answer = answer_table(unit);
        let unk = if self.three { ", unk" } else { "" };
        self.push(
            format!(
                "CREATE TEMP TABLE IF NOT EXISTS {demand}(id INTEGER PRIMARY KEY, file, node{columns}{context}, dirty INTEGER DEFAULT 1)"
            ),
            Effect::default(),
        );
        // The whole key: a reference finds its row by `(file, node)` and every key column, and a
        // node can be demanded under many bindings.
        self.push(
            format!(
                "CREATE INDEX IF NOT EXISTS {demand}_at ON {demand}(file, node{columns}{context})"
            ),
            Effect::default(),
        );
        // The dirty keys, which every pass reads and the loop's guard counts before it runs.
        self.push(
            format!("CREATE INDEX IF NOT EXISTS {demand}_dirty ON {demand}(id) WHERE dirty = 1"),
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
    /// unit's key; `None`, the key's own; `context`: the slot of the following context passed,
    /// none where NULL): the keys are demanded, the loop of the unit's component runs where the
    /// unit is not being evaluated already, and each row whose key matched takes the key's
    /// answer. Rows whose key did not match are dropped. Three-valued, a row whose key is unknown
    /// is unknown (its answer's bindings are the key's, so the row's own).
    fn recur(
        &mut self,
        unit: usize,
        site: Option<&[SlotId]>,
        context: Option<SlotId>,
        input: &str,
    ) -> String {
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
        let mut matched: String = site_columns
            .iter()
            .enumerate()
            .map(|(index, column)| format!(" AND d.c{index} IS f.{column}"))
            .collect();
        let mut columns: String = (0..site_columns.len())
            .map(|index| format!(", c{index}"))
            .collect();
        // The input's distinct keys, as `c0…` (and `cf`), each probed once against the demand.
        let mut keys: String = site_columns
            .iter()
            .enumerate()
            .map(|(index, column)| format!(", f.{column} AS c{index}"))
            .collect();
        let mut probe: String = (0..site_columns.len())
            .map(|index| format!(" AND d.c{index} IS f.c{index}"))
            .collect();
        if units[unit].following.is_some() {
            let passed = context.map_or_else(|| "NULL".to_owned(), |slot| format!("f.s{slot}"));
            write!(matched, " AND d.cf IS {passed}").expect("write to a string");
            columns.push_str(", cf");
            write!(keys, ", {passed} AS cf").expect("write to a string");
            probe.push_str(" AND d.cf IS f.cf");
        }
        self.insert(
            &demand,
            format!(
                "INSERT INTO {demand}(file, node{columns}) SELECT f.file, f.node{columns} FROM \
                 (SELECT DISTINCT f.file AS file, f.node AS node{keys} FROM {input} f) f \
                 WHERE NOT EXISTS (SELECT 1 FROM {demand} d WHERE d.file = f.file AND d.node = f.node{probe})"
            ),
            Reads::join(&[input]),
        );
        let scc = units[unit].scc;
        if !self.emitting.contains(&scc) {
            self.shared_loop(scc);
        }
        // The reference, for the recursion check: from the key of the unit whose body it lies in,
        // into that unit's component's reads, or from the statement's own rows; recorded from the
        // rows the matcher evaluates (`lineage`). Without an origin slot to name the key, it
        // counts as the statement's own, which only starts the check at more keys.
        let (reads, source) = match (self.evaluating.last(), self.origin) {
            (Some(&from), Some(origin)) => (
                format!("{NAMESPACE}e{}", units[from].scc),
                format!("{from}, f.s{origin}"),
            ),
            _ => (start_table(), "-1, 0".to_owned()),
        };
        self.record_read(
            &reads,
            input,
            &demand,
            format!("INSERT INTO {reads} SELECT DISTINCT {source}, {unit}, d.id FROM {input} f"),
            format!(" JOIN {demand} d ON d.file = f.file AND d.node = f.node{matched}"),
        );
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
            let following = units[unit].following.map(|slot| format!("s{slot}"));
            let demand = demand_table(unit);
            let answer = answer_table(unit);
            // The body's output holds the key, its answer; its keys what the body reads too.
            let answered: Live = Rc::new(key.iter().copied().collect());
            let held = self.live_before(&units[unit].body, &answered);
            let keys = self.within(&held, |this| {
                let mut select = String::from("d.id, d.file, d.node");
                for column in this.layout.columns.iter() {
                    let value = match key_columns.iter().position(|key| key == column) {
                        Some(index) => format!("d.c{index}"),
                        None if Some(column) == origin.as_ref() => "d.id".to_owned(),
                        None if Some(column) == following.as_ref() => "d.cf".to_owned(),
                        None => "NULL".to_owned(),
                    };
                    write!(select, ", {value}").expect("write to a string");
                }
                select.push_str(this.zeros());
                this.frame(
                    "u",
                    &format!("SELECT {select} FROM {demand} d WHERE d.dirty = 1"),
                    Reads::join(&[&demand]),
                )
            });
            // One row per demanded key (its following context with it), and one per demand row
            // (its origin).
            let mut demanded: BTreeSet<SlotId> = key.iter().copied().collect();
            demanded.extend(units[unit].following);
            let mut sets = vec![demanded];
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
            self.member_start(&keys);
            let out = self.op(&units[unit].body, &keys, &answered);
            self.member_end();
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
            // The frame's distinct nodes as the input set: only they are matched.
            let with = format!(
                "WITH p AS MATERIALIZED (SELECT file, node{values} FROM pattern WHERE term = {term} \
                 AND nk IN (SELECT file * 4294967296 + node FROM {input})) "
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

    /// `not { not { rule } }`: the rows `rule` holds at, with their own bindings (both negations
    /// drop the rule's), as one semi-join. Three-valued, unknown where `rule` is unknown, as two
    /// negations of it are.
    pub(super) fn holds_at(&mut self, rule: &Op, input: &str) -> String {
        let out = self.op(rule, input, &Live::default());
        let select = if self.three {
            format!(
                "SELECT {} FROM {input} f JOIN {out} o ON o.k = f.k",
                self.verdict("f", &[], "o.unk")
            )
        } else {
            format!(
                "SELECT {} FROM {input} f WHERE f.k IN (SELECT k FROM {out})",
                self.cols("f")
            )
        };
        self.frame("x", &select, Reads::join(&[input, &out]))
    }

    /// A relation's candidates from each row of `input`: a frame table with a fresh `k` per
    /// candidate and the row's bindings of `held`, plus `pk` (the row's `k`) and `ord` (the walk
    /// order: ids, negated for descending walks). Only those `need` admits (`plan::Need`): the
    /// walk reads its kinds from the kind postings, and tests its shape on each candidate before
    /// the candidate's row is built. A k-space whose rows are all needed where their parent is
    /// (`lineage`); a first-match search says it stops at its first match.
    pub(super) fn candidates(
        &mut self,
        walk: Walk,
        need: &Need,
        input: &str,
        held: &BTreeSet<SlotId>,
    ) -> String {
        let candidates = self.within(held, |this| match this.leaf_candidates(walk, input) {
            Some(candidates) => candidates,
            None => this.tree_candidates(walk, need, input),
        });
        self.open_space(&candidates, input, lineage::Derive::Probe);
        candidates
    }

    /// `candidates` of a walk over the tree.
    fn tree_candidates(&mut self, walk: Walk, need: &Need, input: &str) -> String {
        let kinds = need.kinds.as_deref();
        // Mutant 15: a field relation walks as its plain relation.
        let walk = match walk {
            Walk::FieldHas(_, stop) if super::mutant(15) => Walk::Has(stop),
            Walk::FieldInside(_, stop) if super::mutant(15) => Walk::Inside(stop),
            walk => walk,
        };
        let at = "JOIN node fn ON fn.file = f.file AND fn.id = f.node";
        let simple = |from: &str| {
            format!("SELECT f.k AS pk, c.file AS file, c.id AS node FROM {input} f {from}")
        };
        // The kind test on the candidate `node` row `c`, in its join.
        let c = match kinds {
            None | Some([]) => String::new(),
            Some(kinds) => format!(" AND c.kind IN ({})", list(kinds)),
        };
        let mut with = String::new();
        let source = match walk {
            // A sub-rule no node can match (`Never`'s kinds): no candidate.
            _ if kinds.is_some_and(<[u16]>::is_empty) => {
                format!("SELECT f.k AS pk, f.file AS file, f.node AS node FROM {input} f WHERE 0")
            }
            // Mutant 8: only the named children.
            Walk::Has(Stop::Neighbor) => simple(&format!(
                "JOIN node c ON c.file = f.file AND c.parent = f.node{c}{}",
                if super::mutant(8) {
                    " AND c.named = 1"
                } else {
                    ""
                }
            )),
            Walk::Has(Stop::End) => simple(&format!(
                "{at} JOIN node c ON c.file = f.file AND c.id > f.node AND c.id <= fn.\"last\"{c}"
            )),
            // A rule stop's walks read the stop set `S` through `nearest` (no recursion). `has`
            // visits a descendant `c` of `f` where no node strictly between them is in `S`: where
            // `c`'s nearest proper ancestor in `S` is `f`'s nearest ancestor-or-self in `S`. With
            // kinds, the subtree's nodes of those kinds come from the postings and each is
            // looked up in `nearest`; without, `nearest` lists the nodes under `f`'s base.
            Walk::Has(Stop::Rule(stage)) if kinds.is_some() => simple(&format!(
                "CROSS JOIN nearest nf ON {} AND nf.id = f.node \
                 CROSS JOIN node c ON c.file = f.file AND c.id > f.node AND c.id <= nf.\"last\"{c} \
                 CROSS JOIN nearest nc ON {} AND nc.id = c.id AND nc.anc = nf.base",
                nearest::set_of("nf", stage, "f.file"),
                nearest::set_of("nc", stage, "c.file")
            )),
            Walk::Has(Stop::Rule(stage)) => simple(&format!(
                "CROSS JOIN nearest nf ON {} AND nf.id = f.node \
                 CROSS JOIN nearest c ON {} AND c.anc = nf.base AND c.id > f.node AND c.id <= nf.\"last\"",
                nearest::set_of("nf", stage, "f.file"),
                nearest::set_of("c", stage, "f.file")
            )),
            Walk::Inside(Stop::Neighbor) => simple(&format!(
                "{at} JOIN node c ON c.file = f.file AND c.id = fn.parent{c}"
            )),
            Walk::Inside(Stop::End) => simple(&format!(
                "JOIN node c ON c.file = f.file AND c.id < f.node AND c.\"last\" >= f.node{c}"
            )),
            // `inside` ends at `f`'s nearest proper ancestor in `S`, inclusive.
            Walk::Inside(Stop::Rule(stage)) => simple(&format!(
                "CROSS JOIN nearest nf ON {} AND nf.id = f.node \
                 CROSS JOIN node c ON c.file = f.file AND c.id < f.node AND c.\"last\" >= f.node AND c.id >= nf.anc{c}",
                nearest::set_of("nf", stage, "f.file")
            )),
            Walk::Precedes(Stop::Neighbor) => simple(&format!(
                "{at} JOIN node c ON c.file = f.file AND c.id = fn.next{c}"
            )),
            Walk::Precedes(Stop::End) => simple(&format!(
                "{at} JOIN node c ON c.file = f.file AND c.parent = fn.wpar AND c.id >= fn.anext{c}"
            )),
            // The sibling walks end at the first node in `S` of their chain, inclusive.
            Walk::Precedes(Stop::Rule(stage)) => simple(&format!(
                "{at} CROSS JOIN nearest na ON {} AND na.id = fn.anext \
                 CROSS JOIN node c ON c.file = f.file AND c.parent = fn.wpar AND c.id >= fn.anext \
                 AND c.id <= COALESCE(na.nxt, 4294967295){c}",
                nearest::set_of("na", stage, "f.file")
            )),
            Walk::Follows(Stop::Neighbor) => simple(&format!(
                "{at} JOIN node c ON c.file = f.file AND c.id = fn.prev{c}"
            )),
            Walk::Follows(Stop::End) => simple(&format!(
                "{at} JOIN node c ON c.file = f.file AND c.parent = fn.wpar AND c.id <= fn.aprev{c}"
            )),
            Walk::Follows(Stop::Rule(stage)) => simple(&format!(
                "{at} CROSS JOIN nearest na ON {} AND na.id = fn.aprev \
                 CROSS JOIN node c ON c.file = f.file AND c.parent = fn.wpar AND c.id <= fn.aprev \
                 AND c.id >= COALESCE(na.prv, 0){c}",
                nearest::set_of("na", stage, "f.file")
            )),
            Walk::FieldHas(field, Stop::Neighbor) => simple(&format!(
                "JOIN node c ON c.file = f.file AND c.parent = f.node{c} WHERE c.field = {field} AND c.ffirst = 1"
            )),
            Walk::FieldHas(field, Stop::End) => simple(&format!(
                "JOIN node fc ON fc.file = f.file AND fc.parent = f.node \
                 JOIN node c ON c.file = fc.file AND c.id >= fc.id AND c.id <= fc.\"last\"{c} \
                 WHERE fc.field = {field} AND fc.ffirst = 1"
            )),
            // The field child, then its children where it is not in `S`.
            Walk::FieldHas(field, Stop::Rule(set)) => {
                with = format!(
                    "WITH q AS (SELECT f.k AS pk, c.file AS file, c.id AS node FROM {input} f \
                     JOIN node c ON c.file = f.file AND c.parent = f.node WHERE c.field = {field} AND c.ffirst = 1) "
                );
                let first = if kinds.is_some() {
                    format!(
                        "SELECT q.pk, q.file, q.node FROM q JOIN node c ON c.file = q.file AND c.id = q.node{c}"
                    )
                } else {
                    "SELECT pk, file, node FROM q".to_owned()
                };
                format!(
                    "{first} UNION ALL SELECT x.pk, c.file, c.id FROM q x \
                     JOIN node c ON c.file = x.file AND c.parent = x.node{c} WHERE NOT {}",
                    in_set(set, "x.file", "x.node")
                )
            }
            Walk::FieldInside(field, Stop::Neighbor) => simple(&format!(
                "{at} JOIN node c ON c.file = f.file AND c.id = fn.parent{c} WHERE fn.field = {field} AND fn.ffirst = 1"
            )),
            Walk::FieldInside(field, Stop::End) => simple(&format!(
                "JOIN node w ON w.file = f.file AND w.id <= f.node AND w.\"last\" >= f.node \
                 JOIN node c ON c.file = w.file AND c.id = w.parent{c} WHERE w.field = {field} AND w.ffirst = 1"
            )),
            // The ancestors `inside` walks to, through a field child `w` of each: `w` an
            // ancestor-or-self of `f` below its nearest proper ancestor in `S`.
            Walk::FieldInside(field, Stop::Rule(stage)) => simple(&format!(
                "CROSS JOIN nearest nf ON {} AND nf.id = f.node \
                 CROSS JOIN node w ON w.file = f.file AND w.id <= f.node AND w.\"last\" >= f.node AND w.id > nf.anc \
                 CROSS JOIN node c ON c.file = w.file AND c.id = w.parent{c} WHERE w.field = {field} AND w.ffirst = 1",
                nearest::set_of("nf", stage, "f.file")
            )),
            Walk::Slot(_) | Walk::Related(_) => {
                unreachable!("a terminal's walk (`leaf_candidates`)")
            }
        };
        // Mutant 9: the stop node is not a candidate.
        let source = match walk {
            Walk::Has(Stop::Rule(stage))
            | Walk::Inside(Stop::Rule(stage))
            | Walk::Precedes(Stop::Rule(stage))
            | Walk::Follows(Stop::Rule(stage))
            | Walk::FieldHas(_, Stop::Rule(stage))
            | Walk::FieldInside(_, Stop::Rule(stage))
                if super::mutant(9) =>
            {
                format!(
                    "SELECT * FROM ({source}) m WHERE NOT {}",
                    in_set(stage, "m.file", "m.node")
                )
            }
            _ => source,
        };
        // The shape `need` tests on a candidate, past its kinds.
        let source = match &need.shape {
            Some(shape) => format!(
                "SELECT * FROM ({source}) m WHERE {}",
                shape_test(shape, "m.file", "m.node", &mut 0)
            ),
            None => source,
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

    #[allow(clippy::too_many_arguments)]
    fn nth_child(
        &mut self,
        op: &Op,
        step: i32,
        offset: i32,
        reverse: bool,
        of_rule: Option<&OfRule>,
        input: &str,
        live: &Live,
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
            Some(OfRule::Stage(set)) => {
                // Ranks among the named children `ofRule` holds at, of the input's parents alone:
                // a node's rank reads its parent's children, never the rest of the stage, which
                // can hold most nodes of the batch.
                let with = if super::mutant(19) {
                    // Mutant 19: ranks over the whole stage, not the input's parents' children.
                    format!(
                        "WITH r AS (SELECT n.file AS file, n.id AS node, \
                         ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS r, \
                         COUNT(*) OVER (PARTITION BY n.file) AS cnt FROM {} st \
                         JOIN node n ON n.file = st.file AND n.id = st.node WHERE n.named = 1 AND n.parent IS NOT NULL) ",
                        stage_table(set.stage)
                    )
                } else {
                    format!(
                        "WITH p AS (SELECT DISTINCT n.file AS file, n.parent AS parent FROM {input} f \
                         JOIN node n ON n.file = f.file AND n.id = f.node WHERE n.parent IS NOT NULL), \
                         r AS (SELECT c.file AS file, c.id AS node, \
                         ROW_NUMBER() OVER (PARTITION BY c.file, c.parent ORDER BY c.id) AS r, \
                         COUNT(*) OVER (PARTITION BY c.file, c.parent) AS cnt FROM p \
                         JOIN node c ON c.file = p.file AND c.parent = p.parent \
                         WHERE c.named = 1 AND {}) ",
                        in_set(*set, "c.file", "c.id")
                    )
                };
                let index = if reverse { "(r.cnt - r.r + 1)" } else { "r.r" };
                let select = format!(
                    "SELECT {} FROM {input} f JOIN r ON r.file = f.file AND r.node = f.node WHERE {}",
                    self.cols("f"),
                    holds(index, step, offset)
                );
                let table = set_inputs(*set);
                let mut reads = vec![input];
                reads.extend(table.as_deref());
                self.frame_with("x", &with, &select, Reads::join(&reads))
            }
            Some(OfRule::Frame(rule)) => {
                // The probe returns the slots of `live` the rule may rebind.
                let back = self.rebound(op, live);
                let held = self.live_before(op, &back);
                let siblings = self.name("g");
                self.within(&held, |this| {
                    let columns = format!(
                        "k INTEGER PRIMARY KEY, file, node{}, pk, ord{}",
                        this.physical_list(),
                        this.extra_list()
                    );
                    this.create(&siblings, &columns);
                    this.insert(
                        &siblings,
                        format!(
                            "INSERT INTO {siblings} SELECT NULL, c.file, c.id{}, f.k, c.id{} FROM {input} f \
                             JOIN node fn ON fn.file = f.file AND fn.id = f.node \
                             JOIN node c ON c.file = f.file AND c.parent = fn.parent WHERE c.named = 1",
                            this.physical_of("f"),
                            this.zeros()
                        ),
                        Reads::join(&[input]),
                    );
                });
                self.open_space(&siblings, input, lineage::Derive::Probe);
                let matched = self.op(rule, &siblings, &back);
                let returned = self.within(&back, |this| this.physical_of("s"));
                let with = format!(
                    "WITH r AS (SELECT g.pk AS pk, g.node AS node, \
                     ROW_NUMBER() OVER (PARTITION BY g.pk ORDER BY g.node) AS r, \
                     COUNT(*) OVER (PARTITION BY g.pk) AS cnt{returned} FROM {matched} s JOIN {siblings} g ON g.k = s.k) "
                );
                let index = if reverse { "(r.cnt - r.r + 1)" } else { "r.r" };
                let overrides = self.all_from(&back, "r");
                let select = format!(
                    "SELECT {} FROM {input} f JOIN r ON r.pk = f.k AND r.node = f.node WHERE {}",
                    self.select("f", &overrides),
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
        live: &Live,
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
        // The body leaves `live` and what the return reads: every caller and parameter slot.
        let returned: Live = if returns.is_empty() {
            Rc::clone(live)
        } else {
            let mut returned = (**live).clone();
            for ret in returns {
                returned.insert(ret.caller);
                returned.extend(ret.params.iter().copied());
            }
            Rc::new(returned)
        };
        // The input's rows, each under its own `k`, the parameters bound: the input itself where
        // no parameter is copied, rather than a copy of every row.
        let framed = if overrides.is_empty() {
            input.to_owned()
        } else {
            let held = self.live_before(body, &returned);
            let framed = self.within(&held, |this| {
                this.frame(
                    "i",
                    &format!("SELECT {} FROM {input} f", this.select("f", &overrides)),
                    Reads::copy(input),
                )
            });
            self.same_space(input, &framed);
            framed
        };
        let bound = self.op(body, &framed, &returned);
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
        // Mutant 12: conflicting parameters return anyway.
        let ok = if super::mutant(12) {
            "1".to_owned()
        } else {
            format!("({})", ok.join(" AND "))
        };
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
