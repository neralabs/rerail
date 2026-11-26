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

// Mounted by `rerail-bitlex` from the compiler's folder (`#[path]`): its submodule is found beside it.
#[path = "bitlex_lex_rust/bytes.rs"]
mod bytes;

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
    /// The grammar's assumptions (decision 0607): its scope (`None` at every token's start, else
    /// in the tokens of a kind), a mask's name, what it reads as there, and whether the prover
    /// proved what tier 1 makes of it.
    pub assumed: &'a [(Option<String>, String, Expr, bool)],
    /// The masks assumed only at a region token's last byte (`assume … at ends`), each with
    /// whether the prover proved it.
    pub assumed_ends: &'a [(String, bool)],
    /// The mark bits tier 2 as one loop reads of a record's code, where it is written: where the loop reads the bytes,
    /// tier 1 writes no other token mark (decision 0623).
    pub tier2_reads: Option<u32>,
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
    after: Option<Bytes>,
    last: [Bytes; 4],
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

/// The error statements tier 2 tests at a region's escapes (`bitlex_piece_checks`): of a grammar whose one region read
/// in parts has one part of a fixed length, as tier 2's loop reads it.
pub fn piece_covers(stmts: &[Stmt]) -> Vec<usize> {
    match readers(stmts).as_slice() {
        [rd] => match rd.parts.as_slice() {
            [(_, start, PartLen::Bytes(_), _)] => crate::bitlex_piece_checks::piece_checks(stmts, start, None).map(|c| c.covers).unwrap_or_default(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Each ring read in `code` (`r_NAME[q % RING]`, `[(q + RING - 1) % RING]`, `[(q + 1) % RING]`): its node and its
/// block's offset from the reader's.
fn ring_reads(code: &str, by_name: &HashMap<&str, usize>) -> Vec<(usize, i32)> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(k) = code[at..].find("r_") {
        let start = at + k;
        at = start + 2;
        if start > 0 && code[..start].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let Some(open) = code[start..].find('[') else { break };
        let Some(&id) = by_name.get(&code[start + 2..start + open]) else { continue };
        let rest = &code[start + open + 1..];
        let off = if rest.starts_with("q % RING]") {
            0
        } else if rest.starts_with("(q + RING - 1) % RING]") {
            -1
        } else if rest.starts_with("(q + 1) % RING]") {
            1
        } else {
            continue;
        };
        out.push((id, off));
    }
    out
}

/// The blocks between a word's write (its stage `lx`) and its read (by stage `lr`, at `off` from the reader's block):
/// `None` where the writer's stage may have stopped by then.
fn age(lr: u32, lx: u32, off: i32) -> Option<u32> {
    if lr >= lx + 2 {
        return None;
    }
    u32::try_from(i64::from(lr) - i64::from(lx) - i64::from(off)).ok()
}

/// `code` (of a node of stage `lr`) with each ring read its local where it has one.
fn local_reads(code: &str, by_name: &HashMap<&str, usize>, nodes: &[Node], lr: u32) -> String {
    let mut out = String::new();
    let mut rest = code;
    loop {
        let reads = ring_reads(rest, by_name);
        let Some(&(id, off)) = reads.first() else { break };
        let name = &nodes[id].name;
        let index = match off {
            0 => "q % RING]",
            -1 => "(q + RING - 1) % RING]",
            _ => "(q + 1) % RING]",
        };
        let pat = format!("r_{name}[{index}");
        let k = rest.find(&pat).expect("the read found");
        out.push_str(&rest[..k]);
        match age(lr, nodes[id].lag, off) {
            Some(0) => {
                let _ = write!(out, "v_{name}");
            }
            Some(1) => {
                let _ = write!(out, "p_{name}");
            }
            _ => out.push_str(&pat),
        }
        rest = &rest[k + pat.len()..];
    }
    out.push_str(rest);
    out
}

/// `e`'s bits in a file of `n` bytes whose first `pre` are its `prefix`, where it reads the file's length alone (no
/// class): `~empty` is the file's bytes, `next` past the end nothing, each operation's bits in the file.
fn length_bits(e: &Expr, defs: &HashMap<String, Expr>, n: usize, pre: usize, depth: u32) -> Option<Vec<bool>> {
    if depth > 32 {
        return None;
    }
    let lv = |i: usize| i >= pre;
    let r = |x: &Expr| length_bits(x, defs, n, pre, depth + 1);
    Some(match e {
        Expr::Empty => vec![false; n],
        Expr::Name(m) => r(defs.get(m)?)?,
        Expr::Not(a) => r(a)?.iter().enumerate().map(|(i, b)| !b && lv(i)).collect(),
        Expr::And(a, b) => r(a)?.iter().zip(r(b)?).map(|(x, y)| *x && y).collect(),
        Expr::Or(a, b) => r(a)?.iter().zip(r(b)?).map(|(x, y)| *x || y).collect(),
        Expr::Xor(a, b) => r(a)?.iter().zip(r(b)?).map(|(x, y)| *x ^ y).collect(),
        Expr::Call(f, args) if f == "next" || f == "prev" => {
            let Some(Arg::E(a)) = args.first() else { return None };
            let k = match args.get(1) {
                None => 1,
                Some(Arg::N(k)) => usize::try_from(*k).ok()?,
                _ => return None,
            };
            let a = r(a)?;
            (0..n).map(|i| if f == "next" { i + k < n && a[i + k] } else { i >= k && a[i - k] && lv(i) }).collect()
        }
        _ => return None,
    })
}

/// The records tier 1 writes, as a mask of the grammar: the tokens' starts (but a region's inside tokens), their
/// common conjuncts once (`factored`), named where an error refuses the bytes they differ at (`refused_equal`).
fn records_of(stmts: &[Stmt]) -> Option<Expr> {
    let u = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::Token { rule: Some(TokenRule::Inside(..)), .. } => None,
            Stmt::Token { start, .. } => Some(start.clone()),
            _ => None,
        })
        .reduce(|a, b| Expr::Or(Box::new(a), Box::new(b)))?;
    let f = factored(&u, stmts);
    Some(refused_equal(&f, stmts).unwrap_or(f))
}

/// A class's or a byte mask's bytes, where its byte decides it.
fn bytes_of_expr(e: &Expr, stmts: &[Stmt]) -> Option<[bool; 256]> {
    let mut set = [false; 256];
    for b in 0..=255u8 {
        set[usize::from(b)] = at_byte(e, b, stmts)?;
    }
    Some(set)
}

/// The bytes a token that runs over `run` (`run R`) may end before, as simdjson's `is_not_structural_or_whitespace`
/// is false at them: a byte no token starts at (skipped), or one a record may start at right after a byte of `run`
/// (each `prev(X)`, `X` holding all of `run`, true there; one holding none of it false). At any other byte after a run
/// no record starts and none is skipped: tier 1's records equal the tokens' starts but where an error refuses the
/// file (`refused_equal`), so the file is refused, and tier 2 refuses it there.
pub fn ends_after_run(stmts: &[Stmt], run: &Expr) -> Option<[bool; 256]> {
    let r = bytes_of_expr(run, stmts)?;
    let records = records_of(stmts)?;
    let mut ats = Vec::new();
    atoms(&records, stmts, &mut ats);
    if ats.len() > 16 {
        return None;
    }
    let forced: Vec<Option<bool>> = ats
        .iter()
        .map(|(_, e)| match e {
            Expr::Call(f, a) if f == "prev" => match a.as_slice() {
                [Arg::E(x)] | [Arg::E(x), Arg::N(1)] => {
                    let xs = bytes_of_expr(x, stmts)?;
                    if (0..256).all(|b| !r[b] || xs[b]) {
                        Some(true)
                    } else if (0..256).all(|b| !r[b] || !xs[b]) {
                        Some(false)
                    } else {
                        None
                    }
                }
                _ => None,
            },
            _ => None,
        })
        .collect();
    // The tokens outside a region (a region's inside ones start only inside it).
    let starts: Vec<&Expr> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::Token { rule: Some(TokenRule::Inside(..)), .. } => None,
            Stmt::Token { start, .. } => Some(start),
            _ => None,
        })
        .collect();
    let mut ok = [false; 256];
    for b in 0..=255u8 {
        let skipped = starts.iter().all(|s| false_at(s, b, stmts));
        let starts_here = (0..1u32 << ats.len()).filter(|v| forced.iter().enumerate().all(|(i, f)| f.is_none_or(|f| (v >> i & 1 == 1) == f))).any(|v| at_atoms(&records, b, stmts, &ats, v) == Some(true));
        ok[usize::from(b)] = skipped || starts_here;
    }
    Some(ok)
}

/// The errors tier 2 tests at a run token's end, where its end test is `ends_after_run`'s: each holds only at a byte
/// right after one of the run's bytes (a conjunct `prev(X)`, `X` within the run's bytes), outside every region (a
/// conjunct `~T`, `T` a region's fill but its opener, `F ^ O`), and only at a byte the run may not end before and
/// does not run over. Such a byte is a run token's next (the run's bytes outside a region are tokens tier 2 reads, or
/// refuses), and tier 2 refuses it there, as simdjson refuses a scalar's dirty end (`[1"a"]`).
pub fn run_end_covers(stmts: &[Stmt]) -> Vec<usize> {
    let runs: Vec<&Expr> = stmts.iter().filter_map(|s| if let Stmt::Token { rule: Some(TokenRule::Run(r)), .. } = s { Some(r) } else { None }).collect();
    let [run] = runs.as_slice() else { return Vec::new() };
    let (Some(ok), Some(r)) = (ends_after_run(stmts, run), bytes_of_expr(run, stmts)) else { return Vec::new() };
    let def = |n: &str| stmts.iter().rev().find_map(|s| if let Stmt::Mask(m, e) = s && m == n { Some(e.clone()) } else { None });
    // Each region's fill (and the masks that are it alone) and its opener.
    let regions: Vec<(Vec<String>, Expr)> = readers(stmts)
        .iter()
        .flat_map(|rd| rd.regions.iter())
        .map(|g| {
            let mut fills = vec![format!("f_{}", g.kind)];
            for s in stmts {
                if let Stmt::Mask(m, Expr::Name(n)) = s
                    && fills.contains(n)
                {
                    fills.push(m.clone());
                }
            }
            (fills, g.opener.clone())
        })
        .collect();
    let tail = |t: &str| {
        let Some(Expr::Xor(a, b)) = def(t) else { return false };
        regions.iter().any(|(fills, o)| {
            let fill = |x: &Expr| matches!(x, Expr::Name(n) if fills.contains(n));
            let opener = |x: &Expr| format!("{x:?}") == format!("{o:?}");
            (fill(&a) && opener(&b)) || (fill(&b) && opener(&a))
        })
    };
    let mut out = Vec::new();
    for (i, s) in stmts.iter().enumerate() {
        let Stmt::Error(_, e) = s else { continue };
        let mut cs = Vec::new();
        fn conj(e: &Expr, out: &mut Vec<Expr>) {
            if let Expr::And(a, b) = e {
                conj(a, out);
                conj(b, out);
            } else {
                out.push(e.clone());
            }
        }
        conj(e, &mut cs);
        let resolve = |c: &Expr| match c {
            Expr::Name(n) => def(n).unwrap_or_else(|| c.clone()),
            c => c.clone(),
        };
        let after_run = cs.iter().any(|c| match resolve(c) {
            Expr::Call(f, a) if f == "prev" => match a.as_slice() {
                [Arg::E(x)] | [Arg::E(x), Arg::N(1)] => bytes_of_expr(x, stmts).is_some_and(|xs| (0..256).all(|b| !xs[b] || r[b])),
                _ => false,
            },
            _ => false,
        });
        let outside = cs.iter().any(|c| matches!(c, Expr::Not(t) if matches!(&**t, Expr::Name(n) if tail(n))));
        if after_run && outside && (0..=255u8).all(|b| false_at(e, b, stmts) || (!ok[usize::from(b)] && !r[usize::from(b)])) {
            out.push(i);
        }
    }
    out
}

/// Whether tier 1 refuses a file whose region `kind` is open at its end: an error `F & L`, `F` its fill (`f_KIND`, or
/// a mask that is it alone) and `L` the file's last byte. Then a read of the region in tier 2 meets its closer before
/// the file's end.
pub fn refuses_open_at_end(stmts: &[Stmt], kind: &str) -> bool {
    let f = format!("f_{kind}");
    let mut defs: HashMap<String, Expr> = HashMap::new();
    let mut prefix = 0;
    let mut fills = vec![f.clone()];
    for s in stmts {
        match s {
            Stmt::Mask(m, e) => {
                if matches!(e, Expr::Name(n) if fills.contains(n)) {
                    fills.push(m.clone());
                }
                defs.insert(m.clone(), e.clone());
            }
            Stmt::Prefix(b) => prefix = b.len(),
            _ => {}
        }
    }
    stmts.iter().any(|s| match s {
        Stmt::Error(_, Expr::And(a, b)) => {
            let fill = |x: &Expr| matches!(x, Expr::Name(n) if fills.contains(n));
            (fill(a) && is_last_byte(b, &defs, prefix)) || (fill(b) && is_last_byte(a, &defs, prefix))
        }
        _ => false,
    })
}

/// Whether `e` holds at the file's last byte alone (outside its `prefix`), at every length tried: of up to three
/// blocks, the `prefix` there or not.
fn is_last_byte(e: &Expr, defs: &HashMap<String, Expr>, pre: usize) -> bool {
    (1..=200).all(|n| {
        [0, pre].iter().filter(|&&p| p <= n).all(|&p| match length_bits(e, defs, n, p, 0) {
            Some(bits) => bits.iter().enumerate().all(|(i, b)| *b == (i + 1 == n && i >= p)),
            None => false,
        })
    })
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
        if let Stmt::Token { kind, rule: Some(TokenRule::Inside(_, of, c, _)), .. } = s
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
    /// A class's bytes: its code is made again where stage 1 writes it, in the tables of the classes it reads.
    class: Option<std::boxed::Box<[bool; 256]>>,
    /// The expression it is, where the loop may read it from the bytes instead (decision 0623, `bytes`).
    expr: Option<Expr>,
}

/// A row of the token loop, for `dispatch`: its start, the test build's checks where the loop
/// first reads an assumption, and its code (a block that makes its record).
/// Stage 1's code in its parts (`Gen::stage1_parts`).
struct Stage1Text {
    /// The classes' nibble tables and the byte-pair errors' tables.
    tables: String,
    /// `Stage1`, the masks it gives out.
    record: String,
    /// `write_indexes`, where it writes the records.
    indexes: String,
    /// Its function's first lines, to `nw`.
    header: String,
    /// The words it keeps over the blocks, and the masks it writes.
    decls: String,
    /// Its steps (`t`), as an expression, and how many blocks its last masks are behind the classes.
    steps: String,
    most: u32,
    /// A step's code: block `t`'s classes, and each mask of its own block behind them.
    body: String,
    /// The tests once after the last step.
    finals: String,
    /// The `Stage1` it gives, and its function's end.
    ret: String,
}

/// A row's read of its token's word (`RowArm::word`): `word_at` in its place where the word table is made.
const WORD_READ: &str = "word(&src[s..=l])";

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
    /// Whether a word may be its token: a grammar token's whose first byte may be its first and the rest its tail's.
    /// Only such a row reads its token's word, as the lean lexer reads only a word run's.
    word: bool,
    /// Whether it reads a region with `open K` (`Region::open`): its record, where `open_bad` is set, `K`'s, with no
    /// word and no marks.
    open: bool,
}

/// `dispatch`'s choices, by the first byte's group: the region opened, and the row.
struct Picks {
    open: String,
    rows: String,
    /// Each byte's pick, for one load (`TOKEN`, decision 0623): its row where its group picks one with no test, else
    /// the group's tests' arm (`rows.len() + group`), else none (`u8::MAX`); and the tested groups' arms. None past 254.
    by_byte: Option<([u8; 256], String)>,
    /// Whether the rows are picked by the openers themselves: no `open`.
    fused: bool,
}

/// A class's nibble-table bit: the high nibbles and the low nibbles it holds (one bit a nibble).
/// The ways a mask may hold (`Gen::cubes`): each the bytes it needs at offsets from where it
/// holds; an offset not named, any byte.
type Cubes = Vec<Way>;

/// A set of bytes, a bit each: what a mask's ways need at an offset (`Gen::cubes`), and the
/// bytes about a byte (`Ctx`). `b[i]` reads byte `i`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Bytes([u64; 4]);

impl Bytes {
    const ALL: Bytes = Bytes([!0; 4]);
    const NONE: Bytes = Bytes([0; 4]);

    fn of(b: &[bool; 256]) -> Bytes {
        let mut w = [0u64; 4];
        (0..256).filter(|&i| b[i]).for_each(|i| w[i >> 6] |= 1 << (i & 63));
        Bytes(w)
    }

    fn has(&self, i: usize) -> bool {
        self.0[i >> 6] >> (i & 63) & 1 != 0
    }

    fn bools(&self) -> [bool; 256] {
        std::array::from_fn(|i| self.has(i))
    }

    fn and(self, o: Bytes) -> Bytes {
        Bytes(std::array::from_fn(|k| self.0[k] & o.0[k]))
    }

    fn or(self, o: Bytes) -> Bytes {
        Bytes(std::array::from_fn(|k| self.0[k] | o.0[k]))
    }

    fn not(self) -> Bytes {
        Bytes(self.0.map(|w| !w))
    }

    fn any(&self) -> bool {
        self.0.iter().any(|&w| w != 0)
    }

    /// Some byte of both.
    fn meets(&self, o: &Bytes) -> bool {
        (0..4).any(|k| self.0[k] & o.0[k] != 0)
    }

    /// Every byte of `self` one of `o`'s.
    fn within(&self, o: &Bytes) -> bool {
        (0..4).all(|k| self.0[k] & !o.0[k] == 0)
    }
}

impl std::ops::Index<usize> for Bytes {
    type Output = bool;
    fn index(&self, i: usize) -> &bool {
        if self.has(i) { &true } else { &false }
    }
}

impl From<[bool; 256]> for Bytes {
    fn from(b: [bool; 256]) -> Bytes {
        Bytes::of(&b)
    }
}

/// One way a mask may hold (`Cubes`): the bytes it needs at some offsets from where it holds,
/// in the offsets' order, each offset once.
#[derive(Clone, Debug, Default)]
struct Way(Vec<(i64, Bytes)>);

impl Way {
    fn get(&self, d: &i64) -> Option<&Bytes> {
        self.0.iter().find(|(o, _)| o == d).map(|(_, b)| b)
    }

    fn contains_key(&self, d: &i64) -> bool {
        self.0.iter().any(|(o, _)| o == d)
    }

    fn iter(&self) -> impl Iterator<Item = (&i64, &Bytes)> {
        self.0.iter().map(|(d, b)| (d, b))
    }

    /// Its bytes at `d` narrowed to `b`'s: whether any are left.
    fn and_at(&mut self, d: i64, b: &Bytes) -> bool {
        match self.0.binary_search_by_key(&d, |(o, _)| *o) {
            Ok(i) => {
                self.0[i].1 = self.0[i].1.and(*b);
                self.0[i].1.any()
            }
            Err(i) => {
                self.0.insert(i, (d, *b));
                b.any()
            }
        }
    }
}

impl FromIterator<(i64, Bytes)> for Way {
    fn from_iter<I: IntoIterator<Item = (i64, Bytes)>>(it: I) -> Way {
        let mut v: Vec<(i64, Bytes)> = Vec::new();
        for (d, b) in it {
            match v.binary_search_by_key(&d, |(o, _)| *o) {
                Ok(i) => v[i].1 = b,
                Err(i) => v.insert(i, (d, b)),
            }
        }
        Way(v)
    }
}

/// An expression interned (`Gen::intern`), its operands by id: hashed and compared a node at a
/// time, not as a tree. Two expressions have one id where they are the same.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Shape {
    Empty,
    Name(u32),
    Not(u32),
    And(u32, u32),
    Or(u32, u32),
    Xor(u32, u32),
    /// A call: its function's name id, and its arguments.
    Call(u32, std::rc::Rc<[ArgShape]>),
    Test(std::rc::Rc<Expr>),
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum ArgShape {
    E(u32),
    N(u64),
    S(std::rc::Rc<[u8]>),
}
/// The ways each name may hold, as `Gen::cubes` reads it.
type ByteMemo = HashMap<String, std::rc::Rc<Cubes>>;

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
struct Ctx([Bytes; 9]);

impl Ctx {
    fn all() -> Ctx {
        Ctx([Bytes::ALL; 9])
    }

    fn of(b: impl Into<Bytes>) -> Ctx {
        let mut c = Ctx::all();
        c.0[4] = b.into();
        c
    }

    fn at(&self, o: i64) -> Bytes {
        usize::try_from(o + 4).ok().and_then(|i| self.0.get(i)).copied().unwrap_or(Bytes::ALL)
    }

    /// Its byte's set narrowed to `b`'s.
    fn and0(&self, b: &Bytes) -> Ctx {
        let mut c = self.clone();
        c.0[4] = c.0[4].and(*b);
        c
    }
}

impl std::ops::Deref for Ctx {
    type Target = Bytes;
    fn deref(&self) -> &Bytes {
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

/// `code` with each refusal (`refused = true;` then `break;`) first a pause where stage 1 has blocks left to make
/// (decision 0619): what refuses the blocks made so far may hold past them.
fn pause_refusals(code: &str) -> String {
    let lines: Vec<&str> = code.lines().collect();
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if line.trim() == "refused = true;" && lines.get(i + 1).is_some_and(|n| n.trim() == "break;") {
            let pad = &line[..line.len() - line.trim_start().len()];
            let _ = writeln!(out, "{pad}if made < nw {{\n{pad}    wait = (2 * made - b).min(nw);\n{pad}    continue 'pass;\n{pad}}}");
        }
        out.push_str(line);
        out.push('\n');
    }
    if !code.ends_with('\n') {
        out.pop();
    }
    out
}

/// `code` with each search of a mask (`first_bit(&M, …)`) in the blocks stage 1 has made (`M[..made]`, decision 0619):
/// past them a mask is not made yet, so a search there would read every word to the file's end at each pause.
fn made_searches(code: &str) -> String {
    let mut out = String::new();
    let mut rest = code;
    while let Some(i) = rest.find("first_bit(&") {
        let at = i + "first_bit(&".len();
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let name = rest.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(rest.len());
        out.push_str(&rest[..name]);
        if rest[name..].starts_with(',') {
            out.push_str("[..made]");
        }
        rest = &rest[name..];
    }
    out.push_str(rest);
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
    c.0.retain(|(d, _)| v.iter().all(|w| w.contains_key(d)));
    for w in &v[1..] {
        for (d, b) in c.0.iter_mut() {
            *b = b.or(w.get(d).copied().unwrap_or(Bytes::NONE));
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
    /// The escapes that hold only at the file's last byte (`A & M`, `M` that byte): `A`'s mask, tested there once,
    /// after the pass (the lean lexer's test of the string open at the end).
    end_sinks: Vec<(String, usize)>,
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
    /// `prefix`'s bytes (decision 0335).
    prefix: Option<Vec<u8>>,
    /// Simdjson's stage 1 (no `skip`, every mask in the pass): the mask whose bits are the records, written as
    /// indexes at its block (`bit_indexer`), the source padded (`PADDING`) and each block loaded where it is.
    index: Option<usize>,
    /// What stage 1 writes from each block's bits: the records' mask and each error ORed (`sinks`), where none holds
    /// at the padding's byte past the end, its masks need not be the file's bits (`lv`).
    outputs: Vec<Expr>,
    /// Each toggle's node and the mask it toggles on.
    toggles: HashMap<usize, Expr>,
    /// The byte-pair errors in simdjson's `utf8_checker` form (`bitlex_pair_lookup`), in simdjson's stage 1.
    lookup: Option<crate::bitlex_pair_lookup::Lookup>,
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
    /// `in_region_of`'s values, by the mask's id (`intern`), the region, its part and the bytes about it: a mask's
    /// parts and the shifts' values at each part are asked again and again, so without it the walk grows as the
    /// parts' count to the depth of the shifts. Forgotten with the ids (`forget_cubes`, `forget_name`).
    region_values: std::cell::RefCell<HashMap<(u32, Option<usize>, u8, [Bytes; 9]), Option<Expr>>>,
    opener_at: [Bytes; 4],
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
    /// The expressions `cubes` has read, interned: each a `Shape` of interned operands, by id.
    shapes: std::cell::RefCell<Vec<Shape>>,
    shape_ids: std::cell::RefCell<rustc_hash::FxHashMap<Shape, u32>>,
    /// `regional_id`'s answers by shape id: 0 not asked, 1 no, 2 yes.
    regional_ids: std::cell::RefCell<Vec<u8>>,
    /// `opener_id`'s and `def_id`'s answers.
    opener_ids: std::cell::RefCell<Vec<u32>>,
    def_ids: std::cell::RefCell<Vec<u32>>,
    /// `is_end_name`'s answers by name id, likewise.
    end_names: std::cell::RefCell<Vec<u8>>,
    /// The names in `shapes`, by id.
    name_strs: std::cell::RefCell<Vec<String>>,
    name_ids: std::cell::RefCell<rustc_hash::FxHashMap<String, u32>>,
    /// `cubes` of each shape, kept across its calls: each call made its own memo again, and
    /// Kotlin's escapes asked it of the same masks some millions of times. A name only if not
    /// `tangled` (or a class): its ways are the same whichever call asks, a cycle's placeholder
    /// met only as its own; any other shape only if made from no tangled name, and not in a
    /// name's definition (`cube_open`). Forgotten where a name it read changes (`forget_name`),
    /// or the regions (`forget_cubes`).
    kept: std::cell::RefCell<Vec<Option<std::rc::Rc<Cubes>>>>,
    /// The tangled names `cubes` has read (`tangled`): a value that read one is not kept.
    cube_tangled: std::cell::Cell<u64>,
    /// The names whose definitions `cubes` is in.
    cube_open: std::cell::Cell<u32>,
    /// The names (by id) `cubes` and `exact_bytes` have looked up (`forget_name`).
    cube_read: std::cell::RefCell<Vec<bool>>,
    /// `tangled`'s names by id, made once (emptied with `kept`).
    tangles: std::cell::RefCell<Option<Vec<bool>>>,
    /// `opener_bytes`' answers, by region and offset (emptied with `kept`).
    opener_cache: std::cell::RefCell<HashMap<(usize, i64), Bytes>>,
}

/// What a name means where a statement reads it: made of the regions or not, its local after stage 1,
/// its stage-1 mask. An error is written after every statement, so it reads its names as they were at it.
#[derive(Clone)]
struct Scope {
    regional: std::collections::HashSet<String>,
    names: HashMap<String, String>,
    node_of: HashMap<String, usize>,
}

/// The names `e` reads.
fn names_in(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Name(n) => out.push(n.clone()),
        Expr::Empty | Expr::Test { .. } => {}
        Expr::Not(a) => names_in(a, out),
        Expr::And(a, b) | Expr::Or(a, b) | Expr::Xor(a, b) => {
            names_in(a, out);
            names_in(b, out);
        }
        Expr::Call(_, args) => {
            for a in args {
                if let Arg::E(e) = a {
                    names_in(e, out);
                }
            }
        }
    }
}

/// The masks tier 2 finds a token's end by where tier 1 is simdjson's stage 1 (no `skip`), each a field of
/// `Masks`, by row: a run's (`run_K`), a pair's second byte (`pair_K`), a last byte's (`last_K`). A reader's are
/// `readers`'. One byte (`pair empty`, no rule) has none.
pub fn stage1_ends(stmts: &[Stmt]) -> Result<Vec<(String, Expr)>, String> {
    let mut out = Vec::new();
    for s in stmts {
        let Stmt::Token { kind, rule, last, .. } = s else { continue };
        let k = ident(kind);
        match (rule, last) {
            (Some(TokenRule::Run(r)), _) => out.push((format!("run_{k}"), r.clone())),
            (Some(TokenRule::Pair(Expr::Empty)), _) | (None, None) | (Some(TokenRule::ReadParts(..) | TokenRule::Inside(..)), _) => {}
            (Some(TokenRule::Pair(p)), _) => out.push((format!("pair_{k}"), p.clone())),
            (None, Some(l)) => out.push((format!("last_{k}"), l.clone())),
            (Some(other), _) => return Err(format!("tier 1 as Rust: `{kind}`'s rule {other:?} with no `skip` is not written yet")),
        }
    }
    Ok(out)
}

impl Gen<'_> {
    /// The names as they are here (`Scope`).
    fn scope(&self) -> Scope {
        Scope { regional: self.regional.clone(), names: self.names.clone(), node_of: self.node_of.clone() }
    }

    /// Whether name `n` means the same in `a` as now.
    fn same(&self, a: &Scope, n: &str) -> bool {
        let key = format!("={n}");
        a.regional.contains(n) == self.regional.contains(n) && a.names.get(n) == self.names.get(n) && a.node_of.get(&key) == self.node_of.get(&key)
    }

    /// `f` with the names as they were at `at`, where a name `e` reads means something else now (a
    /// class `high`, then a mask `high`): nothing made since of a name's new meaning is reused.
    fn in_scope<R>(&mut self, at: &Scope, e: &Expr, f: impl FnOnce(&mut Self) -> Result<R, String>) -> Result<R, String> {
        let mut read = Vec::new();
        names_in(e, &mut read);
        if read.iter().all(|n| self.same(at, n)) {
            return f(self);
        }
        let now = std::mem::replace(&mut self.regional, at.regional.clone());
        let names = std::mem::replace(&mut self.names, at.names.clone());
        let node_of = std::mem::replace(&mut self.node_of, at.node_of.clone());
        let temps = std::mem::take(&mut self.temps);
        let r = f(self);
        (self.regional, self.names, self.node_of, self.temps) = (now, names, node_of, temps);
        r
    }

    /// A local of `name`'s that no stage-1 mask and no other name has (a class `high_2` is `m_high_2`).
    fn fresh(&mut self, name: &str) -> String {
        let base = format!("m_{}", ident(name));
        let mut local = base.clone();
        while self.nodes.iter().any(|n| n.name == local) || self.names.values().any(|v| *v == local) {
            local = format!("{base}_{}", self.next);
            self.next += 1;
        }
        local
    }

    /// `name` is `local` from here; a name defined again leaves no part made of its old self to be read as its new.
    fn bind(&mut self, name: &str, local: String) {
        if self.names.insert(name.to_owned(), local).is_some() {
            self.temps.clear();
        }
    }

    /// The prefix's bytes out of block 0's bits (`prefix`, decision 0335): no class, `~` or token has them, and
    /// positions stay the file's. `lv` is a block's word (`word`) or the file's words.
    fn prefix_code(&self, word: bool, pad: &str) -> String {
        match &self.prefix {
            Some(b) => {
                let bytes = b.iter().map(|x| format!("{x:#04x}")).collect::<Vec<_>>().join(", ");
                let m = format!("!((1u64 << {}) - 1)", b.len());
                if word {
                    format!("{pad}// `prefix`: a file that starts with these bytes is read from after them.\n{pad}let lv = if q == 0 && src.starts_with(&[{bytes}]) {{ lv & {m} }} else {{ lv }};\n")
                } else {
                    format!("{pad}// `prefix`: a file that starts with these bytes is read from after them.\n{pad}if src.starts_with(&[{bytes}]) {{\n{pad}    lv[0] &= {m};\n{pad}}}\n")
                }
            }
            None => String::new(),
        }
    }

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
        self.nodes.push(Node { name, what, code, lag, kept: false, carry, class: None, also: Vec::new(), expr: None });
        self.node_of.insert(key, id);
        id
    }

    /// Class `name` of bytes `set`: one compare where it is one byte, else its nibble-table bits,
    /// each a product of high nibbles and low nibbles, in the first pair of tables with room.
    fn class(&mut self, name: &str, set: &[bool; 256]) -> usize {
        let code = class_code(&mut self.pairs, set);
        let local = format!("m_{}", ident(name));
        let id = self.push_node(format!("={name}"), local, format!("class {name}"), code, 0, false);
        self.nodes[id].class = Some(std::boxed::Box::new(*set));
        self.nodes[id].expr = Some(Expr::Name(name.to_owned()));
        id
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
            if !top
                && !matches!(e, Expr::Name(_))
                && let Some(&id) = g.node_of.get(&format!("{e:?}"))
            {
                return g.nodes[id].name.clone();
            }
            let inner = |a: &Expr| match a {
                Expr::And(..) | Expr::Or(..) | Expr::Xor(..) if !g.node_of.contains_key(&format!("{a:?}")) => format!("({})", go(g, a, false)),
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
        let id = self.make_node_of(key, name, what, e)?;
        self.nodes[id].expr = Some(e.clone());
        Ok(id)
    }

    fn make_node_of(&mut self, key: String, name: String, what: String, e: &Expr) -> Result<usize, String> {
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
                "toggle" => {
                    let a = self.node(ea(0)?)?;
                    let lag = self.nodes[a].lag;
                    let a = self.ring(a, "q");
                    // Each byte's parity of `a`'s bits up to it, the block before's carried (simdjson's `prefix_xor`).
                    let code = format!("{{\n        let mut x = {a};\n        x ^= x << 1;\n        x ^= x << 2;\n        x ^= x << 4;\n        x ^= x << 8;\n        x ^= x << 16;\n        x ^= x << 32;\n        let x = x ^ 0u64.wrapping_sub(c_{name});\n        c_{name} = x >> 63;\n        x & lv\n    }}");
                    let operand = ea(0)?.clone();
                    let id = self.push_node(key, name, what, code, lag, true);
                    self.toggles.insert(id, operand);
                    return Ok(id);
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
            Expr::Call(f, _) if matches!(f.as_str(), "fill" | "escaped" | "text" | "toggle") => {
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
        let p = self.stage1_parts(self.index.is_some());
        format!("{}{}{}{}{}    let mut refused = false;\n    for t in 0..{} {{\n{}    }}\n{}{}", p.tables, p.record, p.indexes, p.header, p.decls, p.steps, p.body, p.finals, p.ret)
    }

    /// Stage 1's code in its parts (`Stage1Text`): `stage1` writes them as a function of its own, and the one pass
    /// (decision 0619) writes its block's code where the token loop needs the block. `padded`: the source is padded,
    /// each block loaded where it is.
    /// The masks stage 1 writes (`live`), the nibble tables of the classes among them (`pairs`), and each mask's code
    /// (a class's in those tables).
    fn stage1_codes(&self) -> (Vec<bool>, Vec<Vec<Rect>>, Vec<String>) {
        // The masks stage 1 writes: those an escape, the records or a later stage reads (tier 2's words, `also`, among
        // them), and their operands.
        let mut live: Vec<bool> = self.nodes.iter().enumerate().map(|(i, n)| n.kept || !n.also.is_empty() || self.index == Some(i) || self.sinks.iter().chain(&self.end_sinks).any(|(_, s)| *s == i)).collect();
        for i in (0..self.nodes.len()).rev() {
            if live[i] {
                let code = &self.nodes[i].code;
                for (l, m) in live[..i].iter_mut().zip(&self.nodes) {
                    if code.contains(&format!("r_{}[", m.name)) {
                        *l = true;
                    }
                }
            }
        }
        // The classes' tables, of the classes it writes alone.
        let mut pairs: Vec<Vec<Rect>> = Vec::new();
        let codes: Vec<String> = self
            .nodes
            .iter()
            .zip(&live)
            .map(|(n, &l)| match &n.class {
                Some(set) if l => class_code(&mut pairs, set),
                _ => n.code.clone(),
            })
            .collect();
        (live, pairs, codes)
    }

    fn stage1_parts(&self, padded: bool) -> Stage1Text {
        let mut out = String::new();
        let (live, pairs, codes) = self.stage1_codes();
        for (p, rects) in pairs.iter().enumerate() {
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
        if let Some(l) = &self.lookup {
            let list = |t: &[u8; 16]| t.iter().map(|b| format!("{b:#04x}")).collect::<Vec<_>>().join(", ");
            let whys: Vec<String> = l.covers.iter().map(|(_, w)| format!("`{w}`")).collect();
            let _ = writeln!(
                out,
                "/// The byte-pair errors ({}) as simdjson's `check_special_cases`: each rule a bit of the byte before's high\n/// nibble's table, its low nibble's, and this byte's high nibble's.\nstatic BYTE_1_HIGH: [u8; 16] = [{}];\nstatic BYTE_1_LOW: [u8; 16] = [{}];\nstatic BYTE_2_HIGH: [u8; 16] = [{}];\n/// `is_incomplete`'s `max_value`: past it, a block's last bytes leave a sequence open (255: never).\nstatic MAX_VALUE: [u8; 64] = [{}];\n",
                whys.join(", "),
                list(&l.byte_1_high),
                list(&l.byte_1_low),
                list(&l.byte_2_high),
                (0..64).map(|i| format!("{:#04x}", if i >= 61 { l.open[63 - i].map_or(0xff, |t| t - 1) } else { 0xff })).collect::<Vec<_>>().join(", ")
            );
        }
        let kept: Vec<&Node> = self.nodes.iter().filter(|n| n.kept).collect();
        let tables = std::mem::take(&mut out);
        out.push_str("/// Stage 1's masks a later stage reads, a bit a byte, and whether an escape holds.\nstruct Stage1 {\n");
        for n in &kept {
            let _ = writeln!(out, "    {}: Vec<u64>,", n.name);
        }
        if self.index.is_some() {
            out.push_str("    /// The records (simdjson's structural indexes), then `n`, `n` and 0; and their count.\n    starts: Vec<u32>,\n    count: usize,\n");
        }
        for a in self.nodes.iter().flat_map(|n| &n.also) {
            let _ = writeln!(out, "    {a}: Vec<u64>,");
        }
        out.push_str("    refused: bool,\n}\n\n");
        let record = std::mem::take(&mut out);
        let most = self.nodes.iter().zip(&live).filter(|(_, l)| **l).map(|(n, _)| n.lag).max().unwrap_or(0);
        // Each mask's words where its readers read them, by age (the blocks between its write and the read): this
        // block's a local (`v_`), the block before's another (`p_`), older ones a ring. A reader two stages or more
        // behind its operand reads a ring too: past the file's last block, the operand's stage stops first.
        let by_name: HashMap<&str, usize> = self.nodes.iter().enumerate().map(|(i, n)| (n.name.as_str(), i)).collect();
        // The block before's word: a local of its stage where only that stage reads it.
        let (mut needs_p, mut p_outer, mut needs_ring) = (vec![false; self.nodes.len()], vec![false; self.nodes.len()], vec![false; self.nodes.len()]);
        for i in (0..self.nodes.len()).filter(|&i| live[i]) {
            for (x, off) in ring_reads(&codes[i], &by_name) {
                match age(self.nodes[i].lag, self.nodes[x].lag, off) {
                    Some(0) => {}
                    Some(1) => {
                        needs_p[x] = true;
                        p_outer[x] |= self.nodes[i].lag != self.nodes[x].lag;
                    }
                    _ => needs_ring[x] = true,
                }
            }
        }
        let rings = needs_ring.iter().any(|r| *r);
        // No mask holds the file's bits (`lv`) where each block is the file's or the padding (no stage behind, the
        // source padded) and no output holds at the padding's byte, whatever the masks carried hold: a bit past the
        // end reaches no byte before it (shifts, toggles and carries go up), and no block follows the last. The
        // `prefix` is then skipped as simdjson skips a byte order mark: the blocks start after it.
        let bare = most == 0 && self.index.is_some() && self.outputs.iter().all(|e| false_at(e, b' ', self.x.stmts));
        // An end test of a toggle whose mask holds at no padding byte: its carry out of the last block is its value at
        // the file's last byte (simdjson's `finish`, `prev_in_string != 0`).
        let carried = |i: usize| bare && self.toggles.get(&i).is_some_and(|op| false_at(op, b' ', self.x.stmts));
        let mut outer: Vec<bool> = (0..self.nodes.len()).map(|i| needs_p[i] || (most == 0 && !carried(i) && self.end_sinks.iter().any(|(_, s)| *s == i))).collect();
        for i in (0..self.nodes.len()).filter(|&i| live[i]) {
            for (x, off) in ring_reads(&codes[i], &by_name) {
                if self.nodes[i].lag != self.nodes[x].lag && age(self.nodes[i].lag, self.nodes[x].lag, off).is_some() {
                    outer[x] = true;
                }
            }
        }
        let ring = (most as usize + 2).next_power_of_two();
        if most == 0 && self.index.is_some() {
            // simdjson's `bit_indexer::write`: four, then two at a time while the count goes past them, each test in
            // the one before, to 24, then one at a time.
            let mut stepped = String::new();
            for i in (4..24).step_by(2).rev() {
                let inner = std::mem::take(&mut stepped);
                stepped = format!("one(*count + {i});\none(*count + {});\n{}", i + 1, if inner.is_empty() { String::new() } else { format!("if {} < cnt {{\n{}}}\n", i + 2, indent(&inner, "    ")) });
            }
            let _ = write!(out, "/// Block bits `bits` from `base` as indexes at `si[*count..]` (simdjson's `bit_indexer::write`).\n///\n/// # Safety\n/// `si` has room for `*count` and 64 more.\n#[inline(always)]\nunsafe fn write_indexes(si: *mut u32, count: &mut usize, base: u32, mut bits: u64) {{\n    if bits == 0 {{\n        return;\n    }}\n    let cnt = bits.count_ones() as usize;\n    // SAFETY: the caller's: each write within `*count + 64`.\n    let mut one = |k: usize| unsafe {{\n        *si.add(k) = base.wrapping_add(bits.trailing_zeros());\n        bits &= bits.wrapping_sub(1);\n    }};\n    for k in 0..4 {{\n        one(*count + k);\n    }}\n    if 4 < cnt {{\n{}    }}\n    if 24 < cnt {{\n        for k in 24..cnt {{\n            one(*count + k);\n        }}\n    }}\n    *count += cnt;\n}}\n\n", indent(&stepped, "        "));
        }
        let off = |q: &str| if bare && self.prefix.is_some() { format!("off + 64 * {q}") } else { format!("64 * {q}") };
        let indexes = std::mem::take(&mut out);
        let _ = writeln!(
            out,
            "/// Stage 1: one pass over the blocks, 64 bytes a step, as the lean lexer's `Blocks::of`: the classes of block `t`{}, each mask a local of its block{}.\n#[inline(always)]\nunsafe fn stage1<S: rerail_simd8::Simd8x64>({}) -> Stage1 {{\n{}    let nw = n.div_ceil(64);",
            if most > 0 { ", each mask of block `t - lag` (a `next` reads the block after its operand's)" } else { "" },
            if rings { format!(", the block before's where it is read, its last {ring} words in a ring where older ones are") } else { " and the block before's where it is read".to_owned() },
            if self.index.is_some() { "src: &[u8], n: usize" } else { "src: &[u8]" },
            if self.index.is_some() { "" } else { "    let n = src.len();\n" }
        );
        if bare && let Some(b) = &self.prefix {
            let bytes = b.iter().map(|x| format!("{x:#04x}")).collect::<Vec<_>>().join(", ");
            out.truncate(out.len() - "    let nw = n.div_ceil(64);\n".len());
            let _ = writeln!(out, "    // `prefix`: a file that starts with these bytes is read from after them, its blocks from there (as simdjson\n    // reads past a byte order mark).\n    let off = if src.starts_with(&[{bytes}]) {{ {} }} else {{ 0 }};\n    let nw = (n - off).div_ceil(64);", b.len());
        }
        let header = std::mem::take(&mut out);
        if rings {
            let _ = writeln!(out, "    const RING: usize = {ring};");
        }
        if self.index.is_some() {
            // Room for a record a byte, and the slack of a block's unconditional writes.
            out.push_str("    let mut starts: Vec<u32> = Vec::with_capacity(n + 64 + 3);\n    let si = starts.as_mut_ptr();\n    let mut count = 0usize;\n");
        }
        for (i, n) in self.nodes.iter().enumerate().filter(|(i, _)| live[*i]) {
            if outer[i] {
                let _ = writeln!(out, "    let mut v_{} = 0u64;", n.name);
            }
            if p_outer[i] {
                let _ = writeln!(out, "    let mut p_{} = 0u64;", n.name);
            }
            if needs_ring[i] {
                let _ = writeln!(out, "    let mut r_{} = [0u64; RING];", n.name);
            }
            if n.carry {
                let _ = writeln!(out, "    let mut c_{} = 0u64;", n.name);
            }
        }
        if most > 0 {
            for (_, id) in &self.end_sinks {
                let _ = writeln!(out, "    let mut e_{} = 0u64;", self.nodes[*id].name);
            }
        }
        for n in &kept {
            let _ = writeln!(out, "    let mut {} = vec![0u64; nw];", n.name);
        }
        for a in self.nodes.iter().flat_map(|n| &n.also) {
            let _ = writeln!(out, "    let mut {a} = vec![0u64; nw];");
        }
        if self.lookup.is_some() {
            out.push_str("    // The byte-pair errors' check, simdjson's `utf8_checker`: the block before, the errors, and whether the block before left\n    // a sequence open.\n    let mut prev_input = unsafe { S::splat(0) };\n    let mut error = unsafe { S::splat(0) };\n    let mut prev_incomplete = unsafe { S::splat(0) };\n    let max_value = unsafe { S::load(&MAX_VALUE) };\n");
        }
        // The file's bits of each block (`lv`), once a block: a stage behind takes the block before's.
        if let Some(b) = &self.prefix
            && !bare
        {
            let bytes = b.iter().map(|x| format!("{x:#04x}")).collect::<Vec<_>>().join(", ");
            let _ = writeln!(out, "    // `prefix`: a file that starts with these bytes is read from after them.\n    let prefix = src.starts_with(&[{bytes}]);");
        }
        let lv_of = |q: &str, past: bool| {
            let mut c = if past { format!("if 64 * {q} + 64 <= n {{ !0u64 }} else if {q} < nw {{ (1u64 << (n % 64)) - 1 }} else {{ 0 }}") } else { format!("if 64 * {q} + 64 <= n {{ !0u64 }} else {{ (1u64 << (n % 64)) - 1 }}") };
            if let Some(b) = &self.prefix {
                c = format!("{{\n    let lv = {c};\n    if {q} == 0 && prefix {{ lv & !((1u64 << {}) - 1) }} else {{ lv }}\n}}", b.len());
            }
            c
        };
        if most > 0 {
            out.push_str("    let mut lv_1 = 0u64;\n");
        }
        if !self.sinks.is_empty() {
            out.push_str("    // The errors' bits in the blocks so far, ORed.\n    let mut errors = 0u64;\n");
        }
        if most == 0 && self.index.is_some() {
            out.push_str("    // The block before's records, written at the next block (simdjson's `prev_structurals`).\n    let mut prev_structurals = 0u64;\n");
        }
        let decls = std::mem::take(&mut out);
        let steps = if most == 0 { "nw".to_owned() } else { format!("nw + {most}") };
        if !bare {
            let _ = writeln!(out, "        // The block's bits inside the file (none past its end).\n        let lv_0 = {};", indent(&lv_of("t", most > 0), "        ").trim());
        }
        for lag in 0..=most {
            let ids: Vec<usize> = (0..self.nodes.len()).filter(|&i| live[i] && self.nodes[i].lag == lag).collect();
            if ids.is_empty() {
                continue;
            }
            let _ = writeln!(out, "        // {} behind the classes.", if lag == 0 { "The classes, and the masks no blocks".to_owned() } else { format!("The masks {lag} block{}", if lag == 1 { "" } else { "s" }) });
            // With no stage behind, every block is the file's: no guard.
            let (guard, lv) = match lag {
                0 if most == 0 => ("{\n            let q = t;".to_owned(), "lv_0".to_owned()),
                0 => ("if t <= nw {\n            let q = t;".to_owned(), "lv_0".to_owned()),
                1 => ("if t >= 1 && t - 1 <= nw {\n            let q = t - 1;".to_owned(), "lv_1".to_owned()),
                _ => (format!("if t >= {lag} && t - {lag} <= nw {{\n            let q = t - {lag};"), lv_of("q", true)),
            };
            let _ = writeln!(out, "        {guard}");
            if !bare {
                let _ = writeln!(out, "            let lv = {lv};");
            }
            if lag == 0 {
                if padded {
                    // The source padded: each block of the file loaded where it is (`PADDING` past its end).
                    if most == 0 {
                        let _ = writeln!(out, "            let b: &[u8; 64] = src[{0}..{0} + 64].try_into().expect(\"a padded block\");\n            let v = unsafe {{ S::load(b) }};", off("q"));
                    } else {
                        let _ = writeln!(out, "            const NONE: [u8; 64] = [0u8; 64];\n            let b: &[u8; 64] = if q < nw {{ src[64 * q..64 * q + 64].try_into().expect(\"a padded block\") }} else {{ &NONE }};\n            let v = unsafe {{ S::load(b) }};");
                    }
                } else {
                    let _ = writeln!(out, "            let b = if q < nw {{ block(src, q) }} else {{ [0u8; 64] }};\n            let v = unsafe {{ S::load(&b) }};");
                }
                for p in 0..pairs.len() {
                    let _ = writeln!(out, "            let c{p} = unsafe {{ S::classify(v, &LO_{p}, &HI_{p}) }};");
                }
                if let Some(l) = &self.lookup {
                    out.push_str(&lookup_code(l));
                }
            }
            for i in ids {
                let n = &self.nodes[i];
                let code = local_reads(&codes[i], &by_name, &self.nodes, n.lag).replace(" & lv & lv", " & lv");
                let code = if bare { without_lv(&code) } else { code };
                let code = indent(&code, "            ");
                let code = code.trim();
                let unsafe_ = if codes[i].contains("S::") { format!("unsafe {{ {code} }}") } else { code.to_owned() };
                // The block's word: a local of the block's code, or of the pass where it is read past it.
                let w = format!("v_{}", n.name);
                if needs_p[i] {
                    let _ = writeln!(out, "            {}p_{1} = v_{1};", if p_outer[i] { "" } else { "let " }, n.name);
                }
                let _ = writeln!(out, "            // {}\n            {}{w} = {unsafe_};", n.what, if outer[i] { "" } else { "let " });
                if needs_ring[i] {
                    let _ = writeln!(out, "            r_{}[q % RING] = {w};", n.name);
                }
                let stores: Vec<String> = n.kept.then(|| n.name.clone()).into_iter().chain(n.also.iter().cloned()).map(|v| format!("{v}[q] = {w};")).collect();
                if !stores.is_empty() {
                    let _ = writeln!(out, "            if q < nw {{\n                {}\n            }}", stores.join("\n                "));
                }
                // An error's bits, ORed into one word, tested after the pass (`index`'s `unescaped_chars_error`).
                for (why, _) in self.sinks.iter().filter(|(_, s)| *s == i) {
                    if most == 0 {
                        let _ = writeln!(out, "            // {why}\n            errors |= {w};");
                    } else {
                        let _ = writeln!(out, "            // {why}\n            if q < nw {{\n                errors |= {w};\n            }}");
                    }
                }
                if self.index == Some(i) {
                    // simdjson's `bit_indexer::write`: four indexes, four more where there are, then one at a time; each
                    // write past the count is written over by the next block's, or past the end.
                    let write = "let (base, cnt, mut bits) = ({B}, {W}.count_ones() as usize, {W});\n// SAFETY: `count + cnt` is at most `n`; the room is `n + 64 + 3`.\nunsafe {\n    for k in 0..4 {\n        *si.add(count + k) = base + bits.trailing_zeros();\n        bits &= bits.wrapping_sub(1);\n    }\n    if cnt > 4 {\n        for k in 4..8 {\n            *si.add(count + k) = base + bits.trailing_zeros();\n            bits &= bits.wrapping_sub(1);\n        }\n    }\n    let mut k = 8;\n    while k < cnt {\n        *si.add(count + k) = base + bits.trailing_zeros();\n        bits &= bits.wrapping_sub(1);\n        k += 1;\n    }\n}\ncount += cnt;\n";
                    let write = write.replace("{W}", &w).replace("{B}", &if bare && self.prefix.is_some() { "(off + 64 * q) as u32".to_owned() } else { "64 * q as u32".to_owned() });
                    let write = write.as_str();
                    if most == 0 {
                        // As `index`: the block before's records written at this block (`indexer.write(idx - 64,
                        // prev_structurals)`), the last block's after the pass, through one `write`.
                        let base = if bare && self.prefix.is_some() { "((off + 64 * q) as u32).wrapping_sub(64)" } else { "(64 * q as u32).wrapping_sub(64)" };
                        let _ = writeln!(out, "            // The block before's records (simdjson's `indexer.write(idx - 64, prev_structurals)`).\n            unsafe {{ write_indexes(si, &mut count, {base}, prev_structurals) }};\n            prev_structurals = {w};");
                    } else {
                        out.push_str("            // The records: the block's bits as indexes (simdjson's `bit_indexer::write`).\n");
                        let _ = write!(out, "            if q < nw {{\n{}            }}\n", indent(write, "                "));
                    }
                }
                // An end test's mask, its file's last block's word (the pass ends at it with no stage behind).
                if most > 0 && self.end_sinks.iter().any(|(_, s)| *s == i) {
                    let _ = writeln!(out, "            if q + 1 == nw {{\n                e_{} = {w};\n            }}", n.name);
                }
            }
            out.push_str("        }\n");
        }
        if most > 0 {
            out.push_str("        lv_1 = lv_0;\n");
        }
        let body = std::mem::take(&mut out);
        if most == 0 && self.index.is_some() {
            let base = if bare && self.prefix.is_some() { "((off + 64 * nw) as u32).wrapping_sub(64)" } else { "(64 * nw as u32).wrapping_sub(64)" };
            let _ = writeln!(out, "    // The last block's records (simdjson's `finish`).\n    unsafe {{ write_indexes(si, &mut count, {base}, prev_structurals) }};");
        }
        if !self.sinks.is_empty() {
            out.push_str("    // The errors, once.\n    refused |= errors != 0;\n");
        }
        // The tests at the file's last byte, once.
        for (why, id) in &self.end_sinks {
            let word = if most == 0 { format!("v_{}", self.nodes[*id].name) } else { format!("e_{}", self.nodes[*id].name) };
            if carried(*id) {
                let _ = writeln!(out, "    // {why}: the toggle carried out of the file's last block, once (simdjson's `finish`).\n    refused |= c_{} != 0;", self.nodes[*id].name);
                continue;
            }
            if bare && self.prefix.is_some() {
                let _ = writeln!(out, "    // {why}: at the file's last byte, once (its block's word, the pass's last).\n    refused |= n > off && {word} >> ((n - 1 - off) % 64) & 1 != 0;");
                continue;
            }
            let inside = match &self.prefix {
                Some(b) => format!(" && !(prefix && n <= {})", b.len()),
                None => String::new(),
            };
            let _ = writeln!(out, "    // {why}: at the file's last byte, once (its block's word, the pass's last).\n    refused |= n > 0{inside} && {word} >> ((n - 1) % 64) & 1 != 0;");
        }
        if self.lookup.is_some() {
            // The block past the file's end is zeros: an ASCII block, so the same test as there.
            out.push_str("    // The byte-pair errors (simdjson's `errors()`), and a sequence the file's last block leaves open (`check_eof`).\n    refused |= unsafe { S::nonzero(S::or(error, prev_incomplete)) } != 0;\n");
        }
        if self.index.is_some() {
            // simdjson's: the index ends in `n`, `n` and 0, so that stage 2 reads past its last record unchecked.
            out.push_str("    // SAFETY: `count` records written, and room for three more.\n    unsafe {\n        *si.add(count) = n as u32;\n        *si.add(count + 1) = n as u32;\n        *si.add(count + 2) = 0;\n        starts.set_len(count + 3);\n    }\n");
        }
        let finals = std::mem::take(&mut out);
        out.push_str("    Stage1 {\n");
        for n in &kept {
            let _ = writeln!(out, "        {},", n.name);
        }
        for a in self.nodes.iter().flat_map(|n| &n.also) {
            let _ = writeln!(out, "        {a},");
        }
        if self.index.is_some() {
            out.push_str("        starts,\n        count,\n");
        }
        let _ = write!(out, "        refused,\n    }}\n}}\n\nrerail_simd8::dispatch! {{\n    fn stage1_best({}) -> Stage1 = stage1, on stage1_on;\n}}\n\n", if self.index.is_some() { "src: &[u8], n: usize" } else { "src: &[u8]" });
        Stage1Text { tables, record, indexes, header, decls, steps, most, body, finals, ret: out }
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
                "toggle" => {
                    let a = self.val(ea(0)?)?;
                    let _ = writeln!(self.body, "    let {name} = toggle(&{a});");
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
        // An assumption (decision 0607): the mask at a start is its expression.
        if let Expr::Name(n) = e
            && let Some((_, _, a, _)) = self.x.assumed.iter().find(|(sc, m, _, _)| sc.is_none() && m == n)
        {
            if self.regional(a) {
                return Err(format!("tier 1 as Rust: `assume … {n} = …` reads a region's mask"));
            }
            return self.at_start(a);
        }
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
                if let Some(k) = self.opens.as_ref().and_then(|opens| opens.iter().position(|(kind, _, _)| *n == format!("o_{kind}") || *n == format!("f_{kind}"))) {
                    Some(format!("open == {k}"))
                } else if self.kinds.iter().any(|r| *n == format!("o_{}", r.kind) || *n == format!("f_{}", r.kind) || r.empty.as_ref().is_some_and(|(e, _)| e == n)) {
                    // A region's mask where the loop numbers no regions (`opens`: a region opens no token of its own
                    // there): what the grammar states of it (`assume … at starts`), else unknown.
                    None
                } else if let Some(d) = self.defs.get(n).cloned() {
                    self.at_start(&d)?
                } else {
                    // The hidden bytes: none at a start.
                    Some("false".to_owned())
                }
            }
            Expr::Empty => Some("false".to_owned()),
            Expr::Not(a) => {
                let n = self.opens.as_ref().map_or(0, Vec::len);
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
    /// The regions changed: every kept value of `cubes` is forgotten.
    fn forget_cubes(&self) {
        self.region_values.borrow_mut().clear();
        self.kept.borrow_mut().clear();
        self.regional_ids.borrow_mut().clear();
        self.end_names.borrow_mut().clear();
        self.def_ids.borrow_mut().clear();
        self.opener_ids.borrow_mut().clear();
        self.opener_cache.borrow_mut().clear();
        *self.tangles.borrow_mut() = None;
    }

    /// Name `n` was defined, made a class or made regional. The kept values read names only as
    /// `cubes` and `exact_bytes` look them up, so a name neither has looked up, and that no
    /// definition reads, changes none of them; else they are forgotten. The cycles are made again.
    fn forget_name(&self, n: &str) {
        self.region_values.borrow_mut().clear();
        *self.tangles.borrow_mut() = None;
        self.regional_ids.borrow_mut().clear();
        self.end_names.borrow_mut().clear();
        self.def_ids.borrow_mut().clear();
        self.opener_ids.borrow_mut().clear();
        let id = self.name_ids.borrow().get(n).copied();
        let read = id.is_some_and(|id| self.cube_read.borrow().get(id as usize).copied().unwrap_or(false)) || self.defs.iter().any(|(m, d)| m != n && mentions(d, n));
        if read {
            self.forget_cubes();
        }
    }

    /// Whether `cubes` of name `n` may depend on where it is asked from: `n` reaches a cycle of
    /// two names or more through the definitions. `cubes` reads a name being read as anything (its
    /// placeholder), so in such a cycle a name's ways depend on which of the cycle's names was
    /// read first. A name that reads only itself (a carry) meets its placeholder at the same
    /// place wherever it is asked from, and is not tangled.
    fn tangled(&self, n: u32) -> bool {
        if self.tangles.borrow().is_none() {
            let names = self.make_tangles();
            let ids: Vec<u32> = names.iter().map(|m| self.name_id(m)).collect();
            let mut t = vec![false; self.name_strs.borrow().len()];
            ids.iter().for_each(|&i| t[i as usize] = true);
            *self.tangles.borrow_mut() = Some(t);
        }
        // A name not interned when they were made is in no definition, and no cycle.
        self.tangles.borrow().as_ref().is_some_and(|t| t.get(n as usize).copied().unwrap_or(false))
    }

    /// `tangled`'s names: the strongly connected components of the names' reads.
    fn make_tangles(&self) -> std::collections::HashSet<String> {
        fn names_in(e: &Expr, out: &mut Vec<String>) {
            match e {
                Expr::Name(n) => out.push(n.clone()),
                Expr::Empty => {}
                Expr::Not(a) => names_in(a, out),
                Expr::And(a, b) | Expr::Or(a, b) | Expr::Xor(a, b) => {
                    names_in(a, out);
                    names_in(b, out);
                }
                Expr::Call(_, args) => args.iter().for_each(|a| if let Arg::E(x) = a { names_in(x, out) }),
                Expr::Test { arg, .. } => arg.iter().for_each(|a| names_in(a, out)),
            }
        }
        // Each name's reads, as `cubes` follows them: a definition, or a region's opener.
        let mut reads: HashMap<String, Vec<String>> = HashMap::new();
        for (n, d) in &self.defs {
            let mut v = Vec::new();
            names_in(d, &mut v);
            reads.insert(n.clone(), v);
        }
        for r in &self.kinds {
            let mut v = Vec::new();
            names_in(&r.opener, &mut v);
            reads.insert(format!("o_{}", r.kind), v);
        }
        // The strongly connected components (Tarjan's), iteratively.
        let names: Vec<String> = reads.keys().cloned().collect();
        let index_of: HashMap<&str, usize> = names.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
        let edges: Vec<Vec<usize>> = names.iter().map(|n| reads[n].iter().filter_map(|m| index_of.get(m.as_str()).copied()).collect()).collect();
        let n = names.len();
        let (mut index, mut low, mut on) = (vec![usize::MAX; n], vec![0; n], vec![false; n]);
        let (mut stack, mut next, mut comp) = (Vec::new(), 0usize, vec![usize::MAX; n]);
        let mut comps = 0;
        for root in 0..n {
            if index[root] != usize::MAX {
                continue;
            }
            let mut work: Vec<(usize, usize)> = vec![(root, 0)];
            while let Some(&(v, i)) = work.last() {
                if i == 0 && index[v] == usize::MAX {
                    index[v] = next;
                    low[v] = next;
                    next += 1;
                    stack.push(v);
                    on[v] = true;
                }
                if i < edges[v].len() {
                    work.last_mut().unwrap().1 += 1;
                    let w = edges[v][i];
                    if index[w] == usize::MAX {
                        work.push((w, 0));
                    } else if on[w] {
                        low[v] = low[v].min(index[w]);
                    }
                    continue;
                }
                work.pop();
                if let Some(&(u, _)) = work.last() {
                    low[u] = low[u].min(low[v]);
                }
                if low[v] == index[v] {
                    while let Some(w) = stack.pop() {
                        on[w] = false;
                        comp[w] = comps;
                        if w == v {
                            break;
                        }
                    }
                    comps += 1;
                }
            }
        }
        let mut size = vec![0usize; comps];
        comp.iter().for_each(|&c| size[c] += 1);
        // The names in a cycle of two or more, and every name that reaches one.
        let mut tangled: Vec<bool> = (0..n).map(|v| size[comp[v]] > 1).collect();
        let mut changed = true;
        while changed {
            changed = false;
            for v in 0..n {
                if !tangled[v] && edges[v].iter().any(|&w| tangled[w]) {
                    tangled[v] = true;
                    changed = true;
                }
            }
        }
        (0..n).filter(|&v| tangled[v]).map(|v| names[v].clone()).collect()
    }

    /// The bytes region `k`'s opener may be at `d` bytes on from its first (`bytes_at`, with a
    /// memo of its own), kept: `shift_before`'s `near` asks it for each way and offset a mask may
    /// hold, and Kotlin's escapes some millions of times.
    fn opener_bytes(&self, k: usize, d: i64) -> Bytes {
        if let Some(b) = self.opener_cache.borrow().get(&(k, d)) {
            return *b;
        }
        let b = self.bytes_at(&self.kinds[k].opener, d, &mut HashMap::new());
        self.opener_cache.borrow_mut().insert((k, d), b);
        b
    }

    /// `cubes` or `exact_bytes` looked up name `n` (`forget_name`).
    fn note_read(&self, n: u32) {
        let mut r = self.cube_read.borrow_mut();
        if r.len() <= n as usize {
            r.resize(n as usize + 1, false);
        }
        r[n as usize] = true;
    }

    /// Name `n`'s id in `shapes`.
    fn name_id(&self, n: &str) -> u32 {
        if let Some(&i) = self.name_ids.borrow().get(n) {
            return i;
        }
        let i = self.name_strs.borrow().len() as u32;
        self.name_strs.borrow_mut().push(n.to_owned());
        self.name_ids.borrow_mut().insert(n.to_owned(), i);
        i
    }

    fn shape_id(&self, s: Shape) -> u32 {
        if let Some(&i) = self.shape_ids.borrow().get(&s) {
            return i;
        }
        let i = self.shapes.borrow().len() as u32;
        self.shapes.borrow_mut().push(s.clone());
        self.shape_ids.borrow_mut().insert(s, i);
        i
    }

    /// `e`'s id: its shape's, its operands interned first.
    fn intern(&self, e: &Expr) -> u32 {
        let s = match e {
            Expr::Empty => Shape::Empty,
            Expr::Name(n) => Shape::Name(self.name_id(n)),
            Expr::Not(a) => Shape::Not(self.intern(a)),
            Expr::And(a, b) => Shape::And(self.intern(a), self.intern(b)),
            Expr::Or(a, b) => Shape::Or(self.intern(a), self.intern(b)),
            Expr::Xor(a, b) => Shape::Xor(self.intern(a), self.intern(b)),
            Expr::Call(f, args) => Shape::Call(
                self.name_id(f),
                args.iter()
                    .map(|a| match a {
                        Arg::E(x) => ArgShape::E(self.intern(x)),
                        Arg::N(k) => ArgShape::N(*k),
                        Arg::S(t) => ArgShape::S(t.as_slice().into()),
                    })
                    .collect(),
            ),
            Expr::Test { .. } => Shape::Test(std::rc::Rc::new(e.clone())),
        };
        self.shape_id(s)
    }

    /// Region `k`'s opener, interned (emptied with `kept`).
    fn opener_id(&self, k: usize) -> u32 {
        if let Some(&i) = self.opener_ids.borrow().get(k) {
            return i;
        }
        let ids: Vec<u32> = self.kinds.iter().map(|r| self.intern(&r.opener)).collect();
        let i = ids[k];
        *self.opener_ids.borrow_mut() = ids;
        i
    }

    /// Name id `n`'s definition `d`, interned (emptied wherever a name changes).
    fn def_id(&self, n: u32, d: &Expr) -> u32 {
        if let Some(&i) = self.def_ids.borrow().get(n as usize)
            && i != u32::MAX
        {
            return i;
        }
        let i = self.intern(d);
        let mut v = self.def_ids.borrow_mut();
        if v.len() <= n as usize {
            v.resize(n as usize + 1, u32::MAX);
        }
        v[n as usize] = i;
        i
    }

    /// `is_end_name` of name `name` (id `n`), kept (emptied wherever a name changes).
    fn is_end_id(&self, n: u32, name: &str) -> bool {
        if let Some(&r) = self.end_names.borrow().get(n as usize)
            && r != 0
        {
            return r == 2;
        }
        let r = self.is_end_name(name);
        let mut v = self.end_names.borrow_mut();
        if v.len() <= n as usize {
            v.resize(n as usize + 1, 0);
        }
        v[n as usize] = if r { 2 } else { 1 };
        r
    }

    /// Shape `id`.
    fn shape(&self, id: u32) -> Shape {
        self.shapes.borrow()[id as usize].clone()
    }

    /// Whether name id `f` is `name`.
    fn is_fn(&self, f: u32, name: &str) -> bool {
        self.name_strs.borrow()[f as usize] == name
    }

    /// `prev`'s or `next`'s operand and its count (`prev` negative), where call `f` of `args` is
    /// one: `exact`, the count a number or none.
    fn shift_of(&self, f: u32, args: &[ArgShape]) -> Option<(u32, i64, bool)> {
        let next = self.is_fn(f, "next");
        if !next && !self.is_fn(f, "prev") {
            return None;
        }
        let ArgShape::E(a) = args.first()? else { return None };
        let (k, exact) = match args.get(1) {
            None => (1, true),
            Some(ArgShape::N(k)) => (*k as i64, true),
            _ => (1, false),
        };
        Some((*a, if next { k } else { -k }, exact))
    }

    /// `regional` of interned expression `id`, kept (emptied wherever a name changes).
    fn regional_id(&self, id: u32) -> bool {
        if let Some(&r) = self.regional_ids.borrow().get(id as usize)
            && r != 0
        {
            return r == 2;
        }
        let r = match self.shape(id) {
            Shape::Name(n) => self.regional.contains(&self.name_strs.borrow()[n as usize]),
            Shape::Empty | Shape::Test(_) => false,
            Shape::Not(a) => self.regional_id(a),
            Shape::And(a, b) | Shape::Or(a, b) | Shape::Xor(a, b) => self.regional_id(a) || self.regional_id(b),
            Shape::Call(_, args) => args.iter().any(|a| matches!(a, ArgShape::E(x) if self.regional_id(*x))),
        };
        let mut v = self.regional_ids.borrow_mut();
        if v.len() <= id as usize {
            v.resize(id as usize + 1, 0);
        }
        v[id as usize] = if r { 2 } else { 1 };
        r
    }

    fn kept_of(&self, id: u32) -> Option<std::rc::Rc<Cubes>> {
        self.kept.borrow().get(id as usize).and_then(|c| c.clone())
    }

    fn keep(&self, id: u32, c: &std::rc::Rc<Cubes>) {
        let mut k = self.kept.borrow_mut();
        if k.len() <= id as usize {
            k.resize(id as usize + 1, None);
        }
        k[id as usize] = Some(c.clone());
    }

    fn exact_bytes(&self, e: &Expr) -> Option<Bytes> {
        self.exact_of(self.intern(e))
    }

    /// The bytes interned expression `id` holds at, where they are all it reads.
    fn exact_of(&self, id: u32) -> Option<Bytes> {
        let s = self.shapes.borrow()[id as usize].clone();
        match s {
            Shape::Empty => Some(Bytes::NONE),
            Shape::Name(n) => {
                self.note_read(n);
                let name = self.name_strs.borrow()[n as usize].clone();
                match self.class_bytes.get(&name) {
                    Some(b) => Some(Bytes::of(b)),
                    None if !self.regional.contains(&name) => self.defs.get(&name).and_then(|d| self.exact_bytes(d)),
                    None => None,
                }
            }
            Shape::Not(a) => self.exact_of(a).map(Bytes::not),
            Shape::And(a, b) => Some(self.exact_of(a)?.and(self.exact_of(b)?)),
            Shape::Or(a, b) => Some(self.exact_of(a)?.or(self.exact_of(b)?)),
            _ => None,
        }
    }

    /// What a mask of stage 1 is at the bytes `at` of a token: none where it holds at none of
    /// them, every one where it holds exactly at all of them, else `test`.
    fn known_over(&self, e: &Expr, at: &[bool; 256], test: impl FnOnce() -> Option<String>) -> Option<Known> {
        let at = Bytes::of(at);
        let b = self.bytes_at(e, 0, &mut HashMap::new());
        if !at.meets(&b) {
            return Some(Known::No);
        }
        if let Some(x) = self.exact_bytes(e)
            && at.within(&x)
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
            // What `in_region_of` read has changed.
            self.region_values.get_mut().clear();
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
        let zb = z.as_ref().map_or(Bytes::NONE, |z| self.bytes_at(z, 0, &mut HashMap::new()));
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
        // What `in_region_of` read has changed.
        self.region_values.get_mut().clear();
        Ok(true)
    }

    /// `e` in one part of a token that reads region `k` (decision 0556's step B): at its start
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
        let at: Vec<Bytes> = (0..4).map(|d| self.bytes_at(&r.opener, d, &mut memo)).collect();
        self.cubes(e, &mut memo).iter().any(|c| c.iter().all(|(o, b)| usize::try_from(q + o).ok().and_then(|d| at.get(d)).is_none_or(|x| x.meets(b))))
    }

    /// Whether a way `e` may hold fits the bytes of region `k`'s closer, at its byte `rr`.
    fn fits_closer(&self, e: &Expr, k: usize, rr: i64) -> bool {
        let Some(r) = self.kinds.get(k) else { return true };
        let mut memo = HashMap::new();
        let cubes = self.cubes(e, &mut memo);
        // Each closer over its own bytes: byte `rr` is a closer's only where it has more than
        // `rr` (decision 0699). `str2`'s `"` is one byte, its `$${` two: the byte after a `"`
        // is no closer's, and may be a name's letter.
        let mut alts = vec![(r.closer.clone(), i64::from(r.bytes))];
        if let Some((c2, m)) = &r.or {
            alts.push((c2.clone(), i64::from(*m)));
        }
        alts.iter().filter(|(_, n)| rr < (*n).max(1)).any(|(c, _)| {
            let at: Vec<Bytes> = (0..4).map(|d| self.bytes_at(c, d, &mut memo)).collect();
            cubes.iter().any(|cube| cube.iter().all(|(o, b)| usize::try_from(rr + o).ok().and_then(|d| at.get(d)).is_none_or(|x| x.meets(b))))
        })
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
            if let std::collections::hash_map::Entry::Vacant(v) = seen.entry(sig) {
                v.insert(b);
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
            let mut w = self.bytes_at(&s1, 0, &mut memo).bools();
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
                if self.regional(e2) || !self.holds_after(e2, &w, &cb.bools(), &atoms)? {
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
        (1..=4).map(body_part).chain([BODY_LATE]).map(|p| self.in_region_of(e, Some(k), p, &Ctx::all(), &mut HashMap::new())).collect()
    }

    /// `in_region` where the byte is one of `ctx` (the bytes the rest of a `&` holds at): a mask
    /// that holds at none of them is none there.
    fn in_region_of(&self, e: &Expr, k: Option<usize>, part: u8, ctx: &Ctx, memo: &mut ByteMemo) -> Option<Expr> {
        self.region_of(e, self.intern(e), k, part, ctx, memo)
    }

    /// `in_region_of` of `e`, interned as `id`, kept (`region_values`).
    fn region_of(&self, e: &Expr, id: u32, k: Option<usize>, part: u8, ctx: &Ctx, memo: &mut ByteMemo) -> Option<Expr> {
        let key = (id, k, part, ctx.0);
        if let Some(v) = self.region_values.borrow().get(&key) {
            return v.clone();
        }
        let v = self.region_once(e, id, k, part, ctx, memo);
        self.region_values.borrow_mut().insert(key, v.clone());
        v
    }

    /// `region_of`, worked out: its operands walked by their ids.
    fn region_once(&self, e: &Expr, id: u32, k: Option<usize>, part: u8, ctx: &Ctx, memo: &mut ByteMemo) -> Option<Expr> {
        let all = || Expr::Not(Box::new(Expr::Empty));
        let opens = self.opens.as_ref()?;
        // A region's start is its opener's first byte.
        let start_ctx: Ctx;
        let ctx = match (k, part) {
            (Some(k), 0) if k < self.kinds.len() => {
                let ob = bytes_in(&self.cubes_id(self.opener_id(k), memo), 0);
                start_ctx = ctx.and0(&ob);
                &start_ctx
            }
            _ => ctx,
        };
        let shape = self.shape(id);
        let here = bytes_in(&self.cubes_id(id, memo), 0);
        if !here.meets(ctx) {
            return Some(Expr::Empty);
        }
        // A fill the loop keeps, outside the regions: itself.
        if k.is_none() && self.loop_read(e) {
            return Some(e.clone());
        }
        // Outside the regions, a shift or a fill of masks made of them: of their values outside,
        // where what is in a region before or after the byte can not change it.
        if k.is_none()
            && self.regional_id(id)
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
            && self.regional_id(id)
            && let Expr::Call(f, args) = e
            && (f == "prev" || f == "next" || f == "fill")
        {
            return self.inside_shift(f, args, kk, part, ctx, memo);
        }
        Some(match e {
            // Assumed at a region token's last byte: none outside one, nor before its closer.
            Expr::Name(n) if matches!(shape, Shape::Name(m) if self.is_end_id(m, n)) => {
                if k.is_none() || !is_closer(part) {
                    Expr::Empty
                } else {
                    return None;
                }
            }
            Expr::Name(n) if self.regional_id(id) => {
                let kind = |p: &str| opens.iter().position(|(q, _, _)| n.strip_prefix(p).and_then(|r| r.strip_prefix('_')) == Some(q.as_str()));
                let holds = if let Some(q) = kind("o") {
                    k == Some(q) && part == 0
                } else if let Some(q) = kind("f") {
                    k == Some(q) && !is_closer(part)
                } else if let Some(d) = self.defs.get(n) {
                    let Shape::Name(m) = shape else { unreachable!("a name's shape") };
                    return self.region_of(d, self.def_id(m, d), k, part, ctx, memo);
                } else {
                    // The hidden bytes.
                    k.is_some() && part > 0
                };
                if holds { all() } else { Expr::Empty }
            }
            Expr::Name(_) | Expr::Empty => e.clone(),
            Expr::Not(a) => {
                let Shape::Not(ia) = shape else { unreachable!("a `~`'s shape") };
                simplify(Expr::Not(Box::new(self.region_of(a, ia, k, part, ctx, memo)?)))
            }
            Expr::And(a, b) => {
                let Shape::And(ia, ib) = shape else { unreachable!("a `&`'s shape") };
                // Each side read where the other holds, at the byte and those about it.
                let mut ca = ctx.clone();
                let mut cb = ctx.clone();
                let (wa, wb) = (self.cubes_id(ia, memo), self.cubes_id(ib, memo));
                for o in -4..=4i64 {
                    let (x, y) = (bytes_in(&wa, o), bytes_in(&wb, o));
                    let i = (o + 4) as usize;
                    ca.0[i] = ca.0[i].and(y);
                    cb.0[i] = cb.0[i].and(x);
                }
                // A side that is none there: none, whatever the other.
                match (self.region_of(a, ia, k, part, &ca, memo), self.region_of(b, ib, k, part, &cb, memo)) {
                    (Some(Expr::Empty), _) | (_, Some(Expr::Empty)) => Expr::Empty,
                    (Some(x), Some(y)) => simplify(Expr::And(Box::new(x), Box::new(y))),
                    _ => return None,
                }
            }
            Expr::Or(a, b) | Expr::Xor(a, b) => {
                let (Shape::Or(ia, ib) | Shape::Xor(ia, ib)) = shape else { unreachable!("a `|`'s shape") };
                let (x, y) = (Box::new(self.region_of(a, ia, k, part, ctx, memo)?), Box::new(self.region_of(b, ib, k, part, ctx, memo)?));
                simplify(if matches!(e, Expr::Or(..)) { Expr::Or(x, y) } else { Expr::Xor(x, y) })
            }
            Expr::Call(..) if !self.regional_id(id) => e.clone(),
            Expr::Call(..) | Expr::Test { .. } => return None,
        })
    }

    /// `prev`, `next` or `fill` (`f` of `args`) at a byte outside the regions that is one of
    /// `ctx`, as of the masks' values outside (`in_region_of`): `prev(a, j)` where the `j` bytes
    /// before, where they are in a region token at its end, are none `a` holds at, in it or
    /// out; `next(a, j)` likewise at a region token's start; `fill(s, e)` where `e` stops it at
    /// a region token's last byte (or at its closer after it), and `s` does not start it there.
    fn outside_shift(&self, f: &str, args: &[Arg], ctx: &Ctx, memo: &mut ByteMemo) -> Option<Expr> {
        self.shift_before(f, args, ctx, &|_| Bytes::ALL, memo)
    }

    /// `outside_shift`, where the bytes `o` on from the byte are `near(o)`'s.
    fn shift_before(&self, f: &str, args: &[Arg], ctx: &Ctx, near: &dyn Fn(i64) -> Bytes, memo: &mut ByteMemo) -> Option<Expr> {
        let ends = self.ends.as_ref()?;
        let all = Bytes::ALL;
        let meets = |x: &Bytes, y: &Bytes| x.meets(y);
        // Whether some way `e` may hold at a byte of `ctx` fits the bytes `at(o)` the place
        // leaves `o` bytes on.
        let fits = |e: &Expr, at: &dyn Fn(i64) -> Bytes, memo: &mut ByteMemo| {
            self.cubes(e, memo).iter().any(|c| {
                let here = c.get(&0).copied().unwrap_or(all).and(**ctx);
                let at = |o: i64| at(o).and(near(o)).and(ctx.at(o));
                // The place fits the bytes about the byte too.
                meets(&here, &at(0)) && (-4..=4).all(|o| at(o).any()) && c.iter().all(|(o, b)| meets(b, &at(*o)))
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
                // Whether `a` is none in region `k`'s parts (its closer's only, or all three): the
                // same at each byte and end of a region, so asked once each.
                let mut none_of: HashMap<(usize, bool), bool> = HashMap::new();
                let mut none_in_region = |k: usize, closer: bool, memo: &mut ByteMemo| {
                    *none_of.entry((k, closer)).or_insert_with(|| {
                        let parts: &[u8] = if closer { &[2] } else { &[0, 1, 2] };
                        parts.iter().all(|&p| matches!(self.in_region_of(a, Some(k), p, &Ctx::all(), memo), Some(Expr::Empty)))
                    })
                };
                let none_in_any = f != "prev" && (0..self.kinds.len()).all(|k| [0u8, 1, 2].iter().all(|&p| matches!(self.in_region(a, Some(k), p), Some(Expr::Empty))));
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
                            let none_in = none_in_region(end.k, end.after.is_none() && m <= end.ext, memo);
                            (none_in || !fits(&e, &at, memo)) && !fits(&e0, &at, memo)
                        })
                    } else {
                        // `o` bytes on is `o - j + m` after the token's first.
                        let at = |o: i64| match o - j + m {
                            d @ 0..=3 => self.opener_at[d as usize],
                            _ => all,
                        };
                        // `a` in the token there: none in any region's part, or no way it holds fits.
                        (none_in_any || !fits(&e, &at, memo)) && !fits(&e0, &at, memo)
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
                let stops = |s: &Expr, e: &Expr, yb: &Bytes, memo: &mut ByteMemo| {
                    !meets(&self.bytes_at(s, 0, memo), yb) && self.exact_bytes(e).is_some_and(|x| yb.within(&x))
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
        let all = Bytes::ALL;
        let meets = |x: &Bytes, y: &Bytes| x.meets(y);
        let fits = |e: &Expr, at: &dyn Fn(i64) -> Bytes, memo: &mut ByteMemo| {
            self.cubes(e, memo).iter().any(|c| {
                let here = c.get(&0).copied().unwrap_or(all).and(**ctx);
                let at = |o: i64| at(o).and(ctx.at(o));
                meets(&here, &at(0)) && (-4..=4).all(|o| at(o).any()) && c.iter().all(|(o, b)| meets(b, &at(*o)))
            })
        };
        // The bytes `d` on from the token's first byte, or from its closer's first.
        let from_open = |d: i64, memo: &mut ByteMemo| if (0..4).contains(&d) { self.bytes_at(&opener, d, memo) } else { all };
        let from_close = |d: i64, memo: &mut ByteMemo| if (0..4).contains(&d) { self.bytes_at(&closers, d, memo) } else { all };
        let val = |a: &Expr, p: u8, memo: &mut ByteMemo| self.in_region_of(a, Some(k), p, &Ctx::all(), memo).filter(|v| !self.regional(v));
        let same = |a: &Option<Expr>, b: &Expr| a.as_ref() == Some(b);
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
                let safe = |v: &Expr, there: Option<Expr>, at: &dyn Fn(i64) -> Bytes, memo: &mut ByteMemo| same(&there, v) || (!fits(&e, at, memo) && !fits(&shifted(v), at, memo));
                if part == 0 {
                    // The token's first byte (its opener's): before it is as before a byte outside.
                    if f != "prev" {
                        return None;
                    }
                    let ob = self.bytes_at(&opener, 0, memo);
                    let c = ctx.and0(&ob);
                    let near = |o: i64| if (0..4).contains(&o) { self.opener_bytes(k, o) } else { all };
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
                            let near = |o: i64| if (0..4).contains(&(q + o)) { self.opener_bytes(k, q + o) } else { all };
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
                                let near = |o: i64| if (0..4).contains(&(q + o)) { self.opener_bytes(k, q + o) } else { all };
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
                                let near = |o: i64| if (0..4).contains(&(q + o)) { self.opener_bytes(k, q + o) } else { all };
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
                    let at_open: Vec<Bytes> = (0..8).map(|d| from_open(d - 4, memo)).collect();
                    for q in 1..=j {
                        let at = |o: i64| at_open.get((q + o + 4) as usize).copied().unwrap_or(all);
                        if !safe(&v, if q == j { v0.clone() } else { None }, &at, memo) {
                            return None;
                        }
                    }
                    if part == 2 {
                        // From the closer's first byte `r` on, `j` or more: in the closer.
                        let v2 = val(a, 2, memo);
                        let at_close: Vec<Bytes> = (0..8).map(|d| from_close(d - 4, memo)).collect();
                        for rr in j..i64::from(r.bytes) {
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
                    let at_close: Vec<Bytes> = (0..8).map(|d| from_close(d - 4, memo)).collect();
                    for q in 1..=j {
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
                let stops = |s: &Expr, e: &Expr, memo: &mut ByteMemo| !meets(&self.bytes_at(s, 0, memo), &ob) && self.exact_bytes(e).is_some_and(|x| ob.within(&x));
                let (s0, e0) = (val(st, 0, memo)?, val(stop, 0, memo)?);
                // Started in no part, and stopped at the token's first byte: none in it.
                let none = |v: Option<Expr>| matches!(v, Some(Expr::Empty));
                if none(Some(s0.clone())) && none(val(st, 1, memo)) && none(val(st, 2, memo)) && self.exact_bytes(&e0).is_some_and(|x| ob.within(&x)) {
                    return Some(Expr::Empty);
                }
                if part == 0 {
                    return self.exact_bytes(&e0).is_some_and(|x| ob.within(&x)).then_some(s0);
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
        let mut opener_at = [Bytes::NONE; 4];
        for r in &self.kinds {
            for (m, at) in opener_at.iter_mut().enumerate() {
                *at = at.or(self.bytes_at(&r.opener, m as i64, &mut memo));
            }
        }
        self.opener_at = opener_at;
        self.ends = Some(ends);
        // What `in_region_of` read has changed.
        self.region_values.get_mut().clear();
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
            arm_bytes.push((self.bytes_at(o, 0, &mut memo).bools(), if self.regional(o) { None } else { self.exact_bytes(o).map(|b| b.bools()) }));
        }
        for r in rows {
            arm_bytes.push((self.bytes_at(&r.start, 0, &mut memo).bools(), if self.regional(&r.start) { None } else { self.exact_bytes(&r.start).map(|b| b.bools()) }));
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
        let mut picks = Picks { open: String::new(), rows: String::new(), by_byte: None, fused: true };
        // Each group's chain, its fused chain, its bytes, how it is shown.
        let mut per: Vec<(Vec<(String, String)>, Vec<(String, String)>, [bool; 256], String)> = Vec::new();
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
            let mut fkept: Vec<(String, String)> = Vec::new();
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
                fkept = fchain.clone();
                if !fchain.is_empty() {
                    let _ = write!(fused_rows, "                // `{shown}`\n                {pats} => {},\n", if_chain(&fchain, "usize::MAX", "                "));
                }
            }
            per.push((chain.clone(), fkept, *b, shown.clone()));
            chains.push(chain.iter().map(|(_, v)| v.parse().unwrap_or(usize::MAX)).collect());
            if !chain.is_empty() {
                let _ = write!(picks.rows, "                // `{shown}`\n                {pats} => {},\n", if_chain(&chain, "usize::MAX", "                "));
            }
        }
        if picks.fused {
            picks.rows = fused_rows;
        }
        // By byte: a group's row where its chain's first is no test, else its arm.
        let tested = rows.len();
        let mut by = [u8::MAX; 256];
        let mut arms = String::new();
        let mut fits = tested + per.len() < 255;
        for (g, (chain, fchain, b, shown)) in per.iter().enumerate() {
            let ch = if picks.fused { fchain } else { chain };
            let v = match ch.first() {
                None => continue,
                Some((c, r)) if c == "true" => r.parse::<usize>().unwrap_or(usize::MAX),
                Some(_) => {
                    let _ = write!(arms, "                // `{shown}`\n                {} => {},\n", tested + g, if_chain(ch, "usize::MAX", "                "));
                    tested + g
                }
            };
            fits &= v < 255;
            for c in 0..256 {
                if b[c] {
                    by[c] = v as u8;
                }
            }
        }
        if fits {
            picks.by_byte = Some((by, arms));
        }
        Ok((tables, picks, chains, row_opens))
    }

    /// Whether `e` may hold at a byte, given the bytes about it (`ctx`): some way it holds
    /// (`cubes`) needs only bytes `ctx` allows at each offset.
    fn may_hold(&self, e: &Expr, ctx: &Ctx) -> bool {
        self.cubes(e, &mut HashMap::new()).iter().any(|c| {
            c.iter().all(|(&o, b)| b.meets(&ctx.at(o)))
        })
    }

    fn bytes_at(&self, e: &Expr, d: i64, memo: &mut ByteMemo) -> Bytes {
        bytes_in(&self.cubes(e, memo), d)
    }

    /// The ways `e` may hold, each the bytes it needs at some offsets from where it holds: a
    /// class's at its own byte; what a `prev` or a `next` reads, shifted; both sides of a `&`
    /// at once (a way whose bytes at an offset are none cannot be); either of a `|`. So `a & b`
    /// where `a` needs a digit after it and `b` a sign there is none.
    fn cubes(&self, e: &Expr, memo: &mut ByteMemo) -> std::rc::Rc<Cubes> {
        self.cubes_id(self.intern(e), memo)
    }

    /// `cubes` of interned expression `id`.
    fn cubes_id(&self, id: u32, memo: &mut ByteMemo) -> std::rc::Rc<Cubes> {
        let s = self.shapes.borrow()[id as usize].clone();
        match s {
            Shape::Empty => std::rc::Rc::new(Vec::new()),
            Shape::Name(n) => self.name_cubes(n, id, memo),
            s => {
                // In a name's definition, a value may read the name's placeholder: made, not kept.
                if self.cube_open.get() > 0 {
                    return self.cubes_made(&s, memo);
                }
                if let Some(r) = self.kept_of(id) {
                    return r;
                }
                let before = self.cube_tangled.get();
                let r = self.cubes_made(&s, memo);
                if self.cube_tangled.get() == before {
                    self.keep(id, &r);
                }
                r
            }
        }
    }

    /// `cubes` of name `n` (shape `id`).
    fn name_cubes(&self, n: u32, id: u32, memo: &mut ByteMemo) -> std::rc::Rc<Cubes> {
        let all = || std::rc::Rc::new(vec![Way::default()]);
        self.note_read(n);
        if let Some(r) = self.kept_of(id) {
            return r;
        }
        let name = self.name_strs.borrow()[n as usize].clone();
        if let Some(set) = self.class_bytes.get(&name) {
            let r = std::rc::Rc::new(vec![Way(vec![(0, Bytes::of(set))])]);
            self.keep(id, &r);
            return r;
        }
        let tangled = self.tangled(n);
        if tangled {
            self.cube_tangled.set(self.cube_tangled.get() + 1);
        }
        if let Some(r) = memo.get(&name) {
            return r.clone();
        }
        // A region's opener mark holds where its opener does.
        if let Some(r) = name.strip_prefix("o_").and_then(|kind| self.kinds.iter().find(|r| r.kind == kind)) {
            let opener = r.opener.clone();
            return self.cubes(&opener, memo);
        }
        // A name being read (a cycle) says nothing.
        memo.insert(name.clone(), all());
        self.cube_open.set(self.cube_open.get() + 1);
        let r = match self.defs.get(&name) {
            Some(def) => self.cubes(def, memo),
            None => all(),
        };
        self.cube_open.set(self.cube_open.get() - 1);
        memo.insert(name, r.clone());
        if !tangled {
            self.keep(id, &r);
        }
        r
    }

    /// `cubes` of a shape not a name, made.
    fn cubes_made(&self, s: &Shape, memo: &mut ByteMemo) -> std::rc::Rc<Cubes> {
        let one = |d: i64, b: Bytes| std::rc::Rc::new(vec![Way(vec![(d, b)])]);
        let all = || std::rc::Rc::new(vec![Way::default()]);
        let made = std::rc::Rc::new;
        match s {
            Shape::Empty => made(Vec::new()),
            Shape::Name(_) => unreachable!("names are `name_cubes`'"),
            // A class's complement, at its own byte, or shifted at the byte it reads (where the
            // class's bytes are exact); `~~a`, `a`'s; else nothing known.
            Shape::Not(a) => match self.shape(*a) {
                Shape::Name(n) => {
                    self.note_read(n);
                    let name = self.name_strs.borrow()[n as usize].clone();
                    match self.class_bytes.get(&name) {
                        Some(set) => one(0, Bytes::of(set).not()),
                        None => all(),
                    }
                }
                Shape::Not(b) => self.cubes_id(b, memo),
                Shape::Call(f, args) => match self.shift_of(f, &args) {
                    Some((x, k, true)) => match self.exact_of(x) {
                        Some(b) => one(k, b.not()),
                        None => all(),
                    },
                    _ => all(),
                },
                _ => all(),
            },
            Shape::And(a, b) => {
                let (x, y) = (self.cubes_id(*a, memo), self.cubes_id(*b, memo));
                let mut v = Vec::new();
                for p in x.iter() {
                    'q: for q in y.iter() {
                        let mut c = p.clone();
                        for (d, b) in q.iter() {
                            if !c.and_at(*d, b) {
                                continue 'q;
                            }
                        }
                        v.push(c);
                    }
                }
                made(widen(v))
            }
            Shape::Or(a, b) | Shape::Xor(a, b) => {
                let mut v = (*self.cubes_id(*a, memo)).clone();
                v.extend(self.cubes_id(*b, memo).iter().cloned());
                made(widen(v))
            }
            Shape::Call(f, args) => {
                // A fill holds at its start, or at a byte its stop is not at.
                if self.is_fn(*f, "fill") {
                    return match **args {
                        [ArgShape::E(a), ArgShape::E(e)] => {
                            let mut v = (*self.cubes_id(a, memo)).clone();
                            let not_e = self.shape_id(Shape::Not(e));
                            v.extend(self.cubes_id(not_e, memo).iter().map(|c| c.iter().filter(|(d, _)| **d == 0).map(|(d, b)| (*d, *b)).collect()));
                            made(widen(v))
                        }
                        _ => all(),
                    };
                }
                // `prev(a, k)` holds `k` bytes after `a`: `a`'s byte `d` is its `d - k`.
                if let Some((a, k, _)) = self.shift_of(*f, args) {
                    return made(self.cubes_id(a, memo).iter().map(|c| c.iter().map(|(d, b)| (d + k, *b)).collect()).collect());
                }
                match args.first() {
                    Some(ArgShape::S(t)) if self.is_fn(*f, "text") && !t.is_empty() => made(vec![t.iter().enumerate().map(|(i, &c)| (i as i64, Bytes::of(&std::array::from_fn(|j| j == usize::from(c))))).collect()]),
                    _ => all(),
                }
            }
            Shape::Test(_) => all(),
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
                self.bytes_at(&read, 0, &mut memo).bools()
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
            let mine = self.bytes_at(e, 0, &mut memo).bools();
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
                    let mine = self.bytes_at(c2, 0, &mut memo).bools();
                    alts.push((test(&before, &mine, format!("bit(&{v}, close)")), Geo { k, ext: i64::from(*m) - 1, h: i64::from(*m), closer: c2.clone(), body }));
                }
                let mine = self.bytes_at(&own, 0, &mut memo).bools();
                alts.push((test(&before, &mine, format!("!bit(&{v}, close)")), Geo { k, ext: i64::from(r.bytes) - 1, h: i64::from(r.bytes), closer: own.clone(), body }));
            }
            None => {
                let mine = self.bytes_at(&own, 0, &mut memo).bools();
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
            let skipped = skip.map(|k| self.bytes_at(k, 0, &mut memo)).unwrap_or(Bytes::NONE);
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
        let at = self.bytes_at(&self.fills_as_all(&v), 0, &mut HashMap::new()).bools();
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
            self.forget_name("code_l");
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
                let skipped = skip.map(|k| self.bytes_at(k, 0, &mut HashMap::new())).unwrap_or(Bytes::NONE);
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
                // A regional mask the walk can not write in every region: its value is the masks'
                // fixed point's, which only a test build makes. Refused, not written so: a product
                // build of that Rust would not compile (decision 0699).
                let (Some(outside), true) = (outside, per.len() == n) else {
                    return Err(format!("the part mask `{f}` reads the regions, and the walk writes it in {} of their {n} kinds", per.len()));
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
        self.forget_cubes();
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
            // What `in_region_of` read has changed.
            self.region_values.get_mut().clear();
        }
        // One region whose closer is its opener, one byte, nothing else (a JSON string): its bytes are the parity of
        // the openers up to each (simdjson's `prefix_xor`), in stage 1, and so is every mask made of it.
        if let [r] = ks.as_slice()
            && format!("{:?}", r.opener) == format!("{:?}", r.closers())
            && r.bytes == 1
            && !r.outside
            && r.empty.is_none()
            && r.or.is_none()
            && r.after.is_empty()
        {
            let (f, o) = (format!("f_{}", r.kind), format!("o_{}", r.kind));
            let inside = Expr::Call("toggle".into(), vec![Arg::E(r.opener.clone())]);
            // Its word named as the grammar names it, where a mask is it alone (`in_string = f_str`, as simdjson's).
            let alias = self.x.stmts.iter().find_map(|s| match s {
                Stmt::Mask(m, Expr::Name(n)) if *n == f && !self.x.stmts.iter().any(|t| matches!(t, Stmt::Mask(m2, _) | Stmt::Class(m2, _) if m2 == m && !matches!(t, Stmt::Mask(_, Expr::Name(n2)) if *n2 == f))) => Some(m.clone()),
                _ => None,
            });
            let (local, what) = match &alias {
                Some(m) => (format!("m_{}", ident(m)), format!("{m}: the region `{}`'s opener to its closer, `toggle({})`", r.kind, show(&r.opener))),
                None => (format!("m_{}", ident(&f)), format!("{f}: a region's opener to its closer, `toggle({})`", show(&r.opener))),
            };
            self.make_node(format!("={f}"), local, what, &inside)?;
            let opener = Expr::And(Box::new(r.opener.clone()), Box::new(Expr::Name(f.clone())));
            self.make_node(format!("={o}"), format!("m_{}", ident(&o)), format!("{o}: its openers"), &opener)?;
            let hidden = Expr::Xor(Box::new(Expr::Name(f.clone())), Box::new(r.opener.clone()));
            self.make_node(format!("={}", rg.hidden), format!("m_{}", ident(&rg.hidden)), format!("{}: after its opener to its closer", rg.hidden), &hidden)?;
            return Ok(());
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
        // The kind of string a template is in: kept only where a region can set it or read it (a second closer, an
        // `after`, a string resumed, the nest).
        let reads_template = rg.nest.is_some() || ks.iter().any(|r| r.or.is_some() || !r.after.is_empty() || r.resumes);
        let template = if reads_template { "\n        // The kind of string the template is in: the region closed last at its second closer (a template's `$`), of the kinds with no `after`.\n        let mut template = usize::MAX;" } else { "" };
        let _ = writeln!(self.body, "    {{\n        let mut at = 0usize;{template}\n        // Each kind's next opener, at or after where it was looked for (`n`: none).");
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
                // With a nest, a template's level: pushed, its `{` the byte after its second
                // closer's last (`tb`: `or C2 bytes M`, `M` bytes on).
                let tm = ors[k].as_ref().map_or(1, |(_, m)| *m);
                let push = |q: usize| match &self.nest {
                    Some((_, _, d, deep)) => format!("if depth == {d} {{\n                                        set(&mut m_{}, c + {tm}, c + {});\n                                    }} else {{\n                                        nest[depth] = {q};\n                                        depth += 1;\n                                    }}\n                                    tb = c + {tm};\n                                    template = {q};", ident(deep), tm + 1),
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
        self.forget_name(&rg.hidden);
        // The nest's `else`: no mask of the loop's (it refuses where it would push past its levels).
        if let Some(n) = &rg.nest {
            self.names.insert(n.deep.clone(), format!("m_{}", ident(&n.deep)));
        }
        for r in ks {
            self.names.insert(format!("o_{}", r.kind), format!("m_o_{}", ident(&r.kind)));
            self.names.insert(format!("f_{}", r.kind), format!("m_f_{}", ident(&r.kind)));
            self.regional.insert(format!("o_{}", r.kind));
            self.forget_name(&format!("o_{}", r.kind));
            self.regional.insert(format!("f_{}", r.kind));
            self.forget_name(&format!("f_{}", r.kind));
            if let Some((e, _)) = &r.empty {
                self.names.insert(e.clone(), format!("m_{}", ident(e)));
                self.regional.insert(e.clone());
                self.forget_name(&e);
            }
            // A kind that nests: its `else`, no mask of the loop's (it refuses past its levels).
            if let Some((_, e)) = &r.nests {
                self.names.insert(e.clone(), format!("m_{}", ident(e)));
            }
        }
        // What `in_region_of` read has changed.
        self.region_values.get_mut().clear();
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
        let open = if resumed { "\n                        if opened == held.len() {\n                            refused = true;\n                            break;\n                        }\n                        held[opened] = len;\n                        opened += 1;" } else { "" };
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

    /// `lex`'s start: the file's sizes, and its words' bits inside the file (the prefix's out).
    fn lex_head(&self) -> String {
        format!(
            "/// `src`'s records.\npub fn lex(src: &[u8]) -> Lexed {{\n    let n = src.len();\n    let nw = n.div_ceil(64);\n    // Each word's bits inside the file.\n    let {}lv: Vec<u64> = (0..nw).map(|q| if 64 * q + 64 <= n {{ !0 }} else {{ (1u64 << (n % 64)) - 1 }}).collect();\n{}",
            if self.prefix.is_some() { "mut " } else { "" },
            self.prefix_code(false, "    ")
        )
    }

    /// Tier 1 with no `skip`: simdjson's stage 1. Its records are each token's first byte, the bits of the rows'
    /// starts (simdjson's structural indexes), each with its row's kind; tier 2 finds a token's end from the masks
    /// tier 1 gives it (`stage1_ends`, `readers`), as simdjson's stage 2 does.
    fn records(mut self, rows: &[(usize, &Stmt)]) -> Result<String, String> {
        let x = self.x;
        let mut all: Option<Expr> = None;
        let mut chain = String::new();
        // Every mask in the pass: the records too, there (simdjson's stage 1), with no kinds.
        // (After stage 1 then, only its escapes' verdict: every escape tested in the pass.)
        let in_pass = self.body.trim() == "let mut refused = escapes;";
        for (row, s) in rows {
            let Stmt::Token { kind, start, rule, .. } = s else { continue };
            if matches!(rule, Some(TokenRule::Inside(..))) {
                continue;
            }
            let st = if in_pass { String::new() } else { self.val(start)? };
            let k = self.kind_of(*row);
            let _ = write!(chain, "{}if bit(&{st}, s) {{\n                // `{kind}`\n                {k}\n            }} ", if chain.is_empty() { "" } else { "else " });
            all = Some(match all.take() {
                Some(a) => Expr::Or(Box::new(a), Box::new(start.clone())),
                None => start.clone(),
            });
        }
        let all_e = factored(&all.ok_or("tier 1 as Rust: no token")?, x.stmts);
        let all_e = refused_equal(&all_e, x.stmts).unwrap_or(all_e);
        let all = if in_pass {
            self.index = Some(self.node(&all_e)?);
            self.outputs.push(all_e.clone());
            String::new()
        } else {
            self.val(&all_e)?
        };
        // The masks tier 2 reads: a reader's, then each row's end's.
        let mut fields: Vec<(String, Expr)> = readers(x.stmts).into_iter().flat_map(|r| r.masks).collect();
        fields.extend(stage1_ends(x.stmts)?);
        let mut used: Vec<String> = Vec::new();
        let mut fields_init = String::new();
        let mut decl = String::new();
        for (f, e) in fields.iter().filter(|_| !in_pass) {
            let v = self.val(e)?;
            let v = if used.contains(&v) { format!("{v}.clone()") } else { v };
            used.push(v.clone());
            let _ = writeln!(decl, "    pub {f}: Vec<u64>,");
            let _ = writeln!(fields_init, "            {f}: {v},");
        }
        let mut out = String::new();
        out.push_str("// Generated by native/crates/compiler/src/bitlex_lex_rust.rs; do not edit. The Bitlex compiler's Rust from the grammar's lexer statements.\n\n");
        if !in_pass {
            let _ = write!(out, "/// The masks tier 2 reads, a bit a byte: those it finds a token's end by, and splits a region by.\npub struct Masks {{\n{decl}}}\n\n");
        }
        if in_pass {
            // Nothing after stage 1 reads a mask: simdjson's stage 1 writes its indexes alone.
            for n in &mut self.nodes {
                n.kept = false;
            }
            out.push_str("/// The bytes the source has past its end, spaces (simdjson's padding): tier 1 loads each block where it is, and\n/// tier 2 reads the byte at the index's last entries.\npub const PADDING: usize = 64;\n\n");
            out.push_str("/// Tier 1's records, simdjson's stage 1: each token's first byte (its structural index), then `len`, `len` and 0;\n/// their count; the file's length; and whether the file goes to tree-sitter. Tier 2 reads the rest from the bytes.\npub struct Lexed {\n    pub starts: Vec<u32>,\n    pub count: usize,\n    pub len: usize,\n    pub refused: bool,\n}\n\n");
            out.push_str(&self.stage1());
            let kept: Vec<&str> = self.nodes.iter().filter(|n| n.kept).map(|n| n.name.as_str()).collect();
            let _ = write!(
                out,
                "/// The records of `buf[..n]`, its `PADDING` bytes after it spaces.\npub fn lex(buf: &[u8], n: usize) -> Lexed {{\n    assert!(buf.len() >= n + PADDING && buf[n..n + PADDING].chunks_exact(8).all(|w| w == b\"        \"), \"the source padded with spaces\");\n    // Stage 1: the records, and whether an escape holds.\n    let Stage1 {{ {}starts, count, refused }} = stage1_best(buf, n);\n    Lexed {{ starts, count, len: n, refused }}\n}}\n",
                kept.iter().map(|k| format!("{k}, ")).collect::<String>()
            );
            out.push_str(&used_helpers(&out));
            return Ok(out);
        }
        out.push_str("/// Tier 1's records, simdjson's stage 1: each token's first byte and its kind, the masks tier 2 reads, and whether the file goes to tree-sitter.\npub struct Lexed {\n    pub starts: Vec<u32>,\n    pub codes: Vec<u32>,\n    pub masks: Masks,\n    pub refused: bool,\n}\n\n");
        let kinds: Vec<String> = x.kinds.iter().map(|k| k.to_string()).collect();
        let _ = write!(out, "/// Each byte's record kind, as a token's first byte.\nconst KINDS: [u8; 256] = [{}];\n\n", kinds.join(", "));
        out.push_str(&self.stage1());
        out.push_str(&self.lex_head());
        let kept: Vec<&str> = self.nodes.iter().filter(|n| n.kept).map(|n| n.name.as_str()).collect();
        let _ = writeln!(out, "    // Stage 1: the masks the regions and the tokens read, and whether an escape holds.\n    let Stage1 {{ {}, refused: escapes }} = stage1_best(src);", kept.join(", "));
        out.push_str(&self.body);
        let _ = write!(
            out,
            "    // The tokens: each bit of a row's start, in order, its kind its row's.\n    let mut starts: Vec<u32> = Vec::with_capacity(n / 4 + 1);\n    let mut codes: Vec<u32> = Vec::with_capacity(n / 4 + 1);\n    for (q, &word) in {all}.iter().enumerate() {{\n        let mut bits = word;\n        while bits != 0 {{\n            let s = 64 * q + bits.trailing_zeros() as usize;\n            bits &= bits - 1;\n            let kind: u32 = {chain}else {{\n                0\n            }};\n            starts.push(s as u32);\n            codes.push(kind);\n        }}\n    }}\n    Lexed {{\n        starts,\n        codes,\n        masks: Masks {{\n{fields_init}        }},\n        refused,\n    }}\n}}\n"
        );
        out.push_str(HELPERS);
        if self.body.contains("toggle(&") {
            out.push_str(TOGGLE);
        }
        Ok(out)
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
                self.forget_name(&name);
            }
        }
        let mut skip: Option<Expr> = None;
        let mut marks: Vec<&Stmt> = Vec::new();
        let mut rows: Vec<(usize, &Stmt)> = Vec::new();
        let mut row = 0usize;
        let mut escapes: Vec<(&str, &Expr, Scope)> = Vec::new();
        // The counts (`scan N = count(up, down) max M`), each an error past its bound.
        let mut counts: Vec<(&str, &Expr, &Expr, u32, Scope)> = Vec::new();
        let mut token_escapes: Vec<&Stmt> = Vec::new();
        for s in x.stmts {
            match s {
                // A token's check is tier 2's, in the token's arm.
                Stmt::Lexer(_) | Stmt::Class(..) | Stmt::Keywords { .. } | Stmt::TokenError(..) | Stmt::Tree { .. } | Stmt::Check { .. } | Stmt::Assume { .. } | Stmt::AssumeEnds { .. } | Stmt::Speculate { .. } => {}
                Stmt::Prefix(b) => self.prefix = Some(b.clone()),
                // A mask not made of the regions: stage 1's.
                Stmt::Mask(name, e) if !self.regional(e) => {
                    self.defs.insert(name.clone(), e.clone());
                    self.forget_name(&name);
                    let id = match e {
                        Expr::Name(_) => self.node(e)?,
                        _ => {
                            let key = format!("{e:?}");
                            match self.node_of.get(&key) {
                                Some(&id) => id,
                                None => {
                                    let local = self.fresh(name);
                                    self.make_node(key, local, format!("mask {name} = {}", show(e)), e)?
                                }
                            }
                        }
                    };
                    self.node_of.insert(format!("={name}"), id);
                }
                Stmt::Mask(name, e) => {
                    self.regional.insert(name.clone());
                    self.forget_name(&name);
                    self.defs.insert(name.clone(), e.clone());
                    self.forget_name(&name);
                    let _ = writeln!(self.body, "    // mask {name} = {}", show(e));
                    // Its expression reads the names before it, so it is made before it is a name of the regions.
                    let v = match e {
                        Expr::Name(n) => self.val(&Expr::Name(n.clone()))?,
                        _ => {
                            // A name defined again (a class `high`, then a mask `high`) is a local of its own: what
                            // was made of the first keeps reading it.
                            let local = self.fresh(name);
                            self.make(&local, e)?;
                            local
                        }
                    };
                    self.regional.insert(name.clone());
                    // What `in_region_of` read has changed.
                    self.region_values.get_mut().clear();
                    self.bind(name, v);
                }
                Stmt::Rounds { regions: Some(rg), .. } => self.regions(rg)?,
                // Its names as they are here: a name defined again after it is not its.
                Stmt::Escape(why, e) | Stmt::Error(why, e) => escapes.push((why, e, self.scope())),
                Stmt::TokenEscape { .. } => token_escapes.push(s),
                Stmt::Count { name, up, down, reset: None, max } => {
                    if x.stmts.iter().any(|t| matches!(t, Stmt::Mask(_, e) | Stmt::Escape(_, e) | Stmt::Error(_, e) | Stmt::Skip(e) if { let mut v = Vec::new(); names_in(e, &mut v); v.contains(name) })) {
                        return Err(format!("tier 1 as Rust: count {name} read as a mask is not written yet"));
                    }
                    counts.push((name, up, down, *max, self.scope()));
                }
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
        // One pass (decision 0619): stage 1 makes the blocks as the token loop comes to need them, where the loop keeps
        // nothing over a token but its records and counts (no template's string, no fill kept over regions, no region
        // read in parts for tier 2), and the file is read from its first byte (no `prefix`).
        let fused = skip.is_some()
            && self.prefix.is_none()
            && x.assumed_ends.is_empty()
            && self.kinds.iter().all(|r| !r.resumes && r.or.is_none() && r.after.is_empty())
            && !x.stmts.iter().any(|s| matches!(s, Stmt::Token { rule: Some(TokenRule::ReadParts(..)), .. }));
        // The escapes and errors: a bit of any sends the file to tree-sitter.
        // Those not made of the regions, stage 1 tests a block at a time.
        // Those made of the regions where they hold only in a region a token reads: tests there.
        // With no `skip`, tier 1 is simdjson's stage 1 (`records`): no token reads a region in it, so each of the
        // regions' escapes is tested over the file.
        let records = skip.is_none() && marks.is_empty() && token_escapes.is_empty();
        // In simdjson's stage 1, the byte-pair errors are its `utf8_checker`, where they are its form.
        if records {
            self.lookup = crate::bitlex_pair_lookup::lookup(x.stmts, self.prefix.as_deref()).ok();
        }
        // In simdjson's stage 1, the errors at an escape are tier 2's, where it reads the escape (decision 0599).
        // A scalar's dirty end too: tier 2 tests it at the run's end (`run_end_covers`), as simdjson's stage 2.
        let pieces: Vec<usize> = if records { piece_covers(x.stmts).into_iter().chain(run_end_covers(x.stmts)).collect() } else { Vec::new() };
        let covered: Vec<*const Expr> = self
            .lookup
            .iter()
            .flat_map(|l| l.covers.iter().map(|(i, _)| i))
            .chain(pieces.iter())
            .filter_map(|i| match &x.stmts[*i] {
                Stmt::Error(_, e) | Stmt::Escape(_, e) => Some(e as *const Expr),
                _ => None,
            })
            .collect();
        let read: Vec<usize> = match &self.opens {
            Some(_) if records => Vec::new(),
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
                Some(k) => self.bytes_at(k, 0, &mut HashMap::new()).bools(),
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
            for (_, e, _) in &escapes {
                let mut inner = Vec::new();
                fills_in(e, &mut inner);
                exprs.extend(inner.into_iter().map(|f| (None, f)));
            }
            self.find_ends(&rows)?;
            self.find_loop_fills(&rows, &exprs)?;
        }
        let mut refuse = Vec::new();
        for (why, e, at) in escapes.iter().filter(|(_, e, _)| !covered.contains(&(*e as *const Expr))) {
            let line = self.in_scope(at, e, |g| {
                // Past the nest's levels: the loop refuses where it would push one more.
                if let (Expr::Name(x), Some((.., deep))) = (e, &g.nest)
                    && x == deep
                {
                    return Ok(None);
                }
                // Past a kind's own levels, likewise (`Region::nests`).
                if let Expr::Name(x) = e
                    && g.kinds.iter().any(|r| r.nests.as_ref().is_some_and(|(_, d)| d == x))
                {
                    return Ok(None);
                }
                if !g.regional(e) {
                    // At the file's last byte alone: once, after the pass.
                    if let Expr::And(a, b) = e {
                        let (pre, defs) = (g.prefix.as_ref().map_or(0, Vec::len), &g.defs);
                        let other = if is_last_byte(b, defs, pre) { Some(a) } else if is_last_byte(a, defs, pre) { Some(b) } else { None };
                        if let Some(other) = other {
                            let id = g.node(other)?;
                            g.end_sinks.push((why.to_string(), id));
                            return Ok(None);
                        }
                    }
                    let id = g.node(e)?;
                    g.sinks.push((why.to_string(), id));
                    g.outputs.push((*e).clone());
                    return Ok(None);
                }
                if g.span_escape(why, e, skip.as_ref())? {
                    return Ok(None);
                }
                // At the file's last byte, a fill the loop keeps: tested after the loop.
                if let Expr::And(a, b) = e
                    && let Some(i) = [(a, b), (b, a)].iter().find_map(|(l, f)| if g.is_last_byte(l) { g.fill_of(f) } else { None })
                {
                    g.end_checks.push((why.to_string(), i));
                    return Ok(None);
                }
                if g.stretch_escape(why, e)? || g.start_escape(why, e, &rows)? || g.region_escape(why, e, &read, skip.as_ref())? || g.code_escape(why, e)? {
                    return Ok(None);
                }
                let v = g.val(e)?;
                Ok(Some(format!("    // {why}\n    refused |= any(&{v});\n")))
            })?;
            refuse.extend(line);
        }
        let _ = writeln!(self.body, "    let mut refused = escapes;");
        for r in &refuse {
            self.body.push_str(r);
        }
        // A count: its bits in order, as the token loop reads the starts, up one at `up` (where both hold, `up`'s),
        // down one at `down` and never below 0; past its bound, the file goes to tree-sitter.
        // A count in the token loop, where it has one (decision 0619): its bits in each token of code, whose bytes
        // hold no region's mask (a region opens only at a token's start, and its bytes are its token's), and at none of
        // the skipped bytes. Its name, its masks (`up`, then both) as stage 1 makes them, and its bound.
        let mut token_scans: Vec<(String, String, String, u32, [bool; 256])> = Vec::new();
        if skip.is_some() {
            let regional: Vec<String> = self.kinds.iter().flat_map(|r| [format!("o_{}", r.kind), format!("f_{}", r.kind)].into_iter().chain(r.empty.iter().map(|(e, _)| e.clone()))).chain(std::iter::once(self.hidden.clone())).collect();
            let none: Vec<(&str, &Expr)> = regional.iter().map(|n| (n.as_str(), &Expr::Empty)).collect();
            let skipped = skip.as_ref().map(|k| self.bytes_at(k, 0, &mut HashMap::new())).unwrap_or(Bytes::NONE);
            counts.retain(|(name, up, down, max, at)| {
                let (u, d) = (substitute(up, &none, &self.defs), substitute(down, &none, &self.defs));
                let both = Expr::Or(Box::new(u.clone()), Box::new(d));
                let bytes = self.bytes_at(&both, 0, &mut HashMap::new());
                if self.regional(&both) || skipped.meets(&bytes) {
                    return true;
                }
                match self.in_scope(at, &both, |g| Ok((g.val(&u)?, g.val(&both)?))) {
                    Ok((u, b)) => {
                        token_scans.push((name.to_string(), u, b, *max, bytes.bools()));
                        false
                    }
                    Err(_) => true,
                }
            });
        }
        for (name, up, down, max, at) in &counts {
            let both = Expr::Or(Box::new((*up).clone()), Box::new((*down).clone()));
            let (u, b) = self.in_scope(at, &both, |g| Ok((g.val(up)?, g.val(&both)?)))?;
            let _ = write!(
                self.body,
                "    // scan {name} = count({}, {}) max {max}\n    {{\n        let mut c = 0u32;\n        'count: for (q, &word) in {b}.iter().enumerate() {{\n            let mut bits = word;\n            while bits != 0 {{\n                let i = bits.trailing_zeros();\n                bits &= bits - 1;\n                if {u}[q] >> i & 1 == 1 {{\n                    c += 1;\n                    if c > {max} {{\n                        refused = true;\n                        break 'count;\n                    }}\n                }} else {{\n                    c = c.saturating_sub(1);\n                }}\n            }}\n        }}\n    }}\n",
                show(up),
                show(down)
            );
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
            /// Its mask's one byte, where tier 2 may ask it of the bytes (decision 0623 step 5): kept by no
            /// accumulator, an extra keeps none of it, and no `else escape` reads it at the file's end.
            byte: Option<u8>,
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
            Some(k) => self.bytes_at(k, 0, &mut HashMap::new()).bools(),
            None => [false; 256],
        };
        self.skipped = skipped;
        let mut gaps: Vec<Gap> = Vec::new();
        let mut unplaced_code = String::new();
        // The same where tier 2 asks the token tests (`tested`, decision 0655 step 6): each `else escape` mark's bit and
        // why; whether one reads a gap mask besides (then the tests stay the loop's).
        let mut unplaced_asks: Vec<(u8, String, String)> = Vec::new();
        let mut mark_names: HashMap<u8, String> = HashMap::new();
        let mut unplaced_masked = false;
        // Each mark's token tests: the kind, `after`, the test, the mask, the least, the bit.
        let mut tests: Vec<(String, bool, TokenTest, Expr, u32, u8)> = Vec::new();
        for (m, s) in marks.iter().enumerate() {
            let Stmt::Mark { kind, at, unless, unplaced, checks, .. } = s else { continue };
            let Some(bit) = x.mark_bits.get(m).copied().flatten() else { continue };
            mark_names.insert(bit, kind.clone());
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
            // `else escape`: a bit of it with no grammar token at or after it refuses, at the file's end.
            if let Some(why) = unplaced {
                unplaced_asks.push((bit, kind.clone(), why.clone()));
                unplaced_masked |= mask.is_some();
                let left = match &mask {
                    Some(m) => format!("pending >> {bit} & 1 == 1 || any_in(&{m}, from, n)"),
                    None => format!("pending >> {bit} & 1 == 1"),
                };
                let _ = write!(unplaced_code, "    // `{kind}`, else escape: {why}\n    if {left} {{\n        refused = true;\n    }}\n");
            }
            let unless = unless
                .iter()
                .map(|w| x.words.iter().position(|x| x == w).map(|i| i as u32 + 1).ok_or_else(|| format!("tier 1 as Rust: mark {kind}: `unless after` a word no code names")))
                .collect::<Result<_, _>>()?;
            for c in checks {
                tests.push((c.kind.clone(), c.after, c.test, c.expr.clone(), c.least, bit));
            }
            let byte = match (&mask, acc, unplaced, x.own >> bit & 1) {
                (Some(_), None, None, 0) => bytes_of_expr(at, x.stmts).and_then(|set| {
                    let mut one = (0..=255u8).filter(|&b| set[usize::from(b)]);
                    one.next().filter(|_| one.next().is_none())
                }),
                _ => None,
            };
            gaps.push(Gap { bit, mask, unless, name: kind.clone(), acc, rec, byte });
        }
        let skip = match &skip {
            Some(e) => self.val(e)?,
            None if records => return self.records(&rows),
            None => return Err("tier 1 as Rust: marks or token escapes with no `skip` are not written yet".into()),
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
        // The pairs rows with no triple: their function, and their pairs' bytes (for the block's mask, `pairs_masks`).
        let mut pair_rows: Vec<(String, Vec<(u8, u8)>)> = Vec::new();
        let mut checked: Vec<String> = Vec::new();
        // The gap accumulators a row's bytes after its start may hold.
        let mut tails: std::collections::BTreeSet<usize> = Default::default();
        // Each row's start, its test build's checks and its code: picked by `dispatch`.
        let mut row_arms: Vec<RowArm> = Vec::new();
        // The marks' token tests, for tier 2 to ask where it reads a mark (decision 0655 step 6): each one's bit, `after`,
        // its row's kind and its test; and each row's kind, whether an extra, its extra bits, its first bytes, its kind.
        let mut test_asks: Vec<(u8, bool, String, String)> = Vec::new();
        // The token tests a row's own read decides (`run R once D`, its test a count of `D` of one at least, at the
        // token): the run read its one `D` (`once`), as the lean lexer's number arm knows it read a `.`. Each its row's
        // kind and the test's bit.
        let mut once_asks: Vec<(String, u8)> = Vec::new();
        let mut row_ids: Vec<(String, bool, u32, [bool; 256], String)> = Vec::new();
        // Each row's rule where a common token's (decision 0655 step 4): a run of one mask (`run M`), a pair (`pairs F`:
        // the pairs mask `k_F`; `pair M`), one byte (`one`).
        let mut row_shapes: Vec<Option<String>> = Vec::new();
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
            let mut open_k: Option<String> = None;
            let (how, l) = match rule {
                Some(TokenRule::Run(r)) => {
                    // Its run as its kind's assumptions read it (`in KIND`, decision 0607).
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
                // `pair empty`: no byte pairs, the token its first byte.
                Some(TokenRule::Pair(Expr::Empty)) => ("one byte".to_owned(), "s".to_owned()),
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
                    if three.is_empty() {
                        pair_rows.push((f.clone(), pairs.iter().filter_map(|p| match p.as_slice() {
                            [a, b] => Some((*a, *b)),
                            _ => None,
                        }).collect()));
                    }
                    let third = if three.is_empty() { "0".to_owned() } else { format!("usize::from(matches!([a, b, c], {}))", three.join(" | ")) };
                    let _ = write!(pair_fns, "/// `{kind}`'s last byte from its first, `s`: its next where the two are a pair, and the one after where the three are a triple, as the lean lexer reads an operator.\nfn {f}(src: &[u8], s: usize) -> usize {{\n    let at = |i: usize| src.get(i).copied().unwrap_or(0);\n    let (a, b, c) = (at(s), at(s + 1), at(s + 2));\n    match [a, b] {{\n        {} => s + 1 + {third},\n        _ => s,\n    }}\n}}\n\n", if two.is_empty() { "[_, _] if false".to_owned() } else { two.join(" | ") });
                    ("pair".to_owned(), format!("{f}(src, s)"))
                }
                Some(TokenRule::Read(rs)) => {
                    let mut code = String::new();
                    // `open K`: `K`'s record kind, the kind of a byte it begins with (`KINDS`), or its row's own.
                    let open_kind = |this: &mut Self, name: &str| -> Result<String, String> {
                        let Some((r, start)) = rows.iter().find_map(|(r, q)| match q {
                            Stmt::Token { kind, start, .. } if kind == name => Some((*r, start.clone())),
                            _ => None,
                        }) else {
                            return Err(format!("tier 1 as Rust: `open {name}`, no such token"));
                        };
                        if let Some(k) = this.x.row_kinds.get(r).copied().flatten() {
                            return Ok(format!("{k}"));
                        }
                        let at = this.bytes_at(&start, 0, &mut HashMap::new());
                        let mut ks: Vec<u8> = (0..256).filter(|&i| at[i] && !this.skipped[i]).map(|i| this.x.kinds[i]).collect();
                        ks.sort_unstable();
                        ks.dedup();
                        match ks.as_slice() {
                            [k] => Ok(format!("{k}")),
                            _ => Err(format!("tier 1 as Rust: `open {name}`, a token of one record kind, not {}", ks.len())),
                        }
                    };
                    for r in rs.iter().filter(|r| r.open.is_some()) {
                        let k = open_kind(&mut self, r.open.as_deref().unwrap_or_default())?;
                        match &open_k {
                            Some(o) if *o != k => return Err(format!("tier 1 as Rust: `{kind}`'s regions open as two kinds")),
                            _ => open_k = Some(k),
                        }
                    }
                    for (i, r) in rs.iter().enumerate() {
                        // A region with `else E bytes M`: its opener with no closer after it is `E`'s, a token of `M` bytes
                        // (the records' reader's `ReadNone::Bytes`).
                        let o = match self.opens.as_ref().and_then(|v| v.iter().position(|(k, _, _)| *k == r.kind)) {
                            Some(k) => format!("open == {k}"),
                            None => {
                                // Where the loop numbers no regions: the region the token opens is the first whose
                                // opener holds at its start, which is never hidden (the loop is in no region); its
                                // `else` form (no closer after it) too, which `none` reads. A test build checks it
                                // against the regions' walk.
                                let at = self.at_start(&r.opener)?.ok_or_else(|| format!("tier 1 as Rust: region `{}`'s opener at a start reads a region", r.kind))?;
                                let walk = match &r.empty {
                                    Some((e, _)) => format!("bit(&{}, s) || bit(&{}, s)", self.val(&Expr::Name(format!("o_{}", r.kind)))?, self.val(&Expr::Name(e.clone()))?),
                                    None => format!("bit(&{}, s)", self.val(&Expr::Name(format!("o_{}", r.kind)))?),
                                };
                                format!("{{\n                #[cfg(feature = \"test-hooks\")]\n                assert_eq!({at}, {walk}, \"region `{}` opens at byte {{s}}\");\n                {at}\n            }}", r.kind)
                            }
                        };
                        let none = match &r.empty {
                            // In one pass (decision 0619), no closer in the blocks made so far is none in the file only
                            // once every block is made: before, the token is read again.
                            Some((_, m)) if fused => format!("{{\n                    if made < nw {{\n                        wait = (2 * made - b).min(nw);\n                        continue 'pass;\n                    }}\n                    s + {}\n                }}", m.saturating_sub(1)),
                            Some((_, m)) => format!("s + {},", m.saturating_sub(1)),
                            // `open K`: to the file's end, `K`'s token (the lean lexer's `Bad`), once every block is made.
                            None if r.open.is_some() && fused => "{\n                    if made < nw {\n                        wait = (2 * made - b).min(nw);\n                        continue 'pass;\n                    }\n                    open_bad = true;\n                    n - 1\n                }".to_owned(),
                            None if r.open.is_some() => "{\n                    open_bad = true;\n                    n - 1\n                }".to_owned(),
                            None => "{\n                    refused = true;\n                    break;\n                }".to_owned(),
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
                            format!("match {search} {{\n                Some(c) => {some},\n                None => {none}\n            }}")
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
            let row_shape: Option<String> = match rule {
                Some(TokenRule::Run(r)) if l == format!("run_last(&{}, s)", self.val(r)?) => Some(format!("run {}", self.val(r)?)),
                Some(TokenRule::Pairs { .. }) if l == format!("pair_{row}(src, s)") && pair_rows.iter().any(|(f, _)| *f == format!("pair_{row}")) => Some(format!("pairs pair_{row}")),
                Some(TokenRule::Pair(p)) if l == format!("s + usize::from(bit(&{}, s))", self.val(p)?) => Some(format!("pair {}", self.val(p)?)),
                None | Some(TokenRule::Pair(Expr::Empty)) if l == "s" => Some("one".to_owned()),
                _ => None,
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
                if let Some(TokenRule::RunOnce(_, d)) = rule
                    && !*after
                    && matches!(test, TokenTest::Count)
                    && *least == 1
                    && v == self.val(d)?
                {
                    once_asks.push((kind.clone(), *bit));
                }
                test_asks.push((*bit, *after, kind.clone(), holds));
            }
            // The gap masks the loop keeps, over a token of code's bytes after its start.
            if !matches!(rule, Some(TokenRule::Read(_) | TokenRule::ReadParts(..))) {
                let tail = self.row_tail_bytes(kind, rule, last);
                for (j, a) in accs.iter().enumerate() {
                    let at = self.bytes_at(&a.code, 0, &mut HashMap::new());
                    if (0..256).any(|i| at[i] && tail[i]) {
                        let v = self.val(&a.code)?;
                        let _ = writeln!(hits, "            tail_{j} = any_in(&{v}, s + 1, l);");
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
                    let _ = writeln!(hits, "            fill_{i} = {fl};");
                    if f.prev {
                        let _ = writeln!(hits, "            xl_{i} = {xl};");
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
                at_tail.0[3] = in_token.into();
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
                    Some(format!("            // Resumes the open string's record: no record of its own.\n            match opened {{\n                0 => {{\n                    refused = true;\n                    break;\n                }}\n                _ => {{\n                    // SAFETY: record `held[opened - 1]` is written (`< len`).\n                    unsafe {{ (*out.add(held[opened - 1])).last = l as u32 }};\n                    {again}\n                }}\n            }}\n            at = l + 1;\n            if at >= base + 64 {{\n                b = at / 64;\n                continue 'tokens;\n            }}\n            firsts &= !0u64 << (at - base);\n            continue;\n        }}"))
                }
                _ => None,
            };
            // In one pass (decision 0619): a token whose byte after it is in a block stage 1 has not made is read
            // again once it has made more (its distance from the token's block, again), as is one the blocks made so
            // far refuse; the token changed nothing yet.
            let (l, pause) = if fused {
                (pause_refusals(&made_searches(&l)), "            // The byte after the token in a block stage 1 has not made: it makes more, and the token is read again.\n            if l + 1 >= 64 * made && made < nw {\n                wait = (2 * made - b).min(nw);\n                continue 'pass;\n            }\n".to_owned())
            } else {
                (l, String::new())
            };
            // The counts in the token loop, at a token of code whose bytes may hold its: its bits in the token's bytes,
            // in order.
            let mut scans = String::new();
            if !matches!(rule, Some(TokenRule::Read(_) | TokenRule::ReadParts(..))) {
                for (name, u, both, max, _) in token_scans.iter().filter(|c| (0..256).any(|i| (sb[i] || tail[i]) && c.4[i])) {
                    let _ = write!(scans, "            // scan {name}: its bits in the token's bytes, all of them code\n            let mut q = s;\n            while let Some(i) = first_in(&{both}, q, l) {{\n                if bit(&{u}, i) {{\n                    c_{name} += 1;\n                    if c_{name} > {max} {{\n                        refused = true;\n                        break;\n                    }}\n                }} else {{\n                    c_{name} = c_{name}.saturating_sub(1);\n                }}\n                q = i + 1;\n            }}\n", name = ident(name));
                }
            }
            let word = !extra && x.words.iter().any(|w| w.first().is_some_and(|&c| sb[usize::from(c)]) && w[1..].iter().all(|&c| tail[usize::from(c)]));
            // A region open at the file's end (`open K`): `K`'s record, no word, no marks, none of the tests.
            let (decl, k, extra_is, hits) = match &open_k {
                Some(o) => ("            let mut open_bad = false;\n".to_owned(), if *o == k { k } else { format!("if open_bad {{ {o} }} else {{ {k} }}") }, if extra { "!open_bad".to_owned() } else { "false".to_owned() }, if hits.is_empty() { hits } else { format!("            if !open_bad {{\n{}            }}\n", indent(&hits, "    ")) }),
                None => (String::new(), k, extra.to_string(), hits),
            };
            row_ids.push((kind.clone(), extra, bits, sb, self.kind_of(*row)));
            row_shapes.push(row_shape);
            row_arms.push(RowArm { resume, word, open: open_k.is_some(), row: *row, first: sb, tail, starts: first_tests, start: start.clone(), check, body: format!("{{\n            // `{kind}`: {how}\n{decl}            //~defs\n            //~spans\n            let l = {l};\n            let l = l.max(s);\n{pause}{scans}            //~passed\n{hits}            (l, {k}, {extra_is}, "), bits });
        }
        // Each gap mark's bit at a grammar token, and at an extra the marks it keeps of its own gap.
        let mut gap_code = String::new();
        let mut own_code = String::new();
        // The same where tier 2 asks the one-byte gap marks (`asked`) of the bytes: only the token tests' hits of them.
        let mut gap_asked = String::new();
        let mut asked: Vec<(u8, u8, String, String)> = Vec::new();
        // The same where tier 2 asks the token tests too (`tested`): no hits; and each gap mark's `unless`, by its bit.
        let mut gap_tested = String::new();
        let mut unless_of: HashMap<u8, String> = HashMap::new();
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
            if !unless.is_empty() {
                unless_of.insert(g.bit, unless.clone());
            }
            let untested = match (&g.mask, g.acc, g.byte) {
                (_, _, Some(_)) | (None, None, None) => None,
                (_, Some(j), None) => Some(format!("acc_{j}{}", if accs[j].here == "false" { String::new() } else { format!(" || here_{j}") })),
                (Some(m), None, None) => Some(format!("any_in(&{m}, from, s)")),
            };
            if let Some(since) = untested {
                let _ = write!(gap_tested, "                // `{}`\n                if ({since}){unless} {{\n                    marks |= 1 << {};\n                }}\n", g.name, g.bit);
            }
            let _ = write!(gap_code, "                // `{}`\n                if ({since}){unless} {{\n                    marks |= 1 << {};\n                }}\n", g.name, g.bit);
            match g.byte {
                Some(b) => {
                    let _ = write!(gap_asked, "                // `{}`\n                // Its gap's byte tier 2 asks (`gap`): here only the token tests' hits.\n                if (tested >> {1} & 1 == 1){unless} {{\n                    marks |= 1 << {1};\n                }}\n", g.name, g.bit);
                    asked.push((g.bit, b, unless.clone(), g.name.clone()));
                }
                None => {
                    let _ = write!(gap_asked, "                // `{}`\n                if ({since}){unless} {{\n                    marks |= 1 << {};\n                }}\n", g.name, g.bit);
                }
            }
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
        // The same with only the marks tier 2 reads (`tier2_reads`), and the bits of the others.
        let mut token_read = String::new();
        let mut unread = 0u32;
        let tests_of = |code: &mut String, read: &mut String, unread: &mut u32, test: &str, bit: u32| {
            let line = format!("        if {test} {{\n            marks |= 1 << {bit};\n        }}\n");
            code.push_str(&line);
            if x.tier2_reads.is_none_or(|r| r >> bit & 1 == 1) {
                read.push_str(&line);
            } else {
                *unread |= 1 << bit;
            }
        };
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
            tests_of(&mut token_code, &mut token_read, &mut unread, &test, u32::from(*bit));
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
            let _ = writeln!(fw_decl, "    // `{}` for tier 2, written over each region token.\n    let mut {} = vec![0u64; nw];", fw.parts.first().map_or("", |p| p.why.as_str()).trim_start_matches('`').split('`').next().unwrap_or(""), fw.var);
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
                tests_of(&mut token_code, &mut token_read, &mut unread, test, *bit);
            }
        }
        open_code.push_str(GROUP_LINE);
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
            // A row that resumes a string's record goes on to the next token before the record reads `here`.
            if picks.fused && arm.resume.is_none() {
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
        let branches = format!("{{\n            // The row: the first whose start holds, of those whose start may be the byte's.\n            let row = match group {{\n{}                _ => usize::MAX,\n            }};\n{nest_step}            match row {{\n{}                _ => {{\n                    // A byte no row takes: its group's tests, none of which holds.\n                    refused = true;\n                    break;\n                }}\n            }}\n        }}", picks.rows, row_arms.iter().enumerate().map(|(i, a)| format!("                {i} => {},\n", indent(&match &a.resume {
            // The body up to its record's tuple, then the resume.
            // Not fused, a row's bytes leave `code_l` after the row (`acc_after`), which a resume's
            // own `continue` passes by: it clears them itself (decision 0699).
            Some(r) => format!("{}{}{r}", a.body.rsplit_once("            (l, ").map_or(a.body.as_str(), |x| x.0), if !picks.fused && !self.code_after.is_empty() { "            clear(&mut code_l, s, l);\n" } else { "" }),
            None if a.open => {
                let or_none = |x: String| if x == "0" || x == "0x0" { x } else { format!("if open_bad {{ 0 }} else {{ {x} }}") };
                format!("{}{}, {})\n        }}", a.body, or_none(format!("{:#x}", a.bits)), or_none(if a.word { WORD_READ.to_owned() } else { "0".to_owned() }))
            }
            None => format!("{}{:#x}, {})\n        }}", a.body, a.bits, if a.word { WORD_READ } else { "0" }),
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
            let _ = write!(span_after, "    for (c, v) in code_l.iter_mut().zip(&lv) {{\n        *c &= v;\n    }}\n    #[cfg(feature = \"test-hooks\")]\n    if !refused {{\n        if let Some(q) = (0..nw).find(|&q| code_l[q] != {code}[q]) {{\n            panic!(\"code_l is not the regions' code in word {{q}} ({{:#x}} against {{:#x}}) of {{:?}}\", code_l[q], {code}[q], String::from_utf8_lossy(src));\n        }}\n    }}\n{}", self.code_after);
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
                    for (j, l) in live.iter_mut().enumerate() {
                        if !*l && reads(&text, j) {
                            *l = true;
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
        out.push_str("// Generated by native/crates/compiler/src/bitlex_lex_rust.rs; do not edit. The Bitlex compiler's Rust from the grammar's lexer statements.\n\n");
        if fields.is_empty() {
            // No region tier 2 reads in parts (decision 0619, as 0586's): no mask for it.
            out.push_str("/// A token's record (the lean lexer's `Tok`): its first and last byte, and its code (its kind, its word and its marks).\n#[derive(Clone, Copy)]\npub struct Rec {\n    pub start: u32,\n    pub last: u32,\n    pub code: u32,\n}\n\n/// Tier 1's records, one a grammar token; the comments (the extras) apart, as the lean lexer keeps its comments; and\n/// whether the file goes to tree-sitter.\npub struct Lexed {\n    pub recs: Vec<Rec>,\n    pub extras: Vec<Rec>,\n    pub refused: bool,\n}\n\n");
        } else {
            out.push_str("/// A token's record (the lean lexer's `Tok`): its first and last byte, and its code (its kind, its word and its marks).\n#[derive(Clone, Copy)]\npub struct Rec {\n    pub start: u32,\n    pub last: u32,\n    pub code: u32,\n}\n\n/// The masks tier 2 reads, a bit a byte: those it splits a region by, where it reads one in parts.\npub struct Masks {\n");
            for f in &fields {
                let _ = writeln!(out, "    pub {f}: Vec<u64>,");
            }
            out.push_str("}\n\n/// Tier 1's records, one a grammar token; the comments (the extras) apart, as the lean lexer keeps its comments; the\n/// masks tier 2 reads; and whether the file goes to tree-sitter.\npub struct Lexed {\n    pub recs: Vec<Rec>,\n    pub extras: Vec<Rec>,\n    pub masks: Masks,\n    pub refused: bool,\n}\n\n");
        }
        let kinds: Vec<String> = x.kinds.iter().map(|k| k.to_string()).collect();
        let _ = write!(out, "/// Each byte's record kind, as a token's first byte.\nconst KINDS: [u8; 256] = [{}];\n\n", kinds.join(", "));
        let mut words = String::new();
        for (i, w) in x.words.iter().enumerate() {
            let _ = writeln!(words, "        b\"{}\" => {},", lit(w), i + 1);
        }
        let word_fn = format!("/// A token's word: the code of the word its bytes are, 0 for none.\nfn word(b: &[u8]) -> u32 {{\n    match b {{\n{words}        _ => 0,\n    }}\n}}\n\n");
        out.push_str(&word_fn);
        out.push_str(&pair_fns);
        let s1 = self.stage1_parts(fused);
        // Stage 1 as a function of its own, for the test build's walk: none where the loop reads the bytes (below).
        let mut alone_text = String::new();
        if fused {
            let alone = self.stage1_parts(false);
            // Stage 1 as a function of its own too, for the test build's walk alone.
            let test = "#[cfg(feature = \"test-hooks\")]\n";
            // Its nibble tables are the loop's too.
            out.push_str(&alone.tables);
            let _ = write!(
                alone_text,
                "{test}{}{test}{}{}    let mut refused = false;\n    for t in 0..{} {{\n{}    }}\n{}{}",
                alone.record,
                alone.header,
                alone.decls,
                alone.steps,
                alone.body,
                alone.finals,
                alone.ret.replacen("rerail_simd8::dispatch!", &format!("{test}rerail_simd8::dispatch!"), 1)
            );
            out.push_str(&alone_text);
        } else {
            out.push_str(&self.stage1());
        }
        out.push_str(&tables);
        if fused {
            // Its head once the loop's form is known (`from_bytes`, below).
            out.push_str(LEX_HEAD);
        } else {
            out.push_str(&self.lex_head());
        }
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
        if !fused {
            let _ = writeln!(out, "    // Stage 1: the masks the regions and the tokens read, and whether an escape holds.\n    let Stage1 {{ {}, refused: escapes }} = stage1_best(src);", kept.join(", "));
        }
        let rest = format!("{plain_decl_text}{fw_decl}{open_code}{branches}{token_code}{own_code}{gap_code}{acc_after}{span_after}{fields_init}");
        // A refusal in the token loop leaves both its loops.
        let tokens = |t: &str| t.replace("break;\n", "break 'tokens;\n");
        let (open_code, branches, token_code, own_code, gap_code, acc_after) = (tokens(&open_code), tokens(&branches), tokens(&token_code), tokens(&own_code), tokens(&gap_code), tokens(&acc_after));
        // A token's row and record kind by one load (decision 0623 step 3, the lean lexer's `TOKEN`): its row where its first
        // byte picks one, else the arm of its group's tests. Where the rows' pick makes no region of its own.
        let token_form = picks.by_byte.as_ref().filter(|_| (self.opens.is_none() || picks.fused) && x.words.len() < 255).and_then(|(by, arms)| {
            let head = format!("let row = match group {{\n{}                _ => usize::MAX,\n            }};", tokens(&picks.rows));
            let pick = format!("let row = match pick {{\n                p if usize::from(p) < {} => usize::from(p),\n{}                _ => usize::MAX,\n            }};", row_arms.len(), tokens(arms));
            let one = "        // The token's row (or its first byte's group's tests) and its record kind, by one load (the lean lexer's `TOKEN`).\n        let (pick, kind1, word1) = TOKEN[usize::from(src[s])];\n";
            (open_code.contains(GROUP_LINE) && branches.contains(&head)).then(|| (open_code.replacen(GROUP_LINE, one, 1), branches.replacen(&head, &pick, 1).replace("u32::from(KINDS[usize::from(src[s])])", "u32::from(kind1)"), *by))
        });
        // The one pass with its masks read from the bytes where the loop reads them (decision 0623 step 1), as the lean
        // lexer's window: no stage 1, no mask of the whole file, and so no block to wait for. Where stage 1 tests
        // nothing of its own and every mask the loop reads has that form.
        // Tier 2 asks the marks' token tests where it reads a mark (decision 0655 step 6), as the lean parser asks what it
        // reads of a token: where the loop reads the bytes alone, each byte's kind is `TOKEN`'s, an `else escape` reads
        // no gap mask, and a token's row is known by its first byte (no two grammar rows with tests share one).
        let asking = token_form.is_some() && readers(x.stmts).is_empty() && !test_asks.is_empty() && !unplaced_masked && {
            let tested: Vec<&(String, bool, u32, [bool; 256], String)> = row_ids.iter().filter(|r| !r.1 && test_asks.iter().any(|a| a.2 == r.0)).collect();
            tested.iter().enumerate().all(|(i, a)| tested[i + 1..].iter().all(|b| (0..256).all(|c| !(a.3[c] && b.3[c]))))
        };
        // The loop in the lean lexer's shape (decision 0655 steps 4, 5): a block's token starts at once, the common tokens
        // (a word's run, an operator) before each rare start written by one load, and only the rare ones read by their
        // rows' `match`. Where tier 2 asks the marks (the loop keeps no gap state) and the loop keeps nothing else between
        // tokens: the rows each common, their rare starts' bytes, the run's mask, the pairs' rule.
        let lean_rows: Option<(Vec<bool>, [bool; 256], String, String)> = (|| {
            let (_, _, by) = token_form.as_ref()?;
            if !asking || [&token_read, &own_code, &gap_tested, &acc_after].iter().any(|c| !c.trim().is_empty()) || !plain_decl_text.is_empty() || !fw_decl.is_empty() || !span_after.is_empty() || !fields.is_empty() {
                return None;
            }
            // A row of one byte that records no word is common too, where none of its bytes is a word alone: the load's
            // word is none at them (the lean lexer's `BAD` of one byte, by its `TOKEN`).
            let wordless = |i: usize| row_shapes[i].as_deref() == Some("one") && (0..256).all(|c| !row_arms[i].first[c] || !x.words.iter().any(|w| w.as_slice() == [c as u8]));
            let mut common: Vec<bool> = row_arms.iter().enumerate().map(|(i, a)| row_shapes[i].is_some() && !row_ids[i].1 && a.bits == 0 && (a.word || wordless(i)) && !a.open && a.resume.is_none() && a.starts.is_empty()).collect();
            // One run's mask; one pairs' rule, else one-byte rows.
            // A word's run holds its first byte.
            let run = (0..common.len()).find(|&i| common[i] && row_shapes[i].as_deref().is_some_and(|s| s.starts_with("run ")) && (0..256).all(|c| !row_arms[i].first[c] || row_arms[i].tail[c]))?;
            let w = row_shapes[run].as_deref()?.strip_prefix("run ")?.to_owned();
            let pair = (0..common.len()).find(|&i| common[i] && row_shapes[i].as_deref().is_some_and(|s| s.starts_with("pair")));
            for i in 0..common.len() {
                let s = row_shapes[i].as_deref().unwrap_or_default();
                if common[i] && ((s.starts_with("run ") && i != run) || (s.starts_with("pair") && Some(i) != pair) || (s == "one" && pair.is_some_and(|p| (0..256).any(|c| row_arms[i].first[c] && row_arms[p].first[c])))) {
                    common[i] = false;
                }
            }
            // No other common token holds a byte of a word.
            let word = row_arms[run].tail;
            for i in 0..common.len() {
                if common[i] && i != run && (0..256).any(|c| (row_arms[i].first[c] || (row_shapes[i].as_deref() != Some("one") && row_arms[i].tail[c])) && word[c]) {
                    common[i] = false;
                }
            }
            let pair = match pair.filter(|&i| common[i]) {
                Some(i) => row_shapes[i].clone()?,
                None => "none".to_owned(),
            };
            let rare: [bool; 256] = std::array::from_fn(|c| !self.skipped[c] && !(usize::from(by[c]) < common.len() && common[usize::from(by[c])]));
            Some((common, rare, w, pair))
        })();
        // In the lean lexer's shape, a test its row's read decides is its record's bit, not tier 2's to ask (`once_hits`),
        // where tier 2 asks others still.
        if lean_rows.is_some() && test_asks.iter().any(|a| !once_asks.iter().any(|(k, b)| a.2 == *k && a.0 == *b && !a.1)) {
            test_asks.retain(|a| !once_asks.iter().any(|(k, b)| a.2 == *k && a.0 == *b && !a.1));
        } else {
            once_asks.clear();
        }
        let from_bytes = if fused && self.stage1_parts(true).finals.trim().is_empty() && !self.body.lines().any(|l| l.trim_start().starts_with("refused |=")) {
            let locals: HashMap<String, Expr> = self.nodes.iter().filter_map(|n| n.expr.clone().map(|e| (n.name.clone(), e))).collect();
            // Each class stage 1 writes, by the nibble tables the file holds (`stage1_codes`).
            let (live, _, codes) = self.stage1_codes();
            let class_code: HashMap<String, String> = self.nodes.iter().zip(&live).zip(&codes).filter(|((n, l), _)| **l && n.class.is_some()).filter_map(|((n, _), c)| n.what.strip_prefix("class ").map(|w| (w.to_owned(), c.clone()))).collect();
            let mut by = bytes::Bytes::new(x.stmts, locals, class_code);
            by.row_firsts = row_ids.iter().zip(&row_arms).map(|(r, a)| (r.0.clone(), a.first)).collect();
            let firsts: Vec<[bool; 256]> = row_arms.iter().map(|a| a.first).collect();
            let ask = if asking { token_tests_fn(&test_asks, &row_ids, &mark_names) } else { None };
            let mut go = || -> Result<_, String> {
                let mut pieces = Vec::new();
                // An operator's last byte by its bit in the block's pairs mask (`k_pairs_K`).
                let pairs_read = |t: &str| pair_rows.iter().fold(t.to_owned(), |t, (f, _)| t.replace(&format!("{f}(src, s)"), &format!("s + (k_{f} >> (s - base) & 1) as usize")));
                let alone = readers(x.stmts).is_empty();
                let (open_t, branches_t) = match &token_form {
                    Some((o, b, _)) => (o, b),
                    None => (&open_code, &branches),
                };
                // Where tier 2 asks the token tests (decision 0655 step 6), the loop makes no hits of them.
                let (branches_t, gap_t) = match &ask {
                    Some(_) => (&no_hits(branches_t), &gap_tested),
                    None => (branches_t, if alone { &gap_asked } else { &gap_code }),
                };
                // In the lean lexer's shape (decision 0655 steps 4, 5): the rows' `match` reads the rare tokens only (and the
                // common rows a group's tests pick), each read of the bytes from its first (its windows, its bytes), not of
                // the block's masks.
                // Each read of a mask in the lean lexer's form: its micro-lexer, or the bytes in a loop (decision 0655 step 7).
                by.lean = lean_rows.is_some();
                let branches_t = &match &lean_rows {
                    Some((common, ..)) => {
                        let picked = group_rows(branches_t);
                        once_hits(&without_rows(branches_t, &(0..common.len()).filter(|&i| common[i] && !picked.contains(&i)).collect::<Vec<_>>()), &once_asks, &mark_names)?
                    }
                    None => branches_t.clone(),
                };
                for (i, t) in [open_t, branches_t, if alone { &token_read } else { &token_code }, &own_code, gap_t, &acc_after].into_iter().enumerate() {
                    let t = bytes::strip_scans(&bytes::strip_pauses(&pairs_read(t)));
                    let t = if i == 1 && lean_rows.is_some() { by.rewrite_windows(&t)? } else { by.rewrite(&t)? };
                    let t = bytes::fuse_tests(&t, &by.escapes);
                    if uses(&t, "made") || uses(&t, "wait") || t.contains("'pass") {
                        return Err("a pause the loop still reads".into());
                    }
                    pieces.push(t);
                }
                // A row's region end and its tests in one window loop, where its first bytes start no escape.
                pieces[1] = bytes::fuse_read(&pieces[1], &firsts, &by.escapes, &by.escape_bytes);
                // The token tests tier 2 asks: each a test of the windows from the token's first byte.
                pieces.push(match &ask {
                    Some(a) => by.rewrite_windows(a)?,
                    None => String::new(),
                });
                let starts = by.starts(&skip)?;
                // The lean lexer's loop: its four masks, the bytes `skip` holds, a word's, a rare token's first, a pair's.
                pieces.push(match &lean_rows {
                    Some((_, rare, w, pair)) => {
                        let space = starts.strip_prefix('!').ok_or("the starts are no mask's complement")?.to_owned();
                        let kw = by.block_word(w);
                        let wfn = by.byte_test(w)?;
                        let kp = match pair.as_str() {
                            "none" => "0u64".to_owned(),
                            p if p.starts_with("pairs ") => format!("k_{}", &p[6..]),
                            p => by.block_word(&p[5..]),
                        };
                        by.rare = Some(*rare);
                        // (Each arm's `,` and the match's `;` on the line they end.)
                        let branches = indent(&pieces[1], "    ").replace("}\n    ,\n", "},\n").replace("}\n,\n", "},\n");
                        lean_loop(&space, &kw, &wfn, &kp, &indent(&pieces[0], "    "), branches.trim())
                    }
                    None => String::new(),
                });
                // The pairs masks the loop reads: of the block's classes, as its other masks (`Bytes::pairs`).
                by.pairs = pair_rows.iter().filter(|(f, _)| pieces.iter().any(|q| q.contains(&format!("k_{f}")))).cloned().collect();
                let (decl, words) = by.block_words();
                let head = if pieces[7].is_empty() { format!("{words}        let mut firsts = {starts}") } else { words };
                Ok((pieces, head, decl))
            };
            go().ok().map(|(p, st, decl)| (p, st, by.code(), decl))
        } else {
            None
        };
        let (open_code, branches, token_code, own_code, gap_code, acc_after) = match &from_bytes {
            Some((p, ..)) => (p[0].clone(), p[1].clone(), p[2].clone(), p[3].clone(), p[4].clone(), p[5].clone()),
            None => (open_code, branches, token_code, own_code, gap_code, acc_after),
        };
        let walk = gate_walk(&self.body, &rest);
        // The counts' words over the tokens.
        // No count where the loop reads the bytes (`strip_scans`).
        let scan_decl: String = if from_bytes.is_some() { String::new() } else { token_scans.iter().map(|(name, ..)| format!("    let mut c_{} = 0u32;\n", ident(name))).collect() };
        if fused {
            let head = if from_bytes.is_some() {
                String::from("/// `src`'s records, in one pass over its blocks (decisions 0619, 0623): each block's token starts from its 64 bytes, and\n/// each mask a token reads from the bytes where it reads it, as the lean lexer reads its window's.\n#[inline(always)]\nunsafe fn lex_on<S: rerail_simd8::Simd8x64>(src: &[u8], n: usize) -> Lexed {\n    let nw = n.div_ceil(64);\n    let mut refused = false;\n")
            } else {
                format!(
                    "/// `src`'s records, in one pass over its blocks (decision 0619): stage 1 makes each block's masks as the token loop\n/// comes to need them, and the token loop reads its tokens from them, as the lean lexer's `lex_blocks` reads its\n/// blocks' masks.\n#[inline(always)]\nunsafe fn lex_on<S: rerail_simd8::Simd8x64>(src: &[u8], n: usize) -> Lexed {{\n    let nw = n.div_ceil(64);\n{}    let mut refused = false;\n    // Stage 1's next step, the blocks whose masks it has made, and those the token loop waits for.\n    let mut t = 0usize;\n    let mut made = 0usize;\n    let mut wait = nw.min(1);\n",
                    s1.decls
                )
            };
            out = out.replacen(LEX_HEAD, &head, 1);
        }
        // A token's word from one load (decision 0623 step 4): the padded source's, or one that stops at the file's end.
        let word_table = bytes::word_table(x.words, from_bytes.is_some());
        let (walk, loop_head, pass_end) = if fused && from_bytes.is_some() {
            // No stage 1, and no walk of the regions: the lean lexer's loop holds no checker. The tests hold the records
            // to the interpreter's (decision 0655 step 3).
            let decl = from_bytes.as_ref().map_or("", |f| f.3.as_str());
            (String::new(), format!("{decl}    'tokens: while b < nw {{\n"), "")
        } else if fused {
            // Stage 1 to the blocks the token loop waits for (every one, once it waits for the file's end), then its
            // tests once after its last step; a test build's walk; then the token loop, to a token in a block not made.
            let walk = walk.replacen("    let mut refused = escapes;\n", "", 1);
            let (keep, walk) = match walk.find("    // The regions' walk") {
                Some(k) => walk.split_at(k),
                None => (walk.as_str(), ""),
            };
            let keep = if keep.trim().is_empty() { String::new() } else { format!("        // The escapes after stage 1, once it has made every block.\n        if made == nw {{\n{}        }}\n", indent(keep, "        ")) };
            // The test build's walk, of its own stage 1 over the file: the pass reads it only in its checks.
            let walk = walk.replacen(
                " = {\n",
                &format!(" = {{\n        let Stage1 {{ {}, refused: _ }} = stage1_best(&src[..n]);\n        // Each word's bits inside the file.\n        let lv: Vec<u64> = (0..nw).map(|q| if 64 * q + 64 <= n {{ !0 }} else {{ (1u64 << (n % 64)) - 1 }}).collect();\n", kept.join(", ")),
                1,
            );
            let steps = &s1.steps;
            let stage = format!(
                "{walk}    'pass: loop {{\n        // Stage 1, to the blocks the token loop waits for: every one where it waits for the file's end.\n        while t < {steps} && (made < wait || wait == nw) {{\n{}            t += 1;\n            made = t.saturating_sub({}).min(nw);\n        }}\n        if t == {steps} {{\n{}        }}\n{keep}",
                indent(&s1.body, "    "),
                s1.most,
                indent(&s1.finals, "        ")
            );
            (stage, "    'tokens: loop {\n        if b >= made {\n            if made < nw {\n                // A block stage 1 has not made: it makes it first.\n                wait = (b + 1).min(nw);\n                continue 'pass;\n            }\n            break 'tokens;\n        }\n".to_owned(), "    break 'pass;\n    }\n")
        } else {
            (walk, "    'tokens: while b < nw {\n".to_owned(), "")
        };
        if !fused {
            out.push_str(&walk);
        }
        let lean = from_bytes.as_ref().map_or("", |f| f.0[7].as_str());
        if !lean.is_empty() {
            let _ = write!(
                out,
                "    // The tokens: each block's starts at once, the lean lexer's `lex_blocks`.\n    // Room for a record a byte, as the lean lexer's: each token's first byte is after the last's, so the writes need no check.\n    let mut recs: Vec<Rec> = Vec::with_capacity(n + 1);\n    let out = recs.as_mut_ptr();\n    // The comments, apart from the tokens, as the lean lexer's `comments`.\n    let mut extras: Vec<Rec> = Vec::new();\n    let mut len = 0usize;\n    // No token starts before `at`.\n    let mut at = 0usize;\n    let mut b = 0usize;\n{loop_head}        let base = b * 64;\n{words}{lean}        b += 1;\n    }}\n    // SAFETY: `len` records written.\n    unsafe {{ recs.set_len(len) }};\n    Lexed {{\n        recs,\n        extras,\n        refused,\n    }}\n}}\n",
                words = from_bytes.as_ref().map_or(String::new(), |f| f.1.clone()),
            );
            out.push_str(LEFT_TO_RIGHT);
        } else {
        let _ = write!(
            out,
            "    // The tokens: from the first byte `skip` does not hold, the token whose start holds there, to its last byte by its rule.\n    // Room for a record a byte, as the lean lexer's: each token's first byte is after the last's, so the writes need no check.\n    let mut recs: Vec<Rec> = Vec::with_capacity(n + 1);\n    let out = recs.as_mut_ptr();\n    // The comments, apart from the tokens, as the lean lexer's `comments`.\n    let mut extras: Vec<Rec> = Vec::new();\n    let mut len = 0usize;\n    let mut at = 0usize;\n    // The gap's start, since the grammar token before and since the record before; the grammar token before's word; the marks its tests carry to the next grammar token.\n    let (mut from, mut from_record, mut before, mut pending) = (0usize, 0usize, 0u32, 0u32);\n{plain_decl}{fw_decl}{scan_decl}    // The blocks of 64 bytes, and in each the bytes `skip` does not hold from the last token's end on: each a token's first byte (the lean lexer's `lex_blocks`).\n    let mut b = at / 64;\n{stage}{loop_head}        let base = b * 64;\n{block_starts} & (!0u64 << at.saturating_sub(base));\n        while firsts != 0 {{\n        let s = base + firsts.trailing_zeros() as usize;\n        if s >= n {{\n            break 'tokens;\n        }}\n{open_code}        let mut hits = 0u32;\n        let (l, kind, extra, mut marks, word): (usize, u32, bool, u32, u32) = {branches};\n        // The token marks: tier 2 tests the rest of each.\n{token_code}        if extra {{\n            // An extra keeps the marks of its own gap; its tests' hits go to the next grammar token.\n{own}            pending |= hits & 0xffff | hits >> 16;\n        }} else {{\n            let tested = pending | hits;\n{gap_code}            from = s + 1;\n            before = word;\n            pending = hits >> 16;\n        }}\n        from_record = s + 1;\n{acc_after}        let rec = Rec {{ start: s as u32, last: l as u32, code: kind | word << 8 | marks << 16 }};\n        if extra {{\n            extras.push(rec);\n        }} else {{\n            debug_assert!(len < n && l >= s, \"a token a byte at most\");\n            // SAFETY: `len < n`, the room reserved: a record's first byte is after the record before's last.\n            unsafe {{ out.add(len).write(rec) }};\n            len += 1;\n        }}\n        at = l + 1;\n        if at >= base + 64 {{\n            b = at / 64;\n            continue 'tokens;\n        }}\n        firsts &= !0u64 << (at - base);\n        }}\n        b += 1;\n    }}\n{pass_end}{unplaced_code}    let _ = (from, from_record, before, pending);\n{span_after}    // SAFETY: `len` records written.\n    unsafe {{ recs.set_len(len) }};\n    Lexed {{\n        recs,\n        extras,\n{masks_init}        refused,\n    }}\n}}\n",
            own = own_code,
            plain_decl = plain_decl_text,
            stage = if fused { walk.as_str() } else { "" },
            block_starts = match &from_bytes {
                Some((_, st, ..)) => st.clone(),
                None => format!("        let mut firsts = !{skip}[b]"),
            },
            masks_init = if fields.is_empty() { String::new() } else { format!("        masks: Masks {{\n{fields_init}        }},\n") },
        );
        }
        if from_bytes.is_some() {
            out = out.replacen(&alone_text, "", 1);
            out = no_test_checks(&out);
        }
        // Where tier 2 asks the token tests: no hits, nothing carried to the next grammar token; an `else escape` asks
        // them at the file's end.
        let asked_tests = from_bytes.as_ref().is_some_and(|f| !f.0[6].is_empty());
        if asked_tests {
            for (from, to) in [
                ("    // The gap's start, since the grammar token before and since the record before; the grammar token before's word; the marks its tests carry to the next grammar token.\n    let (mut from, mut from_record, mut before, mut pending) = (0usize, 0usize, 0u32, 0u32);\n", "    // The gap's start, since the grammar token before and since the record before; the grammar token before's word.\n    let (mut from, mut from_record, mut before) = (0usize, 0usize, 0u32);\n"),
                ("        let mut hits = 0u32;\n", ""),
                ("        // The token marks: tier 2 tests the rest of each.\n", "        // The token marks: tier 2 asks the rest of each, and the marks' token tests (`tested`).\n"),
                ("            // An extra keeps the marks of its own gap; its tests' hits go to the next grammar token.\n", "            // An extra keeps the marks of its own gap.\n"),
                ("            pending |= hits & 0xffff | hits >> 16;\n", ""),
                ("            let tested = pending | hits;\n", ""),
                ("            pending = hits >> 16;\n", ""),
                ("    let _ = (from, from_record, before, pending);\n", "    let _ = (from, from_record, before);\n"),
            ] {
                out = out.replacen(from, to, 1);
            }
            if !unplaced_code.is_empty() {
                out = out.replacen(&unplaced_code, "", 1);
                let mut end = String::new();
                for (bit, kind, why) in &unplaced_asks {
                    let _ = write!(end, "    // `{kind}`, else escape: {why}\n    if tested(&src[..n], &recs, &extras, len, {bit}) {{\n        refused = true;\n    }}\n");
                }
                out = out.replacen("    // SAFETY: `len` records written.\n    unsafe { recs.set_len(len) };\n", &format!("    // SAFETY: `len` records written.\n    unsafe {{ recs.set_len(len) }};\n{end}"), 1);
            }
            let body = &out[out.find("unsafe fn lex_on<").unwrap_or(0)..];
            if !body[..body.find("\n}\n").unwrap_or(body.len())].contains("marks |=") {
                out = out.replacen("let (l, kind, extra, mut marks, word)", "let (l, kind, extra, marks, word)", 1);
            }
            // The gap's start and the word before, where nothing in the loop reads them: the lean lexer keeps neither.
            let kept = [
                "    // The gap's start, since the grammar token before and since the record before; the grammar token before's word.\n    let (mut from, mut from_record, mut before) = (0usize, 0usize, 0u32);\n",
                "        // The token marks: tier 2 asks the rest of each, and the marks' token tests (`tested`).\n        if extra {\n            // An extra keeps the marks of its own gap.\n        } else {\n            from = s + 1;\n            before = word;\n        }\n        from_record = s + 1;\n",
                "    let _ = (from, from_record, before);\n",
            ];
            if kept.iter().all(|k| out.contains(k)) {
                let none = kept.iter().fold(out.clone(), |o, k| o.replacen(k, "", 1));
                let at = none.find("unsafe fn lex_on<").unwrap_or(0);
                let body = &none[at..];
                // Its code, not its comments.
                let body: String = body[..body.find("\n}\n").unwrap_or(body.len())].lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n").replace("::from(", "");
                if !["from", "from_record", "before"].iter().any(|v| uses(&body, v)) {
                    out = none;
                }
            }
        }
        if fused {
            out.push_str("\nrerail_simd8::dispatch! {\n    fn lex_best(src: &[u8], n: usize) -> Lexed = lex_on, on lex_on_best;\n}\n\n/// The spaces after a file `lex` reads, as simdjson's `SIMDJSON_PADDING`: each block of it is loaded where it is.\npub const PADDING: usize = 64;\n\n/// The records of `buf[..n]`, its `PADDING` bytes after it spaces.\npub fn lex(buf: &[u8], n: usize) -> Lexed {\n    assert!(buf.len() >= n + PADDING && buf[n..n + PADDING].chunks_exact(8).all(|w| w == b\"        \"), \"the source padded with spaces\");\n    lex_best(buf, n)\n}\n");
        }
        if let Some((_, _, code, _)) = &from_bytes {
            out.push_str(code);
            for (f, two) in &pair_rows {
                if !out.contains(&format!("k_{f} >>")) {
                    continue;
                }
                // Each pair's two tests and the pairs joined in the vectors, then one mask (lean's `pair`, of its one classify).
                let pairs: Vec<String> = two.iter().map(|(a, b)| format!("S::and(S::eq(v, S::splat({a:#04x})), S::eq(w, S::splat({b:#04x})))")).collect();
                let joined = pairs.iter().skip(1).fold(pairs.first().cloned().unwrap_or_default(), |acc, p| format!("S::or({acc},\n            {p})"));
                let test = [format!("S::to_bitmask({joined})")];
                // Its own loads only where the block's classes are not made (`Bytes::block_words`).
                if out.contains(&format!("w_{f}::<S>")) {
                    let _ = write!(out, "\n/// `{f}`'s pairs' first bytes in the block at `base`, as the lean lexer's `pair`: where a pair's two bytes are. A token\n/// never starts at a pair's second byte, so at a token's start this is the grammar's mask of pairs (read left to right).\n///\n/// # Safety\n/// The CPU runs `S`'s backend.\n#[inline(always)]\nunsafe fn w_{f}<S: rerail_simd8::Simd8x64>(src: &[u8], base: usize) -> u64 {{\n    // The block and the one a byte on: in the padded source.\n    let at: &[u8; 64] = src[base..base + 64].try_into().expect(\"the padded source\");\n    let on: &[u8; 64] = src[base + 1..base + 65].try_into().expect(\"the padded source\");\n    // SAFETY: the caller's.\n    unsafe {{\n        let (v, w) = (S::load(at), S::load(on));\n        {}\n    }}\n}}\n", test.join("\n            | "));
                }
                // The function the mask replaces, where nothing calls it.
                if !out.contains(&format!("{f}(src, s)"))
                    && let Some(at) = out.find(&format!("`'s last byte from its first, `s`: its next where the two are a pair, and the one after where the three are a triple, as the lean lexer reads an operator.\nfn {f}("))
                {
                    let start = out[..at].rfind("/// `").unwrap_or(at);
                    let end = at + out[at..].find("\n}\n\n").map_or(0, |e| e + 4);
                    out.replace_range(start..end, "");
                }
            }
            if let Some((_, _, by)) = &token_form {
                let one = |c: usize| x.words.iter().position(|w| w.as_slice() == [c as u8]).map_or(0, |i| i + 1);
                let mut rows: Vec<(u8, u8, u8)> = (0..256).map(|c| (by[c], x.kinds[c], one(c) as u8)).collect();
                // A pair's entry (`256 + c`, `c` its first byte): its word by the load, as the lean lexer's `TOKEN[c +
                // 256]`. Where a first byte starts two pairs, the second byte picks (`+ 512`), as lean's `<:`.
                let mut alt = String::new();
                if out.contains("/*PAIR_ALT ") {
                    let pairs: Vec<(u8, u8)> = pair_rows.iter().flat_map(|(_, two)| two.iter().copied()).collect();
                    let code = |a: u8, b: u8| x.words.iter().position(|w| w.as_slice() == [a, b]).map_or(0, |i| i + 1) as u8;
                    // The second bytes that pick `+ 512`: the second pair's of each first byte that starts two.
                    let mut seconds: Vec<u8> = Vec::new();
                    for &(a, b) in &pairs {
                        if pairs.iter().take_while(|&&q| q != (a, b)).any(|q| q.0 == a) {
                            seconds.push(b);
                        }
                    }
                    let picks = |a: u8| pairs.iter().filter(|q| q.0 == a).map(|q| seconds.contains(&q.1)).collect::<Vec<_>>();
                    let ok = pairs.iter().all(|&(a, _)| {
                        let p = picks(a);
                        p.len() <= 2 && (p.len() < 2 || p[0] != p[1])
                    });
                    if !ok {
                        return Err("a first byte's pairs the second byte does not pick apart".into());
                    }
                    if !seconds.is_empty() {
                        rows.resize(1024, (u8::MAX, 0, 0));
                        let test = if seconds.len() == 1 { format!("src[s + 1] == {}", bytes::lit(seconds[0])) } else { format!("matches!(src[s + 1], {})", seconds.iter().map(|&b| bytes::lit(b)).collect::<Vec<_>>().join(" | ")) };
                        alt = format!(" | ({{p}} & usize::from({test})) << 9");
                    } else {
                        rows.resize(512, (u8::MAX, 0, 0));
                    }
                    for &(a, b) in &pairs {
                        let at = 256 + a as usize + if seconds.contains(&b) { 512 } else { 0 };
                        rows[at] = (by[a as usize], x.kinds[a as usize], code(a, b));
                    }
                }
                // A pair row's arm: its word by the load too, its pair `l - s`.
                if out.contains("/*PAIR_ALT ") {
                    let mut from = 0;
                    while let Some(at) = out[from..].find(" >> (s - base) & 1) as usize;\n").map(|a| a + from) {
                        from = at + 1;
                        let line = out[..at].rfind('\n').map_or(0, |l| l + 1);
                        if !out[line..at].trim_start().starts_with("let l = s + (") {
                            continue;
                        }
                        let end = out[at..].find(" => {").map_or(out.len(), |e| e + at);
                        if let Some(w) = out[at..end].find(WORD_READ).map(|w| w + at) {
                            out.replace_range(w..w + WORD_READ.len(), "u32::from(TOKEN[usize::from(src[s]) | (l - s) << 8/*PAIR_ALT (l - s)*/].2)");
                        }
                    }
                }
                while let Some(at) = out.find("/*PAIR_ALT ") {
                    let end = at + out[at..].find("*/").ok_or("an unclosed pair mark")? + 2;
                    let v = out[at + "/*PAIR_ALT ".len()..end - 2].to_owned();
                    out.replace_range(at..end, &alt.replace("{p}", &v));
                }
                let _ = write!(out, "\n/// Each byte's pick as a token's first byte, its record kind, and the code of the word it is alone (the lean lexer's\n/// `TOKEN`): the token's row where the byte picks one, else {} and its group's index, whose tests pick it; `u8::MAX`,\n/// none.{}\nstatic TOKEN: [(u8, u8, u8); {}] = {};\n", row_arms.len(), if rows.len() > 256 { " A pair's at `256 + c`, `c` its first byte, its code its word's (`+ 512` where its second byte picks it)." } else { "" }, rows.len(), table_block(&rows));
                // The tables the one load replaces, where nothing else reads them.
                if !out.contains("DISPATCH_LO[") {
                    out = out.replacen(&tables, "", 1);
                }
                if !out.contains("KINDS[")
                    && let Some(at) = out.find("/// Each byte's record kind, as a token's first byte.\nconst KINDS")
                {
                    let end = at + out[at..].find("];\n\n").map_or(0, |e| e + 4);
                    out.replace_range(at..end, "");
                }
            }
        }
        if let Some(t) = &word_table {
            // A token of one byte's word by `TOKEN`'s load, as the lean lexer's operators; a longer one's by the table.
            let read = if out.contains("let (pick, kind1, word1) = TOKEN[") { "if l == s { u32::from(word1) } else { word_at(src, s, l + 1 - s) }" } else { "word_at(src, s, l + 1 - s)" };
            out = out.replace(WORD_READ, read);
            out = out.replacen(&word_fn, "", 1);
            out.push_str(t);
        }
        if !out.contains("u32::from(word1)") {
            out = out.replace("let (pick, kind1, word1) = TOKEN[", "let (pick, kind1, _) = TOKEN[");
        }
        // The gap marks tier 2 asks of the bytes (decision 0623 step 5): where the loop reads the bytes, and no region is
        // read in parts (a piece's start is not its record's).
        let alone = from_bytes.is_some() && readers(x.stmts).is_empty();
        let (asked, unread) = if alone { (asked, unread) } else { (Vec::new(), 0) };
        let bits = asked.iter().fold(0u32, |m, a| m | 1 << a.0);
        // The marks whose token tests tier 2 asks (decision 0655 step 6).
        let tested_bits = match &from_bytes {
            Some((p, ..)) if !p[6].is_empty() => test_asks.iter().fold(0u32, |m, a| m | 1 << a.0),
            _ => 0,
        };
        let _ = write!(out, "\n/// The marks tier 1's records leave to tier 2 (decisions 0623, 0655): those it asks of the bytes (`ASKED`), those whose\n/// token tests it asks (`TESTED`), and the token marks its code reads no bit of (it asks them of the tokens).\npub const UNRECORDED: u32 = {:#x};\n", bits | tested_bits | unread);
        match &from_bytes {
            Some((p, ..)) if !p[6].is_empty() => {
                let unless: String = (0..32u8).filter(|b| tested_bits >> b & 1 == 1).filter_map(|b| unless_of.get(&b).map(|u| format!("        {b} => {},\n", u.trim_start_matches(" && ")))).collect();
                let unless = if unless.is_empty() { String::new() } else { format!("    // A mark's `unless after`: none at a token after those words.\n    let before = j.checked_sub(1).map_or(0, |p| recs[p].code >> 8 & 0xff);\n    let unless = match b {{\n{unless}        _ => true,\n    }};\n    if !unless {{\n        return false;\n    }}\n") };
                let _ = write!(
                    out,
                    "\n/// The marks whose token tests tier 2 asks where a rule reads the mark (decision 0655 step 6), as the lean parser asks\n/// what it reads of a token where it reads it: no record holds them.\npub const TESTED: u32 = {tested_bits:#x};\n\n/// Whether token `r` (an extra, where `extra`) holds a token test of mark `b`: one of the token after it (`after`), else\n/// one of its own. Its row by its first byte, its kind and whether it is an extra.\n///\n/// # Safety\n/// The CPU runs `S`'s backend.\n#[inline(always)]\nunsafe fn token_test_on<S: rerail_simd8::Simd8x64>(src: &[u8], n: usize, r: &Rec, extra: bool, b: u32, after: bool) -> bool {{\n    let (s, l, kind) = (r.start as usize, r.last as usize, r.code & 0xff);\n    let _ = (l, kind);\n{}}}\n\nrerail_simd8::dispatch! {{\n    fn token_test(src: &[u8], n: usize, r: &Rec, extra: bool, b: u32, after: bool) -> bool = token_test_on, on token_test_on_best;\n}}\n\n/// Whether a token test of mark `b` holds at grammar token `j` of `recs` (`recs.len()`: the file's end), as tier 1's loop\n/// carried them: `j`'s own, or one of the token after it at the grammar token before or at an extra (`xs`) between.\npub fn tested(src: &[u8], recs: &[Rec], xs: &[Rec], j: usize, b: u32) -> bool {{\n    let n = src.len();\n{unless}    let p = j.checked_sub(1);\n    let (from, to) = (p.map_or(0, |p| recs[p].start + 1), recs.get(j).map_or(u32::MAX, |r| r.start));\n    let k = xs.partition_point(|x| x.start < from);\n    (j < recs.len() && token_test(src, n, &recs[j], false, b, false))\n        || p.is_some_and(|p| token_test(src, n, &recs[p], false, b, true))\n        || xs[k..].iter().take_while(|x| x.start < to).any(|x| token_test(src, n, x, true, b, false) || token_test(src, n, x, true, b, true))\n}}\n",
                    p[6]
                );
            }
            _ => out.push_str("\n/// The marks whose token tests tier 2 asks (decision 0655 step 6): none, the records hold them.\npub const TESTED: u32 = 0;\n\n/// Whether a token test of mark `b` holds at grammar token `j`: never asked.\npub fn tested(src: &[u8], recs: &[Rec], xs: &[Rec], j: usize, b: u32) -> bool {\n    let _ = (src, recs, xs, j, b);\n    false\n}\n"),
        }
        let _ = write!(out, "\n/// The gap marks tier 2 asks of the bytes, where a rule asks one (decision 0623 step 5), by their bits: tier 1's\n/// records hold only its token tests' hits of them.\npub const ASKED: u32 = {bits:#x};\n\n/// Whether gap mark `b` holds over `src[from..to]`, the bytes since the grammar token before's first byte (`before` its\n/// word), as the lean parser asks whether a token is on a line.\n#[inline]\npub fn gap(src: &[u8], from: usize, to: usize, before: u32, b: u32) -> bool {{\n");
        if asked.is_empty() {
            out.push_str("    let _ = (src, from, to, before, b);\n    false\n}\n");
        } else {
            if asked.iter().all(|a| a.2.is_empty()) {
                out.push_str("    let _ = before;\n");
            }
            out.push_str("    match b {\n");
            for (bit, byte, unless, name) in &asked {
                let _ = writeln!(out, "        // `{name}`\n        {bit} => src[from..to].contains(&{byte:#04x}){unless},");
            }
            out.push_str("        _ => false,\n    }\n}\n");
        }
        out.push_str(HELPERS);
        if self.body.contains("toggle(&") {
            out.push_str(TOGGLE);
        }
        // Only what the loop calls, where it reads the bytes (decision 0655 step 3).
        if from_bytes.is_some() {
            out = drop_uncalled_fns(&out);
        }
        // One `match` of the rare tokens' first byte, a group's tests its guards (decision 0655 step 7).
        if let Some(o) = one_pick(&out) {
            out = tidy_lean(&o);
        }
        // The lean lexer's micro-lexers, where a read is one (decision 0655 step 7).
        if out.contains("micro::")
            && let Some(at) = out.find("\n\n")
        {
            out.insert_str(at + 2, "use rerail_simd8::micro;\n\n");
        }
        Ok(out)
    }
}

/// The token loop's line that finds a token's first byte's group (`dispatch`).
const GROUP_LINE: &str = "        // The token's first byte's group, by the dispatch tables (simdjson's `classify`).\n        let c = src[s];\n        let group = (DISPATCH_LO[usize::from(c & 15)] & DISPATCH_HI[usize::from(c >> 4)]).trailing_zeros();\n";

/// Where the one pass's head goes, until its form is known.
const LEX_HEAD: &str = "\u{0}lex_on's head\u{0}";

/// The byte-pair errors' check of block `q` (its bytes `b`, its vector `v`), as simdjson's `utf8_checker::check_next_input`.
fn lookup_code(l: &crate::bitlex_pair_lookup::Lookup) -> String {
    let mut out = String::new();
    out.push_str("            // The byte-pair errors, as simdjson's `utf8_checker`: a block of ASCII holds one only where the block before left a\n            // sequence open.\n            if unsafe { S::to_bitmask(v) } == 0 {\n                error = unsafe { S::or(error, prev_incomplete) };\n            } else {\n");
    out.push_str("                // The byte before's nibbles and this byte's high nibble, each rule a bit (`check_special_cases`).\n                let p1 = unsafe { S::prev(v, prev_input, 1) };\n                let sc = unsafe { S::and(S::and(S::lookup_16(S::high_nibbles(p1), &BYTE_1_HIGH), S::lookup_16(S::low_nibbles(p1), &BYTE_1_LOW)), S::lookup_16(S::high_nibbles(v), &BYTE_2_HIGH)) };\n");
    match l.xor {
        Some((x, y)) => {
            let _ = writeln!(out, "                // A continuation needed 2 or 3 bytes after a lead byte (`must_be_2_3_continuation`), against two in a row (bit 0x80).\n                let must23 = unsafe {{ S::and(S::or(S::ge(S::prev(v, prev_input, 2), {x:#04x}), S::ge(S::prev(v, prev_input, 3), {y:#04x})), S::splat(0x80)) }};\n                error = unsafe {{ S::or(error, S::xor(sc, must23)) }};");
        }
        None => out.push_str("                error = unsafe { S::or(error, sc) };\n"),
    }
    out.push_str("                // A sequence the block's last bytes leave open (`is_incomplete`): the bytes above `MAX_VALUE`'s.\n                prev_incomplete = unsafe { S::gt_bits(v, max_value) };\n            }\n            prev_input = v;\n");
    out
}

/// Whether `e` holds at a byte `b`, where that says (its classes, and the masks of them); `None` where it reads more.
fn at_byte(e: &Expr, b: u8, stmts: &[Stmt]) -> Option<bool> {
    let f = |x: &Expr| at_byte(x, b, stmts);
    match e {
        Expr::Name(n) => stmts.iter().rev().find_map(|s| match s {
            Stmt::Class(c, items) if c == n => Some(Some(items.iter().any(|&(lo, hi)| (lo..=hi).contains(&u32::from(b))))),
            Stmt::Mask(m, x) if m == n => Some(f(x)),
            _ => None,
        })?,
        Expr::Not(x) => f(x).map(|v| !v),
        Expr::And(x, y) => match (f(x), f(y)) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        },
        Expr::Or(x, y) => match (f(x), f(y)) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        },
        Expr::Xor(x, y) => Some(f(x)? ^ f(y)?),
        _ => None,
    }
}

/// The masks a start reads that its byte does not decide: each call (`escaped(B)`, `prev(X)`, …) and each name no
/// statement defines (a region's), by its text.
fn atoms(e: &Expr, stmts: &[Stmt], out: &mut Vec<(String, Expr)>) {
    match e {
        Expr::Name(n) => match stmts.iter().rev().find(|s| matches!(s, Stmt::Class(c, _) | Stmt::Mask(c, _) if c == n)) {
            Some(Stmt::Mask(_, x)) => atoms(x, stmts, out),
            Some(_) => {}
            None => {
                if !out.iter().any(|a| a.0 == *n) {
                    out.push((n.clone(), e.clone()));
                }
            }
        },
        Expr::Call(..) => {
            let k = format!("{e:?}");
            if !out.iter().any(|a| a.0 == k) {
                out.push((k, e.clone()));
            }
        }
        Expr::Not(x) => atoms(x, stmts, out),
        Expr::And(x, y) | Expr::Or(x, y) | Expr::Xor(x, y) => {
            atoms(x, stmts, out);
            atoms(y, stmts, out);
        }
        _ => {}
    }
}

/// Whether `e` is false at byte `b` whatever the masks its byte does not decide hold (`atoms`).
fn false_at(e: &Expr, b: u8, stmts: &[Stmt]) -> bool {
    let mut ats = Vec::new();
    atoms(e, stmts, &mut ats);
    ats.len() <= 16 && (0..1u32 << ats.len()).all(|v| at_atoms(e, b, stmts, &ats, v) == Some(false))
}

/// A mask's code with no `& lv` (the file's bits): `(!x)` left `!x`.
fn without_lv(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(k) = rest.find(" & lv") {
        let after = &rest[k + 5..];
        if after.starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '[') {
            out.push_str(&rest[..k + 5]);
        } else {
            out.push_str(&rest[..k]);
        }
        rest = after;
    }
    out.push_str(rest);
    // `(!name)` is `!name`.
    let mut s = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(k) = rest.find("(!") {
        let tail = &rest[k + 2..];
        let id = tail.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(tail.len());
        if id > 0 && tail[id..].starts_with(')') {
            s.push_str(&rest[..k]);
            s.push('!');
            s.push_str(&tail[..id]);
            rest = &tail[id + 1..];
        } else {
            s.push_str(&rest[..k + 2]);
            rest = tail;
        }
    }
    s.push_str(rest);
    s
}

/// `e` at byte `b`, the atoms valued `v`; `None` where it reads what is neither.
fn at_atoms(e: &Expr, b: u8, stmts: &[Stmt], atoms: &[(String, Expr)], v: u32) -> Option<bool> {
    let f = |x: &Expr| at_atoms(x, b, stmts, atoms, v);
    let atom = |k: &str| atoms.iter().position(|a| a.0 == k).map(|i| v >> i & 1 == 1);
    match e {
        Expr::Name(n) => match stmts.iter().rev().find(|s| matches!(s, Stmt::Class(c, _) | Stmt::Mask(c, _) if c == n)) {
            Some(Stmt::Class(_, items)) => Some(items.iter().any(|&(lo, hi)| (lo..=hi).contains(&u32::from(b)))),
            Some(Stmt::Mask(_, x)) => f(x),
            _ => atom(n),
        },
        Expr::Call(..) => atom(&format!("{e:?}")),
        Expr::Not(x) => f(x).map(|v| !v),
        Expr::And(x, y) => Some(f(x)? & f(y)?),
        Expr::Or(x, y) => Some(f(x)? | f(y)?),
        Expr::Xor(x, y) => Some(f(x)? ^ f(y)?),
        _ => None,
    }
}

/// The records `u` as a mask the grammar names, where the two differ only at bytes an error of tier 1 refuses the
/// file at: simdjson's `structural_start`, where a token's start (a string's quote, the region's opener) is not the
/// mask's and an error refuses the bytes between.
///
/// Proved by enumeration: each byte, and each value of the atoms (`atoms`) the starts and errors read, where an
/// escaped byte's `B` is before it (`escaped(B)` makes `prev(X)` true for each class mask `X` holding `B`).
fn refused_equal(u: &Expr, stmts: &[Stmt]) -> Option<Expr> {
    let covered = piece_covers(stmts);
    let errors: Vec<&Expr> = stmts.iter().enumerate().filter(|(i, _)| !covered.contains(i)).filter_map(|(_, s)| if let Stmt::Error(_, e) = s { Some(e) } else { None }).collect();
    // The candidates: the masks the terms of `u` have as a conjunct.
    let mut names = Vec::new();
    fn conj(e: &Expr, out: &mut Vec<String>) {
        match e {
            Expr::And(a, b) => {
                conj(a, out);
                conj(b, out);
            }
            Expr::Or(a, b) => {
                conj(a, out);
                conj(b, out);
            }
            Expr::Name(n) => out.push(n.clone()),
            _ => {}
        }
    }
    conj(u, &mut names);
    names.dedup();
    let class_of = |x: &Expr| -> Option<[bool; 256]> {
        let mut set = [false; 256];
        for b in 0..=255u8 {
            set[usize::from(b)] = at_byte(x, b, stmts)?;
        }
        Some(set)
    };
    for n in names {
        if !stmts.iter().any(|s| matches!(s, Stmt::Mask(m, _) if *m == n)) {
            continue;
        }
        let m = Expr::Name(n.clone());
        let mut ats = Vec::new();
        atoms(u, stmts, &mut ats);
        atoms(&m, stmts, &mut ats);
        // Only the errors that read the atoms alone (and classes) can be weighed.
        let errs: Vec<&Expr> = errors
            .iter()
            .copied()
            .filter(|e| {
                let mut a = ats.clone();
                atoms(e, stmts, &mut a);
                a.len() == ats.len()
            })
            .collect();
        if ats.len() > 12 || errs.is_empty() {
            continue;
        }
        // `escaped(B)` before `prev(X)`, `B` within `X`.
        let mut implies = Vec::new();
        for (i, (_, ei)) in ats.iter().enumerate() {
            let Expr::Call(f, args) = ei else { continue };
            let [Arg::E(bx)] = args.as_slice() else { continue };
            if f != "escaped" {
                continue;
            }
            let Some(bset) = class_of(bx) else { continue };
            for (j, (_, ej)) in ats.iter().enumerate() {
                let Expr::Call(g, a2) = ej else { continue };
                let x = match a2.as_slice() {
                    [Arg::E(x)] | [Arg::E(x), Arg::N(1)] if g == "prev" => x,
                    _ => continue,
                };
                if let Some(xs) = class_of(x)
                    && (0..256).all(|c| !bset[c] || xs[c])
                {
                    implies.push((i, j));
                }
            }
        }
        let ok = (0..1u32 << ats.len()).filter(|v| implies.iter().all(|&(i, j)| v >> i & 1 == 0 || v >> j & 1 == 1)).all(|v| {
            (0..=255u8).all(|b| {
                let err = errs.iter().any(|e| at_atoms(e, b, stmts, &ats, v) == Some(true));
                err || at_atoms(u, b, stmts, &ats, v).is_some_and(|x| Some(x) == at_atoms(&m, b, stmts, &ats, v))
            })
        });
        if ok {
            return Some(m);
        }
    }
    None
}

/// `a | b | …` with the conjuncts every term has taken out once: `(a' | b' | …) & c`; `c` alone where `a' | b' | …`
/// is classes holding every byte `c` can hold (simdjson's `structural_start`: each token's start `structural_start &
/// C`, their union `structural_start`).
fn factored(e: &Expr, stmts: &[Stmt]) -> Expr {
    fn terms<'e>(e: &'e Expr, out: &mut Vec<&'e Expr>) {
        match e {
            Expr::Or(a, b) => {
                terms(a, out);
                terms(b, out);
            }
            e => out.push(e),
        }
    }
    fn conj(e: &Expr, out: &mut Vec<Expr>) {
        match e {
            Expr::And(a, b) => {
                conj(a, out);
                conj(b, out);
            }
            e => out.push(e.clone()),
        }
    }
    let mut ts = Vec::new();
    terms(e, &mut ts);
    let cs: Vec<Vec<Expr>> = ts
        .iter()
        .map(|t| {
            let mut c = Vec::new();
            conj(t, &mut c);
            c
        })
        .collect();
    let key = |e: &Expr| format!("{e:?}");
    let common: Vec<Expr> = cs[0].iter().filter(|c| cs.iter().all(|t| t.iter().any(|x| key(x) == key(c)))).cloned().collect();
    // Each term keeps something past the common part, or the factoring says nothing.
    if ts.len() < 2 || common.is_empty() || cs.iter().any(|t| t.len() <= common.len()) {
        return e.clone();
    }
    let and = |v: Vec<Expr>| v.into_iter().reduce(|a, b| Expr::And(Box::new(a), Box::new(b))).expect("a conjunct");
    let rest = cs.into_iter().map(|t| and(t.into_iter().filter(|x| !common.iter().any(|c| key(c) == key(x))).collect())).reduce(|a, b| Expr::Or(Box::new(a), Box::new(b))).expect("a term");
    let common = and(common);
    if (0..=255u8).all(|b| at_byte(&common, b, stmts) == Some(false) || at_byte(&rest, b, stmts) == Some(true)) {
        return common;
    }
    Expr::And(Box::new(rest), Box::new(common))
}

/// Class `set`'s code in stage 1: one compare where it is one byte, else its nibble-table bits, each a product of high
/// nibbles and low nibbles, in the first pair of `pairs` with room.
fn class_code(pairs: &mut Vec<Vec<Rect>>, set: &[bool; 256]) -> String {
    let bytes: Vec<u8> = (0..=255u8).filter(|&c| set[usize::from(c)]).collect();
    match bytes.as_slice() {
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
                let p = match pairs.iter().position(|p| fits(p, &left)) {
                    Some(p) => p,
                    None => {
                        pairs.push(Vec::new());
                        pairs.len() - 1
                    }
                };
                let mut bits = 0u8;
                let mut rest = Vec::new();
                for r in left {
                    let pair = &mut pairs[p];
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

/// `e` with `assumed`'s masks read as their expressions (decision 0607's `in KIND`): each mask whose
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
        Expr::And(a, b) if a == b => *a,
        Expr::Or(a, b) if a == b => *a,
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
/// The helpers of `HELPERS` that `code` calls, outside its comments, and those they call.
fn used_helpers(code: &str) -> String {
    let called = |text: &str, name: &str| text.lines().filter(|l| !l.trim_start().starts_with("//")).any(|l| l.contains(&format!("{name}(")) && !l.contains(&format!("fn {name}(")));
    // Each helper: its doc comment, attributes and body, up to the next's.
    let mut parts: Vec<(String, String)> = Vec::new();
    for chunk in HELPERS.split("\n\n") {
        let Some(name) = chunk.lines().find_map(|l| l.strip_prefix("fn ")).and_then(|l| l.split(['(', '<']).next()) else { continue };
        parts.push((name.to_owned(), chunk.trim_matches('\n').to_owned()));
    }
    let mut keep = vec![false; parts.len()];
    let mut text = code.to_owned();
    loop {
        let mut more = false;
        for (k, (name, body)) in parts.iter().enumerate() {
            if !keep[k] && called(&text, name) {
                keep[k] = true;
                text.push_str(body);
                more = true;
            }
        }
        if !more {
            break;
        }
    }
    parts.iter().zip(keep).filter(|(_, k)| *k).map(|((_, b), _)| format!("\n{b}\n")).collect()
}

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
    match first_in(d, s + 1, l).and_then(|p| first_in(d, p + 1, l)) {
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

/// Whether `x` holds anywhere in bytes `a..=b` (none where `a > b`), its words to `b`'s alone read.
#[inline(always)]
fn any_in(x: &[u64], a: usize, b: usize) -> bool {
    first_in(x, a, b).is_some()
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

/// `toggle` over the file's words, where a mask made of the regions is one.
const TOGGLE: &str = r#"
/// Each byte's parity of `x`'s bits up to it (`toggle`), a word at a time, the word before's carried.
fn toggle(x: &[u64]) -> Vec<u64> {
    let mut carry = 0u64;
    x.iter()
        .map(|&w| {
            let mut v = w;
            v ^= v << 1;
            v ^= v << 2;
            v ^= v << 4;
            v ^= v << 8;
            v ^= v << 16;
            v ^= v << 32;
            let v = v ^ 0u64.wrapping_sub(carry);
            carry = v >> 63;
            v
        })
        .collect()
}
"#;

/// Tier 1's Rust, written from the lexer's statements.
pub fn tier1(x: &Input<'_>) -> Result<String, String> {
    Gen { x, body: String::new(), names: HashMap::new(), temps: HashMap::new(), next: 0, nodes: Vec::new(), node_of: HashMap::new(), regional: Default::default(), pairs: Vec::new(), sinks: Vec::new(), end_sinks: Vec::new(), opens: None, defs: HashMap::new(), region_checks: Vec::new(), prefix: None, index: None, outputs: Vec::new(), toggles: HashMap::new(), lookup: None, code_checks: Vec::new(), class_bytes: HashMap::new(), kinds: Vec::new(), hidden: String::new(), nest: None, spans: Vec::new(), skipped: [false; 256], start_in: None, fills: Vec::new(), field_writes: Vec::new(), ends: None, fill_proofs: Default::default(), region_values: Default::default(), opener_at: [Bytes::ALL; 4], start_checks: Vec::new(), stretch_checks: Vec::new(), tail_checks: Vec::new(), code_after: String::new(), end_checks: Vec::new(), shapes: Default::default(), regional_ids: Default::default(), opener_ids: Default::default(), def_ids: Default::default(), end_names: Default::default(), shape_ids: Default::default(), name_strs: Default::default(), name_ids: Default::default(), kept: Default::default(), cube_tangled: Default::default(), opener_cache: Default::default(), cube_open: Default::default(), tangles: Default::default(), cube_read: Default::default() }.source()
}

/// Whether `e` names `n`.
fn mentions(e: &Expr, n: &str) -> bool {
    match e {
        Expr::Name(m) => m == n,
        Expr::Empty => false,
        Expr::Not(a) => mentions(a, n),
        Expr::And(a, b) | Expr::Or(a, b) | Expr::Xor(a, b) => mentions(a, n) || mentions(b, n),
        Expr::Call(_, args) => args.iter().any(|a| matches!(a, Arg::E(x) if mentions(x, n))),
        Expr::Test { arg, .. } => arg.as_deref().is_some_and(|a| mentions(a, n)),
    }
}

/// The bytes some way of `cubes` may be at `d` bytes on (`Gen::bytes_at`).
fn bytes_in(cubes: &Cubes, d: i64) -> Bytes {
    let mut v = Bytes::NONE;
    for c in cubes {
        match c.get(&d) {
            Some(b) => v = v.or(*b),
            None => return Bytes::ALL,
        }
    }
    v
}

/// `code` with no test build's checks (decision 0655 step 3): each `#[cfg(feature = "test-hooks")]` assert and the
/// comment that says why, and a block left holding its value alone (`if {\n    E\n} {`) as that value (`if E {`), as
/// the lean lexer holds no checker.
/// The lean lexer's `left_to_right`: a run of pairs' first bytes (`=>=`) read left to right.
const LEFT_TO_RIGHT: &str = "\n/// A run of pairs' first bytes (`=>=`), read left to right, as the lean lexer's: one whose first byte is the second of\n/// the one before is none.\n#[inline(always)]\nfn left_to_right(mut m: u64) -> u64 {\n    if m & m << 1 == 0 {\n        return m;\n    }\n    let mut kept = 0;\n    while m != 0 {\n        let i = m.trailing_zeros();\n        kept |= 1 << i;\n        m &= !(1 << i);\n        if i < 63 {\n            m &= !(1 << (i + 1));\n        }\n    }\n    kept\n}\n";

/// The token loop's block in the lean lexer's shape (decision 0655 steps 4, 5): the block's starts at once from its four
/// masks (`space`, the word's `kw`, `k_rare`, the pairs' `kp`); the run of common tokens before each rare start, each
/// by one load (`TOKEN`) and its word's run or its pair; each rare token by the rows' `match` (`open`, `branches`).
fn lean_loop(space: &str, kw: &str, wfn: &str, kp: &str, open: &str, branches: &str) -> String {
    format!(
        "        // The block's starts from its byte `k` on, as the lean lexer's: a byte `skip` does not hold, not a word's after its
        // first, not a pair's second (the pairs read left to right); and byte `k`, the byte after a token.
        let lv = valid(n, base as isize);
        let starts_from = |k: usize| {{
            let s = !{space} & lv & (!({kw} & {kw} << 1) & (!0u64 << k) | 1u64 << k);
            s & !(left_to_right({kp} & s) << 1)
        }};
        let mut starts = starts_from(at.saturating_sub(base));
        loop {{
            let rare = starts & k_rare;
            // The run: the starts before the next rare one, each a word or an operator, by one load (the lean lexer's
            // `TOKEN`): its kind, its word alone; a word's last byte by its run, an operator's by its pair.
            let mut run = starts & (rare.wrapping_sub(1) & !rare);
            while run != 0 {{
                let q = run.trailing_zeros() as usize;
                run &= run - 1;
                let s = base + q;
                let (l, kind1, word) = if {kw} >> q & 1 == 1 {{
                    // A word: its run, its code by its hash (a word of one byte's by the load).
                    let (_, kind1, word1) = TOKEN[usize::from(src[s])];
                    let l = word_last(n, base, {kw}, s, {wfn});
                    (l, kind1, if l == s {{ u32::from(word1) }} else {{ word_at(src, s, l + 1 - s) }})
                }} else {{
                    // An operator, one byte or a pair (`p`): its kind and code by one load, as the lean lexer's
                    // `TOKEN[c + 256]`.
                    let p = ({kp} >> q & 1) as usize;
                    let (_, kind1, word1) = TOKEN[usize::from(src[s]) | p << 8/*PAIR_ALT p*/];
                    (s + p, kind1, u32::from(word1))
                }};
                debug_assert!(len < n, \"a token a byte at most\");
                // SAFETY: `len < n`, the room reserved: a record's first byte is after the record before's last.
                unsafe {{ out.add(len).write(Rec {{ start: s as u32, last: l as u32, code: u32::from(kind1) | word << 8 }}) }};
                len += 1;
                if l + 1 >= base + 64 {{
                    // A token on past the block: the blocks from its end.
                    at = l + 1;
                    b = at / 64;
                    continue 'tokens;
                }}
            }}
            if rare == 0 {{
                break;
            }}
            // A rare token, read by its row: the lean lexer's `match` on its first byte.
            let s = base + rare.trailing_zeros() as usize;
{open}            let (l, kind, extra, marks, word): (usize, u32, bool, u32, u32) = {branches};
            let rec = Rec {{ start: s as u32, last: l as u32, code: kind | word << 8 | marks << 16 }};
            if extra {{
                extras.push(rec);
            }} else {{
                debug_assert!(len < n && l >= s, \"a token a byte at most\");
                // SAFETY: as the run's.
                unsafe {{ out.add(len).write(rec) }};
                len += 1;
            }}
            at = l + 1;
            if at >= base + 64 {{
                b = at / 64;
                continue 'tokens;
            }}
            starts = starts_from(at - base);
        }}
"
    )
}

/// The rows a group's tests pick, in the rows' `match` (`branches`): each a line of its own in the picks' arms.
fn group_rows(branches: &str) -> Vec<usize> {
    let picks = branches.split("match row {").next().unwrap_or_default();
    picks.lines().filter_map(|l| l.trim().parse::<usize>().ok()).collect()
}

/// The rows' `match` in the lean lexer's shape (decision 0655 step 7): one `match` on the first byte's pick (`TOKEN`), a
/// group's tests its arms' guards, and a region's openers (`//` or `/*`, one, two or three backticks) an arm each, as lean's
/// `b'/' if d == b'/'`: no row picked by the tests, then the same tests again in the row. None where the loop's text is
/// not of that form.
fn one_pick(out: &str) -> Option<String> {
    let head = "let (l, kind, extra, marks, word): (usize, u32, bool, u32, u32) = {\n";
    let at = out.find(head)?;
    let open = at + head.len() - 2;
    let close = closing_brace(out, open)?;
    let lines: Vec<&str> = out[open + 2..close].lines().collect();
    let pad = |l: &str| l.len() - l.trim_start().len();
    // The picks: `p if usize::from(p) < K => usize::from(p),`, each group's `G => if C {` chain of rows, `_ => usize::MAX`.
    let p0 = lines.iter().position(|l| l.trim() == "let row = match pick {")?;
    let ind = pad(lines[p0]);
    let p1 = (p0 + 1..lines.len()).find(|&i| pad(lines[i]) == ind && lines[i].trim() == "};")?;
    let mut direct = 0usize;
    // Each group: its comment, its byte's pick, its rows by their tests, its row where none holds.
    let mut groups: Vec<(String, String, Vec<(String, usize)>, Option<usize>)> = Vec::new();
    let mut comment = String::new();
    let mut i = p0 + 1;
    while i < p1 {
        let t = lines[i].trim();
        if let Some(k) = t.strip_prefix("p if usize::from(p) < ").and_then(|r| r.strip_suffix(" => usize::from(p),")) {
            direct = k.parse().ok()?;
        } else if t.starts_with("//") {
            comment = lines[i].to_owned();
        } else if t == "_ => usize::MAX," {
        } else if let Some((g, c)) = t.split_once(" => if ") {
            let mut chain = Vec::new();
            let mut cond = c.strip_suffix(" {")?.to_owned();
            let dflt;
            i += 1;
            loop {
                let v = lines[i].trim();
                let row = if v == "usize::MAX" { None } else { Some(v.parse::<usize>().ok()?) };
                let next = lines.get(i + 1)?.trim();
                match next.strip_prefix("} else if ").and_then(|c| c.strip_suffix(" {")) {
                    Some(c2) => {
                        chain.push((std::mem::replace(&mut cond, c2.to_owned()), row?));
                        i += 2;
                    }
                    None if next == "} else {" => {
                        chain.push((cond.clone(), row?));
                        dflt = if lines.get(i + 2)?.trim() == "usize::MAX" { None } else { Some(lines[i + 2].trim().parse().ok()?) };
                        if lines.get(i + 3)?.trim() != "}," {
                            return None;
                        }
                        i += 3;
                        break;
                    }
                    None => return None,
                }
            }
            groups.push((std::mem::take(&mut comment), g.to_owned(), chain, dflt));
        } else {
            return None;
        }
        i += 1;
    }
    // The rows: `K => {` … `},` and the last, `_ => {` … `}`.
    let m0 = (p1 + 1..lines.len()).find(|&i| lines[i].trim() == "match row {")?;
    let ai = pad(lines[m0]) + 4;
    let mut rows: Vec<(usize, Vec<&str>)> = Vec::new();
    let mut none_arm: Vec<&str> = Vec::new();
    let mut i = m0 + 1;
    while i < lines.len() && pad(lines[i]) >= ai {
        let t = lines[i].trim();
        let end = (i + 1..lines.len()).find(|&j| pad(lines[j]) == ai && lines[j].trim_start().starts_with('}'))?;
        if let Some(k) = t.strip_suffix(" => {").and_then(|k| k.parse::<usize>().ok()) {
            rows.push((k, lines[i + 1..end].to_vec()));
        } else if t == "_ => {" {
            none_arm = lines[i + 1..end].to_vec();
        } else {
            return None;
        }
        i = end + 1;
    }
    // Which rows a byte picks itself (`TOKEN`'s first of each byte under `direct`).
    // Its entries, `(pick, kind, word)`, in its `const` block (`table_block`).
    let table = out.split_once("static TOKEN: [(u8, u8, u8); ")?.1.split_once("\n};")?.0;
    let picked: Vec<usize> = table.split('(').skip(1).filter_map(|e| e.split_once(", ")?.0.parse::<usize>().ok()).filter(|&p| p < direct).collect();
    let norm = |c: &str| c.chars().filter(|c| !matches!(c, '(' | ')' | ' ')).collect::<String>();
    let body = |r: usize| rows.iter().find(|x| x.0 == r).map(|x| x.1.clone());
    let arm = |pat: &str, b: &[&str]| format!("{}{pat} => {{\n{}\n{}}},\n", " ".repeat(ai), b.join("\n"), " ".repeat(ai));
    let mut arms = String::new();
    let mut split: Vec<usize> = Vec::new();
    for (comment, g, chain, _) in &groups {
        if !comment.is_empty() {
            let _ = writeln!(arms, "{}{}", " ".repeat(ai), comment.trim());
        }
        for (c, r) in chain {
            let b = body(*r)?;
            // A region read by its openers' alternatives, which are the group's test: an arm each.
            let l0 = b.iter().position(|l| l.trim().starts_with("let l = if ") && l.trim().ends_with(" {"));
            let alts = l0.and_then(|l0| {
                let li = pad(b[l0]);
                let l1 = (l0 + 1..b.len()).find(|&j| pad(b[j]) == li && b[j].trim() == "};")?;
                let mut alts: Vec<(String, Vec<&str>)> = vec![(b[l0].trim().strip_prefix("let l = if ")?.strip_suffix(" {")?.to_owned(), Vec::new())];
                for &l in &b[l0 + 1..l1] {
                    if pad(l) == li && l.trim().starts_with("} else") {
                        let c = l.trim().strip_prefix("} else if ").and_then(|c| c.strip_suffix(" {")).unwrap_or("").to_owned();
                        alts.push((c, Vec::new()));
                    } else if !l.trim().is_empty() {
                        alts.last_mut()?.1.push(l);
                    }
                }
                // The last, none of the openers: a refusal, as the byte no token begins with.
                let last = alts.pop()?;
                (last.0.is_empty() && last.1.iter().any(|l| l.contains("refused = true"))).then_some(())?;
                (norm(c) == norm(&alts.iter().map(|a| a.0.as_str()).collect::<Vec<_>>().join("||"))).then_some((l0, l1, li, alts))
            });
            match alts {
                Some((l0, l1, li, alts)) => {
                    for (a, x) in alts {
                        let mut nb: Vec<String> = b[..l0].iter().map(|l| (*l).to_owned()).collect();
                        let mut first = true;
                        for (k, l) in x.iter().enumerate() {
                            let rel = pad(l).saturating_sub(li + 4);
                            let mut s = if first { format!("{}let l = {}", " ".repeat(li), l.trim_start()) } else { format!("{}{}", " ".repeat(li + rel), l.trim_start()) };
                            if k + 1 == x.len() {
                                s.push(';');
                            }
                            first = false;
                            nb.push(s);
                        }
                        nb.extend(b[l1 + 1..].iter().map(|l| (*l).to_owned()));
                        let mut text = nb.join("\n");
                        // A read that is never open: no `open_bad`, its record as when closed.
                        if !text.contains("open_bad = true") {
                            text = text.lines().filter(|l| l.trim() != "let mut open_bad = false;").collect::<Vec<_>>().join("\n").replace("!open_bad", "true");
                            while let Some(k) = text.find("if open_bad { 0 } else { ") {
                                let from = k + "if open_bad { 0 } else { ".len();
                                let Some(e) = text[from..].find(" }").map(|e| from + e) else { break };
                                let x = text[from..e].to_owned();
                                text.replace_range(k..e + 2, &x);
                            }
                        }
                        let _ = write!(arms, "{}{g} if {a} => {{\n{text}\n{}}},\n", " ".repeat(ai), " ".repeat(ai));
                    }
                    split.push(*r);
                }
                None => arms.push_str(&arm(&format!("{g} if {c}"), &b)),
            }
        }
    }
    // Each row: the bytes that pick it, and the groups it is the row of where none of their tests holds.
    for (r, b) in &rows {
        let mut pats: Vec<String> = Vec::new();
        if picked.contains(r) {
            pats.push(r.to_string());
        }
        pats.extend(groups.iter().filter(|g| g.3 == Some(*r)).map(|g| g.1.clone()));
        if pats.is_empty() {
            if !split.contains(r) && !groups.iter().any(|g| g.2.iter().any(|x| x.1 == *r)) {
                return None;
            }
            continue;
        }
        arms.push_str(&arm(&pats.join(" | "), b));
    }
    let _ = write!(arms, "{}_ => {{\n{}\n{}}}\n", " ".repeat(ai), none_arm.join("\n"), " ".repeat(ai));
    let new = format!("let (l, kind, extra, marks, word): (usize, u32, bool, u32, u32) = match pick {{\n{arms}{}}}", " ".repeat(ind));
    // Each arm a level out: the `match row` arms were one deeper than the one `match`'s.
    let new = dedent_from(&new, 4);
    Some(format!("{}{}{}", &out[..at], new, &out[close + 1..]))
}

/// The rare tokens' `match` with each group's tests as the lean lexer's arms make them (decision 0655 step 12): no
/// test of the first byte where the pick says it (one byte picks the group: `b'/' if d == b'/'`), and a group whose arms
/// are runs of one byte (one backtick, two, three) one arm that counts the run once and reads by its count (lean's
/// `k`), its arm of none the `match`'s. Else `out`.
fn counted_runs(out: &str) -> String {
    let Some(h) = out.find(" = match pick {\n") else { return out.to_owned() };
    let open = h + " = match pick {".len() - 1;
    let Some(close) = closing_brace(out, open) else { return out.to_owned() };
    let body = &out[open + 2..close];
    // The `}` line's indent, after the arms.
    let tail = &body[body.rfind('\n').map_or(0, |k| k + 1)..];
    let lines: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty()).collect();
    let pad = |l: &str| l.len() - l.trim_start().len();
    let Some(ai) = lines.first().map(|l| pad(l)) else { return out.to_owned() };
    let table = out.split_once("static TOKEN: [(u8, u8, u8); ").and_then(|x| x.1.split_once("\n};")).map_or("", |x| x.0);
    // The one byte that picks group `g`, where one does (`t[b'/' as usize] = (13, `, no range of them).
    let alone = |g: &str| {
        let tag = format!(" = ({g}, ");
        let hits: Vec<&str> = table.lines().filter(|l| l.contains(&tag)).collect();
        match hits.as_slice() {
            [l] => l.trim().strip_prefix("t[").and_then(|r| r.split_once(" as usize]")).map(|x| x.0.to_owned()).filter(|b| b.starts_with("b'") || b.starts_with("0x")),
            _ => None,
        }
    };
    let norm = |c: &str| c.chars().filter(|c| !matches!(c, '(' | ')' | ' ')).collect::<String>();
    let strip = |mut c: String| {
        while c.starts_with('(') && c.ends_with(')') && closing_paren(&c, 0) == Some(c.len() - 1) {
            c = c[1..c.len() - 1].to_owned();
        }
        c
    };
    // The arms: their header lines, bodies and the comments before them.
    struct Arm {
        head: Vec<String>,
        pick: String,
        cond: Option<String>,
        body: Vec<String>,
    }
    let mut arms: Vec<Arm> = Vec::new();
    let mut lead: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        if pad(l) == ai && l.trim().starts_with("//") {
            lead.push(l.to_owned());
            i += 1;
            continue;
        }
        let t = l.trim();
        let Some(h) = t.strip_suffix(" => {").filter(|_| pad(l) == ai) else { return out.to_owned() };
        let Some(end) = (i + 1..lines.len()).find(|&j| pad(lines[j]) == ai && lines[j].trim_start().starts_with('}')) else { return out.to_owned() };
        let (pick, cond) = match h.split_once(" if ") {
            Some((p, c)) => (p.to_owned(), Some(c.to_owned())),
            None => (h.to_owned(), None),
        };
        arms.push(Arm { head: std::mem::take(&mut lead), pick, cond, body: lines[i + 1..end].iter().map(|x| (*x).to_owned()).collect() });
        i = end + 1;
    }
    // No test of the byte the pick says.
    for a in &mut arms {
        let (Some(c), Some(b)) = (a.cond.clone(), alone(&a.pick)) else { continue };
        let first = format!("(src[s] == {b}) && ");
        if let Some(k) = c.find(&first) {
            let mut c2 = c.clone();
            c2.replace_range(k..k + first.len(), "");
            a.cond = Some(strip(c2));
        }
    }
    // A group of runs: arm `j` of its arms in order a run of `j` of byte `B` from the first.
    let run_of = |b: &str, j: usize| {
        let at = |i: usize| format!("s+{i}<n&&src[s+{i}]=={b}");
        let mut v: Vec<String> = (1..j).map(at).collect();
        v.push(format!("!{}", at(j)));
        v.join("&&")
    };
    let mut text = String::new();
    let none_body = arms.iter().find(|a| a.pick == "_").map(|a| a.body.clone());
    let mut k = 0;
    while k < arms.len() {
        let g = arms[k].pick.clone();
        let group: Vec<usize> = (k..arms.len()).take_while(|&x| arms[x].pick == g && arms[x].cond.is_some()).collect();
        let byte = alone(&g);
        let counted = group.len() >= 2 && byte.as_ref().is_some_and(|b| group.iter().enumerate().all(|(j, &x)| arms[x].cond.as_deref().map(norm) == Some(run_of(b, j + 1))));
        let sp = " ".repeat(ai);
        match (counted, &none_body) {
            (true, Some(none)) => {
                let b = byte.unwrap_or_default();
                for l in &arms[k].head {
                    let _ = writeln!(text, "{l}");
                }
                let _ = write!(text, "{sp}{g} => {{\n{sp}    // Its run of {b} counted once, read by its count (the lean lexer's `k`).\n{sp}    let mut k = 1;\n{sp}    while s + k < n && src[s + k] == {b} {{\n{sp}        k += 1;\n{sp}    }}\n{sp}    match k {{\n");
                for (j, &x) in group.iter().enumerate() {
                    let _ = writeln!(text, "{sp}        {} => {{", j + 1);
                    for l in &arms[x].body {
                        let _ = writeln!(text, "{}{l}", if l.is_empty() { "" } else { "        " });
                    }
                    let _ = writeln!(text, "{sp}        }}");
                }
                let _ = writeln!(text, "{sp}        _ => {{\n{sp}            // A run no arm reads.");
                for l in none.iter().filter(|l| !l.trim().starts_with("//")) {
                    let _ = writeln!(text, "{}{l}", if l.is_empty() { "" } else { "        " });
                }
                let _ = writeln!(text, "{sp}        }}\n{sp}    }}\n{sp}}},");
                k += group.len();
            }
            _ => {
                let a = &arms[k];
                for l in &a.head {
                    let _ = writeln!(text, "{l}");
                }
                let head = match &a.cond {
                    Some(c) => format!("{} if {c}", a.pick),
                    None => a.pick.clone(),
                };
                let _ = writeln!(text, "{sp}{head} => {{");
                for l in &a.body {
                    let _ = writeln!(text, "{l}");
                }
                let _ = writeln!(text, "{sp}}}{}", if a.pick == "_" { "" } else { "," });
                k += 1;
            }
        }
    }
    format!("{}{}{}{}", &out[..open + 2], text, tail, &out[close..])
}

/// The lean form's reads as hand-written Rust: a count that asks for one as `any`, a test of every byte as `all`, a
/// region's end found as it is (`c`, not `c + 0`), the run's next byte by its index (`l + 1`).
fn tidy_lean(out: &str) -> String {
    let out = counted_runs(out);
    let mut out = out.replace("Some(c) => c + 0,", "Some(c) => c,").replace("(l + 1) >= 1 && ", "").replace("(l + 1) - 1", "l").replace("(l + 1) + 1", "l + 2").replace("((l + 1) < n", "(l + 1 < n");
    // `(a..=b).filter(|&i| P).count() as u32 } >= 1`: whether one holds.
    while let Some(e) = out.find(".count() as u32 } >= 1") {
        let Some(f) = out[..e].rfind(".filter(|&i| ") else { break };
        out.replace_range(e..e + ".count() as u32 } >= 1".len(), " }");
        out.replace_range(f..f + ".filter(|&i| ".len(), ".any(|i| ");
    }
    // `(a..b).find(|&i| !(P)).is_none()`: whether every byte holds.
    while let Some(f) = out.find(".find(|&i| !(") {
        let open = f + ".find(|&i| !".len();
        let Some(close) = closing_paren(&out, open) else { break };
        if !out[close + 1..].starts_with(").is_none()") {
            break;
        }
        let p = out[open..=close].to_owned();
        out.replace_range(f..close + 1 + ").is_none()".len(), &format!(".all(|i| {p})"));
    }
    // `(!(P)).then_some(s).is_none()`: whether `P` holds.
    let mut from = 0;
    while let Some(f) = out[from..].find("(!(").map(|f| f + from) {
        from = f + 1;
        let open = f + 2;
        let Some(close) = closing_paren(&out, open) else { break };
        if out[close + 1..].starts_with(").then_some(s).is_none()") {
            let p = out[open..=close].to_owned();
            out.replace_range(f..close + 1 + ").then_some(s).is_none()".len(), &p);
        }
    }
    // A run of one byte at least from the token's first: always.
    out = out.replace(" && l + 1 - s >= 1 && ", " && ");
    // The token's last byte and kind, where the tests read both.
    if let Some(a) = out.find("    let _ = (l, kind);\n")
        && let Some(e) = out[a..].find("\n}\n").map(|e| e + a)
        && out[a..e].contains("kind ==")
        && out[a..e].contains("..=l)")
    {
        out.replace_range(a..a + "    let _ = (l, kind);\n".len(), "");
    }
    // A record's marks: none but an extra's own bit, which the extras' list says (decision 0655 step 9). The record is
    // its kind and word, as the lean lexer's `Tok`, and an extra its span in the list, as its `comments`.
    out = no_record_marks(&out);
    if out.contains("code: kind | word << 8 }") && !out.contains("marks << 16") {
        out = out.replace("its code (its kind, its word and its marks).", "its code (its kind and its word).");
    }
    // The token tests read no vector: a plain function, as the lean parser's tests of a token.
    if let Some(a) = out.find("unsafe fn token_test_on<S: rerail_simd8::Simd8x64>(")
        && let Some(d) = out[a..].find("\nrerail_simd8::dispatch! {\n    fn token_test(").map(|d| d + a)
        && let Some(e) = out[d..].find("\n}\n").map(|e| e + d + 3)
        && !out[a..d].contains("S::")
        && !out[a..d].contains("<S>")
    {
        out.replace_range(d..e, "");
        out.replace_range(a..a + "unsafe fn token_test_on<S: rerail_simd8::Simd8x64>(".len(), "fn token_test(");
        out = out.replace("one of its own. Its row by its first byte, its kind and whether it is an extra.\n///\n/// # Safety\n/// The CPU runs `S`'s backend.\n#[inline(always)]\nfn token_test(", "one of its own. Its row by its first byte, its kind and whether it is an extra.\nfn token_test(");
    }
    out
}

/// `out` with no marks in the rare tokens' records, where each arm's are none (`0x0`) or, at an extra, the extra's
/// bit alone (`true, B` or `!open_bad, if open_bad { 0 } else { B }`): the tuple's marks gone, the record its kind and
/// word, and the token tests' `r.code >> 16 & B == B` (an extra's bit) the `extra` before it. Else `out`.
fn no_record_marks(out: &str) -> String {
    const HEAD: &str = "let (l, kind, extra, marks, word): (usize, u32, bool, u32, u32) = match pick {";
    let Some(h) = out.find(HEAD) else { return out.to_owned() };
    let Some(e) = closing_brace(out, h + HEAD.len() - 1) else { return out.to_owned() };
    let mut bit: Option<String> = None;
    let mut arms: Vec<(usize, usize, String)> = Vec::new();
    let from = h + HEAD.len();
    for (at, _) in out[from..e].match_indices("(l, ") {
        let at = from + at;
        // An arm's record, a line of its own; not a binding's pattern (`let (l, once) =`).
        if !out[..at].rsplit('\n').next().is_some_and(|p| p.chars().all(char::is_whitespace)) {
            continue;
        }
        let Some(close) = closing_paren(out, at) else { return out.to_owned() };
        // Its five parts, split at its own commas.
        let inner = &out[at + 1..close];
        let mut parts = Vec::new();
        let (mut depth, mut from) = (0i32, 0usize);
        for (i, ch) in inner.char_indices() {
            match ch {
                '(' | '{' | '[' => depth += 1,
                ')' | '}' | ']' => depth -= 1,
                ',' if depth == 0 => {
                    parts.push(inner[from..i].trim());
                    from = i + 1;
                }
                _ => {}
            }
        }
        parts.push(inner[from..].trim());
        if parts.len() != 5 {
            return out.to_owned();
        }
        let (extra, marks) = (parts[2], parts[3]);
        let own = match (extra, marks) {
            (_, "0x0" | "0") => None,
            ("true", b) if b.starts_with("0x") => Some(b.to_owned()),
            ("!open_bad", m) => m.strip_prefix("if open_bad { 0 } else { ").and_then(|r| r.strip_suffix(" }")).map(str::to_owned),
            _ => return out.to_owned(),
        };
        if let Some(b) = own {
            if bit.as_ref().is_some_and(|x| *x != b) {
                return out.to_owned();
            }
            bit = Some(b);
        }
        arms.push((at, close, format!("({}, {}, {}, {})", parts[0], parts[1], parts[2], parts[4])));
    }
    let mut s = out.to_owned();
    for (at, close, t) in arms.into_iter().rev() {
        s.replace_range(at..=close, &t);
    }
    s = s.replace(HEAD, "let (l, kind, extra, word): (usize, u32, bool, u32) = match pick {");
    s = s.replace("let rec = Rec { start: s as u32, last: l as u32, code: kind | word << 8 | marks << 16 };", "let rec = Rec { start: s as u32, last: l as u32, code: kind | word << 8 };");
    if let Some(b) = bit {
        s = s.replace(&format!("extra && r.code >> 16 & {b} == {b} && "), "extra && ");
    }
    s
}

/// The index of the `)` closing the `(` at `open`.
fn closing_paren(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// The index of the `}` closing the `{` at `open`.
fn closing_brace(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// `s`'s lines after its first, `k` spaces less indented.
fn dedent_from(s: &str, k: usize) -> String {
    let mut out = String::new();
    for (i, l) in s.split_inclusive('\n').enumerate() {
        let lead = l.len() - l.trim_start_matches(' ').len();
        out.push_str(if i == 0 { l } else { &l[lead.min(k)..] });
    }
    out
}

/// The rows' `match` (`branches`) without the arms of rows `drop`.
fn without_rows(branches: &str, drop: &[usize]) -> String {
    let mut out = branches.to_owned();
    for i in drop {
        let Some(at) = out.find("match row {") else { break };
        let head = format!("\n                {i} => {{\n");
        let Some(s) = out[at..].find(&head).map(|k| at + k + 1) else { continue };
        let Some(e) = out[s..].find("\n,\n").map(|k| s + k + 3) else { continue };
        out.replace_range(s..e, "");
    }
    out
}

/// A byte's test of being in `set`, as a `matches!` of its ranges.
fn byte_set_test(b: &str, set: &[bool; 256]) -> String {
    let mut arms = Vec::new();
    let mut c = 0;
    while c < 256 {
        if !set[c] {
            c += 1;
            continue;
        }
        let lo = c;
        while c + 1 < 256 && set[c + 1] {
            c += 1;
        }
        arms.push(if lo == c { bytes::lit(lo as u8) } else { format!("{}..={}", bytes::lit(lo as u8), bytes::lit(c as u8)) });
        c += 1;
    }
    match arms.as_slice() {
        [] => "false".to_owned(),
        [one] if !one.contains("..=") => format!("({b} == {one})"),
        _ => format!("matches!({b}, {})", arms.join(" | ")),
    }
}

/// `rows` as a `const` block, as the lean lexer builds its tables: a byte's most common entry everywhere, then each run
/// of bytes of the same entry (a loop where it is longer than one byte), each pair's entry.
fn table_block(rows: &[(u8, u8, u8)]) -> String {
    let mut counts: HashMap<(u8, u8, u8), usize> = HashMap::new();
    for r in &rows[..rows.len().min(256)] {
        *counts.entry(*r).or_default() += 1;
    }
    let dflt = *counts.iter().max_by_key(|&(r, c)| (*c, std::cmp::Reverse(*r))).map(|(r, _)| r).unwrap_or(&(0, 0, 0));
    let name = |i: usize| match i {
        0..=255 => format!("{} as usize", bytes::lit(i as u8)),
        _ => format!("{} + {} as usize", i & !0xff, bytes::lit(i as u8)),
    };
    let mut out = format!("{{\n    let mut t = [{dflt:?}; {}];\n", rows.len());
    let mut i = 0;
    while i < rows.len() {
        // A pair's slot no pair has is never read.
        if rows[i] == dflt || (i >= 256 && rows[i] == (u8::MAX, 0, 0)) {
            i += 1;
            continue;
        }
        let lo = i;
        while i + 1 < rows.len() && (i + 1) % 256 != 0 && rows[i + 1] == rows[lo] {
            i += 1;
        }
        if lo == i {
            let _ = writeln!(out, "    t[{}] = {:?};", name(lo), rows[lo]);
        } else {
            let _ = writeln!(out, "    let mut c = {};\n    while c <= {} {{\n        t[c] = {:?};\n        c += 1;\n    }}", name(lo), name(i), rows[lo]);
        }
        i += 1;
    }
    out.push_str("    t\n}");
    out
}

/// The body of the token tests tier 2 asks (`token_test_on`, decision 0655 step 6): a `match` on the mark's bit, each arm
/// its tests at the rows they are of, each row known by its first byte, its kind and whether it is an extra (its extra
/// bits). In the whole file's form: the bytes' form rewrites it. None where a test's row is not found.
fn token_tests_fn(asks: &[(u8, bool, String, String)], rows: &[(String, bool, u32, [bool; 256], String)], names: &HashMap<u8, String>) -> Option<String> {
    let mut bits: Vec<u8> = asks.iter().map(|a| a.0).collect();
    bits.sort_unstable();
    bits.dedup();
    let mut out = String::from("    match b {\n");
    for b in bits {
        // Each row's tests of the mark, `after` or not: one arm, its tests joined.
        let mut rows_of: Vec<(&String, bool, Vec<&String>)> = Vec::new();
        for (_, after, kind, holds) in asks.iter().filter(|a| a.0 == b) {
            match rows_of.iter_mut().find(|r| r.0 == kind && r.1 == *after) {
                Some(r) => r.2.push(holds),
                None => rows_of.push((kind, *after, vec![holds])),
            }
        }
        let mut arms = Vec::new();
        for (kind, after, holds) in rows_of {
            let (_, extra, xb, first, k) = rows.iter().find(|r| r.0 == *kind)?;
            let who = if *extra { format!("extra && r.code >> 16 & {xb:#x} == {xb:#x}") } else { format!("!extra && kind == {}", k.replace("u32::from(KINDS[usize::from(src[s])])", "u32::from(TOKEN[usize::from(src[s])].1)")) };
            let holds = if holds.len() == 1 { holds[0].clone() } else { format!("({})", holds.iter().map(|h| h.as_str()).collect::<Vec<_>>().join(" || ")) };
            arms.push(format!("({}after && {who} && {} && {holds})", if after { "" } else { "!" }, byte_set_test("src[s]", first)));
        }
        let name = names.get(&b).map_or(String::new(), |n| format!("        // `{n}`\n"));
        let _ = writeln!(out, "{name}        {b} => {},", arms.join("\n            || "));
    }
    out.push_str("        _ => false,\n    }\n");
    Some(out)
}

/// `code` (the rows' `match`) with no hits of the marks' token tests (`if T { hits |= 1 << B; }`), which tier 2 asks
/// (decision 0655 step 6), nor a test of an open region left with nothing in it.
/// The rows' arms with each test its row's read decides (`once_asks`: its row's kind, its mark's bit) its record's bit:
/// the run read with its one `D` (`run_once_hit`, `once`), and the bit in the record's kind, as the lean lexer's number
/// arm writes `FLOAT` where it read a `.`. Tier 2 reads the bit from the record (`TESTED` holds it not).
fn once_hits(code: &str, once_asks: &[(String, u8)], names: &HashMap<u8, String>) -> Result<String, String> {
    let mut out = code.to_owned();
    for (kind, bit) in once_asks {
        let head = format!("// `{kind}`: run ");
        let at = out.find(&head).ok_or_else(|| format!("`{kind}`'s arm is not its run's"))?;
        let read = out[at..].find("let l = run_once_last(&").map(|r| r + at).ok_or_else(|| format!("`{kind}`'s arm reads no run once"))?;
        out.replace_range(read..read + "let l = run_once_last(&".len(), "let (l, once) = run_once_hit(&");
        let past = read + "let (l, once) = run_once_hit(&".len();
        let tuple = out[past..].find("(l, ").map(|t| t + past).ok_or_else(|| format!("`{kind}`'s arm has no record"))?;
        let line = out[..tuple].rfind('\n').map_or(0, |i| i + 1);
        let pad = out[line..tuple].to_owned();
        if !pad.chars().all(char::is_whitespace) {
            return Err(format!("`{kind}`'s record is not a line of its own"));
        }
        let k_from = tuple + "(l, ".len();
        let mut depth = 0i32;
        let k_len = out[k_from..]
            .char_indices()
            .find(|&(_, c)| {
                depth += i32::from(matches!(c, '(' | '{' | '[')) - i32::from(matches!(c, ')' | '}' | ']'));
                c == ',' && depth == 0
            })
            .map(|(i, _)| i)
            .ok_or_else(|| format!("`{kind}`'s record has no kind"))?;
        let k = out[k_from..k_from + k_len].trim().to_owned();
        let name = names.get(bit).map_or_else(|| format!("mark {bit}"), |n| format!("`{n}`"));
        out.replace_range(line..k_from + k_len, &format!("{pad}// {name}: the run's one byte read (`once`), its record's bit, as the lean lexer writes its kind.\n{pad}(l, {k} | u32::from(once) << {}", 16 + u32::from(*bit)));
    }
    Ok(out)
}

fn no_hits(code: &str) -> String {
    let lines: Vec<&str> = code.split_inclusive('\n').collect();
    let mut keep: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if i + 2 < lines.len() && lines[i].trim_end().ends_with(" {") && lines[i + 1].trim_start().starts_with("hits |= 1 << ") && lines[i + 2].trim() == "}" {
            i += 3;
            continue;
        }
        keep.push(lines[i]);
        i += 1;
    }
    let mut out = String::new();
    let mut i = 0;
    while i < keep.len() {
        if i + 1 < keep.len() && keep[i].trim() == "if !open_bad {" && keep[i + 1].trim() == "}" {
            i += 2;
            continue;
        }
        out.push_str(keep[i]);
        i += 1;
    }
    out
}

fn no_test_checks(code: &str) -> String {
    let lines: Vec<&str> = code.lines().collect();
    let mut kept: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim_start();
        if t == "#[cfg(feature = \"test-hooks\")]" && lines.get(i + 1).is_some_and(|n| n.trim_start().starts_with("assert")) {
            if kept.last().is_some_and(|l| l.trim_start().starts_with("//") && l.contains("test build")) {
                kept.pop();
            }
            i += 2;
            continue;
        }
        kept.push(lines[i].to_owned());
        i += 1;
    }
    let mut out: Vec<String> = Vec::with_capacity(kept.len());
    let mut i = 0;
    while i < kept.len() {
        if kept[i].ends_with("if {") && i + 2 < kept.len() && kept[i + 2].trim_start() == "} {" {
            let head = &kept[i][..kept[i].len() - 1];
            out.push(format!("{head}{} {{", kept[i + 1].trim()));
            i += 3;
            continue;
        }
        out.push(kept[i].clone());
        i += 1;
    }
    let mut s = out.join("\n");
    if code.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// `code` with no top-level function that nothing names: its doc and attributes with it, again until none is left, as
/// the lean lexer holds only what its loop calls.
fn drop_uncalled_fns(code: &str) -> String {
    let mut lines: Vec<String> = code.lines().map(str::to_owned).collect();
    loop {
        // Each top-level function and one-line `static`: its first line (its doc's), its last line, its name.
        let mut items: Vec<(usize, usize, String)> = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let l = &lines[i];
            let name = l.strip_prefix("unsafe fn ").or_else(|| l.strip_prefix("fn ")).map(|r| r.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next().unwrap_or("").to_owned());
            let table = l.strip_prefix("static ").filter(|_| l.ends_with(';')).and_then(|r| r.split_once(':')).map(|(n, _)| n.to_owned());
            if let Some(name) = table {
                let mut first = i;
                while first > 0 && lines[first - 1].starts_with("///") {
                    first -= 1;
                }
                items.push((first, i, name));
                i += 1;
                continue;
            }
            if let Some(name) = name.filter(|n| !n.is_empty()) {
                let mut first = i;
                while first > 0 && (lines[first - 1].starts_with("///") || lines[first - 1].starts_with("#[")) {
                    first -= 1;
                }
                let Some(end) = (i..lines.len()).find(|&j| lines[j] == "}") else { break };
                items.push((first, end, name));
                i = end + 1;
                continue;
            }
            i += 1;
        }
        // A call of the function: its name and `(`, `::<S>(`, or the name given (`= lex_on, on …;`, as `dispatch!` takes
        // it); a table's name read (`&LO_0,`, `RARE_0.0`). Not in a comment, not a method's (`.any(`), not in its own
        // body (a variable of the same name is no call, nor is the function calling itself).
        let named = |n: &str, own: (usize, usize)| {
            lines
                .iter()
                .enumerate()
                .filter(|&(j, l)| !(own.0..=own.1).contains(&j) && !l.trim_start().starts_with("//"))
                .map(|(_, l)| {
                    l.match_indices(n)
                        .filter(|&(k, _)| {
                            let before = l[..k].chars().next_back();
                            !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') && {
                                let rest = &l[k + n.len()..];
                                rest.starts_with(['(', ',', ';']) || rest.starts_with("::<") || (n.bytes().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_') && !rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_'))
                            }
                        })
                        .count()
                })
                .sum::<usize>()
        };
        let gone: Vec<(usize, usize)> = items.iter().filter(|(a, b, n)| named(n, (*a, *b)) == 0).map(|&(a, b, _)| (a, b)).collect();
        if gone.is_empty() {
            break;
        }
        for &(a, b) in gone.iter().rev() {
            // The blank line after it too.
            let b = if lines.get(b + 1).is_some_and(|l| l.is_empty()) { b + 1 } else { b };
            lines.drain(a..=b);
        }
    }
    // The blank line before a last function gone too.
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let mut s = lines.join("\n");
    if code.ends_with('\n') {
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// JSON's records are `structural_start` where the tokens' starts differ from it only where an error refuses the
    /// file (`refused_equal`): without the error, not.
    #[test]
    fn records_are_structural_start_where_an_error_refuses_the_rest() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../json_parser/json-simdjson.bitlex");
        let source = std::fs::read_to_string(path).expect("reads");
        let stmts = ::bitlex::syntax::parse(&source).expect("parses");
        let starts = |stmts: &[Stmt]| stmts.iter().filter_map(|s| match s {
            Stmt::Token { rule: Some(TokenRule::Inside(..)), .. } => None,
            Stmt::Token { start, .. } => Some(start.clone()),
            _ => None,
        }).reduce(|a, b| Expr::Or(Box::new(a), Box::new(b))).expect("a token");
        let u = factored(&starts(&stmts), &stmts);
        assert!(matches!(refused_equal(&u, &stmts), Some(Expr::Name(n)) if n == "structural_start"));
        let without: Vec<Stmt> = stmts.into_iter().filter(|s| !matches!(s, Stmt::Error(why, _) if why == "a string right after a scalar")).collect();
        assert!(refused_equal(&factored(&starts(&without), &without), &without).is_none(), "no error refuses a quote after a scalar");
    }

    /// JSON's tier 1 refuses a string open at the end (its fill, as `in_string`, at the last byte): without that
    /// error, not.
    #[test]
    fn json_refuses_a_string_open_at_the_end() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../json_parser/json-simdjson.bitlex");
        let stmts = ::bitlex::syntax::parse(&std::fs::read_to_string(path).expect("reads")).expect("parses");
        assert!(refuses_open_at_end(&stmts, "str"));
        let without: Vec<Stmt> = stmts.into_iter().filter(|s| !matches!(s, Stmt::Error(why, _) if why == "a string open at the end of the file")).collect();
        assert!(!refuses_open_at_end(&without, "str"));
    }

    /// The dirty end of a scalar (`[1"a"]`) is tier 2's (`run_end_covers`): JSON's error "a string right after a
    /// scalar" is the one covered, and `ends_after_run` is simdjson's structural or whitespace. On every file of up to
    /// five bytes of `1"\ [a`, by the grammar's reference evaluator: where the error holds, a run token ends before a
    /// byte it may not end before (tier 2 refuses there); and where one does, the file is refused (an error, or the
    /// tree), so tier 2's refusal is the grammar's.
    #[test]
    fn a_scalars_dirty_end_is_tier_2s() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../json_parser/json-simdjson.bitlex");
        let stmts = ::bitlex::syntax::parse(&std::fs::read_to_string(path).expect("reads")).expect("parses");
        let covered = run_end_covers(&stmts);
        assert_eq!(covered.len(), 1);
        assert!(matches!(&stmts[covered[0]], Stmt::Error(why, _) if why == "a string right after a scalar"));
        let run = stmts.iter().find_map(|s| if let Stmt::Token { rule: Some(TokenRule::Run(r)), .. } = s { Some(r.clone()) } else { None }).expect("a run");
        let (ok, r) = (ends_after_run(&stmts, &run).expect("the ends"), bytes_of_expr(&run, &stmts).expect("its bytes"));
        let structural_or_ws = |b: u8| b"{}[],: \t\n\r".contains(&b);
        assert!((0..=255u8).all(|b| ok[usize::from(b)] == structural_or_ws(b)), "simdjson's structural or whitespace");
        let tables = HashMap::new();
        let alphabet = b"1\"\\ [a";
        let (mut files, mut dirty) = (0, 0);
        for len in 1..=5u32 {
            for k in 0..(alphabet.len() as u32).pow(len) {
                let src: Vec<u8> = (0..len).map(|d| alphabet[(k / (alphabet.len() as u32).pow(d)) as usize % alphabet.len()]).collect();
                let out = ::bitlex::eval::Ev::run(&stmts, &src, &tables).expect("runs");
                let error = out.errors.iter().any(|e| e == "a string right after a scalar");
                let ends_dirty = out.tokens.iter().any(|t| !t.mark && ["number", "true", "false", "null"].contains(&t.kind.as_str()) && src.get(t.end).is_some_and(|&b| !ok[usize::from(b)] && !r[usize::from(b)]));
                assert!(!error || ends_dirty, "{:?}: the error, and no run's dirty end", String::from_utf8_lossy(&src));
                if ends_dirty {
                    let refused = !out.errors.is_empty() || out.tree.as_ref().is_none_or(|d| ::bitlex::tree::nodes(d, &out.tokens, src.len()).is_err());
                    assert!(refused, "{:?}: a run's dirty end in a file the grammar takes", String::from_utf8_lossy(&src));
                    dirty += 1;
                }
                files += 1;
            }
        }
        assert!(dirty > 100, "{dirty} of {files}");
    }

    /// A mask's `& lv` dropped: where it ends an operand, and `(!x)` then `!x`; not where `lv` begins a longer name.
    #[test]
    fn without_lv_drops_the_files_bits() {
        assert_eq!(without_lv("(v_m_op | v_m_scalar & (!v_m_follows & lv)) & (!v_m_string_tail & lv)"), "(v_m_op | v_m_scalar & !v_m_follows) & !v_m_string_tail");
        assert_eq!(without_lv("{\n    x & lv\n}"), "{\n    x\n}");
        assert_eq!(without_lv("a & lv_1 & lvx & lv[i]"), "a & lv_1 & lvx & lv[i]");
        assert_eq!(without_lv("!(a | b) & lv"), "!(a | b)");
    }

    /// JSON's records and errors hold at no space whatever the carried masks hold; a class holding the space does.
    #[test]
    fn outputs_false_at_the_padding() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../json_parser/json-simdjson.bitlex");
        let stmts = ::bitlex::syntax::parse(&std::fs::read_to_string(path).expect("reads")).expect("parses");
        let name = |n: &str| Expr::Name(n.to_owned());
        assert!(false_at(&name("structural_start"), b' ', &stmts));
        assert!(false_at(&Expr::And(Box::new(name("in_string")), Box::new(name("control"))), b' ', &stmts));
        assert!(!false_at(&name("ws"), b' ', &stmts));
        assert!(!false_at(&name("in_string"), b' ', &stmts), "a carried mask is free");
    }
}
