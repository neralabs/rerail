//! A component's spec as values. The planner hands these to the engine in process (`open_spec`, `reprobe_spec`),
//! so a plan is not written as text and parsed again. The text (the module's spec lines, joins hex-encoded within)
//! is kept for the SQL functions (`set_fixpoint_open(spec)`: T's extension, a copy of the engine with its own
//! components, over text symbols), the lab's capture files and the spikes' spec files: `to_text` writes what the
//! planner wrote, and `parse` reads it back.

use rerail_blocks::vector::Vector;
use rerail_section::{Archive, Bound, Handle, Section, Tuple2};
use std::fmt::Write as _;
use std::sync::Arc;

/// A value position: `_`, `v<slot>`, `i:<int>` or `x:<hex symbol>`.
#[derive(Archive)]
#[archive(derive(Debug, PartialEq, Eq, Hash), decode)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Tok {
    Wild,
    Slot(usize),
    Int(i64),
    Sym(String),
}

/// An atom's column: a value position, or an aggregate's own column (`a<name>`, the same name tests equality).
#[derive(Archive)]
#[archive(derive(Debug, PartialEq, Eq, Hash), decode)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Arg {
    Tok(Tok),
    Local(String),
}

/// A head term: a value, or two slots' symbols joined by "." (`cat:v<a>:v<b>`).
#[derive(Archive)]
#[archive(derive(Debug, PartialEq, Eq, Hash), decode)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum HeadTok {
    Tok(Tok),
    Cat(usize, usize),
}

/// An input read for joins: `t:<b|d>:<hex select>:<hex columns>:<types>`. `base`: a base relation, kept across
/// runs that do not change it.
#[derive(Archive)]
#[archive(derive(Debug, PartialEq, Eq, Hash), decode)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InputKey {
    pub base: bool,
    pub sql: String,
    pub cols: String,
    pub types: String,
}

/// Where an atom's rows are: the component's relation `i`'s delta (`d<i>`) or all of it (`c<i>`), a lower stratum's
/// native relation (`r:<hex table>`) or an input.
#[derive(Archive)]
#[archive(derive(Debug, PartialEq, Eq, Hash), decode)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Delta(usize),
    Full(usize),
    Native(String),
    Input(InputKey),
    /// A lower stratum's native relation in one of its versions (decision 0364): `v:<o|n|m|p>:<hex table>`.
    Version(String, Ver),
}

/// A relation's version a join reads (decision 0364): its rows at the run's start (`Old`), now (`New`), those of
/// the start gone since (`Minus`) and those added since (`Plus`).
#[derive(Archive)]
#[archive(derive(Debug, PartialEq, Eq, Hash), decode)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ver {
    Old,
    New,
    Minus,
    Plus,
}

#[derive(Archive)]
#[archive(derive(Debug, PartialEq, Eq, Hash), decode)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Extreme {
    Sum,
    Min,
    Max,
}

/// One step of a native join, in plan order, or its head (last).
#[derive(Archive)]
#[archive(derive(Debug, PartialEq, Eq, Hash), decode)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum JoinItem {
    /// `pos|neg <src> <args…>`
    Atom { neg: bool, src: Source, args: Vec<Arg> },
    /// `count <src> <args…> <out>`
    Count { src: Source, args: Vec<Arg>, out: Tok },
    /// `sum|min|max <src> <args…> <value column> <out>`; bounded (decision 0511), `maxlt|maxle|mingt|minge <src>
    /// <args…> <value column> <bound> <out>`: the extreme of the values on the bound's side, strictly or not.
    Extreme { op: Extreme, src: Source, args: Vec<Arg>, col: usize, out: Tok, bound: Option<(Tok, bool)> },
    /// `strjoin <src> <args…> <x column>:<0|1> <by columns|-> <separator> <out>` (a column's `1`: a symbol)
    StrJoin { src: Source, args: Vec<Arg>, x: (usize, bool), by: Vec<(usize, bool)>, sep: Tok, out: Tok },
    /// `cat <out> <a> <b>`
    Cat { out: Tok, a: Tok, b: Tok },
    /// `lt|le <a> <b>`
    Lt { a: Tok, b: Tok, or_eq: bool },
    /// `func <name> <out> <args…>`
    Func { name: String, out: Tok, args: Vec<Tok> },
    /// `test <op> <s> <pattern> <0|1>`
    Test { op: String, s: Tok, p: Tok, want: bool },
    /// `docof <out> <n>`
    DocOf { out: Tok, n: Tok },
    /// `testset <op> <s> <0|1> <members…>`
    TestSet { op: String, s: Tok, want: bool, members: Vec<Tok> },
    /// `in <a> <0|1> <members…>`
    In { a: Tok, want: bool, set: Vec<Tok> },
    /// `deq|dne <a> <b>`
    Doc { a: Tok, b: Tok, eq: bool },
    /// `eq|ne <a> <b>`
    Eq { a: Tok, b: Tok, ne: bool },
    /// `near|down <n> <a> <hex kinds>`: the engine has no step for these (it refuses the spec).
    Near { down: bool, n: Tok, a: Tok, kinds: String },
    /// `head <terms…>`
    Head(Vec<HeadTok>),
}

/// A native join: its steps, then its head.
#[derive(Archive)]
#[archive(derive(Debug, PartialEq, Eq, Hash), decode)]
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct JoinSpec {
    pub items: Vec<JoinItem>,
    /// Per item before the head, the planner's step it was made for (`plan_steps` for the head's computed terms),
    /// and its steps: a literal can make several items (`$k = @sub(@strlen($d), 1)` makes two), so the engine's
    /// steps are not the plan's. The engine reports counts, CPU and histograms by plan step through it
    /// (`JoinRule::from`). Empty (a spec read from text): each item is its own step.
    pub from: Vector<u32>,
    pub plan_steps: u32,
}

/// The rows the planner expects after each plan step of a line's join, every row walked (`StepPlan::all`, decision
/// 0277), where it priced the rule: what the device sizes a stage's table by before the stage has run. Not the rule:
/// it lies on the line that runs the join (`Line::Seed`, `Line::Join`), not in the join's section, so the join's
/// identity (`level::digest`), and what is kept by it (the stages' growth seen, compiled forms), stay a chunk's to the
/// next while its estimates change, and a join kept for the run (`join_spec`) is priced again without being made
/// again. Every two are equal and they hash as nothing.
#[derive(Clone, Debug, Default, Archive)]
#[archive(derive(Debug))]
pub struct Est(pub Vector<f64>);

impl PartialEq for Est {
    fn eq(&self, _: &Est) -> bool {
        true
    }
}

impl Eq for Est {}

impl std::hash::Hash for Est {
    fn hash<H: std::hash::Hasher>(&self, _: &mut H) {}
}

impl PartialEq for ArchivedEst {
    fn eq(&self, _: &ArchivedEst) -> bool {
        true
    }
}

impl Eq for ArchivedEst {}

impl std::hash::Hash for ArchivedEst {
    fn hash<H: std::hash::Hasher>(&self, _: &mut H) {}
}

/// A join's section (decision 0569 §4): encoded once, where the planner made it (`JoinSpec::encode`), and named by
/// the component's spec, which holds it.
pub type Join = Bound<Section<JoinSpec>>;

impl JoinSpec {
    /// Its section.
    pub fn encode(&self) -> Join {
        Bound::new(Section::of(self))
    }
}

/// A component's spec's section: its lines, and the joins they name.
pub type Spec = Section<ComponentSpec>;

impl ComponentSpec {
    /// Its section.
    pub fn encode(&self) -> Spec {
        Section::of(self)
    }
}

/// How a relation starts: its table's rows, none, a query's rows, or a kept native relation less another's
/// (`base`), plus its table's rows when `seeded`. A `base` with no relation gone (`-`) is a copy of the kept
/// relation, its levels with it (decision 0090's `h__live`).
#[derive(Archive)]
#[archive(derive(Debug, PartialEq, Eq, Hash))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Start {
    Rows,
    Empty,
    From(String),
    /// `gone` "-": no rows gone (all of `old`).
    Base { old: String, gone: String, seeded: bool },
}

/// An edge rule (`rule …`): see the module's spec.
#[derive(Archive)]
#[archive(derive(Debug, PartialEq, Eq, Hash))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EdgeSpec {
    pub head: usize,
    pub src: usize,
    /// The edge table and its column types.
    pub edge: Option<(String, String)>,
    pub key: Vector<usize>,
    pub ecols: Vector<usize>,
    pub pat: Vec<Tok>,
    pub out: Vec<HeadTok>,
}

/// One spec line, after `name`.
#[derive(Archive)]
#[archive(derive(Debug, PartialEq, Eq, Hash))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Line {
    Rel { name: String, types: String, start: Start },
    NoWrite,
    /// The placement put the stratum on the device (decision 0095 §7).
    Device,
    /// Its recursive relations keep each row's level (decision 0090): a session maintains them by levels. Without it
    /// no row has one, and a closure may run on the device (bit rows give reach, not distance). The relations listed
    /// reach themselves through the stratum's rules as written, so they keep levels even where this run's rules over
    /// empty relations were left out and their cycle is not run (`leveled <rels|->`).
    Leveled(Vector<usize>),
    /// A recursive component the static check proved (decision 0364 §5): no levels, its candidates rechecked once
    /// each in the measure's order. Each relation's measure column (`cols`) and rank (`ranks`: a row's derivations
    /// at its own node read relations of lower rank); a row's key is its measure's value, or the one row `key`'s
    /// SQL reads with it bound (`?1`: the node's depth or document order); descending where `desc`. Not `keyed`:
    /// the order is not the measure's (a node's key the run cannot read), and a candidate that stays is checked again
    /// when a row it may read goes (`measured a|d <key|-> k|u <cols> <ranks>`).
    Measured { desc: bool, key: Option<String>, keyed: bool, cols: Vector<usize>, ranks: Vector<usize> },
    /// Its relations started from kept rows (`Start::Base`) hold them as no wave's delta: its first wave is driven by
    /// its inputs' changes (seeds that read them, `plan::reads_change`), not by its kept rows (decision 0122 §2).
    KeptOld,
    /// The relations whose rows the run keeps past it (a chunk's exports): made in blocks of their own
    /// (`Life::Kept`), apart from the run's other relations (`keptrows <rels>`).
    KeptRows(Vector<usize>),
    /// Relation `rel` is a lattice's (decision 0124): its last column the value, joined by the lattice `name`'s join,
    /// the `keys` columns before it the key; after `widen` changes of a key, widened.
    Lattice { rel: usize, keys: usize, name: String, widen: Option<u32> },
    /// Relation `rel`'s layout says columns `cols` are a key (decision 0361 step 6): its rows are checked as they are
    /// written, and a second row of a key is reported (`key_breaks`).
    Key { rel: usize, cols: Vector<usize> },
    Request(usize),
    Produced(usize),
    /// The relation's statistics are kept as its rows come (`Rel::stats`): a session keeps it for its next run.
    Stats(usize),
    /// The relations declared sorted (`.decl … sorted`, decision 0456 K7): laid out in canonical order when done
    /// (`sorted <rels>`).
    Sorted(Vector<usize>),
    Seed { head: usize, join: Join, est: Est },
    Join { head: usize, delta: usize, join: Join, whole: Option<Join>, est: Est },
    Probe { head: usize, delta: usize, sql: String },
    /// A probe not planned yet (decision 0123 §9): its delta has had no rows. The run returns for its plan when its
    /// delta first has rows (`done 4`), and the caller gives it its join (`reprobe`).
    Unplanned { head: usize, delta: usize },
    Gate { done: usize, src: Option<usize>, quiet: Vector<usize>, after: Vector<usize> },
    Edge(EdgeSpec),
    /// The maintain mode (decision 0364 stage 3): a removal join, its head rows the candidates of relation `head`,
    /// read from its versions (`Source::Version`): run once, before the seeds (`remove <head> <hex join>`).
    Remove { head: usize, join: Join },
    /// An add join whose rows are checked before they are added (one driven by a negated atom's rows gone, which
    /// may be back): `addchecked <head> <hex join>`.
    AddChecked { head: usize, join: Join },
    /// A rule of `head` over the relations now, run per candidate with its head's slots bound: a candidate with no
    /// row from any of them is killed in place (`recheck <head> <hex join>`).
    Recheck { head: usize, join: Join },
}

/// A component: its name, then its lines in order (relations are numbered in order, probes are numbered in order).
#[derive(Archive)]
#[archive(derive(Debug, PartialEq, Eq, Hash))]
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ComponentSpec {
    pub name: String,
    pub lines: Vec<Line>,
}

/// A probe's new body (`reprobe`): SQL, a join, or the join it runs in the wave whose delta is the whole relation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Reprobe {
    Sql(String),
    Join(Join),
    Whole(Join),
}

/// Lowercase hex of a string's bytes.
pub fn hex(s: &str) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(2 * s.len());
    for b in s.bytes() {
        out.push(DIGITS[usize::from(b >> 4)] as char);
        out.push(DIGITS[usize::from(b & 15)] as char);
    }
    out
}

pub fn unhex(s: &str) -> Result<String, String> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return Err(format!("bad hex {s}"));
    }
    let b: Result<Vec<u8>, String> = (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string())).collect();
    String::from_utf8(b?).map_err(|e| e.to_string())
}

fn num<T: std::str::FromStr>(t: &str) -> Result<T, String> {
    t.parse().map_err(|_| format!("bad number {t}"))
}

fn slot(t: &str) -> Result<usize, String> {
    t.strip_prefix('v').and_then(|s| s.parse().ok()).ok_or_else(|| format!("bad slot {t}"))
}

fn tok(t: &str) -> Result<Tok, String> {
    Ok(if t == "_" {
        Tok::Wild
    } else if let Some(i) = t.strip_prefix("i:") {
        Tok::Int(num(i)?)
    } else if let Some(x) = t.strip_prefix("x:") {
        Tok::Sym(unhex(x)?)
    } else {
        Tok::Slot(slot(t)?)
    })
}

fn head_tok(t: &str) -> Result<HeadTok, String> {
    Ok(match t.strip_prefix("cat:") {
        Some(c) => {
            let (a, b) = c.split_once(':').ok_or("bad cat")?;
            HeadTok::Cat(slot(a)?, slot(b)?)
        }
        None => HeadTok::Tok(tok(t)?),
    })
}

fn arg(t: &str, aggregate: bool) -> Result<Arg, String> {
    match t.strip_prefix('a') {
        Some(local) if aggregate => Ok(Arg::Local(local.to_owned())),
        _ => Ok(Arg::Tok(tok(t)?)),
    }
}

fn source(t: &str) -> Result<Source, String> {
    if let Some(rest) = t.strip_prefix("r:") {
        return Ok(Source::Native(unhex(rest)?));
    }
    if let Some(rest) = t.strip_prefix("v:") {
        let (v, table) = rest.split_once(':').ok_or_else(|| format!("bad version source {t}"))?;
        let ver = match v {
            "o" => Ver::Old,
            "n" => Ver::New,
            "m" => Ver::Minus,
            "p" => Ver::Plus,
            _ => return Err(format!("bad version {v}")),
        };
        return Ok(Source::Version(unhex(table)?, ver));
    }
    if let Some(rest) = t.strip_prefix("t:") {
        return Ok(Source::Input(InputKey::parse(rest)?));
    }
    if let Some(i) = t.strip_prefix('d') {
        return Ok(Source::Delta(num(i)?));
    }
    if let Some(i) = t.strip_prefix('c') {
        return Ok(Source::Full(num(i)?));
    }
    Err(format!("bad source {t}"))
}

fn list(t: &str) -> Result<Vector<usize>, String> {
    if t == "-" { Ok(rerail_blocks::vector::Vector::new()) } else { t.split(',').map(num::<usize>).collect() }
}

impl std::fmt::Display for ArchivedTok {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchivedTok::Wild => f.write_str("_"),
            ArchivedTok::Slot(s) => write!(f, "v{s}"),
            ArchivedTok::Int(i) => write!(f, "i:{i}"),
            ArchivedTok::Sym(s) => write!(f, "x:{}", hex(s)),
        }
    }
}

impl std::fmt::Display for ArchivedArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchivedArg::Tok(t) => t.fmt(f),
            ArchivedArg::Local(l) => write!(f, "a{l}"),
        }
    }
}

impl std::fmt::Display for ArchivedHeadTok {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchivedHeadTok::Tok(t) => t.fmt(f),
            ArchivedHeadTok::Cat(a, b) => write!(f, "cat:v{a}:v{b}"),
        }
    }
}

impl InputKey {
    /// `<b|d|x>:<hex select>:<hex columns>:<types>` (after `t:`).
    pub fn parse(t: &str) -> Result<InputKey, String> {
        let parts: Vec<&str> = t.splitn(4, ':').collect();
        let [which, sql, cols, types] = parts[..] else { return Err(format!("bad input {t}")) };
        Ok(InputKey { base: which == "b", sql: unhex(sql)?, cols: unhex(cols)?, types: types.to_owned() })
    }
}

impl ArchivedInputKey {
    /// The key as a value, to find its loaded input by.
    pub fn owned(&self) -> InputKey {
        InputKey { base: self.base, sql: self.sql.to_string(), cols: self.cols.to_string(), types: self.types.to_string() }
    }
}

impl std::fmt::Display for ArchivedInputKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}:{}", if self.base { "b" } else { "d" }, hex(&self.sql), hex(&self.cols), self.types)
    }
}

impl std::fmt::Display for ArchivedSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchivedSource::Delta(i) => write!(f, "d{i}"),
            ArchivedSource::Full(i) => write!(f, "c{i}"),
            ArchivedSource::Native(t) => write!(f, "r:{}", hex(t)),
            ArchivedSource::Input(k) => write!(f, "t:{k}"),
            ArchivedSource::Version(t, v) => write!(f, "v:{}:{}", match v { ArchivedVer::Old => "o", ArchivedVer::New => "n", ArchivedVer::Minus => "m", ArchivedVer::Plus => "p" }, hex(t)),
        }
    }
}

fn joined<T: std::fmt::Display>(xs: &[T]) -> String {
    let mut s = String::new();
    for (i, x) in xs.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let _ = write!(s, "{x}");
    }
    s
}

fn comma(xs: &[u64]) -> String {
    if xs.is_empty() { "-".to_owned() } else { xs.iter().map(u64::to_string).collect::<Vec<_>>().join(",") }
}

impl std::fmt::Display for ArchivedJoinItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use ArchivedExtreme as E;
        let col = |c: &Tuple2<u64, bool>| format!("{}:{}", c.0, u8::from(c.1));
        match self {
            ArchivedJoinItem::Atom { neg, src, args } => write!(f, "{} {src} {}", if *neg { "neg" } else { "pos" }, joined(args)),
            ArchivedJoinItem::Count { src, args, out } => write!(f, "count {src} {} {out}", joined(args)),
            ArchivedJoinItem::Extreme { op, src, args, col, out, bound } => {
                let name = match (op, bound.as_ref()) {
                    (E::Sum, _) => "sum",
                    (E::Min, None) => "min",
                    (E::Max, None) => "max",
                    (E::Min, Some(Tuple2(_, true))) => "mingt",
                    (E::Min, Some(Tuple2(_, false))) => "minge",
                    (E::Max, Some(Tuple2(_, true))) => "maxlt",
                    (E::Max, Some(Tuple2(_, false))) => "maxle",
                };
                match bound.as_ref() {
                    Some(Tuple2(b, _)) => write!(f, "{name} {src} {} {col} {b} {out}", joined(args)),
                    None => write!(f, "{name} {src} {} {col} {out}", joined(args)),
                }
            }
            ArchivedJoinItem::StrJoin { src, args, x, by, sep, out } => {
                let by = if by.is_empty() { "-".to_owned() } else { by.iter().map(col).collect::<Vec<_>>().join(",") };
                write!(f, "strjoin {src} {} {} {by} {sep} {out}", joined(args), col(x))
            }
            ArchivedJoinItem::Cat { out, a, b } => write!(f, "cat {out} {a} {b}"),
            ArchivedJoinItem::Lt { a, b, or_eq } => write!(f, "{} {a} {b}", if *or_eq { "le" } else { "lt" }),
            ArchivedJoinItem::Func { name, out, args } => write!(f, "func {name} {out} {}", joined(args)),
            ArchivedJoinItem::Test { op, s, p, want } => write!(f, "test {op} {s} {p} {}", u8::from(*want)),
            ArchivedJoinItem::DocOf { out, n } => write!(f, "docof {out} {n}"),
            ArchivedJoinItem::TestSet { op, s, want, members } => write!(f, "testset {op} {s} {} {}", u8::from(*want), joined(members)),
            ArchivedJoinItem::In { a, want, set } => write!(f, "in {a} {} {}", u8::from(*want), joined(set)),
            ArchivedJoinItem::Doc { a, b, eq } => write!(f, "{} {a} {b}", if *eq { "deq" } else { "dne" }),
            ArchivedJoinItem::Eq { a, b, ne } => write!(f, "{} {a} {b}", if *ne { "ne" } else { "eq" }),
            ArchivedJoinItem::Near { down, n, a, kinds } => write!(f, "{} {n} {a} {}", if *down { "down" } else { "near" }, hex(kinds)),
            ArchivedJoinItem::Head(terms) => write!(f, "head {}", joined(terms)),
        }
    }
}

impl ArchivedJoinSpec {
    /// Its text: one line per item.
    pub fn to_text(&self) -> String {
        self.items.iter().map(ArchivedJoinItem::to_string).collect::<Vec<_>>().join("\n")
    }
}

impl JoinSpec {
    /// Its text (`ArchivedJoinSpec::to_text`).
    pub fn to_text(&self) -> String {
        self.encode().to_text()
    }

    pub fn parse(text: &str) -> Result<JoinSpec, String> {
        let mut items = Vec::new();
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let t: Vec<&str> = line.split_whitespace().collect();
            let n = t.len();
            let at = |i: usize| t.get(i).copied().ok_or_else(|| format!("short join item: {line}"));
            let toks = |xs: &[&str]| xs.iter().map(|x| tok(x)).collect::<Result<Vec<_>, _>>();
            let args = |xs: &[&str], aggregate: bool| xs.iter().map(|x| arg(x, aggregate)).collect::<Result<Vec<_>, _>>();
            let want = |x: &str| x == "1";
            items.push(match t[0] {
                "pos" | "neg" => JoinItem::Atom { neg: t[0] == "neg", src: source(at(1)?)?, args: args(&t[2..], false)? },
                "count" if n >= 3 => JoinItem::Count { src: source(t[1])?, args: args(&t[2..n - 1], true)?, out: tok(t[n - 1])? },
                "sum" | "min" | "max" if n >= 4 => JoinItem::Extreme {
                    op: match t[0] {
                        "sum" => Extreme::Sum,
                        "min" => Extreme::Min,
                        _ => Extreme::Max,
                    },
                    src: source(t[1])?,
                    args: args(&t[2..n - 2], true)?,
                    col: num(t[n - 2])?,
                    out: tok(t[n - 1])?,
                    bound: None,
                },
                "maxlt" | "maxle" | "mingt" | "minge" if n >= 5 => JoinItem::Extreme {
                    op: if t[0].starts_with("max") { Extreme::Max } else { Extreme::Min },
                    src: source(t[1])?,
                    args: args(&t[2..n - 3], true)?,
                    col: num(t[n - 3])?,
                    out: tok(t[n - 1])?,
                    bound: Some((tok(t[n - 2])?, matches!(t[0], "maxlt" | "mingt"))),
                },
                "strjoin" if n >= 7 => {
                    let column = |x: &str| -> Result<(usize, bool), String> {
                        let (c, sym) = x.split_once(':').ok_or("bad strjoin column")?;
                        Ok((num(c)?, sym == "1"))
                    };
                    JoinItem::StrJoin {
                        src: source(t[1])?,
                        args: args(&t[2..n - 4], true)?,
                        x: column(t[n - 4])?,
                        by: if t[n - 3] == "-" { Vec::new() } else { t[n - 3].split(',').map(column).collect::<Result<_, _>>()? },
                        sep: tok(t[n - 2])?,
                        out: tok(t[n - 1])?,
                    }
                }
                "strjoin" => return Err("bad strjoin".into()),
                "cat" => JoinItem::Cat { out: tok(at(1)?)?, a: tok(at(2)?)?, b: tok(at(3)?)? },
                "lt" | "le" => JoinItem::Lt { a: tok(at(1)?)?, b: tok(at(2)?)?, or_eq: t[0] == "le" },
                "func" => JoinItem::Func { name: at(1)?.to_owned(), out: tok(at(2)?)?, args: toks(&t[3.min(n)..])? },
                "test" => JoinItem::Test { op: at(1)?.to_owned(), s: tok(at(2)?)?, p: tok(at(3)?)?, want: want(at(4)?) },
                "docof" => JoinItem::DocOf { out: tok(at(1)?)?, n: tok(at(2)?)? },
                "testset" => JoinItem::TestSet { op: at(1)?.to_owned(), s: tok(at(2)?)?, want: want(at(3)?), members: toks(&t[4.min(n)..])? },
                "in" => JoinItem::In { a: tok(at(1)?)?, want: want(at(2)?), set: toks(&t[3.min(n)..])? },
                "deq" | "dne" => JoinItem::Doc { a: tok(at(1)?)?, b: tok(at(2)?)?, eq: t[0] == "deq" },
                "eq" | "ne" => JoinItem::Eq { a: tok(at(1)?)?, b: tok(at(2)?)?, ne: t[0] == "ne" },
                "near" | "down" => JoinItem::Near { down: t[0] == "down", n: tok(at(1)?)?, a: tok(at(2)?)?, kinds: unhex(at(3)?)? },
                "head" => JoinItem::Head(t[1..].iter().map(|x| head_tok(x)).collect::<Result<_, _>>()?),
                other => return Err(format!("unknown join item {other}")),
            });
        }
        Ok(JoinSpec { items, ..Default::default() })
    }
}

/// A spec line's text: its joins (`Join`) found in `spec`, the section that names them.
fn line_text(spec: &Spec, l: &ArchivedLine) -> String {
    let join = |j: &Handle<Section<JoinSpec>>| hex(&spec.bound(j).to_text());
    match l {
        ArchivedLine::Rel { name, types, start } => {
            let start = match start {
                ArchivedStart::Rows => String::new(),
                ArchivedStart::Empty => " empty".to_owned(),
                ArchivedStart::From(sql) => format!(" from {}", hex(sql)),
                ArchivedStart::Base { old, gone, seeded } => format!(" base {} {}{}", hex(old), if gone == "-" { "-".to_owned() } else { hex(gone) }, if *seeded { " seeded" } else { "" }),
            };
            format!("rel {name} {types}{start}")
        }
        ArchivedLine::NoWrite => "nowrite".to_owned(),
        ArchivedLine::Device => "device".to_owned(),
        ArchivedLine::Leveled(rels) => format!("leveled {}", comma(rels)),
        ArchivedLine::Measured { desc, key, keyed, cols, ranks } => format!("measured {} {} {} {} {}", if *desc { "d" } else { "a" }, key.as_ref().map_or_else(|| "-".to_owned(), |k| hex(k)), if *keyed { "k" } else { "u" }, comma(cols), comma(ranks)),
        ArchivedLine::KeptOld => "keptold".to_owned(),
        ArchivedLine::KeptRows(rels) => format!("keptrows {}", comma(rels)),
        ArchivedLine::Lattice { rel, keys, name, widen } => format!("lattice {rel} {keys} {} {}", hex(name), widen.get().map_or_else(|| "-".to_owned(), |w| w.to_string())),
        ArchivedLine::Key { rel, cols } => format!("key {rel} {}", cols.iter().map(u64::to_string).collect::<Vec<_>>().join(",")),
        ArchivedLine::Request(r) => format!("request {r}"),
        ArchivedLine::Produced(r) => format!("produced {r}"),
        ArchivedLine::Stats(r) => format!("stats {r}"),
        ArchivedLine::Sorted(rels) => format!("sorted {}", comma(rels)),
        ArchivedLine::Seed { head, join: j, .. } => format!("seed {head} {}", join(j)),
        ArchivedLine::Remove { head, join: j } => format!("remove {head} {}", join(j)),
        ArchivedLine::AddChecked { head, join: j } => format!("addchecked {head} {}", join(j)),
        ArchivedLine::Recheck { head, join: j } => format!("recheck {head} {}", join(j)),
        ArchivedLine::Join { head, delta, join: j, whole, .. } => match whole.as_ref() {
            Some(w) => format!("join {head} {delta} {} {}", join(j), join(w)),
            None => format!("join {head} {delta} {}", join(j)),
        },
        ArchivedLine::Probe { head, delta, sql } => format!("probe {head} {delta} {}", hex(sql)),
        ArchivedLine::Unplanned { head, delta } => format!("unplanned {head} {delta}"),
        ArchivedLine::Gate { done, src, quiet, after } => {
            format!("gate {done} {} {} {}", src.get().map_or("-".to_owned(), |s| s.to_string()), comma(quiet), comma(after))
        }
        ArchivedLine::Edge(e) => {
            let slots = |xs: &[u64]| xs.iter().map(|s| format!("v{s}")).collect::<Vec<_>>().join(" ");
            let src = match e.edge.as_ref() {
                Some(Tuple2(table, types)) => format!("{table} {}", if types.is_empty() { "-" } else { types }),
                None => "- -".to_owned(),
            };
            format!("rule {} {} {src} key {} ; ecols {} ; pat {} ; head {}", e.head, e.src, slots(&e.key), slots(&e.ecols), joined(&e.pat), joined(&e.out))
        }
    }
}

/// A component's spec's text: `name`, then one line per item.
pub fn spec_text(spec: &Spec) -> String {
    let mut s = format!("name {}", spec.name);
    for l in spec.lines.iter() {
        let _ = write!(s, "\n{}", line_text(spec, l));
    }
    s
}

impl ComponentSpec {
    /// Its text (`spec_text`).
    pub fn to_text(&self) -> String {
        spec_text(&self.encode())
    }

    pub fn parse(text: &str) -> Result<ComponentSpec, String> {
        let mut spec = ComponentSpec::default();
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let t: Vec<&str> = line.split_whitespace().collect();
            let at = |i: usize| t.get(i).copied().ok_or_else(|| format!("short spec item: {line}"));
            let join = |x: &str| -> Result<Join, String> { Ok(JoinSpec::parse(&unhex(x)?)?.encode()) };
            spec.lines.push(match t[0] {
                "name" => {
                    spec.name = at(1)?.to_owned();
                    continue;
                }
                "rel" => {
                    let (mut empty, mut from, mut base, mut seeded) = (false, None, None, false);
                    let mut k = 3;
                    while k < t.len() {
                        match t[k] {
                            "empty" => {
                                empty = true;
                                k += 1;
                            }
                            "seeded" => {
                                seeded = true;
                                k += 1;
                            }
                            "base" => {
                                let (old, gone) = (t.get(k + 1).ok_or("rel: base needs two tables")?, t.get(k + 2).ok_or("rel: base needs two tables")?);
                                base = Some((unhex(old)?, if *gone == "-" { "-".to_owned() } else { unhex(gone)? }));
                                k += 3;
                            }
                            "from" => {
                                from = Some(unhex(t.get(k + 1).ok_or("rel: from needs SQL")?)?);
                                k += 2;
                            }
                            other => return Err(format!("rel: unknown token {other}")),
                        }
                    }
                    let start = match (base, from) {
                        (Some((old, gone)), _) => Start::Base { old, gone, seeded },
                        (None, Some(sql)) => Start::From(sql),
                        (None, None) if empty => Start::Empty,
                        (None, None) => Start::Rows,
                    };
                    Line::Rel { name: at(1)?.to_owned(), types: at(2)?.to_owned(), start }
                }
                "nowrite" => Line::NoWrite,
                "device" => Line::Device,
                "leveled" => Line::Leveled(t.get(1).map(|x| list(x)).transpose()?.unwrap_or_default()),
                "measured" => Line::Measured {
                    desc: at(1)? == "d",
                    key: if at(2)? == "-" { None } else { Some(unhex(at(2)?)?) },
                    keyed: at(3)? == "k",
                    cols: list(at(4)?)?,
                    ranks: list(at(5)?)?,
                },
                "keptold" => Line::KeptOld,
                "keptrows" => Line::KeptRows(list(at(1)?)?),
                "lattice" => Line::Lattice {
                    rel: num(at(1)?)?,
                    keys: num(at(2)?)?,
                    name: unhex(at(3)?)?,
                    widen: if at(4)? == "-" { None } else { Some(num(at(4)?)?) },
                },
                "key" => Line::Key { rel: num(at(1)?)?, cols: list(at(2)?)? },
                "request" => Line::Request(num(at(1)?)?),
                "produced" => Line::Produced(num(at(1)?)?),
                "stats" => Line::Stats(num(at(1)?)?),
                "sorted" => Line::Sorted(list(at(1)?)?),
                "seed" => Line::Seed { head: num(at(1)?)?, join: join(at(2)?)?, est: Est::default() },
                "remove" => Line::Remove { head: num(at(1)?)?, join: join(at(2)?)? },
                "addchecked" => Line::AddChecked { head: num(at(1)?)?, join: join(at(2)?)? },
                "recheck" => Line::Recheck { head: num(at(1)?)?, join: join(at(2)?)? },
                "join" => Line::Join {
                    head: num(at(1)?)?,
                    delta: num(at(2)?)?,
                    join: join(at(3)?)?,
                    whole: t.get(4).map(|w| join(w)).transpose()?,
                    est: Est::default(),
                },
                "probe" => Line::Probe { head: num(at(1)?)?, delta: num(at(2)?)?, sql: unhex(at(3)?)? },
                "unplanned" => Line::Unplanned { head: num(at(1)?)?, delta: num(at(2)?)? },
                "gate" => Line::Gate {
                    done: num(at(1)?)?,
                    src: if at(2)? == "-" { None } else { Some(num(at(2)?)?) },
                    quiet: list(at(3)?)?,
                    after: list(at(4)?)?,
                },
                "rule" => {
                    let sections: Vec<&[&str]> = t.get(5..).unwrap_or(&[]).split(|x| *x == ";").collect();
                    let section = |name: &str| -> Result<&[&str], String> {
                        sections.iter().find(|s| s.first() == Some(&name)).map(|s| &s[1..]).ok_or_else(|| format!("no {name}"))
                    };
                    let slots = |xs: &[&str]| xs.iter().map(|x| slot(x)).collect::<Result<Vector<_>, _>>();
                    let edge = if at(3)? == "-" { None } else { Some((t[3].to_owned(), if at(4)? == "-" { String::new() } else { t[4].to_owned() })) };
                    Line::Edge(EdgeSpec {
                        head: num(at(1)?)?,
                        src: num(at(2)?)?,
                        edge,
                        key: slots(section("key")?)?,
                        ecols: slots(section("ecols")?)?,
                        pat: section("pat")?.iter().map(|x| tok(x)).collect::<Result<_, _>>()?,
                        out: section("head")?.iter().map(|x| head_tok(x)).collect::<Result<_, _>>()?,
                    })
                }
                other => return Err(format!("unknown spec item {other}")),
            });
        }
        Ok(spec)
    }
}

impl Reprobe {
    /// The SQL function's `kind` and hex `body` (`set_fixpoint_reprobe`), as a caller holding only text writes them.
    pub fn text(&self) -> (&'static str, String) {
        match self {
            Reprobe::Sql(sql) => ("sql", hex(sql)),
            Reprobe::Join(j) => ("join", hex(&j.to_text())),
            Reprobe::Whole(j) => ("whole", hex(&j.to_text())),
        }
    }

    pub fn parse(kind: &str, body: &str) -> Result<Reprobe, String> {
        let body = unhex(body)?;
        Ok(match kind {
            "sql" => Reprobe::Sql(body),
            "join" => Reprobe::Join(JoinSpec::parse(&body)?.encode()),
            "whole" => Reprobe::Whole(JoinSpec::parse(&body)?.encode()),
            other => return Err(format!("unknown probe kind {other}")),
        })
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn a_spec_reads_back_from_its_text() {
        let join = "pos t:b:73656c:6330:i v0 _ i:3\nneg r:6162 v0 x:6869\ncount c1 v0 a0 a0 v2\nmax d0 v0 _ 1 v3\n\
                    strjoin c0 v0 _ _ 1:1 2:0,3:1 x:2c v4\ncat v5 v0 v4\nle v2 i:9\nfunc str_concat v6 v5 x:2e\n\
                    test prefix v6 x:61 1\ndocof v7 v0\ntestset contains v6 0 i:4 i:5\nin v0 1 i:1 i:2\ndeq v0 v7\n\
                    ne v0 v2\nhead v0 cat:v5:v6 i:1 x:7a";
        let parsed = JoinSpec::parse(join).unwrap();
        assert_eq!(parsed.to_text(), join);
        let comp = format!(
            "name prog_a\nrel a is empty\nrel b s from {}\nrel c i base {} {} seeded\nnowrite\nrequest 0\nproduced 1\nseed 0 {}\n\
             join 0 1 {} {}\nprobe 1 0 {}\ngate 2 - 0,1 -\ngate 2 0 - 0\nrule 0 1 sfx_a_0 ii key v0 ; ecols v1 ; pat v0 _ ; head v1 cat:v0:v1\n\
             rule 1 0 - - key  ; ecols  ; pat i:1 x:61 ; head v0",
            hex("SELECT 1"),
            hex("a_kept"),
            hex("a_gone"),
            hex(join),
            hex(join),
            hex("pos c0 v0\nhead v0"),
            hex("SELECT c0 FROM x"),
        );
        let spec = ComponentSpec::parse(&comp).unwrap();
        assert_eq!(spec.to_text(), comp);
        assert_eq!(ComponentSpec::parse(&spec.to_text()).unwrap(), spec);
        for (kind, body) in [("sql", hex("SELECT 1")), ("join", hex(join)), ("whole", hex(join))] {
            assert_eq!(Reprobe::parse(kind, &body).unwrap().text(), (kind, body));
        }
    }

    /// `RERAIL_SPEC_DIR=<dir>`: every spec a run wrote there (`RERAIL_SPIKE_SPECS`: its rules, a blank line, its spec)
    /// reads back to the same text.
    #[test]
    #[ignore]
    fn captured_specs_read_back_to_their_text() {
        let dir = std::env::var_os("RERAIL_SPEC_DIR").expect("RERAIL_SPEC_DIR");
        let mut n = 0;
        for e in std::fs::read_dir(dir).unwrap() {
            let path = e.unwrap().path();
            let file = std::fs::read_to_string(&path).unwrap();
            let text = file.rsplit("\n\n").next().unwrap();
            let spec = ComponentSpec::parse(text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert_eq!(spec.to_text(), text, "{}", path.display());
            n += 1;
        }
        eprintln!("{n} specs read back to their text");
        assert!(n > 0);
    }
}
