//! The rule IR (`docs/facts/IR.md` is its specification; this file is its types).
//!
//! A rule compiles to a [`Program`]: per-file plans, repository plans and the exported relations
//! between them. Relations are bags of rows of SQLite values over typed columns ([`Column`]:
//! name and SQLite affinity). Everything here is immutable after compilation and `Send + Sync`
//! (literals are [`Lit`], not the executors' `Rc`-based values), so one compiled `Program` is
//! shared by every worker through an `Arc`. Row order is part of the semantics where the rules can
//! observe it (`row_number()`, `LIMIT`, `group_concat`, first-row picks); IR.md § Order defines it.
//!
//! Classification is by construction: a relation is generic over the sources it may scan.
//! [`FileSource`] has no variant that reaches another content, an exported relation or the
//! reference graph, so a [`Plan<FileSource>`] cannot express a cross-file read:
//!
//! ```compile_fail
//! use rerail_facts::ir::{FileSource, Rel, RepoSource};
//! // A per-file relation cannot scan an exported relation: the types differ.
//! let leak: Rel<FileSource> = Rel::Scan { src: RepoSource::Exported(0), schema: vec![] };
//! ```
pub use crate::sql::value::Aff;
use serde_json::Value as Json;

pub type InputId = u32;
pub type StageId = u32;
pub type ExportId = u32;
pub type LocalId = u32;

/// A typed column: its name and its SQLite affinity (the declared type of an input or stage
/// column, CTAS's expression-affinity rule for derived columns).
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub name: String,
    pub aff: Aff,
}

impl Column {
    pub fn new(name: &str, aff: Aff) -> Self {
        Self { name: name.to_owned(), aff }
    }
}

/// A literal SQLite value.
#[derive(Clone, Debug, PartialEq)]
pub enum Lit {
    Null,
    Int(i64),
    Real(f64),
    Text(Box<str>),
}

// ---------------------------------------------------------------------------------------------
// Sources

/// What a per-file plan may read: the facts of one content and what it derived from them.
#[derive(Clone, Debug, PartialEq)]
pub enum FileSource {
    /// An input of this content: a pattern (AST) query or a regex clause over its facts, one row
    /// per witness (`__path`, `__start`, `__end`, projected fields in name order).
    Input(InputId),
    /// A table of the plan's `syntax` spec, built from this content's facts.
    Syntax(SyntaxTable),
    /// An earlier stage of the same plan: a rule-private relation derived from this content.
    Stage(StageId),
}

/// What a repository plan may read: exported relations (over the member contents of a
/// worktree) and its own earlier stages.
#[derive(Clone, Debug, PartialEq)]
pub enum RepoSource {
    Exported(ExportId),
    Stage(StageId),
}

/// The front ends' unclassified sources: everything a clause can name. The classifier maps
/// these to [`FileSource`] or [`RepoSource`], or refuses.
#[derive(Clone, Debug, PartialEq)]
pub enum AnySource {
    Input(InputId),
    Syntax(SyntaxTable),
    Stage(StageId),
    /// The native reference graph `reference_edges(__path, __start, __end, target_path,
    /// target_start, target_end, source_name, target_name, target_class)` (relational
    /// `references`): cross-file.
    ReferenceEdges,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxTable {
    Nodes,
    Edges,
    Ownership,
}

// ---------------------------------------------------------------------------------------------
// Scalar expressions

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

/// Scalar functions (the native authorizer's whitelist, as far as the rules use it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Func {
    Coalesce,
    Ifnull,
    Iif,
    Nullif,
    Length,
    Lower,
    Upper,
    Trim,
    Ltrim,
    Rtrim,
    Substr,
    Instr,
    Replace,
    Char,
    Abs,
    Typeof,
    /// `printf(format, ...)` (SQLite's `%` formats).
    Printf,
    /// Scalar `max(a, b, ...)` / `min(a, b, ...)` (two or more arguments).
    Max,
    Min,
    JsonQuote,
    JsonExtract,
    /// `json_array(a, ...)`.
    JsonArray,
    /// `rx_matches(text, pattern)`: Rust `regex`, NULL text reads as `''`.
    RxMatches,
    /// `rx_extract(text, pattern, group)`.
    RxExtract,
    /// `rx_replace(text, pattern, replacement)`.
    RxReplace,
    /// `source_text(path, start, end)`: byte slice of a content's source. In a per-file plan the
    /// only readable path is the plan's own content.
    SourceText,
}

/// A scalar expression evaluated over one row. `Col { depth: 0 }` reads the row of the operator
/// holding the expression; `depth: d` reads the row `d` subquery levels out (correlation).
#[derive(Clone, Debug)]
pub enum Expr<S> {
    Lit(Lit),
    Col { depth: u16, col: u16 },
    Not(Box<Expr<S>>),
    Neg(Box<Expr<S>>),
    /// Unary `+`: the value unchanged, its affinity dropped.
    Plus(Box<Expr<S>>),
    Arith(ArithOp, Box<Expr<S>>, Box<Expr<S>>),
    Concat(Box<Expr<S>>, Box<Expr<S>>),
    And(Box<Expr<S>>, Box<Expr<S>>),
    Or(Box<Expr<S>>, Box<Expr<S>>),
    /// Comparison with the operands' comparison affinities applied first (SQLite § 4.2).
    Cmp(CmpOp, Box<Expr<S>>, Box<Expr<S>>, (Aff, Aff)),
    /// `a IS b` / `a IS NOT b`.
    Is { left: Box<Expr<S>>, right: Box<Expr<S>>, negated: bool },
    IsNull { e: Box<Expr<S>>, negated: bool },
    /// `text LIKE pattern` (ASCII case-insensitive, `%` and `_`).
    Like { text: Box<Expr<S>>, pattern: Box<Expr<S>>, negated: bool },
    Func(Func, Vec<Expr<S>>),
    /// `CASE [operand] WHEN w THEN t … [ELSE e] END`; with an operand, each `WHEN` compares with
    /// its own comparison affinities.
    Case { operand: Option<Box<Expr<S>>>, whens: Vec<(Expr<S>, Expr<S>, (Aff, Aff))>, else_: Option<Box<Expr<S>>> },
    Cast(Box<Expr<S>>, Aff),
    InList { e: Box<Expr<S>>, list: Vec<(Expr<S>, (Aff, Aff))>, negated: bool },
    /// `e [NOT] IN (subquery)`: semijoin / antijoin with SQLite's NULL rules.
    InRel { e: Box<Expr<S>>, rel: Box<Rel<S>>, negated: bool, aff: (Aff, Aff) },
    /// `[NOT] EXISTS (subquery)`: semijoin / antijoin.
    Exists { rel: Box<Rel<S>>, negated: bool },
    /// Scalar subquery: the first column of the first row in the relation's order, NULL if empty
    /// (`ORDER BY … LIMIT 1` is a [`Rel::Sort`] inside).
    Scalar(Box<Rel<S>>),
}

// ---------------------------------------------------------------------------------------------
// Relations

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinKind {
    Inner,
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AggFn {
    /// `count(x)`: non-NULL values.
    Count,
    /// `count(*)`.
    CountStar,
    Min,
    Max,
    Sum,
    Total,
    /// `group_concat(x [, sep])` in input order.
    GroupConcat,
    /// `json_group_array(x)` in input order.
    JsonGroupArray,
}

#[derive(Clone, Debug)]
pub struct AggCall<S> {
    pub f: AggFn,
    pub arg: Option<Expr<S>>,
    pub sep: Option<Expr<S>>,
    pub distinct: bool,
}

#[derive(Clone, Debug)]
pub enum WindowKind<S> {
    /// `row_number()`, numbered in the total order (partition values, order keys, input
    /// position).
    RowNumber,
    /// `min(x)` / `max(x)` over the whole partition (no `ORDER BY`, so no frame).
    Min(Expr<S>),
    Max(Expr<S>),
}

/// A window function `f OVER (PARTITION BY … ORDER BY …)`.
#[derive(Clone, Debug)]
pub struct WindowFn<S> {
    pub kind: WindowKind<S>,
    pub partition: Vec<Expr<S>>,
    pub order: Vec<(Expr<S>, bool)>,
}

/// A `WITH` binding: a plain relation, or a recursive fixpoint.
#[derive(Clone, Debug)]
pub enum Binding<S> {
    Plain(Rel<S>),
    /// `WITH RECURSIVE`: `init` arms seed the result; each `step` arm reads the previous level
    /// (`Rel::Local(id)` inside the step) and adds the next. `set`: `UNION` (rows already in the
    /// result are dropped, so it terminates on finite domains); bag (`UNION ALL`): every derived
    /// row is kept and it terminates iff every derivation chain is finite (IR.md § Recursion).
    Fix { init: Vec<Rel<S>>, step: Vec<Rel<S>>, set: bool, schema: Vec<Column> },
}

/// A relational operator. Every variant's output schema is defined in IR.md § Relations.
#[derive(Clone, Debug)]
pub enum Rel<S> {
    /// A source relation.
    Scan { src: S, schema: Vec<Column> },
    /// A `WITH` binding (or, inside a fixpoint's step, the fixpoint's previous level).
    Local { id: LocalId, schema: Vec<Column> },
    /// One row with no columns (`SELECT` without `FROM`).
    Unit,
    /// For each input row, the rows of `json_each(arg)` (arg over the input row: lateral when it
    /// reads it) that satisfy `on` (over input ++ element): input columns, then `key, value,
    /// type, atom, id, parent, fullkey, path`. `outer`: an input row with no such element is
    /// kept once, NULL-extended (`LEFT JOIN json_each`).
    JsonEach { input: Box<Rel<S>>, arg: Expr<S>, on: Option<Expr<S>>, outer: bool },
    /// Nested-loop join in input order: left columns then right columns; `on` over both.
    Join { kind: JoinKind, left: Box<Rel<S>>, right: Box<Rel<S>>, on: Option<Expr<S>> },
    Filter { input: Box<Rel<S>>, pred: Expr<S> },
    Project { input: Box<Rel<S>>, exprs: Vec<Expr<S>>, schema: Vec<Column> },
    /// Groups of `keys` in first-seen order (no keys and no rows: one group). Output: the group's
    /// representative input row (SQLite bare columns: the row a lone `min`/`max` picked, else
    /// the group's last row; all NULL for an empty input), then the aggregates.
    Aggregate { input: Box<Rel<S>>, keys: Vec<Expr<S>>, aggs: Vec<AggCall<S>> },
    /// Input columns, then one column per window function; rows stay in input order.
    Window { input: Box<Rel<S>>, funcs: Vec<WindowFn<S>> },
    /// First occurrence of each distinct value of the first `prefix` columns.
    Distinct { input: Box<Rel<S>>, prefix: u16 },
    /// `left UNION [ALL] right`: bag concatenation, or set union keeping first occurrences.
    Union { all: bool, left: Box<Rel<S>>, right: Box<Rel<S>> },
    /// Stable sort on input columns, then `OFFSET`/`LIMIT` (constant expressions).
    Sort { input: Box<Rel<S>>, keys: Vec<(u16, bool)>, limit: Option<Expr<S>>, offset: Option<Expr<S>> },
    /// Bindings visible in `body` (and in later bindings).
    With { bindings: Vec<(LocalId, Binding<S>)>, body: Box<Rel<S>> },
}

// ---------------------------------------------------------------------------------------------
// Inputs (leaf relations over one content's facts)

/// A projected field of an input row (`evidence.project`).
#[derive(Clone, Debug, PartialEq)]
pub enum Projection {
    Path,
    Directory,
    Name,
    Stem,
    Capture(String),
    Text(String),
    Kind(String),
    StartByte(String),
    EndByte(String),
    StartLine(String),
    /// `file.module` / `file.package`: the content's module path, derived from the package
    /// markers (`__init__.py`) of the other contents of the snapshot: cross-file.
    Module,
    Package,
    Regex { source: Box<Projection>, regex: String, group: u32 },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Anchor {
    Span,
    Start,
    End,
    FileStart,
    Capture(String),
}

#[derive(Clone, Debug)]
pub enum InputKind {
    /// An AST pattern query (`forbid.ast`, `evidence.query.ast`), resolved for the call
    /// configuration, `when` evaluated and removed; executed by the pattern leaf executor.
    Pattern { query: Json },
    /// A regex clause (`forbid.regex`): `source`, `pattern`, `include`, `exclude`, `lines`, `paths`.
    Regex { spec: Json },
}

/// A leaf relation evaluated over one content. Its rows: `__path TEXT`, `__start INTEGER`,
/// `__end INTEGER` (the anchored span), then the projected fields in name order (no affinity).
#[derive(Clone, Debug)]
pub struct InputDef {
    pub name: String,
    pub kind: InputKind,
    pub fields: Vec<(String, Projection)>,
    pub anchor: Anchor,
    pub each: Option<String>,
    /// `evidence.paths`: an include/exclude glob filter on the content's path.
    pub paths: Option<Json>,
    pub schema: Vec<Column>,
}

// ---------------------------------------------------------------------------------------------
// Plans

/// A relational clause's `syntax` spec: node, edge and ownership tables built from facts.
#[derive(Clone, Debug)]
pub struct SyntaxSpec {
    pub spec: Json,
    pub gate: InputId,
    pub nodes: Vec<Column>,
    pub edges: Option<Vec<Column>>,
    pub ownership: Option<Vec<Column>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Graph {
    Directed,
    Forest,
    /// Descendants over the plan's `syntax` node table: an id-range test on facts.
    Syntax,
}

#[derive(Clone, Debug)]
pub enum StageDef<S> {
    /// A stage query. `deferred` columns are lowered into it: a `Project` over the query whose
    /// replaced or appended columns are scalar expressions over the stored columns (deterministic,
    /// so computing them on read or on write is the same relation; laziness is physical).
    Query { rel: Rel<S> },
    /// Reachability `(__path, root, node)`, reflexive, set semantics, per path.
    Reach { roots: Rel<S>, edges: Option<Rel<S>>, graph: Graph },
}

#[derive(Clone, Debug)]
pub struct Stage<S> {
    pub name: String,
    pub schema: Vec<Column>,
    pub def: StageDef<S>,
}

/// The clause's `violation` as far as findings need it.
#[derive(Clone, Debug)]
pub struct Violation {
    pub code: String,
    pub anchor: Anchor,
    pub each: Option<String>,
}

/// One stage of an evidence `closure` (native `query/name_closure.rs`): per partition (rows
/// with equal non-NULL `partition_by` values), the names reachable from the seeds over
/// `edges`; `contains` annotates every row with `output := (row[field] ∈ reached)`.
#[derive(Clone, Debug)]
pub struct ClosureStage {
    pub name: String,
    pub seeds: Vec<String>,
    pub seed_field: Option<String>,
    pub extends: Vec<String>,
    pub partition_by: Vec<String>,
    pub edges: Option<(String, String)>,
    pub contains: Vec<(String, String)>,
}

/// A budget's row filter (native `query/crossfile.rs` `Condition`): JSON-typed comparisons.
pub type Condition = Json;

/// Where a plan's rows go.
#[derive(Clone, Debug)]
pub enum Sink<S> {
    /// `forbid`: one finding per witness of the input.
    Witnesses { input: InputId, violation: Violation },
    /// `budget`: rows of the input relation (`rel`, whose columns are the input's) annotated by
    /// `closure`, filtered by `condition`, grouped by `group_by` (field names); a group whose
    /// count (of distinct `distinct` tuples, if any) exceeds `lte` is one finding at its first row.
    Budget { rel: Rel<S>, closure: Vec<ClosureStage>, condition: Option<Condition>, group_by: Vec<String>, distinct: Vec<String>, lte: u64, violation: Violation },
    /// A relational final query: each row is a finding at `(path, start_byte, end_byte)`,
    /// identity `key`, code from column `code`; `locations` allow `(path, 0, 0)`.
    Findings { rel: Rel<S>, key: Vec<String>, code: Option<String>, locations: Vec<String>, violation: Violation },
    /// The per-file side of a repository clause: one content's rows of each exported relation.
    Export(Vec<(ExportId, Rel<S>)>),
}

#[derive(Clone, Debug)]
pub struct Plan<S> {
    pub clause: String,
    pub syntax: Option<SyntaxSpec>,
    pub stages: Vec<Stage<S>>,
    pub sink: Sink<S>,
}

/// A relation a repository plan reads: the union over a worktree's member contents of one
/// content's rows, produced by `producer` (an index into [`Program::per_file`]).
#[derive(Clone, Debug)]
pub struct ExportedRelation {
    pub id: ExportId,
    pub name: String,
    pub schema: Vec<Column>,
    /// Rows are made distinct per content (the repository plan only reads it as a set).
    pub distinct: bool,
    /// Row order matters to the repository plan (it reaches `row_number()`, `LIMIT` or
    /// `group_concat` without a total order): the union is in path order, rows in content order.
    pub ordered: bool,
    pub producer: usize,
}

/// A construct a rule uses and the compiler refuses, with where it occurs.
#[derive(Clone, Debug, PartialEq)]
pub struct Refusal {
    pub clause: String,
    /// JSON pointer-like location inside the clause, or the stage name.
    pub at: String,
    /// The construct, named.
    pub construct: String,
}

/// Compile-time refusal: every refused construct of the rule.
#[derive(Clone, Debug, PartialEq)]
pub struct Unsupported {
    pub rule: String,
    pub refusals: Vec<Refusal>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Partition {
    File,
    Repository,
}

/// A compiled rule.
#[derive(Clone, Debug)]
pub struct Program {
    pub rule: String,
    pub inputs: Vec<InputDef>,
    pub per_file: Vec<Plan<FileSource>>,
    pub repository: Vec<Plan<RepoSource>>,
    pub exported: Vec<ExportedRelation>,
    /// `define.report_when.count.gt`: the rule's findings are reported only when their number
    /// over the evaluation exceeds it (a repository-level gate over every plan's findings).
    pub report_when: Option<u64>,
    /// Constructs that compile but the leaf executor refuses per content at run time (a failure
    /// for that content, never an approximation), e.g. `parse: {grammar: annotation}`.
    pub runtime_refusals: Vec<Refusal>,
}

/// A compiled program is shared by every worker: it must stay `Send + Sync`.
#[allow(dead_code)]
fn program_is_shareable() {
    fn shareable<T: Send + Sync>() {}
    shareable::<Program>();
}

// ---------------------------------------------------------------------------------------------
// Traversal

/// Rewrites the column references of a scalar expression outside its subqueries:
/// `Col { depth, col }` becomes `Col { depth, col: f(depth, col) }`.
pub fn map_cols<S>(e: Expr<S>, f: &mut dyn FnMut(u16, u16) -> u16) -> Expr<S> {
    let b = |x: Box<Expr<S>>, f: &mut dyn FnMut(u16, u16) -> u16| Box::new(map_cols(*x, f));
    match e {
        Expr::Col { depth, col } => Expr::Col { depth, col: f(depth, col) },
        Expr::Lit(l) => Expr::Lit(l),
        Expr::Not(x) => Expr::Not(b(x, f)),
        Expr::Neg(x) => Expr::Neg(b(x, f)),
        Expr::Plus(x) => Expr::Plus(b(x, f)),
        Expr::Cast(x, a) => Expr::Cast(b(x, f), a),
        Expr::Arith(o, x, y) => Expr::Arith(o, b(x, f), b(y, f)),
        Expr::Concat(x, y) => Expr::Concat(b(x, f), b(y, f)),
        Expr::And(x, y) => Expr::And(b(x, f), b(y, f)),
        Expr::Or(x, y) => Expr::Or(b(x, f), b(y, f)),
        Expr::Cmp(o, x, y, a) => Expr::Cmp(o, b(x, f), b(y, f), a),
        Expr::Is { left, right, negated } => Expr::Is { left: b(left, f), right: b(right, f), negated },
        Expr::IsNull { e, negated } => Expr::IsNull { e: b(e, f), negated },
        Expr::Like { text, pattern, negated } => Expr::Like { text: b(text, f), pattern: b(pattern, f), negated },
        Expr::Func(fu, args) => Expr::Func(fu, args.into_iter().map(|a| map_cols(a, f)).collect()),
        Expr::Case { operand, whens, else_ } => Expr::Case {
            operand: operand.map(|o| b(o, f)),
            whens: whens.into_iter().map(|(w, t, a)| (map_cols(w, f), map_cols(t, f), a)).collect(),
            else_: else_.map(|x| b(x, f)),
        },
        Expr::InList { e, list, negated } => Expr::InList { e: b(e, f), list: list.into_iter().map(|(x, a)| (map_cols(x, f), a)).collect(), negated },
        Expr::InRel { e, rel, negated, aff } => Expr::InRel { e: b(e, f), rel, negated, aff },
        e @ (Expr::Exists { .. } | Expr::Scalar(_)) => e,
    }
}

/// Maps every scanned source of a relation (including inside subquery expressions), or fails
/// with the first error.
pub fn map_rel<S, T, X>(r: Rel<S>, f: &mut dyn FnMut(S) -> Result<T, X>) -> Result<Rel<T>, X> {
    let bx = |r: Box<Rel<S>>, f: &mut dyn FnMut(S) -> Result<T, X>| -> Result<Box<Rel<T>>, X> { Ok(Box::new(map_rel(*r, f)?)) };
    Ok(match r {
        Rel::Scan { src, schema } => Rel::Scan { src: f(src)?, schema },
        Rel::Local { id, schema } => Rel::Local { id, schema },
        Rel::Unit => Rel::Unit,
        Rel::JsonEach { input, arg, on, outer } => Rel::JsonEach { input: bx(input, f)?, arg: map_expr(arg, f)?, on: on.map(|o| map_expr(o, f)).transpose()?, outer },
        Rel::Join { kind, left, right, on } => Rel::Join { kind, left: bx(left, f)?, right: bx(right, f)?, on: on.map(|o| map_expr(o, f)).transpose()? },
        Rel::Filter { input, pred } => Rel::Filter { input: bx(input, f)?, pred: map_expr(pred, f)? },
        Rel::Project { input, exprs, schema } => Rel::Project { input: bx(input, f)?, exprs: exprs.into_iter().map(|e| map_expr(e, f)).collect::<Result<_, _>>()?, schema },
        Rel::Aggregate { input, keys, aggs } => Rel::Aggregate {
            input: bx(input, f)?,
            keys: keys.into_iter().map(|e| map_expr(e, f)).collect::<Result<_, _>>()?,
            aggs: aggs.into_iter().map(|a| Ok(AggCall { f: a.f, arg: a.arg.map(|e| map_expr(e, f)).transpose()?, sep: a.sep.map(|e| map_expr(e, f)).transpose()?, distinct: a.distinct })).collect::<Result<_, _>>()?,
        },
        Rel::Window { input, funcs } => Rel::Window {
            input: bx(input, f)?,
            funcs: funcs
                .into_iter()
                .map(|w| {
                    Ok(WindowFn {
                        kind: match w.kind {
                            WindowKind::RowNumber => WindowKind::RowNumber,
                            WindowKind::Min(e) => WindowKind::Min(map_expr(e, f)?),
                            WindowKind::Max(e) => WindowKind::Max(map_expr(e, f)?),
                        },
                        partition: w.partition.into_iter().map(|e| map_expr(e, f)).collect::<Result<_, _>>()?,
                        order: w.order.into_iter().map(|(e, d)| Ok((map_expr(e, f)?, d))).collect::<Result<_, _>>()?,
                    })
                })
                .collect::<Result<_, _>>()?,
        },
        Rel::Distinct { input, prefix } => Rel::Distinct { input: bx(input, f)?, prefix },
        Rel::Union { all, left, right } => Rel::Union { all, left: bx(left, f)?, right: bx(right, f)? },
        Rel::Sort { input, keys, limit, offset } => Rel::Sort { input: bx(input, f)?, keys, limit: limit.map(|e| map_expr(e, f)).transpose()?, offset: offset.map(|e| map_expr(e, f)).transpose()? },
        Rel::With { bindings, body } => Rel::With {
            bindings: bindings
                .into_iter()
                .map(|(id, b)| {
                    Ok((
                        id,
                        match b {
                            Binding::Plain(r) => Binding::Plain(map_rel(r, f)?),
                            Binding::Fix { init, step, set, schema } => Binding::Fix {
                                init: init.into_iter().map(|r| map_rel(r, f)).collect::<Result<_, _>>()?,
                                step: step.into_iter().map(|r| map_rel(r, f)).collect::<Result<_, _>>()?,
                                set,
                                schema,
                            },
                        },
                    ))
                })
                .collect::<Result<_, _>>()?,
            body: bx(body, f)?,
        },
    })
}

/// Maps the sources scanned inside an expression's subqueries.
pub fn map_expr<S, T, X>(e: Expr<S>, f: &mut dyn FnMut(S) -> Result<T, X>) -> Result<Expr<T>, X> {
    let bx = |x: Box<Expr<S>>, f: &mut dyn FnMut(S) -> Result<T, X>| -> Result<Box<Expr<T>>, X> { Ok(Box::new(map_expr(*x, f)?)) };
    Ok(match e {
        Expr::Lit(l) => Expr::Lit(l),
        Expr::Col { depth, col } => Expr::Col { depth, col },
        Expr::Not(x) => Expr::Not(bx(x, f)?),
        Expr::Neg(x) => Expr::Neg(bx(x, f)?),
        Expr::Plus(x) => Expr::Plus(bx(x, f)?),
        Expr::Cast(x, a) => Expr::Cast(bx(x, f)?, a),
        Expr::Arith(o, x, y) => Expr::Arith(o, bx(x, f)?, bx(y, f)?),
        Expr::Concat(x, y) => Expr::Concat(bx(x, f)?, bx(y, f)?),
        Expr::And(x, y) => Expr::And(bx(x, f)?, bx(y, f)?),
        Expr::Or(x, y) => Expr::Or(bx(x, f)?, bx(y, f)?),
        Expr::Cmp(o, x, y, a) => Expr::Cmp(o, bx(x, f)?, bx(y, f)?, a),
        Expr::Is { left, right, negated } => Expr::Is { left: bx(left, f)?, right: bx(right, f)?, negated },
        Expr::IsNull { e, negated } => Expr::IsNull { e: bx(e, f)?, negated },
        Expr::Like { text, pattern, negated } => Expr::Like { text: bx(text, f)?, pattern: bx(pattern, f)?, negated },
        Expr::Func(fu, args) => Expr::Func(fu, args.into_iter().map(|a| map_expr(a, f)).collect::<Result<_, _>>()?),
        Expr::Case { operand, whens, else_ } => Expr::Case {
            operand: operand.map(|o| bx(o, f)).transpose()?,
            whens: whens.into_iter().map(|(w, t, a)| Ok((map_expr(w, f)?, map_expr(t, f)?, a))).collect::<Result<_, _>>()?,
            else_: else_.map(|x| bx(x, f)).transpose()?,
        },
        Expr::InList { e, list, negated } => Expr::InList { e: bx(e, f)?, list: list.into_iter().map(|(x, a)| Ok((map_expr(x, f)?, a))).collect::<Result<_, _>>()?, negated },
        Expr::InRel { e, rel, negated, aff } => Expr::InRel { e: bx(e, f)?, rel: Box::new(map_rel(*rel, f)?), negated, aff },
        Expr::Exists { rel, negated } => Expr::Exists { rel: Box::new(map_rel(*rel, f)?), negated },
        Expr::Scalar(rel) => Expr::Scalar(Box::new(map_rel(*rel, f)?)),
    })
}

impl<S> Rel<S> {
    /// The output schema (IR.md § 4).
    pub fn schema(&self) -> Vec<Column> {
        match self {
            Rel::Scan { schema, .. } | Rel::Local { schema, .. } | Rel::Project { schema, .. } => schema.clone(),
            Rel::Unit => vec![],
            Rel::JsonEach { input, .. } => {
                let mut s = input.schema();
                s.extend(["key", "value", "type", "atom", "id", "parent", "fullkey", "path"].iter().map(|c| Column::new(c, Aff::None)));
                s
            }
            Rel::Join { left, right, .. } => {
                let mut s = left.schema();
                s.extend(right.schema());
                s
            }
            Rel::Filter { input, .. } | Rel::Distinct { input, .. } | Rel::Sort { input, .. } => input.schema(),
            Rel::Aggregate { input, aggs, .. } => {
                let mut s = input.schema();
                s.extend((0..aggs.len()).map(|i| Column::new(&format!("__agg_{i}"), Aff::None)));
                s
            }
            Rel::Window { input, funcs } => {
                let mut s = input.schema();
                s.extend((0..funcs.len()).map(|i| Column::new(&format!("__win_{i}"), Aff::None)));
                s
            }
            Rel::Union { left, .. } => left.schema(),
            Rel::With { body, .. } => body.schema(),
        }
    }
}
