//! Tier 2 as one loop, written by the Bitlex compiler from the grammar's tree rules, in simdjson's stage 2's shape
//! (the user, 2026-11-21: "single loop"), never from tier 2's state machine or any lowered form:
//!
//! - **One loop, one dispatch.** A `nested` rule is a level: its call opens its node where the caller is (the node's
//!   key, its field and supertypes, is the caller's), leaves the place it returns to at its depth (`ret[depth]`, the
//!   lean parser's `object[d]`) and goes to the rule's first place; its end closes the node and goes back by its
//!   depth, as simdjson's `scope_end`. The places are the loop's arms (`'run: loop { match st { … } }`); a list
//!   `A (sep A)*` is two, its element and its cycle (simdjson's `object_field`, `object_continue`).
//! - **Decisions at the token**, as simdjson's `switch` on the byte: `[A]`, `A*`, `A+`, `A | B` match the next
//!   record's first byte, and no arm tests again what its decision found. A token kind is its first bytes; a word of a
//!   longer token its first byte, its bytes and the byte after not its row's (simdjson's atoms).
//! - **A token's end from the bytes**: a run's by its check's read (`number_end`) or its row's bytes; a region read in
//!   parts (a string) a block at a time, its closer's and its pieces' bytes (`backslash_quote`), each piece a leaf.
//!   Tier 1 gives the records alone (the index ends in `len`, `len`, 0, the source padded), read unchecked.
//! - **Methods**: a region's read and a choice's alternatives that call no level are written once each (`string`,
//!   `primitive`), the numbers their callers' keys give their arguments.
//! - **The tree** through format 16's writer (`rerail_tape_format::f16::Writer`), each node's number a literal of
//!   the code table the generator makes (`ENTRIES`): each node's kind, field and supertypes, as it writes them. No
//!   node changes once written, so the writer's nodes are final as written (`final_as_written`): no commit.
//! - **Checks**: a `check` of a kind is a function from the token's first byte to its match's end; a
//!   `keywords … required` kind's piece must be one of its words.

// By its path: the `bitlex` crate holds this file by its path too (`#[path]`), where a child module is found beside it.
#[path = "one_loop/records.rs"]
mod records;

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
    /// Out of the decision this is an arm of, to the code after it, which is the continuation inside: a `walk` arm
    /// that ends falls out of its `match`, as `walk`'s cases end in one `close` and one `,` or `}` after it; a level's
    /// call returns to the inside as a place.
    Fall(std::boxed::Box<K>),
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

/// The field of the key a method's code is made with: each number of a node written with it is the method's
/// argument (`n[j]`), the one its caller's key gives, as the lean parser's `string(v, field)` takes its field.
const PLACE: u16 = u16::MAX;

/// A method's numbers: each node kind it writes with its caller's key, and the supertypes it adds to it.
type Numbers = Vec<(String, Vec<u16>)>;

/// Where a part that starts with a choice is decided by its alternatives (`heads`).
struct Heads {
    /// The alternatives, and the key each is written with.
    alts: Vec<Gram>,
    key: Key,
    /// What follows them in the part.
    rest: Vec<Gram>,
    /// The choice they are of, where it is a rule, and the key it is reached with.
    choice: Option<(String, Key)>,
}

/// A decision's arm: its first tokens, and its code given what the decision found.
type Arm<'s> = (Vec<First>, Box<dyn Fn(T) -> Result<String, String> + 's>);

/// The key a node is written with (its code table entry): its field, and the supertypes it is reached through.
#[derive(Clone, Default)]
struct Key {
    field: u16,
    classes: Vec<u16>,
}

/// How a record kind's token ends, as tier 2 finds it from the bytes.
#[derive(Clone)]
enum End {
    /// One byte.
    One,
    /// A run: its check's match, or its row's bytes (`run` in the grammar).
    Run,
    /// A second byte (`pair`): not read from the bytes yet.
    Pair,
    /// A last byte (`last`): not read from the bytes yet.
    Last,
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
    /// The methods made, by name: their numbers.
    methods: RefCell<BTreeMap<String, Numbers>>,
    /// Their code, in the order made.
    method_code: RefCell<Vec<String>>,
    /// The numbers of the methods being made, innermost last.
    making: RefCell<Vec<Numbers>>,
    /// Each place made of code (`to_place`), by its code.
    placed: RefCell<BTreeMap<String, usize>>,
    /// Whether the token taken now is followed in its node by a token of the node's (so no close reads its end):
    /// a separator, an opener, a pair's `:` (`walk`'s `take!()`, its end not kept).
    end_unread: std::cell::Cell<bool>,
    /// Each checked kind's check (`check K = G`).
    check_of: BTreeMap<String, Gram>,
    /// Whether the check being written reads its bytes with no bound (`b[j]`): none of its bytes the padding's, so its
    /// match stops before the padded source's end, as the lean parser's `number_end` reads.
    direct: std::cell::Cell<bool>,
    /// Whether the code being written is the root's, outside every level: there no level is open (`depth` is 0), and
    /// a call tests no depth, as `walk`'s document.
    at_root: std::cell::Cell<bool>,
    /// The run row whose tokens end where `ends` holds (`bitlex_lex_rust::ends_after_run`, simdjson's structural or
    /// whitespace), and `ends`' code.
    run_ends: Option<(u8, String)>,
    /// The methods that decide every byte themselves (a choice's alternatives that call no level, `walk`'s
    /// `primitive`), refusing what none takes as a decision does: a decision's arm calling one is its `_`.
    deciding: RefCell<std::collections::BTreeSet<String>>,
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
            K::Fall(_) => String::new(),
        }
    }

    /// `code` as a place's (`own`, `None` for a new one): a list's `continue 'lN` is its loop's only inside its own
    /// place; elsewhere it goes to the place (`st = N`).
    fn labels(code: &str, own: Option<usize>) -> String {
        let mut out = String::new();
        let mut rest = code;
        while let Some(p) = rest.find("continue 'l") {
            out.push_str(&rest[..p]);
            let after = &rest[p + "continue 'l".len()..];
            let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
            let n: usize = digits.parse().expect("a list's place");
            rest = &after[digits.len()..];
            if own == Some(n) {
                let _ = write!(out, "continue 'l{n}");
            } else {
                let tail = rest.strip_prefix(";\n").expect("a list's continue ends its line");
                let _ = write!(out, "{}", Self::go(n));
                rest = tail;
            }
        }
        out.push_str(rest);
        out
    }

    /// `k` as a place: code used more than once is written once. Code that only goes on to a place is that place.
    fn to_place(&self, k: K) -> usize {
        match k {
            K::Go(l) => l,
            K::Fall(k) => self.to_place(*k),
            K::Code(c) => {
                let c = Self::labels(&c, None);
                if let Some(l) = c.strip_prefix("st = ").and_then(|r| r.strip_suffix(";\ncontinue 'run;\n")).and_then(|l| l.parse().ok()) {
                    return l;
                }
                // The same code is one place.
                if let Some(&l) = self.placed.borrow().get(&c) {
                    return l;
                }
                let root = c == self.root_end;
                let l = self.place(c.clone());
                self.placed.borrow_mut().insert(c, l);
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

    /// The number of a node of `kind` written with `key`: a literal, or in a method, where `key` is its caller's
    /// (`PLACE`), its argument for it.
    fn number(&self, kind: &str, key: &Key) -> Result<String, String> {
        if key.field == PLACE {
            let mut making = self.making.borrow_mut();
            let ns = making.last_mut().ok_or_else(|| "tier 2 as one loop: a method's key outside a method".to_owned())?;
            let e = (kind.to_owned(), key.classes.clone());
            let j = ns.iter().position(|x| *x == e).unwrap_or_else(|| {
                ns.push(e);
                ns.len() - 1
            });
            return Ok(format!("n[{j}]"));
        }
        Ok(self.literal(kind, key)?.to_string())
    }

    /// The number of a node of `kind` written with `key`, a key of the code table.
    fn literal(&self, kind: &str, key: &Key) -> Result<usize, String> {
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

    /// The end (one past the last byte) of the token at `s` of kind `name`, by its kind's rule, read from the bytes:
    /// a run's by its check where it has one (its bytes read once, as the lean parser's `number_end`), else by its
    /// row's bytes; the byte after a run not its row's (`in_row`, simdjson's `is_not_structural_or_whitespace`).
    fn end_code(&self, name: &str, end: &End) -> Result<String, String> {
        Ok(match end {
            End::One => "s + 1".to_owned(),
            End::Run => {
                if self.checked.iter().any(|c| c == name) {
                    format!("{}_end(src, s)?", ident(name))
                } else {
                    let (rk, _) = self.token(name)?;
                    format!("{{\nlet mut x = s + 1;\nwhile in_row(src[x], {}) {{\nx += 1;\n}}\nx\n}}", rk + 1)
                }
            }
            End::Pair | End::Last | End::Region(_) => return self.err(format!("`{name}`: its end from the bytes is not written yet")),
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
        // The byte arms, each arm's bytes one pattern.
        let mut arm_bytes: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
        for (&b, &a) in &by_byte {
            if split.keys().any(|&rk| self.row_of[usize::from(b)] == Some(rk)) {
                return self.err(format!("byte {b:#04x} is a word's and a kind's"));
            }
            arm_bytes.entry(a).or_default().push(b);
        }
        // An arm that calls a method deciding every byte itself is the `_`, where none passing is refused, as `walk`'s
        // `_ => primitive`: the method written once, called from each place.
        let err = "return Err(at(src, starts, i));\n";
        let mut otherwise = none.to_owned();
        if none == err && split.is_empty() {
            let deciding = self.deciding.borrow().clone();
            for &a in arm_bytes.keys() {
                let c = code(a, T::Byte)?;
                if deciding.iter().any(|m| c.starts_with(&format!("e = unsafe {{ {m}::<S>(src, len, starts, d, &mut i, ["))) {
                    otherwise = c;
                    arm_bytes.remove(&a);
                    break;
                }
            }
        }
        // One `match` on the token's first byte: a split kind's words its arms too, each a guard on its bytes, and the
        // kind's own arm its row's bytes after them (simdjson's `primitive`: one `switch`).
        let mut out = if split.is_empty() { String::from("match peek(src, starts, i) {\n") } else { String::from("let s = starts[i] as usize;\nmatch src[s] {\n") };
        for (a, bs) in &arm_bytes {
            let _ = write!(out, "{} => {{\n{}}}\n", pattern(bs), code(*a, T::Byte)?);
        }
        for (rk, (name, words, own)) in &split {
            let (_, end) = self.token(name)?;
            // A word: its first byte, its bytes, the byte after not the row's (simdjson's atoms, `str4ncmp`).
            let word = |w: u8| -> (String, usize) {
                let b = &self.g.x.words[usize::from(w) - 1];
                let after = match &self.run_ends {
                    Some((r, _)) if *r == *rk => format!("ends(src[s + {}])", b.len()),
                    _ => format!("!in_row(src[s + {}], {})", b.len(), rk + 1),
                };
                (format!("{:#04x} if src[s..s + {l}] == *b\"{}\" && {after}", b[0], lit(b), l = b.len()), b.len())
            };
            let _ = writeln!(out, "// `{name}`: its words by their bytes, then its own.");
            for (w, a) in words {
                let (test, l) = word(*w);
                let _ = write!(out, "{test} => {{\nlet end = s + {l};\n{}}}\n", code(*a, T::Word)?);
            }
            if let Some(a) = own {
                // The kind's own arm: none of its hard words.
                for h in self.hard_words(name).iter().filter(|h| !words.iter().any(|(w, _)| w == *h)) {
                    let _ = write!(out, "{} => {{\n{none}}}\n", word(*h).0);
                }
                // A checked run's arm at its check's first bytes alone, where the rest is refused at the token's byte, as
                // the check refuses there (`walk`'s number at a digit or `-`).
                let mut bytes = self.bytes_of(*rk);
                if matches!(end, End::Run)
                    && none == "return Err(at(src, starts, i));\n"
                    && let Some(body) = self.check_of.get(name)
                    && let Ok((first, false)) = self.check_first(body)
                {
                    bytes.retain(|b| first.contains(b));
                }
                let _ = write!(out, "{} => {{\nlet end = {};\n{}}}\n", pattern(&bytes), self.end_code(name, &end)?, code(*a, T::Word)?);
            }
        }
        let _ = write!(out, "_ => {{\n{otherwise}}}\n}}\n");
        Ok(out)
    }

    /// The code that takes the token `test` is, a leaf of `kind` unless it is implied (`None`), with `key`; `t`, what
    /// a decision found of it.
    fn take(&self, test: &Test, kind: Option<&str>, key: &Key, t: T) -> Result<String, String> {
        let (name, end) = match test {
            Test::Kind(k) => (k.clone(), self.token(k)?.1),
            Test::Word(w) => {
                let (name, _, end, _) = self.word_token(*w)?;
                (name, end)
            }
            Test::Mark(_) => return self.err("marks are not written yet"),
        };
        // A run's check is read where its end is found.
        let run_checked = matches!(end, End::Run) && self.checked.contains(&name.to_owned());
        let body = |t: T| -> Result<String, String> {
            // A word's decision bound its start and its end.
            // A leaf of its own with no check after its end: one call, written once (`token`), as `walk`'s `leaf`.
            if let Some(k) = kind
                && !(self.checked.iter().any(|c| c == k) && !(run_checked && k == name))
            {
                let n = self.number(k, key)?;
                return Ok(if t == T::Word { format!("e = token(d, &mut i, {n}, s, end);\n") } else { format!("let s = starts[i] as usize;\ne = token(d, &mut i, {n}, s, {});\n", self.end_code(&name, &end)?) });
            }
            if kind.is_none() && t != T::Word && matches!(end, End::One) && self.end_unread.get() {
                return Ok("i += 1;\n".to_owned());
            }
            let mut body = if t == T::Word { "i += 1;\ne = end;\n".to_owned() } else { format!("let s = starts[i] as usize;\ni += 1;\ne = {};\n", self.end_code(&name, &end)?) };
            if let Some(k) = kind {
                if self.checked.iter().any(|c| c == k) && !(run_checked && k == name) {
                    let _ = writeln!(body, "if check_{}(&src[s..e]) != Some(e - s) {{\nreturn Err(s as u32);\n}}", ident(k));
                }
                let _ = writeln!(body, "leaf(d, {}, s as u64, e as u64);", self.number(k, key)?);
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
        let words: Vec<Vec<u8>> = self.g.x.keywords.iter().filter(|r| r.0 == *part && r.2).flat_map(|r| r.1.iter().map(|(w, _)| w.clone())).collect();
        let required: Vec<String> = words.iter().map(|w| format!("b\"{}\"", lit(w))).collect();
        // The closer's byte (the opener's), and the pieces' first (their words'): the read finds both in the bytes.
        let quote = match self.bytes_of(self.token(&o)?.0).as_slice() {
            [b] => *b,
            _ => return self.err(format!("`{kind}`: a closer of other than one byte is not written yet")),
        };
        let lead = match words.iter().map(|w| w[0]).collect::<std::collections::BTreeSet<u8>>().into_iter().collect::<Vec<_>>().as_slice() {
            [b] if *b != quote => *b,
            _ => return self.err(format!("`{kind}`: pieces of other than one first byte are not written yet")),
        };
        let mut check = if required.is_empty() { String::new() } else { format!("if !matches!(&src[p..p + {len}], {}) {{\nreturn Err(p as u32);\n}}\n", required.join(" | ")) };
        // The grammar's errors at an escape, where it is read (the lean parser's stage 2; decision 0599): tier 1 leaves
        // them out (`bitlex_lex_rust::piece_covers`).
        // The pieces' words of two bytes, the lead's, decided by their second byte once (`escaped_string`'s `c`): the
        // gate's bytes its checks' arm, the other words' an arm of none, any other byte refused.
        let seconds: Option<Vec<u8>> = (*len == 2 && words.iter().all(|w| w.len() == 2)).then(|| words.iter().map(|w| w[1]).collect());
        let state = match crate::bitlex_piece_checks::piece_checks(self.g.x.lexer, part_start, Some(quote)) {
            Ok(pc) => {
                match (&pc.gate, &seconds) {
                    (Some((g, head, inside)), Some(sec)) if sec.iter().any(|b| g[usize::from(*b)]) && (0..256).all(|b| !g[b] || sec.contains(&(b as u8))) => {
                        let gb: Vec<u8> = sec.iter().copied().filter(|b| g[usize::from(*b)]).collect();
                        let rest: Vec<u8> = sec.iter().copied().filter(|b| !g[usize::from(*b)]).collect();
                        let others = if rest.is_empty() { String::new() } else { format!("{} => {{}}\n", pattern(&rest)) };
                        check = format!("{head}// Its second byte decided once: the words', the checks at the gate's.\nmatch src[y] {{\n{} => {{\n{inside}}}\n{others}_ => return Err(p as u32),\n}}\n", pattern(&gb));
                    }
                    _ => check.push_str(&pc.at_piece),
                }
                if !self.method_code.borrow().contains(&pc.statics) {
                    self.method_code.borrow_mut().push(pc.statics);
                }
                pc.state
            }
            Err(_) => String::new(),
        };
        // Tier 1 refuses a file the region is open at the end of: the read meets its closer before it, as simdjson's
        // stage 2 reads past no bound.
        let bound = if !rd.regions.is_empty() && rd.regions.iter().all(|r| crate::bitlex_lex_rust::refuses_open_at_end(self.g.x.lexer, &r.kind)) { String::new() } else { "if x >= len {\nreturn Err(s as u32);\n}\n".to_owned() };
        let test = if t == T::No { format!("match peek(src, starts, i) {{\n{} => {{}}\n_ => return Err(at(src, starts, i)),\n}}\n", pattern(&self.bytes_of(self.token(&o)?.0))) } else { String::new() };
        // The read, a method of its own (`string` in the lean parser), the node's number its caller's.
        let ns = self.method(&ident(kind), &format!("`{kind}`, its pieces leaves"), |key| {
            let node = self.number(kind, key)?;
            let content = self.number(&rd.content, &Key::default())?;
            let piece = self.number(part, &Key::default())?;
            Ok(format!(
            "let s = starts[i] as usize;\n\
             i += 1;\n\
             let (lead, quote) = unsafe {{ marks::<S>(src, s + 1, {lead:#04x}, {quote:#04x}) }};\n\
             // A closer before every piece in the first block (simdjson's likely case): the node and its content at once,\n\
             // or the node a leaf where it is empty.\n\
             if lead.wrapping_sub(1) & quote != 0 {{\n\
             let c = s + 1 + quote.trailing_zeros() as usize;\n\
             e = c + 1;\n\
             if c == s + 1 {{\nleaf(d, {node}, s as u64, e as u64);\n}} else {{\nleaf_in(d, {node}, {content}, s as u64, (s + 1) as u64, c as u64, e as u64);\n}}\n\
             *ip = i;\n\
             return Ok(e);\n\
             }}\n\
             open(d, {node}, s as u64);\n\
             // The content from `p`, read from `x` 32 bytes at a time (`backslash_quote`): its pieces' first bytes and its closers.\n\
             let (mut p, mut x) = (s + 1, s + 1);\n\
             let (mut lead, mut quote) = (lead, quote);\n\
             {state}\
             loop {{\n\
             // A closer before every piece: the end (simdjson's `(bs - 1) & quote`).\n\
             if lead.wrapping_sub(1) & quote != 0 {{\n\
             let c = x + quote.trailing_zeros() as usize;\n\
             if c > p {{\nleaf(d, {content}, p as u64, c as u64);\n}}\n\
             e = c + 1;\n\
             break;\n\
             }}\n\
             if lead == 0 {{\nx += 32;\n{bound}(lead, quote) = unsafe {{ marks::<S>(src, x, {lead_b:#04x}, {quote_b:#04x}) }};\ncontinue;\n}}\n\
             let q = x + lead.trailing_zeros() as usize;\n\
             if q > p {{\nleaf(d, {content}, p as u64, q as u64);\n}}\n\
             p = q;\n{check}\
             leaf(d, {piece}, p as u64, (p + {len}) as u64);\n\
             p += {len};\n\
             x = p;\n{bound}\
             (lead, quote) = unsafe {{ marks::<S>(src, x, {lead_b:#04x}, {quote_b:#04x}) }};\n\
             }}\n\
             close(d, e as u64);\n",
            lead_b = lead,
            quote_b = quote,
            ))
        })?;
        Ok(Some(format!("{test}{}", self.call(&ident(kind), &ns, key)?)))
    }

    /// `g` as alternatives where it starts with a choice (`Heads`). None where `g` does not, or an alternative may be
    /// empty (its first tokens would be those after it). A decision among them is the choice's own: no second one.
    fn heads(&self, g: &Gram, key: &Key) -> Result<Option<Heads>, String> {
        Ok(match g {
            Gram::Alt(gs) => {
                for a in gs {
                    if self.g.first(a)?.1 {
                        return Ok(None);
                    }
                }
                Some(Heads { alts: gs.clone(), key: key.clone(), rest: Vec::new(), choice: None })
            }
            Gram::Sym(n) => match self.g.rules.get(n.as_str()) {
                Some(Stmt::Choice { nested: false, supertype, body, .. }) => self.heads(body, &self.through(key, supertype.as_deref())?)?.map(|h| Heads { choice: h.choice.or_else(|| Some((n.clone(), key.clone()))), ..h }),
                _ => None,
            },
            Gram::Field(f, a) => {
                let field = (self.g.x.field)(f).ok_or_else(|| format!("no field `{f}`"))?;
                self.heads(a, &Key { field, ..key.clone() })?
            }
            Gram::Seq(gs) => match gs.split_first() {
                Some((h, rest)) => self.heads(h, key)?.map(|mut h| {
                    h.rest.extend(rest.iter().cloned());
                    h
                }),
                None => None,
            },
            _ => None,
        })
    }

    /// A decision's arms for `h`'s alternatives, each going on to `k`. Of a choice that is a rule, the alternatives
    /// that call no level are one arm, a call of the choice's method (`primitive` in the lean parser), which decides
    /// among them; each other alternative is an arm of its own.
    fn alt_arms<'s>(&'s self, h: Heads, k: K) -> Result<Vec<Arm<'s>>, String> {
        let mut arms: Vec<Arm<'s>> = Vec::new();
        // The alternatives that call no level, one method (`walk`'s `primitive`), written once and called from each
        // place: its arm the place's `_`. The level calls' alternatives are the place's arms of their own.
        let plain: Vec<&Gram> = h.alts.iter().filter(|a| !self.calls(a)).collect();
        if let Some((name, at)) = &h.choice
            && plain.len() > 1
        {
            let alts: Vec<Gram> = plain.iter().map(|a| (*a).clone()).collect();
            let first: Vec<First> = alts.iter().map(|a| self.g.first(a).map(|f| f.0)).collect::<Result<Vec<_>, _>>()?.concat();
            let mname = format!("{}_primitive", ident(name));
            let ns = self.method(&mname, &format!("`{name}`'s alternatives that call no level (`walk`'s `primitive`)"), |place| {
                let Some(hp) = self.heads(&Gram::Sym(name.clone()), place)? else { return self.err(format!("`{name}`: no alternatives")) };
                let mine: Vec<Gram> = hp.alts.into_iter().filter(|a| !self.calls(a)).collect();
                self.plain(&Gram::Alt(mine), &hp.key, T::No)
            })?;
            self.deciding.borrow_mut().insert(mname.clone());
            let (call, k2) = (self.call(&mname, &ns, at)?, k.clone());
            arms.push((first, Box::new(move |_| Ok(format!("{call}{}", Self::jump(&k2))))));
            for a in h.alts.into_iter().filter(|a| self.calls(a)) {
                let (first, _) = self.g.first(&a)?;
                let (akey, k) = (h.key.clone(), k.clone());
                arms.push((first, Box::new(move |t| self.walk(&a, &akey, k.clone(), t))));
            }
            return Ok(arms);
        }
        for a in h.alts {
            let (first, _) = self.g.first(&a)?;
            let (akey, k) = (h.key.clone(), k.clone());
            arms.push((first, Box::new(move |t| self.walk(&a, &akey, k.clone(), t))));
        }
        Ok(arms)
    }

    /// The arms of a decision on `g`, each going on to `k` (`walk`): its alternatives where it starts with a choice
    /// (`heads`), else `g` itself.
    fn walk_arms<'s>(&'s self, g: &'s Gram, key: &Key, k: K) -> Result<Vec<Arm<'s>>, String> {
        if let Some(mut h) = self.heads(g, key)? {
            let rest = std::mem::take(&mut h.rest);
            let k = if rest.is_empty() { k } else { K::Go(self.to_place(K::Code(self.walk(&Gram::Seq(rest), &Key::default(), k, T::No)?))) };
            return self.alt_arms(h, k);
        }
        let key = key.clone();
        Ok(vec![(self.g.first(g)?.0, Box::new(move |t| self.walk(g, &key, k.clone(), t)))])
    }

    /// The arms of a decision on `g` with no call in it (`plain`): its alternatives where it is a choice alone, else
    /// `g` itself.
    fn plain_arms<'s>(&'s self, g: &'s Gram, key: &Key) -> Result<Vec<Arm<'s>>, String> {
        if let Some(h) = self.heads(g, key)?
            && h.rest.is_empty()
        {
            let mut arms: Vec<Arm<'s>> = Vec::new();
            for a in h.alts {
                let (first, _) = self.g.first(&a)?;
                let akey = h.key.clone();
                arms.push((first, Box::new(move |t| self.plain(&a, &akey, t))));
            }
            return Ok(arms);
        }
        let key = key.clone();
        Ok(vec![(self.g.first(g)?.0, Box::new(move |t| self.plain(g, &key, t)))])
    }

    /// Method `name`, made once: its code `body` with its caller's key (`PLACE`). It takes the next record at `ip` and
    /// returns the token's end, as the lean parser's `string` and `primitive` take a value and return its end.
    fn method(&self, name: &str, what: &str, body: impl FnOnce(&Key) -> Result<String, String>) -> Result<Numbers, String> {
        if let Some(ns) = self.methods.borrow().get(name) {
            return Ok(ns.clone());
        }
        self.making.borrow_mut().push(Vec::new());
        let code = body(&Key { field: PLACE, classes: Vec::new() });
        let ns = self.making.borrow_mut().pop().expect("the method's numbers");
        let code = code?;
        let n = if ns.is_empty() { "_n" } else { "n" };
        self.method_code.borrow_mut().push(format!(
            "/// {what}, read at the next record (`ip`): the token's end. `n`: the numbers of its nodes written with its\n/// caller's key, as each place gives them.\n///\n/// # Safety\n/// The CPU runs `S`'s backend (`run_best` chose it).\n#[inline(always)]\nunsafe fn {name}<S: rerail_simd8::Simd8x64>(src: &[u8], len: usize, starts: &[u32], d: &mut Cursor<Cells>, ip: &mut usize, {n}: [u32; {}]) -> Result<usize, u32> {{\nlet mut i = *ip;\nlet mut e = 0usize;\n{code}*ip = i;\nOk(e)\n}}\n\n",
            ns.len()
        ));
        self.methods.borrow_mut().insert(name.to_owned(), ns.clone());
        Ok(ns)
    }

    /// A call of method `name`, its numbers those of `key`: the token's end in `e`.
    fn call(&self, name: &str, ns: &Numbers, key: &Key) -> Result<String, String> {
        let mut args = Vec::new();
        for (kind, extra) in ns {
            let mut classes = key.classes.clone();
            classes.extend(extra.iter().copied());
            args.push(self.number(kind, &Key { field: key.field, classes })?);
        }
        // SAFETY (in the code): the caller's backend, `run_best`'s.
        Ok(format!("e = unsafe {{ {name}::<S>(src, len, starts, d, &mut i, [{}]) }}?;\n", args.join(", ")))
    }

    /// `a` then `star` as a list `A (sep A)*`: its separator's parts.
    fn list_sep<'g>(a: &Gram, star: &'g Gram) -> Option<&'g [Gram]> {
        let Gram::Star(b) = star else { return None };
        let Gram::Seq(bs) = &**b else { return None };
        let (last, sep) = bs.split_last()?;
        (last == a && !sep.is_empty()).then_some(sep)
    }

    /// A list `a (sep a)*`, then `rest`, then `k`: its head (`a`, its first token tested there) and its cycle (a
    /// decision on the separator, going on to the head, and on what follows, going on to `rest`), one place each.
    /// `f` with `end_unread` set where `g` is a token without a node, and a token of the node follows it
    /// (`follows`): its end then is read by no close.
    fn unread<R>(&self, g: &Gram, follows: bool, f: impl FnOnce() -> R) -> R {
        // A token without a node is one `as` nothing (`take` writes a leaf for every other).
        let bare = matches!(g, Gram::As(k, a) if k == ::bitlex::syntax::IMPLIED && self.g.token_of(a).is_some());
        let was = self.end_unread.replace(follows && bare);
        let r = f();
        self.end_unread.set(was);
        r
    }

    /// Whether some of `gs` cannot be empty: a token is taken there before the node's close.
    fn takes(&self, gs: &[Gram]) -> Result<bool, String> {
        for g in gs {
            if !self.g.first(g)?.1 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn list(&self, a: &Gram, sep: &[Gram], rest: &[Gram], k: K) -> Result<usize, String> {
        let head = self.place(String::new());
        let again = format!("continue 'l{head};\n");
        let (sep, rest, none) = (Gram::Seq(sep.to_vec()), Gram::Seq(rest.to_vec()), Key::default());
        // A separator of one token: the element follows it.
        let one = match &sep {
            Gram::Seq(gs) if gs.len() == 1 => gs[0].clone(),
            g => g.clone(),
        };
        let mut arms: Vec<Arm<'_>> = vec![(self.g.first(&sep)?.0, Box::new(|t| self.unread(&one, true, || self.walk(&one, &none, K::Code(again.clone()), t))))];
        let (_, empty) = self.g.first(&rest)?;
        let otherwise = if empty {
            self.walk(&rest, &none, k, T::No)?
        } else {
            arms.extend(self.walk_arms(&rest, &none, k)?);
            "return Err(at(src, starts, i));\n".to_owned()
        };
        // The cycle after the element, written where the element ends (a level's call returns to it as a place).
        let cycle = self.decide(&arms, &otherwise)?;
        let body = self.walk(a, &none, K::Code(cycle), T::No)?;
        let code = format!("// The list: its element, then its separator back to the element, or what follows it.\n'l{head}: loop {{\n{body}}}\n");
        self.places.borrow_mut()[head] = Self::labels(&code, Some(head));
        Ok(head)
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
                    format!("open(d, {}, at(src, starts, i) as u64);\n{}close(d, e as u64);\n", self.number(kind, key)?, self.plain(body, &Key::default(), t)?)
                }
                Some(Stmt::Choice { supertype, body, .. }) => self.plain(body, &self.through(key, supertype.as_deref())?, t)?,
                _ => return self.err(format!("`{n}`: no rule or token")),
            },
            Gram::Seq(gs) => {
                let (mut out, none) = (String::new(), Key::default());
                for (k, g) in gs.iter().enumerate() {
                    let follows = self.takes(&gs[k + 1..])?;
                    out.push_str(&self.unread(g, follows, || self.plain(g, if k == 0 { key } else { &none }, if k == 0 { t } else { T::No }))?);
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
                    // Empty, it falls out where its continuation is after the decision, else goes to it.
                    let falls = matches!(k, K::Fall(_));
                    let ret = self.to_place(k);
                    let empty_end = if falls { String::new() } else { Self::go(ret) };
                    let number = self.number(kind, key)?;
                    let mut out = String::new();
                    let depth_test = if self.at_root.get() { "" } else { "if depth == DEPTH {\nreturn Err(at(src, starts, i));\n}\n" };
                    if let Some((_, close)) = self.brackets(n)? {
                        // The level the root's value: the file's last record its closer, checked before the walk, as
                        // the lean parser's `last != '}'`.
                        if self.root_place.get() == Some(ret) {
                            let _ = write!(out, "if src[starts[count - 1] as usize] != {close:#04x} {{\nreturn Err(at(src, starts, i));\n}}\n");
                        }
                        // Empty: one leaf from its opener (its decision's) to its closer, as the lean parser's `empty!`;
                        // its depth tested as a level's.
                        let _ = write!(
                            out,
                            "{}if peek(src, starts, i + 1) == {close:#04x} {{\n// `{kind}`, empty.\nlet s = starts[i] as usize;\ne = starts[i + 1] as usize + 1;\ni += 2;\nleaf(d, {number}, s as u64, e as u64);\n{}}} else {{\n",
                            depth_test, empty_end
                        );
                    } else {
                        out.push_str(depth_test);
                    }
                    let _ = write!(out, "open(d, {number}, at(src, starts, i) as u64);\nret[depth] = {ret};\ndepth += 1;\n{}", Self::go(first));
                    if self.brackets(n)?.is_some() {
                        out.push_str("}\n");
                    }
                    out
                }
                Some(Stmt::Node { kind, body, .. }) => {
                    let after = format!("close(d, e as u64);\n{}", Self::jump(&k));
                    format!("open(d, {}, at(src, starts, i) as u64);\n{}", self.number(kind, key)?, self.walk(body, &Key::default(), K::Code(after), t)?)
                }
                Some(Stmt::Choice { nested: false, supertype, body, .. }) => match self.heads(g, key)? {
                    Some(h) => {
                        // Each alternative that ends falls out of the decision to the continuation, written once after
                        // it; a level's call returns to it as a place.
                        let arms = self.alt_arms(h, K::Fall(std::boxed::Box::new(k.clone())))?;
                        format!("{}{}", self.decide(&arms, "return Err(at(src, starts, i));\n")?, Self::jump(&k))
                    }
                    None => self.walk(body, &self.through(key, supertype.as_deref())?, k, t)?,
                },
                _ => return self.err(format!("`{n}`: a nested choice is not written yet")),
            },
            Gram::Seq(gs) => {
                let none = Key::default();
                // A list `A (sep A)*`: A one place, its cycle one place (the lean parser's `object_field` and
                // `object_continue`). Not first where a decision found its token or it has a key: its head reads none.
                let list = (0..gs.len().saturating_sub(1)).find(|&p| Self::list_sep(&gs[p], &gs[p + 1]).is_some() && (p > 0 || (t == T::No && key.field == 0 && key.classes.is_empty())));
                let (gs, mut k) = match list {
                    Some(p) => {
                        let sep = Self::list_sep(&gs[p], &gs[p + 1]).expect("a list");
                        (&gs[..p], K::Go(self.list(&gs[p], sep, &gs[p + 2..], k)?))
                    }
                    None => (&gs[..], k),
                };
                for (j, g) in gs.iter().enumerate().rev() {
                    let follows = list.is_some() || self.takes(&gs[j + 1..])?;
                    k = K::Code(self.unread(g, follows, || self.walk(g, if j == 0 { key } else { &none }, k, if j == 0 { t } else { T::No }))?);
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
        let root = self.at_root.replace(false);
        let Some(Stmt::Node { body, .. }) = self.g.rules.get(rule) else { return self.err(format!("`{rule}`: no node")) };
        // Bracketed, `O [M] C`: each call took `O C` as the level empty, so here `M` is there.
        let there;
        let body = match body {
            Gram::Seq(gs) if gs.len() == 3 && self.brackets(rule)?.is_some() => match &gs[1] {
                Gram::Opt(m) => {
                    let mut parts = vec![gs[0].clone()];
                    match &**m {
                        Gram::Seq(ms) => parts.extend(ms.iter().cloned()),
                        m => parts.push(m.clone()),
                    }
                    parts.push(gs[2].clone());
                    there = Gram::Seq(parts);
                    &there
                }
                _ => body,
            },
            _ => body,
        };
        let end = "close(d, e as u64);\n// The place the level's call returns to, by its depth (a level ends only inside its call).\ndepth -= 1;\nst = ret[depth];\ncontinue 'run;\n".to_owned();
        let code = self.walk(body, &Key::default(), K::Code(end), T::Byte);
        self.at_root.set(root);
        self.places.borrow_mut()[first] = code?;
        Ok(first)
    }

    /// A `check K = G` as a function of the token's bytes: `G` over them, its names byte classes, its literals
    /// bytes, each decision at the next byte.
    fn check_fn(&self, kind: &str, body: &Gram) -> Result<String, String> {
        // The padding's byte (a space) none of the check's: its match stops at the padding at the latest.
        self.direct.set(!self.check_bytes(body)?.contains(&b' '));
        let code = self.check_code(body, false)?;
        self.direct.set(false);
        Ok(format!("/// `check {kind}`: the end of the check's grammar's match from the start of `b` (the bytes from the token's,\n/// to the padded source's end), or none.\nfn check_{}(b: &[u8]) -> Option<usize> {{\nlet mut j = 0usize;\n{}Some(j)\n}}\n\n", ident(kind), code))
    }

    /// Every byte a check's grammar may read.
    fn check_bytes(&self, g: &Gram) -> Result<Vec<u8>, String> {
        Ok(match g {
            Gram::Lit(_) | Gram::Sym(_) => self.check_first(g)?.0,
            Gram::Seq(gs) | Gram::Alt(gs) => gs.iter().map(|g| self.check_bytes(g)).collect::<Result<Vec<_>, _>>()?.concat(),
            Gram::Opt(a) | Gram::Star(a) | Gram::Plus(a) => self.check_bytes(a)?,
            other => return self.err(format!("a check's {other:?}")),
        })
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
        Ok(if self.direct.get() { format!("matches!(b[j], {})", pattern(&f)) } else { format!("matches!(b.get(j), Some({}))", pattern(&f)) })
    }

    /// The code of a check's `g`; `tested`, whether the test before it found its first byte (an `[A]`, `A*`, `A+` or
    /// `A | B` found it there, so its code reads it as found).
    fn check_code(&self, g: &Gram, tested: bool) -> Result<String, String> {
        Ok(match g {
            Gram::Lit(_) | Gram::Sym(_) if tested => "j += 1;\n".to_owned(),
            Gram::Lit(_) | Gram::Sym(_) => format!("if !{} {{\nreturn None;\n}}\nj += 1;\n", self.at_byte(g)?),
            Gram::Seq(gs) => gs.iter().enumerate().map(|(k, g)| self.check_code(g, tested && k == 0)).collect::<Result<String, String>>()?,
            Gram::Opt(a) => format!("if {} {{\n{}}}\n", self.at_byte(a)?, self.check_code(a, true)?),
            Gram::Star(a) => format!("while {} {{\n{}}}\n", self.at_byte(a)?, self.check_code(a, true)?),
            Gram::Plus(a) => format!("{}while {} {{\n{}}}\n", self.check_code(a, tested)?, self.at_byte(a)?, self.check_code(a, true)?),
            // Each alternative one byte: one test of them all (`walk`'s exponent sign), none where the test before found it.
            Gram::Alt(gs) if gs.iter().all(|g| matches!(g, Gram::Lit(_) | Gram::Sym(_))) => {
                if tested {
                    "j += 1;\n".to_owned()
                } else {
                    format!("if !{} {{\nreturn None;\n}}\nj += 1;\n", self.at_byte(g)?)
                }
            }
            Gram::Alt(gs) => {
                let mut out = String::new();
                for g in gs {
                    let _ = write!(out, "if {} {{\n{}}} else ", self.at_byte(g)?, self.check_code(g, true)?);
                }
                out + "{\nreturn None;\n}\n"
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

/// The parser's own code: the writer's ops, the record's first byte, the block's bytes; what nothing calls is left out.
const RUNTIME: &str = r#"
/// Node `number` opened at `start`, stored through the walk's hold on the tape (`d`, the lean parser's `Walk`).
/// `parse` reserved the run's room once (`reserve_run`), for every node and level it can write.
#[inline(always)]
fn open(d: &mut Cursor<Cells>, number: u32, start: u64) {
    // SAFETY: the cursor's state's room is reserved for the run (`parse`).
    unsafe { d.open(number, start) }
}

/// A leaf, as `open`.
#[inline(always)]
fn leaf(d: &mut Cursor<Cells>, number: u32, start: u64, end: u64) {
    // SAFETY: as `open`'s.
    unsafe { d.leaf(number, start, end) }
}

/// A node of one leaf child, both at once (the writer's `leaf_in`), as `open`.
#[inline(always)]
fn leaf_in(d: &mut Cursor<Cells>, number: u32, child: u32, start: u64, cs: u64, ce: u64, end: u64) {
    // SAFETY: as `open`'s.
    unsafe { d.leaf_in(number, child, start, cs, ce, end) }
}

/// A token's leaf from `s` to `end`, the next record taken: its end (the lean parser's `leaf`, for each place's arm).
#[inline(always)]
fn token(d: &mut Cursor<Cells>, i: &mut usize, number: u32, s: usize, end: usize) -> usize {
    *i += 1;
    leaf(d, number, s as u64, end as u64);
    end
}

/// The innermost open node closed at `end`, as `open`.
#[inline(always)]
fn close(d: &mut Cursor<Cells>, end: u64) {
    // SAFETY: as `open`'s; a node is open (each close is its open's).
    unsafe { d.close(end) }
}

/// The first byte of record `i`: past the last, the padding's (a space, which no token starts with), as simdjson's
/// stage 2 reads `*next`.
#[inline(always)]
fn peek(src: &[u8], starts: &[u32], i: usize) -> u8 {
    debug_assert!(i < starts.len() && (starts[i] as usize) < src.len());
    // SAFETY: `i` is at most the count: a record past the last is the first of the index's three last entries,
    // `len`, whose byte is the padding's, which no arm takes, so nothing reads past it; `len` is within the padded
    // source.
    unsafe { *src.get_unchecked(*starts.get_unchecked(i) as usize) }
}

/// Where record `i` starts: past the last, the file's end (the index's last entries).
#[inline(always)]
fn at(src: &[u8], starts: &[u32], i: usize) -> u32 {
    let _ = src;
    debug_assert!(i < starts.len());
    // SAFETY: as `peek`'s.
    unsafe { *starts.get_unchecked(i) }
}

#[inline(always)]
fn bit(x: &[u64], i: usize) -> bool {
    x.get(i / 64).is_some_and(|w| w >> (i % 64) & 1 == 1)
}

/// The 32 bytes from `p` of the padded source: the bits of byte `a` and of byte `b` (simdjson's `backslash_quote`).
///
/// # Safety
/// The CPU runs `S`'s backend (`run_best` chose it).
#[inline(always)]
unsafe fn marks<S: rerail_simd8::Simd8x64>(src: &[u8], p: usize, a: u8, b: u8) -> (u32, u32) {
    let block: &[u8; 32] = src[p..p + 32].try_into().expect("the padded source");
    // SAFETY: the caller's.
    unsafe { S::marks32(block, a, b) }
}

/// Whether byte `b` is of row `r` (its number, one past the row's): a byte a token of the row runs over.
#[inline(always)]
fn in_row(b: u8, r: u8) -> bool {
    ROWS[usize::from(b)] == r
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
        // Tokens tier 1 decides (their marks, the comments, the flags, the readings): the records form.
        return records::tier2(x);
    }
    let readers = crate::bitlex_lex_rust::readers(x.lexer);
    let mut ends = BTreeMap::new();
    let mut row_of: [Option<u8>; 256] = [None; 256];
    let mut checked = Vec::new();
    let mut checks = Vec::new();
    for s in x.lexer {
        match s {
            Stmt::Token { kind, start, rule, last, .. } => {
                let end = match (rule, last) {
                    (Some(TokenRule::Inside(..)), _) => continue,
                    (Some(TokenRule::Run(_)), _) => End::Run,
                    (Some(TokenRule::Pair(Expr::Empty)), _) | (None, None) => End::One,
                    (Some(TokenRule::Pair(_)), _) => End::Pair,
                    (None, Some(_)) => End::Last,
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
    // A run's end: the bytes it may end before, as tier 1's records allow, one test (`ends`) where they are rows.
    let run_ends = x.lexer.iter().find_map(|s| match s {
        Stmt::Token { kind, rule: Some(::bitlex::syntax::TokenRule::Run(r)), .. } => Some((kind.clone(), r.clone())),
        _ => None,
    });
    let run_ends = run_ends.and_then(|(kind, r)| {
        let (rk, _) = *ends.get(&kind)?;
        let ok = crate::bitlex_lex_rust::ends_after_run(x.lexer, &r)?;
        let row = |b: usize| row_of[b].map_or(0, |r| u16::from(r) + 1);
        let mut rows: Vec<u16> = (0..256).filter(|&b| ok[b]).map(row).collect();
        rows.sort_unstable();
        rows.dedup();
        if !(0..256).all(|b| ok[b] == rows.contains(&row(b))) || rows.contains(&(u16::from(rk) + 1)) {
            return None;
        }
        let pat = rows.iter().map(u16::to_string).collect::<Vec<_>>().join(" | ");
        Some((rk, format!("/// Whether a `{kind}` token, and a word of its row, may end before byte `b`: a byte no token starts at, or one a
/// record starts at after it (simdjson's `is_not_structural_or_whitespace` false); at any other the file is refused.
#[inline(always)]
fn ends(b: u8) -> bool {{
matches!(ROWS[usize::from(b)], {pat})
}}

")))
    });
    let lp = Lp { g: &g, places: RefCell::new(Vec::new()), entries: RefCell::new(Vec::new()), levels: RefCell::new(BTreeMap::new()), ends, readers, checked, row_of, root_end: String::new(), root_place: std::cell::Cell::new(None), methods: RefCell::new(BTreeMap::new()), method_code: RefCell::new(Vec::new()), making: RefCell::new(Vec::new()), placed: RefCell::new(BTreeMap::new()), end_unread: std::cell::Cell::new(false), check_of: checks.iter().cloned().collect(), direct: std::cell::Cell::new(false), at_root: std::cell::Cell::new(false), run_ends, deciding: RefCell::new(std::collections::BTreeSet::new()) };
    // The root: its node, its body, then the stream's end. It ends at the file's end, as tree-sitter's root does.
    let root = *g.order.first().ok_or("tier 2 as one loop: no rule")?;
    let (kind, body) = match g.rules[root] {
        Stmt::Node { kind, body, nested: false, whole: false, .. } => (kind, body),
        _ => return Err("tier 2 as one loop: a root other than a node of no nesting is not written yet".into()),
    };
    let first = lp.place(String::new());
    let number = lp.number(kind, &Key::default())?;
    let end = "close(d, len as u64);\nif i != count {\nreturn Err(at(src, starts, i));\n}\nreturn Ok(());\n".to_owned();
    let mut lp = lp;
    lp.root_end = end.clone();
    lp.at_root.set(true);
    let code = format!("open(d, {number}, at(src, starts, i) as u64);\n{}", lp.walk(body, &Key::default(), K::Code(end), T::No)?);
    lp.at_root.set(false);
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
    let mut check_fns = lp.run_ends.as_ref().map(|(_, c)| c.clone()).unwrap_or_default();
    for (k, b) in &checks {
        check_fns.push_str(&lp.check_fn(k, b)?);
        // A run's end, its check's (the lean parser's `number_end`): written once, read at each place.
        if let Ok((rk, End::Run)) = lp.token(k) {
            let after = match &lp.run_ends {
                Some((r, _)) if *r == rk => "ends(src[s + j])".to_owned(),
                _ => format!("!in_row(src[s + j], {})", rk + 1),
            };
            let _ = write!(check_fns, "/// A `{k}`'s end from its first byte at `s`: its check's match, the byte after it one it may end before (the lean\n/// parser's `number_end`), or the byte it is refused at.\n#[inline(always)]\nfn {name}_end(src: &[u8], s: usize) -> Result<usize, u32> {{\nmatch check_{name}(&src[s..]) {{\nSome(j) if {after} => Ok(s + j),\n_ => Err(s as u32),\n}}\n}}\n\n", name = ident(k));
        }
    }
    let places = lp.places.borrow();
    // A place that takes a token and goes on to a list's loop is one arm with it, as `walk`'s `ObjectBegin |
    // ObjectKey`: its code first where it is the place, then the loop.
    let mut into: BTreeMap<usize, usize> = BTreeMap::new();
    for (l, code) in places.iter().enumerate() {
        let Some(prefix) = code.strip_suffix(";\ncontinue 'run;\n").and_then(|c| c.rsplit_once("st = ")) else { continue };
        let Ok(b) = prefix.1.parse::<usize>() else { continue };
        let straight = !prefix.0.contains("continue 'run") && !prefix.0.contains("return ") && !prefix.0.contains("match ");
        if straight && b != l && places[b].starts_with("// The list:") && !into.values().any(|&x| x == b) {
            into.insert(l, b);
        }
    }
    // The places a level's call returns to are one arm, a loop of its own (`walk`'s `scope_end`): a level that ends
    // there returns to another of them without passing the run's match, so a run of closers stays in the loop.
    const RETURN: &str = "st = ret[depth];\ncontinue 'run;\n";
    let mut rets: Vec<usize> = Vec::new();
    for code in places.iter() {
        for (k, _) in code.match_indices("ret[depth] = ") {
            let n: String = code[k + 13..].chars().take_while(char::is_ascii_digit).collect();
            if let Ok(n) = n.parse::<usize>()
                && !rets.contains(&n)
            {
                rets.push(n);
            }
        }
    }
    rets.sort_unstable();
    if rets.len() < 2 || rets.iter().any(|r| into.contains_key(r) || into.values().any(|b| b == r)) {
        rets.clear();
    }
    let mut arms = String::new();
    for (l, code) in places.iter().enumerate() {
        if into.contains_key(&l) || rets.contains(&l) && l != rets[0] {
            continue;
        }
        if rets.first() == Some(&l) {
            let pats = rets.iter().map(usize::to_string).collect::<Vec<_>>().join(" | ");
            let mut inner = String::new();
            for &r in &rets {
                let _ = write!(inner, "{r} => {{\n{}}}\n", places[r].replace(RETURN, "st = ret[depth];\ncontinue 'ret;\n"));
            }
            let _ = write!(arms, "{pats} => 'ret: loop {{\n// Where a level returns: its caller's rest, by the place it pushed.\nmatch st {{\n{inner}_ => unreachable!(),\n}}\n}},\n");
            continue;
        }
        match into.iter().find(|(_, b)| **b == l) {
            Some((&a, _)) => {
                let prefix = places[a].rsplit_once("st = ").expect("its jump").0;
                let _ = write!(arms, "{a} | {l} => {{\nif st == {a} {{\n{prefix}}}\n{code}}}\n");
            }
            None => {
                let _ = write!(arms, "{l} => {{\n{code}}}\n");
            }
        }
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
    out.push_str("// Generated by native/crates/compiler/src/bitlex_rust/one_loop.rs; do not edit. The Bitlex compiler's Rust from the grammar's tree rules, as one loop.\n\n");
    out.push_str("use rerail_tape_format::f16::Writer;\nuse rerail_tape_format::writer16::{Cursor, Fixed};\n\n");
    // The language's widths (slots, gaps, lengths) are the parser's: a value past its width is its side list's, as
    // every format 16 tape's. One byte each, the implementer's choice.
    out.push_str("/// The language's cell widths, slots, gaps and lengths: the writer's language is to have these (`Lang::new`).\npub const WIDTHS: [usize; 3] = [1, 1, 1];\n\n/// The writer's cells at those widths, constants in the ops.\ntype Cells = Fixed<1, 1, 1>;\n\n");
    let _ = write!(out, "/// The code table the numbers below are of: each number's kind, field and supertypes.\npub const ENTRIES: &[(u16, u16, &[u16])] = &[\n{entries}];\n\n");
    out.push_str("/// The levels open at most: past it, the file is refused, as the JIT's calls (`native_cpu_stream::DEPTH`).\nconst DEPTH: usize = 1024;\n");
    // Each byte's row, one past its number (0: none): what `in_row` reads.
    let rows: Vec<String> = lp.row_of.iter().map(|r| r.map_or(0, |r| u16::from(r) + 1).to_string()).collect();
    let _ = write!(out, "\n/// Each byte's token row, one past its number (0: none), as simdjson's structural-or-whitespace table.\nstatic ROWS: [u8; 256] = [{}];\n", rows.join(", "));
    out.push_str(RUNTIME);

    out.push('\n');
    out.push_str(&words);
    out.push_str(&check_fns);
    for m in lp.method_code.borrow().iter() {
        out.push_str(m);
    }
    let _ = write!(
        out,
        "/// `src`'s tape into `w` (`src` the file padded, `lx.len` its length), from tier 1's records (each token's first byte) and the bytes, or the byte it was\n/// refused at.\n/// The run's room is reserved once: a node a record, or a byte in a region's pieces, and the root; the levels' nodes\n/// open at once ({frames} a level), and the root's.\npub fn parse(src: &[u8], lx: &crate::tier1::Lexed, w: &mut Writer) -> Result<(), u32> {{\n// Where tier 1 sends the file to tree-sitter.\nif lx.refused {{\nreturn Err(0);\n}}\n// A file with no token: simdjson's `EMPTY`.\nif lx.count == 0 {{\nreturn Err(lx.len as u32);\n}}\nw.reserve_run(1 + lx.count + lx.len, (DEPTH + 1) * {frames} + 1);\n// Each node final as written: no commit after each.\nw.final_as_written();\nlet d = w.state_address();\nrun_best(src, lx, d)?;\nmatch w.failure() {{\nNone => Ok(()),\nSome(_) => Err(u32::MAX),\n}}\n}}\n\n/// `parse`'s walk, on the writer's state `d`, its byte reads on `S`'s backend (chosen once, `run_best`).\n///\n/// # Safety\n/// The CPU runs `S`'s backend.\n#[inline(always)]\nunsafe fn run<S: rerail_simd8::Simd8x64>(src: &[u8], lx: &crate::tier1::Lexed, state: *mut u8) -> Result<(), u32> {{\n// The walk's hold on the tape, its own while it runs (the lean parser's `Walk`): the nodes stored through it.\n// SAFETY: `state` is the writer's, its room reserved (`parse`).\nlet mut cursor = unsafe {{ Cursor::<Cells>::begin(state) }};\nlet d = &mut cursor;\n// The records, then the index's three last entries; their count; the file's length (`src` is it padded).\nlet (starts, count, len) = (&lx.starts[..], lx.count, lx.len);\n// The next record, the last token's end, by depth the place each open level returns to (the lean parser's `object[d]`), the levels open, the place.\nlet mut i = 0usize;\nlet mut e = 0usize;\nlet mut ret: [{ty}; DEPTH] = [0; DEPTH];\nlet mut depth = 0usize;\nlet mut st: {ty} = 0;\n'run: loop {{\nmatch st {{\n{arms}_ => unreachable!(),\n}}\n}}\n}}\n\nrerail_simd8::dispatch! {{\n    fn run_best(src: &[u8], lx: &crate::tier1::Lexed, state: *mut u8) -> Result<(), u32> = run, on run_on;\n}}\n"
    );
    let _ = &x.tokens;
    // The parser's own code, but for what nothing in it calls.
    let parts: Vec<String> = RUNTIME.split("\n\n").chain(words.split("\n\n")).map(|p| p.trim_matches('\n').to_owned()).filter(|p| !p.is_empty()).collect();
    // The methods take the file's length only where one reads it (`walk`'s `string` takes none).
    let (sig, call) = ("src: &[u8], len: usize, starts: &[u32]", "(src, len, starts, d, &mut i, [");
    let methods: String = lp.method_code.borrow().concat();
    if !reads_ident(&methods.replace(sig, "").replace(call, ""), "len") {
        out = out.replace(sig, "src: &[u8], starts: &[u32]").replace(call, "(src, starts, d, &mut i, [");
    }
    Ok(drop_uncalled(out, &parts))
}

/// Whether `code` names `id` (not as part of a longer name, a field or a method).
fn reads_ident(code: &str, id: &str) -> bool {
    let b = code.as_bytes();
    code.match_indices(id).any(|(k, _)| {
        let before = k.checked_sub(1).map(|j| b[j]);
        let after = b.get(k + id.len()).copied();
        let ident = |c: Option<u8>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_');
        !ident(before) && before != Some(b'.') && !ident(after)
    })
}

/// `out` without each of `parts` (a function and its comments, as written in `out`) that nothing else in `out` calls.
pub(super) fn drop_uncalled(mut out: String, parts: &[String]) -> String {
    let name = |p: &str| p.lines().find_map(|l| l.strip_prefix("fn ").or_else(|| l.strip_prefix("unsafe fn ")).or_else(|| l.strip_prefix("    fn "))).and_then(|l| l.split(['(', '<']).next()).map(str::to_owned);
    loop {
        let mut dropped = false;
        for p in parts {
            let Some(n) = name(p) else { continue };
            let Some(at) = out.find(p.as_str()) else { continue };
            let without = format!("{}{}", &out[..at], &out[at + p.len()..]);
            let called = without.lines().filter(|l| !l.trim_start().starts_with("//")).any(|l| l.contains(&format!("{n}(")) || l.contains(&format!("{n}::<")) || l.contains(&format!("{n},")) || l.contains(&format!("= {n}")));
            if !called {
                out = without;
                dropped = true;
            }
        }
        if !dropped {
            return out.replace("\n\n\n\n", "\n\n").replace("\n\n\n", "\n\n");
        }
    }
}
