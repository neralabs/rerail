//! Tier 1 as Rust, written by the Bitlex compiler from the grammar's lexer statements, in the lean
//! lexer's shape (`rerail_kotlin_lean::lex`), never lowered from the block program.
//!
//! - **The masks.** Each `class` is a bit a byte, found 64 bytes at a time (simdjson's
//!   `classify`); each `mask` is its expression over the masks before it, a word at a time,
//!   written from its statement: `prev` and `next` read the word before or after, `fill` and
//!   `escaped` carry from word to word.
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

struct Gen<'a> {
    x: &'a Input<'a>,
    /// The body of `lex` so far: the masks, in the statements' order.
    body: String,
    /// Each mask's local, by its grammar name.
    names: HashMap<String, String>,
    /// Each expression made a local of its own, by its text.
    temps: HashMap<String, String>,
    next: usize,
    /// `prefix`'s bytes (decision 0335).
    prefix: Option<Vec<u8>>,
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
    /// A local holding `e`'s words.
    fn val(&mut self, e: &Expr) -> Result<String, String> {
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
                "text" => {
                    let Some(Arg::S(t)) = args.first() else { return Err("tier 1 as Rust: `text` needs a string".into()) };
                    let _ = writeln!(self.body, "    let {name} = text(src, b\"{}\");", lit(t));
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
        for r in ks {
            self.names.insert(format!("o_{}", r.kind), format!("m_o_{}", ident(&r.kind)));
            self.names.insert(format!("f_{}", r.kind), format!("m_f_{}", ident(&r.kind)));
            if let Some((e, _)) = &r.empty {
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

    /// The prefix's bytes out of the file's bits (`prefix`, decision 0335): no class, `~` or token has them, and
    /// positions stay the file's.
    fn prefix_code(&self) -> String {
        match &self.prefix {
            Some(b) => format!(
                "    // `prefix`: a file that starts with these bytes is read from after them.\n    if src.starts_with(&[{}]) {{\n        lv[0] &= !((1u64 << {}) - 1);\n    }}\n",
                b.iter().map(|x| format!("{x:#04x}")).collect::<Vec<_>>().join(", "),
                b.len()
            ),
            None => String::new(),
        }
    }

    /// Tier 1 with no `skip`: simdjson's stage 1. Its records are each token's first byte, the bits of the rows'
    /// starts (simdjson's structural indexes), each with its row's kind; tier 2 finds a token's end, from the masks
    /// tier 1 gives it (`stage1_ends`, `readers`), as simdjson's stage 2 does.
    fn stage1(mut self, rows: &[(usize, &Stmt)]) -> Result<String, String> {
        let x = self.x;
        let mut all: Option<Expr> = None;
        let mut chain = String::new();
        for (row, s) in rows {
            let Stmt::Token { kind, start, rule, .. } = s else { continue };
            if matches!(rule, Some(TokenRule::Inside(..))) {
                continue;
            }
            let st = self.val(start)?;
            let k = self.kind_of(*row);
            let _ = write!(chain, "{}if bit(&{st}, s) {{\n                // `{kind}`\n                {k}\n            }} ", if chain.is_empty() { "" } else { "else " });
            all = Some(match all.take() {
                Some(a) => Expr::Or(Box::new(a), Box::new(start.clone())),
                None => start.clone(),
            });
        }
        let all = self.val(&all.ok_or("tier 1 as Rust: no token")?)?;
        // The masks tier 2 reads: a reader's, then each row's end's.
        let mut fields: Vec<(String, Expr)> = readers(x.stmts).into_iter().flat_map(|r| r.masks).collect();
        fields.extend(stage1_ends(x.stmts)?);
        let mut used: Vec<String> = Vec::new();
        let mut fields_init = String::new();
        let mut decl = String::new();
        for (f, e) in &fields {
            let v = self.val(e)?;
            let v = if used.contains(&v) { format!("{v}.clone()") } else { v };
            used.push(v.clone());
            let _ = writeln!(decl, "    pub {f}: Vec<u64>,");
            let _ = writeln!(fields_init, "            {f}: {v},");
        }
        let mut out = String::new();
        out.push_str("// Generated by the Bitlex compiler (`rerail_compiler::bitlex_lex_rust`) from the grammar's lexer statements. Do not edit.\n\n");
        let _ = write!(out, "/// The masks tier 2 reads, a bit a byte: those it finds a token's end by, and splits a region by.\npub struct Masks {{\n{decl}}}\n\n");
        out.push_str("/// Tier 1's records, simdjson's stage 1: each token's first byte and its kind, the masks tier 2 reads, and whether the file goes to tree-sitter.\npub struct Lexed {\n    pub starts: Vec<u32>,\n    pub codes: Vec<u32>,\n    pub masks: Masks,\n    pub refused: bool,\n}\n\n");
        let kinds: Vec<String> = x.kinds.iter().map(|k| k.to_string()).collect();
        let _ = write!(out, "/// Each byte's record kind, as a token's first byte.\nconst KINDS: [u8; 256] = [{}];\n\n", kinds.join(", "));
        out.push_str(&format!("/// `src`'s records.\npub fn lex(src: &[u8]) -> Lexed {{\n    let n = src.len();\n    let nw = n.div_ceil(64);\n    // Each word's bits inside the file.\n    let {}lv: Vec<u64> = (0..nw).map(|q| if 64 * q + 64 <= n {{ !0 }} else {{ (1u64 << (n % 64)) - 1 }}).collect();\n", if self.prefix.is_some() { "mut " } else { "" }));
        out.push_str(&self.prefix_code());
        out.push_str(&self.body);
        let _ = write!(
            out,
            "    // The tokens: each bit of a row's start, in order, its kind its row's.\n    let mut starts: Vec<u32> = Vec::with_capacity(n / 4 + 1);\n    let mut codes: Vec<u32> = Vec::with_capacity(n / 4 + 1);\n    for (q, &word) in {all}.iter().enumerate() {{\n        let mut bits = word;\n        while bits != 0 {{\n            let s = 64 * q + bits.trailing_zeros() as usize;\n            bits &= bits - 1;\n            let kind: u32 = {chain}else {{\n                0\n            }};\n            starts.push(s as u32);\n            codes.push(kind);\n        }}\n    }}\n    Lexed {{\n        starts,\n        codes,\n        masks: Masks {{\n{fields_init}        }},\n        refused,\n    }}\n}}\n"
        );
        out.push_str(HELPERS);
        Ok(out)
    }

    fn source(mut self) -> Result<String, String> {
        let x = self.x;
        // The classes, all in one pass over the blocks (simdjson's `classify`).
        let mut classes = String::new();
        for s in x.stmts {
            if let Stmt::Class(name, items) = s {
                let local = format!("m_{}", ident(name));
                let ranges: Vec<String> = items.iter().map(|&(lo, hi)| format!("({lo:#04x}, {hi:#04x})")).collect();
                let _ = writeln!(self.body, "    let mut {local} = vec![0u64; nw];");
                let _ = writeln!(classes, "        {local}[q] = bytes_in(&b, &[{}]) & lv[q];", ranges.join(", "));
                self.names.insert(name.clone(), local);
            }
        }
        let _ = writeln!(self.body, "    for q in 0..nw {{\n        let b = block(src, q);\n{classes}    }}");
        let mut skip: Option<Expr> = None;
        let mut marks: Vec<&Stmt> = Vec::new();
        let mut rows: Vec<(usize, &Stmt)> = Vec::new();
        let mut row = 0usize;
        let mut escapes: Vec<(&str, &Expr)> = Vec::new();
        let mut token_escapes: Vec<&Stmt> = Vec::new();
        for s in x.stmts {
            match s {
                // A token's check is tier 2's, in the token's arm.
                Stmt::Lexer(_) | Stmt::Class(..) | Stmt::Keywords { .. } | Stmt::TokenError(..) | Stmt::Tree { .. } | Stmt::Check { .. } => {}
                Stmt::Prefix(b) => self.prefix = Some(b.clone()),
                Stmt::Mask(name, e) => {
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
        let mut refuse = Vec::new();
        for (why, e) in &escapes {
            let v = self.val(e)?;
            refuse.push(format!("    // {why}\n    refused |= any(&{v});\n"));
        }
        let _ = writeln!(self.body, "    let mut refused = false;");
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
            None if marks.is_empty() && token_escapes.is_empty() => return self.stage1(&rows),
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
        // The token loop: each top row's branch.
        let mut branches = String::new();
        let mut pair_fns = String::new();
        for (row, s) in &rows {
            let Stmt::Token { kind, start, last, rule, .. } = s else { continue };
            if matches!(rule, Some(TokenRule::Inside(..))) {
                continue;
            }
            let st = self.val(start)?;
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
                        let o = self.val(&Expr::Name(format!("o_{}", r.kind)))?;
                        let c = self.val(&r.closers())?;
                        let found = if r.outside { "c - 1".to_owned() } else { self.closer_last(r, &[])? };
                        let none = if r.outside { "n - 1".to_owned() } else { "{\n                    refused = true;\n                    break;\n                }".to_owned() };
                        let one = format!("match first_bit(&{c}, s + 1) {{\n                Some(c) => {found},\n                None => {none}\n            }}");
                        if rs.len() == 1 {
                            code = one;
                        } else {
                            let _ = write!(code, "{}if bit(&{o}, s) {{\n                {}\n            }} ", if i == 0 { "" } else { "else " }, indent(&one, "    ").trim_start());
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
                "{}if bit(&{st}, s) {{\n            // `{kind}`: {how}\n            let l = {l};\n            let l = l.max(s);\n{hits}            (l, {k}, {extra}, {bits:#x})\n        }} ",
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
        let mut out = String::new();
        out.push_str("// Generated by native/crates/compiler/src/bitlex_lex_rust.rs; do not edit. The Bitlex compiler's Rust from the grammar's lexer statements.\n\n");
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
        out.push_str(&format!("/// `src`'s records.\npub fn lex(src: &[u8]) -> Lexed {{\n    let n = src.len();\n    let nw = n.div_ceil(64);\n    // Each word's bits inside the file.\n    let {}lv: Vec<u64> = (0..nw).map(|q| if 64 * q + 64 <= n {{ !0 }} else {{ (1u64 << (n % 64)) - 1 }}).collect();\n", if self.prefix.is_some() { "mut " } else { "" }));
        out.push_str(&self.prefix_code());
        out.push_str(&self.body);
        let _ = write!(
            out,
            "    // The tokens: from the first byte `skip` does not hold, the token whose start holds there, to its last byte by its rule.\n    let mut starts: Vec<u32> = Vec::with_capacity(n / 4 + 1);\n    let mut lasts: Vec<u32> = Vec::with_capacity(n / 4 + 1);\n    let mut codes: Vec<u32> = Vec::with_capacity(n / 4 + 1);\n    let mut at = 0usize;\n    // The gap's start, since the grammar token before and since the record before; the grammar token before's word; the marks its tests carry to the next grammar token.\n    let (mut from, mut from_record, mut before, mut pending) = (0usize, 0usize, 0u32, 0u32);\n    while let Some(s) = first_clear(&{skip}, at, n) {{\n        let mut hits = 0u32;\n        let (l, kind, extra, mut marks): (usize, u32, bool, u32) = {branches}else {{\n            // A byte no token begins with.\n            refused = true;\n            break;\n        }};\n        let word = if extra {{ 0 }} else {{ word(&src[s..=l]) }};\n        // The token marks: tier 2 tests the rest of each.\n{token_code}        if extra {{\n            // An extra keeps the marks of its own gap; its tests' hits go to the next grammar token.\n{own}            pending |= hits & 0xffff | hits >> 16;\n        }} else {{\n            let tested = pending | hits;\n{gap_code}            from = s + 1;\n            before = word;\n            pending = hits >> 16;\n        }}\n        from_record = s + 1;\n        starts.push(s as u32);\n        lasts.push(l as u32);\n        codes.push(kind | word << 8 | marks << 16);\n        at = l + 1;\n    }}\n    let _ = (from, from_record, before, pending);\n    Lexed {{\n        starts,\n        lasts,\n        codes,\n        masks: Masks {{\n{fields_init}        }},\n        refused,\n    }}\n}}\n",
            own = own_code
        );
        out.push_str(HELPERS);
        Ok(out)
    }
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

/// The bytes of a block in any of the ranges, a bit a byte: on arm64 a subtract and a compare a
/// range, 16 bytes at a time, and simdjson's `to_bitmask`.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
fn bytes_in(b: &[u8; 64], ranges: &[(u8, u8)]) -> u64 {
    use core::arch::aarch64::*;
    // SAFETY: NEON is in the aarch64 baseline; each load reads 16 bytes of the block.
    unsafe {
        let at = |o: usize| {
            let d = vld1q_u8(b.as_ptr().add(o));
            let mut m = vdupq_n_u8(0);
            for &(lo, hi) in ranges {
                m = vorrq_u8(m, vcleq_u8(vsubq_u8(d, vdupq_n_u8(lo)), vdupq_n_u8(hi - lo)));
            }
            m
        };
        const WEIGHTS: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];
        let w = vld1q_u8(WEIGHTS.as_ptr());
        let s0 = vpaddq_u8(vandq_u8(at(0), w), vandq_u8(at(16), w));
        let s1 = vpaddq_u8(vandq_u8(at(32), w), vandq_u8(at(48), w));
        let s = vpaddq_u8(s0, s1);
        let s = vpaddq_u8(s, s);
        vgetq_lane_u64::<0>(vreinterpretq_u64_u8(s))
    }
}

/// Off arm64, the ranges a byte at a time (not a measured path).
#[cfg(not(target_arch = "aarch64"))]
fn bytes_in(b: &[u8; 64], ranges: &[(u8, u8)]) -> u64 {
    b.iter().enumerate().fold(0, |m, (j, &c)| m | u64::from(ranges.iter().any(|&(lo, hi)| lo <= c && c <= hi)) << j)
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

/// `text(t)`: the bytes `t` starts at.
fn text(src: &[u8], t: &[u8]) -> Vec<u64> {
    let nw = src.len().div_ceil(64);
    let eqs: Vec<Vec<u64>> = t.iter().map(|&c| (0..nw).map(|q| bytes_in(&block(src, q), &[(c, c)])).collect()).collect();
    let lv = |q: usize| if 64 * q + 64 <= src.len() { !0 } else { (1u64 << (src.len() % 64)) - 1 };
    (0..nw)
        .map(|i| eqs.iter().enumerate().fold(lv(i), |m, (k, e)| m & if k == 0 { e[i] } else { next(e, i, k as u32) }))
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
    Gen { x, body: String::new(), names: HashMap::new(), temps: HashMap::new(), next: 0, prefix: None }.source()
}
