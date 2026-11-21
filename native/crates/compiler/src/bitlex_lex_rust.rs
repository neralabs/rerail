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
        self.make_node(key, name, show(e), e)
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
            return Ok(Some(format!("bit(&{}, s)", self.val(e)?)));
        }
        let Some(opens) = &self.opens else { return Ok(None) };
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
            Expr::Not(a) => self.at_start(a)?.map(|a| format!("!({a})")),
            Expr::And(a, b) => match (self.at_start(a)?, self.at_start(b)?) {
                // One region of several where the region opened is one of them: that one.
                (Some(a), Some(b)) => Some(match (regions_of(&a), regions_of(&b)) {
                    (Some(x), Some(y)) if y.len() == 1 && x.contains(&y[0]) => b,
                    (Some(x), Some(y)) if x.len() == 1 && y.contains(&x[0]) => a,
                    _ => format!("({a} && {b})"),
                }),
                _ => None,
            },
            Expr::Or(a, b) => match (self.at_start(a)?, self.at_start(b)?) {
                (Some(a), Some(b)) => Some(format!("({a} || {b})")),
                _ => None,
            },
            Expr::Xor(a, b) => match (self.at_start(a)?, self.at_start(b)?) {
                (Some(a), Some(b)) => Some(format!("({a} != {b})")),
                _ => None,
            },
            Expr::Call(..) | Expr::Test { .. } => None,
        })
    }

    /// `e` in one part of a token that reads region `k` (decision 0551's step B): at its start
    /// (`part` 0), after it to the closer (1), over the closer (2); and outside every region token
    /// (`k` none). There the regions' masks are known: a region's opener holds at its start, its
    /// fill from there to the closer, the hidden bytes after its start through its closer, none
    /// of them outside. `None`: `e` reads one of them shifted or carried.
    fn in_region(&self, e: &Expr, k: Option<usize>, part: u8) -> Option<Expr> {
        let all = || Expr::Not(Box::new(Expr::Empty));
        let opens = self.opens.as_ref()?;
        Some(match e {
            Expr::Name(n) if self.regional.contains(n) => {
                let kind = |p: &str| opens.iter().position(|(q, _, _)| *n == format!("{p}_{q}"));
                let holds = if let Some(q) = kind("o") {
                    k == Some(q) && part == 0
                } else if let Some(q) = kind("f") {
                    k == Some(q) && part < 2
                } else if let Some(d) = self.defs.get(n) {
                    return self.in_region(d, k, part);
                } else {
                    // The hidden bytes.
                    k.is_some() && part > 0
                };
                if holds { all() } else { Expr::Empty }
            }
            Expr::Name(_) | Expr::Empty => e.clone(),
            Expr::Not(a) => simplify(Expr::Not(Box::new(self.in_region(a, k, part)?))),
            Expr::And(a, b) => simplify(Expr::And(Box::new(self.in_region(a, k, part)?), Box::new(self.in_region(b, k, part)?))),
            Expr::Or(a, b) => simplify(Expr::Or(Box::new(self.in_region(a, k, part)?), Box::new(self.in_region(b, k, part)?))),
            Expr::Xor(a, b) => simplify(Expr::Xor(Box::new(self.in_region(a, k, part)?), Box::new(self.in_region(b, k, part)?))),
            Expr::Call(..) if !self.regional(e) => e.clone(),
            Expr::Call(..) | Expr::Test { .. } => return None,
        })
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
    fn region_escape(&mut self, why: &str, e: &Expr, read: &[usize]) -> Result<bool, String> {
        let Some(n) = self.opens.as_ref().map(|o| o.len()) else { return Ok(false) };
        if !matches!(self.in_region(e, None, 0), Some(Expr::Empty)) {
            return Ok(false);
        }
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
                Stmt::Lexer(_) | Stmt::Class(..) | Stmt::Keywords { .. } | Stmt::TokenError(..) | Stmt::Tree { .. } => {}
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
        let mut refuse = Vec::new();
        for (why, e) in &escapes {
            if !self.regional(e) {
                let id = self.node(e)?;
                self.sinks.push((why.to_string(), id));
                continue;
            }
            if self.region_escape(why, e, &read)? {
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
        }
        let mut gaps: Vec<Gap> = Vec::new();
        // Each mark's token tests: the kind, `after`, the test, the mask, the least, the bit.
        let mut tests: Vec<(String, bool, TokenTest, String, u32, u8)> = Vec::new();
        for (m, s) in marks.iter().enumerate() {
            let Stmt::Mark { kind, at, unless, unplaced, checks, .. } = s else { continue };
            let Some(bit) = x.mark_bits.get(m).copied().flatten() else { continue };
            if unplaced.is_some() {
                return Err(format!("tier 1 as Rust: mark {kind}'s `else escape` is not written yet"));
            }
            let mask = match at {
                Expr::Empty => None,
                e => Some(self.val(e)?),
            };
            let unless = unless
                .iter()
                .map(|w| x.words.iter().position(|x| x == w).map(|i| i as u32 + 1).ok_or_else(|| format!("tier 1 as Rust: mark {kind}: `unless after` a word no code names")))
                .collect::<Result<_, _>>()?;
            for c in checks {
                let v = self.val(&c.expr)?;
                tests.push((c.kind.clone(), c.after, c.test, v, c.least, bit));
            }
            gaps.push(Gap { bit, mask, unless, name: kind.clone() });
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
        for (row, s) in &rows {
            let Stmt::Token { kind, start, last, rule, .. } = s else { continue };
            if matches!(rule, Some(TokenRule::Inside(..))) {
                continue;
            }
            let st = match self.at_start(start)? {
                Some(t) => t,
                None => format!("bit(&{}, s)", self.val(start)?),
            };
            let k = self.kind_of(*row);
            let (how, l) = match rule {
                Some(TokenRule::Run(r)) => (format!("run {}", show(r)), format!("run_last(&{}, s)", self.val(r)?)),
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
                let holds = match test {
                    TokenTest::Count => format!("count_in(&{v}, s, l) >= {least}"),
                    TokenTest::Run => format!("l + 1 - s >= {least} && first_clear(&{v}, s, s + {least}).is_none()"),
                    TokenTest::All => format!("l + 1 - s >= {least} && first_clear(&{v}, s, l + 1).is_none()"),
                };
                let at = if *after { 16 + u32::from(*bit) } else { u32::from(*bit) };
                let _ = write!(hits, "            if {holds} {{\n                hits |= 1 << {at};\n            }}\n");
            }
            for t in &token_escapes {
                let Stmt::TokenEscape { why, kind: tk, test, expr, least } = t else { continue };
                if tk != kind {
                    continue;
                }
                let v = self.val(expr)?;
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
            let since = match &g.mask {
                Some(m) => format!("tested >> {0} & 1 == 1 || any_in(&{m}, from, s)", g.bit),
                None => format!("tested >> {} & 1 == 1", g.bit),
            };
            let unless = if g.unless.is_empty() { String::new() } else { format!(" && !matches!(before, {})", g.unless.iter().map(|w| w.to_string()).collect::<Vec<_>>().join(" | ")) };
            let _ = write!(gap_code, "                // `{}`\n                if ({since}){unless} {{\n                    marks |= 1 << {};\n                }}\n", g.name, g.bit);
            if x.own >> g.bit & 1 == 1
                && let Some(m) = &g.mask
            {
                let _ = write!(own_code, "                if any_in(&{m}, from_record, s) {{\n                    marks |= 1 << {};\n                }}\n", g.bit);
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
                    Some(format!("bit(&{}, s)", self.val(&e)?))
                }
            };
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
            "    // The tokens: from the first byte `skip` does not hold, the token whose start holds there, to its last byte by its rule.\n    let mut starts: Vec<u32> = Vec::with_capacity(n / 4 + 1);\n    let mut lasts: Vec<u32> = Vec::with_capacity(n / 4 + 1);\n    let mut codes: Vec<u32> = Vec::with_capacity(n / 4 + 1);\n    let mut at = 0usize;\n    // The gap's start, since the grammar token before and since the record before; the grammar token before's word; the marks its tests carry to the next grammar token.\n    let (mut from, mut from_record, mut before, mut pending) = (0usize, 0usize, 0u32, 0u32);\n{plain_decl}    while let Some(s) = first_clear(&{skip}, at, n) {{\n{open_code}        let mut hits = 0u32;\n        let (l, kind, extra, mut marks): (usize, u32, bool, u32) = {branches}else {{\n            // A byte no token begins with.\n            refused = true;\n            break;\n        }};\n        let word = if extra {{ 0 }} else {{ word(&src[s..=l]) }};\n        // The token marks: tier 2 tests the rest of each.\n{token_code}        if extra {{\n            // An extra keeps the marks of its own gap; its tests' hits go to the next grammar token.\n{own}            pending |= hits & 0xffff | hits >> 16;\n        }} else {{\n            let tested = pending | hits;\n{gap_code}            from = s + 1;\n            before = word;\n            pending = hits >> 16;\n        }}\n        from_record = s + 1;\n        starts.push(s as u32);\n        lasts.push(l as u32);\n        codes.push(kind | word << 8 | marks << 16);\n        at = l + 1;\n    }}\n    let _ = (from, from_record, before, pending);\n    Lexed {{\n        starts,\n        lasts,\n        codes,\n        masks: Masks {{\n{fields_init}        }},\n        refused,\n    }}\n}}\n",
            own = own_code,
            plain_decl = if self.opens.is_some() { "    // The region opened last of the kinds with no `after`.\n    let mut plain = usize::MAX;\n" } else { "" },
        );
        out.push_str(HELPERS);
        Ok(out)
    }
}

/// `e` with its constants folded: `empty` (none) and `~empty` (every byte).
fn simplify(e: Expr) -> Expr {
    let all = |x: &Expr| matches!(x, Expr::Not(a) if matches!(**a, Expr::Empty));
    match e {
        Expr::Not(a) => match *a {
            Expr::Not(b) if all(&Expr::Not(b.clone())) => Expr::Empty,
            a => Expr::Not(Box::new(a)),
        },
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
    let t = t.strip_prefix('(').and_then(|t| t.strip_suffix(')')).unwrap_or(t);
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
    Gen { x, body: String::new(), names: HashMap::new(), temps: HashMap::new(), next: 0, nodes: Vec::new(), node_of: HashMap::new(), regional: Default::default(), pairs: Vec::new(), sinks: Vec::new(), opens: None, defs: HashMap::new(), region_checks: Vec::new() }.source()
}
