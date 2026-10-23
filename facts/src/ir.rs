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
    /// A relation of the content's syntactic core (SCHEMA § 1.2, I2): what the pattern front end
    /// lowers matcher queries over.
    Core(CoreRel),
    /// Native's call summary of this content (SCHEMA § 3.4, `cgn_*`), rows
    /// `(__path, variant, …)` under every symbolic variant, after `as_references` with `links`
    /// (the clause's `siblingLinks`/`constructorFields` as canonical JSON: part of the plan's
    /// digest, INVARIANTS I7), in the reading the path selects (`.pyi`: stub).
    Summary { rel: SummaryRel, links: Box<str> },
}

/// The relations of native's per-content call summary the reference graph reads (SCHEMA § 3.4).
/// Values are texts as exported (`cgnative::render`); names spell sentinel runs as `\u{1}r`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SummaryRel {
    Module,
    Def,
    Edge,
    Alias,
    ReferenceAlias,
    Opaque,
    Open,
}

impl SummaryRel {
    pub const ALL: [SummaryRel; 7] = [SummaryRel::Module, SummaryRel::Def, SummaryRel::Edge, SummaryRel::Alias, SummaryRel::ReferenceAlias, SummaryRel::Opaque, SummaryRel::Open];

    /// The export's relation name (`cgn_<rel>`).
    pub fn name(self) -> &'static str {
        match self {
            SummaryRel::Module => "cgn_module",
            SummaryRel::Def => "cgn_def",
            SummaryRel::Edge => "cgn_edge",
            SummaryRel::Alias => "cgn_alias",
            SummaryRel::ReferenceAlias => "cgn_reference_alias",
            SummaryRel::Opaque => "cgn_opaque",
            SummaryRel::Open => "cgn_open",
        }
    }

    /// The summary's own columns (after `__path, variant`).
    pub fn columns(self) -> &'static [&'static str] {
        match self {
            SummaryRel::Module | SummaryRel::Opaque => &["name"],
            SummaryRel::Def => &["idx", "name", "start", "end", "flags"],
            SummaryRel::Edge => &["def", "name"],
            SummaryRel::Alias | SummaryRel::ReferenceAlias => &["key", "target"],
            SummaryRel::Open => &["open", "open_sites"],
        }
    }

    pub fn schema(self) -> Vec<Column> {
        let mut s = vec![Column::new("__path", Aff::Text), Column::new("variant", Aff::None)];
        s.extend(self.columns().iter().map(|c| Column::new(c, Aff::None)));
        s
    }
}

/// The syntactic core and parser facts of one content, as relations (SCHEMA § 1.2/1.3). Rows in
/// id order (`line`: line order); columns as [`CoreRel::schema`]; no declared affinity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CoreRel {
    /// `file(path)`: one row, the content's path in the snapshot being evaluated.
    File,
    /// `node(id, kind, parent, start, end, start_line, end_line, subtree_end, named, flags,
    /// field)`: every node, preorder.
    Node,
    /// `child(parent, idx, child)`.
    Child,
    /// `field(parent, field, child, rank)`.
    Field,
    /// `identifier(id, name)`.
    Identifier,
    /// `line(no, start, end, indent)`.
    Line,
    /// `syntax_error()`: one row when the tree has an error or missing node, else none.
    SyntaxError,
    /// `string_value(id, value)`: `string`/`concatenated_string` nodes whose literal decodes to a
    /// `str`.
    StringValue,
    /// `string_value_unknown(id, named_escape)`: those whose value is unknown (interpolation,
    /// str/bytes mix, undecodable); `named_escape` = 1 when a `\N{...}` escape was seen.
    StringValueUnknown,
}

impl CoreRel {
    pub fn columns(self) -> &'static [&'static str] {
        match self {
            CoreRel::File => &["path"],
            CoreRel::Node => &["id", "kind", "parent", "start", "end", "start_line", "end_line", "subtree_end", "named", "flags", "field"],
            CoreRel::Child => &["parent", "idx", "child"],
            CoreRel::Field => &["parent", "field", "child", "rank"],
            CoreRel::Identifier => &["id", "name"],
            CoreRel::Line => &["no", "start", "end", "indent"],
            CoreRel::SyntaxError => &[],
            CoreRel::StringValue => &["id", "value"],
            CoreRel::StringValueUnknown => &["id", "named_escape"],
        }
    }

    pub fn schema(self) -> Vec<Column> {
        self.columns().iter().map(|c| Column::new(c, Aff::None)).collect()
    }

    pub fn name(self) -> &'static str {
        match self {
            CoreRel::File => "file",
            CoreRel::Node => "node",
            CoreRel::Child => "child",
            CoreRel::Field => "field",
            CoreRel::Identifier => "identifier",
            CoreRel::Line => "line",
            CoreRel::SyntaxError => "syntax_error",
            CoreRel::StringValue => "string_value",
            CoreRel::StringValueUnknown => "string_value_unknown",
        }
    }
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
    /// A membership relation (I6): the union over the worktree's member contents of a
    /// per-content core relation (`member(__path)`: every member's path, which `file.module`'s
    /// package chain reads). Only the views defined over the worktree read it. A per-file plan
    /// cannot (`classify::file_source` refuses it); a repository clause exports the per-content
    /// [`FileSource::Core`] it is the union of.
    Members(CoreRel),
    /// A membership relation (I6): the union over the members of a per-content summary relation
    /// ([`FileSource::Summary`]: every member's `cgn_*`, which `reference_edges` links). Read and
    /// classified as [`AnySource::Members`].
    Summaries { rel: SummaryRel, links: Box<str> },
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
    /// `rerail_refuse(construct)`: evaluating it refuses with the construct named (INVARIANTS
    /// I9); a relation that may hold a row the engine cannot answer exactly computes it there.
    Refuse,
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
    /// For `group_concat`/`json_group_array`: whether the order it concatenates in is total.
    pub total: Total,
}

/// Whether an order-dependent construct (`row_number()`, `LIMIT`/`OFFSET`, ordered aggregates)
/// sees a total order (IR.md § Order). Set by the classifier on repository plans; S3's generator
/// and S2's executor emit a tie check only for `Checked`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Total {
    /// A per-file plan: ties are broken by input order, which is deterministic (IR.md § 4).
    #[default]
    InputOrder,
    /// A repository plan whose order keys contain a candidate key of the input (no two
    /// distinct rows tie): proven at compile time.
    Proven,
    /// A repository plan whose order is not proven total: a tie between distinct rows is a
    /// run-time refusal of the clause (retractable per worktree), never a guess.
    Checked,
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
    /// `row_number()`: whether partition + order is total.
    pub total: Total,
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
    /// `total`: whether the keys order the input totally (matters with `limit`/`offset`, and for
    /// an ordered aggregate above).
    Sort { input: Box<Rel<S>>, keys: Vec<(u16, bool)>, limit: Option<Expr<S>>, offset: Option<Expr<S>>, total: Total },
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
    /// The pattern front end's lowering of this input to the algebra over the content's core
    /// relations (I3), or the construct it refuses.
    pub algebra: Result<Lowered, String>,
}

/// A matcher input lowered to the algebra (`front::pattern`): its rows (the input's schema, in
/// the leaf executor's order) over [`FileSource::Core`] relations only, and the relations whose
/// non-emptiness fails the content with the given message (the executor's run-time refusals).
#[derive(Clone, Debug)]
pub struct Lowered {
    pub rows: Rel<FileSource>,
    pub refuse_if: Vec<(Rel<FileSource>, String)>,
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
            aggs: aggs.into_iter().map(|a| Ok(AggCall { f: a.f, arg: a.arg.map(|e| map_expr(e, f)).transpose()?, sep: a.sep.map(|e| map_expr(e, f)).transpose()?, distinct: a.distinct, total: a.total })).collect::<Result<_, _>>()?,
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
                        total: w.total,
                    })
                })
                .collect::<Result<_, _>>()?,
        },
        Rel::Distinct { input, prefix } => Rel::Distinct { input: bx(input, f)?, prefix },
        Rel::Union { all, left, right } => Rel::Union { all, left: bx(left, f)?, right: bx(right, f)? },
        Rel::Sort { input, keys, limit, offset, total } => Rel::Sort { input: bx(input, f)?, keys, limit: limit.map(|e| map_expr(e, f)).transpose()?, offset: offset.map(|e| map_expr(e, f)).transpose()?, total },
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

// ---------------------------------------------------------------------------------------------
// Analysis shared by the backends (join keys, semijoins)

/// One side of an equality key: an expression over that side's row (depth 0, no subquery, no
/// outer reference) and the comparison affinities of the equality it came from, `(this side,
/// the other side)` (SQLite § 4.2: both are needed to decide which conversion applies). SQL `=`:
/// a NULL on either side matches nothing.
#[derive(Clone, Debug)]
pub struct Key<S> {
    pub e: Expr<S>,
    pub affs: (Aff, Aff),
}

/// The conjuncts of a predicate, left to right.
pub fn conjuncts<S: Clone>(e: &Expr<S>) -> Vec<Expr<S>> {
    fn go<S: Clone>(e: &Expr<S>, out: &mut Vec<Expr<S>>) {
        match e {
            Expr::And(a, b) => {
                go(a, out);
                go(b, out)
            }
            _ => out.push(e.clone()),
        }
    }
    let mut out = Vec::new();
    go(e, &mut out);
    out
}

/// The conjunction of `v` in order (`None` for no conjunct).
pub fn conjoin<S>(v: Vec<Expr<S>>) -> Option<Expr<S>> {
    v.into_iter().reduce(|a, b| Expr::And(Box::new(a), Box::new(b)))
}

/// Calls `f(col)` for each column of the operator's own row (depth 0) an expression reads,
/// including through its subqueries (where that row is at depth 1, 2, …).
pub fn cols_read<S>(e: &Expr<S>, f: &mut dyn FnMut(u16)) {
    fn ex<S>(e: &Expr<S>, d: u16, f: &mut dyn FnMut(u16)) {
        each_subexpr(e, &mut |x| match x {
            Expr::Col { depth, col } if *depth == d => f(*col),
            Expr::InRel { rel, .. } | Expr::Exists { rel, .. } | Expr::Scalar(rel) => rel_exprs(rel, &mut |y| ex(y, d + 1, f)),
            _ => {}
        });
    }
    ex(e, 0, f)
}

/// Whether an expression reads a row `k` or more levels out of its operator (`k = 1`: the row
/// enclosing the subquery the operator belongs to).
pub fn reads_outer<S>(e: &Expr<S>, k: u16) -> bool {
    let mut found = false;
    each_subexpr(e, &mut |x| match x {
        Expr::Col { depth, .. } if *depth >= k => found = true,
        Expr::InRel { rel, .. } | Expr::Exists { rel, .. } | Expr::Scalar(rel) => found |= rel_reads_outer(rel, k + 1),
        _ => {}
    });
    found
}

/// Whether a relation's expressions read a row `k` or more levels out of it.
pub fn rel_reads_outer<S>(r: &Rel<S>, k: u16) -> bool {
    let mut found = false;
    rel_exprs_deep(r, &mut |e| found |= reads_outer(e, k));
    found
}

/// Whether an expression holds a subquery (outside nested relations it is not asked about).
pub fn has_subquery<S>(e: &Expr<S>) -> bool {
    let mut found = false;
    each_subexpr(e, &mut |x| found |= matches!(x, Expr::InRel { .. } | Expr::Exists { .. } | Expr::Scalar(_)));
    found
}

/// Visits `e` and its scalar sub-expressions (not the relations of its subqueries; a subquery
/// expression itself and an `InRel`'s left operand are visited).
pub fn each_subexpr<S>(e: &Expr<S>, f: &mut dyn FnMut(&Expr<S>)) {
    f(e);
    match e {
        Expr::Lit(_) | Expr::Col { .. } | Expr::Exists { .. } | Expr::Scalar(_) => {}
        Expr::Not(x) | Expr::Neg(x) | Expr::Plus(x) | Expr::Cast(x, _) => each_subexpr(x, f),
        Expr::IsNull { e, .. } => each_subexpr(e, f),
        Expr::InRel { e, .. } => each_subexpr(e, f),
        Expr::Arith(_, a, b) | Expr::Concat(a, b) | Expr::And(a, b) | Expr::Or(a, b) | Expr::Cmp(_, a, b, _) => {
            each_subexpr(a, f);
            each_subexpr(b, f)
        }
        Expr::Is { left, right, .. } => {
            each_subexpr(left, f);
            each_subexpr(right, f)
        }
        Expr::Like { text, pattern, .. } => {
            each_subexpr(text, f);
            each_subexpr(pattern, f)
        }
        Expr::Func(_, a) => a.iter().for_each(|x| each_subexpr(x, f)),
        Expr::Case { operand, whens, else_ } => {
            if let Some(o) = operand {
                each_subexpr(o, f)
            }
            for (w, t, _) in whens {
                each_subexpr(w, f);
                each_subexpr(t, f)
            }
            if let Some(x) = else_ {
                each_subexpr(x, f)
            }
        }
        Expr::InList { e, list, .. } => {
            each_subexpr(e, f);
            list.iter().for_each(|(x, _)| each_subexpr(x, f))
        }
    }
}

/// The expressions held directly by a relation's operators, through its whole tree (inputs,
/// bindings), relative to each operator's own row; not into subquery relations.
pub fn rel_exprs<S>(r: &Rel<S>, f: &mut dyn FnMut(&Expr<S>)) {
    rel_exprs_deep(r, f)
}

fn rel_exprs_deep<S>(r: &Rel<S>, f: &mut dyn FnMut(&Expr<S>)) {
    match r {
        Rel::Scan { .. } | Rel::Local { .. } | Rel::Unit => {}
        Rel::JsonEach { input, arg, on, .. } => {
            rel_exprs_deep(input, f);
            f(arg);
            if let Some(o) = on {
                f(o)
            }
        }
        Rel::Join { left, right, on, .. } => {
            rel_exprs_deep(left, f);
            rel_exprs_deep(right, f);
            if let Some(o) = on {
                f(o)
            }
        }
        Rel::Filter { input, pred } => {
            rel_exprs_deep(input, f);
            f(pred)
        }
        Rel::Project { input, exprs, .. } => {
            rel_exprs_deep(input, f);
            exprs.iter().for_each(|e| f(e))
        }
        Rel::Aggregate { input, keys, aggs } => {
            rel_exprs_deep(input, f);
            keys.iter().for_each(|e| f(e));
            for a in aggs {
                a.arg.iter().chain(&a.sep).for_each(|e| f(e))
            }
        }
        Rel::Window { input, funcs } => {
            rel_exprs_deep(input, f);
            for w in funcs {
                if let WindowKind::Min(e) | WindowKind::Max(e) = &w.kind {
                    f(e)
                }
                w.partition.iter().for_each(|e| f(e));
                w.order.iter().for_each(|(e, _)| f(e));
            }
        }
        Rel::Distinct { input, .. } => rel_exprs_deep(input, f),
        Rel::Union { left, right, .. } => {
            rel_exprs_deep(left, f);
            rel_exprs_deep(right, f)
        }
        Rel::Sort { input, limit, offset, .. } => {
            rel_exprs_deep(input, f);
            limit.iter().chain(offset).for_each(|e| f(e))
        }
        Rel::With { bindings, body } => {
            for (_, b) in bindings {
                match b {
                    Binding::Plain(r) => rel_exprs_deep(r, f),
                    Binding::Fix { init, step, .. } => init.iter().chain(step).for_each(|r| rel_exprs_deep(r, f)),
                }
            }
            rel_exprs_deep(body, f)
        }
    }
}

/// Splits a join's `ON` into equality keys and a residual (S3's delta joins, S2's index probes).
/// A conjunct `a = b` is a key when one operand reads only left columns (`< left_width`) and the
/// other only right columns, neither holds a subquery nor an outer reference; the right key is
/// rebased to the right relation's own columns. Every other conjunct, in order, is the residual
/// (over the joined row). Keys keep conjunct order. `l_keys[i]` pairs with `r_keys[i]`.
///
/// Exactness: `ON` = ∧ keys ∧ residual, conjunction is commutative under three-valued logic for
/// a filter's purpose (a row survives iff every conjunct is TRUE), and `x = y` is TRUE iff
/// neither is NULL and they compare equal under `affs`, which is what a hash probe on
/// affinity-converted values with NULLs excluded decides.
pub fn equi_keys<S: Clone>(on: Option<&Expr<S>>, left_width: usize) -> (Vec<Key<S>>, Vec<Key<S>>, Option<Expr<S>>) {
    let (mut lk, mut rk, mut rest) = (Vec::new(), Vec::new(), Vec::new());
    let lw = left_width as u16;
    let side = |e: &Expr<S>| -> Option<bool> {
        if has_subquery(e) || reads_outer(e, 1) {
            return None;
        }
        let (mut l, mut r) = (false, false);
        cols_read(e, &mut |c| if c < lw { l = true } else { r = true });
        match (l, r) {
            (true, false) => Some(true),
            (false, true) => Some(false),
            _ => None,
        }
    };
    for c in on.map(conjuncts).unwrap_or_default() {
        if let Expr::Cmp(CmpOp::Eq, a, b, (fa, fb)) = &c {
            let rebase = |e: &Expr<S>| map_cols(e.clone(), &mut |_, col| col - lw);
            match (side(a), side(b)) {
                (Some(true), Some(false)) => {
                    lk.push(Key { e: (**a).clone(), affs: (*fa, *fb) });
                    rk.push(Key { e: rebase(b), affs: (*fb, *fa) });
                    continue;
                }
                (Some(false), Some(true)) => {
                    lk.push(Key { e: (**b).clone(), affs: (*fb, *fa) });
                    rk.push(Key { e: rebase(a), affs: (*fa, *fb) });
                    continue;
                }
                _ => {}
            }
        }
        rest.push(c);
    }
    (lk, rk, conjoin(rest))
}

/// A filter conjunct recognised as a semijoin or antijoin of the filtered row against a
/// subquery body (S3's delta semijoins, S2's physical semi/anti join). The row is kept iff the
/// conjunct is TRUE:
///
/// - `EXISTS`: some body row matches the correlation keys (and `probe`, if any);
/// - `anti` (`NOT EXISTS`): no body row matches the keys;
/// - `e IN (SELECT f …)` (`probe`, not `anti`): some body row matches the keys with `f = e`;
/// - `e NOT IN (SELECT f …)` (`probe`, `anti`, null-aware): no body row matches the keys with
///   `f = e` or `f IS NULL` or `e IS NULL`. In particular an empty match set keeps the row even
///   when `e` is NULL (SQLite: `NULL NOT IN ()` is 1).
#[derive(Clone, Debug)]
pub struct SemiJoin<S> {
    pub anti: bool,
    /// The subquery without its correlation conjuncts: it reads no outer row.
    pub body: Rel<S>,
    /// Correlation keys: `outer[i]` over the filtered row (depth 0), `inner[i]` over the body's
    /// rows, from `inner = outer` conjuncts of the body's top `WHERE`.
    pub outer: Vec<Key<S>>,
    pub inner: Vec<Key<S>>,
    /// `IN`: the left operand (over the filtered row) and the subquery's first output column
    /// (over the body's rows), each with the `IN`'s comparison affinities.
    pub probe: Option<(Key<S>, Key<S>)>,
}

impl<S: Clone> SemiJoin<S> {
    /// The conjunct as a plain `[NOT] EXISTS` over the body, with the correlation re-attached
    /// (the recognition's inverse; the differential test compares both under the reference).
    pub fn to_expr(&self) -> Expr<S> {
        let up = |e: &Expr<S>| e.clone().map_depth0_to(1);
        let mut conds: Vec<Expr<S>> = self.outer.iter().zip(&self.inner).map(|(o, i)| Expr::Cmp(CmpOp::Eq, Box::new(i.e.clone()), Box::new(up(&o.e)), (i.affs.0, o.affs.0))).collect();
        if let Some((o, i)) = &self.probe {
            let eq = Expr::Cmp(CmpOp::Eq, Box::new(i.e.clone()), Box::new(up(&o.e)), (i.affs.0, o.affs.0));
            conds.push(if self.anti {
                let or = |a: Expr<S>, b: Expr<S>| Expr::Or(Box::new(a), Box::new(b));
                or(or(eq, Expr::IsNull { e: Box::new(i.e.clone()), negated: false }), Expr::IsNull { e: Box::new(up(&o.e)), negated: false })
            } else {
                eq
            });
        }
        let rel = match conjoin(conds) {
            None => self.body.clone(),
            Some(pred) => Rel::Filter { input: Box::new(self.body.clone()), pred },
        };
        Expr::Exists { rel: Box::new(rel), negated: self.anti }
    }
}

impl<S> Expr<S> {
    /// Moves depth-0 column references (outside subqueries) to `depth`.
    fn map_depth0_to(self, depth: u16) -> Expr<S> {
        fn go<S>(e: Expr<S>, depth: u16) -> Expr<S> {
            match e {
                Expr::Col { depth: 0, col } => Expr::Col { depth, col },
                e => e.map_children(&mut |x| go(x, depth)),
            }
        }
        go(self, depth)
    }

    /// Rebuilds the expression with `f` applied to each direct scalar child (subquery relations
    /// untouched).
    fn map_children(self, f: &mut dyn FnMut(Expr<S>) -> Expr<S>) -> Expr<S> {
        let b = |x: Box<Expr<S>>, f: &mut dyn FnMut(Expr<S>) -> Expr<S>| Box::new(f(*x));
        match self {
            e @ (Expr::Lit(_) | Expr::Col { .. } | Expr::Exists { .. } | Expr::Scalar(_)) => e,
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
            Expr::Func(fu, args) => Expr::Func(fu, args.into_iter().map(&mut *f).collect()),
            Expr::Case { operand, whens, else_ } => Expr::Case {
                operand: operand.map(|o| b(o, f)),
                whens: whens.into_iter().map(|(w, t, a)| (f(w), f(t), a)).collect(),
                else_: else_.map(|x| b(x, f)),
            },
            Expr::InList { e, list, negated } => Expr::InList { e: b(e, f), list: list.into_iter().map(|(x, a)| (f(x), a)).collect(), negated },
            Expr::InRel { e, rel, negated, aff } => Expr::InRel { e: b(e, f), rel, negated, aff },
        }
    }
}

/// Recognises a filter conjunct as a semijoin or antijoin ([`SemiJoin`]): `[NOT] EXISTS (q)`,
/// `NOT (EXISTS (q))`, `e [NOT] IN (q)`, `NOT (e IN (q))`. `None`: not such a conjunct. `Err`
/// names a correlation shape that is not decorrelated: the subquery's correlation must be
/// equalities `inner = outer` in its top `WHERE` (under any `Project`/`Distinct`/unlimited
/// `Sort` on top, which existence ignores), with the outer side reading only the enclosing row.
pub fn semi_join<S: Clone>(c: &Expr<S>) -> Option<Result<SemiJoin<S>, String>> {
    let (c, neg) = match c {
        Expr::Not(x) if matches!(**x, Expr::Exists { .. } | Expr::InRel { .. }) => (&**x, true),
        x => (x, false),
    };
    match c {
        Expr::Exists { rel, negated } => Some(decorrelate(rel, false).map(|(body, outer, inner, _)| SemiJoin { anti: *negated != neg, body, outer, inner, probe: None })),
        Expr::InRel { e, rel, negated, aff } => {
            if has_subquery(e) {
                return Some(Err("subquery as the left operand of IN".into()));
            }
            Some(decorrelate(rel, true).map(|(body, outer, inner, first)| SemiJoin {
                anti: *negated != neg,
                body,
                outer,
                inner,
                probe: Some((Key { e: (**e).clone(), affs: *aff }, Key { e: first.expect("IN probe"), affs: (aff.1, aff.0) })),
            }))
        }
        _ => None,
    }
}

type Decorrelated<S> = (Rel<S>, Vec<Key<S>>, Vec<Key<S>>, Option<Expr<S>>);

fn decorrelate<S: Clone>(rel: &Rel<S>, want_first: bool) -> Result<Decorrelated<S>, String> {
    // The first output column, as an expression over the current relation's rows.
    let mut first: Option<Expr<S>> = want_first.then(|| Expr::Col { depth: 0, col: 0 });
    let mut cur = rel;
    loop {
        match cur {
            Rel::Project { input, exprs, .. } => {
                if exprs.iter().any(|e| reads_outer(e, 1)) {
                    return Err("correlated subquery projecting an outer column".into());
                }
                if let Some(f) = first.take() {
                    if exprs.is_empty() {
                        return Err("IN over a subquery with no column".into());
                    }
                    first = Some(f.substitute_cols(exprs));
                }
                cur = input;
            }
            // Existence, and the set of first-column values (prefix ≥ 1), ignore duplicates.
            Rel::Distinct { input, prefix } if !want_first || *prefix >= 1 => cur = input,
            Rel::Sort { input, limit: None, offset: None, .. } => cur = input,
            _ => break,
        }
    }
    if !rel_reads_outer(cur, 1) {
        return Ok((cur.clone(), Vec::new(), Vec::new(), first));
    }
    let Rel::Filter { input, pred } = cur else {
        return Err("correlated subquery whose correlation is not its top-level WHERE".into());
    };
    if rel_reads_outer(input, 1) {
        return Err("correlation below the subquery's WHERE (in a join or nested relation)".into());
    }
    let (mut outer, mut inner, mut keep) = (Vec::new(), Vec::new(), Vec::new());
    for c in conjuncts(pred) {
        if !reads_outer(&c, 1) {
            keep.push(c);
            continue;
        }
        if let Expr::Cmp(CmpOp::Eq, a, b, (fa, fb)) = &c {
            let outer_only = |x: &Expr<S>| {
                let mut own = false;
                cols_read(x, &mut |_| own = true);
                !own && !has_subquery(x) && !reads_outer(x, 2)
            };
            let inner_only = |x: &Expr<S>| !reads_outer(x, 1);
            let down = |x: &Expr<S>| x.clone().outer_to_depth0();
            if outer_only(a) && inner_only(b) {
                outer.push(Key { e: down(a), affs: (*fa, *fb) });
                inner.push(Key { e: (**b).clone(), affs: (*fb, *fa) });
                continue;
            }
            if outer_only(b) && inner_only(a) {
                outer.push(Key { e: down(b), affs: (*fb, *fa) });
                inner.push(Key { e: (**a).clone(), affs: (*fa, *fb) });
                continue;
            }
        }
        return Err("correlated subquery with a non-equality correlation".into());
    }
    let body = match conjoin(keep) {
        None => (**input).clone(),
        Some(p) => Rel::Filter { input: input.clone(), pred: p },
    };
    Ok((body, outer, inner, first))
}

impl<S> Expr<S> {
    /// Depth-1 column references (outside subqueries) become depth 0.
    fn outer_to_depth0(self) -> Expr<S> {
        match self {
            Expr::Col { depth: 1, col } => Expr::Col { depth: 0, col },
            e => e.map_children(&mut |x| x.outer_to_depth0()),
        }
    }

    /// Replaces depth-0 column `c` (outside subqueries) by `exprs[c]`.
    fn substitute_cols(self, exprs: &[Expr<S>]) -> Expr<S>
    where
        S: Clone,
    {
        match self {
            Expr::Col { depth: 0, col } => exprs[col as usize].clone(),
            e => e.map_children(&mut |x| x.substitute_cols(exprs)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::*;

    fn cmp(op: CmpOp, a: Expr<u32>, b: Expr<u32>) -> Expr<u32> {
        Expr::Cmp(op, Box::new(a), Box::new(b), (Aff::None, Aff::None))
    }
    fn and(a: Expr<u32>, b: Expr<u32>) -> Expr<u32> {
        Expr::And(Box::new(a), Box::new(b))
    }

    #[test]
    fn equi_keys_split_and_rebase() {
        // a(3) ⋈ b(3) ON a.0 = b.1 AND a.1 < b.0 AND 2 = b.2 AND b.0 = a.2 AND a.0 = a.1
        let on = and(and(and(and(cmp(CmpOp::Eq, col(0), col(4)), cmp(CmpOp::Lt, col(1), col(3))), cmp(CmpOp::Eq, int(2), col(5))), cmp(CmpOp::Eq, col(3), col(2))), cmp(CmpOp::Eq, col(0), col(1)));
        let (l, r, rest) = equi_keys(Some(&on), 3);
        assert_eq!(l.len(), 2);
        assert!(matches!(l[0].e, Expr::Col { depth: 0, col: 0 }) && matches!(r[0].e, Expr::Col { depth: 0, col: 1 }));
        assert!(matches!(l[1].e, Expr::Col { depth: 0, col: 2 }) && matches!(r[1].e, Expr::Col { depth: 0, col: 0 }));
        assert_eq!(conjuncts(rest.as_ref().unwrap()).len(), 3);
        // Differential: keys ∧ residual over the cross product = the join.
        for seed in 0..200 {
            let mut g = Rng::new(seed);
            let tables = vec![g.rows(3, 8), g.rows(3, 8)];
            let join = Rel::Join { kind: JoinKind::Inner, left: Box::new(scan(0, 3)), right: Box::new(scan(1, 3)), on: Some(on.clone()) };
            let keys = l.iter().zip(&r).map(|(a, b)| Expr::Cmp(CmpOp::Eq, Box::new(a.e.clone()), Box::new(map_cols(b.e.clone(), &mut |_, c| c + 3)), (a.affs.0, b.affs.0)));
            let pred = conjoin(keys.chain(rest.clone()).collect()).unwrap();
            let split = Rel::Filter { input: Box::new(Rel::Join { kind: JoinKind::Inner, left: Box::new(scan(0, 3)), right: Box::new(scan(1, 3)), on: None }), pred };
            assert!(same(&eval(&join, &tables).unwrap(), &eval(&split, &tables).unwrap()), "seed {seed}");
        }
    }

    /// `SELECT … FROM a WHERE c` for each recognised conjunct `c` against its inverse.
    #[test]
    fn semi_join_recognition_is_exact() {
        let corr = |extra: Option<Expr<u32>>| {
            let p = cmp(CmpOp::Eq, col(0), outer(0));
            let pred = match extra {
                Some(e) => and(p, e),
                None => p,
            };
            Rel::Filter { input: Box::new(scan(1, 3)), pred }
        };
        let first = |r: Rel<u32>| Rel::Project { input: Box::new(r), exprs: vec![col(1)], schema: schema(1) };
        let cases: Vec<Expr<u32>> = vec![
            Expr::Exists { rel: Box::new(corr(Some(cmp(CmpOp::Gt, col(2), int(1))))), negated: false },
            Expr::Exists { rel: Box::new(corr(None)), negated: true },
            Expr::Not(Box::new(Expr::Exists { rel: Box::new(corr(None)), negated: false })),
            Expr::InRel { e: Box::new(col(1)), rel: Box::new(first(corr(None))), negated: false, aff: (Aff::None, Aff::None) },
            Expr::InRel { e: Box::new(col(1)), rel: Box::new(first(corr(None))), negated: true, aff: (Aff::None, Aff::None) },
            Expr::Not(Box::new(Expr::InRel { e: Box::new(col(2)), rel: Box::new(Rel::Distinct { input: Box::new(first(corr(None))), prefix: 1 }), negated: false, aff: (Aff::None, Aff::None) })),
            Expr::InRel { e: Box::new(col(1)), rel: Box::new(first(scan(1, 3))), negated: true, aff: (Aff::None, Aff::None) },
            Expr::InRel { e: Box::new(col(0)), rel: Box::new(first(Rel::Filter { input: Box::new(scan(1, 3)), pred: cmp(CmpOp::Eq, outer(2), col(2)) })), negated: true, aff: (Aff::Integer, Aff::Text) },
        ];
        for (k, c) in cases.iter().enumerate() {
            let sj = semi_join(c).expect("recognised").expect("decorrelated");
            if k < 6 {
                assert_eq!(sj.outer.len(), 1, "case {k}");
            }
            for seed in 0..300 {
                let mut g = Rng::new(seed * 31 + k as u64);
                let tables = vec![g.rows(3, 6), g.rows(3, 6)];
                let a = Rel::Filter { input: Box::new(scan(0, 3)), pred: c.clone() };
                let b = Rel::Filter { input: Box::new(scan(0, 3)), pred: sj.to_expr() };
                assert!(same(&eval(&a, &tables).unwrap(), &eval(&b, &tables).unwrap()), "case {k} seed {seed}");
            }
        }
        // Not decorrelated: a correlation that is not an equality.
        let lt = Expr::Exists { rel: Box::new(Rel::Filter { input: Box::new(scan(1, 3)), pred: cmp(CmpOp::Lt, col(0), outer(0)) }), negated: false };
        assert!(semi_join(&lt).unwrap().is_err());
        assert!(semi_join(&cmp(CmpOp::Eq, col(0), int(1))).is_none());
    }
}

/// Calls `f` on every relation directly below `r`: its inputs, bindings and body, and the
/// relations of the subqueries in its own expressions (not deeper; `f` recurses if it wants).
pub fn children_mut<S>(r: &mut Rel<S>, f: &mut dyn FnMut(&mut Rel<S>)) {
    fn subs<S>(e: &mut Expr<S>, f: &mut dyn FnMut(&mut Rel<S>)) {
        match e {
            Expr::Lit(_) | Expr::Col { .. } => {}
            Expr::Exists { rel, .. } | Expr::Scalar(rel) => f(rel),
            Expr::InRel { e, rel, .. } => {
                subs(e, f);
                f(rel)
            }
            Expr::Not(x) | Expr::Neg(x) | Expr::Plus(x) | Expr::Cast(x, _) => subs(x, f),
            Expr::IsNull { e, .. } => subs(e, f),
            Expr::Arith(_, a, b) | Expr::Concat(a, b) | Expr::And(a, b) | Expr::Or(a, b) | Expr::Cmp(_, a, b, _) => {
                subs(a, f);
                subs(b, f)
            }
            Expr::Is { left, right, .. } => {
                subs(left, f);
                subs(right, f)
            }
            Expr::Like { text, pattern, .. } => {
                subs(text, f);
                subs(pattern, f)
            }
            Expr::Func(_, a) => a.iter_mut().for_each(|x| subs(x, f)),
            Expr::Case { operand, whens, else_ } => {
                if let Some(o) = operand {
                    subs(o, f)
                }
                for (w, t, _) in whens {
                    subs(w, f);
                    subs(t, f)
                }
                if let Some(x) = else_ {
                    subs(x, f)
                }
            }
            Expr::InList { e, list, .. } => {
                subs(e, f);
                list.iter_mut().for_each(|(x, _)| subs(x, f))
            }
        }
    }
    match r {
        Rel::Scan { .. } | Rel::Local { .. } | Rel::Unit => {}
        Rel::JsonEach { input, arg, on, .. } => {
            f(input);
            subs(arg, f);
            if let Some(o) = on {
                subs(o, f)
            }
        }
        Rel::Join { left, right, on, .. } => {
            f(left);
            f(right);
            if let Some(o) = on {
                subs(o, f)
            }
        }
        Rel::Filter { input, pred } => {
            f(input);
            subs(pred, f)
        }
        Rel::Project { input, exprs, .. } => {
            f(input);
            exprs.iter_mut().for_each(|e| subs(e, f))
        }
        Rel::Aggregate { input, keys, aggs } => {
            f(input);
            keys.iter_mut().for_each(|e| subs(e, f));
            for a in aggs {
                a.arg.iter_mut().chain(a.sep.iter_mut()).for_each(|e| subs(e, f))
            }
        }
        Rel::Window { input, funcs } => {
            f(input);
            for w in funcs {
                if let WindowKind::Min(e) | WindowKind::Max(e) = &mut w.kind {
                    subs(e, f)
                }
                w.partition.iter_mut().for_each(|e| subs(e, f));
                w.order.iter_mut().for_each(|(e, _)| subs(e, f));
            }
        }
        Rel::Distinct { input, .. } => f(input),
        Rel::Union { left, right, .. } => {
            f(left);
            f(right)
        }
        Rel::Sort { input, limit, offset, .. } => {
            f(input);
            limit.iter_mut().chain(offset.iter_mut()).for_each(|e| subs(e, f))
        }
        Rel::With { bindings, body } => {
            for (_, b) in bindings {
                match b {
                    Binding::Plain(r) => f(r),
                    Binding::Fix { init, step, .. } => init.iter_mut().chain(step.iter_mut()).for_each(|r| f(r)),
                }
            }
            f(body)
        }
    }
}
