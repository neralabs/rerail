//! Tier 2 as one loop, written by the Bitlex compiler from the grammar's tree rules, in simdjson's stage 2's shape
//! (the user, 2026-11-21: "single loop"), never from tier 2's state machine or any lowered form:
//!
//! - **One loop, one dispatch.** A `nested` rule is a level: its call opens its node where the caller is (the node's
//!   key, its field and supertypes, is the caller's), pushes the place it returns to and goes to the rule's first
//!   place; its end closes the node and goes to the place it pops, as simdjson's `scope_end`. The places are the
//!   loop's arms (`'run: loop { match st { … } }`): a nested rule's first one, and each one after a level's call
//!   (simdjson's `object_continue`, `array_continue`). Every other rule is written where it is used.
//! - **Decisions at the token**, as simdjson's `switch` on the byte: `[A]`, `A*`, `A+`, `A | B` match the next
//!   record's first byte. A token kind is its first bytes; a word of a one-byte token its byte; a word of a longer
//!   one its kind's bytes, then its word.
//! - **A token's end** from tier 1's masks (`bitlex_lex_rust::stage1_ends`): a run's, a pair's, a region's closer.
//!   A region read in parts (a string) is read at its token, its pieces leaves, as simdjson's `parse_string`.
//! - **The tree** through format 16's writer (`rerail_tape_format::f16::Writer`), each node's number a literal of
//!   the code table the generator makes (`ENTRIES`): each node's kind, field and supertypes, as it writes them. No
//!   node changes once written, so each is committed as it is made, as the JIT's direct run's.
//! - **Checks**: a `check` of a kind is a function of the token's bytes, run where a token of the kind is a leaf;
//!   a `keywords … required` kind's piece must be one of its words.

use super::{First, G, Input, Test, ident};
use ::bitlex::syntax::{Expr, Gram, PartLen, Stmt, TokenRule};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Write;

/// Where the code goes on: a place of the loop, or code of its own, written where it is used (once).
#[derive(Clone)]
enum K {
    Go(usize),
    Code(String),
}

/// What the decision an arm is reached by has already found of the token it starts at: nothing (`No`: the arm's
/// code tests it), its first byte (`Byte`), or its word, with `s` its start and `end` its end bound (`Word`). An arm
/// tests nothing its decision found, as `walk`'s `primitive` reads the value its `switch` took.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum T {
    No,
    Byte,
    Word,
}

/// A decision's arm: its first tokens, and its code given what the decision found.
type Arm<'s> = (Vec<First>, Box<dyn Fn(T) -> Result<String, String> + 's>);

/// The key a node is written with (its code table entry): its field, and the supertypes it is reached through.
#[derive(Clone, Default)]
struct Key {
    field: u16,
    classes: Vec<u16>,
}

/// How a record kind's token ends, as tier 2 finds it from tier 1's masks.
#[derive(Clone)]
enum End {
    /// One byte.
    One,
    /// A run of a mask (`run_K`).
    Run(String),
    /// A second byte where a mask holds (`pair_K`).
    Pair(String),
    /// The first bit of a mask (`last_K`).
    Last(String),
    /// A region read in parts: its reader.
    Region(usize),
}

struct Lp<'g, 'a> {
    g: &'g G<'a>,
    /// The loop's places' code, by number.
    places: RefCell<Vec<String>>,
    /// The code table: each entry's kind, field and supertypes, by number.
    entries: RefCell<Vec<(u16, u16, Vec<u16>)>>,
    /// Each nested rule's first place.
    levels: RefCell<BTreeMap<String, usize>>,
    /// Each token kind's (its name's) record kind and its end.
    ends: BTreeMap<String, (u8, End)>,
    readers: Vec<crate::bitlex_lex_rust::Reader>,
    /// The kinds whose leaves are checked (`check K = …`).
    checked: Vec<String>,
    /// Each byte's token row (its "record kind" here): the row whose start may hold at it.
    row_of: [Option<u8>; 256],
    /// The root's end (its close and the stream's end), and the place it is once one is made of it.
    root_end: String,
    root_place: std::cell::Cell<Option<usize>>,
}

/// A set of bytes.
type Set = [bool; 256];

/// A mask's bytes, bounded by its classes alone: those it holds at wherever a byte is one of them, and those it
/// may hold at. A positional part (`prev`, `next`, a region's fill, a carry) may hold anywhere and surely nowhere.
fn bounds(stmts: &[Stmt], e: &Expr, depth: u32) -> (Set, Set) {
    let unknown = ([false; 256], [true; 256]);
    if depth > 64 {
        return unknown;
    }
    match e {
        Expr::Empty => ([false; 256], [false; 256]),
        Expr::Name(n) => {
            for st in stmts {
                match st {
                    Stmt::Class(c, items) if c == n => {
                        let mut set = [false; 256];
                        for &(lo, hi) in items {
                            for b in lo..=hi.min(255) {
                                set[b as usize] = true;
                            }
                        }
                        return (set, set);
                    }
                    Stmt::Mask(m, d) if m == n => return bounds(stmts, d, depth + 1),
                    _ => {}
                }
            }
            unknown
        }
        Expr::Not(a) => {
            let (u, o) = bounds(stmts, a, depth + 1);
            (std::array::from_fn(|b| !o[b]), std::array::from_fn(|b| !u[b]))
        }
        Expr::And(a, b) | Expr::Or(a, b) => {
            let ((u1, o1), (u2, o2)) = (bounds(stmts, a, depth + 1), bounds(stmts, b, depth + 1));
            if matches!(e, Expr::And(..)) {
                (std::array::from_fn(|k| u1[k] && u2[k]), std::array::from_fn(|k| o1[k] && o2[k]))
            } else {
                (std::array::from_fn(|k| u1[k] || u2[k]), std::array::from_fn(|k| o1[k] || o2[k]))
            }
        }
        _ => unknown,
    }
}

/// A byte set as a match pattern: its runs as ranges.
fn pattern(bytes: &[u8]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut k = 0;
    while k < bytes.len() {
        let mut j = k;
        while j + 1 < bytes.len() && bytes[j + 1] == bytes[j] + 1 {
            j += 1;
        }
        out.push(if j == k { format!("{:#04x}", bytes[k]) } else { format!("{:#04x}..={:#04x}", bytes[k], bytes[j]) });
        k = j + 1;
    }
    out.join(" | ")
}

/// A byte string as a Rust byte string literal's text.
fn lit(b: &[u8]) -> String {
    b.iter().map(|&c| if c.is_ascii_alphanumeric() || c == b' ' { (c as char).to_string() } else { format!("\\x{c:02x}") }).collect()
}

impl Lp<'_, '_> {
    fn err<X>(&self, what: impl std::fmt::Display) -> Result<X, String> {
        Err(format!("tier 2 as one loop: {what}"))
    }

    /// A new place, its code `code`.
    fn place(&self, code: String) -> usize {
        let mut p = self.places.borrow_mut();
        p.push(code);
        p.len() - 1
    }

    fn go(l: usize) -> String {
        format!("st = {l};\ncontinue 'run;\n")
    }

    fn jump(k: &K) -> String {
        match k {
            K::Go(l) => Self::go(*l),
            K::Code(c) => c.clone(),
        }
    }

    /// `k` as a place: code used more than once is written once. Code that only goes on to a place is that place.
    fn to_place(&self, k: K) -> usize {
        match k {
            K::Go(l) => l,
            K::Code(c) => {
                if let Some(l) = c.strip_prefix("st = ").and_then(|r| r.strip_suffix(";\ncontinue 'run;\n")).and_then(|l| l.parse().ok()) {
                    return l;
                }
                let root = c == self.root_end;
                let l = self.place(c);
                if root {
                    self.root_place.set(Some(l));
                }
                l
            }
        }
    }

    /// A one-byte token's byte: a word of a token of one byte, or a kind all of whose tokens start at one byte
    /// and are one byte long.
    fn one_byte(&self, g: &Gram) -> Option<u8> {
        let g = match g {
            Gram::As(_, a) => a,
            g => g,
        };
        let (t, _) = self.g.token_of(g)?;
        match self.admits(&(t, None)).ok()? {
            Admit::Bytes(b) if b.len() == 1 => {
                let row = self.row_of[usize::from(b[0])]?;
                self.ends.values().any(|(r, e)| *r == row && matches!(e, End::One)).then_some(b[0])
            }
            _ => None,
        }
    }

    /// A level's brackets: its body an opener of one byte, what may be empty, and a closer of one byte.
    fn brackets(&self, rule: &str) -> Result<Option<(u8, u8)>, String> {
        let Some(Stmt::Node { body: Gram::Seq(gs), .. }) = self.g.rules.get(rule) else { return Ok(None) };
        let [first, middle @ .., last] = gs.as_slice() else { return Ok(None) };
        for g in middle {
            if !self.g.first(g)?.1 {
                return Ok(None);
            }
        }
        Ok(self.one_byte(first).zip(self.one_byte(last)))
    }

    /// The number of a node of `kind` written with `key`.
    fn number(&self, kind: &str, key: &Key) -> Result<usize, String> {
        let id = (self.g.x.kind)(kind).ok_or_else(|| format!("no kind `{kind}`"))?;
        let e = (id, key.field, key.classes.clone());
        let mut es = self.entries.borrow_mut();
        Ok(match es.iter().position(|x| *x == e) {
            Some(k) => k,
            None => {
                es.push(e);
                es.len() - 1
            }
        })
    }

    /// The first bytes of a token row.
    fn bytes_of(&self, rk: u8) -> Vec<u8> {
        (0..=255u8).filter(|&b| self.row_of[usize::from(b)] == Some(rk)).collect()
    }

    /// A token kind's record kind and end.
    fn token(&self, name: &str) -> Result<(u8, End), String> {
        self.ends.get(name).cloned().ok_or_else(|| format!("tier 2 as one loop: `{name}` is no token"))
    }

    /// A word's token kind: the one whose record kind its first byte is.
    fn word_token(&self, w: u8) -> Result<(String, u8, End, Vec<u8>), String> {
        let bytes = self.g.x.words[usize::from(w) - 1].clone();
        let rk = self.row_of[usize::from(bytes[0])].ok_or_else(|| format!("tier 2 as one loop: no token starts word {w}"))?;
        let (name, (_, end)) = self.ends.iter().find(|(_, (r, _))| *r == rk).ok_or_else(|| format!("tier 2 as one loop: no token of word {w}"))?;
        Ok((name.clone(), rk, end.clone(), bytes))
    }

    /// The words of a record kind's keywords that name another kind (its hard words): its tokens with these words
    /// are not of its kind.
    fn hard_words(&self, name: &str) -> Vec<u8> {
        self.g.x.keywords.iter().filter(|r| r.0 == name).flat_map(|r| r.1.iter()).filter(|(_, k)| k.as_deref().is_some_and(|k| k != name)).filter_map(|(w, _)| self.g.word_of(&String::from_utf8_lossy(w))).collect()
    }

    /// The end (one past the last byte) of the token at `s`, by its kind's rule.
    fn end_code(&self, end: &End) -> Result<String, String> {
        Ok(match end {
            End::One => "s + 1".to_owned(),
            End::Run(f) => format!("run_last(&m.{f}, s) + 1"),
            End::Pair(f) => format!("s + 1 + usize::from(bit(&m.{f}, s))"),
            End::Last(f) => format!("first_bit(&m.{f}, s).unwrap_or(s) + 1"),
            End::Region(r) => format!("first_bit(&m.{}, s + 1).map_or(src.len(), |c| c + 1)", self.readers[*r].close),
        })
    }

    /// What a test admits: the byte arms it is (one-byte words, and kinds no word splits), or its kind's arm
    /// with the word it needs (`Some`) or the words it is not (`None`).
    fn admits(&self, t: &First) -> Result<Admit, String> {
        if t.1.is_some() {
            return self.err("a flag's test is not written yet");
        }
        Ok(match &t.0 {
            Test::Mark(_) => return self.err("marks are not written yet"),
            Test::Word(w) => {
                let (name, rk, end, bytes) = self.word_token(*w)?;
                if bytes.len() == 1 && matches!(end, End::One) {
                    Admit::Bytes(bytes)
                } else {
                    Admit::Word { name, rk, word: *w }
                }
            }
            Test::Kind(k) => {
                let (rk, _) = self.token(k)?;
                if self.hard_words(k).is_empty() {
                    Admit::Bytes(self.bytes_of(rk))
                } else {
                    Admit::Kind { name: k.clone(), rk }
                }
            }
        })
    }

    /// The code of a decision at the next record among `arms` (each its tests and its code): a `match` on its
    /// first byte (`b`); a kind split by its words, its token's word then. None passes: `none`.
    fn decide(&self, arms: &[Arm<'_>], none: &str) -> Result<String, String> {
        // Each arm's code for what it is reached by, made once.
        let made: RefCell<BTreeMap<(usize, T), String>> = RefCell::new(BTreeMap::new());
        let code = |a: usize, t: T| -> Result<String, String> {
            if let Some(c) = made.borrow().get(&(a, t)) {
                return Ok(c.clone());
            }
            let c = (arms[a].1)(t)?;
            made.borrow_mut().insert((a, t), c.clone());
            Ok(c)
        };
        // Each byte's arm; each split kind's words' arms and its own arm.
        let mut by_byte: BTreeMap<u8, usize> = BTreeMap::new();
        let mut split: BTreeMap<u8, (String, Vec<(u8, usize)>, Option<usize>)> = BTreeMap::new();
        for (a, (tests, _)) in arms.iter().enumerate() {
            for t in tests {
                match self.admits(t)? {
                    Admit::Bytes(bs) => {
                        for b in bs {
                            if by_byte.insert(b, a).is_some_and(|o| o != a) {
                                return self.err(format!("byte {b:#04x} starts two arms"));
                            }
                        }
                    }
                    Admit::Word { name, rk, word } => split.entry(rk).or_insert_with(|| (name, Vec::new(), None)).1.push((word, a)),
                    Admit::Kind { name, rk } => {
                        let e = split.entry(rk).or_insert_with(|| (name, Vec::new(), None));
                        if e.2.is_some_and(|o| o != a) {
                            return self.err(format!("kind {rk} starts two arms"));
                        }
                        e.2 = Some(a);
                    }
                }
            }
        }
        // A split kind whose words and own test all go to one arm is that arm's by its bytes alone: no word read.
        let whole: Vec<u8> = split.iter().filter(|(_, (_, words, own))| own.is_some_and(|a| words.iter().all(|w| w.1 == a))).map(|(&rk, _)| rk).collect();
        for rk in whole {
            let (_, _, own) = split.remove(&rk).expect("the kind");
            for b in self.bytes_of(rk) {
                if by_byte.insert(b, own.expect("its arm")).is_some() {
                    return self.err(format!("byte {b:#04x} starts two arms"));
                }
            }
        }
        let mut out = String::from("match peek(src, starts, i) {\n");
        // The byte arms, each arm's bytes one pattern.
        let mut arm_bytes: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
        for (&b, &a) in &by_byte {
            if split.keys().any(|&rk| self.row_of[usize::from(b)] == Some(rk)) {
                return self.err(format!("byte {b:#04x} is a word's and a kind's"));
            }
            arm_bytes.entry(a).or_default().push(b);
        }
        for (a, bs) in &arm_bytes {
            let _ = write!(out, "{} => {{\n{}}}\n", pattern(bs), code(*a, T::Byte)?);
        }
        for (rk, (name, words, own)) in &split {
            let (_, end) = self.token(name)?;
            let mut inner = String::new();
            for (w, a) in words {
                let _ = write!(inner, "{w} => {{\n{}}}\n", code(*a, T::Word)?);
            }
            let rest = match own {
                Some(a) => {
                    // The kind's own arm: none of its hard words.
                    let hard: Vec<String> = self.hard_words(name).iter().filter(|h| !words.iter().any(|(w, _)| w == *h)).map(|h| h.to_string()).collect();
                    if !hard.is_empty() {
                        let _ = write!(inner, "{} => {{\n{none}}}\n", hard.join(" | "));
                    }
                    code(*a, T::Word)?
                }
                None => none.to_owned(),
            };
            let _ = write!(inner, "_ => {{\n{rest}}}\n");
            let _ = write!(
                out,
                "{} => {{\n// `{name}`: its word.\nlet s = starts[i] as usize;\nlet end = {};\nmatch word_{}(&src[s..end]) {{\n{inner}}}\n}}\n",
                pattern(&self.bytes_of(*rk)),
                self.end_code(&end)?,
                ident(name)
            );
        }
        let _ = write!(out, "_ => {{\n{none}}}\n}}\n");
        Ok(out)
    }

    /// The code that takes the token `test` is, a leaf of `kind` unless it is implied (`None`), with `key`; `t`, what
    /// a decision found of it.
    fn take(&self, test: &Test, kind: Option<&str>, key: &Key, t: T) -> Result<String, String> {
        let end = match test {
            Test::Kind(k) => self.token(k)?.1,
            Test::Word(w) => self.word_token(*w)?.2,
            Test::Mark(_) => return self.err("marks are not written yet"),
        };
        let body = |t: T| -> Result<String, String> {
            // A word's decision bound its start and its end.
            let mut body = if t == T::Word { "i += 1;\ne = end;\n".to_owned() } else { format!("let s = starts[i] as usize;\ni += 1;\ne = {};\n", self.end_code(&end)?) };
            if let Some(k) = kind {
                if self.checked.iter().any(|c| c == k) {
                    let _ = writeln!(body, "if !check_{}(&src[s..e]) {{\nreturn Err(s as u32);\n}}", ident(k));
                }
                let _ = writeln!(body, "leaf(d, {}, s as u64, e as u64);\ncommit(d);", self.number(k, key)?);
            }
            Ok(body)
        };
        if t != T::No {
            return body(t);
        }
        self.decide(&[(vec![(test.clone(), None)], Box::new(body))], "return Err(at(src, starts, i));\n")
    }

    /// A region's node (`node K = OPENER (PIECES)* CLOSER`, its opener a token read in parts): read at its token,
    /// each piece a leaf (its kind's words checked where they are required), as simdjson's `parse_string`.
    fn region(&self, kind: &str, body: &Gram, key: &Key, t: T) -> Result<Option<String>, String> {
        let Gram::Seq(gs) = body else { return Ok(None) };
        let [open, Gram::Star(pieces), close] = gs.as_slice() else { return Ok(None) };
        let tok = |g: &Gram| match g {
            Gram::As(k, a) if k == ::bitlex::syntax::IMPLIED => self.g.token_of(a).map(|t| (t, true)),
            g => self.g.token_of(g).map(|t| (t, false)),
        };
        let (Some(((Test::Kind(o), _), o_implied)), Some(((Test::Kind(c), _), c_implied))) = (tok(open), tok(close)) else { return Ok(None) };
        if o != c {
            return Ok(None);
        }
        let Ok((_, End::Region(r))) = self.token(&o) else { return Ok(None) };
        if !o_implied || !c_implied {
            return self.err(format!("`{kind}`: a region's quotes as leaves are not written yet"));
        }
        let rd = &self.readers[r];
        let mut kinds: Vec<&str> = Vec::new();
        match &**pieces {
            Gram::Alt(ps) => {
                for p in ps {
                    let Gram::Sym(n) = p else { return self.err(format!("`{kind}`: a piece that is no kind")) };
                    kinds.push(n);
                }
            }
            Gram::Sym(n) => kinds.push(n),
            _ => return self.err(format!("`{kind}`: its pieces are no kinds")),
        }
        let [(part, part_start, PartLen::Bytes(len), _)] = rd.parts.as_slice() else {
            return self.err(format!("`{kind}`: a region of other than one part of a fixed length is not written yet"));
        };
        if !rd.closes.is_empty() || kinds.iter().any(|k| *k != part && *k != rd.content) {
            return self.err(format!("`{kind}`: its pieces are not its region's"));
        }
        let required: Vec<String> = self.g.x.keywords.iter().filter(|r| r.0 == *part && r.2).flat_map(|r| r.1.iter().map(|(w, _)| format!("b\"{}\"", lit(w)))).collect();
        let check = if required.is_empty() { String::new() } else { format!("if !matches!(&src[p..p + {len}], {}) {{\nreturn Err(p as u32);\n}}\n", required.join(" | ")) };
        let node = self.number(kind, key)?;
        let content = self.number(&rd.content, &Key::default())?;
        let piece = self.number(part, &Key::default())?;
        let test = if t == T::No { format!("match peek(src, starts, i) {{\n{} => {{}}\n_ => return Err(at(src, starts, i)),\n}}\n", pattern(&self.bytes_of(self.token(&o)?.0))) } else { String::new() };
        Ok(Some(format!(
            "{test}// `{kind}`: read at its token, its pieces leaves.\n\
             let s = starts[i] as usize;\ni += 1;\n\
             let c = match first_bit(&m.{close}, s + 1) {{\nSome(c) => c,\nNone => return Err(s as u32),\n}};\n\
             open(d, {node}, s as u64);\ncommit(d);\n\
             let mut p = s + 1;\n\
             loop {{\n\
             let q = match first_bit(&m.{part_start}, p) {{\nSome(q) if q < c => q,\n_ => c,\n}};\n\
             if q > p {{\nleaf(d, {content}, p as u64, q as u64);\ncommit(d);\n}}\n\
             if q == c {{\nbreak;\n}}\n\
             p = q;\n{check}\
             leaf(d, {piece}, p as u64, (p + {len}) as u64);\ncommit(d);\n\
             p += {len};\n\
             }}\n\
             e = c + 1;\nclose(d, e as u64);\n",
            close = rd.close,
        )))
    }

    /// `g` as alternatives where it starts with a choice: each alternative, the key it is written with, and what
    /// follows them in `g` (each alternative then that). None where `g` does not, or an alternative may be empty (its
    /// first tokens would be those after it). A decision among them is the choice's own: no second one.
    fn heads(&self, g: &Gram, key: &Key) -> Result<Option<(Vec<Gram>, Key, Vec<Gram>)>, String> {
        Ok(match g {
            Gram::Alt(gs) => {
                for a in gs {
                    if self.g.first(a)?.1 {
                        return Ok(None);
                    }
                }
                Some((gs.clone(), key.clone(), Vec::new()))
            }
            Gram::Sym(n) => match self.g.rules.get(n.as_str()) {
                Some(Stmt::Choice { nested: false, supertype, body, .. }) => self.heads(body, &self.through(key, supertype.as_deref())?)?,
                _ => None,
            },
            Gram::Field(f, a) => {
                let field = (self.g.x.field)(f).ok_or_else(|| format!("no field `{f}`"))?;
                self.heads(a, &Key { field, ..key.clone() })?
            }
            Gram::Seq(gs) => match gs.split_first() {
                Some((h, rest)) => self.heads(h, key)?.map(|(alts, k, mut more)| {
                    more.extend(rest.iter().cloned());
                    (alts, k, more)
                }),
                None => None,
            },
            _ => None,
        })
    }

    /// The arms of a decision on `g`, each going on to `k` (`walk`): its alternatives where it starts with a choice
    /// (`heads`), else `g` itself.
    fn walk_arms<'s>(&'s self, g: &'s Gram, key: &Key, k: K) -> Result<Vec<Arm<'s>>, String> {
        if let Some((alts, akey, rest)) = self.heads(g, key)? {
            let k = if rest.is_empty() { k } else { K::Go(self.to_place(K::Code(self.walk(&Gram::Seq(rest), &Key::default(), k, T::No)?))) };
            let mut arms: Vec<Arm<'s>> = Vec::new();
            for a in alts {
                let (first, _) = self.g.first(&a)?;
                let (akey, k) = (akey.clone(), k.clone());
                arms.push((first, Box::new(move |t| self.walk(&a, &akey, k.clone(), t))));
            }
            return Ok(arms);
        }
        let key = key.clone();
        Ok(vec![(self.g.first(g)?.0, Box::new(move |t| self.walk(g, &key, k.clone(), t)))])
    }

    /// The arms of a decision on `g` with no call in it (`plain`): its alternatives where it is a choice alone, else
    /// `g` itself.
    fn plain_arms<'s>(&'s self, g: &'s Gram, key: &Key) -> Result<Vec<Arm<'s>>, String> {
        if let Some((alts, akey, rest)) = self.heads(g, key)?
            && rest.is_empty()
        {
            let mut arms: Vec<Arm<'s>> = Vec::new();
            for a in alts {
                let (first, _) = self.g.first(&a)?;
                let akey = akey.clone();
                arms.push((first, Box::new(move |t| self.plain(&a, &akey, t))));
            }
            return Ok(arms);
        }
        let key = key.clone();
        Ok(vec![(self.g.first(g)?.0, Box::new(move |t| self.plain(g, &key, t)))])
    }

    /// The most nodes `g` holds open at once inside its level (a node rule written where it is used, around what
    /// it holds), a level's call not counted (it is its own level's).
    fn open_in(&self, g: &Gram, depth: u32) -> Result<usize, String> {
        if depth > 64 {
            return self.err("rules nested too deep");
        }
        Ok(match g {
            Gram::Sym(n) => match self.g.rules.get(n.as_str()) {
                Some(Stmt::Node { nested: true, .. } | Stmt::Choice { nested: true, .. }) => 0,
                Some(Stmt::Node { body, .. }) => 1 + self.open_in(body, depth + 1)?,
                Some(Stmt::Choice { body, .. }) => self.open_in(body, depth + 1)?,
                _ => 0,
            },
            Gram::Seq(gs) | Gram::Alt(gs) => gs.iter().map(|g| self.open_in(g, depth + 1)).collect::<Result<Vec<_>, _>>()?.into_iter().max().unwrap_or(0),
            Gram::Opt(a) | Gram::Star(a) | Gram::Plus(a) | Gram::Field(_, a) | Gram::As(_, a) => self.open_in(a, depth + 1)?,
            _ => 0,
        })
    }

    /// Whether `g` calls a level (a nested rule), itself or through the rules written where they are used.
    fn calls(&self, g: &Gram) -> bool {
        match g {
            Gram::Sym(n) => match self.g.rules.get(n.as_str()) {
                Some(Stmt::Node { nested: true, .. } | Stmt::Choice { nested: true, .. }) => true,
                Some(Stmt::Node { body, .. } | Stmt::Choice { body, .. }) => self.calls(body),
                _ => false,
            },
            Gram::Seq(gs) | Gram::Alt(gs) => gs.iter().any(|g| self.calls(g)),
            Gram::Opt(a) | Gram::Star(a) | Gram::Plus(a) | Gram::Field(_, a) | Gram::As(_, a) => self.calls(a),
            _ => false,
        }
    }

    /// The code of `g` with no level's call in it, structured, with `key` for the first node it writes; `t`, what a
    /// decision found of its first token.
    fn plain(&self, g: &Gram, key: &Key, t: T) -> Result<String, String> {
        if let Some((test, kind)) = self.g.token_of(g) {
            return self.take(&test, Some(&kind), key, t);
        }
        Ok(match g {
            Gram::As(k, a) => {
                let (test, _) = self.g.token_of(a).ok_or_else(|| "tier 2 as one loop: `as` of no token".to_owned())?;
                self.take(&test, (k != ::bitlex::syntax::IMPLIED).then_some(k.as_str()), key, t)?
            }
            Gram::Sym(n) => match self.g.rules.get(n.as_str()) {
                Some(Stmt::Node { kind, body, .. }) => {
                    if let Some(code) = self.region(kind, body, key, t)? {
                        return Ok(code);
                    }
                    format!("open(d, {}, at(src, starts, i) as u64);\ncommit(d);\n{}close(d, e as u64);\n", self.number(kind, key)?, self.plain(body, &Key::default(), t)?)
                }
                Some(Stmt::Choice { supertype, body, .. }) => self.plain(body, &self.through(key, supertype.as_deref())?, t)?,
                _ => return self.err(format!("`{n}`: no rule or token")),
            },
            Gram::Seq(gs) => {
                let (mut out, none) = (String::new(), Key::default());
                for (k, g) in gs.iter().enumerate() {
                    out.push_str(&self.plain(g, if k == 0 { key } else { &none }, if k == 0 { t } else { T::No })?);
                }
                out
            }
            Gram::Opt(a) => self.decide(&self.plain_arms(a, key)?, "")?,
            Gram::Star(a) => format!("loop {{\n{}}}\n", self.decide(&self.plain_arms(a, key)?, "break;\n")?),
            Gram::Plus(a) => {
                let one = self.plain(a, key, t)?;
                format!("{one}loop {{\n{}}}\n", self.decide(&self.plain_arms(a, key)?, "break;\n")?)
            }
            Gram::Alt(gs) => {
                let mut empty = false;
                for g in gs {
                    empty |= self.g.first(g)?.1;
                }
                let mut arms: Vec<Arm<'_>> = Vec::new();
                for g in gs {
                    let (first, _) = self.g.first(g)?;
                    arms.push((first, Box::new(move |t| self.plain(g, key, t))));
                }
                self.decide(&arms, if empty { "" } else { "return Err(at(src, starts, i));\n" })?
            }
            Gram::Field(f, a) => {
                let field = (self.g.x.field)(f).ok_or_else(|| format!("no field `{f}`"))?;
                self.plain(a, &Key { field, ..key.clone() }, t)?
            }
            other => return self.err(format!("not written yet: {other:?}")),
        })
    }

    /// `key` through a choice of supertype `s`.
    fn through(&self, key: &Key, s: Option<&str>) -> Result<Key, String> {
        let mut k = key.clone();
        if let Some(s) = s {
            k.classes.push((self.g.x.kind)(s).ok_or_else(|| format!("no supertype `{s}`"))?);
        }
        Ok(k)
    }

    /// The code of `g`, then `k`: a level's call ends its place, and the code after it is a place of its own. `t`,
    /// what a decision found of its first token.
    fn walk(&self, g: &Gram, key: &Key, k: K, t: T) -> Result<String, String> {
        if !self.calls(g) {
            return Ok(self.plain(g, key, t)? + &Self::jump(&k));
        }
        Ok(match g {
            Gram::Sym(n) => match self.g.rules.get(n.as_str()) {
                Some(Stmt::Node { kind, nested: true, body, .. }) => {
                    if t == T::No {
                        // The level's first token, tested here: its body reads it as its decision found it.
                        let (first, _) = self.g.first(body)?;
                        let k = k.clone();
                        return self.decide(&[(first, Box::new(move |t| self.walk(g, key, k.clone(), t)))], "return Err(at(src, starts, i));\n");
                    }
                    let first = self.level(n)?;
                    let ret = self.to_place(k);
                    let number = self.number(kind, key)?;
                    let mut out = String::new();
                    if let Some((_, close)) = self.brackets(n)? {
                        // The level the root's value: the file's last record its closer, checked before the walk, as
                        // the lean parser's `last != '}'`.
                        if self.root_place.get() == Some(ret) {
                            let _ = write!(out, "if starts.last().map(|&l| src[l as usize]) != Some({close:#04x}) {{\nreturn Err(at(src, starts, i));\n}}\n");
                        }
                        // Empty: one leaf from its opener (its decision's) to its closer, as the lean parser's `empty!`;
                        // its depth tested as a level's.
                        let _ = write!(
                            out,
                            "if stack.len() == DEPTH {{\nreturn Err(at(src, starts, i));\n}}\nif peek(src, starts, i + 1) == {close:#04x} {{\n// `{kind}`, empty.\nlet s = starts[i] as usize;\ne = starts[i + 1] as usize + 1;\ni += 2;\nleaf(d, {number}, s as u64, e as u64);\ncommit(d);\n{}}}\n",
                            Self::go(ret)
                        );
                    } else {
                        out.push_str("if stack.len() == DEPTH {\nreturn Err(at(src, starts, i));\n}\n");
                    }
                    let _ = write!(out, "open(d, {number}, at(src, starts, i) as u64);\ncommit(d);\nstack.push({ret});\n{}", Self::go(first));
                    out
                }
                Some(Stmt::Node { kind, body, .. }) => {
                    let after = format!("close(d, e as u64);\n{}", Self::jump(&k));
                    format!("open(d, {}, at(src, starts, i) as u64);\ncommit(d);\n{}", self.number(kind, key)?, self.walk(body, &Key::default(), K::Code(after), t)?)
                }
                Some(Stmt::Choice { nested: false, supertype, body, .. }) => self.walk(body, &self.through(key, supertype.as_deref())?, k, t)?,
                _ => return self.err(format!("`{n}`: a nested choice is not written yet")),
            },
            Gram::Seq(gs) => {
                let (mut k, none) = (k, Key::default());
                for (j, g) in gs.iter().enumerate().rev() {
                    k = K::Code(self.walk(g, if j == 0 { key } else { &none }, k, if j == 0 { t } else { T::No })?);
                }
                Self::jump(&k)
            }
            Gram::Opt(a) => {
                let after = self.to_place(k);
                self.decide(&self.walk_arms(a, key, K::Go(after))?, &Self::go(after))?
            }
            Gram::Star(a) => {
                let after = self.to_place(k);
                let head = self.place(String::new());
                let code = self.decide(&self.walk_arms(a, key, K::Go(head))?, &Self::go(after))?;
                self.places.borrow_mut()[head] = code;
                Self::go(head)
            }
            Gram::Plus(a) => self.walk(&Gram::Seq(vec![(**a).clone(), Gram::Star(a.clone())]), key, k, t)?,
            Gram::Alt(gs) => {
                let after = self.to_place(k);
                let mut empty = false;
                for g in gs {
                    empty |= self.g.first(g)?.1;
                }
                let mut arms: Vec<Arm<'_>> = Vec::new();
                for g in gs {
                    let (first, _) = self.g.first(g)?;
                    arms.push((first, Box::new(move |t| self.walk(g, key, K::Go(after), t))));
                }
                self.decide(&arms, &if empty { Self::go(after) } else { "return Err(at(src, starts, i));\n".to_owned() })?
            }
            Gram::Field(f, a) => {
                let field = (self.g.x.field)(f).ok_or_else(|| format!("no field `{f}`"))?;
                self.walk(a, &Key { field, ..key.clone() }, k, t)?
            }
            other => return self.err(format!("not written yet: {other:?}")),
        })
    }

    /// A nested rule's first place: its body after its node's open (its caller's), then its close and the place
    /// its caller pushed.
    fn level(&self, rule: &str) -> Result<usize, String> {
        if let Some(&l) = self.levels.borrow().get(rule) {
            return Ok(l);
        }
        let first = self.place(String::new());
        self.levels.borrow_mut().insert(rule.to_owned(), first);
        let Some(Stmt::Node { body, .. }) = self.g.rules.get(rule) else { return self.err(format!("`{rule}`: no node")) };
        let end = "close(d, e as u64);\nst = match stack.pop() {\nSome(p) => p,\nNone => return Err(at(src, starts, i)),\n};\ncontinue 'run;\n".to_owned();
        let code = self.walk(body, &Key::default(), K::Code(end), T::Byte)?;
        self.places.borrow_mut()[first] = code;
        Ok(first)
    }

    /// A `check K = G` as a function of the token's bytes: `G` over them, its names byte classes, its literals
    /// bytes, each decision at the next byte.
    fn check_fn(&self, kind: &str, body: &Gram) -> Result<String, String> {
        Ok(format!("/// `check {kind}`: its bytes match the check's grammar.\nfn check_{}(b: &[u8]) -> bool {{\nlet mut j = 0usize;\n{}j == b.len()\n}}\n\n", ident(kind), self.check_code(body, false)?))
    }

    /// The bytes a check's grammar may start with, and whether it may match none.
    fn check_first(&self, g: &Gram) -> Result<(Vec<u8>, bool), String> {
        Ok(match g {
            Gram::Lit(t) if t.len() == 1 => (vec![t.as_bytes()[0]], false),
            Gram::Sym(n) => (self.class(n)?, false),
            Gram::Seq(gs) => {
                let mut out = Vec::new();
                for g in gs {
                    let (f, e) = self.check_first(g)?;
                    out.extend(f);
                    if !e {
                        return Ok((out, false));
                    }
                }
                (out, true)
            }
            Gram::Alt(gs) => {
                let (mut out, mut empty) = (Vec::new(), false);
                for g in gs {
                    let (f, e) = self.check_first(g)?;
                    out.extend(f);
                    empty |= e;
                }
                (out, empty)
            }
            Gram::Opt(a) | Gram::Star(a) => (self.check_first(a)?.0, true),
            Gram::Plus(a) => self.check_first(a)?,
            other => return self.err(format!("a check's {other:?}")),
        })
    }

    fn class(&self, name: &str) -> Result<Vec<u8>, String> {
        for s in self.g.x.lexer {
            if let Stmt::Class(n, items) = s
                && n == name
            {
                let mut out: Vec<u8> = items.iter().flat_map(|&(lo, hi)| (lo..=hi).map(|b| b as u8)).collect();
                out.sort_unstable();
                out.dedup();
                return Ok(out);
            }
        }
        self.err(format!("a check's `{name}`: no class"))
    }

    fn at_byte(&self, g: &Gram) -> Result<String, String> {
        let (mut f, _) = self.check_first(g)?;
        f.sort_unstable();
        f.dedup();
        Ok(format!("matches!(b.get(j), Some({}))", pattern(&f)))
    }

    /// The code of a check's `g`; `tested`, whether the test before it found its first byte (an `[A]`, `A*`, `A+` or
    /// `A | B` found it there, so its code reads it as found).
    fn check_code(&self, g: &Gram, tested: bool) -> Result<String, String> {
        Ok(match g {
            Gram::Lit(_) | Gram::Sym(_) if tested => "j += 1;\n".to_owned(),
            Gram::Lit(_) | Gram::Sym(_) => format!("if !{} {{\nreturn false;\n}}\nj += 1;\n", self.at_byte(g)?),
            Gram::Seq(gs) => gs.iter().enumerate().map(|(k, g)| self.check_code(g, tested && k == 0)).collect::<Result<String, String>>()?,
            Gram::Opt(a) => format!("if {} {{\n{}}}\n", self.at_byte(a)?, self.check_code(a, true)?),
            Gram::Star(a) => format!("while {} {{\n{}}}\n", self.at_byte(a)?, self.check_code(a, true)?),
            Gram::Plus(a) => format!("{}while {} {{\n{}}}\n", self.check_code(a, tested)?, self.at_byte(a)?, self.check_code(a, true)?),
            Gram::Alt(gs) => {
                let mut out = String::new();
                for g in gs {
                    let _ = write!(out, "if {} {{\n{}}} else ", self.at_byte(g)?, self.check_code(g, true)?);
                }
                out + "{\nreturn false;\n}\n"
            }
            other => return self.err(format!("a check's {other:?}")),
        })
    }
}

/// What a test admits at a decision.
enum Admit {
    Bytes(Vec<u8>),
    Word { name: String, rk: u8, word: u8 },
    Kind { name: String, rk: u8 },
}

/// The parser's own code: the writer's ops, the record's first byte, the masks' bits.
const RUNTIME: &str = r#"
/// Node `number` opened at `start`, by the writer's op on its state `d`. `parse` reserved the run's room once
/// (`reserve_run`), for every node and level it can write, so the op does not fail.
#[inline(always)]
fn open(d: *mut u8, number: u32, start: u64) {
    // SAFETY: `d` is the writer's state, its room reserved for the run (`parse`).
    let r = unsafe { w_open_with::<Cells>(d, number, start) };
    debug_assert_eq!(r, 0, "the writer's open, its room reserved");
}

/// A leaf, as `open`.
#[inline(always)]
fn leaf(d: *mut u8, number: u32, start: u64, end: u64) {
    // SAFETY: as `open`'s.
    let r = unsafe { w_leaf_with::<Cells>(d, number, start, end) };
    debug_assert_eq!(r, 0, "the writer's leaf, its room reserved");
}

/// The innermost open node closed at `end`, as `open`.
#[inline(always)]
fn close(d: *mut u8, end: u64) {
    // SAFETY: as `open`'s; a node is open (each close is its open's).
    let r = unsafe { w_close_with::<Cells>(d, end) };
    debug_assert_eq!(r, 0, "the writer's close, its room reserved");
}

/// The nodes so far final but for the open ones, as `open`.
#[inline(always)]
fn commit(d: *mut u8) {
    // SAFETY: as `open`'s.
    let r = unsafe { w_commit(d) };
    debug_assert_eq!(r, 0, "the writer's commit, its room reserved");
}

/// The first byte of record `i`, or 256 past the last.
#[inline(always)]
fn peek(src: &[u8], starts: &[u32], i: usize) -> u16 {
    match starts.get(i) {
        Some(&s) => u16::from(src[s as usize]),
        None => 256,
    }
}

/// Where record `i` starts, or the file's end past the last.
#[inline(always)]
fn at(src: &[u8], starts: &[u32], i: usize) -> u32 {
    starts.get(i).copied().unwrap_or(src.len() as u32)
}

#[inline(always)]
fn bit(x: &[u64], i: usize) -> bool {
    x.get(i / 64).is_some_and(|w| w >> (i % 64) & 1 == 1)
}

/// Mask `x`'s first bit at or after `from`.
#[inline(always)]
fn first_bit(x: &[u64], from: usize) -> Option<usize> {
    let mut q = from / 64;
    let mut w = x.get(q)? & (!0u64 << (from % 64));
    loop {
        if w != 0 {
            return Some(64 * q + w.trailing_zeros() as usize);
        }
        q += 1;
        w = *x.get(q)?;
    }
}

/// The last bit of the run of mask `x` from `s` (`s` itself where the run is empty).
#[inline(always)]
fn run_last(x: &[u64], s: usize) -> usize {
    let mut q = (s + 1) / 64;
    let mut w = !x.get(q).copied().unwrap_or(0) & (!0u64 << ((s + 1) % 64));
    loop {
        if w != 0 {
            return 64 * q + w.trailing_zeros() as usize - 1;
        }
        q += 1;
        w = !x.get(q).copied().unwrap_or(0);
    }
}
"#;

/// Tier 2 of the grammar as one loop (the module's doc), or why it is not written.
pub fn tier2_loop(x: &Input<'_>) -> Result<String, String> {
    let g = G::new(x);
    if !x.marks.is_empty() || !x.extras.is_empty() || !g.flags.is_empty() || !g.readings.is_empty() {
        return Err("tier 2 as one loop: marks, extras, flags and readings are not written yet".into());
    }
    let readers = crate::bitlex_lex_rust::readers(x.lexer);
    let mut ends = BTreeMap::new();
    let mut row_of: [Option<u8>; 256] = [None; 256];
    let mut checked = Vec::new();
    let mut checks = Vec::new();
    for s in x.lexer {
        match s {
            Stmt::Token { kind, start, rule, last, .. } => {
                let k = ident(kind);
                let end = match (rule, last) {
                    (Some(TokenRule::Inside(..)), _) => continue,
                    (Some(TokenRule::Run(_)), _) => End::Run(format!("run_{k}")),
                    (Some(TokenRule::Pair(Expr::Empty)), _) | (None, None) => End::One,
                    (Some(TokenRule::Pair(_)), _) => End::Pair(format!("pair_{k}")),
                    (None, Some(_)) => End::Last(format!("last_{k}")),
                    (Some(TokenRule::ReadParts(..)), _) => End::Region(readers.iter().position(|r| r.kind == *kind).ok_or_else(|| format!("tier 2 as one loop: `{kind}` has no reader"))?),
                    (Some(other), _) => return Err(format!("tier 2 as one loop: `{kind}`'s rule {other:?} is not written yet")),
                };
                // Its first bytes: those its start may hold at. A byte two rows may start at is not told by itself.
                let rk = u8::try_from(ends.len()).map_err(|_| "tier 2 as one loop: more than 255 token rows".to_owned())?;
                let (_, may) = bounds(x.lexer, start, 0);
                for b in 0..256 {
                    if may[b] {
                        if let Some(o) = row_of[b] {
                            return Err(format!("tier 2 as one loop: byte {b:#04x} may start two token rows ({o} and `{kind}`)"));
                        }
                        row_of[b] = Some(rk);
                    }
                }
                ends.insert(kind.clone(), (rk, end));
            }
            Stmt::Check { kind, body } => {
                checked.push(kind.clone());
                checks.push((kind.clone(), body.clone()));
            }
            _ => {}
        }
    }
    let lp = Lp { g: &g, places: RefCell::new(Vec::new()), entries: RefCell::new(Vec::new()), levels: RefCell::new(BTreeMap::new()), ends, readers, checked, row_of, root_end: String::new(), root_place: std::cell::Cell::new(None) };
    // The root: its node, its body, then the stream's end. It ends at the file's end, as tree-sitter's root does.
    let root = *g.order.first().ok_or("tier 2 as one loop: no rule")?;
    let (kind, body) = match g.rules[root] {
        Stmt::Node { kind, body, nested: false, whole: false } => (kind, body),
        _ => return Err("tier 2 as one loop: a root other than a node of no nesting is not written yet".into()),
    };
    let first = lp.place(String::new());
    let number = lp.number(kind, &Key::default())?;
    let end = "close(d, src.len() as u64);\nif i != starts.len() {\nreturn Err(at(src, starts, i));\n}\nreturn Ok(());\n".to_owned();
    let mut lp = lp;
    lp.root_end = end.clone();
    let code = format!("open(d, {number}, at(src, starts, i) as u64);\ncommit(d);\n{}", lp.walk(body, &Key::default(), K::Code(end), T::No)?);
    lp.places.borrow_mut()[first] = code;
    // The words of the kinds split by them.
    let mut words = String::new();
    for name in lp.ends.keys() {
        let rows: Vec<&(String, Vec<(Vec<u8>, Option<String>)>, bool)> = x.keywords.iter().filter(|r| r.0 == *name).collect();
        if rows.is_empty() {
            continue;
        }
        let mut arms = String::new();
        for r in rows {
            for (w, _) in &r.1 {
                if let Some(id) = g.word_of(&String::from_utf8_lossy(w)) {
                    let _ = writeln!(arms, "b\"{}\" => {id},", lit(w));
                }
            }
        }
        let _ = write!(words, "/// A `{name}` token's word: its keyword's, 0 for none.\n#[inline(always)]\nfn word_{}(b: &[u8]) -> u32 {{\nmatch b {{\n{arms}_ => 0,\n}}\n}}\n\n", ident(name));
    }
    let mut check_fns = String::new();
    for (k, b) in &checks {
        check_fns.push_str(&lp.check_fn(k, b)?);
    }
    let places = lp.places.borrow();
    let mut arms = String::new();
    for (l, code) in places.iter().enumerate() {
        let _ = write!(arms, "{l} => {{\n{code}}}\n");
    }
    let ty = if places.len() <= 256 { "u8" } else { "u16" };
    // A level's nodes open at once: its own, and those written where they are used inside it (the root's too).
    let mut frames = 1 + lp.open_in(body, 0)?;
    for rule in lp.levels.borrow().keys() {
        if let Some(Stmt::Node { body, .. }) = g.rules.get(rule.as_str()) {
            frames = frames.max(1 + lp.open_in(body, 0)?);
        }
    }
    let mut entries = String::new();
    for (k, f, cs) in lp.entries.borrow().iter() {
        let _ = writeln!(entries, "    ({k}, {f}, &[{}]),", cs.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(", "));
    }
    let mut out = String::new();
    out.push_str("// Generated by the Bitlex compiler (`rerail_bitlex::bitlex_rust::tier2_loop`) from the grammar's tree rules. Do not edit.\n\n");
    out.push_str("use rerail_tape_format::f16::Writer;\nuse rerail_tape_format::writer16::{Fixed, w_close_with, w_commit, w_leaf_with, w_open_with};\n\n");
    // The language's widths (slots, gaps, lengths) are the parser's: a value past its width is its side list's, as
    // every format 16 tape's. One byte each, the implementer's choice.
    out.push_str("/// The language's cell widths, slots, gaps and lengths: the writer's language is to have these (`Lang::new`).\npub const WIDTHS: [usize; 3] = [1, 1, 1];\n\n/// The writer's cells at those widths, constants in the ops.\ntype Cells = Fixed<1, 1, 1>;\n\n");
    let _ = write!(out, "/// The code table the numbers below are of: each number's kind, field and supertypes.\npub const ENTRIES: &[(u16, u16, &[u16])] = &[\n{entries}];\n\n");
    out.push_str("/// The levels open at most: past it, the file is refused, as the JIT's calls (`native_cpu_stream::DEPTH`).\nconst DEPTH: usize = 1024;\n");
    out.push_str(RUNTIME);
    out.push('\n');
    out.push_str(&words);
    out.push_str(&check_fns);
    let _ = write!(
        out,
        "/// `src`'s tape into `w`, from tier 1's records (each token's first byte) and masks, or the byte it was refused at.\n/// The run's room is reserved once: a node a record, or a byte in a region's pieces, and the root; the levels' nodes\n/// open at once ({frames} a level), and the root's.\npub fn parse(src: &[u8], lx: &crate::tier1::Lexed, w: &mut Writer) -> Result<(), u32> {{\n// Where tier 1 sends the file to tree-sitter.\nif lx.refused {{\nreturn Err(0);\n}}\nw.reserve_run(1 + lx.starts.len() + src.len(), (DEPTH + 1) * {frames} + 1);\nlet d = w.state_address();\nrun(src, lx, d)?;\nmatch w.failure() {{\nNone => Ok(()),\nSome(_) => Err(u32::MAX),\n}}\n}}\n\n/// `parse`'s walk, on the writer's state `d`.\nfn run(src: &[u8], lx: &crate::tier1::Lexed, d: *mut u8) -> Result<(), u32> {{\nlet starts = &lx.starts[..];\nlet m = &lx.masks;\n// The next record, the last token's end, the places the open levels return to, the place.\nlet mut i = 0usize;\nlet mut e = 0usize;\nlet mut stack: Vec<{ty}> = Vec::with_capacity(64);\nlet mut st: {ty} = 0;\n'run: loop {{\nmatch st {{\n{arms}_ => unreachable!(),\n}}\n}}\n}}\n"
    );
    let _ = &x.tokens;
    Ok(out)
}
