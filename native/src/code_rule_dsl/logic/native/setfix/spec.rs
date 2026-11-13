//! A component's spec as values. The planner hands these to the engine in process (`open_spec`, `reprobe_spec`),
//! so a plan is not written as text and parsed again. The text (the module's spec lines, joins hex-encoded within)
//! is kept for the SQL functions (`set_fixpoint_open(spec)`: T's extension, a copy of the engine with its own
//! components, over text symbols), the lab's capture files and the spikes' spec files: `to_text` writes what the
//! planner wrote, and `parse` reads it back.

use std::fmt::Write as _;
use std::sync::Arc;

/// A value position: `_`, `v<slot>`, `i:<int>` or `x:<hex symbol>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Tok {
    Wild,
    Slot(usize),
    Int(i64),
    Sym(String),
}

/// An atom's column: a value position, or an aggregate's own column (`a<name>`, the same name tests equality).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Arg {
    Tok(Tok),
    Local(String),
}

/// A head term: a value, or two slots' symbols joined by "." (`cat:v<a>:v<b>`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum HeadTok {
    Tok(Tok),
    Cat(usize, usize),
}

/// An input read for joins: `t:<b|d>:<hex select>:<hex columns>:<types>`. `base`: a base relation, kept across
/// runs that do not change it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InputKey {
    pub base: bool,
    pub sql: String,
    pub cols: String,
    pub types: String,
}

/// Where an atom's rows are: the component's relation `i`'s delta (`d<i>`) or all of it (`c<i>`), a lower stratum's
/// native relation (`r:<hex table>`) or an input.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Delta(usize),
    Full(usize),
    Native(String),
    Input(InputKey),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Extreme {
    Sum,
    Min,
    Max,
}

/// One step of a native join, in plan order, or its head (last).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum JoinItem {
    /// `pos|neg <src> <args…>`
    Atom { neg: bool, src: Source, args: Vec<Arg> },
    /// `count <src> <args…> <out>`
    Count { src: Source, args: Vec<Arg>, out: Tok },
    /// `sum|min|max <src> <args…> <value column> <out>`
    Extreme { op: Extreme, src: Source, args: Vec<Arg>, col: usize, out: Tok },
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
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct JoinSpec {
    pub items: Vec<JoinItem>,
}

/// How a relation starts: its table's rows, none, a query's rows, or a kept native relation less another's
/// (`base`), plus its table's rows when `seeded`. A `base` with no relation gone (`-`) is a copy of the kept
/// relation, its levels with it (decision 0090's `h__live`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Start {
    Rows,
    Empty,
    From(String),
    /// `gone` "-": no rows gone (all of `old`).
    Base { old: String, gone: String, seeded: bool },
}

/// An edge rule (`rule …`): see the module's spec.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EdgeSpec {
    pub head: usize,
    pub src: usize,
    /// The edge table and its column types.
    pub edge: Option<(String, String)>,
    pub key: Vec<usize>,
    pub ecols: Vec<usize>,
    pub pat: Vec<Tok>,
    pub out: Vec<HeadTok>,
}

/// One spec line, after `name`.
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
    Leveled(Vec<usize>),
    /// Its relations started from kept rows (`Start::Base`) hold them as no wave's delta: its first wave is driven by
    /// its inputs' changes (seeds that read them, `plan::reads_change`), not by its kept rows (decision 0122 §2).
    KeptOld,
    Request(usize),
    Produced(usize),
    /// The relation's statistics are kept as its rows come (`Rel::stats`): a session keeps it for its next run.
    Stats(usize),
    Seed { head: usize, join: Arc<JoinSpec> },
    Join { head: usize, delta: usize, join: Arc<JoinSpec>, whole: Option<Arc<JoinSpec>> },
    Probe { head: usize, delta: usize, sql: String },
    Gate { done: usize, src: Option<usize>, quiet: Vec<usize>, after: Vec<usize> },
    /// A recursive relation maintained by levels (decision 0090): its candidates, the level checked, those held,
    /// those gone and the kept rows shown (`levels <cand> <chk> <held> <gone> <live>`).
    Levels { cand: usize, chk: usize, held: usize, gone: usize, live: usize },
    Edge(EdgeSpec),
}

/// A component: its name, then its lines in order (relations are numbered in order, probes are numbered in order).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ComponentSpec {
    pub name: String,
    pub lines: Vec<Line>,
}

/// A probe's new body (`reprobe`): SQL, a join, or the join it runs in the wave whose delta is the whole relation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Reprobe {
    Sql(String),
    Join(Arc<JoinSpec>),
    Whole(Arc<JoinSpec>),
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
    if s.len() % 2 != 0 || !s.is_ascii() {
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

fn list(t: &str) -> Result<Vec<usize>, String> {
    if t == "-" { Ok(Vec::new()) } else { t.split(',').map(num).collect() }
}

impl std::fmt::Display for Tok {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tok::Wild => f.write_str("_"),
            Tok::Slot(s) => write!(f, "v{s}"),
            Tok::Int(i) => write!(f, "i:{i}"),
            Tok::Sym(s) => write!(f, "x:{}", hex(s)),
        }
    }
}

impl std::fmt::Display for Arg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Arg::Tok(t) => t.fmt(f),
            Arg::Local(l) => write!(f, "a{l}"),
        }
    }
}

impl std::fmt::Display for HeadTok {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HeadTok::Tok(t) => t.fmt(f),
            HeadTok::Cat(a, b) => write!(f, "cat:v{a}:v{b}"),
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

impl std::fmt::Display for InputKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}:{}", if self.base { "b" } else { "d" }, hex(&self.sql), hex(&self.cols), self.types)
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Delta(i) => write!(f, "d{i}"),
            Source::Full(i) => write!(f, "c{i}"),
            Source::Native(t) => write!(f, "r:{}", hex(t)),
            Source::Input(k) => write!(f, "t:{k}"),
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

fn comma(xs: &[usize]) -> String {
    if xs.is_empty() { "-".to_owned() } else { xs.iter().map(usize::to_string).collect::<Vec<_>>().join(",") }
}

impl std::fmt::Display for JoinItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let col = |(c, sym): (usize, bool)| format!("{c}:{}", u8::from(sym));
        match self {
            JoinItem::Atom { neg, src, args } => write!(f, "{} {src} {}", if *neg { "neg" } else { "pos" }, joined(args)),
            JoinItem::Count { src, args, out } => write!(f, "count {src} {} {out}", joined(args)),
            JoinItem::Extreme { op, src, args, col, out } => {
                let name = match op {
                    Extreme::Sum => "sum",
                    Extreme::Min => "min",
                    Extreme::Max => "max",
                };
                write!(f, "{name} {src} {} {col} {out}", joined(args))
            }
            JoinItem::StrJoin { src, args, x, by, sep, out } => {
                let by = if by.is_empty() { "-".to_owned() } else { by.iter().map(|&b| col(b)).collect::<Vec<_>>().join(",") };
                write!(f, "strjoin {src} {} {} {by} {sep} {out}", joined(args), col(*x))
            }
            JoinItem::Cat { out, a, b } => write!(f, "cat {out} {a} {b}"),
            JoinItem::Lt { a, b, or_eq } => write!(f, "{} {a} {b}", if *or_eq { "le" } else { "lt" }),
            JoinItem::Func { name, out, args } => write!(f, "func {name} {out} {}", joined(args)),
            JoinItem::Test { op, s, p, want } => write!(f, "test {op} {s} {p} {}", u8::from(*want)),
            JoinItem::DocOf { out, n } => write!(f, "docof {out} {n}"),
            JoinItem::TestSet { op, s, want, members } => write!(f, "testset {op} {s} {} {}", u8::from(*want), joined(members)),
            JoinItem::In { a, want, set } => write!(f, "in {a} {} {}", u8::from(*want), joined(set)),
            JoinItem::Doc { a, b, eq } => write!(f, "{} {a} {b}", if *eq { "deq" } else { "dne" }),
            JoinItem::Eq { a, b, ne } => write!(f, "{} {a} {b}", if *ne { "ne" } else { "eq" }),
            JoinItem::Near { down, n, a, kinds } => write!(f, "{} {n} {a} {}", if *down { "down" } else { "near" }, hex(kinds)),
            JoinItem::Head(terms) => write!(f, "head {}", joined(terms)),
        }
    }
}

impl JoinSpec {
    /// Its text: one line per item.
    pub fn to_text(&self) -> String {
        self.items.iter().map(JoinItem::to_string).collect::<Vec<_>>().join("\n")
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
        Ok(JoinSpec { items })
    }
}

impl std::fmt::Display for Line {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Line::Rel { name, types, start } => {
                write!(f, "rel {name} {types}")?;
                match start {
                    Start::Rows => Ok(()),
                    Start::Empty => f.write_str(" empty"),
                    Start::From(sql) => write!(f, " from {}", hex(sql)),
                    Start::Base { old, gone, seeded } => write!(f, " base {} {}{}", hex(old), if gone == "-" { "-".to_owned() } else { hex(gone) }, if *seeded { " seeded" } else { "" }),
                }
            }
            Line::NoWrite => f.write_str("nowrite"),
            Line::Device => f.write_str("device"),
            Line::Leveled(rels) => write!(f, "leveled {}", comma(rels)),
            Line::KeptOld => f.write_str("keptold"),
            Line::Request(r) => write!(f, "request {r}"),
            Line::Produced(r) => write!(f, "produced {r}"),
            Line::Stats(r) => write!(f, "stats {r}"),
            Line::Seed { head, join } => write!(f, "seed {head} {}", hex(&join.to_text())),
            Line::Join { head, delta, join, whole } => {
                write!(f, "join {head} {delta} {}", hex(&join.to_text()))?;
                match whole {
                    Some(w) => write!(f, " {}", hex(&w.to_text())),
                    None => Ok(()),
                }
            }
            Line::Probe { head, delta, sql } => write!(f, "probe {head} {delta} {}", hex(sql)),
            Line::Gate { done, src, quiet, after } => {
                write!(f, "gate {done} {} {} {}", src.map_or("-".to_owned(), |s| s.to_string()), comma(quiet), comma(after))
            }
            Line::Levels { cand, chk, held, gone, live } => write!(f, "levels {cand} {chk} {held} {gone} {live}"),
            Line::Edge(e) => {
                let slots = |xs: &[usize]| xs.iter().map(|s| format!("v{s}")).collect::<Vec<_>>().join(" ");
                let src = match &e.edge {
                    Some((table, types)) => format!("{table} {}", if types.is_empty() { "-" } else { types }),
                    None => "- -".to_owned(),
                };
                write!(f, "rule {} {} {src} key {} ; ecols {} ; pat {} ; head {}", e.head, e.src, slots(&e.key), slots(&e.ecols), joined(&e.pat), joined(&e.out))
            }
        }
    }
}

impl ComponentSpec {
    /// Its text: `name`, then one line per item.
    pub fn to_text(&self) -> String {
        let mut s = format!("name {}", self.name);
        for l in &self.lines {
            let _ = write!(s, "\n{l}");
        }
        s
    }

    pub fn parse(text: &str) -> Result<ComponentSpec, String> {
        let mut spec = ComponentSpec::default();
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let t: Vec<&str> = line.split_whitespace().collect();
            let at = |i: usize| t.get(i).copied().ok_or_else(|| format!("short spec item: {line}"));
            let join = |x: &str| -> Result<Arc<JoinSpec>, String> { Ok(Arc::new(JoinSpec::parse(&unhex(x)?)?)) };
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
                "keptold" => Line::KeptOld,
                "request" => Line::Request(num(at(1)?)?),
                "produced" => Line::Produced(num(at(1)?)?),
                "stats" => Line::Stats(num(at(1)?)?),
                "seed" => Line::Seed { head: num(at(1)?)?, join: join(at(2)?)? },
                "join" => Line::Join {
                    head: num(at(1)?)?,
                    delta: num(at(2)?)?,
                    join: join(at(3)?)?,
                    whole: t.get(4).map(|w| join(w)).transpose()?,
                },
                "probe" => Line::Probe { head: num(at(1)?)?, delta: num(at(2)?)?, sql: unhex(at(3)?)? },
                "gate" => Line::Gate {
                    done: num(at(1)?)?,
                    src: if at(2)? == "-" { None } else { Some(num(at(2)?)?) },
                    quiet: list(at(3)?)?,
                    after: list(at(4)?)?,
                },
                "levels" => Line::Levels { cand: num(at(1)?)?, chk: num(at(2)?)?, held: num(at(3)?)?, gone: num(at(4)?)?, live: num(at(5)?)? },
                "rule" => {
                    let sections: Vec<&[&str]> = t.get(5..).unwrap_or(&[]).split(|x| *x == ";").collect();
                    let section = |name: &str| -> Result<&[&str], String> {
                        sections.iter().find(|s| s.first() == Some(&name)).map(|s| &s[1..]).ok_or_else(|| format!("no {name}"))
                    };
                    let slots = |xs: &[&str]| xs.iter().map(|x| slot(x)).collect::<Result<Vec<_>, _>>();
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
            "join" => Reprobe::Join(Arc::new(JoinSpec::parse(&body)?)),
            "whole" => Reprobe::Whole(Arc::new(JoinSpec::parse(&body)?)),
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
            hex("a__old"),
            hex("a__del"),
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
