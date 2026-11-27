//! The one pass's masks read from the bytes where the token loop reads them (decision 0623 step 1), as the lean
//! lexer reads its window's: no stage 1 and no mask of the whole file.
//!
//! - A test at a byte (`bit(&M, i)`) is a test of the bytes there, as lean's `c == b'/' && d == b'/'`.
//! - A search or a count over bytes (`first_bit`, `count_in`, `first_in`, `first_clear`, `any_in`, a run's end) reads
//!   the mask 64 bytes at a time from its first byte, each window made of the bytes loaded where they are
//!   (`rerail_simd8`): a class by its byte ranges, `prev` and `next` as the window a byte back or on, `escaped` word by
//!   word with its carry from the search's first byte.
//! - A block's token starts (`!skip[b]`) are the block's window.
//!
//! Written where every mask the loop reads has that form; elsewhere the pass keeps stage 1 (`Gen::source`).

use std::collections::HashMap;
use std::fmt::Write as _;

use ::bitlex::syntax::{Arg, Expr, Stmt};

/// The masks' and the classes' definitions, the last of each name, and the window functions written so far.
pub(super) struct Bytes<'a> {
    classes: HashMap<&'a str, &'a [(u32, u32)]>,
    defs: HashMap<String, Expr>,
    /// Each mask's expression, by the local the token loop names it by (`m_NAME`, a temporary `tK`).
    locals: HashMap<String, Expr>,
    /// The window functions written, by the local.
    written: Vec<String>,
    /// Each class's window function, by name.
    cls: Vec<String>,
    code: String,
    /// The masks the loop reads from its block's word (`k_NAME`, made once a block, as the lean lexer's `Window`).
    block: Vec<String>,
    /// Each class's bits as stage 1 makes them of the byte vector `v` (`cP`: its nibble tables' pair `P`, simdjson's
    /// `classify`), by name.
    class_code: HashMap<String, String>,
    /// Whether the line read is in a region's row: its reads go from the opener, as the lean lexer's micro-lexers.
    in_region: bool,
    /// Whether a read with no form of the bytes stays as it is (`partial_windows`), its mask in `failed`.
    partial: bool,
    /// The run the last token's `run_last` read forward (`forward_run`): its mask and its terms. A test at its last
    /// byte `l` takes them there: the mask holds at `l`, at `l + 1` it and its terms do not, and its terms at `l` are
    /// those the run ended in (`end_last`).
    fwd: Option<(Expr, Vec<Expr>)>,
    /// Whether a run was read forward: the loop's blocks then hold `end_last`.
    fwd_used: bool,
    pub(super) failed: std::collections::HashSet<String>,
    /// The fills' and escapes' windows and tests written (`wf_*`, `af_*`).
    wmemo: HashMap<Expr, String>,
    /// Whether `code` is a closure's question of a byte (`code(q)`), as tier 2 asks it of its records (`code_at`): the
    /// tests then take it, and are named apart (`ca_*`).
    pub(super) code_fn: bool,
    /// A prefix of the tests' names (`at_*`, `ca_*`, `af_*`), where two sets of them are in one file.
    pub(super) tag: &'static str,
    amemo: std::cell::RefCell<HashMap<Expr, String>>,
    /// The operators' pairs masks the loop reads (`k_pair_K`): each row's pairs of bytes, a pair's first byte's bit where
    /// the second is the byte after (lean's `pair`), made of the block's classes too.
    pub(super) pairs: Vec<(String, Vec<(u8, u8)>)>,
    /// The window functions that read one window's escapes (`escaped_step` of a mask), by name, and that mask: each has
    /// a twin (`e_NAME`) given the window's escaped bytes, so tests read in one window loop find them once (`fuse_tests`).
    pub(super) escapes: HashMap<String, String>,
    /// The bytes each such mask holds, where it is a class (`escapes`' masks): a row whose first byte none of them may be
    /// starts no escape, so its region's end and tests read in one loop from its opener (`fuse_read`).
    pub(super) escape_bytes: HashMap<String, [bool; 256]>,
    /// The bytes a rare token starts with (the lean lexer's `rare`, decision 0655 step 4): where set, a block word of its
    /// own (`k_rare`), of its nibble tables.
    pub(super) rare: Option<[bool; 256]>,
    /// The lean lexer's reads (decision 0655 step 7): where set, a search, a count or a run of a mask is lean's: a
    /// micro-lexer (`micro::find`, `find_pair`, `string_end`, a fence) where the mask is one, else a loop of the bytes,
    /// each tested where it is. No window of the mask.
    pub(super) lean: bool,
    /// A fence of three bytes or more searched (`find_fence`, written once).
    pub(super) fence: bool,
    /// The named masks a loop of the bytes tests by a function of their own (a long test, `h4`), each written once:
    /// its name and its code.
    named: std::cell::RefCell<Vec<(String, String)>>,
    /// The classes of more than one byte the lean lexer's loop tests of a byte, by their bit in the byte's `CLASS`
    /// entry (`C_NAME`), in the order first tested, as the lean lexer's `CLASS` table.
    scalar: std::cell::RefCell<Vec<String>>,
    /// Each row's first bytes, by its kind (`// `kind`: …`), and the row the line read is in.
    pub(super) row_firsts: HashMap<String, [bool; 256]>,
    row: Option<String>,
}

/// The calls of the loop that read a mask: their name, and how many of their first arguments are masks.
const READS: [(&str, usize); 11] =
    [("run_once_last", 2), ("run_once_hit", 2), ("first_clear", 1), ("first_bit", 1), ("count_in", 1), ("first_in", 1), ("run_last", 1), ("any_in", 1), ("bit", 1), ("put", 1), ("last_in", 1)];

impl<'a> Bytes<'a> {
    pub(super) fn new(stmts: &'a [Stmt], locals: HashMap<String, Expr>, class_code: HashMap<String, String>) -> Self {
        let mut classes = HashMap::new();
        let mut defs = HashMap::new();
        for s in stmts {
            match s {
                Stmt::Class(n, rs) => {
                    classes.insert(n.as_str(), rs.as_slice());
                    defs.remove(n);
                }
                Stmt::Mask(n, e) => {
                    defs.insert(n.clone(), e.clone());
                    classes.remove(n.as_str());
                }
                _ => {}
            }
        }
        Bytes { classes, defs, locals, written: Vec::new(), cls: Vec::new(), code: String::new(), block: Vec::new(), class_code, in_region: false, pairs: Vec::new(), escapes: HashMap::new(), escape_bytes: HashMap::new(), rare: None, lean: false, fence: false, named: Default::default(), scalar: Default::default(), row_firsts: HashMap::new(), row: None, partial: false, failed: Default::default(), wmemo: HashMap::new(), amemo: Default::default(), code_fn: false, tag: "", fwd: None, fwd_used: false }
    }

    /// `code` with each read of the masks it has (`locals`) in the windows' form, no block's words (decision 0703): a
    /// read of another mask, or one with no such form, stays as it is, the mask then in `failed`.
    /// Reads with no form of the bytes stay as they are (`partial_windows`), and each mask's test is written once.
    pub(super) fn set_partial(&mut self) {
        self.partial = true;
    }

    /// The tests written once (`at_*`, `af_*`, `ca_*`), for the file.
    pub(super) fn named_code(&self) -> String {
        self.named.borrow().iter().map(|x| x.1.as_str()).collect()
    }

    pub(super) fn partial_windows(&mut self, code: &str) -> String {
        self.partial_line(code, true)
    }

    /// `partial_windows` of a line of the token loop: a token of code's (`in_region` false) reads a mask with no escapes
    /// from its block's word (`k_M`, made once a block, as the lean lexer's `lex_blocks` reads its block's masks), then
    /// windows past the block.
    pub(super) fn partial_line(&mut self, code: &str, in_region: bool) -> String {
        self.in_region = in_region;
        self.partial = true;
        let out = self.rewrite_line(code, code).unwrap_or_else(|_| code.to_owned());
        self.partial = false;
        self.in_region = false;
        out
    }

    /// Class `n` tested of byte `b` (`src[i]`) by its bit in the byte's `CLASS` entry, as the lean lexer tests a class:
    /// one load and its bit.
    fn class_bit(&self, n: &str, b: &str) -> String {
        if !self.scalar.borrow().iter().any(|x| x == n) {
            self.scalar.borrow_mut().push(n.to_owned());
        }
        format!("CLASS[usize::from({b})] & C_{} != 0", n.to_uppercase())
    }

    /// The `CLASS` table of the classes tested by their bits (`class_bit`), built at compile time as the lean lexer's.
    fn class_table(&self) -> String {
        let scalar = self.scalar.borrow();
        if scalar.is_empty() {
            return String::new();
        }
        let ty = match scalar.len() {
            0..=8 => "u8",
            9..=16 => "u16",
            _ => "u32",
        };
        let mut out = String::new();
        for (k, n) in scalar.iter().enumerate() {
            let _ = write!(out, "/// `{n}`: its bit in a byte's `CLASS` entry.\nconst C_{}: {ty} = {:#x};\n", n.to_uppercase(), 1u64 << k);
        }
        let _ = write!(out, "\n/// Each byte's classes the token loop tests of one byte, a bit a class, as the lean lexer's `CLASS`.\nstatic CLASS: [{ty}; 256] = {{\n    let mut c = [0{ty}; 256];\n    let mut i = 0;\n    while i < 256 {{\n        let b = i as u8;\n");
        for n in scalar.iter() {
            let rs = self.classes.get(n.as_str()).copied().unwrap_or_default();
            let _ = write!(out, "        if {} {{\n            c[i] |= C_{};\n        }}\n", pattern_test("b", rs), n.to_uppercase());
        }
        out.push_str("        i += 1;\n    }\n    c\n};\n\n");
        out
    }

    /// The bytes `e` can hold at its first byte: `byte_set`'s, a test of other bytes (`next`, `prev`) in an `&` dropped,
    /// so the set holds every byte `e` holds at (and more where a test was dropped).
    pub(super) fn first_bytes(&self, e: &Expr, depth: usize) -> Option<[bool; 256]> {
        if depth > 64 {
            return None;
        }
        match e {
            Expr::Name(n) if !self.classes.contains_key(n.as_str()) => self.first_bytes(self.defs.get(n)?, depth + 1),
            Expr::And(a, b) => match (self.first_bytes(a, depth + 1), self.first_bytes(b, depth + 1)) {
                (Some(x), Some(y)) => Some(std::array::from_fn(|c| x[c] && y[c])),
                (Some(x), None) | (None, Some(x)) => Some(x),
                (None, None) => None,
            },
            Expr::Or(a, b) => {
                let (a, b) = (self.first_bytes(a, depth + 1)?, self.first_bytes(b, depth + 1)?);
                Some(std::array::from_fn(|c| a[c] || b[c]))
            }
            _ => self.byte_set(e, depth),
        }
    }

    /// The bytes `e` holds at a byte, where it reads that byte alone (its classes, `~`, `&`, `|`).
    fn byte_set(&self, e: &Expr, depth: usize) -> Option<[bool; 256]> {
        if depth > 64 {
            return None;
        }
        Some(match e {
            Expr::Name(n) => match self.classes.get(n.as_str()) {
                Some(rs) => std::array::from_fn(|c| rs.iter().any(|&(lo, hi)| (lo..=hi).contains(&(c as u32)))),
                None => self.byte_set(self.defs.get(n)?, depth + 1)?,
            },
            Expr::Not(a) => self.byte_set(a, depth + 1)?.map(|x| !x),
            Expr::And(a, b) => {
                let (a, b) = (self.byte_set(a, depth + 1)?, self.byte_set(b, depth + 1)?);
                std::array::from_fn(|c| a[c] && b[c])
            }
            Expr::Or(a, b) => {
                let (a, b) = (self.byte_set(a, depth + 1)?, self.byte_set(b, depth + 1)?);
                std::array::from_fn(|c| a[c] || b[c])
            }
            _ => return None,
        })
    }

    /// A run of `R` with one `D` at most (`run R once D`) in the lean lexer's form, where `R` is `A | (D & X)` and no
    /// byte is both `A` and `D`: the run of `A`, then, where `D` and `X` hold at the byte after it, that byte and the
    /// run of `A` again, as the lean lexer reads a number's digits and then its `.` and fraction. `X`'s `prev(A)` holds
    /// after the run where the row's first bytes are `A`, and its `next(A)` is the byte the second run starts after.
    /// With `hit`, whether it read its `D` too (`(l, once)`).
    fn two_runs(&self, r: &str, d: &str, s: &str, hit: bool) -> Option<String> {
        let (r, d) = (self.resolved(&self.expr(r).ok()?, 0)?, self.resolved(&self.expr(d).ok()?, 0)?);
        fn ors(e: &Expr, out: &mut Vec<Expr>) {
            match e {
                Expr::Or(a, b) => {
                    ors(a, out);
                    ors(b, out);
                }
                e => out.push(e.clone()),
            }
        }
        fn ands(e: &Expr, out: &mut Vec<Expr>) {
            match e {
                Expr::And(a, b) => {
                    ands(a, out);
                    ands(b, out);
                }
                e => out.push(e.clone()),
            }
        }
        let mut alts = Vec::new();
        ors(&r, &mut alts);
        let same = |a: &Expr, b: &Expr| format!("{a:?}") == format!("{b:?}");
        let k = alts.iter().position(|e| {
            let mut c = Vec::new();
            ands(e, &mut c);
            c.iter().any(|x| same(x, &d))
        })?;
        let with_d = alts.remove(k);
        let a = alts.into_iter().reduce(|x, y| Expr::Or(Box::new(x), Box::new(y)))?;
        let (sa, sd) = (self.byte_set(&a, 0)?, self.byte_set(&d, 0)?);
        if (0..256).any(|c| sa[c] && sd[c]) {
            return None;
        }
        let mut x = Vec::new();
        ands(&with_d, &mut x);
        x.retain(|e| !same(e, &d));
        let shifted = |e: &Expr, k: i64| matches!(e, Expr::Call(f, args) if Self::shift(f, args).is_some_and(|(b, j)| j == k && same(&b, &a)));
        let firsts_a = self.row.as_ref().and_then(|r| self.row_firsts.get(r)).is_some_and(|f| (0..256).all(|c| !f[c] || sa[c]));
        if firsts_a {
            x.retain(|e| !shifted(e, -1));
        }
        let step = if x.iter().any(|e| shifted(e, 1)) { 2 } else { 1 };
        let mut none = None;
        let pa = self.pred(&a, "(l + 1)", "(l + 1)", &mut none, 0)?;
        let mut cond = self.pred(&d, "(l + 1)", "(l + 1)", &mut none, 0)?;
        for e in &x {
            cond = format!("{cond} && {}", self.pred(e, "(l + 1)", "(l + 1)", &mut none, 0)?);
        }
        if none.is_some() {
            return None;
        }
        let run = format!("while {pa} {{ l += 1; }}");
        Some(if hit {
            format!("{{ let mut l = {s}; {run} let once = {cond}; if once {{ l += {step}; {run} }} (l, once) }}")
        } else {
            format!("{{ let mut l = {s}; {run} if {cond} {{ l += {step}; {run} }} l }}")
        })
    }

    /// `prev(E, k)` and `next(E, k)`: the call's mask and its distance, back (negative) or on.
    fn shift(f: &str, args: &[Arg]) -> Option<(Expr, i64)> {
        let e = match args.first() {
            Some(Arg::E(e)) => e.clone(),
            _ => return None,
        };
        let k = match args.get(1) {
            None => 1,
            Some(Arg::N(k)) => i64::try_from(*k).ok()?,
            _ => return None,
        };
        match f {
            "prev" => Some((e, -k)),
            "next" => Some((e, k)),
            _ => None,
        }
    }

    /// `e` at byte `at` (Rust, a `usize`), a test of the bytes there: none past the file or before it. At a token's
    /// first byte (`s`) the byte is in the file.
    pub(super) fn at(&self, e: &Expr, at: &str, depth: usize) -> Result<String, String> {
        // A fill's or an escape's test written once (`af_*`), where a read may stay as it is (`partial_windows`).
        if self.partial && matches!(e, Expr::Call(f, _) if f == "fill" || f == "escaped") {
            let got = self.amemo.borrow().get(e).cloned();
            let f = match got {
                Some(f) => f,
                None => {
                    let body = self.at_raw(e, "q", depth)?;
                    let f = format!("{}{}af_{}", self.tag, if self.code_fn { "c" } else { "" }, self.amemo.borrow().len());
                    self.amemo.borrow_mut().insert(e.clone(), f.clone());
                    let param = if self.code_fn { ", code: &dyn Fn(usize) -> bool" } else { "" };
                    let code = format!("/// Whether a fill or an escape holds at byte `q`, from the bytes.\n#[inline]\n#[allow(unused_variables, dead_code)]\nfn {f}(src: &[u8], n: usize, q: usize{param}) -> bool {{\n    {body}\n}}\n\n");
                    self.named.borrow_mut().push((f.clone(), code));
                    f
                }
            };
            return Ok(format!("{f}(src, n, {at}{})", if self.code_fn { ", code" } else { "" }));
        }
        self.at_raw(e, at, depth)
    }

    fn at_raw(&self, e: &Expr, at: &str, depth: usize) -> Result<String, String> {
        if depth > 64 {
            return Err("a mask's definition reads itself".into());
        }
        Ok(match e {
            Expr::Empty => "false".to_owned(),
            // A term of a run read forward (`forward_run`): whether it held at the byte before.
            Expr::Name(n) if let Some(k) = n.strip_prefix("\u{0}last") => format!("(last >> {k} & 1 == 1)"),
            // A term of the run read forward that ended at the token's last byte: whether the run ended in it.
            Expr::Name(n) if let Some(k) = n.strip_prefix("\u{0}end") => format!("(end_last >> {k} & 1 == 1)"),
            Expr::Name(n) => {
                // Asked of the records (`code_at`), where tier 2 asks it.
                if self.code_fn && n == "code" {
                    return Ok(format!("code({at})"));
                }
                if let Some(rs) = self.classes.get(n.as_str()) {
                    let test = |b: &str| if self.lean && rs.iter().map(|&(lo, hi)| hi + 1 - lo).sum::<u32>() > 1 { self.class_bit(n, b) } else { pattern_test(b, rs) };
                    if at == "s" {
                        test("src[s]")
                    } else {
                        let i = at.strip_prefix('(').and_then(|x| x.strip_suffix(')')).filter(|x| !x.contains('(')).unwrap_or(at);
                        format!("({i} < n && {})", test(&format!("src[{i}]")))
                    }
                } else if let Some(d) = self.defs.get(n) {
                    if self.partial {
                        // Each mask's test written once (`at_M`), as the lean lexer writes a read once.
                        let f = format!("{}{}_{}", self.tag, if self.code_fn { "ca" } else { "at" }, ident(n));
                        if !self.named.borrow().iter().any(|x| x.0 == f) {
                            self.named.borrow_mut().push((f.clone(), String::new()));
                            let byte = |c: &str| self.classes.get(c).and_then(|rs| match rs {
                                [(lo, hi)] if lo == hi => u8::try_from(*lo).ok(),
                                _ => None,
                            });
                            // A set of texts, or one and another test (`two & ~escaped(two)`): the set's tables first.
                            let tabled = texts_table(&f, d, &byte).or_else(|| match d {
                                Expr::And(a, b) => [(a, b), (b, a)].into_iter().find_map(|(t, o)| {
                                    let tt = texts_table(&f, t, &byte)?;
                                    let (st, test) = tt.strip_prefix('\u{0}')?.split_once('\u{0}')?;
                                    let other = self.at(o, "q", depth + 1).ok()?;
                                    Some(format!("\u{0}{st}\u{0}({test}) && {other}"))
                                }),
                                _ => None,
                            });
                            match tabled.map(Ok).unwrap_or_else(|| self.at(&d.clone(), "q", depth + 1)) {
                                Ok(body) => {
                                    let param = if self.code_fn { ", code: &dyn Fn(usize) -> bool" } else { "" };
                                    // A set of texts' tables before its test (`texts_table`).
                                    let (statics, body) = match body.strip_prefix('\u{0}').and_then(|b| b.split_once('\u{0}')) {
                                        Some((st, b)) => (st.to_owned(), b.to_owned()),
                                        None => (String::new(), body),
                                    };
                                    let code = format!("{statics}/// Whether mask `{n}` holds at byte `q`, from the bytes.\n#[inline]\n#[allow(unused_variables, dead_code)]\nfn {f}(src: &[u8], n: usize, q: usize{param}) -> bool {{\n    {body}\n}}\n\n");
                                    if let Some(x) = self.named.borrow_mut().iter_mut().find(|x| x.0 == f) {
                                        x.1 = code;
                                    }
                                }
                                Err(e) => {
                                    self.named.borrow_mut().retain(|x| x.0 != f);
                                    return Err(e);
                                }
                            }
                        }
                        format!("{f}(src, n, {at}{})", if self.code_fn { ", code" } else { "" })
                    } else {
                        self.at(d, at, depth + 1)?
                    }
                } else {
                    return Err(format!("`{n}` is no class or mask of the bytes"));
                }
            }
            Expr::Not(a) if at == "s" => format!("!{}", self.at(a, at, depth + 1)?),
            Expr::Not(a) => format!("({at} < n && !{})", self.at(a, at, depth + 1)?),
            Expr::And(a, b) => format!("({} && {})", self.at(a, at, depth + 1)?, self.at(b, at, depth + 1)?),
            Expr::Or(a, b) => format!("({} || {})", self.at(a, at, depth + 1)?, self.at(b, at, depth + 1)?),
            Expr::Xor(a, b) => format!("({} != {})", self.at(a, at, depth + 1)?, self.at(b, at, depth + 1)?),
            // An escaped byte: after an odd run of the escaping bytes (`partial_windows`).
            Expr::Call(f, args) if f == "escaped" && self.partial && let [Arg::E(b)] = args.as_slice() => {
                let t = self.at(b, "q", depth + 1)?;
                format!("(esc_back(({at}) as isize, |q: usize| {t}) == 1)")
            }
            // A fill's byte (`partial_windows`): its run read back from the byte to its nearest start or end.
            Expr::Call(f, args) if f == "fill" && self.partial && let [Arg::E(st), Arg::E(e)] = args.as_slice() => {
                let (st, e) = (self.at(st, "q", depth + 1)?, self.at(e, "q", depth + 1)?);
                format!("(fill_back(({at}) as isize + 1, |q: usize| {st}, |q: usize| {e}) == 1)")
            }
            // `text(T)`: the bytes from here are `T`'s.
            Expr::Call(f, args) if f == "text" && let [Arg::S(t)] = args.as_slice() && !t.is_empty() => {
                let i = if at == "s" { "s".to_owned() } else { at.to_owned() };
                let tests: Vec<String> = t.iter().enumerate().map(|(k, c)| if k == 0 { format!("src[{i}] == {c:#04x}") } else { format!("src[{i} + {k}] == {c:#04x}") }).collect();
                // At a token's first byte the byte is in the file.
                if i == "s" && t.len() == 1 { tests.join("") } else { format!("({i} + {} <= n && {})", t.len(), tests.join(" && ")) }
            }
            Expr::Call(f, args) => match Self::shift(f, args) {
                Some((a, k)) if k < 0 => format!("({at} >= {} && {})", -k, self.at(&a, &format!("({at} - {})", -k), depth + 1)?),
                Some((a, k)) => self.at(&a, &format!("({at} + {k})"), depth + 1)?,
                None => return Err(format!("`{f}` is no test of the bytes at one")),
            },
            Expr::Test { .. } => return Err("a machine's test is no test of the bytes".into()),
        })
    }

    /// `e`'s bits in the 64 bytes from byte `p + off`: none past the file or before it. `escaped`: the search's
    /// carried escapes (`esc`), from its first byte.
    fn win(&mut self, e: &Expr, off: i64, depth: usize) -> Result<String, String> {
        // A fill's or an escape's window written once (`wf_*`), where a read may stay as it is (`partial_windows`).
        if self.partial && matches!(e, Expr::Call(f, _) if f == "fill" || f == "escaped") {
            let f = match self.wmemo.get(e) {
                Some(f) => f.clone(),
                None => {
                    let body = self.win_raw(e, 0, depth)?;
                    let f = format!("wf_{}", self.wmemo.len());
                    self.wmemo.insert(e.clone(), f.clone());
                    let _ = write!(self.code, "/// A fill's or an escape's bits in the 64 bytes from `p`, from the bytes.\n#[inline]\n#[allow(unused_variables, dead_code)]\nunsafe fn {f}(src: &[u8], n: usize, p: isize) -> u64 {{\n    {body}\n}}\n\n");
                    f
                }
            };
            let p = match off {
                0 => "p".to_owned(),
                o if o < 0 => format!("(p - {})", -o),
                o => format!("(p + {o})"),
            };
            return Ok(format!("unsafe {{ {f}(src, n, {p}) }}"));
        }
        self.win_raw(e, off, depth)
    }

    fn win_raw(&mut self, e: &Expr, off: i64, depth: usize) -> Result<String, String> {
        if depth > 64 {
            return Err("a mask's definition reads itself".into());
        }
        let p = match off {
            0 => "p".to_owned(),
            o if o < 0 => format!("(p - {})", -o),
            o => format!("(p + {o})"),
        };
        Ok(match e {
            Expr::Empty => "0u64".to_owned(),
            // The window's bits a range test asks of (`live_fn`).
            Expr::Name(n) if n == LIVE => "live".to_owned(),
            Expr::Name(n) => {
                if self.classes.contains_key(n.as_str()) {
                    let f = format!("cls_{}", ident(n));
                    // A class without the space: the padding after the file (spaces, `PADDING`) holds none of it, so its
                    // bits need no end, as the lean lexer reads its padded bytes.
                    let held = if self.classes[n.as_str()].iter().any(|&(lo, hi)| lo <= 0x20 && 0x20 <= hi) { "held(b, q, n, p)" } else { "padded(b, q, p)" };
                    if !self.cls.contains(&f)
                        && let Some(code) = self.class_code.get(n.as_str())
                    {
                        self.cls.push(f.clone());
                        // Stage 1's bits of the class: its pairs of nibble tables, then one test (simdjson's `classify`).
                        let code = code.trim_end_matches(" & lv").to_owned();
                        let mut lines = String::new();
                        for k in 0..8 {
                            if code.contains(&format!("c{k},")) || code.contains(&format!("c{k})")) {
                                let _ = writeln!(lines, "    let c{k} = s::classify(v, &LO_{k}, &HI_{k});");
                            }
                        }
                        let _ = write!(
                            self.code,
                            "/// Class `{n}`'s bits in the 64 bytes from `p`: none past the file or before it.\n#[inline(always)]\nunsafe fn {f}(src: &[u8], n: usize, p: isize) -> u64 {{\n    let Some((v, q)) = (unsafe {{ window(src, n, p) }}) else {{ return 0 }};\n{lines}    let b = {code};\n    {held}\n}}\n\n"
                        );
                    }
                    if !self.cls.contains(&f) {
                        self.cls.push(f.clone());
                        let rs = self.classes[n.as_str()];
                        let bits: Vec<String> = rs
                            .iter()
                            .map(|&(lo, hi)| match (lo, hi) {
                                (lo, hi) if lo == hi => format!("s::eq_mask(v, {lo:#04x})"),
                                (lo, 255) => format!("s::to_bitmask(s::ge(v, {lo:#04x}))"),
                                (0, hi) => format!("!s::to_bitmask(s::ge(v, {:#04x}))", hi + 1),
                                (lo, hi) => format!("(s::to_bitmask(s::ge(v, {lo:#04x})) & !s::to_bitmask(s::ge(v, {:#04x})))", hi + 1),
                            })
                            .collect();
                        let _ = write!(
                            self.code,
                            "/// Class `{n}`'s bits in the 64 bytes from `p`: none past the file or before it.\n#[inline(always)]\nunsafe fn {f}(src: &[u8], n: usize, p: isize) -> u64 {{\n    let Some((v, q)) = (unsafe {{ window(src, n, p) }}) else {{ return 0 }};\n    let b = {};\n    {held}\n}}\n\n",
                            if bits.is_empty() { "0".to_owned() } else { bits.join(" | ") }
                        );
                    }
                    let here = format!("unsafe {{ {f}(src, n, p) }}");
                    // (A neighbour byte is in the padded source: before the file's end, or `PADDING` past it.)
                    // At a distance: the window's own bits shifted, its neighbours' bytes tested where they are, as the
                    // lean lexer derives its masks from its window's one classify (`shift_in`), not a window loaded again.
                    let rs = self.classes[n.as_str()];
                    let test = |bound: String, q: String| format!("u64::from({bound} && matches!(unsafe {{ *src.get_unchecked({q} as usize) }}, {}))", rs.iter().map(|&(lo, hi)| if lo == hi { format!("{lo:#04x}") } else { format!("{lo:#04x}..={hi:#04x}") }).collect::<Vec<_>>().join(" | "));
                    match off {
                        0 => here,
                        k if k > 0 && k < 64 && !rs.is_empty() => {
                            let next: Vec<String> = (0..k).map(|j| format!("{} << {}", test(format!("p + {} < n as isize", 64 + j), format!("(p + {})", 64 + j)), 64 - k + j)).collect();
                            format!("({here} >> {k} | {})", next.join(" | "))
                        }
                        k if k < 0 && k > -64 && !rs.is_empty() => {
                            let k = -k;
                            let before: Vec<String> = (0..k).map(|j| format!("{}{}", test(format!("p >= {}", k - j), format!("(p - {})", k - j)), if j == 0 { String::new() } else { format!(" << {j}") })).collect();
                            format!("({here} << {k} | {})", before.join(" | "))
                        }
                        _ => format!("unsafe {{ {f}(src, n, {p}) }}"),
                    }
                } else if let Some(d) = self.defs.get(n).cloned() {
                    if self.partial {
                        // Each mask's window written once (`d_M`), at any byte.
                        let f = format!("d_{}", ident(n));
                        if !self.written.contains(&f) {
                            self.written.push(f.clone());
                            let body = self.win(&d, 0, depth + 1).inspect_err(|_| self.written.retain(|w| *w != f))?;
                            let _ = write!(self.code, "/// Mask `{n}`'s bits in the 64 bytes from `p`, from the bytes: none past the file or before it.\n#[inline]\n#[allow(unused_variables, dead_code)]\nunsafe fn {f}(src: &[u8], n: usize, p: isize) -> u64 {{\n    {body}\n}}\n\n");
                        }
                        format!("unsafe {{ {f}(src, n, {p}) }}")
                    } else {
                        self.win(&d, off, depth + 1)?
                    }
                } else {
                    return Err(format!("`{n}` is no class or mask of the bytes"));
                }
            }
            Expr::Not(a) => format!("(!{} & valid(n, {p}))", self.win(a, off, depth + 1)?),
            Expr::And(a, b) => {
                let (a, b) = (self.win(a, off, depth + 1)?, self.win(b, off, depth + 1)?);
                // A test of several windows after one that rarely holds (a `\u` escape's digits after its `\u`): read
                // only where the first holds, as the lean lexer unescapes a string only when it saw a `\u`. A side
                // that carries escapes is always read, its carry the next window's.
                // A mask made of the bytes (`d_`) or a fill (`wf_`) is many classes' tests: read only where the first holds,
                // as the lean lexer decodes a character only at a byte past ASCII.
                let cost = b.matches("cls_").count() + 3 * (b.matches("unsafe { d_").count() + b.matches("unsafe { wf_").count());
                if cost >= 3 && !b.contains("esc)") {
                    format!("{{ let a = {a}; if a == 0 {{ 0 }} else {{ a & {b} }} }}")
                } else {
                    format!("({a} & {b})")
                }
            }
            Expr::Or(a, b) => format!("({} | {})", self.win(a, off, depth + 1)?, self.win(b, off, depth + 1)?),
            Expr::Xor(a, b) => format!("({} ^ {})", self.win(a, off, depth + 1)?, self.win(b, off, depth + 1)?),
            // A read from anywhere (`partial_windows`): the window's carry read back from the bytes, the run of the
            // escaping bytes that ends before it (odd: the window's first byte is escaped), as simdjson's carry is.
            Expr::Call(f, args) if f == "escaped" && self.partial => {
                let Some(Arg::E(b)) = args.first() else { return Err("`escaped` of no mask".into()) };
                let at = self.at(b, "q", depth + 1)?;
                let b = self.win(b, off, depth + 1)?;
                format!("{{ let mut c = esc_back({p}, |q: usize| {at}); escaped_step({b}, &mut c) }}")
            }
            // `escaped` in a region token whose run starts after its opener's `o` bytes (`escaped_from`, a field's write
            // of the token's body): the run read back no further than that byte, the token's opener `s` the window
            // function's, as the lean lexer reads a string's text from after its quote.
            Expr::Call(f, args) if f == "escaped_from" && self.partial && let [Arg::E(b), Arg::N(o)] = args.as_slice() => {
                let at = self.at(b, "q", depth + 1)?;
                let b = self.win(b, off, depth + 1)?;
                format!("{{ let lo = s as isize + {o}; let mut c = esc_from({p}, lo, |q: usize| {at}); escaped_step({b} & from_bit({p}, lo), &mut c) }}")
            }
            // A fill from anywhere (`partial_windows`): stage 1's add, its carry the fill at the byte before the
            // window, read back over the run to its nearest start (in) or end (out), as the lean lexer reads a run.
            Expr::Call(f, args) if f == "fill" && self.partial && let [Arg::E(st), Arg::E(e)] = args.as_slice() => {
                let (sa, ea) = (self.at(st, "q", depth + 1)?, self.at(e, "q", depth + 1)?);
                let (sw, ew) = (self.win(st, off, depth + 1)?, self.win(e, off, depth + 1)?);
                format!("{{ let (fs, fe) = ({sw}, {ew}); let c = fill_back({p}, |q: usize| {sa}, |q: usize| {ea}); let b = !fe | fs; let into = fs.wrapping_add(b).wrapping_add(c) ^ fs ^ b; (fs | (!fe & into)) & valid(n, {p}) }}")
            }
            // `text(T)`: each of `T`'s bytes in its window, `k` bytes on.
            Expr::Call(f, args) if f == "text" && self.partial && let [Arg::S(t)] = args.as_slice() && !t.is_empty() => {
                let terms: Vec<String> = t.iter().enumerate().map(|(k, c)| format!("unsafe {{ wbyte(src, n, {}, {c:#04x}) }}", if off + k as i64 == 0 { "p".to_owned() } else { format!("(p + {})", off + k as i64) })).collect();
                format!("({})", terms.join(" & "))
            }
            Expr::Call(f, args) if f == "escaped" && off == 0 => {
                let Some(Arg::E(b)) = args.first() else { return Err("`escaped` of no mask".into()) };
                let b = self.win(b, 0, depth + 1)?;
                format!("escaped_step({b}, esc)")
            }
            Expr::Call(f, args) => match Self::shift(f, args) {
                Some((a, k)) => self.win(&a, off + k, depth + 1)?,
                None => return Err(format!("`{f}` is no mask of the bytes 64 at a time")),
            },
            Expr::Test { .. } => return Err("a machine's test is no mask of the bytes".into()),
        })
    }

    /// `e` with the masks it names in place, its classes kept by name. None where a definition reads itself.
    fn resolved(&self, e: &Expr, depth: usize) -> Option<Expr> {
        if depth > 64 {
            return None;
        }
        let r = |x: &Expr| self.resolved(x, depth + 1).map(Box::new);
        Some(match e {
            Expr::Name(n) if !self.classes.contains_key(n.as_str()) => self.resolved(self.defs.get(n)?, depth + 1)?,
            Expr::Not(a) => Expr::Not(r(a)?),
            Expr::And(a, b) => Expr::And(r(a)?, r(b)?),
            Expr::Or(a, b) => Expr::Or(r(a)?, r(b)?),
            Expr::Xor(a, b) => Expr::Xor(r(a)?, r(b)?),
            Expr::Call(f, args) => Expr::Call(f.clone(), args.iter().map(|a| match a { Arg::E(x) => self.resolved(x, depth + 1).map(Arg::E), o => Some(o.clone()) }).collect::<Option<_>>()?),
            other => other.clone(),
        })
    }

    /// The one byte class `e` is.
    fn byte(&self, e: &Expr) -> Option<u8> {
        let Expr::Name(n) = e else { return None };
        match self.classes.get(n.as_str())? {
            [(lo, hi)] if lo == hi => u8::try_from(*lo).ok(),
            _ => None,
        }
    }

    /// The bytes of `e` where it is a conjunction of one byte each at a distance (`tick & next(tick)`, `slash &
    /// prev(star)`): each byte and its distance.
    fn bytes_at(&self, e: &Expr, out: &mut Vec<(u8, i64)>) -> Option<()> {
        match e {
            Expr::And(a, b) => {
                self.bytes_at(a, out)?;
                self.bytes_at(b, out)
            }
            Expr::Call(f, args) => {
                let (a, k) = Self::shift(f, args)?;
                out.push((self.byte(&a)?, k));
                Some(())
            }
            e => {
                out.push((self.byte(e)?, 0));
                Some(())
            }
        }
    }

    /// Lean's micro-lexer for the first byte from `from` that mask `e` holds, where `e` is one: a byte (`micro::find`),
    /// two bytes (`find_pair`), more (`find_fence`), a quote no backslash escapes (`string_end`, as the lean lexer reads
    /// a string's body). A mask's byte after the first of its bytes (`prev`) is found by the first, from as far before.
    pub(super) fn micro_first(&mut self, e: &Expr, from: &str) -> Option<String> {
        let e = self.resolved(e, 0)?;
        if let Expr::And(a, b) = &e
            && self.byte(a) == Some(b'"')
            && let Expr::Not(x) = b.as_ref()
            && let Expr::Call(f, args) = x.as_ref()
            && f == "escaped"
            && matches!(args.as_slice(), [Arg::E(bs)] if self.byte(bs) == Some(b'\\'))
        {
            return Some(format!("micro::string_end(&src[..n], {from}).0"));
        }
        let mut bs = Vec::new();
        self.bytes_at(&e, &mut bs)?;
        bs.sort_by_key(|b| b.1);
        let lo = bs[0].1;
        if lo > 0 || bs.iter().enumerate().any(|(i, b)| b.1 != lo + i as i64) {
            return None;
        }
        let back = usize::try_from(-lo).ok()?;
        let start = match (from.strip_suffix(&format!(" + {back}")), back) {
            (_, 0) => from.to_owned(),
            (Some(f), _) => f.to_owned(),
            (None, k) => format!("{from} - {k}"),
        };
        let found = match bs.as_slice() {
            [(x, _)] => format!("micro::find(&src[..n], {start}, {})", lit(*x)),
            [(x, _), (y, _)] => format!("micro::find_pair(&src[..n], {start}, {}, {})", lit(*x), lit(*y)),
            _ => {
                self.fence = true;
                format!("find_fence(&src[..n], {start}, &[{}])", bs.iter().map(|b| lit(b.0)).collect::<Vec<_>>().join(", "))
            }
        };
        Some(if back == 0 { found } else { format!("{found}.map(|a| a + {back})") })
    }

    /// `e` at byte `at`, a test of the bytes there, as `at`; `escaped(B)` at byte `var` the loop's `esc` (the byte
    /// before ends a run of an odd count of `B`'s), `B` kept in `esc_of`. None where an escape is read elsewhere.
    fn pred(&self, e: &Expr, at: &str, var: &str, esc_of: &mut Option<Expr>, depth: usize) -> Option<String> {
        if depth > 64 {
            return None;
        }
        let mut go = |e: &Expr, at: &str| self.pred(e, at, var, esc_of, depth + 1);
        Some(match e {
            Expr::Name(n) if !self.classes.contains_key(n.as_str()) => {
                let d = self.defs.get(n)?;
                let mut none = None;
                // A long test that reads no escape: a function of its own, by the mask's name (lean's `unescape`).
                match self.pred(d, "i", "i", &mut none, depth + 1) {
                    Some(t) if t.len() > 160 && none.is_none() => {
                        let f = ident(n);
                        if !self.named.borrow().iter().any(|x| x.0 == f) {
                            let code = format!("/// Mask `{n}` at byte `i`, a test of the bytes there.\n#[inline]\nfn {f}(src: &[u8], n: usize, i: usize) -> bool {{\n    {t}\n}}\n\n");
                            self.named.borrow_mut().push((f.clone(), code));
                        }
                        format!("{f}(src, n, {at})")
                    }
                    _ => go(d, at)?,
                }
            }
            Expr::Not(a) => format!("({at} < n && !{})", go(a, at)?),
            Expr::And(a, b) => format!("({} && {})", go(a, at)?, go(b, at)?),
            Expr::Or(a, b) => format!("({} || {})", go(a, at)?, go(b, at)?),
            Expr::Xor(a, b) => format!("({} != {})", go(a, at)?, go(b, at)?),
            Expr::Call(f, args) if f == "escaped" => {
                let [Arg::E(b)] = args.as_slice() else { return None };
                if at != var || esc_of.as_ref().is_some_and(|x| format!("{x:?}") != format!("{b:?}")) {
                    return None;
                }
                *esc_of = Some(b.clone());
                "esc".to_owned()
            }
            Expr::Call(f, args) => {
                let (a, k) = Self::shift(f, args)?;
                if k < 0 { format!("({at} >= {} && {})", -k, go(&a, &format!("{at} - {}", -k))?) } else { go(&a, &format!("{at} + {k}"))? }
            }
            e => self.at(e, at, depth + 1).ok()?,
        })
    }

    /// Mask `m` tested at byte `i` of a loop of the bytes, the escape it reads carried along (`esc` for the next byte:
    /// this one is `B` and not escaped itself). None where it reads an escape elsewhere.
    fn tested_at(&self, m: &str, i: &str) -> Option<(String, Option<String>)> {
        let e = self.expr(m).ok()?;
        let mut esc_of = None;
        let p = self.pred(&e, i, i, &mut esc_of, 0)?;
        let carry = match esc_of {
            Some(b) => Some(format!("esc = !esc && {}", self.at(&b, i, 0).ok()?)),
            None => None,
        };
        Some((p, carry))
    }

    /// One read of the loop in the lean lexer's form: a micro-lexer, else the bytes in a loop, each tested where it is
    /// (a run by `while`, a search by `find`, a count by `filter`). None where a mask has no such form.
    fn lean_read(&mut self, name: &str, args: &[String], m: &str) -> Result<Option<String>, String> {
        let a = |k: usize| args.get(k).map(|x| x.trim().to_owned()).unwrap_or_default();
        if name == "first_bit"
            && let Some(r) = self.micro_first(&self.expr(m)?, &a(1))
        {
            return Ok(Some(r));
        }
        let Some((p, carry)) = self.tested_at(m, "i") else { return Ok(None) };
        // A test whose escape is carried: the closure updates it after its byte's test.
        let test = |p: &str| match &carry {
            Some(c) => format!("{{ let hit = {p}; {c}; hit }}"),
            None => p.to_owned(),
        };
        let esc = if carry.is_some() { "let mut esc = false; " } else { "" };
        let next = self.tested_at(m, "(l + 1)").map(|t| t.0).unwrap_or_default();
        Ok(Some(match (name, args.len()) {
            ("first_bit", 2) => format!("{{ {esc}({}..n).find(|&i| {}) }}", a(1), test(&p)),
            ("first_in", 3) => format!("{{ {esc}({}..={}).find(|&i| {}) }}", a(1), a(2), test(&p)),
            ("any_in", 3) => format!("{{ {esc}({}..={}).any(|i| {}) }}", a(1), a(2), test(&p)),
            ("count_in", 3) => format!("{{ {esc}({}..={}).filter(|&i| {}).count() as u32 }}", a(1), a(2), test(&p)),
            ("first_clear", 3) if carry.is_none() => format!("({}..n.min({})).find(|&i| !{p})", a(1), a(2)),
            ("run_last", 2) if carry.is_none() => format!("{{ let mut l = {}; while {next} {{ l += 1; }} l }}", a(1)),
            ("run_once_last" | "run_once_hit", 3) if carry.is_none() && args[1].trim().strip_prefix('&').is_some_and(|d| self.two_runs(m, d, &a(2), name == "run_once_hit").is_some()) => {
                let d = args[1].trim().strip_prefix('&').unwrap_or_default();
                self.two_runs(m, d, &a(2), name == "run_once_hit").unwrap_or_default()
            }
            ("run_once_last", 3) if carry.is_none() => {
                let Some(d) = args[1].trim().strip_prefix('&') else { return Ok(None) };
                let Some((pd, None)) = self.tested_at(d, "(l + 1)") else { return Ok(None) };
                format!("{{ let (mut l, mut once) = ({}, false); while {next} {{ if {pd} {{ if once {{ break; }} once = true; }} l += 1; }} l }}", a(2))
            }
            // The same, and whether it read its one `D` (`once_hits`).
            ("run_once_hit", 3) if carry.is_none() => {
                let Some(d) = args[1].trim().strip_prefix('&') else { return Ok(None) };
                let Some((pd, None)) = self.tested_at(d, "(l + 1)") else { return Ok(None) };
                format!("{{ let (mut l, mut once) = ({}, false); while {next} {{ if {pd} {{ if once {{ break; }} once = true; }} l += 1; }} (l, once) }}", a(2))
            }
            _ => return Ok(None),
        }))
    }

    /// Mask `m` as a test of byte `i` (`|i: usize| …`), for a run read past its block (the lean lexer's `word_end`).
    pub(super) fn byte_test(&self, m: &str) -> Result<String, String> {
        match self.tested_at(m, "i") {
            Some((p, None)) => Ok(format!("|i: usize| {p}")),
            _ => Err(format!("`{m}` is no test of a byte")),
        }
    }

    /// The local's expression.
    fn expr(&self, local: &str) -> Result<Expr, String> {
        self.locals.get(local).cloned().ok_or_else(|| format!("`{local}` is no mask of the bytes"))
    }

    /// The window function of the local `m` (`w_m`), written once; whether it reads escapes (its windows must then
    /// start where the carry is none: a token's first byte, or the one after).
    pub(super) fn window_fn(&mut self, m: &str) -> Result<(String, bool), String> {
        let f = format!("w_{m}");
        let e = self.expr(m)?;
        let bits = self.win(&e, 0, 0)?;
        let esc = bits.contains("esc)");
        // A read of the token's opener (`escaped_from`): the function takes it.
        if bits.contains("esc_from(") {
            if !self.written.contains(&f) {
                self.written.push(f.clone());
                let _ = write!(
                    self.code,
                    "/// `{m}`'s bits in the 64 bytes from `p` of the region token from `s`, from the bytes: none past the file or before it.\n#[inline(always)]\n#[allow(unused_variables)]\nunsafe fn {f}(src: &[u8], n: usize, p: isize, esc: &mut u64, s: usize) -> u64 {{\n    {bits}\n}}\n\n"
                );
            }
            return Ok((format!("|p: isize, esc: &mut u64| unsafe {{ {f}(src, n, p, esc, s) }}"), esc));
        }
        if !self.written.contains(&f) {
            self.written.push(f.clone());
            let _ = write!(
                self.code,
                "/// `{m}`'s bits in the 64 bytes from `p`, from the bytes: none past the file or before it.\n#[inline(always)]\n#[allow(unused_variables)]\nunsafe fn {f}(src: &[u8], n: usize, p: isize, esc: &mut u64) -> u64 {{\n    {bits}\n}}\n\n"
            );
            // Its twin given the window's escaped bytes, where it reads them of one mask.
            if bits.matches("escaped_step(").count() == 1
                && let Some(at) = bits.find("escaped_step(")
                && let Some(len) = bits[at..].find(", esc)")
            {
                let of = bits[at + "escaped_step(".len()..at + len].to_owned();
                let twin = bits.replacen(&format!("escaped_step({of}, esc)"), "escaped", 1);
                let _ = write!(
                    self.code,
                    "/// `{f}` given its window's escaped bytes (`escaped`).\n#[inline(always)]\n#[allow(unused_variables)]\nunsafe fn e_{m}(src: &[u8], n: usize, p: isize, escaped: u64) -> u64 {{\n    {twin}\n}}\n\n"
                );
                // The class's bytes, where the escapes are of one class.
                if let Some(c) = of.strip_prefix("unsafe { cls_").and_then(|c| c.strip_suffix("(src, n, p) }"))
                    && let Some((name, rs)) = self.classes.iter().find(|(name, _)| ident(name) == c)
                {
                    let _ = name;
                    let held: [bool; 256] = std::array::from_fn(|b| rs.iter().any(|&(lo, hi)| lo as usize <= b && b <= hi as usize));
                    self.escape_bytes.insert(of.clone(), held);
                }
                self.escapes.insert(f.clone(), of);
            }
        }
        Ok((format!("|p: isize, esc: &mut u64| unsafe {{ {f}(src, n, p, esc) }}"), esc))
    }

    /// The last byte of the run of local `m` from byte `s0`, read forward as the lean lexer reads a number (its `digits`,
    /// its `exponent`): `m` is a union of terms, each a fill (`fill(S, E)`) or a test of the bytes, where a term's
    /// `prev` of another term is whether that one held at the byte before (`last`, a bit a term). At each byte the terms
    /// that hold there: a fill where it starts or held before and `E` holds not. Where one fill alone holds, its stop a
    /// class's complement (`~decd`), and no other term may start in that class's run (the byte not of its class, or a
    /// `prev` of a term that is not this one), the run of the class at once, as lean's `class_end`. `None` where `m` is
    /// not of that form.
    fn forward_run(&mut self, m: &str, s0: &str) -> Option<String> {
        let e = self.expr(m).ok()?;
        let mut terms: Vec<(Option<String>, Expr)> = Vec::new();
        fn ors(e: &Expr, out: &mut Vec<Expr>) {
            match e {
                Expr::Or(a, b) => {
                    ors(a, out);
                    ors(b, out);
                }
                e => out.push(e.clone()),
            }
        }
        let mut flat = Vec::new();
        ors(&e, &mut flat);
        if flat.len() < 2 || flat.len() > 32 {
            return None;
        }
        for t in flat {
            match &t {
                Expr::Name(n) if let Some(d) = self.locals.get(n).or_else(|| self.defs.get(n)) => terms.push((Some(n.clone()), d.clone())),
                _ => terms.push((None, t)),
            }
        }
        // A term's expression, its names resolved where they are no class.
        let full: Vec<Expr> = terms.iter().map(|t| t.1.clone()).collect();
        // `prev(T)` of terms `T` (one, or a union of them) as their bits at the byte before; `None` where a term is read
        // but through `prev`.
        fn sub(me: &Bytes, e: &Expr, full: &[Expr], depth: usize, top: bool) -> Option<Expr> {
            if depth > 48 {
                return None;
            }
            fn bits(me: &Bytes, x: &Expr, full: &[Expr], depth: usize) -> Option<Expr> {
                if depth > 48 {
                    return None;
                }
                if let Some(k) = full.iter().position(|t| t == x) {
                    return Some(Expr::Name(format!("\u{0}last{k}")));
                }
                match x {
                    Expr::Name(n) if !me.classes.contains_key(n.as_str()) => bits(me, me.locals.get(n).or_else(|| me.defs.get(n))?, full, depth + 1),
                    Expr::Or(a, b) => Some(Expr::Or(Box::new(bits(me, a, full, depth + 1)?), Box::new(bits(me, b, full, depth + 1)?))),
                    _ => None,
                }
            }
            if !top && full.iter().any(|t| t == e) {
                return None;
            }
            Some(match e {
                Expr::Call(f, args) if f == "prev" && matches!(args.as_slice(), [Arg::E(_)] | [Arg::E(_), Arg::N(1)]) => {
                    let Arg::E(x) = &args[0] else { return None };
                    match bits(me, x, full, 0) {
                        Some(b) => b,
                        None => Expr::Call(f.clone(), vec![Arg::E(sub(me, x, full, depth + 1, false)?)]),
                    }
                }
                Expr::Name(n) if !me.classes.contains_key(n.as_str()) && let Some(d) = me.locals.get(n).or_else(|| me.defs.get(n)) => {
                    if full.iter().any(|t| t == d) {
                        return None;
                    }
                    if reads_terms(me, d, full, 0) { sub(me, d, full, depth + 1, false)? } else { e.clone() }
                }
                Expr::Name(_) => e.clone(),
                Expr::Not(a) => Expr::Not(Box::new(sub(me, a, full, depth + 1, false)?)),
                Expr::And(a, b) => Expr::And(Box::new(sub(me, a, full, depth + 1, false)?), Box::new(sub(me, b, full, depth + 1, false)?)),
                Expr::Or(a, b) => Expr::Or(Box::new(sub(me, a, full, depth + 1, false)?), Box::new(sub(me, b, full, depth + 1, false)?)),
                Expr::Xor(a, b) => Expr::Xor(Box::new(sub(me, a, full, depth + 1, false)?), Box::new(sub(me, b, full, depth + 1, false)?)),
                Expr::Call(f, args) => Expr::Call(
                    f.clone(),
                    args.iter()
                        .map(|a| match a {
                            Arg::E(x) => sub(me, x, full, depth + 1, false).map(Arg::E),
                            a => Some(a.clone()),
                        })
                        .collect::<Option<Vec<_>>>()?,
                ),
                Expr::Empty | Expr::Test { .. } => e.clone(),
            })
        }
        // Each term: its start (a fill's `S`, else the term), and a fill's stop.
        let mut parts: Vec<(Expr, Option<Expr>)> = Vec::new();
        for (_, t) in &terms {
            match t {
                Expr::Call(f, args) if f == "fill" && let [Arg::E(st), Arg::E(stop)] = args.as_slice() => {
                    parts.push((sub(self, st, &full, 0, false)?, Some(sub(self, stop, &full, 0, false)?)));
                }
                Expr::Call(f, _) if f == "fill" || f == "escaped" => return None,
                t => parts.push((sub(self, t, &full, 0, true)?, None)),
            }
        }
        // `e` as one test of the byte at an offset from the byte asked (`prev`, `next` of a set of bytes).
        fn single(me: &Bytes, e: &Expr, depth: usize) -> Option<(i64, [bool; 256])> {
            if depth > 48 {
                return None;
            }
            match e {
                Expr::Name(n) if n.starts_with('\u{0}') => None,
                Expr::Name(n) => match me.classes.get(n.as_str()) {
                    Some(rs) => Some((0, std::array::from_fn(|c| rs.iter().any(|&(lo, hi)| (lo..=hi).contains(&(c as u32)))))),
                    None => single(me, me.locals.get(n).or_else(|| me.defs.get(n))?, depth + 1),
                },
                Expr::Call(f, args) if matches!(f.as_str(), "prev" | "next") => {
                    let (x, k) = Bytes::shift(f, args)?;
                    let (o, set) = single(me, &x, depth + 1)?;
                    Some((o + k, set))
                }
                Expr::Not(a) => single(me, a, depth + 1).map(|(o, set)| (o, set.map(|x| !x))),
                Expr::And(a, b) | Expr::Or(a, b) => {
                    let ((oa, sa), (ob, sb)) = (single(me, a, depth + 1)?, single(me, b, depth + 1)?);
                    if oa != ob {
                        return None;
                    }
                    let and = matches!(e, Expr::And(..));
                    Some((oa, std::array::from_fn(|c| if and { sa[c] && sb[c] } else { sa[c] || sb[c] })))
                }
                _ => None,
            }
        }
        // Whether `e`, read at offset `off` from a byte, may hold where that byte and the one before are of `set` and
        // `last` (the terms at the byte before) holds term `k` alone.
        fn may(me: &Bytes, e: &Expr, set: &[bool; 256], k: usize, off: i64, depth: usize) -> bool {
            if depth > 48 {
                return true;
            }
            if let Some((o, held)) = single(me, e, 0) {
                return !matches!(off + o, 0 | -1) || (0..256).any(|c| set[c] && held[c]);
            }
            match e {
                Expr::Name(n) if let Some(j) = n.strip_prefix("\u{0}last") => off != 0 || j.parse() == Ok(k),
                Expr::Name(n) if !me.classes.contains_key(n.as_str()) => {
                    me.locals.get(n).or_else(|| me.defs.get(n)).is_none_or(|d| may(me, d, set, k, off, depth + 1))
                }
                Expr::Call(f, args) if matches!(f.as_str(), "prev" | "next") => match Bytes::shift(f, args) {
                    Some((x, d)) => may(me, &x, set, k, off + d, depth + 1),
                    None => true,
                },
                Expr::And(a, b) => may(me, a, set, k, off, depth + 1) && may(me, b, set, k, off, depth + 1),
                Expr::Or(a, b) => may(me, a, set, k, off, depth + 1) || may(me, b, set, k, off, depth + 1),
                _ => true,
            }
        }
        let mut tests = String::new();
        let mut jumps = String::new();
        for (k, (st, stop)) in parts.iter().enumerate() {
            let start = self.at(st, "q", 0).ok()?;
            let test = match stop {
                Some(stop) => format!("{start} || (last >> {k} & 1 == 1 && !{})", self.at(stop, "q", 0).ok()?),
                None => start,
            };
            let why = match stop {
                Some(Expr::Not(c)) if let Expr::Name(cn) = &**c => format!(": a run of `{cn}`"),
                Some(_) => ": a fill".to_owned(),
                None => ": a byte".to_owned(),
            };
            let _ = write!(tests, "            // Term {k}{why}.\n            if {test} {{\n                now |= 1 << {k};\n            }}\n");
            // A fill whose stop is a class's complement, where no other term starts in the class's run.
            if let Some(Expr::Not(c)) = stop
                && let Expr::Name(cn) = &**c
                && let Some(rs) = self.classes.get(cn.as_str())
            {
                let set: [bool; 256] = std::array::from_fn(|b| rs.iter().any(|&(lo, hi)| (lo..=hi).contains(&(b as u32))));
                let alone = parts.iter().enumerate().all(|(j, (sj, _))| j == k || !may(self, sj, &set, k, 0, 0));
                if alone {
                    let cls = self.win(c, 0, 0).ok()?;
                    let here = self.at(c, "q", 0).ok()?;
                    let _ = write!(jumps, "            // `{cn}`'s run: term {k} alone holds over it (the lean lexer's `class_end`).\n            if now == 1 << {k} && {here} {{\n                last = now;\n                q = wfirst_clear(n, q + 1, n, |p: isize, esc: &mut u64| {cls}).unwrap_or(n);\n                continue;\n            }}\n");
                }
            }
        }
        self.fwd = Some((self.norm(&e, 0), full.iter().map(|t| self.norm(t, 0)).collect()));
        self.fwd_used = true;
        Some(format!("{{\n        // The run read forward from its first byte (in it, as `run_last` takes it), a term a bit (`last`: those at the byte before).\n        let mut q = {s0};\n        let mut last = 0u32;\n        while q < n {{\n            let mut now = 0u32;\n{tests}            if now == 0 && q > {s0} {{\n                break;\n            }}\n{jumps}            last = now;\n            q += 1;\n        }}\n        end_last = last;\n        q.saturating_sub(1)\n    }}"))
    }

    /// The window function of the local `m` for a test of bytes `..=b` from the window's first (`l_m`), written once:
    /// `live`, the bits it asks of, ANDed into the first test of each side (`with_live`), so a side whose first test holds
    /// at none of the token's bytes reads no further, as the lean lexer tests only the bytes it read.
    fn live_fn(&mut self, m: &str, b: &str) -> Result<String, String> {
        let f = format!("l_{m}");
        if !self.written.contains(&f) {
            let e = with_live(&self.expr(m)?);
            let bits = self.win(&e, 0, 0)?;
            self.written.push(f.clone());
            // A read of the region token's opener (`escaped_from`): the function takes the token's first byte `s`.
            let opener = if bits.contains("esc_from(") { ", s: usize" } else { "" };
            let _ = write!(
                self.code,
                "/// `{m}`'s bits in the 64 bytes from `p` that `live` holds, from the bytes.\n#[inline(always)]\n#[allow(unused_variables)]\nunsafe fn {f}(src: &[u8], n: usize, p: isize, esc: &mut u64, live: u64{opener}) -> u64 {{\n    {bits}\n}}\n\n"
            );
        }
        let opener = if self.code.contains(&format!("unsafe fn {f}(src: &[u8], n: usize, p: isize, esc: &mut u64, live: u64, s: usize)")) { ", s" } else { "" };
        Ok(format!("|p: isize, esc: &mut u64| unsafe {{ {f}(src, n, p, esc, upto(p, {}){opener}) }}", b.trim()))
    }

    /// One read (`name(args)`) of the loop as the bytes' form.
    fn read(&mut self, name: &str, args: &[String], line: &str) -> Result<Option<String>, String> {
        let mask = |a: &str| a.trim().strip_prefix('&').map(|m| m.strip_suffix("[..made]").unwrap_or(m).to_owned());
        // A forward mask's bits made the mask's (`put(&mut V, &M, a, b)`): of its windows.
        if self.partial && name == "put" && args.len() == 4 {
            let Some(m) = mask(&args[1]).filter(|m| self.locals.contains_key(m)) else { return Ok(None) };
            let (f, _) = self.window_fn(&m)?;
            return Ok(Some(format!("wput({}, {}, {}, {f})", args[0].trim(), args[2].trim(), args[3].trim())));
        }
        let Some(m) = mask(&args[0]) else { return Ok(None) };
        // A token's run, read another way than forward: the tests at its end take nothing of the last one's.
        if name.contains("run_") && !line.contains("assert") {
            self.fwd = None;
        }
        // The test build's checks read the walk's masks, which stay the walk's.
        if !self.locals.contains_key(&m) {
            return if line.contains("assert") || self.partial { Ok(None) } else { Err(format!("`{m}` is no mask of the bytes")) };
        }
        let from_start = |a: &str| matches!(a.trim(), "s" | "s + 1");
        let windowed = |me: &mut Self, from: &str| -> Result<String, String> {
            let (f, esc) = me.window_fn(&m)?;
            if esc && !from_start(from) {
                return Err(format!("`{m}` reads escapes from no token's start"));
            }
            Ok(f)
        };
        // A token's one-byte test at its start (`first_clear(&M, s, s + 1)`, `bit(&M, s)`): a bit of the block's mask,
        // as lean's masks of its one `Window`, where the mask reads no escapes.
        let one = name == "first_clear" && args.len() == 3 && args[1].trim() == "s" && args[2].trim() == "s + 1";
        if (one || (name == "bit" && args.len() == 2 && args[1].trim() == "s")) && !self.in_region && !self.partial && !self.window_fn(&m)?.1 {
            let k = self.block_word(&m);
            let holds = format!("({k} >> (s - base) & 1 == 1)");
            return Ok(Some(if one { format!("(!{holds}).then_some(s)") } else { holds }));
        }
        // A read of one byte (`first_clear(&M, a, a + 1)`): a test of the bytes there.
        if name == "first_clear" && args.len() == 3 && args[2].trim() == format!("{} + 1", args[1].trim()) {
            let e = self.expr(&m)?;
            let a = args[1].trim();
            return Ok(Some(format!("(!{}).then_some({a})", self.at(&e, a, 0)?)));
        }
        // A mask with no escapes: its block's word first, made once a block (`k_M`), then windows past the block. In the
        // token loop (`partial_line`), a mask of a few classes alone, as the lean lexer's block masks (a word's, a blank's,
        // a line end's): one of fills or many classes is read where a token asks it.
        if name != "bit" && !self.in_region && !self.window_fn(&m)?.1 && (!self.partial || self.expr(&m).is_ok_and(|e| self.few_classes(&e, 0).is_some_and(|k| k <= 4))) {
            let k = self.block_word(&m);
            let (f, _) = self.window_fn(&m)?;
            return Ok(Some(match (name, args.len()) {
                ("first_bit", 2) => format!("bfirst_bit(n, base, {k}, {}, {f})", args[1].trim()),
                ("count_in", 3) => format!("bcount_in(n, base, {k}, {}, {}, {f})", args[1].trim(), args[2].trim()),
                ("first_in", 3) => format!("bfirst_in(n, base, {k}, {}, {}, {f})", args[1].trim(), args[2].trim()),
                ("any_in", 3) => format!("bfirst_in(n, base, {k}, {}, {}, {f}).is_some()", args[1].trim(), args[2].trim()),
                ("first_clear", 3) => format!("bfirst_clear(n, base, {k}, {}, {}, {f})", args[1].trim(), args[2].trim()),
                ("run_last", 2) => format!("brun_last(n, base, {k}, {}, {f})", args[1].trim()),
                ("run_once_last", 3) => {
                    let Some(d) = mask(&args[1]) else { return Ok(None) };
                    let (fd, esc) = self.window_fn(&d)?;
                    if esc {
                        return Err(format!("`{d}` reads escapes"));
                    }
                    let kd = self.block_word(&d);
                    format!("brun_once_last(n, base, {k}, {kd}, {}, {f}, {fd})", args[2].trim())
                }
                _ => return Err(format!("`{name}` with {} arguments has no form of the bytes", args.len())),
            }));
        }
        // A run of terms read forward from the token's first byte, as the lean lexer reads a number (`forward_run`).
        if self.partial
            && name == "run_last"
            && args.len() == 2
            && let Some(r) = self.forward_run(&m, args[1].trim())
        {
            return Ok(Some(r));
        }
        if self.lean
            && name != "bit"
            && let Some(r) = self.lean_read(name, args, &m)?
        {
            return Ok(Some(r));
        }
        Ok(Some(match (name, args.len()) {
            ("bit", 2) => {
                let e = self.expr(&m)?;
                let e = self.at_run_end(&e, args[1].trim()).unwrap_or(e);
                format!("({})", self.at(&e, args[1].trim(), 0)?)
            }
            ("last_in", 3) if self.partial => format!("wlast_in({}, {}, {})", args[1].trim(), args[2].trim(), self.window_fn(&m)?.0),
            ("first_bit", 2) => format!("wfirst_bit(n, {}, {})", args[1].trim(), windowed(self, &args[1])?),
            // A test of a range: its windows ask only of its bytes (`live_fn`).
            ("count_in", 3) => {
                windowed(self, &args[1])?;
                format!("wcount_in(n, {}, {}, {})", args[1].trim(), args[2].trim(), self.live_fn(&m, &args[2])?)
            }
            ("first_in", 3) => {
                windowed(self, &args[1])?;
                format!("wfirst_in(n, {}, {}, {})", args[1].trim(), args[2].trim(), self.live_fn(&m, &args[2])?)
            }
            ("any_in", 3) => {
                windowed(self, &args[1])?;
                format!("wfirst_in(n, {}, {}, {}).is_some()", args[1].trim(), args[2].trim(), self.live_fn(&m, &args[2])?)
            }
            ("first_clear", 3) => format!("wfirst_clear(n, {}, {}, {})", args[1].trim(), args[2].trim(), windowed(self, &args[1])?),
            ("run_last", 2) => format!("wrun_last(n, {}, {})", args[1].trim(), windowed(self, "s")?),
            ("run_once_last", 3) => {
                let Some(d) = mask(&args[1]) else { return Ok(None) };
                let r = windowed(self, "s")?;
                let (fd, esc) = self.window_fn(&d)?;
                if esc {
                    return Err(format!("`{d}` reads escapes"));
                }
                format!("wrun_once_last(n, {}, {r}, {fd})", args[2].trim())
            }
            _ => return Err(format!("`{name}` with {} arguments has no form of the bytes", args.len())),
        }))
    }

    /// `code` with each read of a mask in the windows' form, as a region's: tests of a token asked apart from the loop
    /// (decision 0655 step 6), with no block's words.
    pub(super) fn rewrite_windows(&mut self, code: &str) -> Result<String, String> {
        let mut out = String::new();
        self.in_region = true;
        for line in code.split_inclusive('\n') {
            // A row's comment names its kind (`// `kind`: rule`).
            if let Some((kind, _)) = line.trim_start().strip_prefix("// `").and_then(|r| r.split_once("`: ")) {
                self.row = Some(kind.to_owned());
            }
            out.push_str(&self.rewrite_line(line, line)?);
        }
        self.in_region = false;
        self.row = None;
        Ok(out)
    }

    /// `code` with each read of a mask in the bytes' form.
    pub(super) fn rewrite(&mut self, code: &str) -> Result<String, String> {
        let mut out = String::new();
        self.in_region = false;
        for line in code.split_inclusive('\n') {
            // A row's comment names its rule (`// `kind`: rule`): a region's is `read …`.
            if let Some(rule) = line.trim_start().strip_prefix("// `").and_then(|r| r.split_once("`: ")).map(|(_, r)| r) {
                self.in_region = rule.starts_with("read ");
            }
            out.push_str(&self.rewrite_line(line, line)?);
        }
        Ok(out)
    }

    fn rewrite_line(&mut self, text: &str, line: &str) -> Result<String, String> {
        let mut out = String::new();
        let mut rest = text;
        'scan: while !rest.is_empty() {
            // A word of a mask (`M[i]`, `prev(&M, i, k)`, `next(&M, i, k)`) in a pass over the words: its window there.
            if self.partial && !out.chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                if let Some(r) = self.word_read(rest)? {
                    out.push_str(&r.0);
                    rest = &rest[r.1..];
                    continue 'scan;
                }
            }
            for (name, _) in READS {
                // `put` only where a read may stay as it is (`partial_windows`).
                if (name == "put" || name == "last_in") && !self.partial {
                    continue;
                }
                let pat = format!("{name}(&");
                if rest.starts_with(&pat) && !out.chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    let inner = &rest[name.len() + 1..];
                    let Some(close) = closing(inner) else { return Err(format!("an unclosed call in `{}`", line.trim())) };
                    let args: Vec<String> = split_args(&inner[..close]).into_iter().map(|a| self.rewrite_line(&a, line)).collect::<Result<_, _>>()?;
                    let read = match self.read(name, &args, line) {
                        Err(_) if self.partial => {
                            if let Some(m) = args.get(usize::from(name == "put")).and_then(|a| a.trim().strip_prefix('&')) {
                                self.failed.insert(m.to_owned());
                            }
                            None
                        }
                        r => r?,
                    };
                    match read {
                        Some(r) => out.push_str(&r),
                        None => {
                            let _ = write!(out, "{name}({})", args.join(","));
                        }
                    }
                    rest = &inner[close + 1..];
                    continue 'scan;
                }
            }
            let c = rest.chars().next().expect("not empty");
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
        Ok(out)
    }

    /// A word of a mask at the start of `rest`, read as its window, and the length read: `M[i]`, `prev(&M, i, k)` or
    /// `next(&M, i, k)`, `i` a name.
    fn word_read(&mut self, rest: &str) -> Result<Option<(String, usize)>, String> {
        let name_len = |t: &str| t.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(t.len());
        for (f, sign) in [("prev(&", "-"), ("next(&", "+")] {
            if let Some(t) = rest.strip_prefix(f) {
                let m = &t[..name_len(t)];
                let Some(close) = closing(t) else { return Ok(None) };
                let args = split_args(&t[..close]);
                if !self.locals.contains_key(m) || args.len() != 3 || args[0].trim() != m {
                    return Ok(None);
                }
                let (i, k) = (args[1].trim(), args[2].trim());
                if name_len(i) != i.len() || k.parse::<u32>().is_err() {
                    return Ok(None);
                }
                let m = m.to_owned();
                if let Err(e) = self.window_fn(&m) {
                    self.failed.insert(m);
                    let _ = e;
                    return Ok(None);
                }
                return Ok(Some((format!("unsafe {{ w_{m}(src, n, ({i} * 64) as isize {sign} {k}, &mut 0) }}"), f.len() + close + 1)));
            }
        }
        let l = name_len(rest);
        let m = &rest[..l];
        if l == 0 || !self.locals.contains_key(m) || !rest[l..].starts_with('[') {
            return Ok(None);
        }
        let t = &rest[l + 1..];
        let Some(e) = t.find(']') else { return Ok(None) };
        let i = &t[..e];
        if i.is_empty() || name_len(i) != i.len() {
            return Ok(None);
        }
        let m = m.to_owned();
        if self.window_fn(&m).is_err() {
            self.failed.insert(m);
            return Ok(None);
        }
        Ok(Some((format!("unsafe {{ w_{m}(src, n, ({i} * 64) as isize, &mut 0) }}"), l + 1 + e + 1)))
    }

    /// The classes `e` reads, where it is made of classes and their shifts alone (no fill, no escape).
    fn few_classes(&self, e: &Expr, depth: usize) -> Option<usize> {
        if depth > 16 {
            return None;
        }
        match e {
            Expr::Name(n) if self.classes.contains_key(n.as_str()) => Some(1),
            Expr::Name(n) => self.few_classes(self.defs.get(n).or_else(|| self.locals.get(n))?, depth + 1),
            Expr::Not(a) => self.few_classes(a, depth + 1),
            Expr::And(a, b) | Expr::Or(a, b) | Expr::Xor(a, b) => Some(self.few_classes(a, depth + 1)? + self.few_classes(b, depth + 1)?),
            Expr::Call(f, args) if matches!(f.as_str(), "prev" | "next") => match args.first() {
                Some(Arg::E(x)) => self.few_classes(x, depth + 1),
                _ => None,
            },
            _ => None,
        }
    }

    /// The block words the loop reads (`block_word`), each made of its window at the block's first byte.
    pub(super) fn block_decls(&self) -> String {
        let mut out: String = self.block.iter().map(|m| format!("        let k_{m} = unsafe {{ w_{m}(src, n, base as isize, &mut 0) }};\n")).collect();
        if self.fwd_used {
            out.push_str("        // The terms a run read forward ended in (`forward_run`), at its token's last byte.\n        #[allow(unused_assignments)]\n        let mut end_last = 0u32;\n");
        }
        out
    }

    /// `e` at `at`, a byte from the last of the run the token read forward (`l`, `l + k`), with what the run tells of
    /// its bytes in place: the run's mask holds at `l`, at `l + 1` neither it nor its terms hold, and its terms at `l`
    /// are those it ended in. `None` where no run was read forward, or `at` is no byte from its last.
    fn at_run_end(&self, e: &Expr, at: &str) -> Option<Expr> {
        let (run, terms) = self.fwd.as_ref()?;
        let off: i64 = match at {
            "l" => 0,
            a => match a.strip_prefix("l + ").or_else(|| a.strip_prefix("l - ").map(|_| "")) {
                Some("") => return None,
                Some(k) => k.parse().ok()?,
                None => return None,
            },
        };
        let mut full = terms.clone();
        full.push(run.clone());
        let all = Expr::Not(Box::new(Expr::Empty));
        fn go(me: &Bytes, e: &Expr, off: i64, run: &Expr, terms: &[Expr], full: &[Expr], all: &Expr, depth: usize) -> Expr {
            if depth > 48 {
                return e.clone();
            }
            if e == run {
                return match off {
                    0 => all.clone(),
                    1 => Expr::Empty,
                    _ => e.clone(),
                };
            }
            if let Some(k) = terms.iter().position(|t| t == e) {
                return match off {
                    0 => Expr::Name(format!("\u{0}end{k}")),
                    1 => Expr::Empty,
                    _ => e.clone(),
                };
            }
            let r = |x: &Expr, o: i64| Box::new(go(me, x, o, run, terms, full, all, depth + 1));
            match e {
                Expr::Name(n) if !me.classes.contains_key(n.as_str()) => match me.locals.get(n).or_else(|| me.defs.get(n)) {
                    Some(d) if reads_terms(me, d, full, 0) => go(me, d, off, run, terms, full, all, depth + 1),
                    _ => e.clone(),
                },
                Expr::Call(f, args) if matches!(f.as_str(), "prev" | "next") => match Bytes::shift(f, args) {
                    Some((x, k)) if reads_terms(me, &x, full, 0) => {
                        let inner = go(me, &x, off + k, run, terms, full, all, depth + 1);
                        if inner == Expr::Empty || inner == *all || !reads_terms(me, &x, full, 0) {
                            inner
                        } else {
                            Expr::Call(f.clone(), vec![Arg::E(inner), Arg::N(k.unsigned_abs())])
                        }
                    }
                    _ => e.clone(),
                },
                Expr::Not(a) => Expr::Not(r(a, off)),
                Expr::And(a, b) => Expr::And(r(a, off), r(b, off)),
                Expr::Or(a, b) => Expr::Or(r(a, off), r(b, off)),
                Expr::Xor(a, b) => Expr::Xor(r(a, off), r(b, off)),
                _ => e.clone(),
            }
        }
        Some(fold(go(self, &self.norm(e, 0), off, run, terms, &full, &all, 0)))
    }

    /// `e` with every name of a mask in place, down to the classes: one form of a mask however the loop's locals or the
    /// grammar name it.
    fn norm(&self, e: &Expr, depth: usize) -> Expr {
        if depth > 64 {
            return e.clone();
        }
        let r = |x: &Expr| Box::new(self.norm(x, depth + 1));
        match e {
            Expr::Name(n) if self.classes.contains_key(n.as_str()) || n.starts_with('\u{0}') => e.clone(),
            Expr::Name(n) => {
                let k = n.strip_prefix("s1.").unwrap_or(n);
                match self.locals.get(k).or_else(|| self.defs.get(k)).or_else(|| self.locals.get(&format!("m_{k}"))) {
                    Some(d) => self.norm(d, depth + 1),
                    None => e.clone(),
                }
            }
            Expr::Not(a) => Expr::Not(r(a)),
            // A union's term that holds at no byte the other side can (`(word | high) & digit`): left out.
            Expr::And(a, b) => {
                let (a, b) = (self.norm(a, depth + 1), self.norm(b, depth + 1));
                let narrow = |u: Expr, by: &Expr| -> Expr {
                    let Some(set) = self.byte_set(by, 0) else { return u };
                    let mut ts = Vec::new();
                    fn ors(e: Expr, out: &mut Vec<Expr>) {
                        match e {
                            Expr::Or(a, b) => {
                                ors(*a, out);
                                ors(*b, out);
                            }
                            e => out.push(e),
                        }
                    }
                    if !matches!(u, Expr::Or(..)) {
                        return u;
                    }
                    ors(u, &mut ts);
                    let kept: Vec<Expr> = ts.into_iter().filter(|t| self.byte_set(t, 0).is_none_or(|x| (0..256).any(|c| x[c] && set[c]))).collect();
                    kept.into_iter().reduce(|x, y| Expr::Or(Box::new(x), Box::new(y))).unwrap_or(Expr::Empty)
                };
                let a2 = narrow(a.clone(), &b);
                let b2 = narrow(b, &a);
                Expr::And(Box::new(a2), Box::new(b2))
            }
            Expr::Or(a, b) => Expr::Or(r(a), r(b)),
            Expr::Xor(a, b) => Expr::Xor(r(a), r(b)),
            Expr::Call(f, args) => Expr::Call(
                f.clone(),
                args.iter()
                    .map(|a| match a {
                        Arg::E(x) => Arg::E(self.norm(x, depth + 1)),
                        a => a.clone(),
                    })
                    .collect(),
            ),
            _ => e.clone(),
        }
    }

    /// The block's word of mask `m` (`k_m`), made once a block.
    pub(super) fn block_word(&mut self, m: &str) -> String {
        if !self.block.iter().any(|b| b == m) {
            self.block.push(m.to_owned());
        }
        format!("k_{m}")
    }

    /// The block's token starts: the bytes `skip` does not hold, from block `b`'s word.
    pub(super) fn starts(&mut self, skip: &str) -> Result<String, String> {
        let (_, esc) = self.window_fn(skip)?;
        if esc {
            return Err(format!("`{skip}` reads escapes"));
        }
        Ok(format!("!{}", self.block_word(skip)))
    }

    /// The block's words, made once a block where the loop comes to it, as the lean lexer's `Window`: each class the
    /// words read classified once a block (`block_classes`, one load and one classify a pair of nibble tables), a class
    /// a few bytes on or back its bits shifted, the neighbour block's bytes tested where they are and shifted in (lean's
    /// `shift_in`). With the declarations the loop's head needs before it. A word of escapes, or of a class 64 bytes away, reads its windows as a window past the block.
    pub(super) fn block_words(&mut self) -> (String, String) {
        match self.block_from_classes() {
            Some(r) => r,
            None => {
                let mut out = String::from("        // The block's masks, from its bytes (the lean lexer's `Window`): what its tokens read of it.\n");
                for m in &self.block {
                    let _ = writeln!(out, "        let k_{m} = unsafe {{ w_{m}(src, n, base as isize, &mut 0) }};");
                }
                for (f, _) in &self.pairs {
                    let _ = writeln!(out, "        let k_{f} = unsafe {{ w_{f}(src, base) }};");
                }
                (String::new(), out)
            }
        }
    }

    /// `block_words` from the block's classes: none where a word reads escapes or a class 64 bytes away.
    fn block_from_classes(&mut self) -> Option<(String, String)> {
        let mut used: Vec<String> = Vec::new();
        let mut words = Vec::new();
        for m in self.block.clone() {
            let e = self.expr(&m).ok()?;
            words.push((m, self.blk(&e, 0, 0, &mut used)?));
        }
        // Each pairs mask of the block's one load too, as lean's `pair`: the byte's nibble tables name the pairs it
        // starts, the next byte's (`shift_in`) the pairs it ends; a pair is a bit of both, packed once. Eight pairs a
        // pair of tables.
        let mut statics = String::new();
        let mut shifted = false;
        for (f, two) in self.pairs.clone() {
            let mut groups = Vec::new();
            for (g, chunk) in two.chunks(8).enumerate() {
                let (mut flo, mut fhi, mut slo, mut shi) = ([0u8; 16], [0u8; 16], [0u8; 16], [0u8; 16]);
                for (bit, &(a, b)) in chunk.iter().enumerate() {
                    flo[usize::from(a & 15)] |= 1 << bit;
                    fhi[usize::from(a >> 4)] |= 1 << bit;
                    slo[usize::from(b & 15)] |= 1 << bit;
                    shi[usize::from(b >> 4)] |= 1 << bit;
                }
                let t = format!("{}_{g}", f.to_uppercase());
                let row = |t: &[u8; 16]| t.iter().map(|b| format!("{b:#04x}")).collect::<Vec<_>>().join(", ");
                let _ = write!(statics, "/// `{f}`'s pairs ({}): a first byte's bits the pairs it starts, a second's those it ends.\nstatic {t}_FIRST: ([u8; 16], [u8; 16]) = ([{}], [{}]);\nstatic {t}_SECOND: ([u8; 16], [u8; 16]) = ([{}], [{}]);\n\n", chunk.iter().map(|(a, b)| format!("`{}{}`", *a as char, *b as char)).collect::<Vec<_>>().join(", "), row(&flo), row(&fhi), row(&slo), row(&shi));
                let c = format!("{f}_{g}");
                self.class_code.insert(c.clone(), format!("s::to_bitmask(s::test(s::classify(v, &{t}_FIRST.0, &{t}_FIRST.1), s::classify(d, &{t}_SECOND.0, &{t}_SECOND.1)))"));
                used.push(c);
                groups.push(format!("cur[{}]", used.len() - 1));
                shifted = true;
            }
            words.push((f, if groups.is_empty() { "0u64".to_owned() } else { groups.join(" | ") }));
        }
        // The lean lexer's classes (decision 0655 step 10): the classes the block's words read and the rare tokens'
        // first bytes, packed in nibble tables of their own, a class the bits it is the union of, each named (lean's
        // `SPACE`, `WORD`, `RARE`). No bit a class no word reads.
        let mut lean_tables = None;
        if self.lean {
            let mut sets: Vec<(String, [bool; 256])> = used.iter().filter(|c| self.classes.contains_key(c.as_str())).map(|c| (c.clone(), class_set(self.classes[c.as_str()]))).collect();
            if let Some(set) = self.rare {
                sets.push(("rare".to_owned(), set));
            }
            let mut packs: Vec<Vec<(u16, u16)>> = Vec::new();
            let mut named: Vec<(String, usize, u8)> = Vec::new();
            // A class of every byte past ASCII and some ASCII ones: the ASCII ones its bits, the others a compare, as
            // lean's `rare` (`s::ge(x, 0xc0)`).
            let mut high: Vec<String> = Vec::new();
            // The smaller classes first, by their ASCII bytes: a larger one shares their bits where it holds them.
            sets.sort_by_key(|(_, set)| set[..0x80].iter().filter(|&&b| b).count());
            for (c, set) in &sets {
                let bytes: Vec<usize> = (0..256).filter(|&b| set[b]).collect();
                if let [b] = bytes.as_slice() {
                    self.class_code.insert(c.clone(), format!("s::eq_mask(v, {b:#04x})"));
                    continue;
                }
                let mut set = *set;
                if (0x80..256).all(|b| set[b]) && (0..0x80).any(|b| set[b]) {
                    (0x80..256).for_each(|b| set[b] = false);
                    high.push(c.clone());
                }
                let (p, bits) = cover(&mut packs, &set)?;
                named.push((c.clone(), p, bits));
            }
            let one = packs.len() == 1;
            // Not the stage-1 tables' names (`LO_0`), which the file holds too until they go unread.
            let c_n = |p: usize| if one { ("LO".to_owned(), "HI".to_owned(), "c".to_owned()) } else { (format!("LO_{}", (b'A' + p as u8) as char), format!("HI_{}", (b'A' + p as u8) as char), format!("c{p}")) };
            let mut text = String::new();
            for (c, p, bits) in &named {
                let k = c.to_uppercase();
                let _ = writeln!(text, "/// `{c}`: its bits in the classes' nibble tables{}.\nconst {k}: u8 = {bits:#04x};", if one { String::new() } else { format!(", pair {p}") });
                let test = format!("s::test({}, s::splat({k}))", c_n(*p).2);
                let test = if high.contains(c) { format!("s::or({test}, s::ge(v, 0x80))") } else { test };
                self.class_code.insert(c.clone(), format!("s::to_bitmask({test})"));
            }
            let row = |t: &[u8; 16]| t.iter().map(|b| format!("{b:#04x}")).collect::<Vec<_>>().join(", ");
            let mut heads = String::new();
            for (p, rects) in packs.iter().enumerate() {
                let (mut lo, mut hi) = ([0u8; 16], [0u8; 16]);
                for (bit, &(h, l)) in rects.iter().enumerate() {
                    for i in 0..16 {
                        if l >> i & 1 == 1 {
                            lo[i] |= 1 << bit;
                        }
                        if h >> i & 1 == 1 {
                            hi[i] |= 1 << bit;
                        }
                    }
                }
                let (l, h, c) = c_n(p);
                let _ = write!(text, "/// The classes' nibble tables{} (the lean lexer's `LO`, `HI`): a byte's bit `b` is set in both where it is of bit `b`'s\n/// high and low nibbles.\nstatic {l}: [u8; 16] = [{}];\nstatic {h}: [u8; 16] = [{}];\n", if one { String::new() } else { format!(", pair {p}") }, row(&lo), row(&hi));
                let _ = writeln!(heads, "        let {c} = s::classify(v, &{l}, &{h});");
            }
            text.push('\n');
            if self.rare.is_some() {
                used.push("rare".to_owned());
                words.push(("rare".to_owned(), format!("cur[{}]", used.len() - 1)));
            }
            statics.insert_str(0, &text);
            lean_tables = Some(heads);
        }
        // The rare tokens' first bytes (lean's `rare`): a byte's nibbles' bits, a set of high nibbles times a set of low
        // ones a bit, eight a pair of tables.
        if let Some(set) = self.rare.filter(|_| lean_tables.is_none()) {
            let mut rects: Vec<(u16, u16)> = Vec::new();
            for h in 0..16usize {
                let lows = (0..16usize).filter(|&l| set[h << 4 | l]).fold(0u16, |m, l| m | 1 << l);
                if lows == 0 {
                    continue;
                }
                match rects.iter_mut().find(|r| r.1 == lows) {
                    Some(r) => r.0 |= 1 << h,
                    None => rects.push((1 << h, lows)),
                }
            }
            let mut groups = Vec::new();
            for (g, chunk) in rects.chunks(8).enumerate() {
                let (mut lo, mut hi) = ([0u8; 16], [0u8; 16]);
                for (bit, &(h, l)) in chunk.iter().enumerate() {
                    for i in 0..16 {
                        if l >> i & 1 == 1 {
                            lo[i] |= 1 << bit;
                        }
                        if h >> i & 1 == 1 {
                            hi[i] |= 1 << bit;
                        }
                    }
                }
                let row = |t: &[u8; 16]| t.iter().map(|b| format!("{b:#04x}")).collect::<Vec<_>>().join(", ");
                let _ = write!(statics, "/// The bytes a rare token starts with, part {g}: a byte's bit `b` is set in both where it is of bit `b`'s high and low nibbles.\nstatic RARE_{g}: ([u8; 16], [u8; 16]) = ([{}], [{}]);\n\n", row(&lo), row(&hi));
                let c = format!("rare_{g}");
                self.class_code.insert(c.clone(), format!("s::to_bitmask(s::test(s::classify(v, &RARE_{g}.0, &RARE_{g}.1), s::splat(0xff)))"));
                used.push(c);
                groups.push(format!("cur[{}]", used.len() - 1));
            }
            words.push(("rare".to_owned(), if groups.is_empty() { "0u64".to_owned() } else { groups.join(" | ") }));
        }
        if used.is_empty() {
            return None;
        }
        // The classes' bits of the block at `base` (in the file), each its class's test of its nibble tables' pair.
        let mut tables = lean_tables.clone().unwrap_or_default();
        for k in (0..8).filter(|_| lean_tables.is_none()) {
            if used.iter().any(|c| self.class_code.get(c.as_str()).is_some_and(|code| code.contains(&format!("c{k},")) || code.contains(&format!("c{k})")))) {
                let _ = writeln!(tables, "        let c{k} = s::classify(v, &LO_{k}, &HI_{k});");
            }
        }
        let bits: Vec<String> = used.iter().map(|c| self.class_bits(c)).collect::<Option<_>>()?;
        self.code.push_str(&statics);
        if shifted {
            // The block a byte on: its last byte the next block's first, in the padded source.
            tables.insert_str(0, "        let d = s::shift_in(v, src[base + 64]);\n");
        }
        let k = used.len();
        let _ = write!(
            self.code,
            "/// The classes the block's words read ({}), each's bits in the block at `base`, a byte of the file: one load, and one\n/// classify a pair of nibble tables, as the lean lexer's `Window`. None past the file.\n#[inline(always)]\nunsafe fn block_classes(src: &[u8], n: usize, base: usize) -> [u64; {k}] {{\n    let block: &[u8; 64] = src[base..base + 64].try_into().expect(\"the padded source\");\n    let lv = valid(n, base as isize);\n    // The block's load and its tables (safe: `std::simd`'s operations).\n    {{\n        let v = s::load(block);\n{tables}        [{}]\n    }}\n}}\n\n",
            used.iter().map(|c| format!("`{c}`")).collect::<Vec<_>>().join(", "),
            bits.iter().map(|b| format!("{b} & lv")).collect::<Vec<_>>().join(", ")
        );
        let decl = String::new();
        let mut head = String::from("        // The block's masks, from its classes (the lean lexer's `Window`): what its tokens read of it.\n");
        let _ = writeln!(head, "        // SAFETY: `base` is in the file.\n        let cur = unsafe {{ block_classes(src, n, base) }};");
        if words.iter().any(|(_, w)| w.contains("lv")) {
            head.push_str("        let lv = valid(n, base as isize);\n");
        }
        for (m, w) in words {
            let _ = writeln!(head, "        let k_{m} = {w};");
        }
        Some((decl, head))
    }

    /// Class `c`'s bits of the byte vector `v` (its nibble tables' `cP`, or its ranges' tests).
    fn class_bits(&self, c: &str) -> Option<String> {
        if let Some(code) = self.class_code.get(c) {
            return Some(code.trim_end_matches(" & lv").to_owned());
        }
        let rs = self.classes.get(c)?;
        let bits: Vec<String> = rs
            .iter()
            .map(|&(lo, hi)| match (lo, hi) {
                (lo, hi) if lo == hi => format!("s::eq_mask(v, {lo:#04x})"),
                (lo, 255) => format!("s::to_bitmask(s::ge(v, {lo:#04x}))"),
                (0, hi) => format!("!s::to_bitmask(s::ge(v, {:#04x}))", hi + 1),
                (lo, hi) => format!("(s::to_bitmask(s::ge(v, {lo:#04x})) & !s::to_bitmask(s::ge(v, {:#04x})))", hi + 1),
            })
            .collect();
        Some(if bits.is_empty() { "0".to_owned() } else { format!("({})", bits.join(" | ")) })
    }

    /// `e`'s bits in the block, `off` bytes on (back, negative), from the block's classes (`cur`, and the neighbour
    /// block's bytes tested where they are): none where it reads escapes or a class 64 bytes away.
    fn blk(&self, e: &Expr, off: i64, depth: usize, used: &mut Vec<String>) -> Option<String> {
        if depth > 64 || off.abs() >= 64 {
            return None;
        }
        Some(match e {
            Expr::Empty => "0u64".to_owned(),
            Expr::Name(n) if self.classes.contains_key(n.as_str()) => {
                let i = match used.iter().position(|c| c == n) {
                    Some(i) => i,
                    None => {
                        used.push(n.clone());
                        used.len() - 1
                    }
                };
                let rs = self.classes[n.as_str()];
                match off {
                    0 => format!("cur[{i}]"),
                    // Bytes on or back: the neighbour block's bytes, each tested where it is (lean's `shift_in`), not
                    // the block classified.
                    o if o > 0 => {
                        let next: Vec<String> = (0..o).map(|j| format!("u64::from(base + {} < n && {}) << {}", 64 + j, pattern_test(&format!("src[base + {}]", 64 + j), rs), 64 - o + j)).collect();
                        format!("(cur[{i}] >> {o} | {})", next.join(" | "))
                    }
                    o => {
                        let k = -o;
                        let before: Vec<String> = (0..k).map(|j| format!("u64::from(base >= {} && {}){}", k - j, pattern_test(&format!("src[base - {}]", k - j), rs), if j == 0 { String::new() } else { format!(" << {j}") })).collect();
                        format!("(cur[{i}] << {k} | {})", before.join(" | "))
                    }
                }
            }
            Expr::Name(n) => self.blk(self.defs.get(n)?, off, depth + 1, used)?,
            Expr::Not(a) => format!("(!{} & lv)", self.blk(a, off, depth + 1, used)?),
            Expr::And(a, b) => format!("({} & {})", self.blk(a, off, depth + 1, used)?, self.blk(b, off, depth + 1, used)?),
            Expr::Or(a, b) => format!("({} | {})", self.blk(a, off, depth + 1, used)?, self.blk(b, off, depth + 1, used)?),
            Expr::Xor(a, b) => format!("({} ^ {})", self.blk(a, off, depth + 1, used)?, self.blk(b, off, depth + 1, used)?),
            Expr::Call(f, args) => {
                let (a, k) = Self::shift(f, args)?;
                self.blk(&a, off + k, depth + 1, used)?
            }
            Expr::Test { .. } => return None,
        })
    }

    /// The window functions and helpers the rewritten reads call.
    pub(super) fn code(&self) -> String {
        let named: String = self.named.borrow().iter().map(|x| x.1.as_str()).collect();
        format!("{HELPERS}{}{named}{}{}", if self.fence { FENCE } else { "" }, self.class_table(), self.code)
    }
}

/// The index of the `)` that closes the call whose arguments start `s`.
pub(super) fn closing(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth > 0 => depth -= 1,
            ')' => return Some(i),
            _ => {}
        }
    }
    None
}

/// A call's arguments, split at its own commas.
fn split_args(s: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(String::new());
                continue;
            }
            _ => {}
        }
        out.last_mut().expect("one").push(c);
    }
    out
}

/// A call `unsafe { f }` as it reads inside an `unsafe` block: `f`.
fn in_unsafe(call: &str) -> &str {
    call.strip_prefix("unsafe { ").and_then(|c| c.strip_suffix(" }")).unwrap_or(call)
}

/// A byte's test of a class's ranges.
pub(super) fn pattern_test(b: &str, rs: &[(u32, u32)]) -> String {
    if rs.is_empty() {
        return "false".to_owned();
    }
    // `src[(s + 1)]` as `src[s + 1]`; a byte by its literal where it has one (`b'/'`), as hand-written Rust.
    let b = match b.strip_prefix("src[(").and_then(|x| x.strip_suffix(")]")).filter(|x| !x.contains('(')) {
        Some(i) => format!("src[{i}]"),
        None => b.to_owned(),
    };
    let byte = |x: u32| u8::try_from(x).map_or_else(|_| format!("{x:#04x}"), lit);
    match rs {
        [(lo, hi)] if lo == hi => format!("({b} == {})", byte(*lo)),
        _ => format!("matches!({b}, {})", rs.iter().map(|&(lo, hi)| if lo == hi { byte(lo) } else { format!("{}..={}", byte(lo), byte(hi)) }).collect::<Vec<_>>().join(" | ")),
    }
}

/// Bytes `w` as Rust writes them (`b"=>"`).
fn byte_str(w: &[u8]) -> String {
    let mut s = String::from("b\"");
    for &x in w {
        match x {
            b'"' => s.push_str("\\\""),
            b'\\' => s.push_str("\\\\"),
            0x20..=0x7e => s.push(x as char),
            _ => {
                let _ = write!(s, "\\x{x:02x}");
            }
        }
    }
    s.push('"');
    s
}

/// Byte `x` as Rust writes it (`b'*'`, `b'\\n'`).
pub(super) fn lit(x: u8) -> String {
    match x {
        b'\n' => "b'\\n'".to_owned(),
        b'\\' => "b'\\\\'".to_owned(),
        b'\'' => "b'\\''".to_owned(),
        0x20..=0x7e => format!("b'{}'", x as char),
        _ => format!("{x:#04x}"),
    }
}

/// The name of the bits a range test asks of (`live_fn`), in a mask's expression.
const LIVE: &str = "\u{0}live";

/// `e` with the range's bits (`LIVE`) ANDed into the first test of each side: down an `And`'s first operand, into both
/// of an `Or`'s. Every early exit (`win`'s `And`) then asks only of them.
/// The tests of a set of texts (a `pairs` mask: `text("->") | text("==") | …`) as the lean lexer's pair tables: for
/// each length, a table a byte position, each byte's bits the texts with it there (`FIRST[c] & FOLLOW[d]`), so a text
/// holds where every position's bits meet. With the tables (statics) before the test, by the test's name `f`. `None`
/// where `d` is no set of four texts or more, of at most 64 of a length.
fn texts_table(f: &str, d: &Expr, byte: &dyn Fn(&str) -> Option<u8>) -> Option<String> {
    // A text: `text(T)`, or a pair's bytes (`pair_byte_2d & next(pair_byte_3e)`, classes of one byte, one at each offset).
    fn texts(e: &Expr, out: &mut Vec<Vec<u8>>, byte: &dyn Fn(&str) -> Option<u8>) -> Option<()> {
        match e {
            Expr::Or(a, b) => {
                texts(a, out, byte)?;
                texts(b, out, byte)
            }
            Expr::Call(c, args) if c == "text" && let [Arg::S(t)] = args.as_slice() && !t.is_empty() => {
                out.push(t.clone());
                Some(())
            }
            Expr::And(..) => {
                fn at(e: &Expr, out: &mut Vec<(i64, u8)>, byte: &dyn Fn(&str) -> Option<u8>) -> Option<()> {
                    match e {
                        Expr::And(a, b) => {
                            at(a, out, byte)?;
                            at(b, out, byte)
                        }
                        Expr::Name(c) => {
                            out.push((0, byte(c)?));
                            Some(())
                        }
                        Expr::Call(f, args) if f == "next" => match args.as_slice() {
                            [Arg::E(Expr::Name(c))] => {
                                out.push((1, byte(c)?));
                                Some(())
                            }
                            [Arg::E(Expr::Name(c)), Arg::N(k)] => {
                                out.push((*k as i64, byte(c)?));
                                Some(())
                            }
                            _ => None,
                        },
                        _ => None,
                    }
                }
                let mut bs = Vec::new();
                at(e, &mut bs, byte)?;
                bs.sort_unstable();
                if !bs.iter().enumerate().all(|(k, (o, _))| *o == k as i64) {
                    return None;
                }
                out.push(bs.into_iter().map(|(_, b)| b).collect());
                Some(())
            }
            _ => None,
        }
    }
    let mut ts = Vec::new();
    texts(d, &mut ts, byte)?;
    if ts.len() < 4 {
        return None;
    }
    let mut lens: Vec<usize> = ts.iter().map(Vec::len).collect();
    lens.sort_unstable();
    lens.dedup();
    let mut statics = String::new();
    let mut tests = Vec::new();
    for len in lens {
        let of: Vec<&Vec<u8>> = ts.iter().filter(|t| t.len() == len).collect();
        if of.len() > 64 {
            return None;
        }
        let ty = if of.len() <= 32 { "u32" } else { "u64" };
        let mut reads = Vec::new();
        for k in 0..len {
            let mut tab = [0u64; 256];
            for (bit, t) in of.iter().enumerate() {
                tab[usize::from(t[k])] |= 1 << bit;
            }
            let name = format!("{}_{len}_{k}", f.to_uppercase());
            let row: Vec<String> = tab.iter().map(|b| format!("{b:#x}")).collect();
            let _ = writeln!(statics, "/// `{f}`'s texts of {len} bytes: each byte's bits the texts with it at their byte {k}.\nstatic {name}: [{ty}; 256] = [{}];", row.join(", "));
            reads.push(if k == 0 { format!("{name}[usize::from(src[q])]") } else { format!("{name}[usize::from(src[q + {k}])]") });
        }
        tests.push(format!("(q + {len} <= n && {} != 0)", if reads.len() == 1 { reads[0].clone() } else { format!("({})", reads.join(" & ")) }));
    }
    Some(format!("\u{0}{statics}\u{0}{}", tests.join(" || ")))
}

fn with_live(e: &Expr) -> Expr {
    match e {
        Expr::And(a, b) => Expr::And(Box::new(with_live(a)), b.clone()),
        Expr::Or(a, b) => Expr::Or(Box::new(with_live(a)), Box::new(with_live(b))),
        _ => Expr::And(Box::new(Expr::Name(LIVE.to_owned())), Box::new(e.clone())),
    }
}

fn ident(n: &str) -> String {
    n.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

/// `code` with each run of a token's tests over the same bytes (`if wcount_in(n, s, l, w_A) >= 1 { … }`, one after
/// another, comments between) read in one window loop (`wany_in`), as the lean lexer reads a string once: the tests'
/// classes are then loaded once a window, not once a test.
/// `code` (the rows' match) with each row's region end and its tests over the region (`wfirst_bit` from the opener's
/// next byte, then `wany_in` of its tests from the opener) read in one window loop from the opener (`wfirst_any`), as the
/// lean lexer reads a string once (`string_end`). Where the row's first bytes (`firsts`, by row) start no escape of the
/// end's mask, so the escapes from the opener are those from the byte after it.
pub(super) fn fuse_read(code: &str, firsts: &[[bool; 256]], escapes: &HashMap<String, String>, escape_bytes: &HashMap<String, [bool; 256]>) -> String {
    let lines: Vec<&str> = code.lines().collect();
    let mut out = String::new();
    let mut row: Option<usize> = None;
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        if let Some(k) = l.strip_prefix("                ").and_then(|t| t.strip_suffix(" => {")).and_then(|t| t.parse::<usize>().ok()) {
            row = Some(k);
        }
        let fused = (|| -> Option<(String, usize)> {
            let first = firsts.get(row?)?;
            let ind = l.len() - l.trim_start().len();
            let pad = " ".repeat(ind);
            let f = l.trim_start().strip_prefix("let l = match wfirst_bit(n, s + 1, |p: isize, esc: &mut u64| unsafe { ")?.strip_suffix("(src, n, p, esc) }) {")?;
            (lines.get(i + 1)? == &format!("{pad}    Some(c) => c + 0,")).then_some(())?;
            // The `None` arm, to the match's end.
            let close = (i + 2..lines.len()).find(|&j| lines[j] == format!("{pad}}};"))?;
            let none: Vec<&str> = lines[i + 2..close].to_vec();
            (lines.get(close + 1)? == &format!("{pad}let l = l.max(s);")).then_some(())?;
            let mut j = close + 2;
            while lines.get(j)?.trim_start().starts_with("//") {
                j += 1;
            }
            let rest = lines[j].trim_start().strip_prefix("let hit = wany_in(n, s, l, |p: isize, esc: &mut [u64; ")?;
            let (_, rest) = rest.split_once("]| unsafe { let escaped = escaped_step(")?;
            let (b, calls) = rest.split_once(", &mut esc[0]); [")?;
            let calls = calls.strip_suffix("] });")?;
            (escapes.get(f)? == b).then_some(())?;
            let bytes = escape_bytes.get(b)?;
            (!(0..256).any(|c| first[c] && bytes[c])).then_some(())?;
            let e = f.strip_prefix("w_")?;
            let mut t = String::new();
            let _ = writeln!(t, "{pad}// The region's end and its tests in one window loop from its opener, the escapes found once a window, as the");
            let _ = writeln!(t, "{pad}// lean lexer reads a string once: no first byte of the row starts an escape.");
            let _ = writeln!(t, "{pad}let (l, hit) = match wfirst_any(n, s, |p: isize, esc: &mut u64| unsafe {{ let escaped = escaped_step({}, esc); (e_{e}(src, n, p, escaped), [{calls}]) }}) {{", in_unsafe(b));
            let _ = writeln!(t, "{pad}    Some((c, hit)) => (c, hit),");
            for x in &none {
                let _ = writeln!(t, "{x}");
            }
            let _ = writeln!(t, "{pad}}};");
            let _ = writeln!(t, "{pad}let l = l.max(s);");
            Some((t, j + 1))
        })();
        match fused {
            Some((t, next)) => {
                out.push_str(&t);
                i = next;
            }
            None => {
                out.push_str(l);
                out.push('\n');
                i += 1;
            }
        }
    }
    out
}

pub(super) fn fuse_tests(code: &str, escapes: &HashMap<String, String>) -> String {
    let lines: Vec<&str> = code.lines().collect();
    // A test's head: its indent, its span and its window function.
    let head = |l: &str| -> Option<(usize, String, String)> {
        let ind = l.len() - l.trim_start().len();
        let rest = l.trim_start().strip_prefix("if wcount_in(n, ")?;
        let (span, f) = rest.split_once(", |p: isize, esc: &mut u64| unsafe { ")?;
        // A range's test (`live_fn`, `l_`), or one over every byte of its windows (`w_`).
        let b = span.split_once(", ")?.1;
        let f = f.strip_suffix("(src, n, p, esc) }) >= 1 {").or_else(|| f.strip_suffix(&format!("(src, n, p, esc, upto(p, {b})) }}) >= 1 {{")))?;
        ((f.starts_with("w_") || f.starts_with("l_")) && !f.contains(' ')).then(|| (ind, span.to_owned(), f.to_owned()))
    };
    // The line after a test's block, from its head at `i`.
    let end = |i: usize, ind: usize| -> Option<usize> {
        let close = format!("{}}}", " ".repeat(ind));
        (i + 1..lines.len()).find(|&j| lines[j] == close).map(|j| j + 1)
    };
    let mut out = String::new();
    let mut i = 0;
    while i < lines.len() {
        let Some((ind, span, _)) = head(lines[i]) else {
            out.push_str(lines[i]);
            out.push('\n');
            i += 1;
            continue;
        };
        // The run: tests of the same span and indent, only comments between.
        let mut tests = Vec::new();
        let (mut j, mut last) = (i, i);
        while j < lines.len() {
            match head(lines[j]) {
                Some((d, sp, f)) if d == ind && sp == span => {
                    let Some(e) = end(j, ind) else { break };
                    tests.push((j, f));
                    j = e;
                    last = e;
                }
                _ if lines[j].trim_start().starts_with("//") && lines[j].len() - lines[j].trim_start().len() == ind => j += 1,
                _ => break,
            }
        }
        if tests.len() < 2 {
            out.push_str(lines[i]);
            out.push('\n');
            i += 1;
            continue;
        }
        let pad = " ".repeat(ind);
        // Tests that read the escapes of one mask find them once a window, with one carry, as the lean lexer's string
        // reader passes each escape once.
        let of: Vec<Option<&String>> = tests.iter().map(|(_, f)| escapes.get(&format!("w_{}", &f[2..]))).collect();
        let b = span.split_once(", ").map_or("", |(_, b)| b);
        let shared = of.iter().all(|o| o.is_some() && *o == of[0]);
        let _ = writeln!(out, "{pad}// The token's tests of its bytes, in one window loop: their classes loaded once a window.");
        if let (true, Some(Some(b))) = (shared, of.first()) {
            let calls: Vec<String> = tests.iter().map(|(_, f)| format!("e_{}(src, n, p, escaped)", &f[2..])).collect();
            let _ = writeln!(out, "{pad}let hit = wany_in(n, {span}, |p: isize, esc: &mut [u64; {}]| unsafe {{ let escaped = escaped_step({}, &mut esc[0]); [{}] }});", tests.len(), in_unsafe(b), calls.join(", "));
        } else {
            let calls: Vec<String> = tests.iter().enumerate().map(|(k, (_, f))| if f.starts_with("l_") { format!("{f}(src, n, p, &mut esc[{k}], upto(p, {b}))") } else { format!("{f}(src, n, p, &mut esc[{k}])") }).collect();
            let _ = writeln!(out, "{pad}let hit = wany_in(n, {span}, |p: isize, esc: &mut [u64; {}]| unsafe {{ [{}] }});", tests.len(), calls.join(", "));
        }
        for (k, line) in lines.iter().enumerate().take(last).skip(i) {
            match tests.iter().position(|(h, _)| *h == k) {
                Some(t) => {
                    let _ = writeln!(out, "{pad}if hit[{t}] {{");
                }
                None => {
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        i = last;
    }
    if !code.ends_with('\n') {
        out.pop();
    }
    out
}

/// `code` without the counts' scans (`scan … max N`, decision 0623): where tier 1 reads the bytes, tier 2 is one loop
/// over its own stack, which refuses a level past its `DEPTH`. So a file nested too deep still ends in a refusal, and
/// no token counts its brackets.
pub(super) fn strip_scans(code: &str) -> String {
    let mut out = String::new();
    let mut lines = code.lines();
    while let Some(line) = lines.next() {
        if line.trim_start().starts_with("// scan ") {
            // To the scan loop's end: its `q = i + 1;`, then its closing brace.
            for l in lines.by_ref() {
                if l.trim() == "q = i + 1;" {
                    lines.next();
                    break;
                }
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !code.ends_with('\n') {
        out.pop();
    }
    out
}

/// The loop's own pauses (decision 0619 step 2), where stage 1 made the blocks as the loop came to need them: none
/// where the loop reads the bytes.
pub(super) fn strip_pauses(code: &str) -> String {
    let lines: Vec<&str> = code.split_inclusive('\n').collect();
    let t = |i: usize| lines.get(i).map(|l| l.trim()).unwrap_or("");
    let mut out = String::new();
    let mut i = 0;
    while i < lines.len() {
        if t(i) == "// The byte after the token in a block stage 1 has not made: it makes more, and the token is read again." && t(i + 1) == "if l + 1 >= 64 * made && made < nw {" {
            i += 5;
            continue;
        }
        if t(i) == "if made < nw {" && t(i + 1) == "wait = (2 * made - b).min(nw);" && t(i + 2) == "continue 'pass;" && t(i + 3) == "}" {
            i += 4;
            continue;
        }
        out.push_str(lines[i]);
        i += 1;
    }
    out
}

/// The helpers every window reads: the 64 bytes loaded where they are, the bits of the file's bytes, the escapes
/// carried from window to window, and the searches 64 bytes at a time.
/// The lean lexer's `find_fence`: where a fence of bytes next starts from `from`, by its first two (`find_pair`), then the
/// rest compared.
const FENCE: &str = r#"/// Where `fence` next starts, from `from`: its first two bytes (`micro::find_pair`), then the rest compared, as the lean
/// lexer's `find_fence`.
fn find_fence(b: &[u8], from: usize, fence: &[u8]) -> Option<usize> {
    let mut i = from;
    loop {
        let at = micro::find_pair(b, i, fence[0], fence[1])?;
        if b[at..].starts_with(fence) {
            return Some(at);
        }
        i = at + 1;
    }
}

"#;

const HELPERS: &str = r#"/// The last byte of a run after byte `s`: from block `base`'s word `k` where the run's next byte is in the block (the
/// lean lexer's `micro::run`), then the bytes past it, `w` each one's test (its `word_end`).
#[inline(always)]
fn word_last(n: usize, base: usize, k: u64, s: usize, w: impl Fn(usize) -> bool) -> usize {
    let q = s + 1 - base;
    let mut l = s;
    if q < 64 {
        let ones = (!(k >> q)).trailing_zeros() as usize;
        if q + ones < 64 {
            return s + ones;
        }
        l = base + 63;
    }
    while l + 1 < n && w(l + 1) {
        l += 1;
    }
    l
}

/// The 64 bytes from `p` as loaded, and the byte they start at: at the file's first byte for a window before it. None
/// for a window past the file's end.
#[inline(always)]
unsafe fn window(src: &[u8], n: usize, p: isize) -> Option<(s::V, isize)> {
    if p >= n as isize || p <= -64 {
        return None;
    }
    let q = p.max(0);
    Some((
        match src.get(q as usize..q as usize + 64) {
            Some(block) => s::load(block.try_into().expect("64 bytes")),
            // A source with no padding (tier 2's, asking a token's tests): its last bytes, and spaces as the padding's.
            None => {
                let mut block = [b' '; 64];
                block[..src.len() - q as usize].copy_from_slice(&src[q as usize..]);
                s::load(&block)
            }
        },
        q,
    ))
}

/// The bits `b` of the window loaded at `q`, as the window from `p`'s: none past the file or before it.
#[inline(always)]
fn held(b: u64, q: isize, n: usize, p: isize) -> u64 {
    let b = if p < q { b << (q - p) } else { b };
    b & valid(n, p)
}

/// `held` of a class the padding holds none of: the bits before the file shifted out, those past it none.
#[inline(always)]
fn padded(b: u64, q: isize, p: isize) -> u64 {
    if p < q { b << (q - p) } else { b }
}

/// The bits of the window from `p` that are the file's bytes.
#[inline(always)]
fn valid(n: usize, p: isize) -> u64 {
    let left = n as isize - p;
    let hi = if left >= 64 { !0 } else if left <= 0 { 0 } else { (1u64 << left) - 1 };
    if p < 0 { hi & (!0u64 << (-p).min(63)) } else { hi }
}

/// The escape carry of the window from `p`, from the bytes: the parity of the run of bytes `t` holds that ends before
/// it.
#[inline(always)]
fn esc_back(p: isize, t: impl Fn(usize) -> bool) -> u64 {
    let mut c = 0;
    let mut q = p;
    while q > 0 && t((q - 1) as usize) {
        c ^= 1;
        q -= 1;
    }
    c
}

/// `esc_back` of a run that starts no earlier than byte `lo`.
#[inline(always)]
fn esc_from(p: isize, lo: isize, t: impl Fn(usize) -> bool) -> u64 {
    let mut c = 0;
    let mut q = p;
    while q > lo.max(0) && t((q - 1) as usize) {
        c ^= 1;
        q -= 1;
    }
    c
}

/// The bits of the window from `p` at byte `lo` and after.
#[inline(always)]
fn from_bit(p: isize, lo: isize) -> u64 {
    if lo <= p { !0 } else if lo - p >= 64 { 0 } else { !0u64 << (lo - p) }
}

/// A fill's carry at the window from `p`, from the bytes: whether it holds at the byte before, the nearest byte back
/// its start `s` or its end `e` holds at deciding.
#[inline(always)]
fn fill_back(p: isize, s: impl Fn(usize) -> bool, e: impl Fn(usize) -> bool) -> u64 {
    let mut q = p;
    while q > 0 {
        let j = (q - 1) as usize;
        if s(j) {
            return 1;
        }
        if e(j) {
            return 0;
        }
        q -= 1;
    }
    0
}

/// Byte `c`'s bits in the 64 bytes from `p`: none past the file or before it.
#[inline(always)]
unsafe fn wbyte(src: &[u8], n: usize, p: isize, c: u8) -> u64 {
    let Some((v, q)) = (unsafe { window(src, n, p) }) else { return 0 };
    held(s::eq_mask(v, c), q, n, p)
}

/// The last byte of `a..=b` the windows `w` give, from `b`'s word back.
#[inline(always)]
fn wlast_in(a: usize, b: usize, mut w: impl FnMut(isize, &mut u64) -> u64) -> Option<usize> {
    if a > b {
        return None;
    }
    let mut q = b / 64;
    loop {
        let lo = if q == a / 64 { !0u64 << (a % 64) } else { !0 };
        let hi = if q == b / 64 { !0u64 >> (63 - b % 64) } else { !0 };
        let m = w((q * 64) as isize, &mut 0) & lo & hi;
        if m != 0 {
            return Some(q * 64 + 63 - m.leading_zeros() as usize);
        }
        if q == a / 64 {
            return None;
        }
        q -= 1;
    }
}

/// `v`'s bits at bytes `a..=b` made those of the windows `w` gives (`put`'s).
#[inline(always)]
fn wput(v: &mut [u64], a: usize, b: usize, mut w: impl FnMut(isize, &mut u64) -> u64) {
    let mut i = a;
    let mut esc = 0;
    while i <= b {
        let (q, r) = (i / 64, i % 64);
        let k = (b + 1 - i).min(64 - r);
        let m = (if k == 64 { !0 } else { (1u64 << k) - 1 }) << r;
        v[q] = v[q] & !m | w((q * 64) as isize, &mut esc) & m;
        i += k;
    }
}

/// One window's escaped bytes (`escaped`), each after an odd run of `bs`'s; `carry`, the run's escape over the window
/// before.
#[inline(always)]
fn escaped_step(bs: u64, carry: &mut u64) -> u64 {
    const ODD: u64 = 0xaaaa_aaaa_aaaa_aaaa;
    if bs == 0 {
        return std::mem::take(carry);
    }
    let potential = bs & !*carry;
    let series = ((potential << 1) | ODD).wrapping_sub(potential);
    let escape_and_terminal = series ^ ODD;
    let escaped = escape_and_terminal ^ (bs | *carry);
    *carry = (escape_and_terminal & bs) >> 63;
    escaped
}

/// A region's end and its tests in one window loop from its opener `a` (`fuse_read`): the first byte after `a` the end
/// holds, and whether each test holds in `a..=` that byte. `w` gives a window's end and tests, its escapes found once.
#[inline(always)]
fn wfirst_any<const K: usize>(n: usize, a: usize, mut w: impl FnMut(isize, &mut u64) -> (u64, [u64; K])) -> Option<(usize, [bool; K])> {
    let mut esc = 0u64;
    let mut any = [0u64; K];
    let mut p = a as isize;
    // The opener's byte is no end.
    let mut opener = 1u64;
    while p < n as isize {
        let (end, t) = w(p, &mut esc);
        let end = end & !opener;
        opener = 0;
        if end != 0 {
            let k = end.trailing_zeros();
            let keep = u64::MAX >> (63 - k);
            for i in 0..K {
                any[i] |= t[i] & keep;
            }
            return Some((p as usize + k as usize, any.map(|x| x != 0)));
        }
        for i in 0..K {
            any[i] |= t[i];
        }
        p += 64;
    }
    None
}

/// The first byte at or after `from` a mask holds, its windows `w` from `from` on.
#[inline(always)]
fn wfirst_bit(n: usize, from: usize, mut w: impl FnMut(isize, &mut u64) -> u64) -> Option<usize> {
    let mut esc = 0u64;
    let mut p = from as isize;
    while p < n as isize {
        let bits = w(p, &mut esc);
        if bits != 0 {
            return Some(p as usize + bits.trailing_zeros() as usize);
        }
        p += 64;
    }
    None
}

/// The bits of the window from `p` in bytes `..=b`: a range test's (`wfirst_in`, `wcount_in`), whose windows start
/// at its first byte.
#[inline(always)]
fn upto(p: isize, b: usize) -> u64 {
    let left = b as isize - p + 1;
    if left >= 64 { !0 } else if left <= 0 { 0 } else { (1u64 << left) - 1 }
}

/// The first byte in `a..=b` a mask holds, its windows `w` from `a` on.
#[inline(always)]
fn wfirst_in(n: usize, a: usize, b: usize, mut w: impl FnMut(isize, &mut u64) -> u64) -> Option<usize> {
    let mut esc = 0u64;
    let mut p = a as isize;
    while p <= b as isize && p < n as isize {
        let mut bits = w(p, &mut esc);
        let left = b as isize - p + 1;
        if left < 64 {
            bits &= (1u64 << left) - 1;
        }
        if bits != 0 {
            return Some(p as usize + bits.trailing_zeros() as usize);
        }
        p += 64;
    }
    None
}

/// A mask's bits in bytes `a..=b`, its windows `w` from `a` on.
#[inline(always)]
fn wcount_in(n: usize, a: usize, b: usize, mut w: impl FnMut(isize, &mut u64) -> u64) -> u32 {
    let mut esc = 0u64;
    let mut c = 0;
    let mut p = a as isize;
    while p <= b as isize && p < n as isize {
        let mut bits = w(p, &mut esc);
        let left = b as isize - p + 1;
        if left < 64 {
            bits &= (1u64 << left) - 1;
        }
        c += bits.count_ones();
        p += 64;
    }
    c
}

/// Whether each of `K` masks has a bit in bytes `a..=b`, their windows `w` from `a` on: one loop, so each window's
/// bytes are loaded and classified once for them all (each mask its own escape carry).
#[inline(always)]
fn wany_in<const K: usize>(n: usize, a: usize, b: usize, mut w: impl FnMut(isize, &mut [u64; K]) -> [u64; K]) -> [bool; K] {
    let mut esc = [0u64; K];
    let mut any = [0u64; K];
    let mut p = a as isize;
    while p <= b as isize && p < n as isize {
        let bits = w(p, &mut esc);
        let left = b as isize - p + 1;
        let keep = if left < 64 { (1u64 << left) - 1 } else { !0 };
        for k in 0..K {
            any[k] |= bits[k] & keep;
        }
        p += 64;
    }
    any.map(|x| x != 0)
}

/// The first byte at or after `from`, before `to`, that a mask does not hold, its windows `w` from `from` on.
#[inline(always)]
fn wfirst_clear(n: usize, from: usize, to: usize, mut w: impl FnMut(isize, &mut u64) -> u64) -> Option<usize> {
    let mut esc = 0u64;
    let mut p = from as isize;
    while p < to as isize {
        let mut bits = !w(p, &mut esc);
        let left = to as isize - p;
        if left < 64 {
            bits &= (1u64 << left) - 1;
        }
        if bits != 0 {
            let i = p as usize + bits.trailing_zeros() as usize;
            return (i < n).then_some(i);
        }
        p += 64;
    }
    None
}

/// The last byte of a mask's run after byte `s` (`s` where none), its windows `w` from `s + 1` on.
#[inline(always)]
fn wrun_last(n: usize, s: usize, mut w: impl FnMut(isize, &mut u64) -> u64) -> usize {
    let _ = n;
    let mut esc = 0u64;
    let mut p = s as isize + 1;
    loop {
        let ones = (!w(p, &mut esc)).trailing_zeros() as isize;
        if ones < 64 {
            return (p + ones - 1) as usize;
        }
        p += 64;
    }
}

/// `wfirst_bit` from block `base`'s word `k` where `from` is in the block, then windows past it.
#[inline(always)]
fn bfirst_bit(n: usize, base: usize, k: u64, from: usize, w: impl FnMut(isize, &mut u64) -> u64) -> Option<usize> {
    let q = from.wrapping_sub(base);
    if q >= 64 {
        return wfirst_bit(n, from, w);
    }
    let bits = k & (!0u64 << q);
    if bits != 0 {
        return Some(base + bits.trailing_zeros() as usize);
    }
    wfirst_bit(n, base + 64, w)
}

/// Block `base`'s word `k` cut to bytes `a..=b` (`a` in the block), and whether `b` is past the block.
#[inline(always)]
fn in_block(base: usize, k: u64, a: usize, b: usize) -> (u64, bool) {
    let mut bits = k & (!0u64 << (a - base));
    let past = b >= base + 64;
    if !past {
        bits &= !0u64 >> (63 - (b - base));
    }
    (bits, past)
}

/// `wfirst_in` from block `base`'s word `k` where `a` is in the block, then windows past it.
#[inline(always)]
fn bfirst_in(n: usize, base: usize, k: u64, a: usize, b: usize, w: impl FnMut(isize, &mut u64) -> u64) -> Option<usize> {
    if a > b {
        return None;
    }
    if a.wrapping_sub(base) >= 64 {
        return wfirst_in(n, a, b, w);
    }
    let (bits, past) = in_block(base, k, a, b);
    if bits != 0 {
        return Some(base + bits.trailing_zeros() as usize);
    }
    if past { wfirst_in(n, base + 64, b, w) } else { None }
}

/// `wcount_in` from block `base`'s word `k` where `a` is in the block, then windows past it.
#[inline(always)]
fn bcount_in(n: usize, base: usize, k: u64, a: usize, b: usize, w: impl FnMut(isize, &mut u64) -> u64) -> u32 {
    if a > b {
        return 0;
    }
    if a.wrapping_sub(base) >= 64 {
        return wcount_in(n, a, b, w);
    }
    let (bits, past) = in_block(base, k, a, b);
    bits.count_ones() + if past { wcount_in(n, base + 64, b, w) } else { 0 }
}

/// `wfirst_clear` from block `base`'s word `k` where `from` is in the block, then windows past it.
#[inline(always)]
fn bfirst_clear(n: usize, base: usize, k: u64, from: usize, to: usize, w: impl FnMut(isize, &mut u64) -> u64) -> Option<usize> {
    if from >= to {
        return None;
    }
    if from.wrapping_sub(base) >= 64 {
        return wfirst_clear(n, from, to, w);
    }
    let (bits, past) = in_block(base, !k, from, to - 1);
    if bits != 0 {
        let i = base + bits.trailing_zeros() as usize;
        return (i < n).then_some(i);
    }
    if past { wfirst_clear(n, base + 64, to, w) } else { None }
}

/// `wrun_last` from block `base`'s word `k` where the run's first byte is in the block (the lean lexer's
/// `micro::run`), then windows past it (its `word_end`).
#[inline(always)]
fn brun_last(n: usize, base: usize, k: u64, s: usize, w: impl FnMut(isize, &mut u64) -> u64) -> usize {
    let q = (s + 1).wrapping_sub(base);
    if q >= 64 {
        return wrun_last(n, s, w);
    }
    let ones = (!(k >> q)).trailing_zeros() as usize;
    if q + ones < 64 {
        return s + ones;
    }
    wrun_last(n, base + 63, w)
}

/// `wrun_once_last` from block `base`'s words `kr` and `kd`, then windows past it.
#[inline(always)]
fn brun_once_last(n: usize, base: usize, kr: u64, kd: u64, s: usize, r: impl FnMut(isize, &mut u64) -> u64, mut d: impl FnMut(isize, &mut u64) -> u64) -> usize {
    let l = brun_last(n, base, kr, s, r);
    match bfirst_in(n, base, kd, s + 1, l, &mut d).and_then(|p| bfirst_in(n, base, kd, p + 1, l, &mut d)) {
        Some(q) => q - 1,
        None => l,
    }
}

/// The last byte of the run of `r` after byte `s` with one bit of `d` in it at most: the second ends the run before
/// it, as a lexer reads a number's one `.` and stops at the next.
#[inline(always)]
fn wrun_once_last(n: usize, s: usize, r: impl FnMut(isize, &mut u64) -> u64, mut d: impl FnMut(isize, &mut u64) -> u64) -> usize {
    let l = wrun_last(n, s, r);
    match wfirst_in(n, s + 1, l, &mut d).and_then(|p| wfirst_in(n, p + 1, l, &mut d)) {
        Some(q) => q - 1,
        None => l,
    }
}

"#;

/// A word's first 8 bytes as one word of memory, little-endian, zeros past its end.
fn key8(w: &[u8]) -> u64 {
    w.iter().take(8).enumerate().fold(0, |k, (i, &c)| k | u64::from(c) << (8 * i))
}

/// `word_at`'s load of 8 bytes where the source is not padded: zeros past the file, as the lean Kotlin lexer's `load8`.
const LOAD_AT_END: &str = "/// The 8 bytes at `i`, little-endian, zeros past the file.\n#[inline(always)]\nfn load8(src: &[u8], i: usize) -> u64 {\n    match src.get(i..i + 8) {\n        Some(b) => u64::from_le_bytes(b.try_into().expect(\"8 bytes\")),\n        None => load8_at_end(src, i),\n    }\n}\n\n/// `load8` within 8 bytes of the file's end.\n#[cold]\n#[inline(never)]\nfn load8_at_end(src: &[u8], i: usize) -> u64 {\n    let rest = src.get(i..).unwrap_or(&[]);\n    let have = rest.len().min(8);\n    let mut w = [0u8; 8];\n    w[..have].copy_from_slice(&rest[..have]);\n    u64::from_le_bytes(w)\n}\n\n";

/// A token's word from one load (decision 0623 step 4, the lean lexer's `word_code`): its first 8 bytes masked to its
/// length, and the length, hashed by one multiply into a table where each of the grammar's words (`words`, code
/// `i + 1`) has a slot of its own; a word of 9 to 16 bytes compares its rest as a second word of memory in the same
/// test, as lean's `ninth`. Where the source is not padded (`padded` false), a load near the file's end reads zeros
/// past it, as the lean Kotlin lexer's `load8`. `None` where a word is longer, or no multiplier the search tries gives each its own slot.
pub(super) fn word_table(words: &[Vec<u8>], padded: bool) -> Option<String> {
    if words.is_empty() || words.iter().any(|w| w.len() > 16) {
        return None;
    }
    let keys: Vec<u64> = words.iter().map(|w| key8(w) ^ w.len() as u64).collect();
    let least = usize::BITS - words.len().max(1).leading_zeros();
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    for bits in least + 1..=least + 4 {
        for _ in 0..100_000 {
            // splitmix64's steps: a fixed sequence, so the table is the same at every build.
            x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            let mul = (z ^ (z >> 31)) | 1;
            let mut slots = vec![usize::MAX; 1 << bits];
            let ok = keys.iter().enumerate().all(|(i, &k)| {
                let slot = (k.wrapping_mul(mul) >> (64 - bits)) as usize;
                let free = slots[slot] == usize::MAX;
                slots[slot] = i;
                free
            });
            if !ok {
                continue;
            }
            let list: Vec<String> = words.iter().map(|w| byte_str(w)).collect();
            let mut table = String::new();
            for chunk in list.chunks(12) {
                let _ = writeln!(table, "    {},", chunk.join(", "));
            }
            return Some(format!(
                "/// The words the grammar reads, a word's code its place plus one, as the lean lexer's `WORD_LIST`.\nconst WORD_LIST: [&[u8]; {}] = [\n{table}];\n\n/// Up to 8 bytes as one word of memory: little-endian, zeros past them.\nconst fn word_key(s: &[u8]) -> u64 {{\n    let (mut v, mut i) = (0u64, 0);\n    while i < s.len() {{\n        v |= (s[i] as u64) << (8 * i);\n        i += 1;\n    }}\n    v\n}}\n\n/// A word's slot in `WORDS`: its first 8 bytes and its length, hashed by a multiplier under which the {} words take\n/// {} slots, as the lean lexer's `word_slot`.\nconst fn word_slot(k: u64, len: usize) -> usize {{\n    ((k ^ len as u64).wrapping_mul({mul:#018x}) >> {}) as usize\n}}\n\n/// Each slot's word (`word_at`): its first 8 bytes, the next 8, its length, its code (0: none), as the lean lexer's\n/// `WORDS`.\nstatic WORDS: [(u64, u64, u32, u32); {}] = {{\n    let mut t = [(0, 0, 0, 0); {}];\n    let mut i = 0;\n    while i < WORD_LIST.len() {{\n        let w = WORD_LIST[i];\n        let (head, tail) = w.split_at(if w.len() > 8 {{ 8 }} else {{ w.len() }});\n        let slot = word_slot(word_key(head), w.len());\n        assert!(t[slot].3 == 0, \"two words in a slot\");\n        t[slot] = (word_key(head), word_key(tail), w.len() as u32, i as u32 + 1);\n        i += 1;\n    }}\n    t\n}};\n\n/// A token's word: the code of the word its `len` bytes from `s` are, 0 for none, as the lean lexer's `word_code`. Its\n/// first 8 bytes, masked to its length, and the length, hashed by one multiply into `WORDS`; its bytes 8 to 16 masked\n/// the same, compared with the first and the length in one test (lean's `ninth`), without a branch.\n#[inline(always)]\nfn word_at(src: &[u8], s: usize, len: usize) -> u32 {{\n    let raw = {l0};\n    let rest = {l8};\n    let k = raw & (u64::MAX >> (64 - 8 * len.clamp(1, 8)));\n    let t = rest & u64::MAX.checked_shr(128 - 8 * len.clamp(8, 16) as u32).unwrap_or(0);\n    let (key, tail, l, code) = WORDS[word_slot(k, len)];\n    code & 0u32.wrapping_sub(u32::from((key == k) & (tail == t) & (l as usize == len)))\n}}\n\n{loads}",
                words.len(),
                words.len(),
                words.len(),
                64 - bits,
                1usize << bits,
                1usize << bits,
                l0 = if padded { "u64::from_le_bytes(src[s..s + 8].try_into().expect(\"the padded source\"))" } else { "load8(src, s)" },
                l8 = if padded { "u64::from_le_bytes(src[s + 8..s + 16].try_into().expect(\"the padded source\"))" } else { "load8(src, s + 8)" },
                loads = if padded { "" } else { LOAD_AT_END },
            ));
        }
    }
    None
}

/// The bytes of a class of byte ranges.
fn class_set(ranges: &[(u32, u32)]) -> [bool; 256] {
    std::array::from_fn(|b| ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&(b as u32))))
}

/// Class `set`'s bits in the first of `packs` (nibble tables' pairs, eight bits each) with room: the bits already
/// there whose bytes are all `set`'s, and new ones for the bytes they leave, each a set of high nibbles times a set of
/// low ones widened over `set` (a bit may hold bytes another bit holds). Its pair and bits; none past eight bits.
fn cover(packs: &mut Vec<Vec<(u16, u16)>>, set: &[bool; 256]) -> Option<(usize, u8)> {
    let at = |h: usize, l: usize| set[h << 4 | l];
    let inside = |r: &(u16, u16)| (0..16).all(|h| r.0 >> h & 1 == 0 || (0..16).all(|l| r.1 >> l & 1 == 0 || at(h, l)));
    let holds = |r: &(u16, u16), b: usize| r.0 >> (b >> 4) & 1 == 1 && r.1 >> (b & 15) & 1 == 1;
    // The new bits for the bytes `left`: a high nibble at a time, the high nibbles with the same low ones one bit, each
    // widened: every high nibble whose bytes at its low ones are all the class's, then every low one at all of them.
    let rects_of = |left: &[bool; 256]| {
        let mut rects: Vec<(u16, u16)> = Vec::new();
        for h in 0..16 {
            let lows = (0..16).filter(|&l| left[h << 4 | l]).fold(0u16, |m, l| m | 1 << l);
            if lows == 0 {
                continue;
            }
            let hs = (0..16).filter(|&g| (0..16).all(|l| lows >> l & 1 == 0 || at(g, l))).fold(0u16, |m, g| m | 1 << g);
            let ls = (0..16).filter(|&l| (0..16).all(|g| hs >> g & 1 == 0 || at(g, l))).fold(0u16, |m, l| m | 1 << l);
            if !rects.contains(&(hs, ls)) {
                rects.push((hs, ls));
            }
        }
        // None whose bytes `left` holds the others hold all of.
        let mut i = 0;
        while i < rects.len() {
            let r = rects[i];
            if (0..256).all(|b| !holds(&r, b) || !left[b] || rects.iter().enumerate().any(|(j, o)| j != i && holds(o, b))) {
                rects.remove(i);
            } else {
                i += 1;
            }
        }
        rects
    };
    let plan = |p: &Vec<(u16, u16)>| {
        let reuse: Vec<usize> = (0..p.len()).filter(|&k| inside(&p[k])).collect();
        let left: [bool; 256] = std::array::from_fn(|b| set[b] && !reuse.iter().any(|&k| holds(&p[k], b)));
        (reuse, rects_of(&left))
    };
    let p = match packs.iter().position(|p| p.len() + plan(p).1.len() <= 8) {
        Some(p) => p,
        None if rects_of(set).len() <= 8 => {
            packs.push(Vec::new());
            packs.len() - 1
        }
        None => return None,
    };
    let (reuse, new) = plan(&packs[p]);
    let mut bits = reuse.iter().fold(0u8, |m, &k| m | 1 << k);
    for r in new {
        bits |= 1 << packs[p].len();
        packs[p].push(r);
    }
    Some((p, bits))
}

/// Whether `e` reads one of the terms `full` (where a name must be expanded to see the term's `prev`).
fn reads_terms(me: &Bytes, e: &Expr, full: &[Expr], depth: usize) -> bool {
    if depth > 48 {
        return true;
    }
    if full.iter().any(|t| t == e) {
        return true;
    }
    match e {
        Expr::Name(n) if !me.classes.contains_key(n.as_str()) => me.locals.get(n).or_else(|| me.defs.get(n)).is_some_and(|d| reads_terms(me, d, full, depth + 1)),
        Expr::Not(a) => reads_terms(me, a, full, depth + 1),
        Expr::And(a, b) | Expr::Or(a, b) | Expr::Xor(a, b) => reads_terms(me, a, full, depth + 1) || reads_terms(me, b, full, depth + 1),
        Expr::Call(_, args) => args.iter().any(|a| matches!(a, Arg::E(x) if reads_terms(me, x, full, depth + 1))),
        _ => false,
    }
}

/// Whether `e` reads a fill (a scan back from its byte).
fn reads_fill(e: &Expr) -> bool {
    match e {
        Expr::Call(f, _) if f == "fill" || f == "escaped" => true,
        Expr::Call(_, args) => args.iter().any(|a| matches!(a, Arg::E(x) if reads_fill(x))),
        Expr::Not(a) => reads_fill(a),
        Expr::And(a, b) | Expr::Or(a, b) | Expr::Xor(a, b) => reads_fill(a) || reads_fill(b),
        _ => false,
    }
}

/// `e` with its constants folded (`simplify`) at every node, a call's arguments too; of an `&`, the side that reads no
/// fill first, so a fill is read where the bytes' tests hold, as the lean lexer tests a byte before it reads on.
fn fold(e: Expr) -> Expr {
    let f = |a: Box<Expr>| Box::new(fold(*a));
    super::simplify(match e {
        Expr::Not(a) => Expr::Not(f(a)),
        Expr::And(a, b) => {
            let (a, b) = (f(a), f(b));
            if reads_fill(&a) && !reads_fill(&b) { Expr::And(b, a) } else { Expr::And(a, b) }
        }
        Expr::Or(a, b) => Expr::Or(f(a), f(b)),
        Expr::Xor(a, b) => Expr::Xor(f(a), f(b)),
        Expr::Call(g, args) => Expr::Call(
            g,
            args.into_iter()
                .map(|a| match a {
                    Arg::E(x) => Arg::E(fold(x)),
                    a => a,
                })
                .collect(),
        ),
        e => e,
    })
}
