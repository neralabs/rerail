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
    /// Each row's code for `fill_I` and `xl_I` after its token.
    rows: Vec<(usize, String, String)>,
}

/// How a region token ends, for what is before a byte outside the regions (`in_region_of`'s
/// shifts and fills): its kind; where its closer is after it, the closer's first bytes (the
/// byte after the token); the bytes its last bytes may be (`last[m]`: `m` bytes before the last).
struct End {
    k: usize,
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
        // string), as the lowering's reader reads them.
        let closer = match rs.as_slice() {
            [one] => one.closers(),
            _ => rs
                .iter()
                .map(|q| rs.iter().filter(|o| o.kind != q.kind).fold(q.closers(), |c, o| Expr::And(Box::new(c), Box::new(Expr::Not(Box::new(name(format!("f_{}", o.kind))))))))
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
    /// A carry from block to block of its own (`fill`, `escaped`): `c_NAME`.
    carry: bool,
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

/// Whether `text` reads the variable `var` (not one whose name it starts).
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
    /// its opener's local, the regions it is `after` (by index), and whether it has none (a region
    /// of those is `plain`, the one the `after`s ask of). `None`: none, or one the loop does not
    /// open yet (`else E bytes M`).
    opens: Option<Vec<(String, String, Vec<usize>)>>,
    /// Each mask's expression, by name: what a token reads of one made of the regions.
    defs: HashMap<String, Expr>,
    /// The escapes over a region's bytes, tested where its token reads it (`in_region`), by
    /// region: each its reason and its masks at the start, after it to the closer, and over the
    /// closer.
    region_checks: Vec<Vec<(String, [Option<String>; 3])>>,
    /// The escapes over the bytes outside every region token, tested over each token of code's
    /// bytes: each its reason and its mask there.
    code_checks: Vec<(String, String)>,
    /// Each class's bytes.
    class_bytes: HashMap<String, [bool; 256]>,
    /// The regions (`regions H { … }`) and the name of their hidden bytes.
    kinds: Vec<Region>,
    hidden: String,
    /// The fills the token loop keeps.
    spans: Vec<Span>,
    /// The bytes the loop skips between tokens: no token starts at one.
    skipped: [bool; 256],
    /// The fills the loop keeps (`LoopFill`).
    fills: Vec<LoopFill>,
    /// How each region token may end (`End`), and the bytes its first bytes may be: what a
    /// byte outside the regions may have before and after it in a region.
    ends: Option<Vec<End>>,
    opener_at: [[bool; 256]; 4],
    /// The escapes tested at each token's start; at the file's end, of a fill the loop keeps.
    start_checks: Vec<(String, String)>,
    end_checks: Vec<(String, usize)>,
}

impl Gen<'_> {
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
        self.nodes.push(Node { name, what, code, lag, kept: false, carry });
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
                if n.kept {
                    let _ = writeln!(out, "            if q < nw {{\n                {}[q] = w;\n            }}", n.name);
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
            // A start is never a skipped byte: a mask at none of the others is none there, one
            // at every one of them is every start.
            let at = self.bytes_at(e, 0, &mut HashMap::new());
            if (0..256).all(|i| self.skipped[i] || !at[i]) {
                return Ok(Some("false".to_owned()));
            }
            if self.skipped.iter().any(|&b| b)
                && let Some(x) = self.exact_bytes(e)
                && (0..256).all(|i| self.skipped[i] || x[i])
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
                    _ => format!("({a} && {b})"),
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
            return Ok(Some(format!("any_in(&{}, at, s - 1)", self.val(e)?)));
        }
        let Some(i) = (0..self.fills.len()).find(|&i| self.fills[i].xg.is_none() && contains_fill(self, e, i)) else { return Ok(None) };
        let (one, none) = (simplify_all(subst_fill(self, e, i, &Expr::Not(Box::new(Expr::Empty)))), simplify_all(subst_fill(self, e, i, &Expr::Empty)));
        let (Some(a), Some(b)) = (self.gap_code(&one, used)?, self.gap_code(&none, used)?) else { return Ok(None) };
        used.insert(i);
        let f = Known::Test(format!("fgap_{i}"));
        let k = |c: String| if c == "false" { Known::No } else { Known::Test(c) };
        Ok(Some(f.clone().and(k(a)).or(f.not().and(k(b))).code()))
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
                a => (false, a.clone()),
            };
            let (Some(x0), Some(e0)) = (self.in_region(&x, None, 0), self.in_region(stop, None, 0)) else { continue };
            if self.regional(&x0) || self.regional(&e0) {
                continue;
            }
            let skipped = self.skipped;
            let Some(xg) = self.in_region_of(&x, None, 0, &skipped, &mut HashMap::new()) else { continue };
            if self.regional(&xg) {
                continue;
            }
            let eb = self.bytes_at(&e0, 0, &mut HashMap::new());
            if (0..256).any(|i| eb[i] && skipped[i]) {
                continue;
            }
            let none = |v: Option<Expr>| matches!(v, Some(Expr::Empty));
            if !(0..n).all(|k| (0..3).all(|part| none(self.in_region(&x, Some(k), part))) && (1..3).all(|part| none(self.in_region(stop, Some(k), part)))) {
                continue;
            }
            let i = self.fills.len();
            let (xv, ev) = (if matches!(x0, Expr::Empty) { None } else { Some(self.val(&x0)?) }, if matches!(e0, Expr::Empty) { None } else { Some(self.val(&e0)?) });
            let xb = self.bytes_at(&x0, 0, &mut HashMap::new());
            let s_at = |xs: Known| if prev { Known::Test(format!("xs_{i}")) } else { xs };
            let carry = Known::Test(format!("fs_{i}"));
            let mut codes = Vec::new();
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
                    // A region token: `X` at none of its bytes, `E` at its start only.
                    let mut v = Known::No;
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
                        v = v.or(cond.and(s_at(Known::No).or(es.not().and(carry.clone()))));
                    }
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
                        (fs, xs)
                    } else {
                        let Some(et) = self.known_over(&e0, &tb, || None) else {
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
                            _ => {
                                ok = false;
                                break;
                            }
                        };
                        let pick = |a: Known, b: Known| {
                            let (a, b) = (a.code(), b.code());
                            if a == b { Known::Test(a) } else { Known::Test(format!("if l > s {{ {a} }} else {{ {b} }}")) }
                        };
                        (pick(ft, fs), pick(xt, xs))
                    }
                };
                codes.push((*row, fl.code(), xl.code()));
            }
            if !ok {
                continue;
            }
            let xg = if matches!(xg, Expr::Empty) { None } else { Some(self.val(&xg)?) };
            self.fills.push(LoopFill { name: name.clone(), key, shown: show(e), prev, x, xg, rows: codes });
        }
        Ok(())
    }

    /// `e` in one part of a token that reads region `k` (decision 0551's step B): at its start
    /// (`part` 0), after it to the closer (1), over the closer (2); and outside every region token
    /// (`k` none). There the regions' masks are known: a region's opener holds at its start, its
    /// fill from there to the closer, the hidden bytes after its start through its closer, none
    /// of them outside. `None`: `e` reads one of them shifted or carried.
    fn in_region(&self, e: &Expr, k: Option<usize>, part: u8) -> Option<Expr> {
        self.in_region_of(e, k, part, &[true; 256], &mut HashMap::new())
    }

    /// `in_region` where the byte is one of `ctx` (the bytes the rest of a `&` holds at): a mask
    /// that holds at none of them is none there.
    fn in_region_of(&self, e: &Expr, k: Option<usize>, part: u8, ctx: &[bool; 256], memo: &mut ByteMemo) -> Option<Expr> {
        let all = || Expr::Not(Box::new(Expr::Empty));
        let opens = self.opens.as_ref()?;
        // A region's start is its opener's first byte.
        let start_ctx: [bool; 256];
        let ctx = match (k, part) {
            (Some(k), 0) if k < self.kinds.len() => {
                let ob = self.bytes_at(&self.kinds[k].opener, 0, memo);
                start_ctx = std::array::from_fn(|i| ctx[i] && ob[i]);
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
            return self.outside_shift(f, args, ctx, memo);
        }
        Some(match e {
            // Assumed at a region token's last byte: none outside one, nor before its closer.
            Expr::Name(n) if self.is_end_name(n) => {
                if k.is_none() || part < 2 {
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
                    k == Some(q) && part < 2
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
                let (x, y) = (self.bytes_at(a, 0, memo), self.bytes_at(b, 0, memo));
                let ca: [bool; 256] = std::array::from_fn(|i| ctx[i] && y[i]);
                let cb: [bool; 256] = std::array::from_fn(|i| ctx[i] && x[i]);
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
    fn outside_shift(&self, f: &str, args: &[Arg], ctx: &[bool; 256], memo: &mut ByteMemo) -> Option<Expr> {
        let ends = self.ends.as_ref()?;
        let all = [true; 256];
        let meets = |x: &[bool; 256], y: &[bool; 256]| (0..256).any(|i| x[i] && y[i]);
        // Whether some way `e` may hold at a byte of `ctx` fits the bytes `at(o)` the place
        // leaves `o` bytes on.
        let fits = |e: &Expr, at: &dyn Fn(i64) -> [bool; 256], memo: &mut ByteMemo| {
            self.cubes(e, memo).iter().any(|c| {
                let here = c.get(&0).copied().unwrap_or(all);
                let here: [bool; 256] = std::array::from_fn(|i| here[i] && ctx[i]);
                meets(&here, &at(0)) && c.iter().all(|(o, b)| meets(b, &at(*o)))
            })
        };
        match (f, args) {
            ("prev" | "next", [Arg::E(a), rest @ ..]) => {
                let j = match rest {
                    [] => 1i64,
                    [Arg::N(j)] => *j as i64,
                    _ => return None,
                };
                let a0 = self.in_region_of(a, None, 0, &all, memo)?;
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
                            !fits(&e, &at, memo) && !fits(&e0, &at, memo)
                        })
                    } else {
                        // `o` bytes on is `o - j + m` after the token's first.
                        let at = |o: i64| match o - j + m {
                            d @ 0..=3 => self.opener_at[d as usize],
                            _ => all,
                        };
                        !fits(&e, &at, memo) && !fits(&e0, &at, memo)
                    };
                    if !ok {
                        return None;
                    }
                }
                Some(e0)
            }
            ("fill", [Arg::E(st), Arg::E(stop)]) => {
                let (s0, e0) = (self.in_region_of(st, None, 0, &all, memo)?, self.in_region_of(stop, None, 0, &all, memo)?);
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
                    ends.push(End { k: g.k, after, last });
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
            // A class's complement, at its own byte; `~~a`, `a`'s; else nothing known.
            Expr::Not(a) => match &**a {
                Expr::Name(n) if self.class_bytes.contains_key(n) => one(0, self.class_bytes[n].map(|b| !b)),
                Expr::Not(b) => self.cubes(b, memo),
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
        let mut body = 0i64;
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
                    let mut inside = vec![self.in_region_of(&p3, None, 0, &at_stop, &mut HashMap::new())];
                    for k in 0..n {
                        for part in 1..3 {
                            inside.push(self.in_region_of(&p3, Some(k), part, &at_stop, &mut HashMap::new()));
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
        for (_, st) in rows {
            let Stmt::Token { kind, last, rule, .. } = st else { continue };
            if matches!(rule, Some(TokenRule::Inside(..) | TokenRule::Read(_) | TokenRule::ReadParts(..))) {
                continue;
            }
            let tb = self.row_tail_bytes(kind, rule, last);
            if (0..256).any(|i| at[i] && tb[i]) {
                return Ok(false);
            }
        }
        if !(0..n).all(|k| (1..3).all(|part| matches!(self.in_region(e, Some(k), part), Some(Expr::Empty)))) {
            return Ok(false);
        }
        let Some(here) = self.at_start(e)? else { return Ok(false) };
        if here != "false" {
            self.start_checks.push((why.to_owned(), here));
        }
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
        for k in 0..n {
            let mut parts: [Option<Expr>; 3] = [None, None, None];
            for part in 0..3u8 {
                let Some(x) = self.in_region(e, Some(k), part) else { return Ok(false) };
                // A region that closes has its closer after the file's last byte: at its start and in it, never.
                if !matches!(x, Expr::Empty) && !(part < 2 && self.is_last_byte(&x)) {
                    parts[usize::from(part)] = Some(x);
                }
            }
            if parts.iter().any(Option::is_some) {
                if !read.contains(&k) {
                    return Ok(false);
                }
                found.push((k, parts));
            }
        }
        if self.region_checks.len() < n {
            self.region_checks.resize(n, Vec::new());
        }
        for (k, parts) in found {
            let mut v: [Option<String>; 3] = [None, None, None];
            for (i, x) in parts.into_iter().enumerate() {
                if let Some(x) = x {
                    v[i] = Some(self.val(&x)?);
                }
            }
            self.region_checks[k].push((why.to_owned(), v));
        }
        if let Some(v) = outside {
            let v = self.val(&v)?;
            self.code_checks.push((why.to_owned(), v));
        }
        Ok(true)
    }

    /// The tests of region `kind`'s escapes in its token's read, once its closer `c` and its last
    /// byte `last` are known; empty for none.
    fn region_tests(&self, kind: &str) -> String {
        let Some(k) = self.opens.as_ref().and_then(|o| o.iter().position(|(q, _, _)| q == kind)) else { return String::new() };
        let mut out = String::new();
        for (why, [at, inside, closer]) in self.region_checks.get(k).map(Vec::as_slice).unwrap_or(&[]) {
            let mut tests = Vec::new();
            match (at, inside) {
                // The same mask at the start and after it: one range.
                (Some(a), Some(i)) if a == i => tests.push(format!("any_in(&{a}, s, c - 1)")),
                _ => {
                    if let Some(m) = at {
                        tests.push(format!("bit(&{m}, s)"));
                    }
                    if let Some(m) = inside {
                        tests.push(format!("any_in(&{m}, s + 1, c - 1)"));
                    }
                }
            }
            if let Some(m) = closer {
                tests.push(format!("any_in(&{m}, c, last)"));
            }
            let _ = write!(out, "// {why}\nif {} {{\n    refused = true;\n    break;\n}}\n", tests.join(" || "));
        }
        out
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
        }
        let _ = writeln!(self.body, "    {{\n        let mut at = 0usize;\n        // The region opened last of the kinds with no `after`.\n        let mut plain = usize::MAX;\n        // Each kind's next opener, at or after where it was looked for (`n`: none).");
        let firsts: Vec<String> = opens.iter().map(|o| format!("first_bit(&{o}, 0).unwrap_or(n)")).collect();
        let _ = writeln!(self.body, "        let mut nx = [{}];", firsts.join(", "));
        let _ = writeln!(self.body, "        loop {{\n            let (mut p, mut k) = (n, usize::MAX);");
        for (k, r) in ks.iter().enumerate() {
            let look = format!("            if nx[{k}] < at {{\n                nx[{k}] = first_bit(&{}, at).unwrap_or(n);\n            }}\n            if nx[{k}] < p {{\n                p = nx[{k}];\n                k = {k};\n            }}\n", opens[k]);
            if r.after.is_empty() {
                let _ = write!(self.body, "            // `{}`\n{look}", r.kind);
            } else {
                let after: Vec<String> = r.after.iter().map(|a| ks.iter().position(|q| q.kind == *a).map(|q| format!("plain == {q}")).ok_or_else(|| format!("tier 1 as Rust: region `{}` after no region `{a}`", r.kind))).collect::<Result<_, _>>()?;
                let _ = write!(self.body, "            // `{}`, after {}\n            if {} {{\n{}            }}\n", r.kind, r.after.join(" "), after.join(" || "), indent(&look, "    "));
            }
        }
        let _ = writeln!(self.body, "            match k {{");
        for (k, r) in ks.iter().enumerate() {
            let (o, f) = (format!("m_o_{}", ident(&r.kind)), format!("m_f_{}", ident(&r.kind)));
            let c = &closers[k];
            let mut arm = String::new();
            let close = if r.outside {
                format!("                    let c = first_bit(&{c}, p + 1).unwrap_or(n);\n                    set(&mut {f}, p, c);\n                    set(&mut {hidden}, p + 1, c);\n                    at = c.max(p + 1);\n")
            } else {
                let last = match &ors[k] {
                    Some((c2, m)) => format!("if bit(&{c2}, c) {{ c + {} }} else {{ c + {} }}", m - 1, r.bytes - 1),
                    None => format!("c + {}", r.bytes - 1),
                };
                format!("                    match first_bit(&{c}, p + 1) {{\n                        Some(c) => {{\n                            let last = {last};\n                            set(&mut {f}, p, c);\n                            set(&mut {hidden}, p + 1, last + 1);\n                            at = last + 1;\n                        }}\n                        None => {{\n                            set(&mut {f}, p, n);\n                            set(&mut {hidden}, p + 1, n);\n                            at = n;\n                        }}\n                    }}\n")
            };
            let _ = write!(arm, "                    set(&mut {o}, p, p + 1);\n{close}");
            if r.after.is_empty() {
                let _ = writeln!(arm, "                    plain = {k};");
            }
            let body = match &r.empty {
                // `else E bytes M`: with no closer after it in the file, no region: `E`, its first `M` bytes.
                Some((e, m)) => format!("                    if first_bit(&{c}, p + 1).is_none() {{\n                        set(&mut m_{e}, p, p + 1);\n                        set(&mut {hidden}, p + 1, p + {m});\n                        at = (p + {m}).max(p + 1);\n                    }} else {{\n{}                    }}\n", indent(&arm, "    "), e = ident(e)),
                None => arm,
            };
            let _ = write!(self.body, "                {k} => {{\n                    // `{} = {} .. {}`\n{body}                }}\n", r.kind, show(&r.opener), show(&r.closer));
        }
        let _ = writeln!(self.body, "                _ => break,\n            }}\n        }}\n    }}");
        self.names.insert(rg.hidden.clone(), hidden);
        self.regional.insert(rg.hidden.clone());
        for r in ks {
            self.names.insert(format!("o_{}", r.kind), format!("m_o_{}", ident(&r.kind)));
            self.names.insert(format!("f_{}", r.kind), format!("m_f_{}", ident(&r.kind)));
            self.regional.insert(format!("o_{}", r.kind));
            self.regional.insert(format!("f_{}", r.kind));
            if let Some((e, _)) = &r.empty {
                self.names.insert(e.clone(), format!("m_{}", ident(e)));
                self.regional.insert(e.clone());
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
    fn closer_last(&mut self, r: &Region, closes: &[(String, Expr, u8)]) -> Result<String, String> {
        let mut out = String::new();
        for (_, e, n) in closes {
            let v = self.val(e)?;
            let _ = write!(out, "if bit(&{v}, c) {{ c + {} }} else ", n - 1);
        }
        let own = match &r.or {
            Some((c2, m)) => format!("if bit(&{}, c) {{ c + {} }} else {{ c + {} }}", self.val(c2)?, m - 1, r.bytes - 1),
            None => format!("c + {}", r.bytes - 1),
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
                if !(f == "fill" && p == "prev" && self.is_end(t) && !self.regional(stop)) {
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
                    self.spans.push(Span { f: name.clone(), t: t.clone(), e: stop.clone(), opens_at, open: Vec::new(), starts: Vec::new(), at_end: Vec::new(), passed: Vec::new() });
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
            if self.start_escape(why, e, &rows)? {
                continue;
            }
            if self.region_escape(why, e, &read, skip.as_ref())? {
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
                    let gap = self.in_region_of(at, None, 0, &skipped, &mut HashMap::new());
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
            let mask = match at {
                Expr::Empty => None,
                _ if acc.is_some() => None,
                e => Some(self.val(e)?),
            };
            let unless = unless
                .iter()
                .map(|w| x.words.iter().position(|x| x == w).map(|i| i as u32 + 1).ok_or_else(|| format!("tier 1 as Rust: mark {kind}: `unless after` a word no code names")))
                .collect::<Result<_, _>>()?;
            for c in checks {
                tests.push((c.kind.clone(), c.after, c.test, c.expr.clone(), c.least, bit));
            }
            gaps.push(Gap { bit, mask, unless, name: kind.clone(), acc });
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
        // The token loop: each top row's branch.
        let mut branches = String::new();
        let mut pair_fns = String::new();
        let mut checked: Vec<String> = Vec::new();
        // The gap accumulators a row's bytes after its start may hold.
        let mut tails: std::collections::BTreeSet<usize> = Default::default();
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
                            let some = if !self.spans.iter().any(|sp| sp.opens_at.iter().any(|(r, _)| r == row)) { some } else { with_close(&some) };
                            format!("match first_bit(&{c}, s + 1) {{\n                Some(c) => {some},\n                None => {{\n                    refused = true;\n                    break;\n                }}\n            }}")
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
                    let found = if !self.spans.iter().any(|sp| sp.opens_at.iter().any(|(r, _)| r == row)) { found } else { with_close(&found) };
                    (format!("read {} parts, to its closer", r.kind), format!("match first_bit(&{c}, s + 1) {{\n                Some(c) => {found},\n                None => {{\n                    refused = true;\n                    break;\n                }}\n            }}"))
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
            // The escapes outside the regions, over a token of code's bytes.
            if !region_row {
                for (why, v) in &self.code_checks {
                    let _ = write!(hits, "            // {why}\n            if any_in(&{v}, s, l) {{\n                refused = true;\n                break;\n            }}\n");
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
            let _ = write!(
                branches,
                "{}if {st} {{\n            // `{kind}`: {how}\n            let l = {l};\n            let l = l.max(s);\n{hits}            (l, {k}, {extra}, {bits:#x})\n        }} ",
                if branches.is_empty() { "" } else { "else " }
            );
        }
        // Each gap mark's bit at a grammar token, and at an extra the marks it keeps of its own gap.
        let mut gap_code = String::new();
        let mut own_code = String::new();
        for g in &gaps {
            let since = match (&g.mask, g.acc) {
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
        let mut token_code = String::new();
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
            let _ = write!(token_code, "        if {test} {{\n            marks |= 1 << {bit};\n        }}\n");
        }
        // The masks tier 2 reads (`readers`).
        let readers = readers(x.stmts);
        let mut fields: Vec<String> = Vec::new();
        let mut fields_init = String::new();
        for rd in &readers {
            for (f, e) in &rd.masks {
                let v = self.val(e)?;
                fields.push(f.clone());
                let _ = writeln!(fields_init, "            {f}: {v},");
            }
        }
        // The region a token's start opens, as the walk opens them (`regions`): the first declared whose
        // opener holds there, a kind `after` others where the region opened last of those with none is one.
        let mut open_code = String::new();
        if let Some(opens) = &self.opens {
            let mut arms = Vec::new();
            for (k, (kind, o, after)) in opens.iter().enumerate() {
                let cond = if after.is_empty() { format!("bit(&{o}, s)") } else { format!("({}) && bit(&{o}, s)", after.iter().map(|a| format!("plain == {a}")).collect::<Vec<_>>().join(" || ")) };
                arms.push(format!("if {cond} {{\n            // `{kind}`\n            {k}\n        }}"));
            }
            let plain: Vec<String> = opens.iter().enumerate().filter(|(_, r)| r.2.is_empty()).map(|(k, _)| k.to_string()).collect();
            let _ = write!(open_code, "        // The region a token here opens, as the walk opens them: the first declared whose opener holds, a kind after others where the region opened last of those with none (`plain`) is one.\n        let open = {} else {{\n            usize::MAX\n        }};\n        if matches!(open, {}) {{\n            plain = open;\n        }}\n", arms.join(" else "), plain.join(" | "));
        }
        let mut span_decl = String::new();
        let mut span_after = String::new();
        let mut acc_after = String::new();
        // The gap masks the loop keeps.
        for (j, a) in accs.iter().enumerate() {
            let what = &a.what;
            let _ = writeln!(span_decl, "    // Whether a gap mask (`{what}`) held since the grammar token before{}.\n    let mut acc_{j} = false;{}", if a.own { ", and since the record before" } else { "" }, if a.own { format!("\n    let mut rec_{j} = false;") } else { String::new() });
            if let Some(g) = &a.gap {
                let _ = write!(open_code, "        // The skipped bytes before the token, for the gap masks.\n        if s > at && {g} {{\n            acc_{j} = true;\n{}        }}\n", if a.own { format!("            rec_{j} = true;\n") } else { String::new() });
            }
            if a.here != "false" {
                let _ = writeln!(open_code, "        let here_{j} = {};", a.here);
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
        if !self.spans.is_empty() {
            span_decl.push_str("    // The closer the region token read ends at, where a fill the loop keeps opens after it.\n    let mut close: usize;\n");
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
            if here.is_empty() && passed.is_empty() && first.is_empty() && rest.is_empty() {
                continue;
            }
            let _ = write!(open_code, "        // `{}`: at the starts in it, and at its end.\n        if s >= span_{i}.1 {{\n", sp.f);
            if !here.is_empty() || !passed.is_empty() {
                let _ = write!(open_code, "            let c = span_{i}.1;\n            if s == c {{\n{here}            }} else {{\n                // A token covered it.\n{passed}            }}\n");
            }
            let _ = write!(open_code, "            span_{i} = (usize::MAX, usize::MAX);\n        }}");
            if first == rest && !first.is_empty() {
                // The same tests at its first byte and after it: one.
                let mut both = String::new();
                for (why, f, _) in &sp.starts {
                    if let Some(f) = f {
                        both.push_str(&refuse(why, f, "            "));
                    }
                }
                let _ = write!(open_code, " else if s >= span_{i}.0 {{\n{both}        }}");
            } else if !first.is_empty() || !rest.is_empty() {
                let _ = write!(open_code, " else if s == span_{i}.0 {{\n{first}        }} else if s > span_{i}.0 {{\n{rest}        }}");
            }
            open_code.push('\n');
            if !after.is_empty() {
                let _ = write!(span_after, "    // `{}`: a token covered its end.\n    if span_{i}.1 < n {{\n        let c = span_{i}.1;\n{after}    }}\n", sp.f);
            }
        }
        let mut fill_top = String::new();
        let mut fill_end = String::new();
        for (i, f) in self.fills.iter().enumerate() {
            let _ = writeln!(span_decl, "    // `{}` at the last byte of the token before{}.\n    let mut fill_{i} = false;{}", f.shown, if f.prev { ", and its `X`" } else { "" }, if f.prev { format!("\n    let mut xl_{i} = false;") } else { String::new() });
            let (fs, xs, fgap) = match (f.prev, &f.xg) {
                (true, None) => (format!("fill_{i} || (s > at && xl_{i})"), Some(format!("s == at && xl_{i}")), Some(format!("fill_{i} || xl_{i}"))),
                (true, Some(g)) => (format!("fill_{i} || (s > at && (xl_{i} || (s > at + 1 && any_in(&{g}, at, s - 2))))"), Some(format!("if s > at {{ bit(&{g}, s - 1) }} else {{ xl_{i} }}")), None),
                (false, None) => (format!("fill_{i}"), None, Some(format!("fill_{i}"))),
                (false, Some(g)) => (format!("fill_{i} || (s > at && any_in(&{g}, at, s - 1))"), None, None),
            };
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
        for (why, t) in &self.start_checks {
            let _ = write!(open_code, "        // {why}\n        if {t} {{\n            refused = true;\n            break;\n        }}\n");
        }
        // Only the reads of them the code makes.
        let rest = format!("{branches}{open_code}{gap_code}{own_code}{token_code}");
        let mut top = String::new();
        let mut pending_comment = String::new();
        for line in fill_top.lines() {
            if line.trim_start().starts_with("//") {
                pending_comment = format!("{line}\n");
                continue;
            }
            let var = line.trim_start().trim_start_matches("let ").split(' ').next().unwrap_or("");
            if !var.is_empty() && uses(&rest, var) {
                top.push_str(&pending_comment);
                pending_comment.clear();
                let _ = writeln!(top, "{line}");
            }
        }
        open_code.insert_str(0, &top);
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
        out.push_str("/// `src`'s records.\npub fn lex(src: &[u8]) -> Lexed {\n    let n = src.len();\n    let nw = n.div_ceil(64);\n    // Each word's bits inside the file.\n    let lv: Vec<u64> = (0..nw).map(|q| if 64 * q + 64 <= n { !0 } else { (1u64 << (n % 64)) - 1 }).collect();\n");
        let kept: Vec<&str> = self.nodes.iter().filter(|n| n.kept).map(|n| n.name.as_str()).collect();
        let _ = writeln!(out, "    // Stage 1: the masks the regions and the tokens read, and whether an escape holds.\n    let Stage1 {{ {}, refused: escapes }} = stage1_best(src);", kept.join(", "));
        out.push_str(&self.body);
        let _ = write!(
            out,
            "    // The tokens: from the first byte `skip` does not hold, the token whose start holds there, to its last byte by its rule.\n    let mut starts: Vec<u32> = Vec::with_capacity(n / 4 + 1);\n    let mut lasts: Vec<u32> = Vec::with_capacity(n / 4 + 1);\n    let mut codes: Vec<u32> = Vec::with_capacity(n / 4 + 1);\n    let mut at = 0usize;\n    // The gap's start, since the grammar token before and since the record before; the grammar token before's word; the marks its tests carry to the next grammar token.\n    let (mut from, mut from_record, mut before, mut pending) = (0usize, 0usize, 0u32, 0u32);\n{plain_decl}    while let Some(s) = first_clear(&{skip}, at, n) {{\n{open_code}        let mut hits = 0u32;\n        let (l, kind, extra, mut marks): (usize, u32, bool, u32) = {branches}else {{\n            // A byte no token begins with.\n            refused = true;\n            break;\n        }};\n        let word = if extra {{ 0 }} else {{ word(&src[s..=l]) }};\n        // The token marks: tier 2 tests the rest of each.\n{token_code}        if extra {{\n            // An extra keeps the marks of its own gap; its tests' hits go to the next grammar token.\n{own}            pending |= hits & 0xffff | hits >> 16;\n        }} else {{\n            let tested = pending | hits;\n{gap_code}            from = s + 1;\n            before = word;\n            pending = hits >> 16;\n        }}\n        from_record = s + 1;\n{acc_after}        starts.push(s as u32);\n        lasts.push(l as u32);\n        codes.push(kind | word << 8 | marks << 16);\n        at = l + 1;\n    }}\n    let _ = (from, from_record, before, pending);\n{span_after}    Lexed {{\n        starts,\n        lasts,\n        codes,\n        masks: Masks {{\n{fields_init}        }},\n        refused,\n    }}\n}}\n",
            own = own_code,
            plain_decl = format!("{}{span_decl}", if self.opens.is_some() { "    // The region opened last of the kinds with no `after`.\n    let mut plain = usize::MAX;\n" } else { "" }),
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
    Gen { x, body: String::new(), names: HashMap::new(), temps: HashMap::new(), next: 0, nodes: Vec::new(), node_of: HashMap::new(), regional: Default::default(), pairs: Vec::new(), sinks: Vec::new(), opens: None, defs: HashMap::new(), region_checks: Vec::new(), code_checks: Vec::new(), class_bytes: HashMap::new(), kinds: Vec::new(), hidden: String::new(), spans: Vec::new(), skipped: [false; 256], fills: Vec::new(), ends: None, opener_at: [[true; 256]; 4], start_checks: Vec::new(), end_checks: Vec::new() }.source()
}
