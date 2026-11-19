//! Array programs: the transducer's side program over integer columns (decision 0421's passes;
//! `docs/parser-generators/12-what-is-wanted.md` §3), held in `Words::array_programs` as block
//! programs are held in `Words::block_programs`.
//!
//! A block program works on masks, a bit a byte. An array program works on arrays of 64-bit
//! integers: a translation unit's directives, definitions, text tokens or invocations, one
//! element each. Operation `i` defines array `%i`. Every operation is one of the data-parallel
//! primitives a device runs as it is: a map, a scan, a sort, a search, a gather, a scatter, a
//! compaction, an expansion. The one exception is `call`: a named function run on each element
//! alone (an `#if` line's value, one invocation's expansion), whose work has a bound of its own,
//! and whose results may be any number a element (a ragged result, read by `part`).
//!
//! A length-1 array is a scalar: a map, a select or a scatter's fill broadcasts it.
//!
//! A program may hold fixed points (`rounds`), as block programs do: each group's carries, each
//! with its next. A group spans its first carry to its last next, and groups do not overlap.
//! The operations run in order; at the end of a group's span, while some carry of it differs
//! from its next, the span's values that depend on its carries run again, each carry its next
//! of the round before. The values after the span read the settled ones. Past `MAX_ROUNDS` the
//! run is refused.

use crate::Error;

thread_local! {
    static PROFILE: std::cell::RefCell<Option<std::collections::HashMap<(usize, usize), (u64, u64, u64)>>> = const { std::cell::RefCell::new(None) };
}

/// Profile this thread's runs from now (`true`), or stop: each operation's steps, time (ns)
/// and elements made, by (program's address, operation), taken.
pub fn profile(on: bool) -> std::collections::HashMap<(usize, usize), (u64, u64, u64)> {
    PROFILE.with(|p| std::mem::replace(&mut *p.borrow_mut(), on.then(Default::default)).unwrap_or_default())
}

/// The most rounds a fixed point runs before its run is refused.
pub const MAX_ROUNDS: usize = 1000;

/// The most keys a sort orders by.
pub const MAX_KEYS: usize = 3;

/// An elementwise operation (`ArrayOp::Map`). A comparison gives 1 or 0. A shift's count is
/// taken modulo 64.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MapOp {
    Add,
    Sub,
    Mul,
    And,
    Or,
    Xor,
    Min,
    Max,
    Eq,
    Ne,
    Lt,
    Le,
    Shl,
    /// An arithmetic shift right.
    Shr,
}

impl MapOp {
    pub const ALL: [(MapOp, &'static str); 14] = [
        (MapOp::Add, "add"),
        (MapOp::Sub, "sub"),
        (MapOp::Mul, "mul"),
        (MapOp::And, "and"),
        (MapOp::Or, "or"),
        (MapOp::Xor, "xor"),
        (MapOp::Min, "min"),
        (MapOp::Max, "max"),
        (MapOp::Eq, "eq"),
        (MapOp::Ne, "ne"),
        (MapOp::Lt, "lt"),
        (MapOp::Le, "le"),
        (MapOp::Shl, "shl"),
        (MapOp::Shr, "shr"),
    ];

    pub fn name(self) -> &'static str {
        MapOp::ALL.iter().find(|p| p.0 == self).expect("every map is named").1
    }

    /// `a op b`.
    pub fn apply(self, a: i64, b: i64) -> i64 {
        match self {
            MapOp::Add => a.wrapping_add(b),
            MapOp::Sub => a.wrapping_sub(b),
            MapOp::Mul => a.wrapping_mul(b),
            MapOp::And => a & b,
            MapOp::Or => a | b,
            MapOp::Xor => a ^ b,
            MapOp::Min => a.min(b),
            MapOp::Max => a.max(b),
            MapOp::Eq => i64::from(a == b),
            MapOp::Ne => i64::from(a != b),
            MapOp::Lt => i64::from(a < b),
            MapOp::Le => i64::from(a <= b),
            MapOp::Shl => a.wrapping_shl(b as u32 & 63),
            MapOp::Shr => a.wrapping_shr(b as u32 & 63),
        }
    }
}

/// A scan's or a scatter's combination: a sum, a maximum or a minimum; a scatter's `Set` writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Combine {
    Set,
    Add,
    Max,
    Min,
}

impl Combine {
    const ALL: [(Combine, &'static str); 4] = [(Combine::Set, "set"), (Combine::Add, "add"), (Combine::Max, "max"), (Combine::Min, "min")];

    pub fn name(self) -> &'static str {
        Combine::ALL.iter().find(|p| p.0 == self).expect("every combination is named").1
    }

    /// `a` combined with `b`.
    pub fn apply(self, a: i64, b: i64) -> i64 {
        match self {
            Combine::Set => b,
            Combine::Add => a.wrapping_add(b),
            Combine::Max => a.max(b),
            Combine::Min => a.min(b),
        }
    }

    /// The scan's identity: what an exclusive scan's first element is.
    pub fn identity(self) -> i64 {
        match self {
            Combine::Set | Combine::Add => 0,
            Combine::Max => i64::MIN,
            Combine::Min => i64::MAX,
        }
    }
}

/// An operation of an array program; operands are earlier values (`%v`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ArrayOp {
    /// The run's column `k`.
    Input(u16),
    /// The run's parameter `k`: a scalar.
    Param(u16),
    /// A scalar.
    Const(i64),
    /// `0, 1, …` as long as `%a`.
    Iota(u16),
    /// `%a`'s length: a scalar.
    Len(u16),
    /// `a op b` element by element; a scalar operand is broadcast.
    Map(MapOp, u16, u16),
    /// `m ? a : b` element by element (`m` nonzero); scalars broadcast.
    Select(u16, u16, u16),
    /// A prefix scan of `a` by `how` (not `Set`): inclusive, or exclusive (each element the
    /// combination of those before it). With `seg`, an element whose flag is nonzero starts a
    /// new scan.
    Scan { how: Combine, excl: bool, a: u16, seg: Option<u16> },
    /// The permutation that sorts by the keys, the first most significant: element `i` is the
    /// index of the `i`th smallest. Stable.
    Sort(Vec<u16>),
    /// `src[idx[i]]` for each `i`.
    Gather(u16, u16),
    /// An array of length `n` (a scalar), each element `init` (a scalar), then each `src[i]`
    /// combined into element `idx[i]` by `how`. An index outside `0..n` writes nothing. With
    /// `Set`, two writes to one element refuse the run (the order of writes is no executor's).
    Scatter { how: Combine, src: u16, idx: u16, n: u16, init: u16 },
    /// For each `q[i]`, the count of `sorted`'s elements below it (`lt`) or at most it (`le`):
    /// `sorted` must be sorted.
    Search { le: bool, sorted: u16, q: u16 },
    /// The indexes of `m`'s nonzero elements, in order.
    Compact(u16),
    /// For counts `c`, each output element's source: `i` repeated `c[i]` times.
    Expand(u16),
    /// `a`'s elements, then `b`'s.
    Concat(u16, u16),
    /// Function `f` (`ArrayProgram::funs`) on each element of the arguments (equally long; a
    /// scalar broadcast), with the `env` arrays whole (a table every element reads, as an
    /// expansion reads the definitions by position): a ragged result. The value is each
    /// element's count of results.
    Call { f: u16, args: Vec<u16>, env: Vec<u16> },
    /// Field `k` of a call's results, every element's in order: as long as the sum of the counts.
    Part(u16, u8),
    /// A fixed point's carry (`ArrayProgram::rounds`): in the first round, `init`; in each round
    /// after, its next's value in the round before.
    Carry(u16),
}

/// Where a function of `call` runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Place {
    /// Words of the program (an entry's label): run where the arrays are, one machine an
    /// element.
    Words,
    /// The host's: a table it answers (files by name). An executor away from the host ends its
    /// run where an element is not in the table; the host answers, and the run starts again.
    Host,
}

/// A function of `call`: its name, its place, and how many fields each of its results has.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Fun {
    pub name: String,
    pub place: Place,
    pub fields: u8,
}

/// An array program.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ArrayProgram {
    /// How many columns and parameters a run gives.
    pub inputs: u16,
    pub params: u16,
    pub funs: Vec<Fun>,
    pub ops: Vec<ArrayOp>,
    /// The fixed points: each group's carries, each with the value that is its next.
    pub rounds: Vec<Vec<(u16, u16)>>,
    pub outputs: Vec<u16>,
}

/// The primitive each operation is on a device: every operation is one of these, and only
/// `Call` runs a loop of its own an element (bounded by the function's bounds).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primitive {
    /// A value the run gives or a constant: no work.
    Given,
    /// One thread an element, no loop.
    Elementwise,
    Scan,
    Sort,
    /// A bounded binary search an element.
    Search,
    Gather,
    Scatter,
    /// A scan and a scatter.
    Compact,
    /// A scan, a scatter and a max scan.
    Expand,
    /// A function an element.
    Call,
}

impl ArrayOp {
    /// The primitive the operation is.
    pub fn primitive(&self) -> Primitive {
        match self {
            ArrayOp::Input(_) | ArrayOp::Param(_) | ArrayOp::Const(_) | ArrayOp::Len(_) | ArrayOp::Carry(_) => Primitive::Given,
            ArrayOp::Iota(_) | ArrayOp::Map(..) | ArrayOp::Select(..) => Primitive::Elementwise,
            ArrayOp::Scan { .. } => Primitive::Scan,
            ArrayOp::Sort(_) => Primitive::Sort,
            ArrayOp::Search { .. } => Primitive::Search,
            ArrayOp::Gather(..) | ArrayOp::Part(..) | ArrayOp::Concat(..) => Primitive::Gather,
            ArrayOp::Scatter { .. } => Primitive::Scatter,
            ArrayOp::Compact(_) => Primitive::Compact,
            ArrayOp::Expand(_) => Primitive::Expand,
            ArrayOp::Call { .. } => Primitive::Call,
        }
    }

    /// The values the operation reads.
    pub fn operands(&self) -> Vec<u16> {
        match self {
            ArrayOp::Input(_) | ArrayOp::Param(_) | ArrayOp::Const(_) => vec![],
            ArrayOp::Iota(a) | ArrayOp::Len(a) | ArrayOp::Compact(a) | ArrayOp::Expand(a) | ArrayOp::Carry(a) | ArrayOp::Part(a, _) => vec![*a],
            ArrayOp::Map(_, a, b) | ArrayOp::Gather(a, b) | ArrayOp::Concat(a, b) => vec![*a, *b],
            ArrayOp::Select(m, a, b) => vec![*m, *a, *b],
            ArrayOp::Scan { a, seg, .. } => [Some(*a), *seg].into_iter().flatten().collect(),
            ArrayOp::Sort(keys) => keys.clone(),
            ArrayOp::Scatter { src, idx, n, init, .. } => vec![*src, *idx, *n, *init],
            ArrayOp::Search { sorted, q, .. } => vec![*sorted, *q],
            ArrayOp::Call { args, env, .. } => args.iter().chain(env).copied().collect(),
        }
    }

    /// The operation's text (`rir`): a mnemonic and its operands.
    pub fn text(&self) -> String {
        let vs = |vs: &[u16]| vs.iter().map(|v| format!("%{v}")).collect::<Vec<_>>().join(" ");
        match self {
            ArrayOp::Input(k) => format!("input {k}"),
            ArrayOp::Param(k) => format!("param {k}"),
            ArrayOp::Const(c) => format!("const {c}"),
            ArrayOp::Iota(a) => format!("iota %{a}"),
            ArrayOp::Len(a) => format!("len %{a}"),
            ArrayOp::Map(op, a, b) => format!("map {} %{a} %{b}", op.name()),
            ArrayOp::Select(m, a, b) => format!("select %{m} %{a} %{b}"),
            ArrayOp::Scan { how, excl, a, seg } => {
                let excl = if *excl { " excl" } else { "" };
                let seg = seg.map_or(String::new(), |s| format!(" seg %{s}"));
                format!("scan {}{excl} %{a}{seg}", how.name())
            }
            ArrayOp::Sort(keys) => format!("sort {}", vs(keys)),
            ArrayOp::Gather(s, i) => format!("gather %{s} %{i}"),
            ArrayOp::Scatter { how, src, idx, n, init } => format!("scatter {} %{src} %{idx} %{n} %{init}", how.name()),
            ArrayOp::Search { le, sorted, q } => format!("search {} %{sorted} %{q}", if *le { "le" } else { "lt" }),
            ArrayOp::Compact(m) => format!("compact %{m}"),
            ArrayOp::Expand(c) => format!("expand %{c}"),
            ArrayOp::Concat(a, b) => format!("concat %{a} %{b}"),
            ArrayOp::Call { f, args, env } => {
                let mut s = format!("call f{f}");
                if !args.is_empty() {
                    s += &format!(" {}", vs(args));
                }
                if !env.is_empty() {
                    s += &format!(" env {}", vs(env));
                }
                s
            }
            ArrayOp::Part(c, k) => format!("part %{c} {k}"),
            ArrayOp::Carry(a) => format!("carry %{a}"),
        }
    }

    /// Parse `text`'s output, as words.
    pub fn parse(words: &[&str]) -> Result<ArrayOp, String> {
        let v = |w: &str| -> Result<u16, String> { w.strip_prefix('%').and_then(|v| v.parse().ok()).ok_or_else(|| format!("a bad value `{w}`")) };
        let n = |w: &str| -> Result<u16, String> { w.parse().map_err(|_| format!("a bad number `{w}`")) };
        let combine = |w: &str| Combine::ALL.iter().find(|p| p.1 == w).map(|p| p.0).ok_or_else(|| format!("a bad combination `{w}`"));
        let values = |ws: &[&str]| ws.iter().map(|w| v(w)).collect::<Result<Vec<_>, _>>();
        Ok(match words {
            ["input", k] => ArrayOp::Input(n(k)?),
            ["param", k] => ArrayOp::Param(n(k)?),
            ["const", c] => ArrayOp::Const(c.parse().map_err(|_| format!("a bad constant `{c}`"))?),
            ["iota", a] => ArrayOp::Iota(v(a)?),
            ["len", a] => ArrayOp::Len(v(a)?),
            ["map", op, a, b] => ArrayOp::Map(MapOp::ALL.iter().find(|p| p.1 == *op).ok_or_else(|| format!("a bad map `{op}`"))?.0, v(a)?, v(b)?),
            ["select", m, a, b] => ArrayOp::Select(v(m)?, v(a)?, v(b)?),
            ["scan", how, rest @ ..] => {
                let how = combine(how)?;
                let (excl, rest) = match rest {
                    ["excl", rest @ ..] => (true, rest),
                    _ => (false, rest),
                };
                match rest {
                    [a] => ArrayOp::Scan { how, excl, a: v(a)?, seg: None },
                    [a, "seg", s] => ArrayOp::Scan { how, excl, a: v(a)?, seg: Some(v(s)?) },
                    _ => return Err("`scan HOW [excl] %a [seg %f]`".into()),
                }
            }
            ["sort", keys @ ..] => ArrayOp::Sort(values(keys)?),
            ["gather", s, i] => ArrayOp::Gather(v(s)?, v(i)?),
            ["scatter", how, s, i, n, init] => ArrayOp::Scatter { how: combine(how)?, src: v(s)?, idx: v(i)?, n: v(n)?, init: v(init)? },
            ["search", le, s, q] if *le == "le" || *le == "lt" => ArrayOp::Search { le: *le == "le", sorted: v(s)?, q: v(q)? },
            ["compact", m] => ArrayOp::Compact(v(m)?),
            ["expand", c] => ArrayOp::Expand(v(c)?),
            ["concat", a, b] => ArrayOp::Concat(v(a)?, v(b)?),
            ["call", f, rest @ ..] => {
                let f = f.strip_prefix('f').and_then(|f| f.parse().ok()).ok_or_else(|| format!("a bad function `{f}`"))?;
                let k = rest.iter().position(|w| *w == "env").unwrap_or(rest.len());
                ArrayOp::Call { f, args: values(&rest[..k])?, env: values(rest.get(k + 1..).unwrap_or(&[]))? }
            }
            ["part", c, k] => ArrayOp::Part(v(c)?, k.parse().map_err(|_| format!("a bad field `{k}`"))?),
            ["carry", a] => ArrayOp::Carry(v(a)?),
            _ => return Err(format!("a bad array operation `{}`", words.join(" "))),
        })
    }
}

impl ArrayProgram {
    /// The verifier's rules: at least one operation and one output; every operand an earlier
    /// value, but a carry's, which may be any (its next is read a round later); inputs,
    /// parameters, functions and fields that the program declares; a sort of 1 to `MAX_KEYS`
    /// keys; a `part` of a call; each rounds pair a carry and a value.
    pub fn verify(&self) -> Result<(), Error> {
        let n = self.ops.len();
        if n == 0 || n > usize::from(u16::MAX) || self.outputs.is_empty() {
            return Err(Error::InvalidOperand(0));
        }
        for (i, op) in self.ops.iter().enumerate() {
            let bad = Err(Error::InvalidOperand(i));
            if op.operands().iter().any(|&v| usize::from(v) >= i) {
                return bad;
            }
            match op {
                ArrayOp::Input(k) if *k >= self.inputs => return bad,
                ArrayOp::Param(k) if *k >= self.params => return bad,
                ArrayOp::Sort(keys) if keys.is_empty() || keys.len() > MAX_KEYS => return bad,
                ArrayOp::Scan { how: Combine::Set, .. } => return bad,
                ArrayOp::Call { f, .. } if usize::from(*f) >= self.funs.len() => return bad,
                ArrayOp::Part(c, k) => match &self.ops[usize::from(*c)] {
                    ArrayOp::Call { f, .. } if *k < self.funs[usize::from(*f)].fields => {}
                    _ => return bad,
                },
                _ => {}
            }
        }
        for group in &self.rounds {
            if group.is_empty() {
                return Err(Error::InvalidOperand(0));
            }
            for &(c, x) in group {
                if !matches!(self.ops.get(usize::from(c)), Some(ArrayOp::Carry(_))) || usize::from(x) >= n {
                    return Err(Error::InvalidOperand(usize::from(c)));
                }
            }
        }
        // A group's nexts after its carries, and the groups' spans apart.
        let mut spans: Vec<(u16, u16)> = Vec::new();
        for group in &self.rounds {
            let lo = group.iter().map(|p| p.0).min().unwrap();
            let hi = group.iter().map(|p| p.1).max().unwrap();
            if group.iter().any(|&(c, x)| x < c) {
                return Err(Error::InvalidOperand(usize::from(lo)));
            }
            spans.push((lo, hi));
        }
        spans.sort_unstable();
        if spans.windows(2).any(|w| w[1].0 <= w[0].1) {
            return Err(Error::InvalidOperand(0));
        }
        // Every carry is in exactly one group.
        let carries = self.ops.iter().filter(|op| matches!(op, ArrayOp::Carry(_))).count();
        let mut grouped: Vec<u16> = self.rounds.iter().flatten().map(|&(c, _)| c).collect();
        grouped.sort_unstable();
        grouped.dedup();
        if grouped.len() != carries || grouped.len() != self.rounds.iter().map(Vec::len).sum::<usize>() {
            return Err(Error::InvalidOperand(0));
        }
        if self.outputs.iter().any(|&v| usize::from(v) >= n) {
            return Err(Error::InvalidOperand(0));
        }
        Ok(())
    }

    /// The program's canonical bytes, for the digest: its rows.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = String::new();
        array_rows(&mut out, 0, self);
        out.into_bytes()
    }

    /// Read one `array N …` row's words after `N` into the program.
    pub fn row(&mut self, words: &[&str]) -> Result<(), String> {
        let number = |w: &str| -> Result<u16, String> { w.parse().map_err(|_| format!("a bad number `{w}`")) };
        let value = |w: &str| -> Result<u16, String> { w.strip_prefix('%').and_then(|v| v.parse().ok()).ok_or_else(|| format!("a bad value `{w}`")) };
        match words {
            ["shape", i, p] => {
                if !self.ops.is_empty() {
                    return Err("array shape after the operations".into());
                }
                self.inputs = number(i)?;
                self.params = number(p)?;
            }
            ["fun", f, place, name, fields] => {
                if f.strip_prefix('f').and_then(|f| f.parse::<usize>().ok()) != Some(self.funs.len()) {
                    return Err(format!("array fun {f} out of order"));
                }
                let place = match *place {
                    "words" => Place::Words,
                    "host" => Place::Host,
                    _ => return Err(format!("array fun's place is `words` or `host`, not `{place}`")),
                };
                let fields = fields.strip_prefix("fields=").and_then(|k| k.parse().ok()).ok_or("`fields=K`")?;
                self.funs.push(Fun { name: (*name).to_owned(), place, fields });
            }
            [v, "=", op @ ..] => {
                if value(v)? as usize != self.ops.len() {
                    return Err(format!("array value {v} out of order"));
                }
                self.ops.push(ArrayOp::parse(op)?);
            }
            ["rounds", pairs @ ..] => {
                if pairs.is_empty() || pairs.len() % 2 != 0 {
                    return Err("`rounds %c %n …`".into());
                }
                self.rounds.push(pairs.chunks(2).map(|p| Ok((value(p[0])?, value(p[1])?))).collect::<Result<_, String>>()?);
            }
            ["output", v] => self.outputs.push(value(v)?),
            _ => return Err(format!("a bad array row `{}`", words.join(" "))),
        }
        Ok(())
    }
}

/// Program `at`'s rows (`rir`), in the order `ArrayProgram::row` reads them.
pub fn array_rows(out: &mut String, at: usize, p: &ArrayProgram) {
    use std::fmt::Write as _;
    let _ = writeln!(out, "  array {at} shape {} {}", p.inputs, p.params);
    for (f, fun) in p.funs.iter().enumerate() {
        let place = match fun.place {
            Place::Words => "words",
            Place::Host => "host",
        };
        let _ = writeln!(out, "  array {at} fun f{f} {place} {} fields={}", fun.name, fun.fields);
    }
    for (v, op) in p.ops.iter().enumerate() {
        let _ = writeln!(out, "  array {at} %{v} = {}", op.text());
    }
    for group in &p.rounds {
        let pairs: Vec<String> = group.iter().map(|(c, x)| format!("%{c} %{x}")).collect();
        let _ = writeln!(out, "  array {at} rounds {}", pairs.join(" "));
    }
    for v in &p.outputs {
        let _ = writeln!(out, "  array {at} output %{v}");
    }
}

/// What a run's `call`s ask of their functions: function `f` (`ArrayProgram::funs`) of one
/// element's arguments and the call's `env` arrays, its results pushed to `out` (each `fields`
/// values).
pub trait Calls {
    fn call(&mut self, f: u16, fun: &Fun, args: &[i64], env: &[&[i64]], out: &mut Vec<i64>) -> Result<(), &'static str>;

    /// The calls of `f` on `w` elements at once (element `j`'s arguments `args[k][j]`, an
    /// argument of one element broadcast): each one's results to `out[j]`. By default one `call`
    /// an element, in order; a host that reads many elements together (a batch on the device)
    /// answers them at once, as one by one would. A refusal is the first failing element's.
    fn call_many(&mut self, f: u16, fun: &Fun, args: &[&[i64]], w: usize, env: &[&[i64]], out: &mut Vec<Vec<i64>>) -> Result<(), &'static str> {
        let mut one = Vec::with_capacity(args.len());
        for j in 0..w {
            one.clear();
            one.extend(args.iter().map(|x| if x.len() == 1 { x[0] } else { x[j] }));
            let mut rows = Vec::new();
            self.call(f, fun, &one, env, &mut rows)?;
            out.push(rows);
        }
        Ok(())
    }
}

/// No functions: a program without calls.
impl Calls for () {
    fn call(&mut self, _: u16, _: &Fun, _: &[i64], _: &[&[i64]], _: &mut Vec<i64>) -> Result<(), &'static str> {
        Err("no functions")
    }
}

/// A run's values: each operation's array, and each call's results (field-major).
pub struct Values {
    pub arrays: Vec<Vec<i64>>,
    pub parts: Vec<Vec<Vec<i64>>>,
    /// The most rounds a fixed point took (1 when there are none).
    pub rounds: usize,
}

impl ArrayProgram {
    /// The program's meaning: its outputs, from the run's columns and parameters. A run is
    /// refused (with the reason) where an operation's operands do not fit it: unequal lengths,
    /// an index outside its array, two writes of a `set` scatter to one element, a search of
    /// an array not sorted, a negative count, or a fixed point past `MAX_ROUNDS`.
    pub fn run(&self, inputs: &[&[i64]], params: &[i64], calls: &mut dyn Calls) -> Result<Vec<Vec<i64>>, &'static str> {
        let v = self.values(inputs, params, calls)?;
        Ok(self.outputs.iter().map(|&o| v.arrays[usize::from(o)].clone()).collect())
    }

    /// Every value of a run (`run`).
    pub fn values(&self, inputs: &[&[i64]], params: &[i64], calls: &mut dyn Calls) -> Result<Values, &'static str> {
        self.values_at(inputs, params, calls).map_err(|e| e.1)
    }

    /// `values`, a refusal with the operation that refused it.
    pub fn values_at(&self, inputs: &[&[i64]], params: &[i64], calls: &mut dyn Calls) -> Result<Values, (usize, &'static str)> {
        if inputs.len() != usize::from(self.inputs) || params.len() != usize::from(self.params) {
            return Err((0, "the run's columns or parameters are not the program's"));
        }
        let n = self.ops.len();
        let mut v = Values { arrays: vec![Vec::new(); n], parts: vec![Vec::new(); n], rounds: 1 };
        // Each group by the end of its span.
        let mut ends: Vec<(usize, &Vec<(u16, u16)>)> = self.rounds.iter().map(|g| (usize::from(g.iter().map(|p| p.1).max().unwrap()), g)).collect();
        ends.sort_unstable_by_key(|e| e.0);
        let mut next_end = 0;
        for i in 0..n {
            self.step(i, &mut v, inputs, params, calls, None).map_err(|e| (i, e))?;
            while next_end < ends.len() && ends[next_end].0 == i {
                self.settle(ends[next_end].1, &mut v, inputs, params, calls)?;
                next_end += 1;
            }
        }
        Ok(v)
    }

    /// Group `group`'s rounds, its span run once: until each carry is its next.
    fn settle(&self, group: &[(u16, u16)], v: &mut Values, inputs: &[&[i64]], params: &[i64], calls: &mut dyn Calls) -> Result<(), (usize, &'static str)> {
        let lo = usize::from(group.iter().map(|p| p.0).min().unwrap());
        let hi = usize::from(group.iter().map(|p| p.1).max().unwrap());
        // The span's values that depend on its carries.
        let mut moved = vec![false; hi + 1];
        for i in lo..=hi {
            let op = &self.ops[i];
            moved[i] = group.iter().any(|p| usize::from(p.0) == i) || op.operands().iter().any(|&o| usize::from(o) >= lo && moved[usize::from(o)]);
        }
        let mut rounds = 1;
        loop {
            if group.iter().all(|&(c, x)| v.arrays[usize::from(c)] == v.arrays[usize::from(x)]) {
                v.rounds = v.rounds.max(rounds);
                return Ok(());
            }
            if rounds >= MAX_ROUNDS {
                return Err((hi, "past the rounds' bound"));
            }
            rounds += 1;
            let nexts: Vec<(usize, Vec<i64>)> = group.iter().map(|&(c, x)| (usize::from(c), v.arrays[usize::from(x)].clone())).collect();
            for i in (lo..=hi).filter(|&i| moved[i]) {
                let carried = nexts.iter().find(|n| n.0 == i).map(|n| n.1.clone());
                self.step(i, v, inputs, params, calls, carried).map_err(|e| (i, e))?;
            }
        }
    }

    /// Operation `i`'s value; a carry's is `carried` after the first round. Where this thread
    /// profiles (`profile`), its count and time.
    fn step(&self, i: usize, v: &mut Values, inputs: &[&[i64]], params: &[i64], calls: &mut dyn Calls, carried: Option<Vec<i64>>) -> Result<(), &'static str> {
        if !PROFILE.with(|p| p.borrow().is_some()) {
            return self.step_op(i, v, inputs, params, calls, carried);
        }
        let t = std::time::Instant::now();
        let r = self.step_op(i, v, inputs, params, calls, carried);
        let ns = t.elapsed().as_nanos() as u64;
        let len = v.arrays[i].len() as u64;
        PROFILE.with(|p| {
            let mut p = p.borrow_mut();
            let e = p.as_mut().unwrap().entry((self as *const ArrayProgram as usize, i)).or_default();
            *e = (e.0 + 1, e.1 + ns, e.2 + len);
        });
        r
    }

    fn step_op(&self, i: usize, v: &mut Values, inputs: &[&[i64]], params: &[i64], calls: &mut dyn Calls, carried: Option<Vec<i64>>) -> Result<(), &'static str> {
        let a = |k: u16| -> &Vec<i64> { &v.arrays[usize::from(k)] };
        let scalar = |k: u16| -> Result<i64, &'static str> {
            match a(k).as_slice() {
                [x] => Ok(*x),
                _ => Err("a scalar operand is not one element"),
            }
        };
        // Element `j` of an operand, a scalar broadcast.
        let at = |x: &[i64], j: usize| if x.len() == 1 { x[0] } else { x[j] };
        let width = |xs: &[&[i64]]| -> Result<usize, &'static str> {
            let long: Vec<usize> = xs.iter().map(|x| x.len()).filter(|&l| l != 1).collect();
            match long.first() {
                None => Ok(1),
                Some(&l) if long.iter().all(|&m| m == l) => Ok(l),
                _ => Err("operands of unequal lengths"),
            }
        };
        let out: Vec<i64> = match &self.ops[i] {
            ArrayOp::Input(k) => inputs[usize::from(*k)].to_vec(),
            ArrayOp::Param(k) => vec![params[usize::from(*k)]],
            ArrayOp::Const(c) => vec![*c],
            ArrayOp::Iota(x) => (0..a(*x).len() as i64).collect(),
            ArrayOp::Len(x) => vec![a(*x).len() as i64],
            ArrayOp::Map(op, x, y) => {
                let (x, y) = (a(*x), a(*y));
                (0..width(&[x, y])?).map(|j| op.apply(at(x, j), at(y, j))).collect()
            }
            ArrayOp::Select(m, x, y) => {
                let (m, x, y) = (a(*m), a(*x), a(*y));
                (0..width(&[m, x, y])?).map(|j| if at(m, j) != 0 { at(x, j) } else { at(y, j) }).collect()
            }
            ArrayOp::Scan { how, excl, a: x, seg } => {
                let x = a(*x);
                let seg = match seg {
                    Some(s) if a(*s).len() != x.len() => return Err("a scan's flags are not as long as its values"),
                    Some(s) => Some(a(*s)),
                    None => None,
                };
                let mut acc = how.identity();
                let mut out = Vec::with_capacity(x.len());
                for (j, &e) in x.iter().enumerate() {
                    if seg.is_some_and(|s| s[j] != 0) {
                        acc = how.identity();
                    }
                    let next = how.apply(acc, e);
                    out.push(if *excl { acc } else { next });
                    acc = next;
                }
                out
            }
            ArrayOp::Sort(keys) => {
                let keys: Vec<&Vec<i64>> = keys.iter().map(|&k| a(k)).collect();
                let len = keys[0].len();
                if keys.iter().any(|k| k.len() != len) {
                    return Err("a sort's keys of unequal lengths");
                }
                let mut perm: Vec<usize> = (0..len).collect();
                perm.sort_by(|&p, &q| keys.iter().map(|k| k[p].cmp(&k[q])).find(|o| o.is_ne()).unwrap_or(std::cmp::Ordering::Equal));
                perm.into_iter().map(|p| p as i64).collect()
            }
            ArrayOp::Gather(s, idx) => {
                let s = a(*s);
                a(*idx).iter().map(|&j| usize::try_from(j).ok().and_then(|j| s.get(j).copied()).ok_or("a gather's index outside its array")).collect::<Result<_, _>>()?
            }
            ArrayOp::Scatter { how, src, idx, n, init } => {
                let len = usize::try_from(scalar(*n)?).map_err(|_| "a scatter's negative length")?;
                let (src, idx) = (a(*src), a(*idx));
                let w = width(&[src, idx])?;
                let mut out = vec![scalar(*init)?; len];
                let mut written = vec![false; if *how == Combine::Set { len } else { 0 }];
                for j in 0..w {
                    let Some(t) = usize::try_from(at(idx, j)).ok().filter(|&t| t < len) else { continue };
                    if *how == Combine::Set {
                        if std::mem::replace(&mut written[t], true) {
                            return Err("two writes of a set scatter to one element");
                        }
                    }
                    out[t] = how.apply(out[t], at(src, j));
                }
                out
            }
            ArrayOp::Search { le, sorted, q } => {
                let s = a(*sorted);
                if s.windows(2).any(|w| w[0] > w[1]) {
                    return Err("a search of an array not sorted");
                }
                a(*q).iter().map(|&x| s.partition_point(|&e| if *le { e <= x } else { e < x }) as i64).collect()
            }
            ArrayOp::Compact(m) => a(*m).iter().enumerate().filter(|e| *e.1 != 0).map(|e| e.0 as i64).collect(),
            ArrayOp::Expand(c) => {
                let mut out = Vec::new();
                for (j, &k) in a(*c).iter().enumerate() {
                    let k = usize::try_from(k).map_err(|_| "a negative count")?;
                    out.extend(std::iter::repeat_n(j as i64, k));
                }
                out
            }
            ArrayOp::Concat(x, y) => a(*x).iter().chain(a(*y)).copied().collect(),
            ArrayOp::Call { f, args, env } => {
                let fun = &self.funs[usize::from(*f)];
                let xs: Vec<&[i64]> = args.iter().map(|&x| a(x).as_slice()).collect();
                let env: Vec<&[i64]> = env.iter().map(|&x| a(x).as_slice()).collect();
                let w = if xs.is_empty() { 1 } else { width(&xs)? };
                let fields = usize::from(fun.fields);
                let mut counts = Vec::with_capacity(w);
                let mut rows = Vec::new();
                let mut one = Vec::with_capacity(xs.len());
                for j in 0..w {
                    one.clear();
                    one.extend(xs.iter().map(|x| at(x, j)));
                    let before = rows.len();
                    calls.call(*f, fun, &one, &env, &mut rows)?;
                    let made = rows.len() - before;
                    if fields == 0 || made % fields != 0 {
                        if fields == 0 && made == 0 {
                            counts.push(0);
                            continue;
                        }
                        return Err("a call's results are not whole rows");
                    }
                    counts.push((made / fields) as i64);
                }
                v.parts[i] = (0..fields).map(|k| rows.iter().skip(k).step_by(fields).copied().collect()).collect();
                counts
            }
            ArrayOp::Part(c, k) => v.parts[usize::from(*c)][usize::from(*k)].clone(),
            ArrayOp::Carry(init) => carried.unwrap_or_else(|| a(*init).clone()),
        };
        v.arrays[i] = out;
        Ok(())
    }
}
