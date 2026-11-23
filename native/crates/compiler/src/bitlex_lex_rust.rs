//! Tier 1 as Rust, written by the Bitlex compiler from the grammar's lexer statements, in the lean
//! lexer's shape (`rerail_kotlin_lean::lex`), never lowered from the block program.
//!
//! - **Stage 1** (`stage1`), as the lean lexer's `Blocks::of`: one pass over the blocks, 64 bytes
//!   a step, on `rerail_simd8`'s backends. Each `class` is a compare where it is one byte, else
//!   its bits in nibble tables the compiler makes from its bytes (simdjson's `classify`); each
//!   `mask` not made of the regions is its expression's word at the block, from its operands':
//!   `prev` reads the block before, `next` the block after (so it runs a block behind), `fill`
//!   and `escaped` carry. The file's words are kept only of the masks a later stage reads, and
//!   an escape is tested at its block.
//! - **The masks made of the regions**, after them, a word at a time over the file's words.
//! - **The regions** (`regions H { … }`) are walked left to right, as the lean lexer reads a
//!   string or a comment: the first opener not hidden opens its region, which runs to its closer.
//! - **The token loop.** From the first byte the skip mask does not hold, the token whose start
//!   holds there, in the order the tokens are declared, is read to its last byte by its rule
//!   (`run`, `pair`, `read`), as the lean lexer's `token_at`; its code is its kind, its word and
//!   its marks, a gap mark's bit where its mask holds since the grammar token before.
//!
//! Tier 2 (`bitlex_rust`) reads the masks it splits a region by from tier 1's `Masks`, by name
//! (`readers`).

use ::bitlex::syntax::{Arg, Expr, PartLen, Region, Stmt, TokenRule, TokenTest};
use std::collections::HashMap;
use std::fmt::Write;

/// What the generator reads: the lexer's statements and the compiler's tables.
pub struct Input<'a> {
    pub stmts: &'a [Stmt],
    /// Each byte's record kind as a token's first byte (`Records::kinds`).
    pub kinds: &'a [u8],
    /// Each token row's own record kind, where its start gives it, not its first byte (a moved
    /// row, decision 0509): the `Stmt::Token`s in order.
    pub row_kinds: &'a [Option<u8>],
    /// The words a code names: word `i + 1` is `words[i]`.
    pub words: &'a [Vec<u8>],
    /// Each mark's bit in a code, the `Stmt::Mark`s and `Stmt::TokenMark`s in order; none for a
    /// mark tier 1 makes no bit of.
    pub mark_bits: &'a [Option<u8>],
    /// The extras' bits (`Records::extra`): the first every extra's, each later kind's its own.
    pub extra_bits: &'a [(u8, String)],
    /// The marks an extra keeps of its own gap (`Records::own`), by bit.
    pub own: u32,
    /// The code's token marks: each its bit, its mask and the word it needs (0: any).
    pub token_bits: &'a [(u8, CodeMask, u16)],
    /// The grammar's assumptions (decision 0552): its scope (`None` at every token's start, else
    /// in the tokens of a kind), a mask's name, what it reads as there, and whether the prover
    /// proved what tier 1 makes of it.
    pub assumed: &'a [(Option<String>, String, Expr, bool)],
    /// The masks assumed only at a region token's last byte (`assume … at ends`), each with
    /// whether the prover proved it.
    pub assumed_ends: &'a [(String, bool)],
}

/// What the token loop knows of a mask at a byte: never, always, or a test there, as code.
#[derive(Clone)]
enum Known {
    No,
    Yes,
    Test(String),
}

impl Known {
    fn not(self) -> Known {
        match self {
            Known::No => Known::Yes,
            Known::Yes => Known::No,
            Known::Test(t) => Known::Test(format!("!{t}")),
        }
    }

    fn and(self, b: Known) -> Known {
        match (self, b) {
            (Known::No, _) | (_, Known::No) => Known::No,
            (Known::Yes, x) | (x, Known::Yes) => x,
            (Known::Test(a), Known::Test(b)) => Known::Test(format!("({a} && {b})")),
        }
    }

    fn or(self, b: Known) -> Known {
        match (self, b) {
            (Known::Yes, _) | (_, Known::Yes) => Known::Yes,
            (Known::No, x) | (x, Known::No) => x,
            (Known::Test(a), Known::Test(b)) => Known::Test(format!("({a} || {b})")),
        }
    }

    fn xor(self, b: Known) -> Known {
        match (self, b) {
            (Known::No, x) | (x, Known::No) => x,
            (Known::Yes, x) | (x, Known::Yes) => x.not(),
            (Known::Test(a), Known::Test(b)) => Known::Test(format!("({a} != {b})")),
        }
    }

    fn code(&self) -> String {
        match self {
            Known::No => "false".to_owned(),
            Known::Yes => "true".to_owned(),
            Known::Test(t) => t.clone(),
        }
    }
}

/// A fill made of the regions that the token loop keeps, as the lean lexer keeps "a `;` since
/// the last token" (`fill(S, E)`, `S` a mask `X` or `prev(X)`, each the same over each part of a
/// token): its value and `X`'s at each token's last byte (`fill_I`, `xl_I`), from which the loop
/// knows it over the gap after (`fgap_I`) and at the byte before the next token (`fs_I`, `xs_I`).
struct LoopFill {
    /// The mask's name, where it is one; its expression's key.
    name: Option<String>,
    key: String,
    shown: String,
    prev: bool,
    x: Expr,
    /// `X` over the skipped bytes, as stage 1's (none: it holds at none of them).
    xg: Option<String>,
    /// `E` over the skipped bytes, as stage 1's, where it holds at some (none: at none of them).
    eg: Option<String>,
    /// For `fill(F & Y, ~F)`, `F`'s fill: each of `F`'s runs from its first `Y` on.
    within: Option<usize>,
    /// Each row's code for `fill_I` and `xl_I` after its token.
    rows: Vec<(usize, String, String)>,
    /// What each row knows of the fill, for a fill within it.
    info: Vec<RowFill>,
    /// Its start and its stop at a byte of code, as stage 1's; for a fill within one, `Y`.
    start0: Expr,
    stop0: Expr,
}

/// A kept fill over a row's token: at its first byte and its last; whether a start may hold over
/// its bytes after the first (`tail_starts`), and whether a stop does (`tail_stop`); for a region's
/// token, whether the fill is none after its first byte (`dies`).
struct RowFill {
    row: usize,
    at_s: Known,
    at_l: Known,
    tail_starts: bool,
    tail_stop: Known,
    region: bool,
    dies: bool,
}

/// How a region token ends, for what is before a byte outside the regions (`in_region_of`'s
/// shifts and fills): its kind; where its closer is after it, the closer's first bytes (the
/// byte after the token); the bytes its last bytes may be (`last[m]`: `m` bytes before the last).
struct End {
    k: usize,
    /// The closer's first byte, this many before the token's last (`-1`: after it).
    ext: i64,
    after: Option<[bool; 256]>,
    last: [[bool; 256]; 4],
}

/// A region token's last byte `l`, as one of its closers leaves it: the closer's first byte `c`
/// is `ext` bytes before `l` (`-1`: the closer is after the token, `outside`), the walk hides `h`
/// bytes from `c` (none for `outside`), and `closer` holds at `c`; the `body` bytes before `c`
/// are after the token's first.
struct Geo {
    k: usize,
    ext: i64,
    h: i64,
    closer: Expr,
    /// The bytes before `c` known to be after the token's first: its opener's bytes after its
    /// first that are no closer's.
    body: i64,
}

/// A fill the token loop keeps (`fill(prev(T), E)`, `T` assumed at ends): it opens at a region
/// token whose last byte `T` holds at, from the byte after it to the next byte of `E` (`c`), and
/// the escapes it decides, tested where it opens (code over `l` and `c`), at the starts in it (at
/// its first byte, at the others), at `c` where a token starts there, and where a token covers it.
struct Span {
    f: String,
    t: Expr,
    e: Expr,
    /// The rows it opens at (by row), each where, as code over `close` and `l`.
    opens_at: Vec<(usize, String)>,
    /// Where its stop is a region's opener (`o_K`, a template's `}`): the rows that read `K`,
    /// whose token ends it, and its checks over it there (decision 0600). Else none, its end
    /// found where it opens.
    ends_at: Vec<usize>,
    open: Vec<(String, String)>,
    starts: Vec<(String, Option<String>, Option<String>)>,
    at_end: Vec<(String, String)>,
    passed: Vec<(String, String)>,
}

/// A token mark's mask in the code (decision 0407), of the rows' starts (`Stmt::Token`s in
/// order): its bit at a record is the mask's at its first byte, with the word it needs.
#[derive(Clone, Debug)]
pub enum CodeMask {
    /// The first rows' starts, and the second's where there are any: a first element of kinds,
    /// at the grammar's tokens.
    Rows(Vec<usize>, Vec<usize>),
    /// No mask: the word alone.
    WordAlone,
    /// Never: tier 2 tests its tokens alone.
    Zero,
}

/// A region tier 2 reads in parts (`read K | … parts { … }`), and the masks tier 1 gives it, each
/// a field of tier 1's `Masks`.
pub struct Reader {
    /// Its token's kind.
    pub kind: String,
    /// The field its tokens' starts are in: where tier 2 splits a record.
    pub open: String,
    /// The field of its closers' first bytes.
    pub close: String,
    pub regions: Vec<Region>,
    /// Each region's openers' field (`o_K`), where it has several: the region of a record.
    pub region_open: Vec<String>,
    /// Each region's second closer's field (`or C2`).
    pub region_or: Vec<Option<String>>,
    /// Each part: its kind, its start's field, and its bytes or its last's field.
    pub parts: Vec<(String, String, PartLen, Option<String>)>,
    pub content: String,
    /// Each closer of a kind of its own (`closes { K = E bytes N }`): its kind, its field, its bytes.
    pub closes: Vec<(String, String, u8)>,
    /// The fields and the expressions tier 1 makes them of.
    pub masks: Vec<(String, Expr)>,
}

/// A mask's field in `Masks`: a grammar name's own, else the reader's `rd{r}_{role}`. One mask is
/// one field, whichever readers read it: `all` holds each field made so far by its expression,
/// and the reader that makes a field first lists it in `masks`.
fn field_of(e: &Expr, r: usize, role: &str, masks: &mut Vec<(String, Expr)>, all: &mut Vec<(String, String)>) -> String {
    let key = format!("{e:?}");
    if let Some((f, _)) = all.iter().find(|(_, k)| *k == key) {
        return f.clone();
    }
    let name = match e {
        Expr::Name(n) => ident(n),
        _ => format!("rd{r}_{role}"),
    };
    all.push((name.clone(), key));
    masks.push((name.clone(), e.clone()));
    name
}

/// The regions tier 2 reads in parts, in the order their tokens are declared: one a kind.
pub fn readers(stmts: &[Stmt]) -> Vec<Reader> {
    let mut out: Vec<Reader> = Vec::new();
    let mut starts: HashMap<String, Vec<Expr>> = HashMap::new();
    let mut all = Vec::new();
    for s in stmts {
        let Stmt::Token { kind, start, rule: Some(TokenRule::ReadParts(rs, parts)), .. } = s else { continue };
        starts.entry(kind.clone()).or_default().push(start.clone());
        if out.iter().any(|r| r.kind == *kind) {
            continue;
        }
        let r = out.len();
        let mut masks = Vec::new();
        let name = |x: String| Expr::Name(x);
        let or = |a: Expr, b: Expr| Expr::Or(Box::new(a), Box::new(b));
        // Its closers: each region's, outside the others' fills (a `"` in a raw string closes no
        // string), each past its opener's bytes, as the lowering's reader reads them.
        let closer = match rs.as_slice() {
            [one] => one.closers_past(&format!("o_{}", one.kind)),
            _ => rs
                .iter()
                .map(|q| rs.iter().filter(|o| o.kind != q.kind).fold(q.closers_past(&format!("o_{}", q.kind)), |c, o| Expr::And(Box::new(c), Box::new(Expr::Not(Box::new(name(format!("f_{}", o.kind))))))))
                .reduce(or)
                .expect("a region"),
        };
        let close = field_of(&closer, r, "close", &mut masks, &mut all);
        let region_open = if rs.len() > 1 { rs.iter().map(|q| field_of(&name(format!("o_{}", q.kind)), r, "in", &mut masks, &mut all)).collect() } else { Vec::new() };
        let region_or = rs.iter().enumerate().map(|(q, g)| g.or.as_ref().map(|(c2, _)| field_of(c2, r, &format!("or{q}"), &mut masks, &mut all))).collect();
        let parts_out = parts
            .parts
            .iter()
            .enumerate()
            .map(|(i, (p, e, len))| {
                let start = field_of(e, r, &format!("part{i}"), &mut masks, &mut all);
                let last = match len {
                    PartLen::Last(l) => Some(field_of(l, r, &format!("part{i}_last"), &mut masks, &mut all)),
                    PartLen::Bytes(_) => None,
                };
                (p.clone(), start, len.clone(), last)
            })
            .collect();
        let closes = parts.closes.iter().enumerate().map(|(j, (k, e, n))| (k.clone(), field_of(e, r, &format!("closes{j}"), &mut masks, &mut all), *n)).collect();
        out.push(Reader { kind: kind.clone(), open: String::new(), close, regions: rs.clone(), region_open, region_or, parts: parts_out, content: parts.content.clone(), closes, masks });
    }
    for (r, rd) in out.iter_mut().enumerate() {
        let open = starts[&rd.kind].iter().cloned().reduce(|a, b| Expr::Or(Box::new(a), Box::new(b))).expect("a row");
        rd.open = field_of(&open, r, "open", &mut rd.masks, &mut all);
    }
    out
}

/// The record kind of a piece of reader `reader`'s region (`Stmt::Token` rows inside it): its
/// row's own, where its start gives it; `None`, its first byte's.
pub fn piece_kind(stmts: &[Stmt], row_kinds: &[Option<u8>], reader: &str, name: &str, closes: bool) -> Option<u8> {
    let rows = stmts.iter().filter(|s| matches!(s, Stmt::Token { .. }));
    for (i, s) in rows.enumerate() {
        if let Stmt::Token { kind, rule: Some(TokenRule::Inside(_, of, c)), .. } = s
            && kind == name
            && of == reader
            && *c == closes
        {
            return row_kinds.get(i).copied().flatten();
        }
    }
    None
}

/// A name as a Rust identifier.
fn ident(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    const KEYWORDS: &[&str] = &[
        "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen",
    ];
    if KEYWORDS.contains(&s.as_str()) { format!("{s}_") } else { s }
}

/// A byte string's literal.
fn lit(bytes: &[u8]) -> String {
    bytes.iter().map(|&c| if c.is_ascii_alphanumeric() || c == b'_' { (c as char).to_string() } else { format!("\\x{c:02x}") }).collect()
}

/// An expression as the grammar writes it, for a comment.
fn show(e: &Expr) -> String {
    match e {
        Expr::Name(n) => n.clone(),
        Expr::Empty => "empty".to_owned(),
        Expr::Not(a) => format!("~{}", show_in(a)),
        Expr::And(a, b) => format!("{} & {}", show_in(a), show_in(b)),
        Expr::Or(a, b) => format!("{} | {}", show_in(a), show_in(b)),
        Expr::Xor(a, b) => format!("{} ^ {}", show_in(a), show_in(b)),
        Expr::Call(f, args) => {
            let args: Vec<String> = args
                .iter()
                .map(|a| match a {
                    Arg::E(e) => show(e),
                    Arg::N(k) => k.to_string(),
                    Arg::S(s) => format!("{:?}", String::from_utf8_lossy(s)),
                })
                .collect();
            format!("{f}({})", args.join(", "))
        }
        Expr::Test { target, method, .. } => format!("{target}.{method}(…)"),
    }
}

fn show_in(e: &Expr) -> String {
    match e {
        Expr::And(..) | Expr::Or(..) | Expr::Xor(..) => format!("({})", show(e)),
        _ => show(e),
    }
}

/// A mask of stage 1 (`Gen::stage1`), the one pass over the blocks: its word at block `q`, from its
/// operands' words there.
struct Node {
    /// Its locals: the ring of its last words (`r_NAME`), and the file's words (`NAME`) where a later
    /// stage reads them (`kept`).
    name: String,
    /// Its comment: the statement or expression it is.
    what: String,
    /// Its word at block `q`, as code: `lv` is the block's bits in the file, an operand's word its
    /// ring's at `q` (`q + 1` for a `next`).
    code: String,
    /// The blocks it is behind the classes: a `next` reads its operand's block after, so it is one
    /// block behind it.
    lag: u32,
    kept: bool,
    /// Other vectors stage 1 stores its words in: a field of tier 2 the token loop then writes
    /// over the region tokens (`FieldWrite`), as the mask outside them.
    also: Vec<String>,
    /// A carry from block to block of its own (`fill`, `escaped`): `c_NAME`.
    carry: bool,
}

/// A row of the token loop, for `dispatch`: its start, the test build's checks where the loop
/// first reads an assumption, and its code (a block that makes its record).
struct RowArm {
    row: usize,
    /// Its first bytes, and those after its first it may hold.
    first: [bool; 256],
    tail: [bool; 256],
    /// The escapes at a token's start it tests first: each why and test.
    starts: Vec<(String, String)>,
    start: Expr,
    check: String,
    /// Its code up to its record's marks, and the marks it always has.
    body: String,
    bits: u32,
    /// A row whose regions resume a string's (decision 0600): in place of its record, its last
    /// the open record's, and the loop on past it.
    resume: Option<String>,
}

/// `dispatch`'s choices, by the first byte's group: the region opened, and the row.
struct Picks {
    open: String,
    rows: String,
    /// Whether the rows are picked by the openers themselves: no `open`.
    fused: bool,
}

/// A class's nibble-table bit: the high nibbles and the low nibbles it holds (one bit a nibble).
/// The ways a mask may hold (`Gen::cubes`): each the bytes it needs at offsets from where it
/// holds; an offset not named, any byte.
type Cubes = Vec<std::collections::BTreeMap<i64, [bool; 256]>>;
/// The ways each name may hold, as `Gen::cubes` reads it.
type ByteMemo = HashMap<String, Cubes>;

/// At most this many ways a mask may hold are kept apart; more are one, of the bytes any of
/// them needs at each offset all of them name.
const CUBES: usize = 16;

/// A field tier 2 reads in a region's tokens that the token loop writes, as each token's parts
/// know it: its variable, and for each region its value at the token's first byte, over its
/// body's ranges, over its closer (`RegionCheck`'s parts).
struct FieldWrite {
    var: String,
    parts: Vec<FieldPart>,
    /// Its value outside the regions, where it is not none: the field starts as it, and each
    /// region token puts its parts over its bytes.
    outside: Option<String>,
}

/// A field's value over one region's token: at its first byte, over its body's ranges, over its
/// closer, each a mask's code, `!` for all, or none.
struct FieldPart {
    why: String,
    at: Option<String>,
    body: Vec<(i64, Option<i64>, Option<String>)>,
    /// Over the closer (`closer_parts`): for each offset after the token's first (`None`: any
    /// other), the value at each of its bytes.
    closer: Vec<(Option<i64>, Vec<Option<String>>)>,
    /// Where the body is known only by both ends (`body2`): its value at byte `q` after the token's
    /// first and `t` before the closer, `[q - 1][t - 1]` (5 for 5 on), each `0`, `!` or a mask.
    grid: Option<Vec<Vec<String>>>,
}

/// An escape where a region's token reads it: its mask at the token's first byte, over ranges of
/// its body (from the byte `lo` after its first to `hi`, none: to the closer), over its closer.
#[derive(Clone)]
struct RegionCheck {
    why: String,
    at: Option<String>,
    body: Vec<(i64, Option<i64>, String)>,
    closer: Option<String>,
}

/// The bytes a mask is read at and those about it (`in_region_of`): offset `o` from it, -4 to 4,
/// is `self.0[o + 4]`, every byte beyond. It reads as its own byte's set.
#[derive(Clone)]
struct Ctx([[bool; 256]; 9]);

impl Ctx {
    fn all() -> Ctx {
        Ctx([[true; 256]; 9])
    }

    fn of(b: [bool; 256]) -> Ctx {
        let mut c = Ctx::all();
        c.0[4] = b;
        c
    }

    fn at(&self, o: i64) -> [bool; 256] {
        usize::try_from(o + 4).ok().and_then(|i| self.0.get(i)).copied().unwrap_or([true; 256])
    }

    /// Its byte's set narrowed to `b`'s.
    fn and0(&self, b: &[bool; 256]) -> Ctx {
        let mut c = self.clone();
        c.0[4] = std::array::from_fn(|i| c.0[4][i] && b[i]);
        c
    }
}

impl std::ops::Deref for Ctx {
    type Target = [bool; 256];
    fn deref(&self) -> &[bool; 256] {
        &self.0[4]
    }
}

/// The parts of a region token's body (`in_region_of`): its byte `q` (1 to 4) after the token's
/// first (`BODY_AT + q`), and its bytes from 5 on (`BODY_LATE`). Part 1 is any body byte.
const BODY_AT: u8 = 8;
const BODY_LATE: u8 = 5;

fn body_part(q: i64) -> u8 {
    if q >= 5 { BODY_LATE } else { BODY_AT + q as u8 }
}

/// The parts of a region token's closer where it is `d` (1 to 4, or 5 for 5 on) bytes after the
/// token's first: its byte `r` (0 to 3), `CLOSE_AT + 4 (d - 1) + r`. Part 2 is any closer byte,
/// wherever the closer is.
const CLOSE_AT: u8 = 32;

fn close_part(d: i64, r: i64) -> u8 {
    CLOSE_AT + 4 * (d.min(5) - 1) as u8 + r as u8
}

/// Whether part `p` is a byte of the closer.
fn is_closer(p: u8) -> bool {
    p == 2 || (CLOSE_AT..CLOSE_AT + 20).contains(&p)
}

/// The parts of a region token's body by both ends: its byte `q` (1 to 4, or 5 for 5 on) after
/// the token's first and `t` (likewise) before its closer's first, `BODY2_AT + 5 (q - 1) + t - 1`.
const BODY2_AT: u8 = 64;

fn body2(q: i64, t: i64) -> u8 {
    BODY2_AT + 5 * (q.min(5) - 1) as u8 + (t.min(5) - 1) as u8
}

/// Whether `text` reads the variable `var` (not one whose name it starts).
/// The masks made after stage 1 (`body`): its escapes' tests as they are, and the regions' walk and
/// the masks of it in a block only a test build makes, which gives out those `rest` reads (the test
/// build's checks).
fn gate_walk(body: &str, rest: &str) -> String {
    // Its statements, each with the comments before it.
    let mut stmts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i64;
    for line in body.lines() {
        cur.push_str(line);
        cur.push('\n');
        let t = line.trim();
        if t.starts_with("//") {
            continue;
        }
        depth += t.matches('{').count() as i64 - t.matches('}').count() as i64;
        if depth == 0 && (t.ends_with(';') || t.ends_with('}')) {
            stmts.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        stmts.push(cur);
    }
    let code_of = |st: &str| st.lines().map(str::trim).find(|l| !l.starts_with("//")).unwrap_or("").to_owned();
    let (keep, walk): (Vec<String>, Vec<String>) = stmts.into_iter().partition(|st| {
        let c = code_of(st);
        c.starts_with("let mut refused = escapes;") || c.starts_with("refused |=")
    });
    let mut out = keep.concat();
    if walk.is_empty() {
        return out;
    }
    // The names the walk makes that the rest reads.
    let mut names: Vec<String> = Vec::new();
    for st in &walk {
        let c = code_of(st);
        if let Some(r) = c.strip_prefix("let ") {
            let r = r.strip_prefix("mut ").unwrap_or(r);
            let name: String = r.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if !name.is_empty() && (uses(rest, &name) || uses(&out, &name)) && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    let inner: String = walk.concat().lines().map(|l| if l.is_empty() { "\n".to_owned() } else { format!("    {l}\n") }).collect();
    let tuple = match names.as_slice() {
        [] => "()".to_owned(),
        [n] => n.clone(),
        ns => format!("({})", ns.join(", ")),
    };
    let _ = write!(out, "    // The regions' walk and the masks made of it: only a test build's checks read them.\n    #[cfg(feature = \"test-hooks\")]\n    let {tuple} = {{\n{inner}        {tuple}\n    }};\n");
    out
}

fn uses(text: &str, var: &str) -> bool {
    text.match_indices(var).any(|(i, _)| {
        let before = text[..i].chars().next_back().is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        let after = text[i + var.len()..].chars().next().is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        before && after
    })
}

/// Whether `e` reads fill `i` the loop keeps.
fn contains_fill(g: &Gen, e: &Expr, i: usize) -> bool {
    if g.fill_of(e) == Some(i) {
        return true;
    }
    match e {
        Expr::Name(n) => g.regional.contains(n) && g.defs.get(n).is_some_and(|d| contains_fill(g, d, i)),
        Expr::Not(a) => contains_fill(g, a, i),
        Expr::And(a, b) | Expr::Or(a, b) | Expr::Xor(a, b) => contains_fill(g, a, i) || contains_fill(g, b, i),
        _ => false,
    }
}

/// `e` with fill `i` the loop keeps as `to`.
fn subst_fill(g: &Gen, e: &Expr, i: usize, to: &Expr) -> Expr {
    if g.fill_of(e) == Some(i) {
        return to.clone();
    }
    match e {
        Expr::Name(n) if g.regional.contains(n) => match g.defs.get(n) {
            Some(d) if contains_fill(g, d, i) => subst_fill(g, d, i, to),
            _ => e.clone(),
        },
        Expr::Not(a) => Expr::Not(Box::new(subst_fill(g, a, i, to))),
        Expr::And(a, b) => Expr::And(Box::new(subst_fill(g, a, i, to)), Box::new(subst_fill(g, b, i, to))),
        Expr::Or(a, b) => Expr::Or(Box::new(subst_fill(g, a, i, to)), Box::new(subst_fill(g, b, i, to))),
        Expr::Xor(a, b) => Expr::Xor(Box::new(subst_fill(g, a, i, to)), Box::new(subst_fill(g, b, i, to))),
        _ => e.clone(),
    }
}

/// The fills in `e` (not inside another call's arguments but a fill's own).
fn fills_in(e: &Expr, out: &mut Vec<Expr>) {
    match e {
        Expr::Call(f, args) => {
            if f == "fill" {
                out.push(e.clone());
            }
            for a in args {
                if let Arg::E(a) = a {
                    fills_in(a, out);
                }
            }
        }
        Expr::Not(a) => fills_in(a, out),
        Expr::And(a, b) | Expr::Or(a, b) | Expr::Xor(a, b) => {
            fills_in(a, out);
            fills_in(b, out);
        }
        _ => {}
    }
}

fn widen(v: Cubes) -> Cubes {
    if v.len() <= CUBES {
        return v;
    }
    let mut c = v[0].clone();
    c.retain(|d, _| v.iter().all(|w| w.contains_key(d)));
    for w in &v[1..] {
        for (d, b) in c.iter_mut() {
            (0..256).for_each(|i| b[i] |= w[d][i]);
        }
    }
    vec![c]
}

type Rect = (u16, u16);

struct Gen<'a> {
    x: &'a Input<'a>,
    /// The body of `lex` so far, after stage 1: the regions and what reads them, in the
    /// statements' order.
    body: String,
    /// Each mask's local after stage 1, by its grammar name: the regions' and those made of them.
    names: HashMap<String, String>,
    /// Each expression made a local of its own after stage 1, by its text.
    temps: HashMap<String, String>,
    next: usize,
    /// Stage 1's masks, each after its operands.
    nodes: Vec<Node>,
    /// Each stage-1 mask, by its grammar name (`=NAME`) or its expression's text.
    node_of: HashMap<String, usize>,
    /// The names made of the regions (`regions H { … }`): read after stage 1, not in it.
    regional: std::collections::HashSet<String>,
    /// The nibble tables' bits, eight a pair of tables (`LO_P`, `HI_P`), as simdjson's `classify`.
    pairs: Vec<Vec<Rect>>,
    /// The escapes stage 1 tests: a bit of any refuses the file.
    sinks: Vec<(String, usize)>,
    /// The regions (`regions H { … }`) as the token loop opens them at a token's start: each kind,
    /// its opener's local, and the regions it is `after` (by index: a template in one of them is
    /// the one the `after`s ask of, `template`). `None`: none, or one the loop does not
    /// open yet (`else E bytes M`).
    opens: Option<Vec<(String, String, Vec<usize>)>>,
    /// Each mask's expression, by name: what a token reads of one made of the regions.
    defs: HashMap<String, Expr>,
    /// The escapes over a region's bytes, tested where its token reads it (`in_region`), by
    /// region: each its reason and its masks at the start, after it to the closer, and over the
    /// closer.
    region_checks: Vec<Vec<RegionCheck>>,
    /// The escapes over the bytes outside every region token, tested over each token of code's
    /// bytes: each its reason and its mask there.
    code_checks: Vec<(String, String, Expr)>,
    /// Each class's bytes.
    class_bytes: HashMap<String, [bool; 256]>,
    /// The regions (`regions H { … }`) and the name of their hidden bytes.
    kinds: Vec<Region>,
    hidden: String,
    /// The regions' nest (`RegionNest`, decision 0621): its opener's and closer's masks, its
    /// levels, and the name of its `else`.
    nest: Option<(String, String, u8, String)>,
    /// The fills the token loop keeps.
    spans: Vec<Span>,
    /// The bytes the loop skips between tokens: no token starts at one.
    skipped: [bool; 256],
    /// The bytes a token's start is known to be one of (`dispatch`'s group); none: any not skipped.
    start_in: Option<[bool; 256]>,
    /// The fills the loop keeps (`LoopFill`).
    fills: Vec<LoopFill>,
    /// Tier 2's fields the loop writes (`FieldWrite`).
    field_writes: Vec<FieldWrite>,
    /// How each region token may end (`End`), and the bytes its first bytes may be: what a
    /// byte outside the regions may have before and after it in a region.
    ends: Option<Vec<End>>,
    /// The fills shown to hold only in a body's bytes (`fill_kept`), by their key.
    fill_proofs: std::cell::RefCell<HashMap<String, bool>>,
    opener_at: [[bool; 256]; 4],
    /// The escapes tested at each token's start; at the file's end, of a fill the loop keeps.
    start_checks: Vec<(String, Expr, [bool; 256])>,
    /// The escapes tested over each stretch of code between region tokens (`stretch_escape`):
    /// each term's shift and stage 1's mask of it.
    stretch_checks: Vec<(String, Vec<(i64, String)>)>,
    /// The escapes tested over a row's token's bytes after its first: why, the row, the test.
    tail_checks: Vec<(String, usize, String)>,
    /// The escapes made of `code` alone, tested after the loop of the bytes outside its region
    /// tokens (`code_l`, `code_escape`): their masks' lines, then their tests.
    code_after: String,
    end_checks: Vec<(String, usize)>,
}

impl Gen<'_> {
    /// Whether row `row`'s read keeps its closer (`close`): a fill the loop keeps opens after
    /// it, or it resumes a string's record, which a close at its template keeps open.
    fn keeps_close(&self, row: usize, rule: &Option<TokenRule>) -> bool {
        self.spans.iter().any(|sp| sp.opens_at.iter().any(|(r, _)| *r == row)) || matches!(rule, Some(TokenRule::ReadParts(rs, _)) if rs.iter().all(|r| r.resumes))
    }

    /// Whether `e` reads a region's mask (`regional`): it is made after stage 1.
    fn regional(&self, e: &Expr) -> bool {
        match e {
            Expr::Name(n) => self.regional.contains(n),
            Expr::Empty => false,
            Expr::Not(a) => self.regional(a),
            Expr::And(a, b) | Expr::Or(a, b) | Expr::Xor(a, b) => self.regional(a) || self.regional(b),
            Expr::Call(_, args) => args.iter().any(|a| matches!(a, Arg::E(e) if self.regional(e))),
            Expr::Test { .. } => false,
        }
    }

    /// A stage-1 mask of its own, `name`'s, with its word's code and lag.
    fn push_node(&mut self, key: String, name: String, what: String, code: String, lag: u32, carry: bool) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node { name, what, code, lag, kept: false, carry, also: Vec::new() });
        self.node_of.insert(key, id);
        id
    }

    /// Class `name` of bytes `set`: one compare where it is one byte, else its nibble-table bits,
    /// each a product of high nibbles and low nibbles, in the first pair of tables with room.
    fn class(&mut self, name: &str, set: &[bool; 256]) -> usize {
        let bytes: Vec<u8> = (0..=255u8).filter(|&c| set[usize::from(c)]).collect();
        let code = match bytes.as_slice() {
            [] => "0".to_owned(),
            [c] => format!("S::eq_mask(v, {c:#04x}) & lv"),
            _ => {
                // Its bytes by high nibble; the high nibbles with the same low ones are one bit.
                let mut lows = [0u16; 16];
                for &c in &bytes {
                    lows[usize::from(c >> 4)] |= 1 << (c & 15);
                }
                let mut rects: Vec<Rect> = Vec::new();
                for (h, &l) in lows.iter().enumerate() {
                    if l == 0 {
                        continue;
                    }
                    match rects.iter_mut().find(|r| r.1 == l) {
                        Some(r) => r.0 |= 1 << h,
                        None => rects.push((1 << h, l)),
                    }
                }
                let mut tests = Vec::new();
                let mut left = rects;
                while !left.is_empty() {
                    // The first pair with room for all it lacks of them, else a new pair.
                    let fits = |p: &Vec<Rect>, rs: &[Rect]| p.len() + rs.iter().filter(|r| !p.contains(r)).count() <= 8;
                    let p = match self.pairs.iter().position(|p| fits(p, &left)) {
                        Some(p) => p,
                        None => {
                            self.pairs.push(Vec::new());
                            self.pairs.len() - 1
                        }
                    };
                    let mut bits = 0u8;
                    let mut rest = Vec::new();
                    for r in left {
                        let pair = &mut self.pairs[p];
                        match pair.iter().position(|x| *x == r) {
                            Some(b) => bits |= 1 << b,
                            None if pair.len() < 8 => {
                                bits |= 1 << pair.len();
                                pair.push(r);
                            }
                            None => rest.push(r),
                        }
                    }
                    tests.push(format!("S::to_bitmask(S::test(c{p}, S::splat({bits:#04x})))"));
                    left = rest;
                }
                if tests.len() == 1 { format!("{} & lv", tests[0]) } else { format!("({}) & lv", tests.join(" | ")) }
            }
        };
        let local = format!("m_{}", ident(name));
        self.push_node(format!("={name}"), local, format!("class {name}"), code, 0, false)
    }

    /// The one-byte class of `c`, for `text`.
    fn byte_node(&mut self, c: u8) -> usize {
        let key = format!("=byte {c:#04x}");
        if let Some(&id) = self.node_of.get(&key) {
            return id;
        }
        let code = format!("S::eq_mask(v, {c:#04x}) & lv");
        self.push_node(key, format!("b_{c:02x}"), format!("the byte {c:#04x}"), code, 0, false)
    }

    /// Stage 1's mask of `e`, made where it is new.
    fn node(&mut self, e: &Expr) -> Result<usize, String> {
        if let Expr::Name(n) = e {
            return self.node_of.get(&format!("={n}")).copied().ok_or_else(|| format!("tier 1 as Rust: `{n}` is no mask before it"));
        }
        let key = format!("{e:?}");
        if let Some(&id) = self.node_of.get(&key) {
            return Ok(id);
        }
        let name = format!("t{}", self.next);
        self.next += 1;
        // Its comment once its operands are made, to name them.
        let id = self.make_node(key, name, String::new(), e)?;
        self.nodes[id].what = self.brief(e);
        Ok(id)
    }

    /// `e` as its comment: a long one names the masks of stage 1 it reads.
    fn brief(&self, e: &Expr) -> String {
        let full = show(e);
        if full.len() <= 100 {
            return full;
        }
        fn go(g: &Gen, e: &Expr, top: bool) -> String {
            if !top && !matches!(e, Expr::Name(_)) {
                if let Some(&id) = g.node_of.get(&format!("{e:?}")) {
                    return g.nodes[id].name.clone();
                }
            }
            let inner = |a: &Expr| match a {
                Expr::And(..) | Expr::Or(..) | Expr::Xor(..) if g.node_of.get(&format!("{a:?}")).is_none() => format!("({})", go(g, a, false)),
                _ => go(g, a, false),
            };
            match e {
                Expr::Not(a) => format!("~{}", inner(a)),
                Expr::And(a, b) => format!("{} & {}", inner(a), inner(b)),
                Expr::Or(a, b) => format!("{} | {}", inner(a), inner(b)),
                Expr::Xor(a, b) => format!("{} ^ {}", inner(a), inner(b)),
                Expr::Call(f, args) => {
                    let args: Vec<String> = args
                        .iter()
                        .map(|a| match a {
                            Arg::E(e) => go(g, e, false),
                            Arg::N(k) => k.to_string(),
                            Arg::S(s) => format!("{:?}", String::from_utf8_lossy(s)),
                        })
                        .collect();
                    format!("{f}({})", args.join(", "))
                }
                _ => show(e),
            }
        }
        go(self, e, true)
    }

    /// Stage 1's mask `name` of `e`: a carried operation's step, else its word.
    fn make_node(&mut self, key: String, name: String, what: String, e: &Expr) -> Result<usize, String> {
        if let Expr::Call(f, args) = e {
            let ea = |i: usize| match args.get(i) {
                Some(Arg::E(e)) => Ok(e),
                _ => Err(format!("tier 1 as Rust: `{f}`'s argument {i} is no mask")),
            };
            match f.as_str() {
                "fill" => {
                    let (s, x) = (self.node(ea(0)?)?, self.node(ea(1)?)?);
                    let lag = self.nodes[s].lag.max(self.nodes[x].lag);
                    let (s, x) = (self.ring(s, "q"), self.ring(x, "q"));
                    // From each byte of `s` up to, not including, the next of `e`: one add, its carry
                    // the next block's.
                    let code = format!("{{\n        let (s, e) = ({s}, {x});\n        let b = !e | s;\n        let into = s.wrapping_add(b).wrapping_add(c_{name}) ^ s ^ b;\n        let f = s | (!e & into);\n        c_{name} = f >> 63;\n        f & lv\n    }}");
                    return Ok(self.push_node(key, name, what, code, lag, true));
                }
                "escaped" => {
                    let a = self.node(ea(0)?)?;
                    let lag = self.nodes[a].lag;
                    let a = self.ring(a, "q");
                    // simdjson's escaped bytes: each byte after an odd run, one carry bit.
                    let code = format!("{{\n        const ODD: u64 = 0xaaaa_aaaa_aaaa_aaaa;\n        let bs = {a};\n        let potential = bs & !c_{name};\n        let series = ((potential << 1) | ODD).wrapping_sub(potential);\n        let escape_and_terminal = series ^ ODD;\n        let escaped = escape_and_terminal ^ (bs | c_{name});\n        c_{name} = (escape_and_terminal & bs) >> 63;\n        escaped & lv\n    }}");
                    return Ok(self.push_node(key, name, what, code, lag, true));
                }
                "text" => {
                    let Some(Arg::S(t)) = args.first() else { return Err("tier 1 as Rust: `text` needs a string".into()) };
                    // Its first byte here, each later one `k` bytes on.
                    let mut terms = Vec::new();
                    for (k, &c) in t.iter().enumerate() {
                        let b = self.byte_node(c);
                        terms.push(if k == 0 { self.ring(b, "q") } else { self.shifted(b, "next", k as u32) });
                    }
                    let lag = u32::from(t.len() > 1);
                    let code = format!("{} & lv", terms.join(" & "));
                    return Ok(self.push_node(key, name, what, code, lag, false));
                }
                _ => {}
            }
        }
        let (code, lag) = self.pass_word(e)?;
        Ok(self.push_node(key, name, what, code, lag, false))
    }

    /// Mask `id`'s word at block `at`.
    fn ring(&self, id: usize, at: &str) -> String {
        format!("r_{}[{at} % RING]", self.nodes[id].name)
    }

    /// `prev(id, k)` or `next(id, k)`'s word at block `q`: its own shifted, and the bits from the
    /// block before or after.
    fn shifted(&self, id: usize, f: &str, k: u32) -> String {
        if f == "prev" {
            format!("({} << {k} | {} >> {})", self.ring(id, "q"), self.ring(id, "(q + RING - 1)"), 64 - k)
        } else {
            format!("({} >> {k} | {} << {})", self.ring(id, "q"), self.ring(id, "(q + 1)"), 64 - k)
        }
    }

    /// `e`'s word at block `q` in stage 1, and its lag.
    fn pass_word(&mut self, e: &Expr) -> Result<(String, u32), String> {
        Ok(match e {
            Expr::Name(_) => {
                let id = self.node(e)?;
                (self.ring(id, "q"), self.nodes[id].lag)
            }
            Expr::Empty => ("0".to_owned(), 0),
            Expr::Not(a) => {
                let (w, l) = self.pass_word(a)?;
                (if matches!(**a, Expr::And(..)) || matches!(**a, Expr::Not(_)) { format!("!({w}) & lv") } else { format!("!{w} & lv") }, l)
            }
            Expr::And(a, b) => {
                let ((x, l), (y, m)) = (self.pass_word(a)?, self.pass_word(b)?);
                let p = |w: String, e: &Expr| if matches!(e, Expr::Not(_)) { format!("({w})") } else { w };
                (format!("{} & {}", p(x, a), p(y, b)), l.max(m))
            }
            Expr::Or(a, b) => {
                let ((x, l), (y, m)) = (self.pass_word(a)?, self.pass_word(b)?);
                (format!("({x} | {y})"), l.max(m))
            }
            Expr::Xor(a, b) => {
                let ((x, l), (y, m)) = (self.pass_word(a)?, self.pass_word(b)?);
                (format!("({x} ^ {y})"), l.max(m))
            }
            Expr::Call(f, args) if f == "prev" || f == "next" => {
                let Some(Arg::E(a)) = args.first() else { return Err(format!("tier 1 as Rust: `{f}` needs a mask")) };
                let k = match args.get(1) {
                    None => 1,
                    Some(Arg::N(k)) if (1..64).contains(k) => *k as u32,
                    _ => return Err(format!("tier 1 as Rust: `{f}` by 1 to 63 bytes")),
                };
                let id = self.node(a)?;
                let l = self.nodes[id].lag;
                // A `prev` takes bits from the block before, so past the file's end it holds there too.
                if f == "prev" { (format!("{} & lv", self.shifted(id, f, k)), l) } else { (self.shifted(id, f, k), l + 1) }
            }
            Expr::Call(f, _) if matches!(f.as_str(), "fill" | "escaped" | "text") => {
                let id = self.node(e)?;
                (self.ring(id, "q"), self.nodes[id].lag)
            }
            Expr::Call(f, _) => return Err(format!("tier 1 as Rust: `{f}` is not written yet")),
            Expr::Test { .. } => return Err(format!("tier 1 as Rust: `{}` is not written yet", show(e))),
        })
    }

    /// Stage 1, after the rest is written (it knows then which masks the rest reads): the nibble
    /// tables, and one pass over the blocks that makes every mask not made of the regions, a block
    /// at a time, each `lag` blocks behind the classes (`next` reads the block after), keeping
    /// the file's words of those read later and testing the escapes.
    fn stage1(&self) -> String {
        let mut out = String::new();
        for (p, rects) in self.pairs.iter().enumerate() {
            let (mut lo, mut hi) = ([0u8; 16], [0u8; 16]);
            for (b, &(h, l)) in rects.iter().enumerate() {
                for i in 0..16 {
                    if l >> i & 1 == 1 {
                        lo[i] |= 1 << b;
                    }
                    if h >> i & 1 == 1 {
                        hi[i] |= 1 << b;
                    }
                }
            }
            let list = |t: &[u8; 16]| t.iter().map(|b| format!("{b:#04x}")).collect::<Vec<_>>().join(", ");
            let _ = writeln!(out, "/// The classes' nibble tables, pair {p} (simdjson's `classify`): a byte's bit `b` is set in both where it is of bit `b`'s high and low nibbles.");
            let _ = writeln!(out, "static LO_{p}: [u8; 16] = [{}];\nstatic HI_{p}: [u8; 16] = [{}];\n", list(&lo), list(&hi));
        }
        let kept: Vec<&Node> = self.nodes.iter().filter(|n| n.kept).collect();
        out.push_str("/// Stage 1's masks a later stage reads, a bit a byte, and whether an escape holds.\nstruct Stage1 {\n");
        for n in &kept {
            let _ = writeln!(out, "    {}: Vec<u64>,", n.name);
        }
        for a in self.nodes.iter().flat_map(|n| &n.also) {
            let _ = writeln!(out, "    {a}: Vec<u64>,");
        }
        out.push_str("    refused: bool,\n}\n\n");
        let most = self.nodes.iter().map(|n| n.lag).max().unwrap_or(0);
        let ring = (most as usize + 2).next_power_of_two();
        let _ = writeln!(out, "/// Stage 1: one pass over the blocks, 64 bytes a step, as the lean lexer's `Blocks::of`: the classes of block `t`, each mask of block `t - lag` (a `next` reads the block after its operand's), its last {ring} words in a ring.\n#[inline(always)]\nunsafe fn stage1<S: rerail_simd8::Simd8x64>(src: &[u8]) -> Stage1 {{\n    let n = src.len();\n    let nw = n.div_ceil(64);\n    const RING: usize = {ring};");
        for n in &self.nodes {
            let _ = writeln!(out, "    let mut r_{} = [0u64; RING];", n.name);
            if n.carry {
                let _ = writeln!(out, "    let mut c_{} = 0u64;", n.name);
            }
        }
        for n in &kept {
            let _ = writeln!(out, "    let mut {} = vec![0u64; nw];", n.name);
        }
        for a in self.nodes.iter().flat_map(|n| &n.also) {
            let _ = writeln!(out, "    let mut {a} = vec![0u64; nw];");
        }
        let _ = writeln!(out, "    let mut refused = false;\n    for t in 0..nw + {most} {{");
        for lag in 0..=most {
            let ids: Vec<usize> = (0..self.nodes.len()).filter(|&i| self.nodes[i].lag == lag).collect();
            if ids.is_empty() {
                continue;
            }
            let _ = writeln!(out, "        // {} behind the classes.", if lag == 0 { "The classes, and the masks no blocks".to_owned() } else { format!("The masks {lag} block{}", if lag == 1 { "" } else { "s" }) });
            let guard = if lag == 0 { "if t <= nw {\n            let q = t;".to_owned() } else { format!("if t >= {lag} && t - {lag} <= nw {{\n            let q = t - {lag};") };
            let _ = writeln!(out, "        {guard}\n            // The block's bits inside the file (none past its end).\n            let lv = if 64 * q + 64 <= n {{ !0u64 }} else if q < nw {{ (1u64 << (n % 64)) - 1 }} else {{ 0 }};");
            if lag == 0 {
                let _ = writeln!(out, "            let b = if q < nw {{ block(src, q) }} else {{ [0u8; 64] }};\n            let v = unsafe {{ S::load(&b) }};");
                for p in 0..self.pairs.len() {
                    let _ = writeln!(out, "            let c{p} = unsafe {{ S::classify(v, &LO_{p}, &HI_{p}) }};");
                }
            }
            for i in ids {
                let n = &self.nodes[i];
                let code = indent(&n.code, "            ");
                let code = code.trim();
                let unsafe_ = if n.code.contains("S::") { format!("unsafe {{ {code} }}") } else { code.to_owned() };
                let _ = writeln!(out, "            // {}\n            let w = {unsafe_};\n            r_{}[q % RING] = w;", n.what, n.name);
                let stores: Vec<String> = n.kept.then(|| n.name.clone()).into_iter().chain(n.also.iter().cloned()).map(|v| format!("{v}[q] = w;")).collect();
                if !stores.is_empty() {
                    let _ = writeln!(out, "            if q < nw {{\n                {}\n            }}", stores.join("\n                "));
                }
                for (why, _) in self.sinks.iter().filter(|(_, s)| *s == i) {
                    let _ = writeln!(out, "            // {why}\n            refused |= q < nw && w != 0;");
                }
            }
            out.push_str("        }\n");
        }
        out.push_str("    }\n    Stage1 {\n");
        for n in &kept {
            let _ = writeln!(out, "        {},", n.name);
        }
        for a in self.nodes.iter().flat_map(|n| &n.also) {
            let _ = writeln!(out, "        {a},");
        }
        out.push_str("        refused,\n    }\n}\n\nrerail_simd8::dispatch! {\n    fn stage1_best(src: &[u8]) -> Stage1 = stage1, on stage1_on;\n}\n\n");
        out
    }

    /// A local holding `e`'s words: stage 1's (kept), or, made of the regions, its own after them.
    fn val(&mut self, e: &Expr) -> Result<String, String> {
        if !self.regional(e) {
            let id = self.node(e)?;
            self.nodes[id].kept = true;
            return Ok(self.nodes[id].name.clone());
        }
        if let Expr::Name(n) = e {
            return self.names.get(n).cloned().ok_or_else(|| format!("tier 1 as Rust: `{n}` is no mask before it"));
        }
        let key = format!("{e:?}");
        if let Some(t) = self.temps.get(&key) {
            return Ok(t.clone());
        }
        let name = format!("t{}", self.next);
        self.next += 1;
        self.make(&name, e)?;
        self.temps.insert(key, name.clone());
        Ok(name)
    }

    /// `let NAME = e`, a word at a time; a carried operation by its own pass.
    fn make(&mut self, name: &str, e: &Expr) -> Result<(), String> {
        if let Expr::Call(f, args) = e {
            let ea = |i: usize| match args.get(i) {
                Some(Arg::E(e)) => Ok(e),
                _ => Err(format!("tier 1 as Rust: `{f}`'s argument {i} is no mask")),
            };
            match f.as_str() {
                "fill" => {
                    let (s, x) = (self.val(ea(0)?)?, self.val(ea(1)?)?);
                    let _ = writeln!(self.body, "    let {name} = fill(&{s}, &{x});");
                    return Ok(());
                }
                "escaped" => {
                    let a = self.val(ea(0)?)?;
                    let _ = writeln!(self.body, "    let {name} = escaped(&{a});");
                    return Ok(());
                }
                _ => {}
            }
        }
        let w = self.word(e)?;
        let _ = writeln!(self.body, "    let {name}: Vec<u64> = (0..nw).map(|i| {w}).collect();");
        Ok(())
    }

    /// `e`'s word `i`, as code.
    fn word(&mut self, e: &Expr) -> Result<String, String> {
        Ok(match e {
            Expr::Name(_) => format!("{}[i]", self.val(e)?),
            Expr::Empty => "0".to_owned(),
            // `!` binds tighter than `&`: an operand of `&`s is one word in parentheses.
            Expr::Not(a) => match **a {
                Expr::And(..) => format!("!({}) & lv[i]", self.word(a)?),
                _ => format!("!{} & lv[i]", self.paren(a)?),
            },
            Expr::And(a, b) => format!("{} & {}", self.paren(a)?, self.paren(b)?),
            Expr::Or(a, b) => format!("({} | {})", self.word(a)?, self.word(b)?),
            Expr::Xor(a, b) => format!("({} ^ {})", self.word(a)?, self.word(b)?),
            Expr::Call(f, args) if f == "prev" || f == "next" => {
                let Some(Arg::E(a)) = args.first() else { return Err(format!("tier 1 as Rust: `{f}` needs a mask")) };
                let k = match args.get(1) {
                    None => 1,
                    Some(Arg::N(k)) if (1..64).contains(k) => *k,
                    _ => return Err(format!("tier 1 as Rust: `{f}` by 1 to 63 bytes")),
                };
                format!("{f}(&{}, i, {k})", self.val(a)?)
            }
            // A carried operation: its own pass (`make`).
            Expr::Call(f, _) if matches!(f.as_str(), "fill" | "escaped" | "text") => format!("{}[i]", self.val(e)?),
            Expr::Call(f, _) => return Err(format!("tier 1 as Rust: `{f}` is not written yet")),
            Expr::Test { .. } => return Err(format!("tier 1 as Rust: `{}` is not written yet", show(e))),
        })
    }

    fn paren(&mut self, e: &Expr) -> Result<String, String> {
        let w = self.word(e)?;
        Ok(match e {
            Expr::Not(_) => format!("({w})"),
            _ => w,
        })
    }

    /// Whether `e` holds at a token's start `s`, as code, where the token loop knows the regions
    /// there (`opens`): the loop is never inside one, so the hidden bytes are none, and a region's
    /// opener and its fill hold at `s` where it opens there (`open`); a mask made of them, its
    /// expression there. `None`: `e` reads a region's mask elsewhere than at `s`.
    fn at_start(&mut self, e: &Expr) -> Result<Option<String>, String> {
        if !self.regional(e) {
            // A start is never a skipped byte, and is one of `start_in` where that is known: a
            // mask at none of those bytes is none there, one at every one of them is every start.
            let u: [bool; 256] = self.start_in.unwrap_or(std::array::from_fn(|i| !self.skipped[i]));
            let at = self.bytes_at(e, 0, &mut HashMap::new());
            if (0..256).all(|i| !u[i] || !at[i]) {
                return Ok(Some("false".to_owned()));
            }
            if (self.start_in.is_some() || self.skipped.iter().any(|&b| b))
                && let Some(x) = self.exact_bytes(e)
                && (0..256).all(|i| !u[i] || x[i])
            {
                return Ok(Some("true".to_owned()));
            }
            return Ok(Some(format!("bit(&{}, s)", self.val(e)?)));
        }
        // An assumption (decision 0552): the mask at a start is its expression.
        if let Expr::Name(n) = e
            && let Some((_, _, a, _)) = self.x.assumed.iter().find(|(sc, m, _, _)| sc.is_none() && m == n)
        {
            if self.regional(a) {
                return Err(format!("tier 1 as Rust: `assume … {n} = …` reads a region's mask"));
            }
            return self.at_start(a);
        }
        let Some(opens) = &self.opens else { return Ok(None) };
        // Assumed at a region token's last byte: never a start where every region token is two
        // bytes at least (a closer is looked for after the opener; one outside after its body).
        if let Expr::Name(n) = e
            && self.is_end_name(n)
            && self.kinds.iter().all(|r| !r.outside || self.opener_body(r) >= 1)
        {
            return Ok(Some("false".to_owned()));
        }
        Ok(match e {
            Expr::Name(n) => {
                if let Some(k) = opens.iter().position(|(kind, _, _)| *n == format!("o_{kind}") || *n == format!("f_{kind}")) {
                    Some(format!("open == {k}"))
                } else if let Some(d) = self.defs.get(n).cloned() {
                    self.at_start(&d)?
                } else {
                    // The hidden bytes: none at a start.
                    Some("false".to_owned())
                }
            }
            Expr::Empty => Some("false".to_owned()),
            Expr::Not(a) => {
                let n = opens.len();
                self.at_start(a)?.map(|a| match a.as_str() {
                    "false" => "true".to_owned(),
                    "true" => "false".to_owned(),
                    // No region opens here.
                    _ if regions_of(&a).is_some_and(|r| (0..n).all(|k| r.contains(&k))) => "open == usize::MAX".to_owned(),
                    _ => format!("!({a})"),
                })
            }
            Expr::And(a, b) => match (self.at_start(a)?, self.at_start(b)?) {
                (Some(a), _) | (_, Some(a)) if a == "false" => Some(a),
                (Some(a), Some(t)) | (Some(t), Some(a)) if t == "true" => Some(a),
                // One region of several where the region opened is one of them: that one.
                (Some(a), Some(b)) => Some(match (regions_of(&a), regions_of(&b)) {
                    (Some(x), Some(y)) if y.len() == 1 && x.contains(&y[0]) => b,
                    (Some(x), Some(y)) if x.len() == 1 && y.contains(&x[0]) => a,
                    _ => absorb(&a, &b),
                }),
                _ => None,
            },
            Expr::Or(a, b) => match (self.at_start(a)?, self.at_start(b)?) {
                (Some(a), _) | (_, Some(a)) if a == "true" => Some(a),
                (Some(a), Some(f)) | (Some(f), Some(a)) if f == "false" => Some(a),
                (Some(a), Some(b)) => Some(format!("({a} || {b})")),
                _ => None,
            },
            Expr::Xor(a, b) => match (self.at_start(a)?, self.at_start(b)?) {
                (Some(a), Some(b)) => Some(format!("({a} != {b})")),
                _ => None,
            },
            // A fill the loop keeps, or its `X`, at the byte before.
            Expr::Call(f, args) if f == "prev" && args.len() == 1 => match &args[0] {
                Arg::E(a) => match self.fill_of(a) {
                    Some(i) => Some(format!("fs_{i}")),
                    None => self.fill_x_of(a).map(|i| format!("xs_{i}")),
                },
                _ => None,
            },
            Expr::Call(..) | Expr::Test { .. } => None,
        })
    }

    /// The fill the loop keeps that `e` is (its name, or its expression).
    fn fill_of(&self, e: &Expr) -> Option<usize> {
        match e {
            Expr::Name(n) => self.fills.iter().position(|f| f.name.as_deref() == Some(n.as_str())),
            Expr::Call(f, _) if f == "fill" => {
                let key = format!("{e:?}");
                self.fills.iter().position(|f| f.key == key)
            }
            _ => None,
        }
    }

    /// The fill the loop keeps of `prev(X)` whose `X` is `e`.
    fn fill_x_of(&self, e: &Expr) -> Option<usize> {
        let key = format!("{e:?}");
        self.fills.iter().position(|f| f.prev && format!("{:?}", f.x) == key)
    }

    /// Whether `e` is a fill the loop keeps, or one or its `X` a byte before.
    fn loop_read(&self, e: &Expr) -> bool {
        self.fill_of(e).is_some()
            || matches!(e, Expr::Call(f, args) if f == "prev" && args.len() == 1 && matches!(&args[0], Arg::E(a) if self.fill_of(a).is_some() || self.fill_x_of(a).is_some()))
    }

    /// `e` with the fills the loop keeps (and their reads a byte before) as every byte: for the
    /// bytes it may hold at.
    fn fills_as_all(&self, e: &Expr) -> Expr {
        if self.loop_read(e) {
            return Expr::Not(Box::new(Expr::Empty));
        }
        match e {
            Expr::Name(n) if self.regional.contains(n) => match self.defs.get(n) {
                Some(d) if self.reads_fill(d) => self.fills_as_all(d),
                _ => e.clone(),
            },
            Expr::Not(a) => Expr::Not(Box::new(self.fills_as_all(a))),
            Expr::And(a, b) => Expr::And(Box::new(self.fills_as_all(a)), Box::new(self.fills_as_all(b))),
            Expr::Or(a, b) => Expr::Or(Box::new(self.fills_as_all(a)), Box::new(self.fills_as_all(b))),
            Expr::Xor(a, b) => Expr::Xor(Box::new(self.fills_as_all(a)), Box::new(self.fills_as_all(b))),
            _ => e.clone(),
        }
    }

    /// Whether `e` reads a fill the loop keeps.
    fn reads_fill(&self, e: &Expr) -> bool {
        if self.loop_read(e) {
            return true;
        }
        match e {
            Expr::Name(n) => self.regional.contains(n) && self.defs.get(n).is_some_and(|d| self.reads_fill(d)),
            Expr::Not(a) => self.reads_fill(a),
            Expr::And(a, b) | Expr::Or(a, b) | Expr::Xor(a, b) => self.reads_fill(a) || self.reads_fill(b),
            _ => false,
        }
    }

    /// `e` over the gap before a token (`at` to `s - 1`) as code, where it is of stage 1 but for
    /// fills the loop keeps the same over the gap (`fgap_I`), each tested as both. Notes the
    /// `fgap_I` it reads in `used`.
    fn gap_code(&mut self, e: &Expr, used: &mut std::collections::BTreeSet<usize>) -> Result<Option<String>, String> {
        if matches!(e, Expr::Empty) {
            return Ok(Some("false".to_owned()));
        }
        if !self.regional(e) {
            // Every byte of the gap is a skipped one.
            let at = self.bytes_at(e, 0, &mut HashMap::new());
            if !(0..256).any(|i| at[i] && self.skipped[i]) {
                return Ok(Some("false".to_owned()));
            }
            if let Some(x) = self.exact_bytes(e)
                && (0..256).all(|i| !self.skipped[i] || x[i])
            {
                return Ok(Some("s > at".to_owned()));
            }
            return Ok(Some(format!("any_in(&{}, at, s - 1)", self.val(e)?)));
        }
        // `prev(F) & prev(C) & R`, `F` a fill the loop keeps, `C` bytes none skipped, `R` of stage
        // 1: the byte before is a record's, so the byte is the gap's first, and `F` there is its
        // value at the record before's last byte (`fill_i`).
        if let Some(code) = self.gap_after_record(e)? {
            return Ok(Some(code));
        }
        let Some(i) = (0..self.fills.len()).find(|&i| self.fills[i].xg.is_none() && self.fills[i].eg.is_none() && self.fills[i].within.is_none() && contains_fill(self, e, i)) else { return Ok(None) };
        let (one, none) = (simplify_all(subst_fill(self, e, i, &Expr::Not(Box::new(Expr::Empty)))), simplify_all(subst_fill(self, e, i, &Expr::Empty)));
        let (Some(a), Some(b)) = (self.gap_code(&one, used)?, self.gap_code(&none, used)?) else { return Ok(None) };
        used.insert(i);
        let f = Known::Test(format!("fgap_{i}"));
        let k = |c: String| if c == "false" { Known::No } else { Known::Test(c) };
        Ok(Some(f.clone().and(k(a)).or(f.not().and(k(b))).code()))
    }

    /// A gap mask `prev(F) & prev(C) & R` (`gap_code`) at the gap's first byte `at`, or `None`
    /// where `e` is not one.
    fn gap_after_record(&mut self, e: &Expr) -> Result<Option<String>, String> {
        fn conjuncts<'a>(e: &'a Expr, out: &mut Vec<&'a Expr>) {
            match e {
                Expr::And(a, b) => {
                    conjuncts(a, out);
                    conjuncts(b, out);
                }
                _ => out.push(e),
            }
        }
        let mut cs = Vec::new();
        conjuncts(e, &mut cs);
        let prev_of = |c: &Expr| match c {
            Expr::Call(f, args) if f == "prev" && args.len() == 1 => match &args[0] {
                Arg::E(a) => Some(a.clone()),
                _ => None,
            },
            _ => None,
        };
        let (mut fill, mut class, mut rest) = (None, None, Vec::new());
        for c in cs {
            match prev_of(c) {
                Some(a) if fill.is_none() && self.fill_of(&a).is_some() => fill = self.fill_of(&a),
                Some(a) if class.is_none() && self.exact_bytes(&a).is_some_and(|b| (0..256).all(|i| !(b[i] && self.skipped[i]))) => class = Some(a),
                _ => rest.push(c.clone()),
            }
        }
        let (Some(i), Some(c)) = (fill, class) else { return Ok(None) };
        let rest = rest.into_iter().reduce(|a, b| Expr::And(Box::new(a), Box::new(b)));
        if rest.as_ref().is_some_and(|r| self.regional(r)) {
            return Ok(None);
        }
        let r = match rest {
            Some(r) => format!(" && bit(&{}, at)", self.val(&r)?),
            None => String::new(),
        };
        Ok(Some(format!("(s > at && at > 0 && fill_{i} && bit(&{}, at - 1){r})", self.val(&c)?)))
    }

    /// The bytes `e` holds at exactly (a class, or masks of classes), where it is so.
    fn exact_bytes(&self, e: &Expr) -> Option<[bool; 256]> {
        match e {
            Expr::Empty => Some([false; 256]),
            Expr::Name(n) => match self.class_bytes.get(n) {
                Some(b) => Some(*b),
                None if !self.regional.contains(n) => self.defs.get(n).and_then(|d| self.exact_bytes(d)),
                None => None,
            },
            Expr::Not(a) => self.exact_bytes(a).map(|b| b.map(|x| !x)),
            Expr::And(a, b) => {
                let (x, y) = (self.exact_bytes(a)?, self.exact_bytes(b)?);
                Some(std::array::from_fn(|i| x[i] && y[i]))
            }
            Expr::Or(a, b) => {
                let (x, y) = (self.exact_bytes(a)?, self.exact_bytes(b)?);
                Some(std::array::from_fn(|i| x[i] || y[i]))
            }
            _ => None,
        }
    }

    /// What a mask of stage 1 is at the bytes `at` of a token: none where it holds at none of
    /// them, every one where it holds exactly at all of them, else `test`.
    fn known_over(&self, e: &Expr, at: &[bool; 256], test: impl FnOnce() -> Option<String>) -> Option<Known> {
        let b = self.bytes_at(e, 0, &mut HashMap::new());
        if !(0..256).any(|i| at[i] && b[i]) {
            return Some(Known::No);
        }
        if let Some(x) = self.exact_bytes(e)
            && (0..256).all(|i| !at[i] || x[i])
        {
            return Some(Known::Yes);
        }
        test().map(Known::Test)
    }

    /// The fills made of the regions the loop can keep (`LoopFill`): `fill(X, E)` or
    /// `fill(prev(X), E)`, where `X` holds in no region and `E` in none after its start, `E`
    /// at no skipped byte, and each is the same over a token of code's bytes after its start.
    fn find_loop_fills(&mut self, rows: &[(usize, &Stmt)], exprs: &[(Option<String>, Expr)]) -> Result<(), String> {
        let n = self.opens.as_ref().map_or(0, Vec::len);
        let mut seen: Vec<String> = Vec::new();
        for (name, e) in exprs {
            let Expr::Call(f, args) = e else { continue };
            let key = format!("{e:?}");
            if f != "fill" || seen.contains(&key) || !self.regional(e) {
                continue;
            }
            seen.push(key.clone());
            let [Arg::E(start), Arg::E(stop)] = args.as_slice() else { continue };
            let (prev, x) = match start {
                Expr::Call(p, pa) if p == "prev" && pa.len() == 1 => match &pa[0] {
                    Arg::E(a) => (true, a.clone()),
                    _ => continue,
                },
                // `prev(X, j)` is `prev(prev(X, j - 1))`.
                Expr::Call(p, pa) if p == "prev" && pa.len() == 2 => match (&pa[0], &pa[1]) {
                    (Arg::E(a), Arg::N(1)) => (true, a.clone()),
                    (Arg::E(a), Arg::N(2)) => (true, Expr::Call("prev".into(), vec![Arg::E(a.clone())])),
                    (Arg::E(a), Arg::N(j)) => (true, Expr::Call("prev".into(), vec![Arg::E(a.clone()), Arg::N(j - 1)])),
                    _ => continue,
                },
                a => (false, a.clone()),
            };
            if !prev && self.within_fill(&key, name, e, &x, stop, rows)? {
                continue;
            }
            let (Some(x0), Some(e0)) = (self.in_region(&x, None, 0), self.in_region(stop, None, 0)) else { continue };
            if self.regional(&x0) || self.regional(&e0) {
                continue;
            }
            let skipped = self.skipped;
            let Some(xg) = self.in_region_of(&x, None, 0, &Ctx::of(skipped), &mut HashMap::new()) else { continue };
            if self.regional(&xg) {
                continue;
            }
            let eb = self.bytes_at(&e0, 0, &mut HashMap::new());
            let eg = if (0..256).any(|i| eb[i] && skipped[i]) {
                // A stop at a skipped byte: read over the gap, where no start is.
                if !matches!(xg, Expr::Empty) {
                    continue;
                }
                let Some(g) = self.in_region_of(stop, None, 0, &Ctx::of(skipped), &mut HashMap::new()) else { continue };
                if self.regional(&g) {
                    continue;
                }
                Some(self.val(&g)?)
            } else {
                None
            };
            let none = |v: Option<Expr>| matches!(v, Some(Expr::Empty));
            let all = |v: Option<Expr>| matches!(v.map(simplify_all), Some(Expr::Not(b)) if matches!(*b, Expr::Empty));
            if !(0..n).all(|k| (0..3).all(|part| none(self.in_region(&x, Some(k), part)))) {
                continue;
            }
            // In each region, `E` at none of its bytes after the first (the fill stays), or at all
            // of them (it is none after the first).
            let dies: Vec<bool> = (0..n).map(|k| !(1..3).all(|part| none(self.in_region(stop, Some(k), part))) && (0..3).all(|part| all(self.in_region(stop, Some(k), part)))).collect();
            if !(0..n).all(|k| dies[k] || (1..3).all(|part| none(self.in_region(stop, Some(k), part)))) {
                continue;
            }
            let i = self.fills.len();
            let (xv, ev) = (if matches!(x0, Expr::Empty) { None } else { Some(self.val(&x0)?) }, if matches!(e0, Expr::Empty) { None } else { Some(self.val(&e0)?) });
            let xb = self.bytes_at(&x0, 0, &mut HashMap::new());
            let s_at = |xs: Known| if prev { Known::Test(format!("xs_{i}")) } else { xs };
            let carry = Known::Test(format!("fs_{i}"));
            let mut codes = Vec::new();
            let mut info = Vec::new();
            let mut ok = true;
            for (row, st) in rows {
                let Stmt::Token { kind, start, last, rule, .. } = st else { continue };
                if matches!(rule, Some(TokenRule::Inside(..))) {
                    continue;
                }
                // A start is never a skipped byte.
                let sb = self.bytes_at(start, 0, &mut HashMap::new());
                let sb: [bool; 256] = std::array::from_fn(|i| sb[i] && !self.skipped[i]);
                let regions: Vec<(Known, usize)> = match rule {
                    Some(TokenRule::Read(rs)) => rs.iter().filter_map(|r| self.kinds.iter().position(|q| q.kind == r.kind).map(|k| (if rs.len() > 1 { Known::Test(format!("open == {k}")) } else { Known::Yes }, k))).collect(),
                    Some(TokenRule::ReadParts(rs, _)) => match Self::row_region(start, rs).and_then(|r| self.kinds.iter().position(|q| q.kind == r.kind)) {
                        Some(k) => vec![(Known::Yes, k)],
                        None => {
                            ok = false;
                            break;
                        }
                    },
                    _ => Vec::new(),
                };
                let (fl, xl) = if !regions.is_empty() {
                    // A region token: `X` at none of its bytes, `E` at its start only, or at all of
                    // them.
                    let mut v = Known::No;
                    let mut v_s = Known::No;
                    let row_dies = regions.iter().all(|r| dies[r.1]);
                    for (cond, k) in regions {
                        // At its opener's byte.
                        let ob = self.bytes_at(&self.kinds[k].opener.clone(), 0, &mut HashMap::new());
                        let sb: [bool; 256] = std::array::from_fn(|i| sb[i] && ob[i]);
                        let Some(es) = self.in_region(stop, Some(k), 0) else {
                            ok = false;
                            break;
                        };
                        let es = if self.regional(&es) {
                            None
                        } else {
                            let ev = if matches!(es, Expr::Empty) { String::new() } else { self.val(&es)? };
                            self.known_over(&es, &sb, || Some(format!("bit(&{ev}, s)")))
                        };
                        let Some(es) = es else {
                            ok = false;
                            break;
                        };
                        let fk = s_at(Known::No).or(es.not().and(carry.clone()));
                        v_s = v_s.or(cond.clone().and(fk.clone()));
                        v = v.or(cond.and(if !dies[k] {
                            fk
                        } else if self.read_never_one_byte(rule, k) {
                            Known::No
                        } else {
                            fk.and(Known::Test("l == s".to_owned()))
                        }));
                    }
                    info.push(RowFill { row: *row, at_s: v_s, at_l: v.clone(), tail_starts: false, tail_stop: Known::No, region: true, dies: row_dies });
                    (v, Known::No)
                } else {
                    // A token of code: `X` and `E` stage 1's over its bytes.
                    let tb = self.row_tail_bytes(kind, rule, last);
                    let has_tail = (0..256).any(|i| tb[i]);
                    let bit = |v: &Option<String>, at: &str| v.as_ref().map(|v| format!("bit(&{v}, {at})"));
                    let (Some(es), Some(xs)) = (self.known_over(&e0, &sb, || bit(&ev, "s")), self.known_over(&x0, &sb, || bit(&xv, "s"))) else {
                        ok = false;
                        break;
                    };
                    let fs = s_at(xs.clone()).or(es.not().and(carry.clone()));
                    if !has_tail {
                        info.push(RowFill { row: *row, at_s: fs.clone(), at_l: fs.clone(), tail_starts: false, tail_stop: Known::No, region: false, dies: false });
                        (fs, xs)
                    } else {
                        let Some(et) = self.known_over(&e0, &tb, || ev.as_ref().map(|v| format!("any_in(&{v}, s + 1, l)"))) else {
                            ok = false;
                            break;
                        };
                        let all: [bool; 256] = std::array::from_fn(|i| sb[i] || tb[i]);
                        let over = |b: &[bool; 256], code: String| if (0..256).any(|i| b[i] && xb[i]) { Known::Test(code) } else { Known::No };
                        let xv_ = xv.clone().unwrap_or_default();
                        let xt = over(&tb, format!("bit(&{xv_}, l)"));
                        let ft = match (&et, prev) {
                            (Known::Yes, true) => over(&all, format!("bit(&{xv_}, l - 1)")),
                            (Known::Yes, false) => xt.clone(),
                            (Known::No, true) => fs.clone().or(over(&all, format!("any_in(&{xv_}, s, l - 1)"))),
                            (Known::No, false) => fs.clone().or(over(&tb, format!("any_in(&{xv_}, s + 1, l)"))),
                            // A stop over some of its bytes: from the last, or the first byte's.
                            (Known::Test(_), _) => {
                                let ev_ = ev.clone().unwrap_or_default();
                                let (after, none) = if prev {
                                    (over(&all, format!("any_in(&{xv_}, q - 1, l - 1)")), fs.clone().or(over(&all, format!("any_in(&{xv_}, s, l - 1)"))))
                                } else {
                                    (over(&tb, format!("any_in(&{xv_}, q, l)")), fs.clone().or(over(&tb, format!("any_in(&{xv_}, s + 1, l)"))))
                                };
                                Known::Test(format!("match last_in(&{ev_}, s + 1, l) {{ Some(q) => {}, None => {} }}", after.code(), none.code()))
                            }
                        };
                        let tail_starts = (0..256).any(|i| (if prev { all[i] } else { tb[i] }) && xb[i]);
                        let pick = |a: Known, b: Known| {
                            let (a, b) = (a.code(), b.code());
                            if a == b { Known::Test(a) } else { Known::Test(format!("if l > s {{ {a} }} else {{ {b} }}")) }
                        };
                        let fl = pick(ft, fs.clone());
                        info.push(RowFill { row: *row, at_s: fs, at_l: fl.clone(), tail_starts, tail_stop: et, region: false, dies: false });
                        (fl, pick(xt, xs))
                    }
                };
                codes.push((*row, fl.code(), xl.code()));
            }
            if !ok {
                continue;
            }
            let xg = if matches!(xg, Expr::Empty) { None } else { Some(self.val(&xg)?) };
            let start0 = if prev { Expr::Call("prev".into(), vec![Arg::E(x0.clone())]) } else { x0.clone() };
            self.fills.push(LoopFill { name: name.clone(), key, shown: show(e), prev, x, xg, eg, within: None, rows: codes, info, start0, stop0: e0.clone() });
        }
        Ok(())
    }

    /// `fill(F & Y, ~F [| Z])` for a fill `F` the loop keeps and `Y`, `Z` of stage 1: each of
    /// `F`'s runs from its first `Y`, to a `Z`. It is none where `F` is, so over a gap it is its
    /// value at the token before's last byte while `F` holds; at a token's first byte `F & (Y |
    /// before & ~Z)`; over the token's bytes after, where no `F` starts and no `Z` is, `F` at its
    /// last byte and `Y` over them or its first byte's. `Z` holds at no skipped byte.
    fn within_fill(&mut self, key: &str, name: &Option<String>, e: &Expr, x: &Expr, stop: &Expr, rows: &[(usize, &Stmt)]) -> Result<bool, String> {
        // The stop's terms: `~F`, and the others `Z`.
        let mut terms = Vec::new();
        let mut todo = vec![stop];
        while let Some(t) = todo.pop() {
            match t {
                Expr::Or(a, b) => {
                    todo.push(b);
                    todo.push(a);
                }
                t => terms.push(t),
            }
        }
        let Some(fi) = terms.iter().position(|t| matches!(t, Expr::Not(f) if self.fill_of(f).is_some())) else { return Ok(false) };
        let Expr::Not(f) = terms.remove(fi) else { return Ok(false) };
        let Some(j) = self.fill_of(f) else { return Ok(false) };
        let z = terms.into_iter().cloned().reduce(|a, b| Expr::Or(Box::new(a), Box::new(b)));
        if z.as_ref().is_some_and(|z| self.regional(z)) {
            return Ok(false);
        }
        let zb = z.as_ref().map_or([false; 256], |z| self.bytes_at(z, 0, &mut HashMap::new()));
        if (0..256).any(|b| zb[b] && self.skipped[b]) {
            return Ok(false);
        }
        let zv = match &z {
            Some(z) => Some(self.val(z)?),
            None => None,
        };
        let y = match x {
            Expr::And(a, b) if self.fill_of(a) == Some(j) => b,
            Expr::And(a, b) if self.fill_of(b) == Some(j) => a,
            _ => return Ok(false),
        };
        if self.regional(y) || self.fills[j].xg.is_some() {
            return Ok(false);
        }
        let yb = self.bytes_at(y, 0, &mut HashMap::new());
        if (0..256).any(|b| yb[b] && self.skipped[b]) {
            return Ok(false);
        }
        let yv = if matches!(**y, Expr::Empty) { None } else { Some(self.val(y)?) };
        let i = self.fills.len();
        let carry = Known::Test(format!("fs_{i}"));
        let mut codes = Vec::new();
        let mut info = Vec::new();
        for (row, st) in rows {
            let Stmt::Token { kind, start, last, rule, .. } = st else { continue };
            if matches!(rule, Some(TokenRule::Inside(..))) {
                continue;
            }
            let Some(fr) = self.fills[j].info.iter().find(|r| r.row == *row) else { return Ok(false) };
            let (f_s, f_l, region, dies, tail_starts, tail_stop) = (fr.at_s.clone(), fr.at_l.clone(), fr.region, fr.dies, fr.tail_starts, fr.tail_stop.clone());
            let sb = self.bytes_at(start, 0, &mut HashMap::new());
            let sb: [bool; 256] = std::array::from_fn(|b| sb[b] && !self.skipped[b]);
            let bit = |at: &str| yv.as_ref().map(|v| format!("bit(&{v}, {at})"));
            let Some(ys) = self.known_over(y, &sb, || bit("s")) else { return Ok(false) };
            // The stop `Z` at the first byte, and none over the bytes after.
            let zs = match &z {
                Some(z) => match self.known_over(z, &sb, || zv.as_ref().map(|v| format!("bit(&{v}, s)"))) {
                    Some(k) => k,
                    None => return Ok(false),
                },
                None => Known::No,
            };
            // `Z` over the bytes after the first (a `->`'s `>`, a `..<`'s `<`): tested there. A
            // `Y` there comes after every `Z` there, and starts the fill again.
            let z_tail = match (&z, region) {
                (Some(z), false) => {
                    let tb = self.row_tail_bytes(kind, rule, last);
                    match self.known_over(z, &tb, || zv.as_ref().map(|v| format!("any_in(&{v}, s + 1, l)"))) {
                        Some(Known::No) => None,
                        Some(k) => {
                            let yt = self.known_over(y, &tb, || yv.as_ref().map(|v| format!("any_in(&{v}, s + 1, l)"))).unwrap_or(Known::No);
                            Some((k, yt))
                        }
                        None => return Ok(false),
                    }
                }
                _ => None,
            };
            let g_s = f_s.clone().and(ys.or(carry.clone().and(zs.not())));
            let g_l = if region {
                if !dies {
                    return Ok(false);
                }
                let k = match rule {
                    Some(TokenRule::Read(rs)) if rs.len() == 1 => self.kinds.iter().position(|q| q.kind == rs[0].kind),
                    Some(TokenRule::ReadParts(rs, _)) => Self::row_region(start, rs).and_then(|r| self.kinds.iter().position(|q| q.kind == r.kind)),
                    _ => None,
                };
                if k.is_some_and(|k| self.read_never_one_byte(rule, k)) { Known::No } else { g_s.clone().and(Known::Test("l == s".to_owned())) }
            } else {
                let tb = self.row_tail_bytes(kind, rule, last);
                if !(0..256).any(|b| tb[b]) {
                    g_s.clone()
                } else if !tail_starts {
                    let Some(yt) = self.known_over(y, &tb, || yv.as_ref().map(|v| format!("any_in(&{v}, s + 1, l)"))) else { return Ok(false) };
                    f_l.clone().and(g_s.clone().or(yt))
                } else if matches!(self.known_over(y, &tb, || None), Some(Known::No)) {
                    // A start of `F` may be over them, and no `Y`: the first byte's while `F` holds,
                    // that is while no byte stops it but starts it.
                    if matches!(tail_stop, Known::No) {
                        g_s.clone()
                    } else {
                        let kill = Expr::And(Box::new(self.fills[j].stop0.clone()), Box::new(Expr::Not(Box::new(self.fills[j].start0.clone()))));
                        let kill = self.val(&kill)?;
                        g_s.clone().and(Known::Test(format!("!any_in(&{kill}, s + 1, l)")))
                    }
                } else {
                    return Ok(false);
                }
            };
            let g_l = match z_tail {
                Some((zt, yt)) => g_l.and(zt.not()).or(f_l.clone().and(yt)),
                None => g_l,
            };
            info.push(RowFill { row: *row, at_s: g_s, at_l: g_l.clone(), tail_starts: false, tail_stop: Known::No, region, dies });
            codes.push((*row, g_l.code(), Known::No.code()));
        }
        self.fills.push(LoopFill { name: name.clone(), key: key.to_owned(), shown: show(e), prev: false, x: x.clone(), xg: None, eg: None, within: Some(j), rows: codes, info, start0: (**y).clone(), stop0: z.unwrap_or(Expr::Empty) });
        Ok(true)
    }

    /// `e` in one part of a token that reads region `k` (decision 0551's step B): at its start
    /// (`part` 0), after it to the closer (1), over the closer (2); and outside every region token
    /// (`k` none). There the regions' masks are known: a region's opener holds at its start, its
    /// fill from there to the closer, the hidden bytes after its start through its closer, none
    /// of them outside. `None`: `e` reads one of them shifted or carried.
    fn in_region(&self, e: &Expr, k: Option<usize>, part: u8) -> Option<Expr> {
        let v = self.in_region_of(e, k, part, &Ctx::all(), &mut HashMap::new());
        // The body where it is one mask over its bytes at each offset.
        if v.is_none() && part == 1 && k.is_some() {
            let k = k?;
            let parts = self.body_parts(e, k)?;
            // From 5 on; at an offset before, the same, or none where it holds at none of the
            // opener's bytes there.
            let late = parts.last()?.clone();
            let key = format!("{late:?}");
            if parts.iter().enumerate().all(|(i, p)| format!("{p:?}") == key || matches!(p, Expr::Empty) && !self.fits_opener(&late, k, i as i64 + 1)) {
                return Some(late);
            }
        }
        v
    }

    /// Whether a way `e` may hold (its `cubes`) fits the bytes of a token of region `k`'s opener,
    /// at byte `q` after the token's first.
    fn fits_opener(&self, e: &Expr, k: usize, q: i64) -> bool {
        let Some(r) = self.kinds.get(k) else { return true };
        let mut memo = HashMap::new();
        let at: Vec<[bool; 256]> = (0..4).map(|d| self.bytes_at(&r.opener, d, &mut memo)).collect();
        self.cubes(e, &mut memo).iter().any(|c| c.iter().all(|(o, b)| usize::try_from(q + o).ok().and_then(|d| at.get(d)).is_none_or(|x| (0..256).any(|i| x[i] && b[i]))))
    }

    /// Whether a way `e` may hold fits the bytes of region `k`'s closer, at its byte `rr`.
    fn fits_closer(&self, e: &Expr, k: usize, rr: i64) -> bool {
        let Some(r) = self.kinds.get(k) else { return true };
        let mut memo = HashMap::new();
        let closers = r.closers();
        let at: Vec<[bool; 256]> = (0..4).map(|d| self.bytes_at(&closers, d, &mut memo)).collect();
        self.cubes(e, &mut memo).iter().any(|c| c.iter().all(|(o, b)| usize::try_from(rr + o).ok().and_then(|d| at.get(d)).is_none_or(|x| (0..256).any(|i| x[i] && b[i]))))
    }

    /// The classes a mask of stage 1 reads, through the masks it names.
    fn classes_read(&self, e: &Expr, out: &mut std::collections::BTreeSet<String>) {
        match e {
            Expr::Name(n) if self.class_bytes.contains_key(n) => {
                out.insert(n.clone());
            }
            Expr::Name(n) => {
                if let Some(d) = self.defs.get(n) {
                    self.classes_read(d, out);
                }
            }
            Expr::Not(a) => self.classes_read(a, out),
            Expr::And(a, b) | Expr::Or(a, b) | Expr::Xor(a, b) => {
                self.classes_read(a, out);
                self.classes_read(b, out);
            }
            Expr::Call(_, args) => {
                for a in args {
                    if let Arg::E(x) = a {
                        self.classes_read(x, out);
                    }
                }
            }
            Expr::Empty | Expr::Test { .. } => {}
        }
    }

    /// The bytes' atoms: one byte of each set of bytes every class holds alike at.
    fn atoms(&self) -> Vec<u8> {
        let mut seen: HashMap<Vec<bool>, u8> = HashMap::new();
        let mut out = Vec::new();
        for b in 0..=255u8 {
            let sig: Vec<bool> = self.class_bytes.values().map(|c| c[usize::from(b)]).collect();
            if !seen.contains_key(&sig) {
                seen.insert(sig, b);
                out.push(b);
            }
        }
        out
    }

    /// The offsets from a byte a mask of stage 1 reads, made of classes, shifts and logic; `None`
    /// for any other.
    fn window(&self, e: &Expr, at: i64, out: &mut std::collections::BTreeSet<i64>) -> Option<()> {
        match e {
            Expr::Empty => Some(()),
            Expr::Name(n) if self.class_bytes.contains_key(n) => {
                out.insert(at);
                Some(())
            }
            Expr::Name(n) if !self.regional.contains(n) => self.window(self.defs.get(n)?, at, out),
            Expr::Not(a) => self.window(a, at, out),
            Expr::And(a, b) | Expr::Or(a, b) | Expr::Xor(a, b) => {
                self.window(a, at, out)?;
                self.window(b, at, out)
            }
            Expr::Call(f, args) if f == "prev" || f == "next" => {
                let (Arg::E(a), j) = (args.first()?, match args.get(1) {
                    None => 1,
                    Some(Arg::N(j)) => *j as i64,
                    _ => return None,
                }) else { return None };
                self.window(a, if f == "prev" { at - j } else { at + j }, out)
            }
            _ => None,
        }
    }

    /// A mask of `window`'s kind at the byte `at`, the bytes about it `bytes`'.
    fn eval_at(&self, e: &Expr, at: i64, bytes: &HashMap<i64, u8>) -> Option<bool> {
        Some(match e {
            Expr::Empty => false,
            Expr::Name(n) if self.class_bytes.contains_key(n) => self.class_bytes[n][usize::from(*bytes.get(&at)?)],
            Expr::Name(n) if !self.regional.contains(n) => self.eval_at(self.defs.get(n)?, at, bytes)?,
            Expr::Not(a) => !self.eval_at(a, at, bytes)?,
            Expr::And(a, b) => self.eval_at(a, at, bytes)? && self.eval_at(b, at, bytes)?,
            Expr::Or(a, b) => self.eval_at(a, at, bytes)? || self.eval_at(b, at, bytes)?,
            Expr::Xor(a, b) => self.eval_at(a, at, bytes)? != self.eval_at(b, at, bytes)?,
            Expr::Call(f, args) if f == "prev" || f == "next" => {
                let j = match args.get(1) {
                    None => 1,
                    Some(Arg::N(j)) => *j as i64,
                    _ => return None,
                };
                let Some(Arg::E(a)) = args.first() else { return None };
                self.eval_at(a, if f == "prev" { at - j } else { at + j }, bytes)?
            }
            _ => return None,
        })
    }

    /// Whether `e` holds at each byte of `here` after each of `before`, whatever the bytes about
    /// them (over the atoms); `None` where not known.
    fn holds_after(&self, e: &Expr, before: &[bool; 256], here: &[bool; 256], atoms: &[u8]) -> Option<bool> {
        let mut offs = std::collections::BTreeSet::new();
        self.window(e, 0, &mut offs)?;
        let rest: Vec<i64> = offs.into_iter().filter(|o| *o != 0 && *o != -1).collect();
        if rest.len() > 3 {
            return None;
        }
        // `e` tells bytes apart only by the classes it reads: one byte of each of its atoms
        // in each set (of `atoms`, every class's), not each of every class's.
        let mut names = std::collections::BTreeSet::new();
        self.classes_read(e, &mut names);
        let sig = |b: u8| names.iter().map(|n| self.class_bytes[n][usize::from(b)]).collect::<Vec<bool>>();
        let pick = |set: &[bool; 256]| {
            let mut seen = std::collections::HashSet::new();
            atoms.iter().copied().filter(|&a| set[usize::from(a)] && seen.insert(sig(a))).collect::<Vec<u8>>()
        };
        let (ys, xs) = (pick(before), pick(here));
        let atoms = &pick(&[true; 256]);
        for &y in &ys {
            for &x in &xs {
                let mut n = vec![0usize; rest.len()];
                loop {
                    let mut bytes: HashMap<i64, u8> = HashMap::from([(-1, y), (0, x)]);
                    for (o, &i) in rest.iter().zip(&n) {
                        bytes.insert(*o, atoms[i]);
                    }
                    if !self.eval_at(e, 0, &bytes)? {
                        return Some(false);
                    }
                    // The next of the bytes about them.
                    let Some(i) = n.iter().position(|&i| i + 1 < atoms.len()) else { break };
                    n[i] += 1;
                    n[..i].iter_mut().for_each(|v| *v = 0);
                }
            }
        }
        Some(true)
    }

    /// Whether `fill(st, stop)` holds only at a body's bytes, and over a body is `fill(S, E)` of
    /// their values there. This holds where:
    /// - `st` holds at no byte outside the regions, at no token's first byte, at none of a
    ///   closer;
    /// - in each body where `st` holds (`S`, `E` its values there), the run stays in a set of
    ///   bytes `W`: `S`'s, and each byte at which `E` may not hold after one of `W`;
    /// - `E` at the closer's first byte holds after each byte of `W`, so no run leaves a body;
    /// - the opener's first byte is not one of `W`, so `fill(S, E)` over the file is none there,
    ///   and the two agree over the body.
    fn fill_kept(&self, st: &Expr, stop: &Expr) -> bool {
        let key = format!("{st:?} {stop:?}");
        if let Some(&v) = self.fill_proofs.borrow().get(&key) {
            return v;
        }
        // A cycle says no.
        self.fill_proofs.borrow_mut().insert(key.clone(), false);
        let v = self.fill_kept_of(st, stop).is_some();
        self.fill_proofs.borrow_mut().insert(key, v);
        v
    }

    fn fill_kept_of(&self, st: &Expr, stop: &Expr) -> Option<()> {
        let none = |v: Option<Expr>| matches!(v, Some(Expr::Empty));
        if !none(self.in_region(st, None, 0)) {
            return None;
        }
        let atoms = self.atoms();
        let mut memo = HashMap::new();
        for (k, r) in self.kinds.iter().enumerate() {
            if !none(self.in_region(st, Some(k), 0)) || !self.closer_parts(st, k)?.iter().all(|(_, v)| v.iter().all(|v| matches!(v, Expr::Empty))) {
                return None;
            }
            let s1 = self.in_region(st, Some(k), 1).filter(|v| !self.regional(v))?;
            if matches!(s1, Expr::Empty) {
                continue;
            }
            let e1 = self.in_region(stop, Some(k), 1).filter(|v| !self.regional(v))?;
            // The run's bytes.
            let mut w = self.bytes_at(&s1, 0, &mut memo);
            loop {
                let mut grew = false;
                for &a in &atoms {
                    if w[usize::from(a)] {
                        continue;
                    }
                    let one: [bool; 256] = std::array::from_fn(|i| i == usize::from(a));
                    if !self.holds_after(&e1, &w, &one, &atoms)? {
                        // Every byte of its atom.
                        let sig = |b: usize| self.class_bytes.values().map(|c| c[b]).collect::<Vec<bool>>();
                        let sa = sig(usize::from(a));
                        (0..256).filter(|&b| sig(b) == sa).for_each(|b| w[b] = true);
                        grew = true;
                    }
                }
                if !grew {
                    break;
                }
            }
            let ob = self.bytes_at(&r.opener, 0, &mut memo);
            if (0..256).any(|i| w[i] && ob[i]) {
                return None;
            }
            let cb = self.bytes_at(&r.closers(), 0, &mut memo);
            for (_, v) in self.closer_parts(stop, k)? {
                let e2 = v.first()?;
                if self.regional(e2) || !self.holds_after(e2, &w, &cb, &atoms)? {
                    return None;
                }
            }
        }
        Some(())
    }

    /// `e` over a token of region `k`'s closer: where the same at each of its bytes wherever it is,
    /// one value (`None` offset); else for each byte `r` of it (to its most bytes), where the closer
    /// is each offset `d` after the token's first that differs from 5 on's, and from 5 on (`None`).
    fn closer_parts(&self, e: &Expr, k: usize) -> Option<Vec<(Option<i64>, Vec<Expr>)>> {
        let r = self.kinds.get(k)?;
        let most = i64::from(r.bytes.max(r.or.as_ref().map_or(0, |o| o.1))).clamp(1, 4);
        if let Some(v) = self.in_region(e, Some(k), 2) {
            // None where it holds at none of the closer's bytes.
            let v = if (0..most).any(|rr| self.fits_closer(&v, k, rr)) { v } else { Expr::Empty };
            return Some(vec![(None, vec![v])]);
        }
        let at = |d: i64| (0..most).map(|rr| self.in_region_of(e, Some(k), close_part(d, rr), &Ctx::all(), &mut HashMap::new()).map(|v| if self.fits_closer(&v, k, rr) { v } else { Expr::Empty })).collect::<Option<Vec<_>>>();
        let late = at(5)?;
        let key = |v: &[Expr]| format!("{v:?}");
        let mut out = Vec::new();
        for d in 1..=4 {
            let v = at(d)?;
            if key(&v) != key(&late) {
                out.push((Some(d), v));
            }
        }
        out.push((None, late));
        Some(out)
    }

    /// `e` in a token of region `k`'s body: at its bytes 1 to 4 after the token's first, and from
    /// its byte 5 on.
    fn body_parts(&self, e: &Expr, k: usize) -> Option<Vec<Expr>> {
        (1..=4).map(|q| body_part(q)).chain([BODY_LATE]).map(|p| self.in_region_of(e, Some(k), p, &Ctx::all(), &mut HashMap::new())).collect()
    }

    /// `in_region` where the byte is one of `ctx` (the bytes the rest of a `&` holds at): a mask
    /// that holds at none of them is none there.
    fn in_region_of(&self, e: &Expr, k: Option<usize>, part: u8, ctx: &Ctx, memo: &mut ByteMemo) -> Option<Expr> {
        let all = || Expr::Not(Box::new(Expr::Empty));
        let opens = self.opens.as_ref()?;
        // A region's start is its opener's first byte.
        let start_ctx: Ctx;
        let ctx = match (k, part) {
            (Some(k), 0) if k < self.kinds.len() => {
                let ob = self.bytes_at(&self.kinds[k].opener, 0, memo);
                start_ctx = ctx.and0(&ob);
                &start_ctx
            }
            _ => ctx,
        };
        let here = self.bytes_at(e, 0, memo);
        if !(0..256).any(|i| here[i] && ctx[i]) {
            return Some(Expr::Empty);
        }
        // A fill the loop keeps, outside the regions: itself.
        if k.is_none() && self.loop_read(e) {
            return Some(e.clone());
        }
        // Outside the regions, a shift or a fill of masks made of them: of their values outside,
        // where what is in a region before or after the byte can not change it.
        if k.is_none()
            && self.regional(e)
            && let Expr::Call(f, args) = e
            && (f == "prev" || f == "next" || f == "fill")
        {
            if let [Arg::E(st), Arg::E(stop)] = args.as_slice()
                && f == "fill"
                && self.fill_kept(st, stop)
            {
                return Some(Expr::Empty);
            }
            return self.outside_shift(f, args, ctx, memo);
        }
        // In a region's part, likewise, of the masks' values in its parts.
        if let Some(kk) = k
            && self.regional(e)
            && let Expr::Call(f, args) = e
            && (f == "prev" || f == "next" || f == "fill")
        {
            return self.inside_shift(f, args, kk, part, ctx, memo);
        }
        Some(match e {
            // Assumed at a region token's last byte: none outside one, nor before its closer.
            Expr::Name(n) if self.is_end_name(n) => {
                if k.is_none() || !is_closer(part) {
                    Expr::Empty
                } else {
                    return None;
                }
            }
            Expr::Name(n) if self.regional.contains(n) => {
                let kind = |p: &str| opens.iter().position(|(q, _, _)| *n == format!("{p}_{q}"));
                let holds = if let Some(q) = kind("o") {
                    k == Some(q) && part == 0
                } else if let Some(q) = kind("f") {
                    k == Some(q) && !is_closer(part)
                } else if let Some(d) = self.defs.get(n) {
                    return self.in_region_of(d, k, part, ctx, memo);
                } else {
                    // The hidden bytes.
                    k.is_some() && part > 0
                };
                if holds { all() } else { Expr::Empty }
            }
            Expr::Name(_) | Expr::Empty => e.clone(),
            Expr::Not(a) => simplify(Expr::Not(Box::new(self.in_region_of(a, k, part, ctx, memo)?))),
            Expr::And(a, b) => {
                // Each side read where the other holds, at the byte and those about it.
                let mut ca = ctx.clone();
                let mut cb = ctx.clone();
                for o in -4..=4i64 {
                    let (x, y) = (self.bytes_at(a, o, memo), self.bytes_at(b, o, memo));
                    let i = (o + 4) as usize;
                    ca.0[i] = std::array::from_fn(|c| ca.0[i][c] && y[c]);
                    cb.0[i] = std::array::from_fn(|c| cb.0[i][c] && x[c]);
                }
                // A side that is none there: none, whatever the other.
                match (self.in_region_of(a, k, part, &ca, memo), self.in_region_of(b, k, part, &cb, memo)) {
                    (Some(Expr::Empty), _) | (_, Some(Expr::Empty)) => Expr::Empty,
                    (Some(x), Some(y)) => simplify(Expr::And(Box::new(x), Box::new(y))),
                    _ => return None,
                }
            }
            Expr::Or(a, b) => simplify(Expr::Or(Box::new(self.in_region_of(a, k, part, ctx, memo)?), Box::new(self.in_region_of(b, k, part, ctx, memo)?))),
            Expr::Xor(a, b) => simplify(Expr::Xor(Box::new(self.in_region_of(a, k, part, ctx, memo)?), Box::new(self.in_region_of(b, k, part, ctx, memo)?))),
            Expr::Call(..) if !self.regional(e) => e.clone(),
            Expr::Call(..) | Expr::Test { .. } => return None,
        })
    }

    /// `prev`, `next` or `fill` (`f` of `args`) at a byte outside the regions that is one of
    /// `ctx`, as of the masks' values outside (`in_region_of`): `prev(a, j)` where the `j` bytes
    /// before, where they are in a region token at its end, are none `a` holds at, in it or
    /// out; `next(a, j)` likewise at a region token's start; `fill(s, e)` where `e` stops it at
    /// a region token's last byte (or at its closer after it), and `s` does not start it there.
    fn outside_shift(&self, f: &str, args: &[Arg], ctx: &Ctx, memo: &mut ByteMemo) -> Option<Expr> {
        self.shift_before(f, args, ctx, &|_| [true; 256], memo)
    }

    /// `outside_shift`, where the bytes `o` on from the byte are `near(o)`'s.
    fn shift_before(&self, f: &str, args: &[Arg], ctx: &Ctx, near: &dyn Fn(i64) -> [bool; 256], memo: &mut ByteMemo) -> Option<Expr> {
        let ends = self.ends.as_ref()?;
        let all = [true; 256];
        let meets = |x: &[bool; 256], y: &[bool; 256]| (0..256).any(|i| x[i] && y[i]);
        // Whether some way `e` may hold at a byte of `ctx` fits the bytes `at(o)` the place
        // leaves `o` bytes on.
        let fits = |e: &Expr, at: &dyn Fn(i64) -> [bool; 256], memo: &mut ByteMemo| {
            self.cubes(e, memo).iter().any(|c| {
                let here = c.get(&0).copied().unwrap_or(all);
                let here: [bool; 256] = std::array::from_fn(|i| here[i] && ctx[i]);
                let at = |o: i64| {
                    let (x, y, z) = (at(o), near(o), ctx.at(o));
                    std::array::from_fn::<bool, 256, _>(|i| x[i] && y[i] && z[i])
                };
                // The place fits the bytes about the byte too.
                meets(&here, &at(0)) && (-4..=4).all(|o| meets(&[true; 256], &at(o))) && c.iter().all(|(o, b)| meets(b, &at(*o)))
            })
        };
        match (f, args) {
            ("prev" | "next", [Arg::E(a), rest @ ..]) => {
                let j = match rest {
                    [] => 1i64,
                    [Arg::N(j)] => *j as i64,
                    _ => return None,
                };
                let a0 = self.in_region_of(a, None, 0, &Ctx::all(), memo)?;
                if self.regional(&a0) {
                    return None;
                }
                let shifted = |x: &Expr| Expr::Call(f.to_owned(), std::iter::once(Arg::E(x.clone())).chain(rest.iter().cloned()).collect());
                let (e, e0) = (shifted(a), shifted(&a0));
                // The `j` bytes before (after) in a region token at its end (start): `a` there,
                // in it or as outside, holds in no way the bytes there fit.
                for m in 0..j {
                    let ok = if f == "prev" {
                        ends.iter().all(|end| {
                            // `o` bytes on from the byte is `m - j - o` before the token's last.
                            let at = |o: i64| match m - j - o {
                                d @ 0..=3 => end.last[d as usize],
                                -1 => end.after.unwrap_or(all),
                                _ => all,
                            };
                            // `a` in the token there (its closer, where the byte is one of the
                            // closer's): none in it, or no way it holds fits.
                            let parts: &[u8] = if end.after.is_none() && m <= end.ext { &[2] } else { &[0, 1, 2] };
                            let none_in = parts.iter().all(|&p| matches!(self.in_region_of(a, Some(end.k), p, &Ctx::all(), memo), Some(Expr::Empty)));
                            (none_in || !fits(&e, &at, memo)) && !fits(&e0, &at, memo)
                        })
                    } else {
                        // `o` bytes on is `o - j + m` after the token's first.
                        let at = |o: i64| match o - j + m {
                            d @ 0..=3 => self.opener_at[d as usize],
                            _ => all,
                        };
                        // `a` in the token there: none in any region's part, or no way it holds fits.
                        let none_in = (0..self.kinds.len()).all(|k| [0u8, 1, 2].iter().all(|&p| matches!(self.in_region(a, Some(k), p), Some(Expr::Empty))));
                        (none_in || !fits(&e, &at, memo)) && !fits(&e0, &at, memo)
                    };
                    if !ok {
                        return None;
                    }
                }
                Some(simplify(e0))
            }
            ("fill", [Arg::E(st), Arg::E(stop)]) => {
                let (s0, e0) = (self.in_region_of(st, None, 0, &Ctx::all(), memo)?, self.in_region_of(stop, None, 0, &Ctx::all(), memo)?);
                if self.regional(&s0) || self.regional(&e0) {
                    return None;
                }
                let stops = |s: &Expr, e: &Expr, yb: &[bool; 256], memo: &mut ByteMemo| {
                    !meets(&self.bytes_at(s, 0, memo), yb) && self.exact_bytes(e).is_some_and(|x| (0..256).all(|i| !yb[i] || x[i]))
                };
                for end in ends {
                    // `s` (anywhere) does not start it there.
                    let ok = match &end.after {
                        // At the closer after the token, outside: `e` stops the fill there.
                        Some(b) => stops(st, &e0, b, memo),
                        // At the token's last byte, in its region.
                        None => match self.in_region(stop, Some(end.k), 2) {
                            Some(ei) if !self.regional(&ei) => stops(st, &ei, &end.last[0], memo),
                            _ => false,
                        },
                    };
                    if !ok {
                        return None;
                    }
                }
                Some(Expr::Call("fill".to_owned(), vec![Arg::E(s0), Arg::E(e0)]))
            }
            _ => None,
        }
    }

    /// `prev`, `next` or `fill` (`f` of `args`) at a byte of part `part` of a token of region
    /// `k` that is one of `ctx` (`in_region_of`): of the masks' values in the part the byte is in,
    /// where at each place a shift reaches another part (or out of the token) the mask's value
    /// there is the same, or no way it may hold fits the bytes there (the opener's from the
    /// token's first byte, the closers' from the closer's first).
    fn inside_shift(&self, f: &str, args: &[Arg], k: usize, part: u8, ctx: &Ctx, memo: &mut ByteMemo) -> Option<Expr> {
        let r = self.kinds.get(k)?;
        let (opener, closers) = (r.opener.clone(), r.closers());
        let all = [true; 256];
        let meets = |x: &[bool; 256], y: &[bool; 256]| (0..256).any(|i| x[i] && y[i]);
        let fits = |e: &Expr, at: &dyn Fn(i64) -> [bool; 256], memo: &mut ByteMemo| {
            self.cubes(e, memo).iter().any(|c| {
                let here = c.get(&0).copied().unwrap_or(all);
                let here: [bool; 256] = std::array::from_fn(|i| here[i] && ctx[i]);
                let at = |o: i64| {
                    let (x, z) = (at(o), ctx.at(o));
                    std::array::from_fn::<bool, 256, _>(|i| x[i] && z[i])
                };
                meets(&here, &at(0)) && (-4..=4).all(|o| meets(&[true; 256], &at(o))) && c.iter().all(|(o, b)| meets(b, &at(*o)))
            })
        };
        // The bytes `d` on from the token's first byte, or from its closer's first.
        let from_open = |d: i64, memo: &mut ByteMemo| if (0..4).contains(&d) { self.bytes_at(&opener, d, memo) } else { all };
        let from_close = |d: i64, memo: &mut ByteMemo| if (0..4).contains(&d) { self.bytes_at(&closers, d, memo) } else { all };
        let val = |a: &Expr, p: u8, memo: &mut ByteMemo| self.in_region_of(a, Some(k), p, &Ctx::all(), memo).filter(|v| !self.regional(v));
        let same = |a: &Option<Expr>, b: &Expr| a.as_ref().is_some_and(|a| format!("{a:?}") == format!("{b:?}"));
        match (f, args) {
            ("prev" | "next", [Arg::E(a), rest @ ..]) => {
                let j = match rest {
                    [] => 1i64,
                    [Arg::N(j)] => *j as i64,
                    _ => return None,
                };
                let shifted = |x: &Expr| Expr::Call(f.to_owned(), std::iter::once(Arg::E(x.clone())).chain(rest.iter().cloned()).collect());
                let e = shifted(a);
                // A place where the shift reads another part (`there`, none: out of the token)
                // than `v`'s: the same value there, or no way it holds fits it.
                let safe = |v: &Expr, there: Option<Expr>, at: &dyn Fn(i64) -> [bool; 256], memo: &mut ByteMemo| same(&there, v) || (!fits(&e, at, memo) && !fits(&shifted(v), at, memo));
                if part == 0 {
                    // The token's first byte (its opener's): before it is as before a byte outside.
                    if f != "prev" {
                        return None;
                    }
                    let ob = self.bytes_at(&opener, 0, memo);
                    let c = ctx.and0(&ob);
                    let near = |o: i64| if (0..4).contains(&o) { self.bytes_at(&opener, o, &mut HashMap::new()) } else { all };
                    return self.shift_before(f, args, &c, &near, memo);
                }
                // Every byte, at a byte in the token: every byte.
                let full = |v: Expr| if matches!(&v, Expr::Not(x) if matches!(**x, Expr::Empty)) { v } else { simplify(shifted(&v)) };
                // A body byte at a known offset from the token's first (`BODY_AT`), or from offset
                // 5 on (`BODY_LATE`): before it, the part of the byte `j` before.
                if f == "prev" && (9..=12).contains(&part) {
                    let q = i64::from(part) - 8;
                    let t = q - j;
                    return match t {
                        // Before the token: as before a byte outside, the token's bytes known.
                        ..0 => {
                            let near = |o: i64| if (0..4).contains(&(q + o)) { self.bytes_at(&opener, q + o, &mut HashMap::new()) } else { all };
                            self.shift_before(f, args, ctx, &near, memo)
                        }
                        0 => val(a, 0, memo).map(full),
                        t => val(a, body_part(t), memo).map(full),
                    };
                }
                // A body byte by both ends (`body2`): before it, further from the closer; after it,
                // nearer, or the closer's byte.
                if part >= BODY2_AT {
                    let (q, t) = (i64::from((part - BODY2_AT) / 5) + 1, i64::from((part - BODY2_AT) % 5) + 1);
                    // The same at each of the parts `p(o)` for `o` from `lo` to 5 on.
                    let same_over = |lo: i64, p: &dyn Fn(i64) -> u8, memo: &mut ByteMemo| {
                        let v = val(a, p(5), memo)?;
                        (lo >= 1 && (lo..5).all(|o| same(&val(a, p(o), memo), &v))).then(|| full(v))
                    };
                    if f == "prev" {
                        if q == 5 {
                            return same_over(5 - j, &|o| body2(o, t + j), memo);
                        }
                        return match q - j {
                            ..0 => {
                                let near = |o: i64| if (0..4).contains(&(q + o)) { self.bytes_at(&opener, q + o, &mut HashMap::new()) } else { all };
                                self.shift_before(f, args, ctx, &near, memo)
                            }
                            0 => val(a, 0, memo).map(full),
                            nq => val(a, body2(nq, t + j), memo).map(full),
                        };
                    }
                    if t == 5 {
                        return same_over(5 - j, &|o| body2(q + j, o), memo);
                    }
                    let shortest = i64::from(r.bytes.min(r.or.as_ref().map_or(r.bytes, |o| o.1)));
                    return match t - j {
                        nt @ 1.. => val(a, body2(q + j, nt), memo).map(full),
                        nt if -nt < shortest => val(a, close_part(q + t, -nt), memo).map(full),
                        _ => None,
                    };
                }
                // A closer byte `rr` with the closer `d` after the token's first: before it, the
                // closer's byte or the body's, at its offset.
                if f == "prev" && part >= CLOSE_AT {
                    let (d, rr) = (i64::from((part - CLOSE_AT) / 4) + 1, i64::from((part - CLOSE_AT) % 4));
                    if rr >= j {
                        return val(a, close_part(d, rr - j), memo).map(full);
                    }
                    let q = d + rr;
                    if d < 5 {
                        return match q - j {
                            ..0 => {
                                let near = |o: i64| if (0..4).contains(&(q + o)) { self.bytes_at(&opener, q + o, &mut HashMap::new()) } else { all };
                                self.shift_before(f, args, ctx, &near, memo)
                            }
                            0 => val(a, 0, memo).map(full),
                            t => val(a, body2(t, j - rr), memo).map(full),
                        };
                    }
                    // From 5 on: the body's bytes from `5 + rr - j` on, each the same.
                    let lo = 5 + rr - j;
                    let v = val(a, body2(5, j - rr), memo)?;
                    if lo < 1 || !(lo..=4).all(|o| same(&val(a, body2(o, j - rr), memo), &v)) {
                        return None;
                    }
                    return Some(full(v));
                }
                if f == "prev" && part == BODY_LATE {
                    let v = val(a, BODY_LATE, memo)?;
                    if j > 4 || !((5 - j).max(1)..=4).all(|o| same(&val(a, body_part(o), memo), &v)) {
                        return None;
                    }
                    return Some(full(v));
                }
                // From the closer, a shift before reads the body; one after, the byte `j` on.
                let at_part = match part {
                    2 if f == "prev" => 1,
                    9..=12 => body_part(i64::from(part) - 8 + j),
                    p => p,
                };
                let v = val(a, at_part, memo)?;
                let v0 = val(a, 0, memo);
                if f == "prev" {
                    // From the token's first byte `q` on: `j - q` before it, out of it; at it, part 0.
                    for q in 1..=j {
                        let at_open: Vec<[bool; 256]> = (0..8).map(|d| from_open(d - 4, memo)).collect();
                        let at = |o: i64| at_open.get((q + o + 4) as usize).copied().unwrap_or(all);
                        if !safe(&v, if q == j { v0.clone() } else { None }, &at, memo) {
                            return None;
                        }
                    }
                    if part == 2 {
                        // From the closer's first byte `r` on, `j` or more: in the closer.
                        let v2 = val(a, 2, memo);
                        for rr in j..i64::from(r.bytes) {
                            let at_close: Vec<[bool; 256]> = (0..8).map(|d| from_close(d - 4, memo)).collect();
                            let at = |o: i64| at_close.get((rr + o + 4) as usize).copied().unwrap_or(all);
                            if !safe(&v, v2.clone(), &at, memo) {
                                return None;
                            }
                        }
                    }
                } else {
                    if is_closer(part) {
                        return None;
                    }
                    // `q` before the closer's first byte: `j - q` on, in the closer.
                    let v2 = val(a, 2, memo);
                    for q in 1..=j {
                        let at_close: Vec<[bool; 256]> = (0..8).map(|d| from_close(d - 4, memo)).collect();
                        let at = |o: i64| at_close.get((o - q + 4) as usize).copied().unwrap_or(all);
                        let there = if j - q < i64::from(r.bytes) && !r.outside { v2.clone() } else { None };
                        if !safe(&v, there, &at, memo) {
                            return None;
                        }
                    }
                }
                Some(full(v))
            }
            ("fill", [Arg::E(st), Arg::E(stop)]) => {
                // Kept to the bodies: none but in one, there of the body's values.
                if self.fill_kept(st, stop) {
                    if part == 0 || is_closer(part) {
                        return Some(Expr::Empty);
                    }
                    let (s1, e1) = (self.in_region(st, Some(k), 1)?, self.in_region(stop, Some(k), 1)?);
                    if matches!(s1, Expr::Empty) {
                        return Some(Expr::Empty);
                    }
                    return Some(Expr::Call("fill".to_owned(), vec![Arg::E(s1), Arg::E(e1)]));
                }
                // At the token's first byte `e` stops it and `s` does not start it, in its value
                // there and in the body's; in the body, of the body's values.
                let ob = self.bytes_at(&opener, 0, memo);
                let stops = |s: &Expr, e: &Expr, memo: &mut ByteMemo| !meets(&self.bytes_at(s, 0, memo), &ob) && self.exact_bytes(e).is_some_and(|x| (0..256).all(|i| !ob[i] || x[i]));
                let (s0, e0) = (val(st, 0, memo)?, val(stop, 0, memo)?);
                // Started in no part, and stopped at the token's first byte: none in it.
                let none = |v: Option<Expr>| matches!(v, Some(Expr::Empty));
                if none(Some(s0.clone())) && none(val(st, 1, memo)) && none(val(st, 2, memo)) && self.exact_bytes(&e0).is_some_and(|x| (0..256).all(|i| !ob[i] || x[i])) {
                    return Some(Expr::Empty);
                }
                if part == 0 {
                    return self.exact_bytes(&e0).is_some_and(|x| (0..256).all(|i| !ob[i] || x[i])).then_some(s0);
                }
                if is_closer(part) {
                    return None;
                }
                let (s1, e1) = (val(st, 1, memo)?, val(stop, 1, memo)?);
                if !stops(st, &e0, memo) || !stops(&s1, &e1, memo) {
                    return None;
                }
                Some(Expr::Call("fill".to_owned(), vec![Arg::E(s1), Arg::E(e1)]))
            }
            _ => None,
        }
    }

    /// How the region tokens end (`End`), from each row that reads a region and its closers
    /// (`row_geos`), and the bytes their openers' first bytes may be.
    fn find_ends(&mut self, rows: &[(usize, &Stmt)]) -> Result<(), String> {
        let n = self.opens.as_ref().map_or(0, Vec::len);
        let mut ends = Vec::new();
        let mut seen = vec![false; n];
        for (_, st) in rows {
            let Stmt::Token { start, rule, .. } = st else { continue };
            let regions: Vec<(Region, Vec<(String, Expr, u8)>)> = match rule {
                Some(TokenRule::Read(rs)) => rs.iter().map(|r| (r.clone(), Vec::new())).collect(),
                Some(TokenRule::ReadParts(rs, parts)) => match Self::row_region(start, rs) {
                    Some(r) => vec![(r.clone(), parts.closes.clone())],
                    None => return Ok(()),
                },
                _ => continue,
            };
            for (r, closes) in regions {
                let Some(geos) = self.row_geos(&r, &closes)? else { return Ok(()) };
                for (_, g) in geos {
                    let mut memo = HashMap::new();
                    let last = std::array::from_fn(|m| self.bytes_at(&g.closer, g.ext - m as i64, &mut memo));
                    let after = (g.ext < 0).then(|| self.bytes_at(&g.closer, 0, &mut memo));
                    seen[g.k] = true;
                    ends.push(End { k: g.k, ext: g.ext, after, last });
                }
            }
        }
        // A region no row reads: it may end at any byte.
        if seen.iter().any(|s| !s) {
            return Ok(());
        }
        let mut memo = HashMap::new();
        let mut opener_at = [[false; 256]; 4];
        for r in &self.kinds {
            for (m, at) in opener_at.iter_mut().enumerate() {
                let b = self.bytes_at(&r.opener, m as i64, &mut memo);
                (0..256).for_each(|i| at[i] |= b[i]);
            }
        }
        self.opener_at = opener_at;
        self.ends = Some(ends);
        Ok(())
    }

    /// The bytes `e` can hold at, `d` bytes on from where it holds (`d` < 0: before): the
    /// bytes of each way it may hold (its `Cubes`) there, together. Every byte where it says nothing.
    /// The token's first byte picks its region and its row as simdjson's `classify` picks a byte's
    /// class: the bytes split in groups, by which openers and rows may start at them and which
    /// start at every one of them; each group a few products of high and low nibbles, a bit each
    /// in two tables (`DISPATCH_LO`, `DISPATCH_HI`), so a byte's group is the lowest bit of
    /// `LO[c & 15] & HI[c >> 4]`. In a group, the openers and the rows that may start there are
    /// tested in their order, each as `at_start` reads it of the group's bytes: one that starts at
    /// every one of them is no test.
    fn dispatch(&mut self, rows: &[RowArm]) -> Result<(String, Picks, Vec<Vec<usize>>, Vec<Vec<String>>), String> {
        let opens: Vec<(String, Expr, Vec<usize>)> = match &self.opens {
            Some(o) => o.iter().map(|(kind, local, after)| (local.clone(), self.kinds.iter().find(|r| &r.kind == kind).map_or(Expr::Not(Box::new(Expr::Empty)), |r| r.opener.clone()), after.clone())).collect(),
            None => Vec::new(),
        };
        // Each arm's bytes, and those it holds at exactly.
        let mut memo = HashMap::new();
        let mut arm_bytes: Vec<([bool; 256], Option<[bool; 256]>)> = Vec::new();
        for (_, o, _) in &opens {
            arm_bytes.push((self.bytes_at(o, 0, &mut memo), if self.regional(o) { None } else { self.exact_bytes(o) }));
        }
        for r in rows {
            arm_bytes.push((self.bytes_at(&r.start, 0, &mut memo), if self.regional(&r.start) { None } else { self.exact_bytes(&r.start) }));
        }
        // The groups: the bytes alike in each arm.
        let mut groups: Vec<(Vec<u8>, [bool; 256])> = Vec::new();
        for c in 0..256usize {
            if self.skipped[c] {
                continue;
            }
            let sig: Vec<u8> = arm_bytes.iter().map(|(b, x)| if !b[c] { 0 } else if x.is_some_and(|x| x[c]) { 2 } else { 1 }).collect();
            if sig.iter().all(|&t| t == 0) {
                continue;
            }
            match groups.iter_mut().find(|g| g.0 == sig) {
                Some(g) => g.1[c] = true,
                None => {
                    let mut b = [false; 256];
                    b[c] = true;
                    groups.push((sig, b));
                }
            }
        }
        // Each group's products of nibbles: its high nibbles with the same low ones are one.
        let mut rects: Vec<(usize, u16, u16)> = Vec::new();
        for (g, (_, b)) in groups.iter().enumerate() {
            let mut lows = [0u16; 16];
            for c in 0..256 {
                if b[c] {
                    lows[c >> 4] |= 1 << (c & 15);
                }
            }
            let mut mine: Vec<(usize, u16, u16)> = Vec::new();
            for (h, &l) in lows.iter().enumerate() {
                if l == 0 {
                    continue;
                }
                match mine.iter_mut().find(|r| r.2 == l) {
                    Some(r) => r.1 |= 1 << h,
                    None => mine.push((g, 1 << h, l)),
                }
            }
            rects.extend(mine);
        }
        let ty = match rects.len() {
            0..=8 => "u8",
            9..=16 => "u16",
            17..=32 => "u32",
            33..=64 => "u64",
            k => return Err(format!("tier 1 as Rust: the first bytes' groups need {k} nibble products, past a table's 64")),
        };
        let (mut lo, mut hi) = ([0u64; 16], [0u64; 16]);
        for (bit, &(_, h, l)) in rects.iter().enumerate() {
            for n in 0..16 {
                if h >> n & 1 == 1 {
                    hi[n] |= 1 << bit;
                }
                if l >> n & 1 == 1 {
                    lo[n] |= 1 << bit;
                }
            }
        }
        let list = |t: &[u64; 16]| t.iter().map(|b| format!("{b:#x}")).collect::<Vec<_>>().join(", ");
        let tables = format!("/// The token's first byte's group (`dispatch`), as simdjson's `classify`: a byte's bit `r` is set in both where it is of product `r`'s high and low nibbles; each product is one group's.\nstatic DISPATCH_LO: [{ty}; 16] = [{}];\nstatic DISPATCH_HI: [{ty}; 16] = [{}];\n\n", list(&lo), list(&hi));
        // Each group's openers and rows, as `at_start` reads them of its bytes.
        let mut picks = Picks { open: String::new(), rows: String::new(), fused: true };
        let mut fused_rows = String::new();
        let mut chains: Vec<Vec<usize>> = Vec::new();
        // The regions opened where each row is picked: of a group's, those no row before it in the
        // group's chain is picked at alone (`open == k`).
        let mut row_opens: Vec<Vec<String>> = vec![Vec::new(); rows.len()];
        for (g, (sig, b)) in groups.iter().enumerate() {
            let pats = rects.iter().enumerate().filter(|r| r.1.0 == g).map(|(bit, _)| bit.to_string()).collect::<Vec<_>>().join(" | ");
            let shown: String = (0..256u32).filter(|&c| b[c as usize]).map(|c| if (33..127).contains(&c) { char::from_u32(c).unwrap_or('?').to_string() } else { format!("\\x{c:02x}") }).collect();
            let exact = |i: usize| sig[i] == 2 && (0..256).all(|c| !b[c] || arm_bytes[i].1.is_some_and(|x| x[c]));
            let mut chain: Vec<(String, String)> = Vec::new();
            for (k, (local, _, after)) in opens.iter().enumerate() {
                if sig[k] == 0 {
                    continue;
                }
                let at = if exact(k) { "true".to_owned() } else { format!("bit(&{local}, s)") };
                let cond = if after.is_empty() { at } else { format!("({}) && {at}", after.iter().map(|a| format!("template == {a}")).collect::<Vec<_>>().join(" || ")).replace(" && true", "") };
                chain.push((cond, k.to_string()));
            }
            // The regions a token here may open: those tested, to the first that is no test, or none.
            let mut may_open: Vec<String> = Vec::new();
            for (c, v) in &chain {
                may_open.push(v.clone());
                if c == "true" {
                    break;
                }
            }
            if chain.last().is_none_or(|(c, _)| c != "true") {
                may_open.push("usize::MAX".to_owned());
            }
            let open_chain = chain.clone();
            if !chain.is_empty() {
                let _ = write!(picks.open, "            // `{shown}`\n            {pats} => {},\n", if_chain(&chain, "usize::MAX", "            "));
            }
            self.start_in = Some(*b);
            let mut chain: Vec<(String, String)> = Vec::new();
            let mut left = may_open.clone();
            let mut reads: Vec<Option<String>> = Vec::new();
            for (i, r) in rows.iter().enumerate() {
                if sig[opens.len() + i] == 0 {
                    continue;
                }
                let st = match self.at_start(&r.start)? {
                    Some(t) => t,
                    None => format!("bit(&{}, s)", self.val(&r.start)?),
                };
                // A test of the region opened, of those it may be here.
                let st = match st.strip_prefix("open == ") {
                    Some(k) if !may_open.iter().any(|m| m == k) => "false".to_owned(),
                    Some(_) if may_open.len() == 1 => "true".to_owned(),
                    _ => st,
                };
                if st == "false" {
                    continue;
                }
                // The region the entry reads (none: a row of code; empty: a test of `open` not alone).
                reads.push(match st.strip_prefix("open == ") {
                    Some("usize::MAX") => None,
                    Some(k) if k.chars().all(|c| c.is_ascii_digit()) => Some(k.to_owned()),
                    Some(_) => Some(String::new()),
                    None if st == "true" && may_open.len() == 1 && may_open[0] != "usize::MAX" => Some(may_open[0].clone()),
                    None if uses(&st, "open") => Some(String::new()),
                    None => None,
                });
                let here: Vec<String> = match st.strip_prefix("open == ") {
                    Some(k) => left.iter().filter(|m| *m == k).cloned().collect(),
                    None => left.clone(),
                };
                for m in here {
                    if !row_opens[i].contains(&m) {
                        row_opens[i].push(m);
                    }
                }
                if let Some(k) = st.strip_prefix("open == ") {
                    left.retain(|m| m != k);
                }
                if st == "true" {
                    left.clear();
                }
                let st = if r.check.is_empty() { st } else { format!("{{{}\n                {st}\n            }}", r.check.replace("\n            ", "\n                ")) };
                chain.push((st, i.to_string()));
            }
            self.start_in = None;
            // Fused: each opener picks its region's row (the first that reads it), and with none
            // the code's rows follow, where every region's row is before every row of code.
            let code_from = reads.iter().position(Option::is_none).unwrap_or(reads.len());
            let ok = reads[code_from..].iter().all(Option::is_none)
                && reads[..code_from].iter().all(|k| k.as_ref().is_some_and(|k| !k.is_empty()))
                && may_open.iter().all(|m| m == "usize::MAX" || reads.iter().any(|k| k.as_deref() == Some(m.as_str())));
            if !ok {
                picks.fused = false;
            }
            if picks.fused {
                let mut fchain: Vec<(String, String)> = Vec::new();
                for (cond, k) in &open_chain {
                    let Some(e) = reads.iter().position(|r| r.as_deref() == Some(k.as_str())) else { continue };
                    let i: usize = chain[e].1.parse().unwrap_or(usize::MAX);
                    let r = &rows[i];
                    let st = if r.check.is_empty() { cond.clone() } else { format!("{{{}\n                {cond}\n            }}", r.check.replace("\n            ", "\n                ")) };
                    fchain.push((st, chain[e].1.clone()));
                }
                if may_open.iter().any(|m| m == "usize::MAX") {
                    // Reached where no opener holds: `open == usize::MAX` is so.
                    fchain.extend(chain[code_from..].iter().map(|(t, v)| (if t.ends_with("open == usize::MAX") { t.replace("open == usize::MAX", "true") } else { t.clone() }, v.clone())));
                }
                if !fchain.is_empty() {
                    let _ = write!(fused_rows, "                // `{shown}`\n                {pats} => {},\n", if_chain(&fchain, "usize::MAX", "                "));
                }
            }
            chains.push(chain.iter().map(|(_, v)| v.parse().unwrap_or(usize::MAX)).collect());
            if !chain.is_empty() {
                let _ = write!(picks.rows, "                // `{shown}`\n                {pats} => {},\n", if_chain(&chain, "usize::MAX", "                "));
            }
        }
        if picks.fused {
            picks.rows = fused_rows;
        }
        Ok((tables, picks, chains, row_opens))
    }

    /// Whether `e` may hold at a byte, given the bytes about it (`ctx`): some way it holds
    /// (`cubes`) needs only bytes `ctx` allows at each offset.
    fn may_hold(&self, e: &Expr, ctx: &Ctx) -> bool {
        self.cubes(e, &mut HashMap::new()).iter().any(|c| {
            c.iter().all(|(&o, b)| {
                let a = ctx.at(o);
                (0..256).any(|i| b[i] && a[i])
            })
        })
    }

    fn bytes_at(&self, e: &Expr, d: i64, memo: &mut ByteMemo) -> [bool; 256] {
        let mut v = [false; 256];
        for c in self.cubes(e, memo) {
            match c.get(&d) {
                Some(b) => (0..256).for_each(|i| v[i] |= b[i]),
                None => return [true; 256],
            }
        }
        v
    }

    /// The ways `e` may hold, each the bytes it needs at some offsets from where it holds: a
    /// class's at its own byte; what a `prev` or a `next` reads, shifted; both sides of a `&`
    /// at once (a way whose bytes at an offset are none cannot be); either of a `|`. So `a & b`
    /// where `a` needs a digit after it and `b` a sign there is none.
    fn cubes(&self, e: &Expr, memo: &mut ByteMemo) -> Cubes {
        let one = |d: i64, b: [bool; 256]| vec![std::collections::BTreeMap::from([(d, b)])];
        let all = || vec![std::collections::BTreeMap::new()];
        match e {
            Expr::Empty => Vec::new(),
            Expr::Name(n) => {
                if let Some(set) = self.class_bytes.get(n) {
                    return one(0, *set);
                }
                if let Some(r) = memo.get(n) {
                    return r.clone();
                }
                // A region's opener mark holds where its opener does.
                if let Some(r) = self.kinds.iter().find(|r| *n == format!("o_{}", r.kind)) {
                    let opener = r.opener.clone();
                    return self.cubes(&opener, memo);
                }
                // A name being read (a cycle) says nothing.
                memo.insert(n.clone(), all());
                let r = match self.defs.get(n) {
                    Some(def) => self.cubes(def, memo),
                    None => all(),
                };
                memo.insert(n.clone(), r.clone());
                r
            }
            // A class's complement, at its own byte, or shifted at the byte it reads (where the
            // class's bytes are exact); `~~a`, `a`'s; else nothing known.
            Expr::Not(a) => match &**a {
                Expr::Name(n) if self.class_bytes.contains_key(n) => one(0, self.class_bytes[n].map(|b| !b)),
                Expr::Not(b) => self.cubes(b, memo),
                Expr::Call(f, args) if f == "prev" || f == "next" => {
                    let k = match args.get(1) {
                        None => 1,
                        Some(Arg::N(k)) => *k as i64,
                        _ => return all(),
                    };
                    match args.first() {
                        Some(Arg::E(x)) => match self.exact_bytes(x) {
                            Some(b) => one(if f == "next" { k } else { -k }, b.map(|t| !t)),
                            None => all(),
                        },
                        _ => all(),
                    }
                }
                _ => all(),
            },
            // A fill holds at its start, or at a byte its stop is not at.
            Expr::Call(f, args) if f == "fill" => match args.as_slice() {
                [Arg::E(a), Arg::E(e)] => {
                    let mut v = self.cubes(a, memo);
                    v.extend(self.cubes(&Expr::Not(Box::new(e.clone())), memo).into_iter().map(|c| c.into_iter().filter(|(d, _)| *d == 0).collect()));
                    widen(v)
                }
                _ => all(),
            },
            Expr::And(a, b) => {
                let (x, y) = (self.cubes(a, memo), self.cubes(b, memo));
                let mut v = Vec::new();
                for p in &x {
                    'q: for q in &y {
                        let mut c = p.clone();
                        for (d, b) in q {
                            let m = c.entry(*d).or_insert([true; 256]);
                            (0..256).for_each(|i| m[i] &= b[i]);
                            if !m.iter().any(|&t| t) {
                                continue 'q;
                            }
                        }
                        v.push(c);
                    }
                }
                widen(v)
            }
            Expr::Or(a, b) | Expr::Xor(a, b) => {
                let mut v = self.cubes(a, memo);
                v.extend(self.cubes(b, memo));
                widen(v)
            }
            Expr::Call(f, args) if (f == "prev" || f == "next") => {
                let Some(Arg::E(a)) = args.first() else { return all() };
                let k = match args.get(1) {
                    Some(Arg::N(k)) => *k as i64,
                    _ => 1,
                };
                // `prev(a, k)` holds `k` bytes after `a`: `a`'s byte `d` is its `d - k`.
                let k = if f == "next" { k } else { -k };
                self.cubes(a, memo).into_iter().map(|c| c.into_iter().map(|(d, b)| (d + k, b)).collect()).collect()
            }
            Expr::Call(f, args) if f == "text" => match args.first() {
                Some(Arg::S(t)) if !t.is_empty() => vec![t.iter().enumerate().map(|(i, &c)| (i as i64, std::array::from_fn(|j| j == usize::from(c)))).collect()],
                _ => all(),
            },
            Expr::Call(..) | Expr::Test { .. } => all(),
        }
    }

    /// Mask `e` as code over the bytes of a row's token: over a token of code, its value outside
    /// the regions, where that is stage 1's.
    fn row_val(&mut self, e: &Expr, rule: &Option<TokenRule>) -> Result<String, String> {
        if self.regional(e)
            && !matches!(rule, Some(TokenRule::Read(_) | TokenRule::ReadParts(..)))
            && let Some(o) = self.in_region(e, None, 0)
            && !self.regional(&o)
        {
            return self.val(&o);
        }
        self.val(e)
    }

    /// The bytes a token of a code row may have after its start: its run's (as its kind's
    /// assumptions read it), its pairs' second and third bytes, none for one byte; else any.
    fn row_tail_bytes(&mut self, kind: &str, rule: &Option<TokenRule>, last: &Option<Expr>) -> [bool; 256] {
        let mut memo = HashMap::new();
        match rule {
            Some(TokenRule::Pairs { pairs, .. }) => {
                let mut v = [false; 256];
                for p in pairs {
                    for &b in p.iter().skip(1) {
                        v[usize::from(b)] = true;
                    }
                }
                v
            }
            Some(TokenRule::Run(r)) => {
                let these: Vec<(&str, &Expr)> = self.x.assumed.iter().filter(|a| a.0.as_deref() == Some(kind)).map(|a| (a.1.as_str(), &a.2)).collect();
                let read = if these.is_empty() { r.clone() } else { substitute(r, &these, &self.defs) };
                self.bytes_at(&read, 0, &mut memo)
            }
            None if last.is_none() => [false; 256],
            _ => [true; 256],
        }
    }

    /// Whether mask `n` holds only at a region token's last byte: assumed so (`assume … at
    /// ends`), or a `&` of one.
    fn is_end_name(&self, n: &str) -> bool {
        self.x.assumed_ends.iter().any(|(m, _)| m == n) || self.defs.get(n).is_some_and(|d| self.is_end(d))
    }

    fn is_end(&self, e: &Expr) -> bool {
        match e {
            Expr::Name(n) => self.is_end_name(n),
            Expr::And(a, b) => self.is_end(a) || self.is_end(b),
            _ => false,
        }
    }

    /// `e` at `d` bytes before a region token's last byte `l` (`d` < 0: after it), as its closer
    /// leaves it (`g`): the region's masks known from the closer back to the byte before it, and
    /// to the byte after the token; a mask of stage 1 its bit there, none where its bytes are not
    /// the closer's. `None`: not known there.
    fn at_end(&mut self, e: &Expr, d: i64, g: &Geo) -> Result<Option<Known>, String> {
        // The byte's place from the closer's first.
        let r = g.ext - d;
        if !self.regional(e) {
            if matches!(e, Expr::Empty) {
                return Ok(Some(Known::No));
            }
            // The bytes known: from the one before the closer to the token's last, or to the
            // closer after it (`outside`).
            // The closer itself, at its first byte (through names and shifts): it holds.
            let mut base = e.clone();
            let mut at = r;
            loop {
                if at == 0 && format!("{base:?}") == format!("{:?}", g.closer) {
                    return Ok(Some(Known::Yes));
                }
                match &base {
                    Expr::Name(n) if self.defs.contains_key(n) && !self.class_bytes.contains_key(n) => base = self.defs[n].clone(),
                    Expr::Call(f, args) if (f == "prev" || f == "next") && matches!(args.first(), Some(Arg::E(_))) => {
                        let k = match args.get(1) {
                            Some(Arg::N(k)) => *k as i64,
                            _ => 1,
                        };
                        at += if f == "prev" { -k } else { k };
                        let Some(Arg::E(a)) = args.first() else { unreachable!() };
                        base = a.clone();
                    }
                    _ => break,
                }
            }
            // Each byte known (`q`, from the closer's first), against what `e` needs there.
            let mut memo = HashMap::new();
            for q in -1..=g.ext.max(0) {
                let (ctx, need) = (self.bytes_at(&g.closer, q, &mut memo), self.bytes_at(e, q - r, &mut memo));
                if !(0..256).any(|i| ctx[i] && need[i]) {
                    return Ok(Some(Known::No));
                }
            }
            let pos = match d {
                0 => "l".to_owned(),
                d if d > 0 => format!("l - {d}"),
                d => format!("l + {}", -d),
            };
            let v = self.val(e)?;
            return Ok(Some(Known::Test(format!("bit(&{v}, {pos})"))));
        }
        Ok(match e {
            Expr::Name(n) => {
                let kind = |p: &str| self.kinds.iter().position(|q| *n == format!("{p}_{}", q.kind));
                // After the closer: the token's own bytes are no region's; the byte after it may
                // open one.
                let after = |r: i64| if r <= g.ext { Some(Known::No) } else { None };
                // From the token's first byte to the closer: its region's fill; after its first, no
                // opener and the hidden bytes.
                let body = r < 0 && r >= -g.body;
                let filled = r < 0 && r >= -g.body - 1;
                if let Some(q) = kind("o") {
                    if r >= g.h {
                        after(r)
                    } else if r >= 0 || body || (filled && q != g.k) {
                        Some(Known::No)
                    } else {
                        None
                    }
                } else if let Some(q) = kind("f") {
                    if r >= g.h {
                        after(r)
                    } else if r >= 0 {
                        Some(Known::No)
                    } else if filled {
                        Some(if q == g.k { Known::Yes } else { Known::No })
                    } else {
                        None
                    }
                } else if let Some(def) = self.defs.get(n).cloned() {
                    return self.at_end(&def, d, g);
                } else if *n == self.hidden {
                    // The closer's bytes; none after them to the byte after the token (an opener
                    // there is not hidden).
                    if r >= g.h {
                        if r <= g.ext + 1 { Some(Known::No) } else { None }
                    } else if r >= 0 || body {
                        Some(Known::Yes)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            Expr::Not(a) => self.at_end(a, d, g)?.map(Known::not),
            Expr::And(a, b) => match (self.at_end(a, d, g)?, self.at_end(b, d, g)?) {
                (Some(Known::No), _) | (_, Some(Known::No)) => Some(Known::No),
                (Some(x), Some(y)) => Some(x.and(y)),
                _ => None,
            },
            Expr::Or(a, b) => match (self.at_end(a, d, g)?, self.at_end(b, d, g)?) {
                (Some(Known::Yes), _) | (_, Some(Known::Yes)) => Some(Known::Yes),
                (Some(x), Some(y)) => Some(x.or(y)),
                _ => None,
            },
            Expr::Xor(a, b) => match (self.at_end(a, d, g)?, self.at_end(b, d, g)?) {
                (Some(x), Some(y)) => Some(x.xor(y)),
                _ => None,
            },
            Expr::Call(f, args) if f == "prev" || f == "next" => {
                let Some(Arg::E(a)) = args.first() else { return Ok(None) };
                let k = match args.get(1) {
                    Some(Arg::N(k)) => *k as i64,
                    _ => 1,
                };
                self.at_end(a, if f == "prev" { d + k } else { d - k }, g)?
            }
            Expr::Empty => Some(Known::No),
            Expr::Call(..) | Expr::Test { .. } => None,
        })
    }

    /// The closers a region token's read may end at, each its test over `close` (the closer's
    /// first byte) and where it leaves the token's last byte (`Geo`). `None`: a closer the walk's
    /// length of is not known from its bytes.
    fn row_geos(&mut self, r: &Region, closes: &[(String, Expr, u8)]) -> Result<Option<Vec<(String, Geo)>>, String> {
        let Some(k) = self.kinds.iter().position(|q| q.kind == r.kind) else { return Ok(None) };
        let body = self.opener_body(r);
        if r.outside {
            return Ok(Some(vec![("true".to_owned(), Geo { k, ext: -1, h: 0, closer: r.closer.clone(), body })]));
        }
        let mut alts = Vec::new();
        // The earlier closers' tests, each with its bytes: one that can not be at this one's byte
        // need not be tested.
        let mut before: Vec<(String, [bool; 256])> = Vec::new();
        let test = |before: &[(String, [bool; 256])], here: &[bool; 256], t: String| before.iter().filter(|(_, b)| (0..256).any(|i| b[i] && here[i])).map(|(t, _)| t.clone()).chain([t]).collect::<Vec<_>>().join(" && ");
        let mut memo = HashMap::new();
        for (_, e, n) in closes {
            let v = self.val(e)?;
            let mine = self.bytes_at(e, 0, &mut memo);
            let h = match &r.or {
                Some((c2, m)) => {
                    let mut memo = HashMap::new();
                    let (be, bc) = (self.bytes_at(e, 0, &mut memo), self.bytes_at(c2, 0, &mut memo));
                    if (0..256).all(|i| !be[i] || bc[i]) {
                        i64::from(*m)
                    } else if (0..256).all(|i| !be[i] || !bc[i]) {
                        i64::from(r.bytes)
                    } else {
                        return Ok(None);
                    }
                }
                None => i64::from(r.bytes),
            };
            alts.push((test(&before, &mine, format!("bit(&{v}, close)")), Geo { k, ext: i64::from(*n) - 1, h, closer: e.clone(), body }));
            before.push((format!("!bit(&{v}, close)"), mine));
        }
        // The region's own closer where no part's took it: its alternatives but the parts'.
        let taken: Vec<String> = closes.iter().map(|(_, e, _)| format!("{e:?}")).collect();
        fn without(e: &Expr, taken: &[String]) -> Option<Expr> {
            if taken.contains(&format!("{e:?}")) {
                return None;
            }
            match e {
                Expr::Or(a, b) => match (without(a, taken), without(b, taken)) {
                    (Some(a), Some(b)) => Some(Expr::Or(Box::new(a), Box::new(b))),
                    (x, None) | (None, x) => x,
                },
                _ => Some(e.clone()),
            }
        }
        let own = without(&r.closer, &taken).unwrap_or(Expr::Empty);
        match &r.or {
            Some((c2, m)) => {
                let v = self.val(c2)?;
                // A closer a part's takes first: none of its own.
                if !closes.iter().any(|(_, e, _)| format!("{e:?}") == format!("{c2:?}")) {
                    let mine = self.bytes_at(c2, 0, &mut memo);
                    alts.push((test(&before, &mine, format!("bit(&{v}, close)")), Geo { k, ext: i64::from(*m) - 1, h: i64::from(*m), closer: c2.clone(), body }));
                }
                let mine = self.bytes_at(&own, 0, &mut memo);
                alts.push((test(&before, &mine, format!("!bit(&{v}, close)")), Geo { k, ext: i64::from(r.bytes) - 1, h: i64::from(r.bytes), closer: own.clone(), body }));
            }
            None => {
                let mine = self.bytes_at(&own, 0, &mut memo);
                let t = before.iter().filter(|(_, b)| (0..256).any(|i| b[i] && mine[i])).map(|(t, _)| t.clone()).collect::<Vec<_>>().join(" && ");
                let t = if t.is_empty() { "true".to_owned() } else { t };
                alts.push((t, Geo { k, ext: i64::from(r.bytes) - 1, h: i64::from(r.bytes), closer: own.clone(), body }));
            }
        }
        Ok(Some(alts))
    }

    /// Region `r`'s opener's bytes after its first that no closer can be: the closer is after them.
    fn opener_body(&self, r: &Region) -> i64 {
        // Past its bytes (`Region::opens`) at least.
        let mut body = i64::from(r.opens) - 1;
        let mut memo = HashMap::new();
        let closers = self.bytes_at(&r.closers(), 0, &mut memo);
        while body < 8 {
            let at = self.bytes_at(&r.opener, body + 1, &mut memo);
            if (0..256).any(|i| at[i] && closers[i]) {
                break;
            }
            body += 1;
        }
        body
    }

    /// `T` at the last byte of a token of region `r` (over `close` and `l`), and whether it is
    /// none at the byte after it; `None` where not known.
    fn end_value(&mut self, t: &Expr, r: &Region, closes: &[(String, Expr, u8)]) -> Result<Option<Known>, String> {
        let Some(alts) = self.row_geos(r, closes)? else { return Ok(None) };
        let mut v = Known::No;
        for (test, g) in &alts {
            // At the byte after the token: none, or the loop does not know it there.
            if !matches!(self.at_end(t, -1, g)?, Some(Known::No)) {
                return Ok(None);
            }
            let Some(here) = self.at_end(t, 0, g)? else { return Ok(None) };
            let cond = if test == "true" {
                Known::Yes
            } else if test.contains("&&") {
                Known::Test(format!("({test})"))
            } else {
                Known::Test(test.clone())
            };
            v = v.or(cond.and(here));
        }
        Ok(Some(v))
    }

    /// `e` in one place of span `sp` (`fill(prev(T), E)`, its token's last byte `l` and `c` the
    /// next byte of `E` after `l + 1`): 0 at `l`, 1 at `l + 1`, 2 after it to `c`, 3 at `c`, 4
    /// elsewhere. `None`: `e` reads the span's masks otherwise, or another's.
    fn in_span(&self, e: &Expr, sp: &Span, at: u8) -> Option<Expr> {
        let all = || Expr::Not(Box::new(Expr::Empty));
        let of = |b: bool| if b { all() } else { Expr::Empty };
        let tname = match &sp.t {
            Expr::Name(n) => n.as_str(),
            _ => return None,
        };
        let reads = |e: &Expr| reads_mask(e, &sp.f, &self.defs) || reads_mask(e, tname, &self.defs) || self.x.assumed_ends.iter().any(|(m, _)| reads_mask(e, m, &self.defs));
        if at == 2 && format!("{e:?}") == format!("{:?}", sp.e) {
            return Some(Expr::Empty);
        }
        if !reads(e) {
            return Some(e.clone());
        }
        Some(match e {
            Expr::Name(n) if *n == sp.f => of(at == 1 || at == 2),
            Expr::Name(n) if n == tname => of(at == 0),
            Expr::Name(n) => match self.defs.get(n) {
                Some(d) if !self.is_end_name(n) || n == tname => return self.in_span(d, sp, at),
                _ => return None,
            },
            Expr::Not(a) => simplify(Expr::Not(Box::new(self.in_span(a, sp, at)?))),
            Expr::And(a, b) => simplify(Expr::And(Box::new(self.in_span(a, sp, at)?), Box::new(self.in_span(b, sp, at)?))),
            Expr::Or(a, b) => simplify(Expr::Or(Box::new(self.in_span(a, sp, at)?), Box::new(self.in_span(b, sp, at)?))),
            Expr::Xor(a, b) => simplify(Expr::Xor(Box::new(self.in_span(a, sp, at)?), Box::new(self.in_span(b, sp, at)?))),
            // The byte before: the span's at `l + 1` and after, to `c`; `T`'s at `l + 1`.
            Expr::Call(f, args) if f == "prev" && matches!(args.get(1), None | Some(Arg::N(1))) => match args.first() {
                Some(Arg::E(Expr::Name(n))) if *n == sp.f => of(at == 2 || at == 3),
                Some(Arg::E(Expr::Name(n))) if n == tname => of(at == 1),
                _ => return None,
            },
            _ => return None,
        })
    }

    /// Whether `e` holds only at a region token's start (nowhere outside one, nor after its
    /// start).
    fn only_at_starts(&self, e: &Expr) -> bool {
        let n = self.opens.as_ref().map_or(0, Vec::len);
        matches!(self.in_region(e, None, 0), Some(Expr::Empty)) && (0..n).all(|k| matches!(self.in_region(e, Some(k), 1), Some(Expr::Empty)) && matches!(self.in_region(e, Some(k), 2), Some(Expr::Empty)))
    }

    /// Escape `why` of `e`, made of a span the token loop keeps, as its tests (`Span`): where
    /// it opens, at the starts in it, at its end.
    fn span_escape(&mut self, why: &str, e: &Expr, skip: Option<&Expr>) -> Result<bool, String> {
        for i in 0..self.spans.len() {
            let sp = &self.spans[i];
            let Some(places) = (0..5).map(|at| self.in_span(e, sp, at)).collect::<Option<Vec<Expr>>>() else { continue };
            let Ok([p0, p1, p2, p3, p4]) = <[Expr; 5]>::try_from(places) else { continue };
            if !matches!(p0, Expr::Empty) || !matches!(p4, Expr::Empty) {
                continue;
            }
            let stop = sp.e.clone();
            let mut memo = HashMap::new();
            let at_stop = self.bytes_at(&stop, 0, &mut memo);
            let skipped = skip.map(|k| self.bytes_at(k, 0, &mut memo)).unwrap_or([false; 256]);
            if (0..256).any(|b| at_stop[b] && skipped[b]) {
                continue;
            }
            let mut open = Vec::new();
            let mut starts = None;
            let mut at_end = None;
            let mut passed = None;
            // In it, from `l + 1` to `c`: a mask of stage 1 tested where it opens; one only at
            // a region's start, at the starts in it.
            let (e1, e2) = (matches!(p1, Expr::Empty), matches!(p2, Expr::Empty));
            if !self.regional(&p1) && !self.regional(&p2) {
                if !e1 && !e2 && format!("{p1:?}") == format!("{p2:?}") {
                    open.push(format!("any_in(&{}, l + 1, c - 1)", self.val(&p1)?));
                } else {
                    if !e1 {
                        open.push(format!("bit(&{}, l + 1)", self.val(&p1)?));
                    }
                    if !e2 {
                        open.push(format!("any_in(&{}, l + 2, c - 1)", self.val(&p2)?));
                    }
                }
            } else if (e1 || self.only_at_starts(&p1)) && (e2 || self.only_at_starts(&p2)) {
                let first = if e1 { None } else { self.at_start(&p1)? };
                let rest = if e2 { None } else { self.at_start(&p2)? };
                if (!e1 && first.is_none()) || (!e2 && rest.is_none()) {
                    continue;
                }
                starts = Some((first, rest));
            } else {
                continue;
            }
            // At `c`: a mask of stage 1 tested where it opens; else at a start there, and where
            // a token covers it, its value inside a token there.
            if !matches!(p3, Expr::Empty) {
                if !self.regional(&p3) {
                    open.push(format!("c < n && bit(&{}, c)", self.val(&p3)?));
                } else {
                    let Some(here) = self.at_start(&p3)? else { continue };
                    let n = self.opens.as_ref().map_or(0, Vec::len);
                    let mut inside = vec![self.in_region_of(&p3, None, 0, &Ctx::of(at_stop), &mut HashMap::new())];
                    for k in 0..n {
                        for part in 1..3 {
                            inside.push(self.in_region_of(&p3, Some(k), part, &Ctx::of(at_stop), &mut HashMap::new()));
                        }
                    }
                    let Some(inside) = inside.into_iter().collect::<Option<Vec<Expr>>>() else { continue };
                    let inside = inside.into_iter().reduce(|a, b| simplify(Expr::Or(Box::new(a), Box::new(b)))).unwrap_or(Expr::Empty);
                    if self.regional(&inside) {
                        continue;
                    }
                    at_end = Some(here);
                    // Its byte is one of `E`'s: where it is `E`, it holds.
                    if format!("{inside:?}") == format!("{stop:?}") {
                        passed = Some("true".to_owned());
                    } else if !matches!(inside, Expr::Empty) {
                        passed = Some(format!("bit(&{}, c)", self.val(&inside)?));
                    }
                }
            }
            let sp = &mut self.spans[i];
            if !open.is_empty() {
                sp.open.push((why.to_owned(), open.join(" || ")));
            }
            if let Some((first, rest)) = starts {
                sp.starts.push((why.to_owned(), first, rest));
            }
            if let Some(h) = at_end {
                sp.at_end.push((why.to_owned(), h));
            }
            if let Some(p) = passed {
                sp.passed.push((why.to_owned(), p));
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// Escape `why` of `e`, which reads a fill the loop keeps and holds only at a token's start
    /// (at no skipped byte, none in a token of code after its start, none in a region after its
    /// start): tested at each start.
    fn start_escape(&mut self, why: &str, e: &Expr, rows: &[(usize, &Stmt)]) -> Result<bool, String> {
        if !self.reads_fill(e) {
            return Ok(false);
        }
        let n = self.opens.as_ref().map_or(0, Vec::len);
        let Some(v) = self.in_region(e, None, 0) else { return Ok(false) };
        let at = self.bytes_at(&self.fills_as_all(&v), 0, &mut HashMap::new());
        if (0..256).any(|i| at[i] && self.skipped[i]) {
            return Ok(false);
        }
        let mut tails = Vec::new();
        for (row, st) in rows {
            let Stmt::Token { kind, last, rule, .. } = st else { continue };
            if matches!(rule, Some(TokenRule::Inside(..) | TokenRule::Read(_) | TokenRule::ReadParts(..))) {
                continue;
            }
            let tb = self.row_tail_bytes(kind, rule, last);
            if (0..256).any(|i| at[i] && tb[i]) {
                // Over the token's bytes after its first, where it is `A & prev(G)`.
                let Some(t) = self.tail_escape(e, *row)? else { return Ok(false) };
                if t != "false" {
                    tails.push((why.to_owned(), *row, t));
                }
            }
        }
        if !(0..n).all(|k| (1..3).all(|part| matches!(self.in_region(e, Some(k), part), Some(Expr::Empty)))) {
            return Ok(false);
        }
        let Some(here) = self.at_start(e)? else { return Ok(false) };
        if here != "false" {
            // Tested in each row it may hold at the start of, as the row's bytes read it.
            self.start_checks.push((why.to_owned(), e.clone(), at));
        }
        self.tail_checks.extend(tails);
        Ok(true)
    }

    /// `e`, `A & prev(G)` for `A` of stage 1 outside the regions and `G` a fill within `F`
    /// (`within_fill`), over a token of row `row`'s bytes after its first, where no `F` starts:
    /// `F` holds from the first byte to the first stop over them, `G` from the first byte or the
    /// first `Y` over them, so an `A` after one of them and up to that stop.
    fn tail_escape(&mut self, e: &Expr, row: usize) -> Result<Option<String>, String> {
        let mut factors = Vec::new();
        fn conj<'a>(e: &'a Expr, out: &mut Vec<&'a Expr>) {
            if let Expr::And(a, b) = e {
                conj(a, out);
                conj(b, out);
            } else {
                out.push(e);
            }
        }
        conj(e, &mut factors);
        let is_g = |g: &Gen, x: &Expr| match x {
            Expr::Call(p, pa) if p == "prev" && pa.len() == 1 => match &pa[0] {
                Arg::E(a) => g.fill_of(a).filter(|&i| g.fills[i].within.is_some()),
                _ => None,
            },
            _ => None,
        };
        let gs: Vec<usize> = factors.iter().filter_map(|x| is_g(self, x)).collect();
        let [i] = gs.as_slice() else { return Ok(None) };
        let i = *i;
        let mut a = Expr::Not(Box::new(Expr::Empty));
        for x in &factors {
            if is_g(self, x).is_none() {
                a = Expr::And(Box::new(a), Box::new((*x).clone()));
            }
        }
        let Some(a0) = self.in_region(&simplify_all(a), None, 0) else { return Ok(None) };
        if self.regional(&a0) || self.reads_fill(&a0) {
            return Ok(None);
        }
        let j = self.fills[i].within.unwrap_or_default();
        let (Some(fr), Some(gr)) = (self.fills[j].info.iter().find(|r| r.row == row), self.fills[i].info.iter().find(|r| r.row == row)) else { return Ok(None) };
        if fr.tail_starts || fr.region {
            return Ok(None);
        }
        let (f_s, g_s, stop) = (fr.at_s.clone(), gr.at_s.clone(), fr.tail_stop.clone());
        if matches!(f_s, Known::No) || matches!(a0, Expr::Empty) {
            return Ok(Some("false".to_owned()));
        }
        let av = self.val(&a0)?;
        let e1 = if matches!(stop, Known::No) {
            "l".to_owned()
        } else {
            format!("first_in(&{}, s + 1, l).unwrap_or(l)", self.val(&self.fills[j].stop0.clone())?)
        };
        let y = self.fills[i].start0.clone();
        let from_y = if matches!(y, Expr::Empty) { Known::No } else { Known::Test(format!("first_in(&{}, s + 1, e1).is_some_and(|y1| any_in(&{av}, y1 + 1, e1))", self.val(&y)?)) };
        let t = f_s.and(Known::Test(format!("{{ let e1 = {e1}; {} }}", g_s.and(Known::Test(format!("any_in(&{av}, s + 1, e1)"))).or(from_y).code())));
        Ok(Some(t.code()))
    }

    /// Escape `why` of `e`, each of its terms `prev(X, j) & Y` where `X` holds in no region's part and
    /// `Y` is a mask of stage 1: at a byte `j` after one outside the region tokens, `prev(X₀, j) & Y`
    /// (`X₀` its value there); else none. So it is tested over each stretch of code between region
    /// tokens, shifted by `j`, as the loop reaches a region token and after the loop.
    fn stretch_escape(&mut self, why: &str, e: &Expr) -> Result<bool, String> {
        if self.opens.is_none() || !self.regional(e) {
            return Ok(false);
        }
        let mut terms = vec![e.clone()];
        let mut flat = Vec::new();
        while let Some(t) = terms.pop() {
            match t {
                Expr::Or(a, b) => {
                    terms.push(*a);
                    terms.push(*b);
                }
                t => flat.push(t),
            }
        }
        let none = |v: Option<Expr>| matches!(v, Some(Expr::Empty));
        let mut out = Vec::new();
        let mut on_tokens = Vec::new();
        for t in flat {
            let mut cs = vec![t];
            let mut conj = Vec::new();
            while let Some(c) = cs.pop() {
                match c {
                    Expr::And(a, b) => {
                        cs.push(*a);
                        cs.push(*b);
                    }
                    c => conj.push(c),
                }
            }
            let (reg, rest): (Vec<Expr>, Vec<Expr>) = conj.into_iter().partition(|c| self.regional(c));
            let [Expr::Call(f, args)] = reg.as_slice() else { return Ok(false) };
            let (Some(Arg::E(x)), j) = (args.first(), match args.get(1) {
                None => 1,
                Some(Arg::N(j)) => *j as i64,
                _ => return Ok(false),
            }) else { return Ok(false) };
            if f != "prev" || j < 1 {
                return Ok(false);
            }
            // `X` in no part of any region.
            for k in 0..self.kinds.len() {
                if !none(self.in_region(x, Some(k), 0)) || !none(self.in_region(x, Some(k), 1)) || !self.closer_parts(x, k).is_some_and(|p| p.iter().all(|(_, v)| v.iter().all(|v| matches!(v, Expr::Empty)))) {
                    return Ok(false);
                }
            }
            let Some(x0) = self.in_region(x, None, 0).filter(|v| !self.regional(v)) else { return Ok(false) };
            let mut y = Expr::Not(Box::new(Expr::Empty));
            for r in &rest {
                y = Expr::And(Box::new(y), Box::new(r.clone()));
            }
            on_tokens.push(Expr::And(Box::new(x0.clone()), Box::new(Expr::Call("next".to_owned(), vec![Arg::E(simplify(y)), Arg::N(j as _)]))));
            let mut t = Expr::Call("prev".to_owned(), vec![Arg::E(x0), Arg::N(j as _)]);
            for r in rest {
                t = Expr::And(Box::new(t), Box::new(r));
            }
            out.push((j, simplify(t)));
        }
        // Where no `X₀` is a skipped byte, each holds at a byte of a token of code: tested over
        // each such token's bytes, `X₀ & next(Y, j)`, as the lean lexer reads the token.
        let skipped = self.skipped;
        if on_tokens.iter().all(|t| {
            let Expr::And(x0, _) = t else { return false };
            let b = self.bytes_at(x0, 0, &mut HashMap::new());
            !(0..256).any(|i| b[i] && skipped[i])
        }) {
            let mut v = Expr::Empty;
            for t in on_tokens {
                v = if matches!(v, Expr::Empty) { t } else { Expr::Or(Box::new(v), Box::new(t)) };
            }
            let v = simplify(v);
            let name = self.val(&v)?;
            self.code_checks.push((why.to_owned(), name, v));
            return Ok(true);
        }
        let mut code = Vec::new();
        for (j, t) in out {
            code.push((j, self.val(&t)?));
        }
        self.stretch_checks.push((why.to_owned(), code));
        Ok(true)
    }

    /// `e` with each mask made of the regions written out by its definition, down to `code`, which
    /// is `code_l`; `None` where one is not so.
    fn of_code(&self, e: &Expr) -> Option<Expr> {
        if !self.regional(e) {
            return Some(e.clone());
        }
        Some(match e {
            Expr::Name(n) if n == "code" => Expr::Name("code_l".to_owned()),
            Expr::Name(n) => self.of_code(self.defs.get(n)?)?,
            Expr::Not(a) => Expr::Not(Box::new(self.of_code(a)?)),
            Expr::And(a, b) => Expr::And(Box::new(self.of_code(a)?), Box::new(self.of_code(b)?)),
            Expr::Or(a, b) => Expr::Or(Box::new(self.of_code(a)?), Box::new(self.of_code(b)?)),
            Expr::Xor(a, b) => Expr::Xor(Box::new(self.of_code(a)?), Box::new(self.of_code(b)?)),
            Expr::Call(f, args) => Expr::Call(f.clone(), args.iter().map(|a| match a {
                Arg::E(x) => self.of_code(x).map(Arg::E),
                a => Some(a.clone()),
            }).collect::<Option<Vec<_>>>()?),
            Expr::Empty | Expr::Test { .. } => return None,
        })
    }

    /// Escape `why` of `e`, made of the regions through `code` alone: after the loop, of the bytes
    /// outside its region tokens, which it records (`code_l`). The test build checks the record is
    /// the regions' `code`.
    fn code_escape(&mut self, why: &str, e: &Expr) -> Result<bool, String> {
        if self.opens.is_none() || !self.defs.contains_key("code") {
            return Ok(false);
        }
        let Some(x) = self.of_code(e) else { return Ok(false) };
        if !self.regional.contains("code_l") {
            self.regional.insert("code_l".to_owned());
            self.names.insert("code_l".to_owned(), "code_l".to_owned());
        }
        let before = std::mem::take(&mut self.body);
        let v = self.val(&x);
        let lines = std::mem::replace(&mut self.body, before);
        let v = v?;
        let _ = write!(self.code_after, "{lines}    // {why}\n    refused |= any(&{v});\n");
        Ok(true)
    }

    /// Whether `e` is the file's last byte (`~next(~empty)`, or a mask of it): it is never in a
    /// region that closes, whose closer comes after it.
    fn is_last_byte(&self, e: &Expr) -> bool {
        match e {
            Expr::Name(n) => self.defs.get(n).is_some_and(|d| self.is_last_byte(d)),
            Expr::Not(a) => matches!(&**a, Expr::Call(f, args) if f == "next" && args.len() == 1 && matches!(&args[0], Arg::E(Expr::Not(x)) if matches!(**x, Expr::Empty))),
            _ => false,
        }
    }

    /// Escape `why` of `e`, made of the regions, as tests where each region's token reads it:
    /// where it holds nowhere outside a region token, and only in regions a token reads.
    fn region_escape(&mut self, why: &str, e: &Expr, read: &[usize], skip: Option<&Expr>) -> Result<bool, String> {
        let Some(n) = self.opens.as_ref().map(|o| o.len()) else { return Ok(false) };
        // Outside every region token: none, or a mask of stage 1 tested over each token of code
        // (the loop's bytes but the skipped ones, which it must hold at none of).
        let outside = match self.in_region(e, None, 0) {
            Some(Expr::Empty) => None,
            Some(v) if !self.regional(&v) => {
                let at = self.bytes_at(&v, 0, &mut HashMap::new());
                let skipped = skip.map(|k| self.bytes_at(k, 0, &mut HashMap::new())).unwrap_or([false; 256]);
                // A skipped byte it may hold at: not one test a token.
                if (0..256).any(|i| at[i] && skipped[i]) {
                    return Ok(false);
                }
                Some(v)
            }
            _ => return Ok(false),
        };
        let mut found = Vec::new();
        // A region that closes has its closer after the file's last byte: at its start and in it, never.
        let kept = |x: Expr, closer: bool| if matches!(x, Expr::Empty) || (!closer && self.is_last_byte(&x)) { None } else { Some(x) };
        for k in 0..n {
            let mut parts: [Option<Expr>; 3] = [None, None, None];
            // The body: one mask, or one at each of its first bytes and one after.
            let mut body: Vec<(i64, Option<i64>, Expr)> = Vec::new();
            for part in 0..3u8 {
                match self.in_region(e, Some(k), part) {
                    Some(x) if part == 1 => {
                        if let Some(x) = kept(x, false) {
                            body.push((1, None, x));
                        }
                    }
                    Some(x) => parts[usize::from(part)] = kept(x, part == 2),
                    None if part == 1 => {
                        let Some(v) = self.body_parts(e, k) else { return Ok(false) };
                        for (i, x) in v.into_iter().enumerate() {
                            let q = i as i64 + 1;
                            let Some(x) = kept(x, false) else { continue };
                            match body.last_mut() {
                                Some((_, hi, y)) if format!("{y:?}") == format!("{x:?}") && *hi == Some(q - 1) => *hi = if q == 5 { None } else { Some(q) },
                                _ => body.push((q, if q == 5 { None } else { Some(q) }, x)),
                            }
                        }
                    }
                    None => return Ok(false),
                }
            }
            if parts.iter().any(Option::is_some) || !body.is_empty() {
                if !read.contains(&k) {
                    return Ok(false);
                }
                found.push((k, parts, body));
            }
        }
        if self.region_checks.len() < n {
            self.region_checks.resize(n, Vec::new());
        }
        for (k, parts, body) in found {
            let mut v: [Option<String>; 2] = [None, None];
            for (i, x) in [&parts[0], &parts[2]].into_iter().enumerate() {
                if let Some(x) = x {
                    v[i] = Some(self.val(x)?);
                }
            }
            let mut b = Vec::new();
            for (lo, hi, x) in body {
                b.push((lo, hi, self.val(&x)?));
            }
            self.region_checks[k].push(RegionCheck { why: why.to_owned(), at: v[0].clone(), body: b, closer: v[1].clone() });
        }
        if let Some(v) = outside {
            let name = self.val(&v)?;
            self.code_checks.push((why.to_owned(), name, v));
        }
        Ok(true)
    }

    /// The tests of region `kind`'s escapes in its token's read, once its closer `c` and its last
    /// byte `last` are known; empty for none.
    fn region_tests(&self, kind: &str) -> String {
        let Some(k) = self.opens.as_ref().and_then(|o| o.iter().position(|(q, _, _)| q == kind)) else { return String::new() };
        let mut out = String::new();
        for RegionCheck { why, at, body, closer } in self.region_checks.get(k).map(Vec::as_slice).unwrap_or(&[]) {
            let mut tests = Vec::new();
            match (at, body.as_slice()) {
                // The same mask at the start and after it: one range.
                (Some(a), [(1, None, i)]) if a == i => tests.push(format!("any_in(&{a}, s, c - 1)")),
                _ => {
                    if let Some(m) = at {
                        tests.push(format!("bit(&{m}, s)"));
                    }
                    for (lo, hi, m) in body {
                        tests.push(match hi {
                            None => format!("any_in(&{m}, s + {lo}, c - 1)"),
                            Some(h) if h == lo => format!("(s + {lo} < c && bit(&{m}, s + {lo}))"),
                            Some(h) => format!("any_in(&{m}, s + {lo}, (s + {h}).min(c - 1))"),
                        });
                    }
                }
            }
            if let Some(m) = closer {
                tests.push(format!("any_in(&{m}, c, last)"));
            }
            let _ = write!(out, "// {why}\nif {} {{\n    refused = true;\n    break;\n}}\n", tests.join(" || "));
        }
        // Tier 2's fields the loop writes, over this token's parts.
        for fw in &self.field_writes {
            let FieldPart { why, at, body, closer, grid } = &fw.parts[k];
            let f = &fw.var;
            // A part none: nothing where the field starts as none, else its bits cleared.
            let w = |m: &Option<String>, a: String, b: String| match m.as_deref() {
                None => None,
                Some("0") => Some(format!("clear(&mut {f}, {a}, {b});")),
                Some("!") => Some(format!("set(&mut {f}, {a}, {b} + 1);")),
                Some(m) => Some(format!("put(&mut {f}, &{m}, {a}, {b});")),
            };
            let mut lines = Vec::new();
            lines.extend(w(at, "s".into(), "s".into()));
            for (lo, hi, m) in body {
                let b = match hi {
                    None => "c - 1".to_owned(),
                    Some(h) => format!("(s + {h}).min(c - 1)"),
                };
                lines.extend(w(m, format!("s + {lo}"), b));
            }
            // A body by both ends: its value from 5 on at both, then the bytes near each end.
            if let Some(g) = grid {
                let far = &g[4][4];
                let as_out = |m: &str| fw.outside.as_deref().unwrap_or("0") == m;
                let put = |m: &str, a: String, b: String| match m {
                    "0" => format!("clear(&mut {f}, {a}, {b});"),
                    "!" => format!("set(&mut {f}, {a}, {b} + 1);"),
                    m => format!("put(&mut {f}, &{m}, {a}, {b});"),
                };
                if !as_out(far) {
                    lines.push(put(far, "s + 1".into(), "c - 1".into()));
                }
                // Known by the closer alone: each byte near it.
                let by_end = g[..4].iter().all(|row| *row == g[4]);
                for q in (1..=4usize).filter(|_| !by_end) {
                    let row = &g[q - 1];
                    if row.iter().all(|m| m == far) {
                        continue;
                    }
                    let b = format!("s + {q}");
                    // The same wherever the closer is.
                    if row.iter().all(|m| *m == row[0]) {
                        lines.push(format!("if {b} < c {{\n    {}\n}}", put(&row[0], b.clone(), b.clone())));
                        continue;
                    }
                    let b = format!("s + {q}");
                    let mut m = format!("if {b} < c {{\n    match c - s - {q} {{\n");
                    for (t, v) in row.iter().enumerate() {
                        if v != far {
                            let arm = if t == 4 { "_".to_owned() } else { (t + 1).to_string() };
                            let _ = writeln!(m, "        {arm} => {{ {} }}", put(v, b.clone(), b.clone()));
                        }
                    }
                    if row[4] == *far {
                        m.push_str("        _ => {}\n");
                    }
                    m.push_str("    }\n}");
                    lines.push(m);
                }
                for t in 1..=4usize {
                    let v = &g[4][t - 1];
                    if v != far {
                        lines.push(format!("if c > s + {} {{\n    {}\n}}", if by_end { t } else { 4 + t }, put(v, format!("c - {t}"), format!("c - {t}"))));
                    }
                }
            }
            // The closer: its bytes' values, by where it is after the token's first.
            let close = |v: &[Option<String>]| -> Vec<String> {
                if v.iter().all(|m| *m == v[0]) {
                    return w(&v[0], "c".into(), "last".into()).into_iter().collect();
                }
                let at = |r: usize| if r == 0 { "c".to_owned() } else { format!("c + {r}") };
                v.iter().enumerate().filter_map(|(r, m)| w(m, at(r), if r == 0 { "c".to_owned() } else { format!("(c + {r}).min(last)") })).collect()
            };
            let arms: Vec<(Option<i64>, Vec<String>)> = closer.iter().map(|(d, v)| (*d, close(v))).filter(|(d, b)| d.is_some() || !b.is_empty()).collect();
            match arms.as_slice() {
                [] => {}
                [(None, b)] => lines.extend(b.iter().cloned()),
                [(Some(d), b)] => lines.push(format!("if c - s == {d} {{\n    {}\n}}", b.join("\n    "))),
                _ => {
                    let mut m = String::from("match c - s {\n");
                    for (d, b) in &arms {
                        let _ = writeln!(m, "    {} => {{\n        {}\n    }}", d.map_or("_".to_owned(), |d| d.to_string()), b.join("\n        "));
                    }
                    if arms.iter().all(|(d, _)| d.is_some()) {
                        m.push_str("    _ => {}\n");
                    }
                    m.push('}');
                    lines.push(m);
                }
            }
            if !lines.is_empty() {
                let _ = writeln!(out, "// {why}\n{}", lines.join("\n"));
            }
        }
        out
    }

    /// Tier 2's fields (`readers`), each one mask of stage 1 where it is the same in every
    /// region's every part and outside them; else, where it is none outside them and known in
    /// each part, written by the token loop over each region token (`FieldWrite`). Each its
    /// code: the mask's or the loop's variable.
    fn plan_fields(&mut self, x: &Input) -> Result<Vec<(String, String)>, String> {
        let n = self.opens.as_ref().map_or(0, Vec::len);
        let mut out = Vec::new();
        for rd in readers(x.stmts) {
            for (f, e) in &rd.masks {
                if !self.regional(e) || n == 0 {
                    out.push((f.clone(), self.val(e)?));
                    continue;
                }
                let key = |v: &Expr| format!("{v:?}");
                let outside = self.in_region(e, None, 0).filter(|v| !self.regional(v));
                type Per = (Expr, Vec<(i64, Option<i64>, Expr)>, Vec<(Option<i64>, Vec<Expr>)>, Option<Vec<Vec<Expr>>>);
                let mut per: Vec<Per> = Vec::new();
                for k in 0..n {
                    let (Some(p0), Some(p2)) = (self.in_region(e, Some(k), 0), self.closer_parts(e, k)) else { break };
                    let (body, grid) = match self.in_region(e, Some(k), 1) {
                        Some(b) => (vec![(1, None, b)], None),
                        None => match self.body_parts(e, k) {
                            Some(v) => (v.into_iter().enumerate().map(|(i, b)| (i as i64 + 1, if i == 4 { None } else { Some(i as i64 + 1) }, b)).collect(), None),
                            // By both ends.
                            None => match (1..=5).map(|q| (1..=5).map(|t| self.in_region_of(e, Some(k), body2(q, t), &Ctx::all(), &mut HashMap::new())).collect::<Option<Vec<_>>>()).collect::<Option<Vec<_>>>() {
                                Some(g) => (Vec::new(), Some(g)),
                                None => break,
                            },
                        },
                    };
                    if self.regional(&p0) || p2.iter().any(|(_, v)| v.iter().any(|v| self.regional(v))) || body.iter().any(|b| self.regional(&b.2)) || grid.iter().flatten().flatten().any(|v| self.regional(v)) {
                        break;
                    }
                    per.push((p0, body, p2, grid));
                }
                let (Some(outside), true) = (outside, per.len() == n) else {
                    out.push((f.clone(), self.val(e)?));
                    continue;
                };
                let ko = key(&outside);
                if per.iter().all(|(p0, body, p2, grid)| key(p0) == ko && p2.iter().all(|(_, v)| v.iter().all(|v| key(v) == ko)) && body.iter().all(|b| key(&b.2) == ko) && grid.iter().flatten().flatten().all(|v| key(v) == ko)) {
                    out.push((f.clone(), self.val(&outside)?));
                    continue;
                }
                let all = |v: &Expr| matches!(v, Expr::Not(x) if matches!(**x, Expr::Empty));
                let var = format!("fw_{}", ident(f));
                let outside = if matches!(outside, Expr::Empty) {
                    None
                } else {
                    let id = self.node(&outside)?;
                    self.nodes[id].also.push(var.clone());
                    Some(self.nodes[id].name.clone())
                };
                let mut parts = Vec::new();
                for (p0, body, p2, grid) in per {
                    // `None`: as outside, nothing to write; `0`: none; `!`: all; else a mask.
                    let mut code = |v: &Expr| -> Result<Option<String>, String> {
                        Ok(match v {
                            v if key(v) == ko => None,
                            Expr::Empty => Some("0".to_owned()),
                            v if all(v) => Some("!".to_owned()),
                            v => Some(self.val(v)?),
                        })
                    };
                    let at = code(&p0)?;
                    let mut closer = Vec::new();
                    for (d, v) in p2 {
                        closer.push((d, v.iter().map(&mut code).collect::<Result<Vec<_>, _>>()?));
                    }
                    let mut b: Vec<(i64, Option<i64>, Option<String>)> = Vec::new();
                    for (lo, hi, v) in body {
                        let m = code(&v)?;
                        match b.last_mut() {
                            Some((_, h, y)) if *y == m && h.is_some_and(|h| h == lo - 1) => *h = hi,
                            _ => b.push((lo, hi, m)),
                        }
                    }
                    // The grid's each value as written: as outside is the outside mask's.
                    let mut g = None;
                    if let Some(grid) = grid {
                        let mut rows = Vec::new();
                        for row in &grid {
                            let mut r = Vec::new();
                            for v in row {
                                r.push(code(v)?.unwrap_or_else(|| outside.clone().unwrap_or_else(|| "0".to_owned())));
                            }
                            rows.push(r);
                        }
                        g = Some(rows);
                    }
                    parts.push(FieldPart { why: format!("`{f}`, for tier 2"), at, body: b, closer, grid: g });
                }
                self.field_writes.push(FieldWrite { var: var.clone(), parts, outside });
                out.push((f.clone(), var));
            }
        }
        Ok(out)
    }

    /// The walk of `regions H { … }`, as the lean lexer reads strings and comments: from the
    /// start, the first opener not hidden (the first kind declared, at one byte) opens its region,
    /// which fills to its closer and hides the bytes after its opener through its closer's last.
    fn regions(&mut self, rg: &::bitlex::syntax::Regions) -> Result<(), String> {
        let ks = &rg.kinds;
        self.kinds = ks.clone();
        self.hidden = rg.hidden.clone();
        let mut opens = Vec::new();
        let mut closers = Vec::new();
        let mut ors = Vec::new();
        for r in ks {
            opens.push(self.val(&r.opener)?);
            closers.push(self.val(&r.closers())?);
            ors.push(match &r.or {
                Some((c2, m)) => Some((self.val(c2)?, *m)),
                None => None,
            });
        }
        if ks.iter().all(|r| r.empty.is_none()) {
            let mut v = Vec::new();
            for (k, r) in ks.iter().enumerate() {
                let after = r.after.iter().map(|a| ks.iter().position(|q| q.kind == *a).ok_or_else(|| format!("tier 1 as Rust: region `{}` after no region `{a}`", r.kind))).collect::<Result<_, _>>()?;
                v.push((r.kind.clone(), opens[k].clone(), after));
            }
            self.opens = Some(v);
        }
        let hidden = format!("m_{}", ident(&rg.hidden));
        let _ = writeln!(self.body, "    // regions {} {{ … }}: the first opener not hidden opens its region, to its closer.", rg.hidden);
        let _ = writeln!(self.body, "    let mut {hidden} = vec![0u64; nw];");
        for r in ks {
            let _ = writeln!(self.body, "    let mut m_o_{0} = vec![0u64; nw];\n    let mut m_f_{0} = vec![0u64; nw];", ident(&r.kind));
            if let Some((e, _)) = &r.empty {
                let _ = writeln!(self.body, "    let mut m_{} = vec![0u64; nw];", ident(e));
            }
            if let Some((_, e)) = &r.nests {
                let _ = writeln!(self.body, "    let mut m_{} = vec![0u64; nw];", ident(e));
            }
        }
        let _ = writeln!(self.body, "    {{\n        let mut at = 0usize;\n        // The kind of string the template is in: the region closed last at its second closer (a template's `$`), of the kinds with no `after`.\n        let mut template = usize::MAX;\n        // Each kind's next opener, at or after where it was looked for (`n`: none).");
        if let Some(n) = &rg.nest {
            let (o, c) = (self.val(&n.opener)?, self.val(&n.closer)?);
            let deep = format!("m_{}", ident(&n.deep));
            let _ = writeln!(self.body, "        // The nest (decision 0621): its levels, each a template's kind or a bracket's `usize::MAX`, innermost last; the place of the last template's `{{`.\n        let mut nest = [usize::MAX; {}];\n        let mut depth = 0usize;\n        let mut tb = usize::MAX;", n.depth);
            self.body = self.body.replacen("    {\n        let mut at = 0usize;", &format!("    let mut {deep} = vec![0u64; nw];\n    {{\n        let mut at = 0usize;"), 1);
            self.nest = Some((o, c, n.depth, n.deep.clone()));
        }
        let firsts: Vec<String> = opens.iter().map(|o| format!("first_bit(&{o}, 0).unwrap_or(n)")).collect();
        let _ = writeln!(self.body, "        let mut nx = [{}];", firsts.join(", "));
        let _ = writeln!(self.body, "        loop {{\n            let (mut p, mut k) = (n, usize::MAX);");
        for (k, r) in ks.iter().enumerate() {
            let look = format!("            if nx[{k}] < at {{\n                nx[{k}] = first_bit(&{}, at).unwrap_or(n);\n            }}\n            if nx[{k}] < p {{\n                p = nx[{k}];\n                k = {k};\n            }}\n", opens[k]);
            if r.after.is_empty() {
                let _ = write!(self.body, "            // `{}`\n{look}", r.kind);
            } else {
                let after: Vec<String> = r.after.iter().map(|a| ks.iter().position(|q| q.kind == *a).map(|q| format!("template == {q}")).ok_or_else(|| format!("tier 1 as Rust: region `{}` after no region `{a}`", r.kind))).collect::<Result<_, _>>()?;
                let _ = write!(self.body, "            // `{}`, after {}\n            if {} {{\n{}            }}\n", r.kind, r.after.join(" "), after.join(" || "), indent(&look, "    "));
            }
        }
        if let Some((o, c, d, deep)) = &self.nest {
            let deep = format!("m_{}", ident(deep));
            let _ = writeln!(self.body, "            // The nest: a bracket's `{{` (not a template's) or a level's closer before the next opener, where a level is open.\n            if depth > 0 {{\n                let b = match first_bit(&{o}, at) {{\n                    Some(x) if x == tb => first_bit(&{o}, x + 1).unwrap_or(n),\n                    x => x.unwrap_or(n),\n                }};\n                let c = first_bit(&{c}, at).unwrap_or(n);\n                if b < p && b < c {{\n                    if depth == {d} {{\n                        set(&mut {deep}, b, b + 1);\n                    }} else {{\n                        nest[depth] = usize::MAX;\n                        depth += 1;\n                    }}\n                    template = usize::MAX;\n                    at = b + 1;\n                    continue;\n                }}\n                if c < p {{\n                    depth -= 1;\n                    template = if depth > 0 {{ nest[depth - 1] }} else {{ usize::MAX }};\n                    at = c + 1;\n                    continue;\n                }}\n            }}");
        }
        let _ = writeln!(self.body, "            match k {{");
        for (k, r) in ks.iter().enumerate() {
            let (o, f) = (format!("m_o_{}", ident(&r.kind)), format!("m_f_{}", ident(&r.kind)));
            let c = &closers[k];
            // Its closer past the opener's bytes (`Region::opens`).
            let past = r.opens;
            let mut arm = String::new();
            let close = if r.outside {
                format!("                    let c = first_bit(&{c}, p + 1).unwrap_or(n);\n                    set(&mut {f}, p, c);\n                    set(&mut {hidden}, p + 1, c);\n                    at = c.max(p + 1);\n")
            } else {
                let last = match &ors[k] {
                    Some((c2, m)) if *m != r.bytes => format!("if bit(&{c2}, c) {{ c + {} }} else {{ c + {} }}", m - 1, r.bytes - 1),
                    Some(_) | None => format!("c + {}", r.bytes - 1),
                };
                // With a nest, a template's level: pushed, its `{` the byte after (`tb`).
                let push = |q: usize| match &self.nest {
                    Some((_, _, d, deep)) => format!("if depth == {d} {{\n                                        set(&mut m_{}, c + 1, c + 2);\n                                    }} else {{\n                                        nest[depth] = {q};\n                                        depth += 1;\n                                    }}\n                                    tb = c + 1;\n                                    template = {q};", ident(deep)),
                    None => format!("template = {q};"),
                };
                let resumed = r.after.first().and_then(|a| ks.iter().position(|q| q.kind == *a)).unwrap_or(usize::MAX);
                // A plain kind's close at its second closer: a template opens in it.
                let last = match &ors[k] {
                    Some((c2, _)) if r.after.is_empty() => format!("{{\n                                if bit(&{c2}, c) {{\n                                    {}\n                                }}\n                                {last}\n                            }}", push(k)),
                    // With a nest, a resuming kind's close at its second closer: its string's template.
                    Some((c2, _)) if r.resumes && self.nest.is_some() => format!("{{\n                                if bit(&{c2}, c) {{\n                                    {}\n                                }}\n                                {last}\n                            }}", push(resumed)),
                    // A resuming kind's close at its own closer: the string ended, no template open.
                    Some((c2, _)) if r.resumes => format!("{{\n                                if !bit(&{c2}, c) {{\n                                    template = usize::MAX;\n                                }}\n                                {last}\n                            }}"),
                    _ => last,
                };
                let search = self.closer_from(r, &format!("p + {past}"), true)?.replace('\n', "\n        ");
                format!("                    match {search} {{\n                        Some(c) => {{\n                            let last = {last};\n                            set(&mut {f}, p, c);\n                            set(&mut {hidden}, p + 1, last + 1);\n                            at = last + 1;\n                        }}\n                        None => {{\n                            set(&mut {f}, p, n);\n                            set(&mut {hidden}, p + 1, n);\n                            at = n;\n                        }}\n                    }}\n")
            };
            // With a nest, a kind with `after` opens at the closer of its template's level.
            let pop = if self.nest.is_some() && !r.after.is_empty() { "                    depth -= 1;\n                    template = if depth > 0 { nest[depth - 1] } else { usize::MAX };\n" } else { "" };
            let _ = write!(arm, "                    set(&mut {o}, p, p + 1);\n{pop}{close}");
            let body = match &r.empty {
                // `else E bytes M`: with no closer after it in the file, no region: `E`, its first `M` bytes.
                Some((e, m)) => format!("                    if first_bit(&{c}, p + 1).is_none() {{\n                        set(&mut m_{e}, p, p + 1);\n                        set(&mut {hidden}, p + 1, p + {m});\n                        at = (p + {m}).max(p + 1);\n                    }} else {{\n{}                    }}\n", indent(&arm, "    "), e = ident(e)),
                None => arm,
            };
            let _ = write!(self.body, "                {k} => {{\n                    // `{} = {} .. {}`\n{body}                }}\n", r.kind, show(&r.opener), match &r.or { Some((c2, _)) => format!("{} or {}", show(&r.closer), show(c2)), None => show(&r.closer) });
        }
        let _ = writeln!(self.body, "                _ => break,\n            }}\n        }}\n    }}");
        self.names.insert(rg.hidden.clone(), hidden);
        self.regional.insert(rg.hidden.clone());
        // The nest's `else`: no mask of the loop's (it refuses where it would push past its levels).
        if let Some(n) = &rg.nest {
            self.names.insert(n.deep.clone(), format!("m_{}", ident(&n.deep)));
        }
        for r in ks {
            self.names.insert(format!("o_{}", r.kind), format!("m_o_{}", ident(&r.kind)));
            self.names.insert(format!("f_{}", r.kind), format!("m_f_{}", ident(&r.kind)));
            self.regional.insert(format!("o_{}", r.kind));
            self.regional.insert(format!("f_{}", r.kind));
            if let Some((e, _)) = &r.empty {
                self.names.insert(e.clone(), format!("m_{}", ident(e)));
                self.regional.insert(e.clone());
            }
            // A kind that nests: its `else`, no mask of the loop's (it refuses past its levels).
            if let Some((_, e)) = &r.nests {
                self.names.insert(e.clone(), format!("m_{}", ident(e)));
            }
        }
        Ok(())
    }

    /// The record kind of row `row`: its own, or its first byte's.
    fn kind_of(&self, row: usize) -> String {
        match self.x.row_kinds.get(row).copied().flatten() {
            Some(k) => format!("{k}"),
            None => "u32::from(KINDS[usize::from(src[s])])".to_owned(),
        }
    }

    /// A region's last byte from its closer's first, `c`, of a token read by its reader: a closer
    /// of a kind of its own (`closes`) its bytes, else the region's.
    /// Whether a row's read of region `k` is never one byte: it looks for the closer from the
    /// byte after the first, and its last byte is the closer's or after it, or, where it ends
    /// before its closer, the opener's second byte is no closer's.
    fn read_never_one_byte(&mut self, rule: &Option<TokenRule>, k: usize) -> bool {
        let kind = self.kinds[k].kind.clone();
        let r = match rule {
            Some(TokenRule::Read(rs) | TokenRule::ReadParts(rs, _)) => rs.iter().find(|r| r.kind == kind).cloned(),
            _ => None,
        };
        let Some(r) = r else { return false };
        if !r.outside {
            return true;
        }
        let opener = self.kinds[k].opener.clone();
        let o = self.bytes_at(&opener, 1, &mut HashMap::new());
        let c = self.bytes_at(&r.closers(), 0, &mut HashMap::new());
        !(0..256).any(|i| o[i] && c[i])
    }

    /// A read of a region of kind `kind` to its closer `c` (`found`, its last byte): where a kind
    /// with `after` names it, a close at its second closer opens a template in it (`template`), as
    /// the lean lexer's string reader keeps the string's kind over the template's code.
    /// Region `r`'s closer from byte `from` on, an `Option<usize>`: the first of its closers, or,
    /// where it nests itself (`Region::nests`), the first with no level open, an opener in it (not
    /// at a closer) opening one, as the lean lexer's `block_comment_end` counts. Past its levels
    /// the token loop refuses (`None`); the walk (`deep`) marks its `else` and opens none, as the
    /// rounds do.
    fn closer_from(&mut self, r: &Region, from: &str, deep: bool) -> Result<String, String> {
        let c = self.val(&r.closers())?;
        let Some((d, e)) = r.nests.clone() else { return Ok(format!("first_bit(&{c}, {from})")) };
        let io = self.val(&Expr::And(Box::new(r.opener.clone()), Box::new(Expr::Not(Box::new(r.closers())))))?;
        let past = if deep { format!("set(&mut m_{}, o + 1, o + 2);", ident(&e)) } else { "break None;".to_owned() };
        Ok(format!("{{
                // `{k}` nests: an opener in it opens a level, a closer closes the innermost.
                let mut q = {from};
                let mut lv = 0usize;
                loop {{
                    let Some(c) = first_bit(&{c}, q) else {{ break None }};
                    if let Some(o) = first_bit(&{io}, q).filter(|&o| o < c) {{
                        if lv == {d} {{
                            {past}
                        }} else {{
                            lv += 1;
                        }}
                        q = o + 1;
                        continue;
                    }}
                    if lv == 0 {{
                        break Some(c);
                    }}
                    lv -= 1;
                    q = c + 1;
                }}
            }}", k = r.kind))
    }

    fn template_close(&mut self, kind: &str, found: String) -> Result<String, String> {
        let Some(k) = self.kinds.iter().position(|q| q.kind == kind) else { return Ok(found) };
        let named = self.kinds.iter().any(|q| q.after.iter().any(|a| a == kind));
        let Some((c2, _)) = self.kinds[k].or.clone().filter(|_| named && self.kinds[k].after.is_empty()) else { return Ok(found) };
        let c2 = self.val(&c2)?;
        // Resumed (decision 0600): this token's record, the next pushed, stays open, over any
        // open already (a template in a template's code, decision 0621) as deep as the nest.
        let resumed = self.kinds.iter().any(|q| q.resumes && q.after.iter().any(|a| a == kind));
        let open = if resumed { "\n                        if opened == held.len() {\n                            refused = true;\n                            break;\n                        }\n                        held[opened] = starts.len();\n                        opened += 1;" } else { "" };
        let push = match &self.nest {
            Some((_, _, d, _)) => format!("\n                        if depth == {d} {{\n                            refused = true;\n                            break;\n                        }}\n                        nest[depth] = {k};\n                        depth += 1;"),
            None => String::new(),
        };
        Ok(format!("{{\n                    if bit(&{c2}, c) {{\n                        template = {k};{push}{open}\n                    }}\n                    {}\n                }}", found.trim_start_matches(' ')))
    }

    fn closer_last(&mut self, r: &Region, closes: &[(String, Expr, u8)]) -> Result<String, String> {
        let mut out = String::new();
        let k = self.kinds.iter().position(|q| q.kind == r.kind);
        for (_, e, n) in closes {
            // At the closer's first byte, the mask as the region knows it there.
            let at = k.and_then(|k| self.closer_parts(e, k)).and_then(|p| {
                let first: Vec<String> = p.iter().map(|(_, v)| format!("{:?}", v[0])).collect();
                first.iter().all(|f| *f == first[0]).then(|| p[0].1[0].clone())
            });
            let at = at.filter(|v| !self.regional(v));
            if matches!(at, Some(Expr::Empty)) {
                continue;
            }
            let v = self.val(at.as_ref().unwrap_or(e))?;
            let _ = write!(out, "if bit(&{v}, c) {{ c + {} }} else ", n - 1);
        }
        let own = match &r.or {
            Some((c2, m)) if *m != r.bytes => format!("if bit(&{}, c) {{ c + {} }} else {{ c + {} }}", self.val(c2)?, m - 1, r.bytes - 1),
            Some(_) | None => format!("c + {}", r.bytes - 1),
        };
        Ok(if out.is_empty() { own } else { format!("{out}{{ {own} }}") })
    }

    /// The region a row reads, by its start (`start & o_K`), or of its rule's one.
    fn row_region<'r>(start: &Expr, rs: &'r [Region]) -> Option<&'r Region> {
        if let [r] = rs {
            return Some(r);
        }
        let mut terms = vec![start];
        while let Some(t) = terms.pop() {
            match t {
                Expr::And(a, b) => {
                    terms.push(a);
                    terms.push(b);
                }
                Expr::Name(n) => {
                    if let Some(r) = rs.iter().find(|r| *n == format!("o_{}", r.kind)) {
                        return Some(r);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn source(mut self) -> Result<String, String> {
        let x = self.x;
        // The classes: stage 1's, by its nibble tables (simdjson's `classify`).
        for s in x.stmts {
            if let Stmt::Class(name, items) = s {
                let mut set = [false; 256];
                for &(lo, hi) in items {
                    for c in lo..=hi {
                        set[c as usize] = true;
                    }
                }
                self.class(name, &set);
                self.class_bytes.insert(name.clone(), set);
            }
        }
        let mut skip: Option<Expr> = None;
        let mut marks: Vec<&Stmt> = Vec::new();
        let mut rows: Vec<(usize, &Stmt)> = Vec::new();
        let mut row = 0usize;
        let mut escapes: Vec<(&str, &Expr)> = Vec::new();
        let mut token_escapes: Vec<&Stmt> = Vec::new();
        for s in x.stmts {
            match s {
                Stmt::Lexer(_) | Stmt::Class(..) | Stmt::Keywords { .. } | Stmt::TokenError(..) | Stmt::Tree { .. } | Stmt::Assume { .. } | Stmt::AssumeEnds { .. } => {}
                // A mask not made of the regions: stage 1's.
                Stmt::Mask(name, e) if !self.regional(e) => {
                    self.defs.insert(name.clone(), e.clone());
                    let id = match e {
                        Expr::Name(_) => self.node(e)?,
                        _ => {
                            let key = format!("{e:?}");
                            match self.node_of.get(&key) {
                                Some(&id) => id,
                                None => self.make_node(key, format!("m_{}", ident(name)), format!("mask {name} = {}", show(e)), e)?,
                            }
                        }
                    };
                    self.node_of.insert(format!("={name}"), id);
                }
                Stmt::Mask(name, e) => {
                    self.regional.insert(name.clone());
                    self.defs.insert(name.clone(), e.clone());
                    let _ = writeln!(self.body, "    // mask {name} = {}", show(e));
                    match e {
                        Expr::Name(n) => {
                            let v = self.val(&Expr::Name(n.clone()))?;
                            self.names.insert(name.clone(), v);
                        }
                        _ => {
                            let local = format!("m_{}", ident(name));
                            self.make(&local, e)?;
                            self.names.insert(name.clone(), local);
                        }
                    }
                }
                Stmt::Rounds { regions: Some(rg), .. } => self.regions(rg)?,
                Stmt::Escape(why, e) | Stmt::Error(why, e) => escapes.push((why, e)),
                Stmt::TokenEscape { .. } => token_escapes.push(s),
                Stmt::Skip(e) => {
                    skip = Some(match skip.take() {
                        Some(k) => Expr::Or(Box::new(k), Box::new(e.clone())),
                        None => e.clone(),
                    })
                }
                Stmt::Token { .. } => {
                    rows.push((row, s));
                    row += 1;
                }
                Stmt::Mark { .. } | Stmt::TokenMark { .. } => marks.push(s),
                other => return Err(format!("tier 1 as Rust: `{}` is not written yet", crate::bitlex_masks::statement(other))),
            }
        }
        // The escapes and errors: a bit of any sends the file to tree-sitter.
        // Those not made of the regions, stage 1 tests a block at a time.
        // Those made of the regions where they hold only in a region a token reads: tests there.
        let read: Vec<usize> = match &self.opens {
            Some(o) => x
                .stmts
                .iter()
                .filter_map(|s| match s {
                    Stmt::Token { rule: Some(TokenRule::Read(rs) | TokenRule::ReadParts(rs, _)), .. } => Some(rs),
                    _ => None,
                })
                .flatten()
                .filter_map(|r| o.iter().position(|(k, _, _)| *k == r.kind))
                .collect(),
            None => Vec::new(),
        };
        // The fills the token loop keeps: `fill(prev(T), E)`, `T` assumed at a region token's
        // last byte, `E` stage 1's.
        if self.opens.is_some() {
            for s in x.stmts {
                let Stmt::Mask(name, Expr::Call(f, args)) = s else { continue };
                let [Arg::E(Expr::Call(p, pa)), Arg::E(stop)] = args.as_slice() else { continue };
                let [Arg::E(t @ Expr::Name(_))] = pa.as_slice() else { continue };
                // A stop that is regions' openers (`o_K | …`): the rows reading those regions end it.
                let mut ends_at = Vec::new();
                if self.regional(stop) {
                    fn openers(e: &Expr, out: &mut Vec<String>) -> bool {
                        match e {
                            Expr::Or(a, b) => openers(a, out) && openers(b, out),
                            Expr::Name(n) => n.strip_prefix("o_").is_some_and(|k| {
                                out.push(k.to_owned());
                                true
                            }),
                            _ => false,
                        }
                    }
                    let mut ks = Vec::new();
                    if !openers(stop, &mut ks) || !ks.iter().all(|k| self.kinds.iter().any(|q| q.kind == *k)) {
                        continue;
                    }
                    for (row, st) in &rows {
                        if let Stmt::Token { rule: Some(TokenRule::Read(rs) | TokenRule::ReadParts(rs, _)), .. } = st
                            && rs.iter().any(|r| ks.contains(&r.kind))
                        {
                            ends_at.push(*row);
                        }
                    }
                    if ends_at.is_empty() {
                        continue;
                    }
                }
                if !(f == "fill" && p == "prev" && self.is_end(t)) {
                    continue;
                }
                // Where it opens: `T` at the last byte of each row's region token, none after it.
                let mut opens_at = Vec::new();
                let mut known = true;
                for (row, st) in &rows {
                    let Stmt::Token { start, rule, .. } = st else { continue };
                    let regions: Vec<(Option<usize>, Region, Vec<(String, Expr, u8)>)> = match rule {
                        Some(TokenRule::Read(rs)) => rs.iter().map(|r| (if rs.len() > 1 { self.kinds.iter().position(|q| q.kind == r.kind) } else { None }, r.clone(), Vec::new())).collect(),
                        Some(TokenRule::ReadParts(rs, parts)) => match Self::row_region(start, rs) {
                            Some(r) => vec![(None, r.clone(), parts.closes.clone())],
                            None => {
                                known = false;
                                break;
                            }
                        },
                        _ => continue,
                    };
                    let mut v = Known::No;
                    for (k, r, closes) in &regions {
                        let Some(here) = self.end_value(t, r, closes)? else {
                            known = false;
                            break;
                        };
                        v = v.or(match k {
                            Some(k) => Known::Test(format!("open == {k}")).and(here),
                            None => here,
                        });
                    }
                    if !known {
                        break;
                    }
                    if !matches!(v, Known::No) {
                        opens_at.push((*row, v.code()));
                    }
                }
                if known {
                    self.spans.push(Span { f: name.clone(), t: t.clone(), e: stop.clone(), opens_at, ends_at, open: Vec::new(), starts: Vec::new(), at_end: Vec::new(), passed: Vec::new() });
                }
            }
        }
        // The fills the loop keeps (`LoopFill`): the masks and the escapes' fills.
        if self.opens.is_some() {
            self.skipped = match &skip {
                Some(k) => self.bytes_at(k, 0, &mut HashMap::new()),
                None => [false; 256],
            };
            let mut exprs: Vec<(Option<String>, Expr)> = Vec::new();
            for s in x.stmts {
                if let Stmt::Mask(name, e) = s {
                    exprs.push((Some(name.clone()), e.clone()));
                    let mut inner = Vec::new();
                    fills_in(e, &mut inner);
                    exprs.extend(inner.into_iter().filter(|f| format!("{f:?}") != format!("{e:?}")).map(|f| (None, f)));
                }
            }
            for (_, e) in &escapes {
                let mut inner = Vec::new();
                fills_in(e, &mut inner);
                exprs.extend(inner.into_iter().map(|f| (None, f)));
            }
            self.find_ends(&rows)?;
            self.find_loop_fills(&rows, &exprs)?;
        }
        let mut refuse = Vec::new();
        for (why, e) in &escapes {
            // Past the nest's levels: the loop refuses where it would push one more.
            if let (Expr::Name(x), Some((.., deep))) = (e, &self.nest)
                && x == deep
            {
                continue;
            }
            // Past a kind's own levels, likewise (`Region::nests`).
            if let Expr::Name(x) = e
                && self.kinds.iter().any(|r| r.nests.as_ref().is_some_and(|(_, d)| d == x))
            {
                continue;
            }
            if !self.regional(e) {
                let id = self.node(e)?;
                self.sinks.push((why.to_string(), id));
                continue;
            }
            if self.span_escape(why, e, skip.as_ref())? {
                continue;
            }
            // At the file's last byte, a fill the loop keeps: tested after the loop.
            if let Expr::And(a, b) = e
                && let Some(i) = [(a, b), (b, a)].iter().find_map(|(l, f)| if self.is_last_byte(l) { self.fill_of(f) } else { None })
            {
                self.end_checks.push((why.to_string(), i));
                continue;
            }
            if self.stretch_escape(why, e)? {
                continue;
            }
            if self.start_escape(why, e, &rows)? {
                continue;
            }
            if self.region_escape(why, e, &read, skip.as_ref())? {
                continue;
            }
            if self.code_escape(why, e)? {
                continue;
            }
            let v = self.val(e)?;
            refuse.push(format!("    // {why}\n    refused |= any(&{v});\n"));
        }
        let _ = writeln!(self.body, "    let mut refused = escapes;");
        for r in &refuse {
            self.body.push_str(r);
        }
        // The gap marks: a mark of the code, its mask's bits since the grammar token before.
        struct Gap {
            bit: u8,
            mask: Option<String>,
            unless: Vec<u32>,
            name: String,
            /// The loop's accumulator of its mask (`GapAcc`), where it keeps one.
            acc: Option<usize>,
            /// A mask that holds at a grammar token's start where an accumulator's held since the
            /// record before (`rec_J`): `A & prev(fill(M, ~W))`, `W` every skipped byte, `M` the
            /// accumulator's (a token first on its line, after a line break), and `A` its test at
            /// the start (none where it holds at every start but an extra's, where no gap mark is
            /// written).
            rec: Option<(usize, Option<String>)>,
        }
        /// A gap mark's mask made of the regions, kept by the loop as it reads (the lean lexer's
        /// "a line break since the token before"): whether it held since the grammar token before
        /// (`acc_J`) and since the record before (`rec_J`). Its mask over the skipped bytes, over
        /// a token of code's bytes (none in a region's after its start), and at a token's start.
        struct GapAcc {
            gap: Option<String>,
            code: Expr,
            here: String,
            own: bool,
            what: String,
        }
        let mut accs: Vec<GapAcc> = Vec::new();
        let mut acc_of: HashMap<String, usize> = HashMap::new();
        // The fills the loop keeps whose value over the gap a gap mask reads.
        let mut fill_used: std::collections::BTreeSet<usize> = Default::default();
        let skipped = match &skip {
            Some(k) => self.bytes_at(k, 0, &mut HashMap::new()),
            None => [false; 256],
        };
        self.skipped = skipped;
        let mut gaps: Vec<Gap> = Vec::new();
        // Each mark's token tests: the kind, `after`, the test, the mask, the least, the bit.
        let mut tests: Vec<(String, bool, TokenTest, Expr, u32, u8)> = Vec::new();
        for (m, s) in marks.iter().enumerate() {
            let Stmt::Mark { kind, at, unless, unplaced, checks, .. } = s else { continue };
            let Some(bit) = x.mark_bits.get(m).copied().flatten() else { continue };
            if unplaced.is_some() {
                return Err(format!("tier 1 as Rust: mark {kind}'s `else escape` is not written yet"));
            }
            // A mask made of the regions: kept by the loop where it is known in each place.
            let mut acc = None;
            if !matches!(at, Expr::Empty) && self.regional(at) && self.opens.is_some() {
                let key = format!("{at:?}");
                if let Some(&j) = acc_of.get(&key) {
                    acc = Some(j);
                } else {
                    let n = self.opens.as_ref().map_or(0, Vec::len);
                    let gap = self.in_region_of(at, None, 0, &Ctx::of(skipped), &mut HashMap::new());
                    let code = self.in_region(at, None, 0);
                    let inside = (0..n).all(|k| (1..3).all(|part| matches!(self.in_region(at, Some(k), part), Some(Expr::Empty))));
                    let here = self.at_start(at)?;
                    // Over a token of code after its start, a mask that reads a fill the loop keeps
                    // must hold at none of its bytes.
                    let code = match code {
                        Some(c) if self.reads_fill(&c) => {
                            let b = self.bytes_at(&self.fills_as_all(&c), 0, &mut HashMap::new());
                            let clear = rows.iter().all(|(_, st)| match st {
                                Stmt::Token { kind, last, rule, .. } if !matches!(rule, Some(TokenRule::Inside(..) | TokenRule::Read(_) | TokenRule::ReadParts(..))) => {
                                    let tb = self.row_tail_bytes(kind, rule, last);
                                    !(0..256).any(|i| b[i] && tb[i])
                                }
                                _ => true,
                            });
                            if clear { Some(Expr::Empty) } else { None }
                        }
                        c => c,
                    };
                    let gap = match gap {
                        Some(g) => self.gap_code(&g, &mut fill_used)?,
                        None => None,
                    };
                    if let (Some(gap), Some(code), true, Some(here)) = (gap, code, inside, here)
                        && !self.regional(&code)
                    {
                        let gap = if gap == "false" { None } else { Some(gap) };
                        let what = self.brief(at);
                        accs.push(GapAcc { gap, code, here, own: false, what });
                        acc_of.insert(key, accs.len() - 1);
                        acc = Some(accs.len() - 1);
                    }
                }
                if let Some(j) = acc
                    && x.own >> bit & 1 == 1
                {
                    accs[j].own = true;
                }
            }
            // `A & prev(fill(M, ~W))`: the gap since the record before is skipped bytes alone, so
            // the fill holds before a start where `M` held in it.
            let mut rec = None;
            if acc.is_none()
                && let Expr::And(a, b) = at
            {
                for (st, f) in [(a, b), (b, a)] {
                    let Expr::Call(p, pa) = &**f else { continue };
                    let [Arg::E(fl)] = pa.as_slice() else { continue };
                    let Expr::Call(fname, fa) = fl else { continue };
                    let [Arg::E(m), Arg::E(Expr::Not(w))] = fa.as_slice() else { continue };
                    if p != "prev" || fname != "fill" {
                        continue;
                    }
                    let wb = self.bytes_at(w, 0, &mut HashMap::new());
                    let covers = (0..256).all(|i| !skipped[i] || wb[i]);
                    if let (Some(&j), true, Some(t)) = (acc_of.get(&format!("{m:?}")), covers, self.at_start(st)?)
                        && t != "false"
                    {
                        accs[j].own = true;
                        // `A`'s conjuncts that read no region, tested at the start; one that reads
                        // them holds at every start (a token's first byte is code or an opener's).
                        let mut parts = Vec::new();
                        let mut stack = vec![(**st).clone()];
                        while let Some(e) = stack.pop() {
                            match e {
                                Expr::And(a, b) => {
                                    stack.push(*a);
                                    stack.push(*b);
                                }
                                e if !self.regional(&e) => parts.push(format!("bit(&{}, s)", self.val(&e)?)),
                                _ => {}
                            }
                        }
                        let _ = t;
                        rec = Some((j, (!parts.is_empty()).then(|| parts.join(" && "))));
                    }
                }
            }
            let mask = match at {
                Expr::Empty => None,
                _ if acc.is_some() || rec.is_some() => None,
                // A mask of the regions is made only by the regions' walk, which only a test build
                // makes (`gate_walk`): the loop must keep it, or the product build reads a name
                // it never makes.
                e if self.regional(e) && self.opens.is_some() => {
                    return Err(format!("tier 1 as Rust: mark {kind}'s mask `{}` is of the regions, and the loop does not keep it", self.brief(e)));
                }
                e => Some(self.val(e)?),
            };
            let unless = unless
                .iter()
                .map(|w| x.words.iter().position(|x| x == w).map(|i| i as u32 + 1).ok_or_else(|| format!("tier 1 as Rust: mark {kind}: `unless after` a word no code names")))
                .collect::<Result<_, _>>()?;
            for c in checks {
                tests.push((c.kind.clone(), c.after, c.test, c.expr.clone(), c.least, bit));
            }
            gaps.push(Gap { bit, mask, unless, name: kind.clone(), acc, rec });
        }
        let skip = match &skip {
            Some(e) => self.val(e)?,
            None => return Err("tier 1 as Rust: a token loop needs `skip`".into()),
        };
        let extras: Vec<&String> = {
            let mut v = Vec::new();
            for s in x.stmts {
                if let Stmt::Tree { body, .. } = s {
                    for t in body {
                        if let Stmt::Extra(ks) = t {
                            v.extend(ks.iter());
                        }
                    }
                }
            }
            v
        };
        // Tier 2's fields: the loop writes those its region tokens know (`plan_fields`).
        let field_plan = self.plan_fields(x)?;
        // The token loop: each top row's branch.
        let mut pair_fns = String::new();
        let mut checked: Vec<String> = Vec::new();
        // The gap accumulators a row's bytes after its start may hold.
        let mut tails: std::collections::BTreeSet<usize> = Default::default();
        // Each row's start, its test build's checks and its code: picked by `dispatch`.
        let mut row_arms: Vec<RowArm> = Vec::new();
        for (row, s) in &rows {
            let Stmt::Token { kind, start, last, rule, .. } = s else { continue };
            if matches!(rule, Some(TokenRule::Inside(..))) {
                continue;
            }
            let mut st = match self.at_start(start)? {
                Some(t) => t,
                None => format!("bit(&{}, s)", self.val(start)?),
            };
            // The at-starts assumptions the prover did not prove, checked in a test build where the
            // loop first reads them: at the first row whose start reads the mask, the rows before
            // it not starting here.
            let mut check = String::new();
            for (scope, name, e, proved) in self.x.assumed {
                if *proved || scope.is_some() || checked.contains(name) || !reads_mask(start, name, &self.defs) {
                    continue;
                }
                checked.push(name.clone());
                let Some(at) = self.at_start(e)? else { continue };
                let m = self.val(&Expr::Name(name.clone()))?;
                let _ = write!(check, "\n            // `assume … at starts {name} = {}`, not proved: a test build checks it.\n            #[cfg(feature = \"test-hooks\")]\n            assert_eq!(bit(&{m}, s), {at}, \"assume {name} at byte {{s}}\");", self.brief(e));
            }
            if !check.is_empty() {
                st = format!("{{{check}\n            {st}\n        }}");
            }
            let k = self.kind_of(*row);
            let (how, l) = match rule {
                Some(TokenRule::Run(r)) => {
                    // Its run as its kind's assumptions read it (`in KIND`, decision 0552).
                    let these: Vec<(&str, &Expr)> = x.assumed.iter().filter(|a| a.0.as_ref() == Some(kind)).map(|a| (a.1.as_str(), &a.2)).collect();
                    let read = if these.is_empty() { None } else { Some(substitute(r, &these, &self.defs)) };
                    match read {
                        Some(read) if !self.regional(&read) => {
                            let v = self.val(&read)?;
                            let proved = x.assumed.iter().filter(|a| a.0.as_ref() == Some(kind)).all(|a| a.3);
                            let check = if proved { String::new() } else { format!(";\n            // Not proved: a test build checks it against the rule over the file.\n            #[cfg(feature = \"test-hooks\")]\n            assert_eq!(run_last(&{}, s), run_last(&{v}, s), \"assume in {kind} at byte {{s}}\")", self.val(r)?) };
                            (format!("run {}, read as {} (assumed)", show(r), self.brief(&read)), format!("run_last(&{v}, s){check}"))
                        }
                        _ => (format!("run {}", show(r)), format!("run_last(&{}, s)", self.val(r)?)),
                    }
                }
                Some(TokenRule::RunOnce(r, d)) => (format!("run {} once {}", show(r), show(d)), format!("run_once_last(&{}, &{}, s)", self.val(r)?, self.val(d)?)),
                Some(TokenRule::Pair(p)) => (format!("pair {}", show(p)), format!("s + usize::from(bit(&{}, s))", self.val(p)?)),
                Some(TokenRule::Pairs { pairs, .. }) => {
                    let f = format!("pair_{row}");
                    let mut two = Vec::new();
                    let mut three = Vec::new();
                    for p in pairs {
                        match p.as_slice() {
                            [a, b] => two.push(format!("[{a:#04x}, {b:#04x}]")),
                            [a, b, c] => three.push(format!("[{a:#04x}, {b:#04x}, {c:#04x}]")),
                            _ => return Err(format!("tier 1 as Rust: `{kind}`'s pair of {} bytes", p.len())),
                        }
                    }
                    let third = if three.is_empty() { "0".to_owned() } else { format!("usize::from(matches!([a, b, c], {}))", three.join(" | ")) };
                    let _ = write!(pair_fns, "/// `{kind}`'s last byte from its first, `s`: its next where the two are a pair, and the one after where the three are a triple, as the lean lexer reads an operator.\nfn {f}(src: &[u8], s: usize) -> usize {{\n    let at = |i: usize| src.get(i).copied().unwrap_or(0);\n    let (a, b, c) = (at(s), at(s + 1), at(s + 2));\n    match [a, b] {{\n        {} => s + 1 + {third},\n        _ => s,\n    }}\n}}\n\n", if two.is_empty() { "[_, _] if false".to_owned() } else { two.join(" | ") });
                    ("pair".to_owned(), format!("{f}(src, s)"))
                }
                Some(TokenRule::Read(rs)) => {
                    let mut code = String::new();
                    for (i, r) in rs.iter().enumerate() {
                        let o = match self.opens.as_ref().and_then(|v| v.iter().position(|(k, _, _)| *k == r.kind)) {
                            Some(k) => format!("open == {k}"),
                            None => format!("bit(&{}, s)", self.val(&Expr::Name(format!("o_{}", r.kind)))?),
                        };
                        let c = self.val(&r.closers())?;
                        let tests = self.region_tests(&r.kind);
                        let one = if r.outside {
                            // To its closer, or the file's end, outside it.
                            if tests.is_empty() {
                                format!("first_bit(&{c}, s + 1).unwrap_or(n) - 1")
                            } else {
                                format!("{{\n                let c = first_bit(&{c}, s + 1).unwrap_or(n);\n                let last = c - 1;\n{}                last\n            }}", indent(&tests, "                "))
                            }
                        } else {
                            let found = self.closer_last(r, &[])?;
                            let some = if tests.is_empty() { found } else { format!("{{\n                    let last = {found};\n{}                    last\n                }}", indent(&tests, "                    ")) };
                            let some = self.template_close(&r.kind, some)?;
                            let some = if !self.keeps_close(*row, rule) { some } else { with_close(&some) };
                            let search = self.closer_from(r, &format!("s + {}", r.opens), false)?;
                            format!("match {search} {{\n                Some(c) => {some},")
                                + &format!("\n                None => {{\n                    refused = true;\n                    break;\n                }}\n            }}")
                        };
                        if rs.len() == 1 {
                            code = one;
                        } else {
                            let _ = write!(code, "{}if {o} {{\n                {}\n            }} ", if i == 0 { "" } else { "else " }, indent(&one, "    ").trim_start());
                        }
                    }
                    if rs.len() > 1 {
                        code.push_str("else {\n                refused = true;\n                break;\n            }");
                    }
                    (format!("read {}", rs.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>().join(" | ")), code)
                }
                Some(TokenRule::ReadParts(rs, parts)) => {
                    let r = Self::row_region(start, rs).ok_or_else(|| format!("tier 1 as Rust: `{kind}`'s row reads no one region"))?.clone();
                    let c = self.val(&r.closers())?;
                    let found = self.closer_last(&r, &parts.closes)?;
                    let tests = self.region_tests(&r.kind);
                    let found = if tests.is_empty() { found } else { format!("{{\n                    let last = {found};\n{}                    last\n                }}", indent(&tests, "                    ")) };
                    let found = self.template_close(&r.kind, found)?;
                    let found = if !self.keeps_close(*row, rule) { found } else { with_close(&found) };
                    let search = self.closer_from(&r, &format!("s + {}", r.opens), false)?;
                    let _ = &c;
                    (format!("read {} parts, to its closer", r.kind), format!("match {search} {{\n                Some(c) => {found},\n                None => {{\n                    refused = true;\n                    break;\n                }}\n            }}"))
                }
                None => match last {
                    Some(l) => (format!("last {}", show(l)), format!("first_bit(&{}, s).unwrap_or(s)", self.val(l)?)),
                    None => ("one byte".to_owned(), "s".to_owned()),
                },
                Some(TokenRule::Inside(..)) => unreachable!(),
            };
            // Its kind's extra bits; its marks' token tests.
            let extra = extras.contains(&kind);
            let mut bits = 0u32;
            if extra {
                if let Some((b, _)) = x.extra_bits.first() {
                    bits |= 1 << b;
                }
                for (b, k) in x.extra_bits.iter().skip(1) {
                    if k == kind {
                        bits |= 1 << b;
                    }
                }
            }
            let mut hits = String::new();
            for (tk, after, test, v, least, bit) in &tests {
                if tk != kind {
                    continue;
                }
                let v = self.row_val(v, rule)?;
                let holds = match test {
                    TokenTest::Count => format!("count_in(&{v}, s, l) >= {least}"),
                    TokenTest::Run => format!("l + 1 - s >= {least} && first_clear(&{v}, s, s + {least}).is_none()"),
                    TokenTest::All => format!("l + 1 - s >= {least} && first_clear(&{v}, s, l + 1).is_none()"),
                };
                let at = if *after { 16 + u32::from(*bit) } else { u32::from(*bit) };
                let _ = write!(hits, "            if {holds} {{\n                hits |= 1 << {at};\n            }}\n");
            }
            // The gap masks the loop keeps, over a token of code's bytes after its start.
            if !matches!(rule, Some(TokenRule::Read(_) | TokenRule::ReadParts(..))) {
                let tail = self.row_tail_bytes(kind, rule, last);
                for (j, a) in accs.iter().enumerate() {
                    let at = self.bytes_at(&a.code, 0, &mut HashMap::new());
                    if (0..256).any(|i| at[i] && tail[i]) {
                        let v = self.val(&a.code)?;
                        let _ = write!(hits, "            tail_{j} = any_in(&{v}, s + 1, l);\n");
                        tails.insert(j);
                    }
                }
            }
            // The fills the loop keeps that open after this token.
            for i in 0..self.spans.len() {
                let sp = &self.spans[i];
                if !sp.ends_at.is_empty() {
                    // Its end is a later token's start: open to it, checked there.
                    for (r, cond) in &sp.opens_at {
                        if r == row {
                            let _ = write!(hits, "            // `{}`: from the byte after `{}`, to the next of `{}`, a token's start.\n            if {cond} {{\n                span_{i} = (l + 1, usize::MAX);\n            }}\n", sp.f, show(&sp.t), show(&sp.e));
                        }
                    }
                    continue;
                }
                let stop = self.val(&self.spans[i].e.clone())?;
                let sp = &self.spans[i];
                for (r, cond) in &sp.opens_at {
                    if r != row {
                        continue;
                    }
                    let _ = write!(hits, "            // `{}`: from the byte after `{}`, to the next of `{}`.\n            if {cond} {{\n                let c = first_bit(&{stop}, l + 2).unwrap_or(n);\n", sp.f, show(&sp.t), show(&sp.e));
                    for (why, t) in &sp.open {
                        let _ = write!(hits, "                // {why}\n                if {t} {{\n                    refused = true;\n                    break;\n                }}\n");
                    }
                    let _ = write!(hits, "                span_{i} = (l + 1, c);\n            }}\n");
                }
            }
            // An at-ends mask the prover did not prove: a test build checks it holds at no byte
            // of a token but a region token's last.
            let region_row = matches!(rule, Some(TokenRule::Read(_) | TokenRule::ReadParts(..)));
            for (m, proved) in self.x.assumed_ends {
                if *proved {
                    continue;
                }
                let v = self.val(&Expr::Name(m.clone()))?;
                let to = if region_row { "l - 1" } else { "l" };
                let _ = write!(hits, "            // `assume … at ends {m}`, not proved: a test build checks it.\n            #[cfg(feature = \"test-hooks\")]\n            assert!(!any_in(&{v}, at, {to}), \"assume {m} at ends, by byte {{l}}\");\n");
            }
            // The fills the loop keeps, at this token's last byte.
            for (i, f) in self.fills.iter().enumerate() {
                if let Some((_, fl, xl)) = f.rows.iter().find(|r| r.0 == *row) {
                    let _ = write!(hits, "            fill_{i} = {fl};\n");
                    if f.prev {
                        let _ = write!(hits, "            xl_{i} = {xl};\n");
                    }
                }
            }
            // The escapes over the token's bytes after its first.
            for (why, r, t) in &self.tail_checks {
                if r == row {
                    let _ = write!(hits, "            // {why}: over the token's bytes after its first.\n            if {t} {{\n                refused = true;\n                break;\n            }}\n");
                }
            }
            // The escapes outside the regions, over a token of code's bytes: at its first byte and
            // over the bytes after, where each may hold among the bytes about it.
            if !region_row {
                let sb = self.bytes_at(start, 0, &mut HashMap::new());
                let sb: [bool; 256] = std::array::from_fn(|i| sb[i] && !self.skipped[i]);
                let tb = self.row_tail_bytes(kind, rule, last);
                let in_token: [bool; 256] = std::array::from_fn(|i| sb[i] || tb[i]);
                let mut at_tail = Ctx::of(tb);
                at_tail.0[3] = in_token;
                for (why, v, e) in &self.code_checks {
                    let first = self.may_hold(e, &Ctx::of(sb));
                    let after = (0..256).any(|i| tb[i]) && self.may_hold(e, &at_tail);
                    let test = match (first, after) {
                        (false, false) => continue,
                        (true, false) => format!("bit(&{v}, s)"),
                        (false, true) => format!("any_in(&{v}, s + 1, l)"),
                        (true, true) => format!("any_in(&{v}, s, l)"),
                    };
                    let _ = write!(hits, "            // {why}\n            if {test} {{\n                refused = true;\n                break;\n            }}\n");
                }
            }
            for t in &token_escapes {
                let Stmt::TokenEscape { why, kind: tk, test, expr, least } = t else { continue };
                if tk != kind {
                    continue;
                }
                let v = self.row_val(expr, rule)?;
                let holds = match test {
                    TokenTest::Count => format!("count_in(&{v}, s, l) >= {least}"),
                    TokenTest::Run => format!("l + 1 - s >= {least} && first_clear(&{v}, s, s + {least}).is_none()"),
                    TokenTest::All => format!("l + 1 - s >= {least} && first_clear(&{v}, s, l + 1).is_none()"),
                };
                let _ = write!(hits, "            // {why}\n            if {holds} {{\n                refused = true;\n                break;\n            }}\n");
            }
            let _ = &st;
            // The escapes at a token's start that may hold at this row's, as its bytes read them.
            let mut first_tests: Vec<(String, String)> = Vec::new();
            let sb = self.bytes_at(start, 0, &mut HashMap::new());
            let sb: [bool; 256] = std::array::from_fn(|i| sb[i] && !self.skipped[i]);
            for (why, e, at) in self.start_checks.clone() {
                if !(0..256).any(|i| sb[i] && at[i]) {
                    continue;
                }
                self.start_in = Some(sb);
                let t = self.at_start(&e);
                self.start_in = None;
                let Some(t) = t? else { return Err(format!("tier 1 as Rust: the escape `{why}` at a start of `{kind}`")) };
                if t != "false" {
                    first_tests.push((why, t));
                }
            }
            let tail = if matches!(rule, Some(TokenRule::Read(_) | TokenRule::ReadParts(..))) { [true; 256] } else { self.row_tail_bytes(kind, rule, last) };
            // A read that resumes a string's record (decision 0600): no record, its last the open
            // one's, which a close at its template keeps open, as the lean lexer's string reader
            // goes on after the template's code; then the loop on past it, as after a record.
            let resume = match rule {
                Some(TokenRule::ReadParts(rs, _)) if rs.iter().all(|r| r.resumes) => {
                    let again = match rs.first().and_then(|r| r.or.clone()) {
                        // Closed at its quote: the string ended, and no template is open (`template`).
                        // The row's own region's string (`tail | rtail`: one a row).
                        Some((c2, _)) => match (self.nest.clone(), Self::row_region(start, rs).and_then(|r| r.after.first()).and_then(|a| self.kinds.iter().position(|q| q.kind == *a))) {
                            // With a nest the `}` closed its template's level (before the row); a
                            // close at its `$` opens its string's next.
                            (Some((_, _, d, _)), Some(q)) => format!("if bit(&{}, close) {{\n                        if depth == {d} {{\n                            refused = true;\n                            break 'tokens;\n                        }}\n                        nest[depth] = {q};\n                        depth += 1;\n                        template = {q};\n                    }} else {{\n                        opened -= 1;\n                    }}", self.val(&c2)?),
                            _ => format!("if !bit(&{}, close) {{\n                        opened -= 1;\n                        template = usize::MAX;\n                    }}", self.val(&c2)?),
                        },
                        None => "opened -= 1;".to_owned(),
                    };
                    Some(format!("            // Resumes the open string's record: no record of its own.\n            match opened {{\n                0 => {{\n                    refused = true;\n                    break;\n                }}\n                _ => {{\n                    lasts[held[opened - 1]] = l as u32;\n                    {again}\n                }}\n            }}\n            at = l + 1;\n            if at >= base + 64 {{\n                b = at / 64;\n                continue 'tokens;\n            }}\n            firsts &= !0u64 << (at - base);\n            continue;\n        }}"))
                }
                _ => None,
            };
            // A region token's bytes are none of `code_l` (`code_escape`), where one is read.
            let code_l = if matches!(rule, Some(TokenRule::Read(_) | TokenRule::ReadParts(..))) { "            //~code_l\n" } else { "" };
            row_arms.push(RowArm { resume, row: *row, first: sb, tail, starts: first_tests, start: start.clone(), check, body: format!("{{\n            // `{kind}`: {how}\n            //~defs\n            //~spans\n            let l = {l};\n            let l = l.max(s);\n{code_l}            //~passed\n{hits}            (l, {k}, {extra}, "), bits });
        }
        // Each gap mark's bit at a grammar token, and at an extra the marks it keeps of its own gap.
        let mut gap_code = String::new();
        let mut own_code = String::new();
        for g in &gaps {
            let since = match (&g.mask, g.acc) {
                _ if let Some((j, a)) = &g.rec => match a {
                    None => format!("tested >> {} & 1 == 1 || rec_{j}", g.bit),
                    Some(a) => format!("tested >> {} & 1 == 1 || rec_{j} && {a}", g.bit),
                },
                (_, Some(j)) => format!("tested >> {0} & 1 == 1 || acc_{j}{1}", g.bit, if accs[j].here == "false" { String::new() } else { format!(" || here_{j}") }),
                (Some(m), None) => format!("tested >> {0} & 1 == 1 || any_in(&{m}, from, s)", g.bit),
                (None, None) => format!("tested >> {} & 1 == 1", g.bit),
            };
            let unless = if g.unless.is_empty() { String::new() } else { format!(" && !matches!(before, {})", g.unless.iter().map(|w| w.to_string()).collect::<Vec<_>>().join(" | ")) };
            let _ = write!(gap_code, "                // `{}`\n                if ({since}){unless} {{\n                    marks |= 1 << {};\n                }}\n", g.name, g.bit);
            if x.own >> g.bit & 1 == 1 {
                if let Some(j) = g.acc {
                    let here = if accs[j].here == "false" { String::new() } else { format!(" || here_{j}") };
                    let _ = write!(own_code, "                if rec_{j}{here} {{\n                    marks |= 1 << {};\n                }}\n", g.bit);
                } else if let Some(m) = &g.mask {
                    let _ = write!(own_code, "                if any_in(&{m}, from_record, s) {{\n                    marks |= 1 << {};\n                }}\n", g.bit);
                }
            }
        }
        // The token marks of the code: each its mask's bit at a record's first byte, and its word.
        // A mark of rows alone is decided by the row the dispatch picks, where every row decides it
        // (`row_marks`).
        let mut token_code = String::new();
        let mut row_marks: Vec<(u32, Vec<usize>, Vec<usize>, String)> = Vec::new();
        for (bit, mask, w) in x.token_bits {
            let starts = |rs: &[usize]| rs.iter().filter_map(|&r| rows.iter().find(|q| q.0 == r)).filter_map(|(_, s)| if let Stmt::Token { start, .. } = s { Some(start.clone()) } else { None }).reduce(|a, b| Expr::Or(Box::new(a), Box::new(b)));
            let word = if *w == 0 { String::new() } else { format!("word == {w}") };
            let at = match mask {
                CodeMask::Zero => continue,
                CodeMask::WordAlone => None,
                CodeMask::Rows(a, b) => {
                    let Some(e) = starts(a) else { continue };
                    let e = match starts(b) {
                        Some(g) => Expr::And(Box::new(e), Box::new(g)),
                        None => e,
                    };
                    Some(match self.at_start(&e)? {
                        Some(t) => t,
                        None => format!("bit(&{}, s)", self.val(&e)?),
                    })
                }
            };
            if at.as_deref() == Some("false") {
                continue;
            }
            let at = at.filter(|t| t != "true");
            let test = [at, (!word.is_empty()).then_some(word)].into_iter().flatten().collect::<Vec<_>>().join(" && ");
            if test.is_empty() {
                return Err(format!("tier 1 as Rust: token mark bit {bit} holds at every record"));
            }
            if let CodeMask::Rows(a, b) = mask
                && *w == 0
            {
                row_marks.push((u32::from(*bit), a.clone(), b.clone(), test));
                continue;
            }
            let _ = write!(token_code, "        if {test} {{\n            marks |= 1 << {bit};\n        }}\n");
        }
        // The masks tier 2 reads (`readers`), as `plan_fields` made them.
        let mut fields: Vec<String> = Vec::new();
        let mut fields_init = String::new();
        for (f, v) in &field_plan {
            fields.push(f.clone());
            let _ = writeln!(fields_init, "            {f}: {v},");
        }
        let mut fw_decl = String::new();
        for fw in self.field_writes.iter().filter(|fw| fw.outside.is_none()) {
            let _ = writeln!(fw_decl, "    // `{}` for tier 2, written over each region token.\n    let mut {} = {};", fw.parts.first().map_or("", |p| p.why.as_str()).trim_start_matches('`').split('`').next().unwrap_or(""), fw.var, "vec![0u64; nw]");
        }
        // The region a token's start opens, as the walk opens them (`regions`): the first declared whose
        // opener holds there, a kind `after` others where the region opened last of those with none is one.
        let mut open_code = String::new();
        let (tables, picks, chains, row_opens) = self.dispatch(&row_arms)?;
        // Each mark of rows, at the row picked: a row of a set starts where the row picked is one,
        // and may where one is after it in a group's chain it is in (those before it do not start).
        for (bit, a, b, test) in &row_marks {
            let of = |p: usize, set: &[usize]| -> Option<bool> {
                if set.contains(&row_arms[p].row) {
                    return Some(true);
                }
                let after = chains.iter().filter(|c| c.contains(&p)).any(|c| c.iter().skip_while(|&&q| q != p).skip(1).any(|&q| set.contains(&row_arms[q].row)));
                if after { None } else { Some(false) }
            };
            let each: Vec<Option<bool>> = (0..row_arms.len()).map(|p| {
                let (x, y) = (of(p, a), if b.is_empty() { Some(true) } else { of(p, b) });
                match (x, y) {
                    (Some(false), _) | (_, Some(false)) => Some(false),
                    (Some(true), Some(true)) => Some(true),
                    _ => None,
                }
            }).collect();
            if each.iter().all(Option::is_some) {
                for (p, v) in each.iter().enumerate() {
                    if *v == Some(true) {
                        row_arms[p].bits |= 1 << bit;
                    }
                }
            } else {
                let _ = write!(token_code, "        if {test} {{\n            marks |= 1 << {bit};\n        }}\n");
            }
        }
        let _ = write!(open_code, "        // The token's first byte's group, by the dispatch tables (simdjson's `classify`).\n        let c = src[s];\n        let group = (DISPATCH_LO[usize::from(c & 15)] & DISPATCH_HI[usize::from(c >> 4)]).trailing_zeros();\n");
        if self.opens.is_some() && !picks.fused {
            let _ = write!(open_code, "        // The region a token here opens, as the walk opens them: the first declared whose opener holds, a kind after others where the string the template is in (`template`) is one; of those whose opener may be the byte's.\n        let open = match group {{\n{}            _ => usize::MAX,\n        }};\n", picks.open);
        }
        let mut span_decl = String::new();
        let mut span_after = String::new();
        // A span whose stop is a region's opener that the file ends in: its checks to the end.
        for (i, sp) in self.spans.iter().enumerate() {
            if sp.ends_at.is_empty() || sp.open.is_empty() {
                continue;
            }
            let mut t1 = String::new();
            for (why, t) in &sp.open {
                let _ = write!(t1, "        // {why}\n        if {t} {{\n            refused = true;\n        }}\n");
            }
            let _ = write!(span_after, "    // `{}` open at the file's end: to it.\n    if !refused && span_{i}.0 != usize::MAX && span_{i}.1 == usize::MAX {{\n        let (l, c) = (span_{i}.0 - 1, n);\n        let _ = (l, c);\n{t1}    }}\n", sp.f);
        }
        let mut acc_after = String::new();
        // The gap masks the loop keeps.
        for (j, a) in accs.iter().enumerate() {
            let what = &a.what;
            let _ = writeln!(span_decl, "    // Whether a gap mask (`{what}`) held since the grammar token before{}.\n    let mut acc_{j} = false;{}", if a.own { ", and since the record before" } else { "" }, if a.own { format!("\n    let mut rec_{j} = false;") } else { String::new() });
            if let Some(g) = &a.gap {
                let _ = write!(open_code, "        // The skipped bytes before the token, for the gap masks (none where there are none).\n        if {g} {{\n            acc_{j} = true;\n{}        }}\n", if a.own { format!("            rec_{j} = true;\n") } else { String::new() });
            }
            if a.here != "false" {
                if picks.fused {
                    // Each row's, as its region reads it.
                    let _ = writeln!(open_code, "        let here_{j}: bool;");
                } else {
                    let _ = writeln!(open_code, "        let here_{j} = {};", a.here);
                }
            }
            if tails.contains(&j) {
                let _ = writeln!(open_code, "        let mut tail_{j} = false;");
            }
            let tail = if tails.contains(&j) { format!("tail_{j}") } else { "false".to_owned() };
            let held = if a.here == "false" { format!("acc_{j}") } else { format!("(acc_{j} || here_{j})") };
            let now = if tail == "false" { format!("extra && {held}") } else { format!("if extra {{ {held} || {tail} }} else {{ {tail} }}") };
            let _ = write!(acc_after, "        // The gap mask `{what}`: on after a token the grammar skips; else from this token's bytes after its start.\n        acc_{j} = {now};\n{}", if a.own { format!("        rec_{j} = {tail};\n") } else { String::new() });
        }
        // The fills the loop keeps: each its first byte and its end (`usize::MAX`: none open), the
        // escapes at the starts in it, and at its end.
        if !self.spans.is_empty() || self.kinds.iter().any(|q| q.resumes) {
            span_decl.push_str("    // The closer the region token read ends at, where a fill the loop keeps opens after it, or a read resumes a string's.\n    let mut close: usize;\n");
        }
        for (i, sp) in self.spans.iter().enumerate() {
            let _ = writeln!(span_decl, "    // `{}`'s first byte and the byte after it (none: `usize::MAX`).\n    let mut span_{i} = (usize::MAX, usize::MAX);", sp.f);
            let refuse = |why: &str, t: &str, pad: &str| if t == "true" { format!("{pad}// {why}\n{pad}refused = true;\n{pad}break;\n") } else { format!("{pad}// {why}\n{pad}if {t} {{\n{pad}    refused = true;\n{pad}    break;\n{pad}}}\n") };
            let mut here = String::new();
            for (why, t) in &sp.at_end {
                here.push_str(&refuse(why, t, "                "));
            }
            let mut passed = String::new();
            let mut after = String::new();
            for (why, t) in &sp.passed {
                passed.push_str(&refuse(why, t, "                "));
                let _ = if t == "true" { write!(after, "        // {why}\n        refused = true;\n") } else { write!(after, "        // {why}\n        if {t} {{\n            refused = true;\n        }}\n") };
            }
            let (mut first, mut rest) = (String::new(), String::new());
            for (why, f, r) in &sp.starts {
                if let Some(f) = f {
                    first.push_str(&refuse(why, f, "                "));
                }
                if let Some(r) = r {
                    rest.push_str(&refuse(why, r, "                "));
                }
            }
            let _ = (&here, &passed, &after, &first, &rest);
            let e = self.bytes_at(&sp.e.clone(), 0, &mut HashMap::new());
            for (p, arm) in row_arms.iter_mut().enumerate() {
                let opens = &row_opens[p];
                let fold = |t: &str| fold_open(t, opens);
                let refuse_at = |why: &str, t: &str, pad: &str| if t == "true" { format!("{pad}// {why}\n{pad}refused = true;\n{pad}break;\n") } else { format!("{pad}// {why}\n{pad}if {t} {{\n{pad}    refused = true;\n{pad}    break;\n{pad}}}\n") };
                let mut code = String::new();
                // Its end, at a token of a row that reads its stop's region: the checks over it,
                // from its first byte to the token's (`c`), then its end, before the starts' tests.
                if sp.ends_at.contains(&arm.row) {
                    let mut t1 = String::new();
                    for (why, t) in &sp.open {
                        t1.push_str(&refuse_at(why, t, "                "));
                    }
                    let _ = write!(code, "            if span_{i}.0 != usize::MAX && span_{i}.1 == usize::MAX && s >= span_{i}.0 {{\n                let (l, c) = (span_{i}.0 - 1, s);\n                let _ = (l, c);\n{t1}                span_{i}.1 = s;\n            }}\n");
                }
                // At the starts in it.
                let (mut f1, mut r1) = (String::new(), String::new());
                for (why, f, r) in &sp.starts {
                    if let Some(f) = f.as_deref().map(fold).filter(|t| t != "false") {
                        f1.push_str(&refuse_at(why, &f, "                "));
                    }
                    if let Some(r) = r.as_deref().map(fold).filter(|t| t != "false") {
                        r1.push_str(&refuse_at(why, &r, "                "));
                    }
                }
                if f1 == r1 && !f1.is_empty() {
                    let _ = write!(code, "            if s >= span_{i}.0 && s < span_{i}.1 {{\n{f1}            }}\n");
                } else if !f1.is_empty() || !r1.is_empty() {
                    let _ = write!(code, "            if s == span_{i}.0 && s < span_{i}.1 {{\n{f1}            }} else if s > span_{i}.0 && s < span_{i}.1 {{\n{r1}            }}\n");
                }
                // At its end, where its end byte is a first byte of the row's.
                if (0..256).any(|b| e[b] && arm.first[b]) {
                    let mut t1 = String::new();
                    for (why, t) in &sp.at_end {
                        let t = fold(t);
                        if t != "false" {
                            t1.push_str(&refuse_at(why, &t, "                "));
                        }
                    }
                    if !t1.is_empty() {
                        let c = if uses(&t1, "c") { format!("                let c = span_{i}.1;\n") } else { String::new() };
                        let _ = write!(code, "            if s == span_{i}.1 {{\n{c}{t1}            }}\n");
                    }
                }
                let at = arm.body.find("            //~spans\n").map_or(0, |k| k + "            //~spans\n".len());
                arm.body.insert_str(at, &code);
                // A token of the row's that covers its end.
                if (0..256).any(|b| e[b] && arm.tail[b]) {
                    let mut t1 = String::new();
                    for (why, t) in &sp.passed {
                        let t = fold(t);
                        if t != "false" {
                            t1.push_str(&refuse_at(why, &t, "                "));
                        }
                    }
                    if !t1.is_empty() {
                        let at = arm.body.find("            //~passed\n").map_or(0, |k| k + "            //~passed\n".len());
                        let c = if uses(&t1, "c") { format!("                let c = span_{i}.1;\n") } else { String::new() };
                        arm.body.insert_str(at, &format!("            // `{}`: this token covers its end.\n            if s < span_{i}.1 && span_{i}.1 <= l {{\n{c}{t1}            }}\n", sp.f));
                    }
                }
            }
        }
        for (p, arm) in row_arms.iter_mut().enumerate() {
            let opens = &row_opens[p];
            let mut code = String::new();
            if picks.fused {
                for (j, a) in accs.iter().enumerate() {
                    if a.here != "false" {
                        let _ = writeln!(code, "            here_{j} = {};", fold_open(&a.here, opens));
                    }
                }
            }
            for (why, t) in &arm.starts {
                let t = if picks.fused || opens.len() == 1 { fold_open(t, opens) } else { t.clone() };
                if t != "false" {
                    let _ = write!(code, "            // {why}\n            if {t} {{\n                refused = true;\n                break;\n            }}\n");
                }
            }
            let at = arm.body.find("            //~spans\n").map_or(0, |k| k + "            //~spans\n".len());
            arm.body.insert_str(at, &code);
            // Fused, the row knows its region: it clears its own bytes of `code_l`.
            let clear = if picks.fused && !self.code_after.is_empty() { "            clear(&mut code_l, s, l);\n" } else { "" };
            arm.body = arm.body.replace("            //~spans\n", "").replace("            //~passed\n", "").replace("            //~code_l\n", clear);
            if picks.fused && uses(&arm.body, "open") {
                return Err(format!("tier 1 as Rust: row {} reads the region opened, which the rows' pick does not make", arm.row));
            }
        }
        let nest_step = match &self.nest {
            Some((o, c, d, _)) => format!("            // The nest: a bracket opens at a `{{` where a level is open; the innermost closes at a `}}` (a tail's row reads on past it).\n            if depth > 0 {{\n                if bit(&{o}, s) {{\n                    if depth == {d} {{\n                        refused = true;\n                        break;\n                    }}\n                    nest[depth] = usize::MAX;\n                    depth += 1;\n                    template = usize::MAX;\n                }} else if bit(&{c}, s) {{\n                    depth -= 1;\n                    template = if depth > 0 {{ nest[depth - 1] }} else {{ usize::MAX }};\n                }}\n            }}\n"),
            None => String::new(),
        };
        let branches = format!("{{\n            // The row: the first whose start holds, of those whose start may be the byte's.\n            let row = match group {{\n{}                _ => usize::MAX,\n            }};\n{nest_step}            match row {{\n{}                _ => {{\n                    // A byte no token begins with.\n                    refused = true;\n                    break;\n                }}\n            }}\n        }}", picks.rows, row_arms.iter().enumerate().map(|(i, a)| format!("                {i} => {},\n", indent(&match &a.resume {
            // The body up to its record's tuple, then the resume.
            Some(r) => format!("{}{r}", a.body.rsplit_once("            (l, ").map_or(a.body.as_str(), |x| x.0)),
            None => format!("{}{:#x})\n        }}", a.body, a.bits),
        }, "        ").trim_start())).collect::<String>());
        let mut fill_top = String::new();
        let mut fill_end = String::new();
        let nf = self.fills.len();
        let mut decl_of = vec![String::new(); nf];
        let mut formula_of = vec![String::new(); nf];
        for (i, f) in self.fills.iter().enumerate() {
            decl_of[i] = format!("    // `{}` at the last byte of the token before{}.\n    let mut fill_{i} = false;\n{}", f.shown, if f.prev { ", and its `X`" } else { "" }, if f.prev { format!("    let mut xl_{i} = false;\n") } else { String::new() });
            let (fs, xs, fgap) = match (f.prev, &f.xg) {
                _ if f.within.is_some() => (format!("fill_{i} && fs_{}", f.within.unwrap_or_default()), None, None),
                (true, None) if f.eg.is_some() => {
                    let g = f.eg.clone().unwrap_or_default();
                    (format!("if s > at {{ (xl_{i} || (fill_{i} && !bit(&{g}, at))) && !any_in(&{g}, at + 1, s - 1) }} else {{ fill_{i} }}"), Some(format!("s == at && xl_{i}")), None)
                }
                (false, None) if f.eg.is_some() => (format!("fill_{i} && (s == at || !any_in(&{}, at, s - 1))", f.eg.clone().unwrap_or_default()), None, None),
                (true, None) => (format!("fill_{i} || (s > at && xl_{i})"), Some(format!("s == at && xl_{i}")), Some(format!("fill_{i} || xl_{i}"))),
                (true, Some(g)) => (format!("fill_{i} || (s > at && (xl_{i} || (s > at + 1 && any_in(&{g}, at, s - 2))))"), Some(format!("if s > at {{ bit(&{g}, s - 1) }} else {{ xl_{i} }}")), None),
                (false, None) => (format!("fill_{i}"), None, Some(format!("fill_{i}"))),
                (false, Some(g)) => (format!("fill_{i} || (s > at && any_in(&{g}, at, s - 1))"), None, None),
            };
            formula_of[i] = format!("{fs} {} {}", xs.clone().unwrap_or_default(), fgap.clone().unwrap_or_default());
            let _ = writeln!(fill_top, "        // `{}`: at the byte before the token.\n        let fs_{i} = {fs};", f.shown);
            if let Some(xs) = xs {
                let _ = writeln!(fill_top, "        let xs_{i} = {xs};");
            }
            if fill_used.contains(&i) {
                let fgap = fgap.ok_or_else(|| format!("tier 1 as Rust: `{}` read over a gap it is not the same over", f.shown))?;
                let _ = writeln!(fill_top, "        // Over the gap.\n        let fgap_{i} = {fgap};");
            }
            for (why, j) in &self.end_checks {
                if *j == i {
                    if uses(&fs, "s") {
                        let _ = write!(fill_end, "    // {why}\n    {{\n        let s = n;\n        if {fs} {{\n            refused = true;\n        }}\n    }}\n");
                    } else {
                        let _ = write!(fill_end, "    // {why}\n    if {fs} {{\n        refused = true;\n    }}\n");
                    }
                }
            }
        }
        // The bytes outside the region tokens, for the escapes made of `code`.
        if !self.code_after.is_empty() {
            fw_decl.push_str("    // The bytes outside the region tokens (`code`), as the loop reads them.\n    let mut code_l = vec![!0u64; nw];\n");
            if !picks.fused {
                let _ = write!(acc_after, "        if open != usize::MAX {{\n            clear(&mut code_l, s, l);\n        }}\n");
            }
            let code = self.names.get("code").cloned().unwrap_or_default();
            let _ = write!(span_after, "    for (c, v) in code_l.iter_mut().zip(&lv) {{\n        *c &= v;\n    }}\n    #[cfg(feature = \"test-hooks\")]\n    if !refused {{\n        assert!(code_l == {code}, \"code_l is the regions' code\");\n    }}\n{}", self.code_after);
        }
        // The escapes over each stretch of code: at a region token, the stretch before it; after
        // the loop, the last.
        if !self.stretch_checks.is_empty() {
            let mut at_region = Vec::new();
            for (why, terms) in &self.stretch_checks {
                let mid: Vec<String> = terms.iter().map(|(j, m)| format!("any_in(&{m}, code_from + {j}, (s + {}).min(n - 1))", j - 1)).collect();
                let end: Vec<String> = terms.iter().map(|(j, m)| format!("(code_from + {j} < n && any_in(&{m}, code_from + {j}, n - 1))")).collect();
                at_region.push(format!("            // {why}\n            if {} {{\n                refused = true;\n                break;\n            }}\n", mid.join(" || ")));
                let _ = write!(span_after, "    // {why}\n    if {} {{\n        refused = true;\n    }}\n", end.join(" || "));
            }
            let _ = write!(open_code, "        // The stretch of code since the region token before ends here.\n        if open != usize::MAX && s > code_from {{\n{}        }}\n", at_region.concat());
            let _ = write!(acc_after, "        if open != usize::MAX {{\n            code_from = l + 1;\n        }}\n");
            fw_decl.push_str("    // The first byte after the last region token.\n    let mut code_from = 0usize;\n");
        }
        // The fills something reads: a test, an escape, the end's check, or a fill so read; the
        // others' state and updates are left out.
        let owner = |line: &str| -> Option<usize> {
            let t = line.trim_start();
            let t = t.strip_prefix("fill_").or_else(|| t.strip_prefix("xl_"))?;
            let d: String = t.chars().take_while(char::is_ascii_digit).collect();
            if d.is_empty() || !t[d.len()..].starts_with(" = ") {
                return None;
            }
            d.parse().ok().filter(|&i| i < nf)
        };
        let mut other = String::new();
        let mut own_of = vec![String::new(); nf];
        for piece in [&branches, &open_code, &gap_code, &own_code, &token_code] {
            for line in piece.lines() {
                match owner(line) {
                    Some(i) => own_of[i].push_str(line),
                    None => other.push_str(line),
                }
                other.push('\n');
            }
        }
        let reads = |text: &str, i: usize| ["fs_", "xs_", "fgap_", "fill_", "xl_"].iter().any(|p| uses(text, &format!("{p}{i}")));
        let mut live: Vec<bool> = (0..nf).map(|i| reads(&other, i) || self.end_checks.iter().any(|(_, j)| *j == i)).collect();
        loop {
            let mut grew = false;
            for i in 0..nf {
                if live[i] {
                    let text = format!("{} {}", own_of[i], formula_of[i]);
                    for j in 0..nf {
                        if !live[j] && reads(&text, j) {
                            live[j] = true;
                            grew = true;
                        }
                    }
                }
            }
            if !grew {
                break;
            }
        }
        let keep = |piece: &str| -> String { piece.lines().filter(|l| owner(l).is_none_or(|i| live[i])).map(|l| format!("{l}\n")).collect() };
        let (branches, gap_code, own_code, token_code) = (keep(&branches), keep(&gap_code), keep(&own_code), keep(&token_code));
        open_code = keep(&open_code);
        for i in 0..nf {
            if live[i] {
                span_decl.push_str(&decl_of[i]);
            }
        }
        // Only the reads of them the code makes: at the loop's top where code outside the rows'
        // arms reads one, else first in each arm that reads it (as the lean lexer reads `nl` where
        // it reads a `;`).
        let mut branches = branches;
        let mut defs: Vec<(String, String)> = Vec::new();
        let mut pending_comment = String::new();
        for line in fill_top.lines() {
            if line.trim_start().starts_with("//") {
                pending_comment = format!("{}\n", line.trim_start());
                continue;
            }
            let var = line.trim_start().trim_start_matches("let ").split(' ').next().unwrap_or("").to_owned();
            defs.push((var, format!("{pending_comment}{}\n", line.trim_start())));
            pending_comment.clear();
        }
        let mut segs: Vec<String> = branches.split("//~defs\n").map(str::to_owned).collect();
        for i in 0..segs.len() - 1 {
            let t = segs[i].trim_end_matches(' ').len();
            segs[i].truncate(t);
        }
        let mut outside = format!("{}{open_code}{gap_code}{own_code}{token_code}{acc_after}", segs[0]);
        let mut top_defs: Vec<String> = Vec::new();
        for (var, def) in defs.iter().rev() {
            if uses(&outside, var) {
                outside.push_str(def);
                top_defs.push(def.clone());
                continue;
            }
            if segs.iter().skip(1).all(|seg| uses(seg, var)) {
                outside.push_str(def);
                top_defs.push(def.clone());
                continue;
            }
            for seg in segs.iter_mut().skip(1) {
                if uses(seg, var) {
                    seg.insert_str(0, def);
                }
            }
        }
        let mut top = String::new();
        for d in top_defs.iter().rev() {
            top.push_str(&indent(d, "        "));
        }
        // Each arm's defs at its indent.
        for seg in segs.iter_mut().skip(1) {
            let body_at = seg.lines().find(|l| l.starts_with(' ') && l.trim_start().starts_with("// `")).map(|l| l.len() - l.trim_start().len()).unwrap_or(20);
            let n = seg.lines().take_while(|l| !l.starts_with(' ')).count();
            let lines: Vec<&str> = seg.lines().collect();
            let head: String = lines[..n].iter().map(|l| format!("{}{l}\n", " ".repeat(body_at))).collect();
            let tail: String = lines[n..].iter().map(|l| format!("{l}\n")).collect();
            *seg = format!("{head}{tail}");
        }
        branches = segs.concat();
        open_code.insert_str(0, &top);
        // Each mask the loop scans the gap before a token of, scanned once (`gap_NAME`), as the
        // lean lexer's `nl`.
        let mut scanned: Vec<String> = Vec::new();
        for piece in [&open_code, &branches, &gap_code, &own_code, &token_code, &acc_after] {
            let mut rest = piece.as_str();
            while let Some(i) = rest.find("any_in(&") {
                let after = &rest[i + 8..];
                let name: String = after.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if after[name.len()..].starts_with(", at, s - 1)") && !scanned.contains(&name) {
                    scanned.push(name.clone());
                }
                rest = &after[name.len()..];
            }
        }
        let shared = |t: &str| -> String {
            let mut t = t.to_owned();
            for m in &scanned {
                t = t.replace(&format!("s > at && any_in(&{m}, at, s - 1)"), &format!("gap_{m}")).replace(&format!("any_in(&{m}, at, s - 1)"), &format!("gap_{m}"));
            }
            t
        };
        let mut defs = String::new();
        for m in &scanned {
            let _ = writeln!(defs, "        // Whether `{m}` holds in the gap before the token.\n        let gap_{m} = s > at && any_in(&{m}, at, s - 1);");
        }
        open_code = format!("{defs}{}", shared(&open_code));
        let (branches, gap_code, own_code, token_code) = (shared(&branches), shared(&gap_code), shared(&own_code), shared(&token_code));
        acc_after = shared(&acc_after);
        span_after.push_str(&fill_end);
        let mut out = String::new();
        out.push_str("// Generated by the Bitlex compiler (`rerail_compiler::bitlex_lex_rust`) from the grammar's lexer statements. Do not edit.\n\n");
        out.push_str("/// The masks tier 2 reads, a bit a byte: those it splits a region by, where it reads one in parts.\npub struct Masks {\n");
        for f in &fields {
            let _ = writeln!(out, "    pub {f}: Vec<u64>,");
        }
        out.push_str("}\n\n/// Tier 1's records: each token's first and last byte and its code (its kind, its word and its marks), the masks tier 2 reads, and whether the file goes to tree-sitter.\npub struct Lexed {\n    pub starts: Vec<u32>,\n    pub lasts: Vec<u32>,\n    pub codes: Vec<u32>,\n    pub masks: Masks,\n    pub refused: bool,\n}\n\n");
        let kinds: Vec<String> = x.kinds.iter().map(|k| k.to_string()).collect();
        let _ = write!(out, "/// Each byte's record kind, as a token's first byte.\nconst KINDS: [u8; 256] = [{}];\n\n", kinds.join(", "));
        let mut words = String::new();
        for (i, w) in x.words.iter().enumerate() {
            let _ = writeln!(words, "        b\"{}\" => {},", lit(w), i + 1);
        }
        let _ = write!(out, "/// A token's word: the code of the word its bytes are, 0 for none.\nfn word(b: &[u8]) -> u32 {{\n    match b {{\n{words}        _ => 0,\n    }}\n}}\n\n");
        out.push_str(&pair_fns);
        out.push_str(&self.stage1());
        out.push_str(&tables);
        out.push_str("/// `src`'s records.\npub fn lex(src: &[u8]) -> Lexed {\n    let n = src.len();\n    let nw = n.div_ceil(64);\n    // Each word's bits inside the file.\n    let lv: Vec<u64> = (0..nw).map(|q| if 64 * q + 64 <= n { !0 } else { (1u64 << (n % 64)) - 1 }).collect();\n");
        let open_decl = if self.kinds.iter().any(|q| q.resumes) {
            let d = self.nest.as_ref().map_or(1, |n| n.2);
            format!("    // The strings' records templates left open (decisions 0600 and 0621), over their templates' code: the first `opened`, innermost last.\n    let mut held = [0usize; {d}];\n    let mut opened = 0usize;\n")
        } else {
            String::new()
        };
        let nest_decl = match &self.nest {
            Some((_, _, d, _)) => format!("    // The nest (decision 0621): its levels, each a template's kind or a bracket's `usize::MAX`, innermost last, as the lean lexer's `interpolation_end` counts a template's braces.\n    let mut nest = [usize::MAX; {d}];\n    let mut depth = 0usize;\n"),
            None => String::new(),
        };
        let span_decl = format!("{open_decl}{nest_decl}{span_decl}");
        let plain_decl_text = format!("{}{span_decl}", if self.opens.is_some() { "    // The kind of string the template is in, as the lean lexer's string reader keeps it over the template's code: the region read last that closed at its second closer (a template's `$`), of the kinds with no `after`.\n    let mut template = usize::MAX;\n" } else { "" });
        let kept: Vec<String> = self.nodes.iter().filter(|n| n.kept).map(|n| n.name.clone()).chain(self.nodes.iter().flat_map(|n| &n.also).map(|a| format!("mut {a}"))).collect();
        let _ = writeln!(out, "    // Stage 1: the masks the regions and the tokens read, and whether an escape holds.\n    let Stage1 {{ {}, refused: escapes }} = stage1_best(src);", kept.join(", "));
        let rest = format!("{plain_decl_text}{fw_decl}{open_code}{branches}{token_code}{own_code}{gap_code}{acc_after}{span_after}{fields_init}");
        // A refusal in the token loop leaves both its loops.
        let tokens = |t: &str| t.replace("break;\n", "break 'tokens;\n");
        let (open_code, branches, token_code, own_code, gap_code, acc_after) = (tokens(&open_code), tokens(&branches), tokens(&token_code), tokens(&own_code), tokens(&gap_code), tokens(&acc_after));
        out.push_str(&gate_walk(&self.body, &rest));
        let _ = write!(
            out,
            "    // The tokens: from the first byte `skip` does not hold, the token whose start holds there, to its last byte by its rule.\n    let mut starts: Vec<u32> = Vec::with_capacity(n / 4 + 1);\n    let mut lasts: Vec<u32> = Vec::with_capacity(n / 4 + 1);\n    let mut codes: Vec<u32> = Vec::with_capacity(n / 4 + 1);\n    let mut at = 0usize;\n    // The gap's start, since the grammar token before and since the record before; the grammar token before's word; the marks its tests carry to the next grammar token.\n    let (mut from, mut from_record, mut before, mut pending) = (0usize, 0usize, 0u32, 0u32);\n{plain_decl}{fw_decl}    // The blocks of 64 bytes, and in each the bytes `skip` does not hold from the last token's end on: each a token's first byte (the lean lexer's `lex_blocks`).\n    let mut b = at / 64;\n    'tokens: while b < nw {{\n        let base = b * 64;\n        let mut firsts = !{skip}[b] & (!0u64 << at.saturating_sub(base));\n        while firsts != 0 {{\n        let s = base + firsts.trailing_zeros() as usize;\n        if s >= n {{\n            break 'tokens;\n        }}\n{open_code}        let mut hits = 0u32;\n        let (l, kind, extra, mut marks): (usize, u32, bool, u32) = {branches};\n        let word = if extra {{ 0 }} else {{ word(&src[s..=l]) }};\n        // The token marks: tier 2 tests the rest of each.\n{token_code}        if extra {{\n            // An extra keeps the marks of its own gap; its tests' hits go to the next grammar token.\n{own}            pending |= hits & 0xffff | hits >> 16;\n        }} else {{\n            let tested = pending | hits;\n{gap_code}            from = s + 1;\n            before = word;\n            pending = hits >> 16;\n        }}\n        from_record = s + 1;\n{acc_after}        starts.push(s as u32);\n        lasts.push(l as u32);\n        codes.push(kind | word << 8 | marks << 16);\n        at = l + 1;\n        if at >= base + 64 {{\n            b = at / 64;\n            continue 'tokens;\n        }}\n        firsts &= !0u64 << (at - base);\n        }}\n        b += 1;\n    }}\n    let _ = (from, from_record, before, pending);\n{span_after}    Lexed {{\n        starts,\n        lasts,\n        codes,\n        masks: Masks {{\n{fields_init}        }},\n        refused,\n    }}\n}}\n",
            own = own_code,
            plain_decl = plain_decl_text,
        );
        out.push_str(HELPERS);
        Ok(out)
    }
}

/// A read's arm at its closer `c` (`Some(c) => …`), keeping `c` as `close` first.
fn with_close(arm: &str) -> String {
    match arm.strip_prefix("{\n") {
        Some(rest) => format!("{{\n                    close = c;\n{rest}"),
        None => format!("{{\n                    close = c;\n                    {arm}\n                }}"),
    }
}

/// Whether `e` reads mask `name`, itself or through the masks it reads (`defs`).
pub fn reads_mask(e: &Expr, name: &str, defs: &HashMap<String, Expr>) -> bool {
    let e = substitute(e, &[(name, &Expr::Name(format!("{name}'")))], defs);
    fn has(e: &Expr, n: &str) -> bool {
        match e {
            Expr::Name(m) => m == n,
            Expr::Empty | Expr::Test { .. } => false,
            Expr::Not(a) => has(a, n),
            Expr::And(a, b) | Expr::Or(a, b) | Expr::Xor(a, b) => has(a, n) || has(b, n),
            Expr::Call(_, args) => args.iter().any(|a| matches!(a, Arg::E(e) if has(e, n))),
        }
    }
    has(&e, &format!("{name}'"))
}

/// `e` with `assumed`'s masks read as their expressions (decision 0552's `in KIND`): each mask whose
/// expression reads one of them, by `defs`, written out to where it does; the others kept by name.
pub fn substitute(e: &Expr, assumed: &[(&str, &Expr)], defs: &HashMap<String, Expr>) -> Expr {
    fn reads(n: &str, assumed: &[(&str, &Expr)], defs: &HashMap<String, Expr>, memo: &mut HashMap<String, bool>) -> bool {
        if assumed.iter().any(|(a, _)| *a == n) {
            return true;
        }
        if let Some(&r) = memo.get(n) {
            return r;
        }
        memo.insert(n.to_owned(), false);
        let r = defs.get(n).is_some_and(|d| names_in(d).iter().any(|m| reads(m, assumed, defs, memo)));
        memo.insert(n.to_owned(), r);
        r
    }
    fn names_in(e: &Expr) -> Vec<String> {
        match e {
            Expr::Name(n) => vec![n.clone()],
            Expr::Empty | Expr::Test { .. } => Vec::new(),
            Expr::Not(a) => names_in(a),
            Expr::And(a, b) | Expr::Or(a, b) | Expr::Xor(a, b) => [names_in(a), names_in(b)].concat(),
            Expr::Call(_, args) => args.iter().flat_map(|a| if let Arg::E(e) = a { names_in(e) } else { Vec::new() }).collect(),
        }
    }
    fn go(e: &Expr, assumed: &[(&str, &Expr)], defs: &HashMap<String, Expr>, memo: &mut HashMap<String, bool>) -> Expr {
        match e {
            Expr::Name(n) => {
                if let Some((_, a)) = assumed.iter().find(|(a, _)| a == n) {
                    (*a).clone()
                } else if reads(n, assumed, defs, memo) {
                    go(&defs[n], assumed, defs, memo)
                } else {
                    e.clone()
                }
            }
            Expr::Empty | Expr::Test { .. } => e.clone(),
            Expr::Not(a) => simplify(Expr::Not(Box::new(go(a, assumed, defs, memo)))),
            Expr::And(a, b) => simplify(Expr::And(Box::new(go(a, assumed, defs, memo)), Box::new(go(b, assumed, defs, memo)))),
            Expr::Or(a, b) => simplify(Expr::Or(Box::new(go(a, assumed, defs, memo)), Box::new(go(b, assumed, defs, memo)))),
            Expr::Xor(a, b) => simplify(Expr::Xor(Box::new(go(a, assumed, defs, memo)), Box::new(go(b, assumed, defs, memo)))),
            Expr::Call(f, args) => Expr::Call(f.clone(), args.iter().map(|a| if let Arg::E(x) = a { Arg::E(go(x, assumed, defs, memo)) } else { a.clone() }).collect()),
        }
    }
    go(e, assumed, defs, &mut HashMap::new())
}

/// `e` with its constants folded: `empty` (none) and `~empty` (every byte).
/// `simplify` at each level of `e`, from its leaves.
fn simplify_all(e: Expr) -> Expr {
    let s = |a: Box<Expr>| Box::new(simplify_all(*a));
    simplify(match e {
        Expr::Not(a) => Expr::Not(s(a)),
        Expr::And(a, b) => Expr::And(s(a), s(b)),
        Expr::Or(a, b) => Expr::Or(s(a), s(b)),
        Expr::Xor(a, b) => Expr::Xor(s(a), s(b)),
        e => e,
    })
}

fn simplify(e: Expr) -> Expr {
    let all = |x: &Expr| matches!(x, Expr::Not(a) if matches!(**a, Expr::Empty));
    match e {
        Expr::Not(a) => match *a {
            Expr::Not(b) => *b,
            a => Expr::Not(Box::new(a)),
        },
        Expr::Call(f, args) if (f == "prev" || f == "next") && matches!(args.first(), Some(Arg::E(Expr::Empty))) => Expr::Empty,
        Expr::And(a, b) if format!("{a:?}") == format!("{b:?}") => *a,
        Expr::Or(a, b) if format!("{a:?}") == format!("{b:?}") => *a,
        Expr::And(a, b) => {
            if matches!(*a, Expr::Empty) || matches!(*b, Expr::Empty) {
                Expr::Empty
            } else if all(&a) {
                *b
            } else if all(&b) {
                *a
            } else {
                Expr::And(a, b)
            }
        }
        Expr::Or(a, b) => {
            if all(&a) || all(&b) {
                Expr::Not(Box::new(Expr::Empty))
            } else if matches!(*a, Expr::Empty) {
                *b
            } else if matches!(*b, Expr::Empty) {
                *a
            } else {
                Expr::Or(a, b)
            }
        }
        Expr::Xor(a, b) => {
            if matches!(*a, Expr::Empty) {
                *b
            } else if matches!(*b, Expr::Empty) {
                *a
            } else {
                Expr::Xor(a, b)
            }
        }
        e => e,
    }
}

/// The regions a start's test is one of (`open == K`, or several of those or'd), as `at_start` writes it.
fn regions_of(t: &str) -> Option<Vec<usize>> {
    let t = t.replace(['(', ')'], "");
    t.split(" || ").map(|x| x.strip_prefix("open == ").and_then(|k| k.parse().ok())).collect()
}

/// `s`'s lines at `p`.
/// The parts of `t` split at `op` (` && ` or ` || `) outside parentheses, its own outer ones
/// taken off.
fn parts_of<'a>(t: &'a str, op: &str) -> Vec<&'a str> {
    let inner = |t: &'a str| -> &'a str {
        if !(t.starts_with('(') && t.ends_with(')')) {
            return t;
        }
        // The outer parentheses are one pair where the depth is above 0 to the last byte.
        let mut depth = 0i32;
        for (i, c) in t.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            if depth == 0 && i + 1 < t.len() {
                return t;
            }
        }
        &t[1..t.len() - 1]
    };
    let mut out = Vec::new();
    let mut todo = vec![inner(t)];
    while let Some(t) = todo.pop() {
        let (mut depth, mut from, mut split) = (0i32, 0usize, false);
        let b = t.as_bytes();
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'(' | b'{' | b'[' => depth += 1,
                b')' | b'}' | b']' => depth -= 1,
                _ if depth == 0 && t[i..].starts_with(op) => {
                    todo.push(inner(&t[from..i]));
                    from = i + op.len();
                    i = from;
                    split = true;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        if split {
            todo.push(inner(&t[from..]));
        } else {
            out.push(t);
        }
    }
    out.reverse();
    out
}

/// `t` with each `open == K` known where the regions `opens` may be open (`usize::MAX`: none), and
/// what is then known of it.
fn fold_open(t: &str, opens: &[String]) -> String {
    let mut out = t.to_owned();
    let mut i = 0;
    while let Some(k) = out[i..].find("open == ") {
        let at = i + k;
        let v: String = out[at + 8..].chars().take_while(|c| c.is_alphanumeric() || *c == ':' || *c == '_').collect();
        let known = if !opens.contains(&v) {
            Some("false")
        } else if opens.len() == 1 {
            Some("true")
        } else {
            None
        };
        match known {
            Some(b) => {
                out.replace_range(at..at + 8 + v.len(), b);
                i = at + b.len();
            }
            None => i = at + 8 + v.len(),
        }
    }
    fold_bool(&out)
}

/// `t`, with its `true`s and `false`s taken in.
fn fold_bool(t: &str) -> String {
    let t = t.trim();
    for (op, unit, zero) in [(" || ", "false", "true"), (" && ", "true", "false")] {
        let ps = parts_of(t, op);
        if ps.len() > 1 {
            let mut kept: Vec<String> = Vec::new();
            for p in ps {
                let f = fold_bool(p);
                if f == zero {
                    return zero.to_owned();
                }
                if f != unit {
                    kept.push(f);
                }
            }
            return match kept.len() {
                0 => unit.to_owned(),
                1 => kept.pop().unwrap_or_default(),
                _ => format!("({})", kept.join(op)),
            };
        }
    }
    if let Some(inner) = t.strip_prefix('!') {
        return match fold_bool(inner).as_str() {
            "true" => "false".to_owned(),
            "false" => "true".to_owned(),
            f if f.starts_with('(') || !f.contains(' ') => format!("!{f}"),
            f => format!("!({f})"),
        };
    }
    let ps = parts_of(t, " && ");
    if ps.len() == 1 && ps[0] != t {
        return fold_bool(ps[0]);
    }
    t.to_owned()
}

/// `a && b`, each conjunct once, less a disjunction one of the others is a part of (`x && (x ||
/// y)` is `x`).
fn absorb(a: &str, b: &str) -> String {
    let mut cs: Vec<&str> = Vec::new();
    for c in parts_of(a, " && ").into_iter().chain(parts_of(b, " && ")) {
        if !cs.contains(&c) {
            cs.push(c);
        }
    }
    let keep: Vec<&str> = cs.iter().copied().filter(|c| {
        let ds = parts_of(c, " || ");
        !(ds.len() > 1 && cs.iter().any(|o| o != c && ds.contains(o)))
    }).collect();
    // A disjunction `parts_of` took out of its parentheses goes back in them: `&&` binds closer.
    let conj = |c: &str| if parts_of(c, " || ").len() > 1 { format!("({c})") } else { c.to_owned() };
    let mut out = String::new();
    for (i, c) in keep.iter().enumerate() {
        out = if i == 0 { conj(c) } else { format!("({out} && {})", conj(c)) };
    }
    out
}

/// `if a { x } else if b { y } else { none }` of `chain`, to its first test that is `true`.
fn if_chain(chain: &[(String, String)], none: &str, at: &str) -> String {
    let mut out = String::new();
    for (i, (c, v)) in chain.iter().enumerate() {
        if c == "true" {
            if i == 0 {
                return v.clone();
            }
            let _ = write!(out, "else {{\n{at}    {v}\n{at}}}");
            return out;
        }
        let _ = write!(out, "{}if {c} {{\n{at}    {v}\n{at}}} ", if i == 0 { "" } else { "else " });
    }
    let _ = write!(out, "else {{\n{at}    {none}\n{at}}}");
    out
}

fn indent(s: &str, p: &str) -> String {
    s.lines().map(|l| if l.is_empty() { "\n".to_owned() } else { format!("{p}{l}\n") }).collect()
}

/// The word operations the masks are made of, and the token loop's searches.
const HELPERS: &str = r#"
/// Block `q`'s 64 bytes; the last block's past the file are 0x00.
#[inline(always)]
fn block(src: &[u8], q: usize) -> [u8; 64] {
    let at = 64 * q;
    match src.get(at..at + 64) {
        Some(b) => b.try_into().expect("64 bytes"),
        None => {
            let mut b = [0u8; 64];
            b[..src.len() - at].copy_from_slice(&src[at..]);
            b
        }
    }
}

/// `prev(x, k)`'s word `i`: each byte's bit from `k` bytes before.
#[inline(always)]
fn prev(x: &[u64], i: usize, k: u32) -> u64 {
    x[i] << k | if i > 0 { x[i - 1] >> (64 - k) } else { 0 }
}

/// `next(x, k)`'s word `i`: each byte's bit from `k` bytes after.
#[inline(always)]
fn next(x: &[u64], i: usize, k: u32) -> u64 {
    x[i] >> k | x.get(i + 1).map_or(0, |w| w << (64 - k))
}

/// `fill(s, e)`: from each byte of `s` up to, not including, the next of `e`; one add a word,
/// its carry the next word's.
fn fill(s: &[u64], e: &[u64]) -> Vec<u64> {
    let mut cin = 0u64;
    s.iter()
        .zip(e)
        .map(|(&s, &e)| {
            let b = !e | s;
            let into = s.wrapping_add(b).wrapping_add(cin) ^ s ^ b;
            let f = s | (!e & into);
            cin = f >> 63;
            f
        })
        .collect()
}

/// `escaped(x)`: simdjson's escaped bytes, each byte after an odd run of `x`'s, a word at a time
/// with one carry bit.
fn escaped(x: &[u64]) -> Vec<u64> {
    const ODD: u64 = 0xaaaa_aaaa_aaaa_aaaa;
    let mut carry = 0u64;
    x.iter()
        .map(|&bs| {
            if bs == 0 {
                return std::mem::take(&mut carry);
            }
            let potential = bs & !carry;
            let series = ((potential << 1) | ODD).wrapping_sub(potential);
            let escape_and_terminal = series ^ ODD;
            let escaped = escape_and_terminal ^ (bs | carry);
            carry = (escape_and_terminal & bs) >> 63;
            escaped
        })
        .collect()
}

/// Whether `x` holds anywhere.
fn any(x: &[u64]) -> bool {
    x.iter().any(|&w| w != 0)
}

/// Whether `x` holds at byte `i`.
#[inline(always)]
fn bit(x: &[u64], i: usize) -> bool {
    x.get(i / 64).is_some_and(|w| w >> (i % 64) & 1 == 1)
}

/// `x`'s first bit at or after byte `from`, a word at a time.
#[inline(always)]
fn first_bit(x: &[u64], from: usize) -> Option<usize> {
    let mut q = from / 64;
    let mut w = x.get(q)? & (!0u64 << (from % 64));
    while w == 0 {
        q += 1;
        w = *x.get(q)?;
    }
    Some(64 * q + w.trailing_zeros() as usize)
}

/// The first byte at or after `from` that `x` does not hold, before `n`.
#[inline(always)]
fn first_clear(x: &[u64], from: usize, n: usize) -> Option<usize> {
    let mut q = from / 64;
    if from >= n {
        return None;
    }
    let mut w = !x.get(q).copied().unwrap_or(0) & (!0u64 << (from % 64));
    while w == 0 {
        q += 1;
        if 64 * q >= n {
            return None;
        }
        w = !x.get(q).copied().unwrap_or(0);
    }
    let i = 64 * q + w.trailing_zeros() as usize;
    (i < n).then_some(i)
}

/// The last byte of the run of `r`'s bits after byte `s` with one bit of `d` in it at most: the second ends the
/// run before it, as a lexer reads a number's one `.` and stops at the next.
#[inline(always)]
fn run_once_last(r: &[u64], d: &[u64], s: usize) -> usize {
    let l = run_last(r, s);
    match first_bit(d, s + 1).filter(|&p| p <= l).and_then(|p| first_bit(d, p + 1)).filter(|&q| q <= l) {
        Some(q) => q - 1,
        None => l,
    }
}

/// The last byte of the run of `r`'s bits after byte `s`: `s` with none, a word at a time.
#[inline(always)]
fn run_last(r: &[u64], s: usize) -> usize {
    let mut i = s + 1;
    loop {
        let Some(&w) = r.get(i / 64) else { return i - 1 };
        let ones = (!(w >> (i % 64))).trailing_zeros() as usize;
        let left = 64 - i % 64;
        if ones < left {
            return i + ones - 1;
        }
        i += left;
    }
}

/// Whether `x` holds anywhere in bytes `a..=b` (none where `a > b`).
#[inline(always)]
fn any_in(x: &[u64], a: usize, b: usize) -> bool {
    a <= b && first_bit(x, a).is_some_and(|i| i <= b)
}

/// The first byte in `a..=b` that `x` holds, a word at a time.
fn first_in(x: &[u64], a: usize, b: usize) -> Option<usize> {
    if a > b {
        return None;
    }
    let mut q = a / 64;
    let mut w = x.get(q).copied().unwrap_or(0) & (!0u64 << (a % 64));
    loop {
        if q == b / 64 && b % 64 != 63 {
            w &= (1u64 << (b % 64 + 1)) - 1;
        }
        if w != 0 {
            return Some(64 * q + w.trailing_zeros() as usize);
        }
        if q == b / 64 {
            return None;
        }
        q += 1;
        w = x.get(q).copied().unwrap_or(0);
    }
}

/// The last byte in `a..=b` that `x` holds, a word at a time.
fn last_in(x: &[u64], a: usize, b: usize) -> Option<usize> {
    if a > b {
        return None;
    }
    let mut q = b / 64;
    let mut w = x.get(q).copied().unwrap_or(0);
    if b % 64 != 63 {
        w &= (1u64 << (b % 64 + 1)) - 1;
    }
    loop {
        if q == a / 64 {
            w &= !0u64 << (a % 64);
        }
        if w != 0 {
            return Some(64 * q + 63 - w.leading_zeros() as usize);
        }
        if q == a / 64 {
            return None;
        }
        q -= 1;
        w = x.get(q).copied().unwrap_or(0);
    }
}

/// `x`'s bits in bytes `a..=b`, a word at a time.
fn count_in(x: &[u64], a: usize, b: usize) -> u32 {
    let mut n = 0;
    for q in a / 64..=b / 64 {
        let mut w = x.get(q).copied().unwrap_or(0);
        if q == a / 64 {
            w &= !0u64 << (a % 64);
        }
        if q == b / 64 && b % 64 != 63 {
            w &= (1u64 << (b % 64 + 1)) - 1;
        }
        n += w.count_ones();
    }
    n
}

/// `v`'s bits at bytes `a..=b` made `m`'s.
#[inline(always)]
fn put(v: &mut [u64], m: &[u64], a: usize, b: usize) {
    let mut i = a;
    while i <= b {
        let (q, r) = (i / 64, i % 64);
        let k = (b + 1 - i).min(64 - r);
        let w = (if k == 64 { !0 } else { (1u64 << k) - 1 }) << r;
        v[q] = v[q] & !w | m[q] & w;
        i += k;
    }
}

/// `v`'s bits at bytes `a..=b` cleared.
#[inline(always)]
fn clear(v: &mut [u64], a: usize, b: usize) {
    let mut i = a;
    while i <= b {
        let (q, r) = (i / 64, i % 64);
        let k = (b + 1 - i).min(64 - r);
        v[q] &= !((if k == 64 { !0 } else { (1u64 << k) - 1 }) << r);
        i += k;
    }
}

/// Bytes `a..b` set in `v` (cut to its words).
#[inline(always)]
fn set(v: &mut [u64], a: usize, b: usize) {
    let b = b.min(64 * v.len());
    let mut i = a;
    while i < b {
        let (q, r) = (i / 64, i % 64);
        let k = (b - i).min(64 - r);
        v[q] |= (if k == 64 { !0 } else { (1u64 << k) - 1 }) << r;
        i += k;
    }
}
"#;

/// Tier 1's Rust, written from the lexer's statements.
pub fn tier1(x: &Input<'_>) -> Result<String, String> {
    Gen { x, body: String::new(), names: HashMap::new(), temps: HashMap::new(), next: 0, nodes: Vec::new(), node_of: HashMap::new(), regional: Default::default(), pairs: Vec::new(), sinks: Vec::new(), opens: None, defs: HashMap::new(), region_checks: Vec::new(), code_checks: Vec::new(), class_bytes: HashMap::new(), kinds: Vec::new(), hidden: String::new(), nest: None, spans: Vec::new(), skipped: [false; 256], start_in: None, fills: Vec::new(), field_writes: Vec::new(), ends: None, fill_proofs: Default::default(), opener_at: [[true; 256]; 4], start_checks: Vec::new(), stretch_checks: Vec::new(), tail_checks: Vec::new(), code_after: String::new(), end_checks: Vec::new() }.source()
}
