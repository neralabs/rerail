//! Tier 2 as Rust, written by the Bitlex compiler from the grammar's tree rules: a recursive
//! descent parser in the lean parsers' shape (`rerail_kotlin_lean::parse`), never lowered from an
//! intermediate form.
//!
//! - **A function a rule.** `node K = …` opens its node at its first token and closes it at its
//!   end (`self.open(k::K)`, `self.close()`); `choice N = …` makes no node. A sequence is its
//!   elements one after the other; a token is taken (`tk`, a leaf) or passed (`skip`, a token the
//!   grammar implies) where the rule says it.
//! - **Decisions at the token.** `[A]`, `A*`, `A+` and `A | B` decide at the next token, as the
//!   lean parser's `if self.peek() == …`: the marks first, in the order they are declared (a mark
//!   holds before its token), then the words, then the kinds. Where the rule may end there, a
//!   mark that ends a level (`, ends`) ends it, as the lean parser's `ends()` asks `asi`.
//! - **Token marks** are lookahead functions written from their patterns, as the lean parser's
//!   `lambda_ahead` and `type_args_end`; a mark of tier 1's is a bit of the token's code.
//! - **The tree** is written as the lean parser's `Tw` writes it (`rerail_tape_format::Writer`):
//!   the comments before a token written into the node open when the next node or token is, a
//!   node put over the one just made (`wrap`) by `Writer::insert`.

use ::bitlex::syntax::{Gram, Stmt, TokenElem};

// Mounted by `rerail-bitlex` from the compiler's folder (`#[path]`): its submodule is found beside it.
#[path = "bitlex_rust/one_loop.rs"]
mod one_loop;
pub use one_loop::{tier2_loop, tier2_loop_reads};
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

/// A mark, as the generated code tests it.
pub struct Mark<'a> {
    pub name: String,
    /// Its bit in a record's code (`16 + bit`): a mark tier 1 makes.
    pub bit: Option<u8>,
    /// The word the token must be too (0: any), with the bit.
    pub word: u16,
    /// The file's end has it.
    pub end: bool,
    /// A token mark's tokens: the token before (`after`), then the run.
    pub tokens: Option<(&'a [TokenElem], &'a [Vec<TokenElem>])>,
    /// Its `before`s and `unless before`s.
    pub next: &'a [(bool, Vec<Vec<TokenElem>>)],
    /// Its `unless after` words.
    pub unless_after: &'a [Vec<u8>],
    pub ends: bool,
    pub settles: bool,
    /// Its questions (`, drops …`, `, loses …`, `, refuses …`, `, refuses taken …`, decision 0451).
    pub drops: &'a [::bitlex::syntax::GapPattern],
    pub loses: &'a [::bitlex::syntax::GapPattern],
    pub refuses: &'a [::bitlex::syntax::GapPattern],
    pub refuses_taken: &'a [::bitlex::syntax::GapPattern],
    pub hidden: bool,
    /// `, passes M`: the marks after it that taking it passes too, a bit each by order.
    pub passes: u64,
}

/// What the generator reads: the tree's rules and the compiler's tables.
pub struct Input<'a> {
    pub tree: &'a [Stmt],
    /// A node kind's id (named first, then anonymous).
    pub kind: &'a dyn Fn(&str) -> Option<u16>,
    pub field: &'a dyn Fn(&str) -> Option<u16>,
    /// The records' kinds: each kind byte and its token's name.
    pub tokens: &'a [(u8, String)],
    /// The words a record's code names: word `i + 1` is `words[i]`.
    pub words: &'a [Vec<u8>],
    /// Each keyword row: its kind, its hard words (with their kinds' names), whether every token is one.
    pub keywords: &'a [(String, Vec<(Vec<u8>, Option<String>)>, bool)],
    pub marks: Vec<Mark<'a>>,
    pub extras: &'a [String],
    /// The marks that tell an extra's record (`Records::extra`): the first, of the first extra
    /// kind, then each later kind's own bit.
    pub extra_bits: Vec<(u8, String)>,
    /// Each row's soft keywords (`Lowering::soft`).
    pub soft: &'a [(String, Vec<Vec<u8>>)],
    /// The lexer's statements: the regions tier 2 reads in parts (`bitlex_lex_rust::readers`).
    pub lexer: &'a [Stmt],
    /// Each token row's own record kind, where its start gives it (`bitlex_lex_rust::Input`).
    pub row_kinds: &'a [Option<u8>],
    /// Each byte's record kind as a token's first byte (`Records::kinds`).
    pub kinds: &'a [u8],
}

/// What a decision tests at the token.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Test {
    /// A mark, by its order.
    Mark(usize),
    Word(u8),
    /// A token kind, by its name.
    Kind(String),
}

/// A test, and the flag that must be clear for it (`unless F`).
type First = (Test, Option<String>);

struct G<'a> {
    x: &'a Input<'a>,
    rules: HashMap<&'a str, &'a Stmt>,
    order: Vec<&'a str>,
    flags: Vec<String>,
    readings: HashMap<&'a str, &'a ::bitlex::syntax::Reading>,
    /// The `remember` choices: a token one takes is its level's word (`P::word`), the kind a wrap's, relabel's or
    /// reading's `as C` names.
    remember: Vec<String>,
    /// The `provisional` fields: a node that holds one at the file's end refuses it.
    provisional: Vec<String>,
    /// `refuse M in L …`: each level's refused marks, their bits in a record's code: a token the level takes with one
    /// refuses the file (`P::refusing`).
    refusing: HashMap<String, u32>,
    /// The mark bits the code written so far reads of a record's code (`16 + bit`): tier 1 writes only these where its
    /// loop reads the bytes (decision 0623).
    reads: std::cell::Cell<u32>,
    /// The field ids past a key's 6 bits, each given the slot `63 - its place` (`field_id`).
    own_fields: std::cell::RefCell<Vec<u16>>,
    /// The largest field id under 64 named: the slots must be above it.
    most_field: std::cell::Cell<u16>,
    firsts: std::cell::RefCell<HashMap<String, (Vec<First>, bool)>>,
    busy: std::cell::RefCell<HashSet<String>>,
    /// The kinds the code names, by their constant's name; the fields; the words.
    kinds: std::cell::RefCell<std::collections::BTreeMap<String, u16>>,
    fields: std::cell::RefCell<std::collections::BTreeMap<String, u16>>,
    /// The token kinds the code tests (`at_K`).
    kind_tests: std::cell::RefCell<std::collections::BTreeSet<String>>,
    /// Each rule's follow (`follow`), once found.
    follows: std::cell::RefCell<Option<HashMap<String, Vec<usize>>>>,
}

/// `s`'s lines at `p`.
fn indent(s: &str, p: &str) -> String {
    let mut out = String::new();
    for l in s.lines() {
        if !l.is_empty() {
            out.push_str(p);
        }
        out.push_str(l);
        out.push('\n');
    }
    out
}

/// A name as a Rust identifier: letters, digits and `_`, a punctuation's name spelled out.
fn ident(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        let w = match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' => {
                out.push(c);
                continue;
            }
            '.' => "dot",
            ',' => "comma",
            ':' => "colon",
            ';' => "semi",
            '(' => "lparen",
            ')' => "rparen",
            '{' => "lbrace",
            '}' => "rbrace",
            '[' => "lbrack",
            ']' => "rbrack",
            '<' => "lt",
            '>' => "gt",
            '=' => "eq",
            '+' => "plus",
            '-' => "minus",
            '*' => "star",
            '/' => "slash",
            '%' => "percent",
            '!' => "bang",
            '?' => "quest",
            '&' => "amp",
            '|' => "bar",
            '@' => "at",
            '#' => "hash",
            '$' => "dollar",
            '\\' => "backslash",
            '"' => "quote",
            '\'' => "tick",
            '^' => "caret",
            '~' => "tilde",
            '`' => "backtick",
            _ => "x",
        };
        if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
        out.push_str(w);
        out.push('_');
    }
    let out = out.trim_end_matches('_').to_owned();
    if out.is_empty() || out.as_bytes()[0].is_ascii_digit() { format!("x{out}") } else { out }
}

fn upper(name: &str) -> String {
    ident(name).to_ascii_uppercase()
}

/// A rule's function name: the rule's, apart from Rust's keywords and the runtime's own.
fn rule_fn(name: &str) -> String {
    let n = ident(name);
    match n.as_str() {
        "type" | "fn" | "if" | "else" | "match" | "loop" | "while" | "for" | "in" | "as" | "use" | "mod" | "struct" | "enum" | "impl" | "trait" | "where" | "return" | "break" | "continue" | "self" | "super" | "crate" | "let" | "mut" | "ref" | "move" | "static" | "const" | "true" | "false" | "dyn" | "box" | "do" | "try" | "async" | "await" | "abstract" | "final" | "override" | "virtual" | "yield" | "macro" | "priv" | "typeof" | "unsized" | "become" | "open" | "close" | "block" | "flush" | "bump" | "leaf" | "tk" | "skip" | "wrap" | "word" | "kind" | "start" | "statement" | "expression" => format!("r_{n}"),
        _ => n,
    }
}

impl<'a> G<'a> {
    fn new(x: &'a Input<'a>) -> Self {
        let mut rules = HashMap::new();
        let mut order = Vec::new();
        let mut flags = Vec::new();
        let mut readings = HashMap::new();
        let (mut remember, mut provisional) = (Vec::new(), Vec::new());
        let mut refusing: HashMap<String, u32> = HashMap::new();
        for s in x.tree {
            match s {
                Stmt::Node { kind, .. } => {
                    rules.insert(kind.as_str(), s);
                    order.push(kind.as_str());
                }
                Stmt::Choice { name, .. } => {
                    rules.insert(name.as_str(), s);
                    order.push(name.as_str());
                }
                Stmt::Flags(f) => flags.extend(f.iter().cloned()),
                Stmt::Reading { name, reading } => {
                    readings.insert(name.as_str(), reading);
                }
                Stmt::Remember(n) => remember.extend(n.iter().cloned()),
                Stmt::Provisional(f) => provisional.extend(f.iter().cloned()),
                // A mark tier 1 gives no bit is refused nowhere here: `source` refuses the grammar.
                Stmt::Refuse { mark, levels } => {
                    let bit = x.marks.iter().find(|m| &m.name == mark).and_then(|m| m.bit).map_or(0, |b| 1u32 << b);
                    for l in levels {
                        *refusing.entry(l.clone()).or_default() |= if bit == 0 { u32::MAX } else { bit };
                    }
                }
                _ => {}
            }
        }
        let reads = refusing.values().filter(|&&b| b != u32::MAX).fold(0, |a, &b| a | b) | x.extra_bits.iter().fold(0, |a, (b, _)| a | 1 << b);
        G { x, rules, order, flags, readings, remember, provisional, refusing, reads: std::cell::Cell::new(reads), own_fields: Default::default(), most_field: Default::default(), firsts: Default::default(), busy: Default::default(), kinds: Default::default(), fields: Default::default(), kind_tests: Default::default(), follows: Default::default() }
    }

    fn mark_of(&self, name: &str) -> Option<usize> {
        self.x.marks.iter().position(|m| m.name == name)
    }

    fn word_of(&self, text: &str) -> Option<u8> {
        self.x.words.iter().position(|w| w.as_slice() == text.as_bytes()).map(|i| (i + 1) as u8)
    }

    /// A keyword's kind of one word (`keywords number { "true" -> true }`): that word, a rule's test of the kind.
    fn keyword_word(&self, kind: &str) -> Option<u8> {
        let mut words = self.x.keywords.iter().flat_map(|r| r.1.iter()).filter(|(_, k)| k.as_deref() == Some(kind));
        let (w, _) = words.next()?;
        if words.next().is_some() {
            return None;
        }
        self.word_of(&String::from_utf8_lossy(w))
    }

    fn is_token_kind(&self, name: &str) -> bool {
        self.x.tokens.iter().any(|(_, n)| n == name || n.strip_suffix("_close") == Some(name))
    }

    /// A kind's constant, `k::NAME`, its id named as the code reads it.
    fn kind_const(&self, name: &str) -> Result<String, String> {
        let id = (self.x.kind)(name).ok_or_else(|| format!("no kind `{name}`"))?;
        let c = upper(name);
        if id > 0x3fe {
            return Err(format!("kind `{name}`: no tape kind"));
        }
        let c = if c.is_empty() { format!("K{id}") } else { c };
        let mut kinds = self.kinds.borrow_mut();
        // Two names one constant (`.` and `dot`): the id names the second.
        let c = match kinds.get(&c) {
            Some(&k) if k != id => format!("{c}_{id}"),
            _ => c,
        };
        kinds.insert(c.clone(), id);
        Ok(format!("k::{c}"))
    }

    /// Field `name`'s id in a node's key, which holds 6 bits: the grammar's id, or for one past them (a field the
    /// tape names not, which a reading renames or `provisional` refuses before the file's end: `rr_parser`'s own ids)
    /// a slot from the top of the 6 bits down, the same wherever it is named.
    fn field_id(&self, name: &str) -> Result<u16, String> {
        let id = (self.x.field)(name).ok_or_else(|| format!("no field `{name}`"))?;
        if id < 64 {
            self.most_field.set(self.most_field.get().max(id));
            return Ok(id);
        }
        let mut own = self.own_fields.borrow_mut();
        let k = own.iter().position(|&o| o == id).unwrap_or_else(|| {
            own.push(id);
            own.len() - 1
        });
        Ok(63 - k as u16)
    }

    fn field_const(&self, name: &str) -> Result<String, String> {
        let id = self.field_id(name)?;
        let c = upper(name);
        self.fields.borrow_mut().insert(c.clone(), id);
        Ok(format!("f::{c}"))
    }

    /// The tests a grammar's first token can pass, and whether it may match nothing.
    fn first(&self, g: &Gram) -> Result<(Vec<First>, bool), String> {
        Ok(match g {
            Gram::Lit(t) => match self.word_of(t) {
                Some(w) => (vec![(Test::Word(w), None)], false),
                None if self.is_token_kind(t) => (vec![(Test::Kind(t.clone()), None)], false),
                None => return Err(format!("`{t}`: no word or token of that name")),
            },
            Gram::Sym(n) => {
                if let Some(m) = self.mark_of(n) {
                    (vec![(Test::Mark(m), None)], false)
                } else if self.rules.contains_key(n.as_str()) {
                    self.rule_first(n)?
                } else if self.is_token_kind(n) {
                    (vec![(Test::Kind(n.clone()), None)], false)
                } else if let Some(w) = self.keyword_word(n).or_else(|| self.word_of(n)) {
                    // A keyword named by its word (`any`'s tokens, as the build names them): that word.
                    (vec![(Test::Word(w), None)], false)
                } else {
                    return Err(format!("`{n}`: no rule, mark or token of that name"));
                }
            }
            Gram::Seq(gs) => {
                let mut out = Vec::new();
                for g in gs {
                    let (f, empty) = self.first(g)?;
                    out.extend(f);
                    if !empty {
                        return Ok((out, false));
                    }
                }
                (out, true)
            }
            Gram::Alt(gs) => {
                let mut out = Vec::new();
                let mut empty = false;
                for g in gs {
                    let (f, e) = self.first(g)?;
                    out.extend(f);
                    empty |= e;
                }
                (out, empty)
            }
            Gram::Opt(a) | Gram::Star(a) => (self.first(a)?.0, true),
            Gram::Plus(a) | Gram::Field(_, a) | Gram::Retag(_, a) | Gram::Merge(_, a) | Gram::As(_, a) => self.first(a)?,
            Gram::Wrap { body, .. } | Gram::Reread { body, .. } | Gram::Flag { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } => self.first(body)?,
            Gram::Unless { flag, body } => {
                let (f, e) = self.first(body)?;
                (f.into_iter().map(|(t, _)| (t, Some(flag.clone()))).collect(), e)
            }
            other => return Err(format!("not written yet: {other:?}")),
        })
    }

    fn rule_first(&self, name: &str) -> Result<(Vec<First>, bool), String> {
        if let Some(f) = self.firsts.borrow().get(name) {
            return Ok(f.clone());
        }
        if !self.busy.borrow_mut().insert(name.to_owned()) {
            return Err(format!("rule `{name}`: left-recursive"));
        }
        let body = match self.rules[name] {
            Stmt::Node { body, .. } | Stmt::Choice { body, .. } => body,
            _ => unreachable!(),
        };
        let f = self.first(body)?;
        self.busy.borrow_mut().remove(name);
        self.firsts.borrow_mut().insert(name.to_owned(), f.clone());
        Ok(f)
    }

    fn rest_first(&self, rest: &[&Gram]) -> Result<(Vec<First>, bool), String> {
        let mut out = Vec::new();
        for g in rest {
            let (f, empty) = self.first(g)?;
            out.extend(f);
            if !empty {
                return Ok((out, false));
            }
        }
        Ok((out, true))
    }

    /// A test as a condition on the token.
    fn cond(&self, t: &First) -> String {
        let base = match &t.0 {
            Test::Mark(m) => format!("self.m_{}()", ident(&self.x.marks[*m].name)),
            Test::Word(w) => format!("self.word() == w::{}", self.word_const(*w)),
            Test::Kind(k) => {
                self.kind_tests.borrow_mut().insert(k.clone());
                format!("self.at_{}()", ident(k))
            }
        };
        match &t.1 {
            Some(f) => format!("(self.flags & FLAG_{} == 0 && {base})", upper(f)),
            None => base,
        }
    }

    fn word_const(&self, w: u8) -> String {
        let text = String::from_utf8_lossy(&self.x.words[usize::from(w) - 1]).into_owned();
        format!("{}_{w}", upper(&text))
    }

    /// The code of a decision among `arms` (each its tests and its code, unindented), as the lean
    /// parser decides at a token: the marks first, in their order (a mark is a token of no width
    /// before its token, tier 2's machine sees it first), consecutive ones that go to one arm one
    /// test; then a `match` on the word, a kind's test a guard of its arm. `ex`: marks that
    /// leave the decision (`exit`, the code after it takes them, or the level ends there,
    /// `exits_ending`). None passes: `none`. An arm reached by two tests apart is numbered, its
    /// code written once. Exits where the level ends at a hidden mark that ends levels and has
    /// questions (`gated`) are tested only where the caller's level is the mark's (`asks`):
    /// elsewhere (inside parentheses) the mark is passed, as the lean parser asks `asi` only where
    /// `stmt` is set.
    fn decide(&self, arms: &[(Vec<First>, String)], ex: &[(usize, bool)], unmarked: &[usize], exit: &str, none: &str, pad: &str) -> String {
        let exits: Vec<usize> = ex.iter().map(|e| e.0).collect();
        let gated = self.gated(ex);
        let n = arms.len();
        let mut marks: Vec<(usize, Option<String>, usize)> = Vec::new();
        let mut words: Vec<(u8, Option<String>, usize)> = Vec::new();
        let mut kinds: Vec<(String, Option<String>, usize)> = Vec::new();
        for (i, (fs, _)) in arms.iter().enumerate() {
            for (t, f) in fs {
                match t {
                    Test::Mark(m) if !marks.iter().any(|x| x.0 == *m && x.1 == *f) => marks.push((*m, f.clone(), i)),
                    Test::Word(w) if !words.iter().any(|x| x.0 == *w && x.1 == *f) => words.push((*w, f.clone(), i)),
                    Test::Kind(k) if !kinds.iter().any(|x| x.0 == *k && x.1 == *f) => kinds.push((k.clone(), f.clone(), i)),
                    _ => {}
                }
            }
        }
        for &m in &exits {
            if !marks.iter().any(|x| x.0 == m) {
                marks.push((m, None, n));
            }
        }
        marks.sort_by_key(|x| x.0);
        let guard = |f: &Option<String>, c: String| match f {
            Some(f) => format!("self.flags & FLAG_{} == 0 && {c}", upper(f)),
            None => c,
        };
        let mut mg: Vec<(Vec<String>, usize)> = Vec::new();
        for (m, f, t) in &marks {
            let c = guard(f, format!("self.m_{}()", ident(&self.x.marks[*m].name)));
            let c = if *t == n && unmarked.contains(m) { format!("!self.marked && {c}") } else { c };
            let c = if *t == n && gated.contains(m) { format!("self.asks >> {m} & 1 != 0 && {c}") } else { c };
            match mg.last_mut() {
                Some((cs, tt)) if tt == t => cs.push(c),
                _ => mg.push((vec![c], *t)),
            }
        }
        let mut wg: Vec<(String, usize)> = Vec::new();
        let mut order: Vec<usize> = Vec::new();
        for (_, f, t) in &words {
            if f.is_none() && !order.contains(t) {
                order.push(*t);
            }
        }
        for t in order {
            let pats: Vec<String> = words.iter().filter(|x| x.1.is_none() && x.2 == t).map(|x| format!("w::{}", self.word_const(x.0))).collect();
            wg.push((pats.join(" | "), t));
        }
        for (w, f, t) in words.iter().filter(|x| x.1.is_some()) {
            wg.push((format!("w::{} if self.flags & FLAG_{} == 0", self.word_const(*w), upper(f.as_deref().unwrap_or_default())), *t));
        }
        let mut kg: Vec<(Vec<String>, usize)> = Vec::new();
        for (k, f, t) in &kinds {
            self.kind_tests.borrow_mut().insert(k.clone());
            let c = guard(f, format!("self.at_{}()", ident(k)));
            match kg.last_mut() {
                Some((cs, tt)) if tt == t => cs.push(c),
                _ => kg.push((vec![c], *t)),
            }
        }
        let body_of = |t: usize| if t == n { exit.to_owned() } else { arms[t].1.clone() };
        let mut uses = vec![0usize; n + 1];
        for t in mg.iter().map(|g| g.1).chain(wg.iter().map(|g| g.1)).chain(kg.iter().map(|g| g.1)) {
            uses[t] += 1;
        }
        let numbered = (0..=n).any(|t| uses[t] > 1 && body_of(t).lines().count() > 1);
        const NONE_ARM: usize = usize::MAX;
        let leaf = |t: usize, p: &str| if numbered { format!("{p}arm = {};\n", if t == NONE_ARM { "usize::MAX".to_owned() } else { t.to_string() }) } else { indent(&if t == NONE_ARM { none.to_owned() } else { body_of(t) }, p) };
        // The word's match and the kinds' tests, at `p`.
        let rest = |p: &str| -> String {
            let mut out = String::new();
            let q = format!("{p}    ");
            let r = format!("{p}        ");
            if !wg.is_empty() {
                let _ = writeln!(out, "{p}match self.word() {{");
                for (pat, t) in &wg {
                    let _ = write!(out, "{q}{pat} => {{\n{}{q}}}\n", leaf(*t, &r));
                }
                for (cs, t) in &kg {
                    let _ = write!(out, "{q}_ if {} => {{\n{}{q}}}\n", cs.join(" || "), leaf(*t, &r));
                }
                let _ = write!(out, "{q}_ => {{\n{}{q}}}\n{p}}}\n", leaf(NONE_ARM, &r));
            } else if !kg.is_empty() {
                for (i, (cs, t)) in kg.iter().enumerate() {
                    let _ = write!(out, "{}if {} {{\n{}{p}}}", if i == 0 { p.to_owned() } else { " else ".to_owned() }, cs.join(" || "), leaf(*t, &q));
                }
                let _ = write!(out, " else {{\n{}{p}}}\n", leaf(NONE_ARM, &q));
            } else {
                out.push_str(&leaf(NONE_ARM, p));
            }
            out
        };
        let mut sel = String::new();
        if mg.is_empty() {
            sel = rest(pad);
        } else {
            let q = format!("{pad}    ");
            for (i, (cs, t)) in mg.iter().enumerate() {
                let _ = write!(sel, "{}if {} {{\n{}{pad}}}", if i == 0 { pad.to_owned() } else { " else ".to_owned() }, cs.join(" || "), leaf(*t, &q));
            }
            let _ = write!(sel, " else {{\n{}{pad}}}\n", rest(&q));
        }
        if !numbered {
            return sel;
        }
        let mut out = format!("{pad}let arm: usize;\n{sel}{pad}match arm {{\n");
        let (q, r) = (format!("{pad}    "), format!("{pad}        "));
        for (t, _) in uses.iter().enumerate().filter(|&(_, &u)| u > 0) {
            let _ = write!(out, "{q}{t} => {{\n{}{q}}}\n", indent(&body_of(t), &r));
        }
        let _ = write!(out, "{q}_ => {{\n{}{q}}}\n{pad}}}\n", indent(none, &r));
        out
    }

    /// The alternatives of `g`: an alternation's, flattened; else `g`.
    fn alts<'g>(g: &'g Gram, out: &mut Vec<&'g Gram>) {
        match g {
            Gram::Alt(gs) => gs.iter().for_each(|a| Self::alts(a, out)),
            _ => out.push(g),
        }
    }

    /// The arms of a decision among `g`'s alternatives, each before `rest`: its tests, its code
    /// (an `unless F` alternative's guard is its tests'), and whether it may match nothing.
    fn arms(&self, g: &Gram, rest: &[&Gram], rule: &str) -> Result<Vec<(Vec<First>, String, bool)>, String> {
        let mut alts = Vec::new();
        Self::alts(g, &mut alts);
        let mut out = Vec::new();
        for a in alts {
            let (f, e) = self.first(a)?;
            let body = match a {
                Gram::Unless { body, .. } => self.code(body, rest, rule, "")?,
                _ => self.code(a, rest, rule, "")?,
            };
            out.push((f, body, e));
        }
        Ok(out)
    }

    /// The marks after a decision, before `rest` in `rule`: the rest's first, and where the
    /// rule may end there, its follow (its level's, a rule inlined in it), or the marks that end
    /// a level (`, ends`) where the rule is a level of its own.
    /// Each with whether the level ends there (a mark that ends a level, `, ends`, at a
    /// rule that is one) rather than taking it.
    fn exits_ending(&self, rest: &[&Gram], rule: &str) -> Result<Vec<(usize, bool)>, String> {
        let (fr, may_end) = self.rest_first(rest)?;
        let mut out: Vec<(usize, bool)> = fr.iter().filter_map(|t| if let Test::Mark(m) = t.0 { Some((m, false)) } else { None }).collect();
        if may_end {
            let end = self.nested(rule);
            out.extend(self.follow(rule)?.into_iter().map(|m| (m, end)));
        }
        out.sort_unstable();
        out.dedup_by_key(|e| e.0);
        Ok(out)
    }

    /// The exits where the level ends at a hidden mark that ends levels (`, ends`): not tested
    /// right after a mark is taken (`marked`), as the reference's state entered by mark arms
    /// alone (`after_mark_only`) does not end there.
    fn after_mark(&self, rest: &[&Gram], rule: &str) -> Result<Vec<usize>, String> {
        let (fr, may_end) = self.rest_first(rest)?;
        if !may_end {
            return Ok(Vec::new());
        }
        let first: Vec<usize> = fr.iter().filter_map(|t| if let Test::Mark(m) = t.0 { Some(m) } else { None }).collect();
        let ending = self.ending();
        Ok(self.follow(rule)?.into_iter().filter(|m| !first.contains(m) && ending.contains(m) && self.x.marks[*m].hidden).collect())
    }

    /// The exits of `ex` where the level ends at a hidden mark that ends levels and has
    /// questions: those `decide` tests only where `asks` has the mark (`gated`).
    fn gated(&self, ex: &[(usize, bool)]) -> Vec<usize> {
        let ending = self.ending();
        ex.iter().filter(|e| e.1 && ending.contains(&e.0) && self.x.marks[e.0].hidden && self.asks(e.0)).map(|e| e.0).collect()
    }

    /// Whether mark `m`'s name has questions a decision asks (`refuses`, `loses`).
    fn asks(&self, m: usize) -> bool {
        let name = &self.x.marks[m].name;
        self.x.marks.iter().any(|x| &x.name == name && (!x.refuses.is_empty() || !x.loses.is_empty()))
    }

    /// The questions a decision asks before its tests: each mark it tests that has some, at an
    /// arm, or where the level may end at it (`end`).
    fn asking(&self, arms: &[(Vec<First>, String)], exits: &[(usize, bool)], pad: &str) -> String {
        let mut asked: Vec<(usize, bool)> = Vec::new();
        for (fs, _) in arms {
            for (t, _) in fs {
                if let Test::Mark(m) = t
                    && !asked.iter().any(|a| a.0 == *m)
                {
                    asked.push((*m, false));
                }
            }
        }
        for &(m, end) in exits {
            if !asked.iter().any(|a| a.0 == m) {
                asked.push((m, end));
            }
        }
        asked.sort_unstable();
        let mut out = String::new();
        for (m, end) in asked.into_iter().filter(|a| self.asks(a.0)) {
            let _ = writeln!(out, "{pad}self.ask_{}({end})?;", ident(&self.x.marks[m].name));
        }
        out
    }

    fn nested(&self, rule: &str) -> bool {
        matches!(self.rules.get(rule), Some(Stmt::Node { nested: true, .. } | Stmt::Choice { nested: true, .. })) || self.order.first() == Some(&rule)
    }

    /// The marks that end a level (`, ends`), by their order.
    fn ending(&self) -> Vec<usize> {
        (0..self.x.marks.len()).filter(|&m| self.x.marks[m].ends && self.mark_of(&self.x.marks[m].name) == Some(m)).collect()
    }

    /// The marks after `rule` where it ends: those of its level, after each call of it.
    fn follow(&self, rule: &str) -> Result<Vec<usize>, String> {
        if self.nested(rule) {
            return Ok(if self.order.first() == Some(&rule) { Vec::new() } else { self.ending() });
        }
        if self.follows.borrow().is_none() {
            let mut sites = Vec::new();
            for name in &self.order {
                let body = match self.rules[name] {
                    Stmt::Node { body, .. } | Stmt::Choice { body, .. } => body,
                    _ => unreachable!(),
                };
                self.sites(body, &[], name, &mut sites)?;
            }
            let mut follows: HashMap<String, Vec<usize>> = HashMap::new();
            loop {
                let mut changed = false;
                for (callee, marks, may_end, caller) in &sites {
                    let mut add = marks.clone();
                    if *may_end {
                        if self.nested(caller) {
                            if self.order.first() != Some(&caller.as_str()) {
                                add.extend(self.ending());
                            }
                        } else {
                            add.extend(follows.get(caller).cloned().unwrap_or_default());
                        }
                    }
                    let f = follows.entry(callee.clone()).or_default();
                    for m in add {
                        if !f.contains(&m) {
                            f.push(m);
                            changed = true;
                        }
                    }
                }
                if !changed {
                    break;
                }
            }
            *self.follows.borrow_mut() = Some(follows);
        }
        let mut f = self.follows.borrow().as_ref().and_then(|f| f.get(rule).cloned()).unwrap_or_default();
        f.sort_unstable();
        Ok(f)
    }

    /// Each call of a rule in `g` before `rest`: the callee, the marks first after it, whether
    /// the caller may end there, the caller.
    fn sites(&self, g: &Gram, rest: &[&Gram], caller: &str, out: &mut Vec<(String, Vec<usize>, bool, String)>) -> Result<(), String> {
        match g {
            Gram::Sym(n) if self.mark_of(n).is_none() && self.rules.contains_key(n.as_str()) => {
                let (fr, may_end) = self.rest_first(rest)?;
                let marks = fr.iter().filter_map(|t| if let Test::Mark(m) = t.0 { Some(m) } else { None }).collect();
                out.push((n.clone(), marks, may_end, caller.to_owned()));
            }
            Gram::Seq(gs) => {
                for (i, x) in gs.iter().enumerate() {
                    let mut r: Vec<&Gram> = gs[i + 1..].iter().collect();
                    r.extend_from_slice(rest);
                    self.sites(x, &r, caller, out)?;
                }
            }
            Gram::Alt(gs) => {
                for x in gs {
                    self.sites(x, rest, caller, out)?;
                }
            }
            Gram::Star(a) | Gram::Plus(a) => {
                let mut r: Vec<&Gram> = vec![g];
                r.extend_from_slice(rest);
                self.sites(a, &r, caller, out)?;
            }
            Gram::Opt(a) | Gram::Field(_, a) | Gram::Retag(_, a) | Gram::Merge(_, a) | Gram::As(_, a) => self.sites(a, rest, caller, out)?,
            Gram::Wrap { body, .. } | Gram::Reread { body, .. } | Gram::Flag { body, .. } | Gram::Unless { body, .. } => self.sites(body, rest, caller, out)?,
            _ => {}
        }
        Ok(())
    }

    /// `g`'s code, `rest` the rule's elements after it.
    fn code(&self, g: &Gram, rest: &[&Gram], rule: &str, pad: &str) -> Result<String, String> {
        let deeper = format!("{pad}    ");
        Ok(match g {
            Gram::Lit(_) | Gram::Sym(_) if self.token_of(g).is_some() => {
                let (test, kind) = self.token_of(g).expect("a token");
                self.take_token(&test, Some(&kind), pad)?
            }
            Gram::As(k, a) if k == ::bitlex::syntax::IMPLIED => {
                let (test, _) = self.token_of(a).ok_or_else(|| format!("rule {rule}: an implied token that is no token"))?;
                self.take_token(&test, None, pad)?
            }
            Gram::As(k, a) if self.token_of(a).is_none() && matches!(&**a, Gram::Sym(n) if self.rules.contains_key(n.as_str()) && !self.nested(n)) => {
                let Gram::Sym(n) = &**a else { unreachable!() };
                self.token_choice(n, Some(k), rule, pad)?
            }
            Gram::As(k, a) => {
                let (test, _) = self.token_of(a).ok_or_else(|| format!("rule {rule}: `as {k}` of no token"))?;
                self.take_token(&test, Some(k), pad)?
            }
            Gram::Sym(n) if self.remember.contains(n) => self.token_choice(n, None, rule, pad)?,
            Gram::Sym(n) if self.mark_of(n).is_some() => {
                let m = &self.x.marks[self.mark_of(n).expect("a mark")];
                format!("{pad}self.mark_{}()?;\n", ident(&m.name))
            }
            Gram::Sym(n) if self.rules.contains_key(n.as_str()) && self.nested(n) => {
                // A level's call: the questions it asks where it may end at a mark, those of the
                // level it returns to (set where that takes the mark, kept where it ends there too).
                let (set, keep, all) = self.call_asks(rest, rule)?;
                let call = if all == 0 {
                    format!("{pad}self.{}()?;\n", rule_fn(n))
                } else {
                    format!("{pad}let asks = self.asks;\n{pad}self.asks = asks & {keep:#x} | {set:#x};\n{pad}self.{}()?;\n{pad}self.asks = asks;\n", rule_fn(n))
                };
                // The level's word is the caller's again where the call returns (the machine's `PUSH`).
                let call = if self.remember.is_empty() { call } else { format!("{pad}let word = self.word;\n{call}{pad}self.word = word;\n") };
                // The marks the callee refuses (`refuse M in L`), the caller's again where it returns.
                if self.refusing.is_empty() { call } else { format!("{pad}let refusing = self.refusing;\n{pad}self.refusing = {:#x};\n{call}{pad}self.refusing = refusing;\n", self.refusing.get(n.as_str()).copied().unwrap_or(0)) }
            }
            Gram::Sym(n) if self.rules.contains_key(n.as_str()) => format!("{pad}self.{}()?;\n", rule_fn(n)),
            Gram::Sym(n) => return Err(format!("rule {rule}: `{n}`, no rule, mark or token")),
            Gram::Seq(gs) => {
                let mut out = String::new();
                for (i, g) in gs.iter().enumerate() {
                    let mut r: Vec<&Gram> = gs[i + 1..].iter().collect();
                    r.extend_from_slice(rest);
                    out.push_str(&self.code(g, &r, rule, pad)?);
                }
                out
            }
            Gram::Opt(a) => {
                let arms = self.arms(a, rest, rule)?;
                let ex = self.exits_ending(rest, rule)?;
                let arms: Vec<(Vec<First>, String)> = arms.into_iter().map(|(f, b, _)| (f, b)).collect();
                let unmarked = self.after_mark(rest, rule)?;
                self.asking(&arms, &ex, pad) + &self.decide(&arms, &ex, &unmarked, "", "", pad)
            }
            Gram::Star(a) | Gram::Plus(a) => {
                let mut out = String::new();
                if let Gram::Plus(_) = g {
                    let r: Vec<&Gram> = std::iter::once(g).chain(rest.iter().copied()).collect();
                    out.push_str(&self.code(a, &r, rule, pad)?);
                }
                let r: Vec<&Gram> = std::iter::once(g).chain(rest.iter().copied()).collect();
                let arms = self.arms(a, &r, rule)?;
                let ex = self.exits_ending(rest, rule)?;
                let arms: Vec<(Vec<First>, String)> = arms.into_iter().map(|(f, b, _)| (f, b)).collect();
                let unmarked = self.after_mark(rest, rule)?;
                let d = self.asking(&arms, &ex, &deeper) + &self.decide(&arms, &ex, &unmarked, "break;\n", "break;\n", &deeper);
                let _ = write!(out, "{pad}loop {{\n{d}{pad}}}\n");
                out
            }
            Gram::Alt(_) => {
                let arms = self.arms(g, rest, rule)?;
                let empty = arms.iter().position(|a| a.2);
                let none = match empty {
                    Some(i) => arms[i].1.clone(),
                    None => "return self.refuse();\n".to_owned(),
                };
                let ex = if empty.is_some() { self.exits_ending(rest, rule)? } else { Vec::new() };
                let arms: Vec<(Vec<First>, String)> = arms.into_iter().map(|(f, b, _)| (f, b)).collect();
                let unmarked = if empty.is_some() { self.after_mark(rest, rule)? } else { Vec::new() };
                self.asking(&arms, &ex, pad) + &self.decide(&arms, &ex, &unmarked, &none, &none, pad)
            }
            Gram::Field(f, a) => {
                let mut out = self.code(a, rest, rule, pad)?;
                let _ = writeln!(out, "{pad}self.field_made({});", self.field_const(f)?);
                out
            }
            Gram::Retag(k, a) => format!("{pad}self.retag({});\n{}", self.kind_const(k)?, self.code(a, rest, rule, pad)?),
            Gram::Wrap { kind, field, moved, body } => {
                let f = match field {
                    Some(f) if f != "_" => self.field_const(f)?,
                    _ => "0".to_owned(),
                };
                let mut out = format!("{pad}self.wrap({}, {f})?;\n", self.kind_const(kind)?);
                if let Some(m) = moved {
                    let _ = writeln!(out, "{pad}self.retype_moved({});", self.moved_kind(m)?);
                }
                out.push_str(&self.code(body, rest, rule, pad)?);
                let _ = writeln!(out, "{pad}self.close()?;");
                out
            }
            // The node made last at this level relabelled, as the machine's `NODE_FIELD` and `RETAG` on it (modes 2 and
            // 3): the field of the node its wrap holds first, then its own field, then its kind; `drop` (kind `GONE`) takes
            // the node out, the nodes inside it in its place.
            Gram::Relabel { field, moved, inner, body } => {
                let mut out = String::new();
                if let Some(g) = inner.as_deref().filter(|g| *g != "_") {
                    let _ = writeln!(out, "{pad}self.relabel_inner({})?;", self.field_const(g)?);
                }
                if let Some(f) = field.as_deref().filter(|f| *f != "_") {
                    let _ = writeln!(out, "{pad}self.relabel_field({})?;", self.field_const(f)?);
                }
                match moved.as_deref() {
                    Some(::bitlex::syntax::GONE) => {
                        let _ = writeln!(out, "{pad}self.drop_made()?;");
                    }
                    Some(m) => {
                        let _ = writeln!(out, "{pad}self.relabel_kind({})?;", self.moved_kind(m)?);
                    }
                    None => {}
                }
                out.push_str(&self.code(body, rest, rule, pad)?);
                out
            }
            Gram::Merge(k, a) => {
                let (test, _) = self.token_of(a).ok_or_else(|| format!("rule {rule}: a merge of no token"))?;
                format!("{pad}self.merge({}, {})?;\n", self.token_cond(&test), self.kind_const(k)?)
            }
            Gram::Flag { flag, set, body } => {
                let f = format!("FLAG_{}", upper(flag));
                let op = if *set { format!("self.flags | {f}") } else { format!("self.flags & !{f}") };
                format!("{pad}let flags = self.flags;\n{pad}self.flags = {op};\n{}{pad}self.flags = flags;\n", self.code(body, rest, rule, pad)?)
            }
            Gram::Unless { flag, body } => format!("{pad}if self.flags & FLAG_{} == 0 {{\n{}{pad}}}\n", upper(flag), self.code(body, rest, rule, &deeper)?),
            Gram::Reread { reading, body } => {
                if !self.readings.contains_key(reading.as_str()) {
                    return Err(format!("rule {rule}: no reading `{reading}`"));
                }
                format!("{pad}self.reread_{}()?;\n{}", ident(reading), self.code(body, rest, rule, pad)?)
            }
            other => return Err(format!("rule {rule}: not written yet: {other:?}")),
        })
    }

    /// A level's call before `rest` in `rule`: the questions it asks where it may end at a mark, set where its caller
    /// takes the mark and kept where the caller ends there too; and every mark's that has questions (none: no call
    /// saves them).
    fn call_asks(&self, rest: &[&Gram], rule: &str) -> Result<(u64, u64, u64), String> {
        let ex = self.exits_ending(rest, rule)?;
        let (mut set, mut keep) = (0u64, 0u64);
        for (m, mark) in self.x.marks.iter().enumerate() {
            if !self.asks(m) {
                continue;
            }
            let first = self.mark_of(&mark.name).unwrap_or(m);
            match ex.iter().find(|e| e.0 == first) {
                Some((_, false)) => set |= 1 << first,
                Some((_, true)) => keep |= 1 << first,
                None => {}
            }
        }
        let all: u64 = (0..self.x.marks.len()).filter(|&m| self.asks(m)).fold(0, |a, m| a | 1 << self.mark_of(&self.x.marks[m].name).unwrap_or(m));
        Ok((set, keep, all))
    }

    /// A kind a wrap or relabel gives its node: a `remember` choice's name is its level's word.
    fn moved_kind(&self, name: &str) -> Result<String, String> {
        if self.remember.iter().any(|r| r == name) { Ok("self.word".to_owned()) } else { self.kind_const(name) }
    }

    /// A choice of tokens `n` taken: one decision, an arm a token, each a leaf of kind `kind` (none: its own); in a
    /// `remember` choice each also its level's word, as the machine's `REMEMBER`.
    fn token_choice(&self, n: &str, kind: Option<&str>, rule: &str, pad: &str) -> Result<String, String> {
        let Some(Stmt::Choice { body, .. }) = self.rules.get(n) else { return Err(format!("rule {rule}: `{n}`, no choice of tokens")) };
        let mut alts = Vec::new();
        Self::alts(body, &mut alts);
        let remembered = self.remember.iter().any(|r| r == n);
        let mut arms = Vec::new();
        for a in alts {
            let (test, own) = self.token_of(a).ok_or_else(|| format!("rule {rule}: choice `{n}` holds more than tokens"))?;
            let mut body = self.take_token(&test, Some(kind.unwrap_or(&own)), "")?;
            if remembered {
                let _ = writeln!(body, "self.word = {};", self.kind_const(&own)?);
            }
            arms.push((self.first(a)?.0, body));
        }
        let none = "return self.refuse();\n";
        Ok(self.asking(&arms, &[], pad) + &self.decide(&arms, &[], &[], none, none, pad))
    }

    /// A token element: its test and its leaf's kind.
    fn token_of(&self, g: &Gram) -> Option<(Test, String)> {
        match g {
            Gram::Lit(t) => match self.word_of(t) {
                Some(w) => Some((Test::Word(w), t.clone())),
                None if self.is_token_kind(t) => Some((Test::Kind(t.clone()), t.clone())),
                None => None,
            },
            Gram::Sym(n) if self.mark_of(n).is_none() && !self.rules.contains_key(n.as_str()) && self.is_token_kind(n) => Some((Test::Kind(n.clone()), n.clone())),
            Gram::Sym(n) if self.mark_of(n).is_none() && !self.rules.contains_key(n.as_str()) => self.keyword_word(n).or_else(|| self.word_of(n)).map(|w| (Test::Word(w), n.clone())),
            _ => None,
        }
    }

    /// The token `t` taken: a leaf of `kind`, or none (a token the grammar implies), as the
    /// lean parser's `expect`: by its word, else by its kind's test.
    fn take_token(&self, t: &Test, kind: Option<&str>, pad: &str) -> Result<String, String> {
        Ok(match (t, kind) {
            (Test::Word(w), Some(k)) => format!("{pad}self.word_leaf(w::{}, {})?;\n", self.word_const(*w), self.kind_const(k)?),
            (Test::Word(w), None) => format!("{pad}self.word_skip(w::{})?;\n", self.word_const(*w)),
            (_, Some(k)) => format!("{pad}self.tk({}, {}, 0)?;\n", self.token_cond(t), self.kind_const(k)?),
            (_, None) => format!("{pad}self.skip({})?;\n", self.token_cond(t)),
        })
    }

    fn token_cond(&self, t: &Test) -> String {
        self.cond(&(t.clone(), None))
    }

    fn rule(&self, name: &str) -> Result<String, String> {
        let (body, node, nested, whole, padded) = match self.rules[name] {
            Stmt::Node { body, nested, whole, padded, .. } => (body, true, *nested, *whole, *padded),
            Stmt::Choice { body, nested, .. } => (body, false, *nested, false, false),
            _ => unreachable!(),
        };
        let root = self.order.first() == Some(&name);
        let mut out = format!("    /// `{name}`.\n    fn {}(&mut self) -> R {{\n", rule_fn(name));
        if nested {
            out.push_str("        self.deeper()?;\n");
        }
        if root {
            let _ = writeln!(out, "        self.open_root({});", self.kind_const(name)?);
            out.push_str(&self.code(body, &[], name, "        ")?);
            let _ = writeln!(out, "        self.close_root({whole}, {padded})");
        } else {
            if node {
                // A kind of the grammar's own (no tape kind, always retagged inside): a placeholder until its retag.
                let id = (self.x.kind)(name).ok_or_else(|| format!("no kind `{name}`"))?;
                if id > 0x3ff {
                    out.push_str("        self.open(UNSET);\n");
                } else {
                    let _ = writeln!(out, "        self.open({});", self.kind_const(name)?);
                }
            }
            out.push_str(&self.code(body, &[], name, "        ")?);
            if node {
                out.push_str("        self.close()?;\n");
            }
            if nested {
                out.push_str("        self.depth -= 1;\n");
            }
            out.push_str("        self.marked = false;\n        Ok(())\n");
        }
        out.push_str("    }\n\n");
        Ok(out)
    }

    /// A token element's test of the token at `j` (a token mark's).
    fn elem(&self, e: &[TokenElem], j: &str) -> Result<String, String> {
        let mut pos = Vec::new();
        let mut and = Vec::new();
        for x in e {
            match x {
                TokenElem::Kind(k) => {
                    self.kind_tests.borrow_mut().insert(k.clone());
                    pos.push(format!("self.at_{}_at({j})", ident(k)));
                }
                TokenElem::Word(w) => match self.word_of(&String::from_utf8_lossy(w)) {
                    Some(id) => pos.push(format!("self.word_at({j}) == w::{}", self.word_const(id))),
                    None => pos.push("false".into()),
                },
                TokenElem::Not(inner) => pos.push(format!("({j} < self.n && !({}))", self.elem(inner, j)?)),
                TokenElem::End => pos.push(format!("{j} >= self.n")),
                TokenElem::Without(m) => and.push(format!("!self.bit_at({j}, {})", self.bit_of(m)?)),
                TokenElem::With(m) => and.push(format!("self.bit_at({j}, {})", self.bit_of(m)?)),
                TokenElem::Starting(bytes) => and.push(format!("({j} < self.n && matches!(self.src[self.recs[{j}].start as usize], {}))", bytes.iter().map(|b| format!("{b:#04x}")).collect::<Vec<_>>().join(" | "))),
                TokenElem::Many(..) => return Err("a run inside an element".into()),
            }
        }
        let mut c = if pos.is_empty() { "true".to_owned() } else { format!("({})", pos.join(" || ")) };
        for a in and {
            c = format!("{c} && {a}");
        }
        Ok(c)
    }

    fn bit_of(&self, m: &str) -> Result<String, String> {
        Ok(self.bit_num(m)?.to_string())
    }

    /// Mark bit `b` read of a record's code (`reads`).
    fn read_bit(&self, b: u32) {
        self.reads.set(self.reads.get() | 1 << b);
    }

    /// The words a run's first token must be, where its first element tests only words (`.` or `@`): else none.
    fn first_words(&self, seq: &[Vec<TokenElem>]) -> Option<Vec<String>> {
        let mut ws = Vec::new();
        for x in seq.first()? {
            match x {
                TokenElem::Word(w) => ws.push(format!("w::{}", self.word_const(self.word_of(&String::from_utf8_lossy(w))?))),
                // A further test of the token only narrows the words.
                TokenElem::Without(_) | TokenElem::With(_) | TokenElem::Starting(_) => {}
                _ => return None,
            }
        }
        (!ws.is_empty()).then_some(ws)
    }

    /// The token before the token a run starts at, as its first function's `prev`: found (a scan back over the extras)
    /// only where the run's first element reads it, a `(C)*` with an ending; else none (`self.n`), as no element reads it.
    fn run_prev(seq: &[Vec<TokenElem>]) -> &'static str {
        match seq.first().map(Vec::as_slice) {
            Some([TokenElem::Many(_, _, ending)]) if !ending.is_empty() => "self.prev_tok(self.i)",
            _ => "self.n",
        }
    }

    /// A run of token elements from `j` (the token before: `prev`), as a function's body: whether
    /// they hold.
    fn run(&self, seq: &[Vec<TokenElem>], name: &str, fns: &mut String) -> Result<(), String> {
        for (k, e) in seq.iter().enumerate() {
            let next = if k + 1 == seq.len() { "true".to_owned() } else { format!("self.{name}_{}(self.next_tok(j + 1), j)", k + 1) };
            let body = if let [TokenElem::Many(choices, nests, ending)] = e.as_slice() {
                // `(C)*`: the elements after it tried at each token from here, where every depth is
                // 0 (and, with `ending`, after one of its tokens); else one more of the run's.
                let after = if k + 1 == seq.len() { "true".to_owned() } else { format!("self.{name}_{}(j, prev)", k + 1) };
                let mut b = String::new();
                let _ = writeln!(b, "        let (mut j, mut prev) = (j, prev);\n        let mut depth = [0u32; {}];", nests.len().max(1));
                b.push_str("        loop {\n");
                let ok_end = if ending.is_empty() { "true".to_owned() } else { format!("prev < self.n && {}", self.elem(ending, "prev")?) };
                let _ = writeln!(b, "            if depth.iter().all(|&d| d == 0) && {ok_end} && {after} {{\n                return true;\n            }}");
                let c = self.elem(choices, "j")?;
                let _ = writeln!(b, "            if j >= self.n || !({c}) {{\n                return false;\n            }}");
                for (d, n) in nests.iter().enumerate() {
                    let _ = writeln!(b, "            if {} {{\n                depth[{d}] += 1;\n            }} else if {} {{\n                if depth[{d}] == 0 {{\n                    return false;\n                }}\n                depth[{d}] -= 1;\n            }}", self.elem(&n.open, "j")?, self.elem(&n.close, "j")?);
                }
                b.push_str("            prev = j;\n            j = self.next_tok(j + 1);\n        }\n");
                b
            } else {
                format!("        let _ = prev;\n        {} && {next}\n", self.elem(e, "j")?)
            };
            let _ = write!(fns, "    #[allow(unused_variables)]\n    fn {name}_{k}(&self, j: usize, prev: usize) -> bool {{\n{body}    }}\n\n");
        }
        Ok(())
    }

    /// A gap question's element at record `j` (`GapPattern`, decision 0451): over the records,
    /// the extras among them; `none` what it gives where there is no record there.
    fn gap_elem(&self, e: &[TokenElem], j: &str) -> Result<(String, bool), String> {
        let (not, choices) = match e {
            [TokenElem::Not(inner)] => (true, inner.as_slice()),
            c => (false, c),
        };
        let mut pos = Vec::new();
        let mut and = Vec::new();
        for c in choices {
            match c {
                TokenElem::Kind(k) if self.x.extras.contains(k) => pos.push(format!("(is_extra(self.recs[{j}].code) && extra_kind(self.recs[{j}].code) == {})", self.kind_const(k)?)),
                TokenElem::Kind(k) => {
                    self.kind_tests.borrow_mut().insert(k.clone());
                    pos.push(format!("(!is_extra(self.recs[{j}].code) && self.at_{}_rec({j}))", ident(k)));
                }
                TokenElem::Word(w) => match self.word_of(&String::from_utf8_lossy(w)) {
                    Some(id) => pos.push(format!("(!is_extra(self.recs[{j}].code) && self.recs[{j}].code >> 8 & 0xff == w::{})", self.word_const(id))),
                    None => pos.push("false".into()),
                },
                TokenElem::With(m) => and.push(format!("self.marks_hold({j}, 1 << {})", self.bit_num(m)?)),
                TokenElem::Without(m) => and.push(format!("!self.marks_hold({j}, 1 << {})", self.bit_num(m)?)),
                TokenElem::Starting(bytes) => and.push(format!("matches!(self.src[self.recs[{j}].start as usize], {})", bytes.iter().map(|b| format!("{b:#04x}")).collect::<Vec<_>>().join(" | "))),
                TokenElem::End | TokenElem::Not(_) | TokenElem::Many(..) => return Err(format!("a gap question's element {c:?}")),
            }
        }
        // No kinds or words: any record.
        let takes = if pos.is_empty() { "true".to_owned() } else { format!("({})", pos.join(" || ")) };
        let mut c = if not { format!("!{takes}") } else { takes };
        for a in and {
            c = format!("{c} && {a}");
        }
        Ok((c, not))
    }

    fn bit_num(&self, m: &str) -> Result<u32, String> {
        let b = u32::from(self.x.marks.iter().find(|x| x.name == m && x.bit.is_some()).and_then(|x| x.bit).ok_or_else(|| format!("mark {m}: no bit in the code"))?);
        self.read_bit(b);
        Ok(b)
    }

    /// A gap question as a function of the token's record (`gap_pattern_holds`): `after`'s
    /// records before it, the last just before; `since`'s from the grammar token before; `before`'s
    /// from it on.
    fn gap_fn(&self, q: &::bitlex::syntax::GapPattern, name: &str, fns: &mut String) -> Result<(), String> {
        let mut conds = Vec::new();
        let a = q.after.len() as isize;
        for (i, e) in q.after.iter().enumerate() {
            let (c, not) = self.gap_elem(e, "j")?;
            if q.since {
                // From the grammar token before (none before the file's first: the record before 0).
                let none = if i == 0 && not { "true" } else { "false" };
                conds.push(format!("{{ let j = g as isize + {i} - 1; if j < 0 {{ {none} }} else {{ let j = j as usize; j < here && {c} }} }}"));
            } else {
                let off = i as isize - a;
                let none = if not { "true" } else { "false" };
                conds.push(format!("{{ let j = here as isize + {off}; if j < 0 {{ {none} }} else {{ let j = j as usize; {c} }} }}"));
            }
        }
        for (i, e) in q.before.iter().enumerate() {
            if matches!(e.as_slice(), [TokenElem::End]) {
                conds.push(format!("here + {i} >= self.n"));
                continue;
            }
            let (c, _) = self.gap_elem(e, "j")?;
            conds.push(format!("{{ let j = here + {i}; j < self.n && {c} }}"));
        }
        let g = if q.since { "        let g = self.prev_tok(here);\n        let g = if g >= self.n { 0 } else { g + 1 };\n" } else { "" };
        let _ = write!(fns, "    #[allow(unused_variables)]\n    fn {name}(&self, here: usize) -> bool {{\n{g}        {}\n    }}\n\n", if conds.is_empty() { "true".to_owned() } else { conds.iter().map(|c| format!("({c})")).collect::<Vec<_>>().join("\n            && ") });
        Ok(())
    }

    /// A mark's questions of one way (`drops`, `loses`, …) as one test at the token: none of the
    /// `unless` ones holds, and one of the others does where it has any (`patterns_hold`).
    fn gap_test(&self, qs: &[::bitlex::syntax::GapPattern], name: &str, at_end: bool, fns: &mut String) -> Result<String, String> {
        let (mut yes, mut no) = (Vec::new(), Vec::new());
        for (e, q) in qs.iter().enumerate() {
            let f = format!("{name}_{e}");
            self.gap_fn(q, &f, fns)?;
            let call = format!("self.{f}(self.i)");
            if q.unless { no.push(call) } else { yes.push(call) }
        }
        let mut c: Vec<String> = no.iter().map(|x| format!("!{x}")).collect();
        if !yes.is_empty() {
            c.push(format!("({})", yes.join(" || ")));
        }
        if !at_end {
            c.insert(0, "self.i < self.n".to_owned());
        }
        let _ = write!(fns, "    fn {name}(&self) -> bool {{\n        {}\n    }}\n\n", if c.is_empty() { "true".to_owned() } else { c.join(" && ") });
        Ok(format!("self.{name}()"))
    }

    /// Each mark's test, `m_NAME`: it holds at the token and is not taken there.
    fn marks(&self, out: &mut String) -> Result<(), String> {
        if self.x.marks.len() > 64 {
            return Err(format!("{} marks: a token's marks are one u64", self.x.marks.len()));
        }
        let mut done: HashSet<&str> = HashSet::new();
        let mut fns = String::new();
        // Each mark's guard words (`first_words`), by its name: none where a definition starts otherwise.
        let mut guards: std::collections::BTreeMap<String, Option<Vec<String>>> = std::collections::BTreeMap::new();
        for (order, m) in self.x.marks.iter().enumerate() {
            if !done.insert(m.name.as_str()) {
                continue;
            }
            let n = ident(&m.name);
            let defs: Vec<(usize, &Mark)> = self.x.marks.iter().enumerate().filter(|(_, x)| x.name == m.name).collect();
            let mut holds = Vec::new();
            // The words the token must be for any definition to hold: a guard before the mark's cache (`held`).
            let mut firsts: Option<Vec<String>> = Some(Vec::new());
            for (d, (_, def)) in defs.iter().enumerate() {
                let mut c = Vec::new();
                match (def.tokens.and_then(|(_, seq)| self.first_words(seq)), firsts.as_mut()) {
                    (Some(ws), Some(all)) => all.extend(ws),
                    _ => firsts = None,
                }
                if let Some((before, seq)) = def.tokens {
                    let pat = format!("pat_{n}_{d}");
                    self.run(seq, &pat, &mut fns)?;
                    c.push(format!("self.{pat}_0(self.i, {})", Self::run_prev(seq)));
                    if !before.is_empty() {
                        let p = self.elem(before, "p")?;
                        let negated = matches!(before, [TokenElem::Not(_)]);
                        c.push(if negated { format!("{{ let p = self.prev_tok(self.i); p >= self.n || {p} }}") } else { format!("{{ let p = self.prev_tok(self.i); p < self.n && {p} }}") });
                    }
                } else if let Some(b) = def.bit {
                    self.read_bit(u32::from(b));
                    if def.word != 0 {
                        c.push(format!("self.word() == {}", def.word));
                    }
                    c.push(if def.end { format!("(if self.i < self.n {{ self.bit_at(self.i, {b}) }} else {{ true }})") } else { format!("self.bit_at(self.i, {b})") });
                } else {
                    c.push("false".into());
                }
                // `unless after W`: none after the word.
                for w in def.unless_after {
                    if let Some(id) = self.word_of(&String::from_utf8_lossy(w)) {
                        c.push(format!("{{ let p = self.prev_tok(self.i); p >= self.n || self.word_at(p) != w::{} }}", self.word_const(id)));
                    }
                }
                // `before` and `unless before`: the runs from the token.
                let (mut yes, mut no) = (Vec::new(), Vec::new());
                for (e, (unless, seq)) in def.next.iter().enumerate() {
                    let pat = format!("next_{n}_{d}_{e}");
                    self.run(seq, &pat, &mut fns)?;
                    let call = format!("self.{pat}_0(self.i, {})", Self::run_prev(seq));
                    if *unless { no.push(call) } else { yes.push(call) }
                }
                for x in no {
                    c.push(format!("!{x}"));
                }
                if !yes.is_empty() {
                    c.push(format!("({})", yes.join(" || ")));
                }
                holds.push(format!("({})", c.join(" && ")));
            }
            guards.insert(n.clone(), firsts.clone());
            let guard = match firsts {
                Some(mut ws) if !ws.is_empty() => {
                    ws.sort();
                    ws.dedup();
                    format!("matches!(self.word(), {}) && ", ws.join(" | "))
                }
                _ => String::new(),
            };
            let _ = writeln!(out, "    /// Mark `{}` holds at the token, not taken there; once a token (`held`).\n    #[inline]\n    fn m_{n}(&mut self) -> bool {{\n        {guard}self.taken >> {order} & 1 == 0 && self.held({order}, Self::h_{n})\n    }}\n\n    fn h_{n}(&self) -> bool {{\n        {}\n    }}\n", m.name, holds.join(" || "));
            // Taken: `refuses taken` asked; the extras before its bit settled; `drops` asked.
            let qs = |f: fn(&Mark<'a>) -> &'a [::bitlex::syntax::GapPattern]| -> Vec<::bitlex::syntax::GapPattern> { defs.iter().flat_map(|(_, d)| f(d).iter().cloned()).collect() };
            let mut take = String::new();
            let refuses_taken = qs(|d| d.refuses_taken);
            if !refuses_taken.is_empty() {
                let t = self.gap_test(&refuses_taken, &format!("refuses_taken_{n}"), true, &mut fns)?;
                let _ = write!(take, "        if {t} {{\n            return self.refuse();\n        }}\n");
            }
            if m.settles
                && let Some(b) = m.bit
            {
                self.read_bit(u32::from(b));
                let _ = writeln!(take, "        self.settle({b});");
            }
            let drops = qs(|d| d.drops);
            if !drops.is_empty() {
                let t = self.gap_test(&drops, &format!("drops_{n}"), false, &mut fns)?;
                let _ = write!(take, "        if {t} {{\n            self.drop_pending();\n        }}\n");
            }
            let _ = writeln!(out, "    /// Mark `{}` taken: it and the marks before it passed at this token.\n    fn take_{n}(&mut self) -> R {{\n{take}        self.taken |= {:#x};\n        Ok(())\n    }}\n", m.name, (u64::MAX >> (63 - order)) | m.passes);
            let _ = writeln!(out, "    /// Mark `{}` where the rule takes it: it must hold.\n    fn mark_{n}(&mut self) -> R {{\n        if !self.m_{n}() {{\n            return self.refuse();\n        }}\n        self.marked = true;\n        self.take_{n}()\n    }}\n", m.name);
            // Asked wherever a decision tests it (`refuses`, then `loses`): at a level that takes
            // it, always; where the level may end at it (`end`), where the caller's level is the
            // mark's (`asks`, the lean parser's `stmt`).
            // `loses since`: the first extra pending moved (`move_first`), apart from the others.
            let (refuses, all) = (qs(|d| d.refuses), qs(|d| d.loses));
            let (moves, loses): (Vec<_>, Vec<_>) = all.into_iter().partition(|q| q.since);
            if !refuses.is_empty() || !loses.is_empty() || !moves.is_empty() {
                let mut ask = String::new();
                if !refuses.is_empty() {
                    let t = self.gap_test(&refuses, &format!("refuses_{n}"), true, &mut fns)?;
                    let _ = write!(ask, "        if {t} {{\n            return self.refuse();\n        }}\n");
                }
                if !loses.is_empty() {
                    let t = self.gap_test(&loses, &format!("loses_{n}"), false, &mut fns)?;
                    let _ = write!(ask, "        if {t} {{\n            self.lose_pending();\n        }}\n");
                }
                if !moves.is_empty() {
                    let t = self.gap_test(&moves, &format!("moves_{n}"), false, &mut fns)?;
                    let _ = write!(ask, "        if {t} {{\n            self.move_first();\n        }}\n");
                }
                let _ = writeln!(out, "    /// Mark `{}`'s questions, where a decision tests it (`end`: where the level may end at it).\n    fn ask_{n}(&mut self, end: bool) -> R {{\n        if end && self.asks >> {order} & 1 == 0 {{\n            return Ok(());\n        }}\n{ask}        Ok(())\n    }}\n", m.name);
            }
        }
        out.push_str(&fns);
        // The marks no rule may pass: where one holds at a token taken, the file is refused.
        // Asked by the token's word, as the lean parser's: a mark whose runs start with words only at those words; the
        // others at every token.
        let mut open: Vec<String> = self.x.marks.iter().filter(|m| !m.hidden).map(|m| ident(&m.name)).collect::<HashSet<_>>().into_iter().collect();
        open.sort();
        let call = |n: &String| format!("self.m_{n}()");
        let always: Vec<String> = open.iter().filter(|n| !matches!(guards.get(*n), Some(Some(_)))).map(call).collect();
        let mut by_word: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
        for n in &open {
            if let Some(Some(ws)) = guards.get(n) {
                for w in ws {
                    by_word.entry(w.clone()).or_default().push(call(n));
                }
            }
        }
        let any = |cs: Vec<String>| if cs.is_empty() { "false".to_owned() } else { cs.join(" || ") };
        let body = if by_word.is_empty() {
            any(always.clone())
        } else {
            let mut arms: String = by_word.into_iter().map(|(w, cs)| format!("            {w} => {},\n", any(cs.into_iter().chain(always.iter().cloned()).collect()))).collect();
            let _ = write!(arms, "            _ => {},\n", any(always.clone()));
            format!("match self.word() {{\n{arms}        }}")
        };
        let _ = writeln!(out, "    /// A mark no rule takes holds at the token (a mark not hidden).\n    fn open_mark(&mut self) -> bool {{\n        {body}\n    }}\n");
        Ok(())
    }

    /// The split of each region tier 2 reads in parts (`Split::Tier2`, decision 0323's lowering B),
    /// as `records::read_parts` reads one: `split` at a record whose first byte opens one, a
    /// `split_R` a reader; `word_in`, a piece's word; `KINDS`, a byte's record kind.
    fn splits(&self, out: &mut String, free: &mut String) -> Result<(), String> {
        let mut at = String::new();
        // A string's region a template's `}` resumes (decision 0600): the reader that resumes
        // it, entered at its opener where that comes before the next record.
        let mut resume = String::new();
        let readers = crate::bitlex_lex_rust::readers(self.x.lexer);
        let lexer_regions: Vec<&::bitlex::syntax::Region> = readers.iter().flat_map(|rd| &rd.regions).collect();
        let kind_or = |k: Option<u8>, at: &str| match k {
            Some(k) => format!("{k}"),
            None => format!("u32::from(KINDS[usize::from(self.src[{at}])])"),
        };
        for (r, rd) in readers.iter().enumerate() {
            let piece = |name: &str, closes: bool| crate::bitlex_lex_rust::piece_kind(self.x.lexer, self.x.row_kinds, &rd.kind, name, closes);
            // A reader that resumes has no record of its own to split (`advance` enters it).
            if !rd.regions.iter().all(|g| g.resumes) {
                let _ = write!(at, "        if bit(&self.m.{}, s) {{\n            return self.split_{r}(j);\n        }}\n", rd.open);
            }
            let content = kind_or(piece(&rd.content, false), "at");
            let mut parts = String::new();
            let mut lasts = String::new();
            for (i, (name, start, len, last)) in rd.parts.iter().enumerate() {
                let bytes = match len {
                    ::bitlex::syntax::PartLen::Bytes(b) => *b,
                    ::bitlex::syntax::PartLen::Last(_) => 0,
                };
                let _ = write!(parts, "            // `{name}`\n            if let Some(e) = first_bit(&m.{start}, at).filter(|&e| e < c) {{\n                if part.is_none_or(|q| (e, {bytes}, {i}) < q) {{\n                    part = Some((e, {bytes}, {i}));\n                }}\n            }}\n");
                let l = match last {
                    Some(f) => format!("match first_bit(&m.{f}, e) {{\n                            Some(l) => l,\n                            None => return self.refuse(),\n                        }}"),
                    None => format!("e + {}", usize::from(bytes).saturating_sub(1)),
                };
                let _ = writeln!(lasts, "                        {i} => ({l}, {}),", kind_or(piece(name, false), "e"));
            }
            // The closer: one of a kind of its own (`closes`), else its region's own, its bytes.
            let own = kind_or(piece(&rd.kind, true), "c");
            let region_last = |q: usize| {
                let g = &rd.regions[q];
                match (&rd.region_or[q], &g.or) {
                    (Some(f), Some((_, m2))) => format!("if bit(&m.{f}, c) {{ c + {} }} else {{ c + {} }}", m2 - 1, g.bytes - 1),
                    _ => format!("c + {}", g.bytes - 1),
                }
            };
            let mut closer = String::new();
            for (k, f, b) in &rd.closes {
                let _ = write!(closer, "if bit(&m.{f}, c) {{\n                        (c + {}, {})\n                    }} else ", b - 1, kind_or(piece(k, true), "c"));
            }
            if rd.regions.len() == 1 {
                let _ = write!(closer, "{{\n                        ({}, {own})\n                    }}", region_last(0));
            } else {
                for q in 0..rd.regions.len() {
                    let test = if q + 1 == rd.regions.len() { String::new() } else { format!("if bit(&m.{}, s) ", rd.region_open[q]) };
                    let _ = write!(closer, "{test}{{\n                        // `{}`\n                        ({}, {own})\n                    }}{}", rd.regions[q].kind, region_last(q), if test.is_empty() { "" } else { " else " });
                }
            }
            let regions: Vec<&str> = rd.regions.iter().map(|g| g.kind.as_str()).collect();
            // Its closer at a template (decision 0600): a string open over the template's code;
            // a read that resumes one and closes at its quote, that string's end.
            let resumes = rd.regions.iter().all(|g| g.resumes);
            let resumed = |g: &::bitlex::syntax::Region| g.resumes || lexer_regions.iter().any(|q| q.resumes && q.after.contains(&g.kind));
            let mut templates: Vec<&String> = rd.regions.iter().zip(&rd.region_or).filter(|(g, _)| resumed(g)).filter_map(|(_, f)| f.as_ref()).collect();
            templates.dedup();
            let strings = if templates.is_empty() {
                String::new()
            } else {
                let t = templates.iter().map(|f| format!("bit(&m.{f}, c)")).collect::<Vec<_>>().join(" || ");
                if resumes {
                    format!("                    if !({t}) {{\n                        self.strings = self.strings.saturating_sub(1);\n                    }}\n")
                } else {
                    format!("                    if {t} {{\n                        self.strings += 1;\n                    }}\n")
                }
            };
            let (name, doc, head) = if resumes {
                // Its opener's kind: its own row's.
                let rows = self.x.lexer.iter().filter(|s| matches!(s, ::bitlex::syntax::Stmt::Token { .. }));
                let own = rows.enumerate().find(|(_, s)| matches!(s, ::bitlex::syntax::Stmt::Token { kind, .. } if *kind == rd.kind)).and_then(|(i, _)| self.x.row_kinds.get(i).copied().flatten());
                let _ = write!(resume, "        if self.strings > 0 && let Some(p) = first_bit(&self.m.{}, from).filter(|&p| self.i + 1 >= self.n || p < self.recs[self.i + 1].start as usize) {{\n            self.known = 0;\n            return self.resume_{r}(p);\n        }}\n", rd.open);
                (format!("resume_{r}"), "at byte `s`, no record of its own: the string's parts after its template's `}`", format!("s: usize) -> R {{\n        self.sub.clear();\n        self.k = 0;\n        self.piece(s, s, {});", kind_or(own, "s")))
            } else {
                (format!("split_{r}"), "record `j`", "j: usize) -> R {\n        let s = self.recs[j].start as usize;\n        self.sub.clear();\n        self.k = 0;\n        self.sub.push((s as u32, s as u32, self.recs[j].code));".to_owned())
            };
            let _ = write!(
                out,
                "    /// `{kind}`'s region (`read {regions} parts`), {doc}: its opener's byte, each part, each run of the bytes between, its closer.\n    fn {name}(&mut self, {head}\n        let m = self.m;\n        let mut at = s + 1;\n        loop {{\n            let Some(c) = first_bit(&m.{close}, at) else {{\n                return self.refuse();\n            }};\n            // The part first after `at`, before the closer: the earliest, the shortest at one byte.\n            let mut part: Option<(usize, u8, usize)> = None;\n{parts}            let to = part.map_or(c, |q| q.0);\n            if to > at {{\n                self.piece(at, to - 1, {content});\n            }}\n            match part {{\n                Some((e, _, i)) => {{\n                    let (l, kind) = match i {{\n{lasts}                        _ => unreachable!(),\n                    }};\n                    self.piece(e, l, kind);\n                    at = l + 1;\n                }}\n                None => {{\n                    let (l, kind) = {closer};\n                    self.piece(c, l, kind);\n{strings}                    return Ok(());\n                }}\n            }}\n        }}\n    }}\n\n",
                kind = rd.kind,
                regions = regions.join(" | "),
                close = rd.close
            );
        }
        let _ = write!(out, "    /// Past the token, to the next grammar token's record{}.\n    #[inline(always)]\n    fn advance(&mut self, from: usize) -> R {{\n{resume}        self.i = self.next_tok(self.i + 1);\n        self.arrive()\n    }}\n\n", if resume.is_empty() { "" } else { ", or the string's parts a template's `}` resumes, at the record before the next (decision 0600)" });
        let _ = write!(out, "    /// Record `j` split where its first byte opens a region tier 2 reads in parts.\n    #[inline(always)]\n    fn split(&mut self, j: usize) -> R {{\n        let s = self.recs[j].start as usize;\n{at}        let _ = s;\n        Ok(())\n    }}\n\n");
        // A piece's word: its kind's keywords', as the lean lexer finds a word.
        let mut arms = String::new();
        for (b, name) in self.x.tokens {
            let mut words: Vec<&Vec<u8>> = self.x.keywords.iter().filter(|r| r.0 == *name).flat_map(|r| r.1.iter().map(|(w, _)| w)).collect();
            words.extend(self.x.soft.iter().filter(|r| r.0 == *name).flat_map(|r| r.1.iter()));
            let mut pats = String::new();
            for w in words {
                if let Some(id) = self.word_of(&String::from_utf8_lossy(w)) {
                    let lit: String = w.iter().map(|&c| if c.is_ascii_alphanumeric() || c == b'_' { (c as char).to_string() } else { format!("\\x{c:02x}") }).collect();
                    let _ = writeln!(pats, "            b\"{lit}\" => w::{},", self.word_const(id));
                }
            }
            if !pats.is_empty() {
                let _ = write!(arms, "        {b} => match b {{\n{pats}            _ => 0,\n        }},\n");
            }
        }
        let _ = write!(free, "/// The word of a piece of record kind `kind`, its bytes `b`: its kind's keywords'.\nfn word_in(kind: u32, b: &[u8]) -> u32 {{\n    match kind {{\n{arms}        _ => 0,\n    }}\n}}\n\n");
        let kinds: Vec<String> = self.x.kinds.iter().map(|k| k.to_string()).collect();
        let _ = write!(free, "/// Each byte's record kind, as a token's first byte (`Records::kinds`).\nconst KINDS: [u8; 256] = [{}];\n\n", kinds.join(", "));
        Ok(())
    }

    /// Each token kind's test, `at_K` (the token) and `at_K_at(j)`: one of its records' kinds, and
    /// none of its row's hard words.
    fn kind_tests(&self, out: &mut String) {
        let names: Vec<String> = self.kind_tests.borrow().iter().cloned().collect();
        for k in names {
            let bytes: Vec<String> = self.x.tokens.iter().filter(|(_, n)| *n == k || n.strip_suffix("_close") == Some(k.as_str())).map(|(b, _)| format!("{b}")).collect();
            let hard: Vec<String> = self.x.keywords.iter().filter(|r| r.0 == k).flat_map(|r| r.1.iter().filter_map(|(w, _)| self.word_of(&String::from_utf8_lossy(w)))).map(|w| format!("{w}")).collect();
            let kinds = if bytes.is_empty() { "false".to_owned() } else { format!("matches!(c & 0xff, {})", bytes.join(" | ")) };
            let words = if hard.is_empty() { String::new() } else { format!(" && !matches!(c >> 8 & 0xff, {})", hard.join(" | ")) };
            let n = ident(&k);
            let _ = writeln!(out, "    fn at_{n}_at(&self, j: usize) -> bool {{\n        if j >= self.n {{\n            return false;\n        }}\n        let c = self.code_at(j);\n        {kinds}{words}\n    }}\n\n    fn at_{n}(&self) -> bool {{\n        self.at_{n}_at(self.i)\n    }}\n\n    /// Record `j` (not the piece at the token) one of the kind's.\n    fn at_{n}_rec(&self, j: usize) -> bool {{\n        let c = self.recs[j].code;\n        {kinds}{words}\n    }}\n");
        }
    }

    /// Each reading's function, `reread_R`: the node just made and the nodes inside it read again
    /// (`reread R`), the tape from it written again.
    fn readings(&self, out: &mut String) -> Result<(), String> {
        let mut names: Vec<&&str> = self.readings.keys().collect();
        names.sort();
        for name in names {
            let r = self.readings[*name];
            let mut arms = String::new();
            for (k, what) in &r.kinds {
                let id = (self.x.kind)(k).ok_or_else(|| format!("reading {name}: no kind `{k}`"))?;
                let to = match what {
                    // A `remember` choice's name: its level's word (`REMEMBERED`, `reread_node`).
                    ::bitlex::syntax::ReadKind::As(m) if self.remember.contains(m) => "Some(Some(REMEMBERED))".to_owned(),
                    ::bitlex::syntax::ReadKind::As(m) => format!("Some(Some({}))", (self.x.kind)(m).ok_or_else(|| format!("reading {name}: no kind `{m}`"))?),
                    ::bitlex::syntax::ReadKind::Gone => "Some(None)".to_owned(),
                    ::bitlex::syntax::ReadKind::Keep => "None".to_owned(),
                };
                let _ = writeln!(arms, "            {id} => {to},");
            }
            for e in self.x.extras {
                let id = (self.x.kind)(e).ok_or_else(|| format!("no kind `{e}`"))?;
                let _ = writeln!(arms, "            {id} => None,");
            }
            let mut fields = String::new();
            for (a, b) in &r.fields {
                let (fa, fb) = (self.field_id(a).map_err(|e| format!("reading {name}: {e}"))?, self.field_id(b).map_err(|e| format!("reading {name}: {e}"))?);
                let _ = writeln!(fields, "            {fa} => {fb},");
            }
            let root = match &r.root {
                Some(::bitlex::syntax::ReadKind::As(m)) => format!("Some(Some({}))", (self.x.kind)(m).ok_or_else(|| format!("no kind `{m}`"))?),
                Some(::bitlex::syntax::ReadKind::Gone) => "Some(None)".to_owned(),
                _ => "Some(Some(u16::MAX))".to_owned(),
            };
            let _ = writeln!(
                out,
                "    /// Reading `{name}`: each kind's way (`Some(Some(K))` read as `K`, `Some(None)` gone, `None` kept with what is inside it).\n    fn reading_{n}(kind: u16) -> Option<Option<u16>> {{\n        match kind {{\n{arms}            _ => Some(Some(u16::MAX - 1)),\n        }}\n    }}\n\n    fn reading_field_{n}(field: u16) -> u16 {{\n        match field {{\n{fields}            f => f,\n        }}\n    }}\n\n    fn reread_{n}(&mut self) -> R {{\n        let root: Option<Option<u16>> = {root};\n        self.reread(root, Self::reading_{n}, Self::reading_field_{n})\n    }}\n",
                n = ident(name)
            );
        }
        Ok(())
    }

    fn source(&self) -> Result<String, String> {
        let mut rules = String::new();
        for name in &self.order {
            rules.push_str(&self.rule(name)?);
        }
        let entry = rule_fn(self.order.first().ok_or("no rule")?);
        self.assemble(&rules, &entry, "// Generated by native/crates/compiler/src/bitlex_rust.rs; do not edit. The Bitlex compiler's Rust from the grammar's tree rules.\n\n", false)
    }

    /// The parser around `rules` (its rules' code, `impl P`'s), entered at `entry` (a method of `P`): the runtime, the
    /// marks, the kinds' tests, the readings, the regions' reads, and `parse`, each part written where the rules use
    /// it. `stack`: `P` holds the one loop's stack (`one_loop`'s records form: a level's caller's place and state).
    fn assemble(&self, rules: &str, entry: &str, header: &str, stack: bool) -> Result<String, String> {
        let mut marks = String::new();
        self.marks(&mut marks)?;
        let mut readings = String::new();
        self.readings(&mut readings)?;
        let mut kinds = String::new();
        self.kind_tests(&mut kinds);
        let mut extras = String::new();
        let mut split = String::new();
        self.splits(&mut split, &mut extras)?;
        // An extra's record: the extra's mark in its code; its kind the first extra's, or a later one's by its own bit.
        let is_extra = match self.x.extra_bits.first() {
            Some((b, _)) => format!("code >> {} & 1 == 1", 16 + u32::from(*b)),
            None => "false".to_owned(),
        };
        let _ = writeln!(extras, "\n/// A record of an extra (a comment): the extra's mark in its code.\n#[inline(always)]\nfn is_extra(code: u32) -> bool {{\n    {is_extra}\n}}\n");
        let mut kind = String::new();
        for (b, k) in self.x.extra_bits.iter().skip(1) {
            let _ = write!(kind, "    if code >> {} & 1 == 1 {{\n        return k::{};\n    }}\n", 16 + u32::from(*b), self.kind_const(k)?.trim_start_matches("k::"));
        }
        let first = match self.x.extra_bits.first() {
            Some((_, k)) => self.kind_const(k)?,
            None => "0".to_owned(),
        };
        let _ = writeln!(extras, "/// An extra's node kind: a later extra kind's by its bit, else the first's.\nfn extra_kind(code: u32) -> u16 {{\n{kind}    {first}\n}}\n");
        let mut out = String::new();
        out.push_str(header);
        out.push_str("use rerail_tape_format::{Layout, Narrow, Widths, Writer};\n\n");
        out.push_str("/// The tape's kinds, by the grammar's names.\n#[allow(dead_code)]\npub mod k {\n");
        for (c, id) in self.kinds.borrow().iter() {
            let _ = writeln!(out, "    pub const {c}: u16 = {id};");
        }
        out.push_str("}\n\n/// The fields.\n#[allow(dead_code)]\npub mod f {\n");
        for (c, id) in self.fields.borrow().iter() {
            let _ = writeln!(out, "    pub const {c}: u16 = {id};");
        }
        out.push_str("}\n\n/// The records' kinds (a code's low byte), by their tokens' names.\n#[allow(dead_code)]\npub mod t {\n");
        for (b, name) in self.x.tokens {
            let _ = writeln!(out, "    pub const {}: u32 = {b};", upper(name));
        }
        out.push_str("}\n\n/// The words, as a record's code names them.\n#[allow(dead_code)]\npub mod w {\n");
        for i in 0..self.x.words.len() {
            let _ = writeln!(out, "    pub const {}: u32 = {};", self.word_const((i + 1) as u8), i + 1);
        }
        out.push_str("}\n\n");
        for (i, f) in self.flags.iter().enumerate() {
            let _ = writeln!(out, "const FLAG_{}: u32 = 1 << {i};", upper(f));
        }
        out.push_str(&extras);
        out.push_str(RUNTIME);
        out.push_str("\n#[allow(clippy::all, unused_parens, dead_code)]\nimpl<L: Layout> P<'_, L> {\n");
        out.push_str(rules);
        out.push_str(&marks);
        out.push_str(&kinds);
        out.push_str(&readings);
        out.push_str(&split);
        out.push_str("}\n");
        // The one loop's stack: by each level open, the place its call returns to and the caller's state it saved.
        if stack {
            for (a, b) in [
                ("    strings: usize,\n}\n", "    strings: usize,\n    /// The one loop's stack (`run`): by each level open, its caller's state it saved, then the place it returns to; a\n    /// flag's value before its rule set it.\n    stack: Vec<u64>,\n}\n"),
                ("moved: None, strings: 0 };", "moved: None, strings: 0, stack: Vec::with_capacity(64) };"),
            ] {
                if out.matches(a).count() != 1 {
                    return Err(format!("tier 2 as one loop: the stack's place in the runtime: {a:?}"));
                }
                out = out.replace(a, b);
            }
        }
        // A field's slot past the grammar's ids (`field_id`) must be no field's id the tape names.
        let own = self.own_fields.borrow().len() as u16;
        if own > 0 && self.most_field.get() >= 64 - own {
            return Err(format!("tier 2 as Rust: {own} fields past a key's 6 bits, and field {} among their slots", self.most_field.get()));
        }
        // A grammar's `remember`: each level's word (`REMEMBER`), the kind a reading's `as C` reads its node as.
        if !self.remember.is_empty() {
            for (a, b) in [
                ("    flags: u32,\n    depth: u32,\n", "    flags: u32,\n    /// The level's word (`remember`): the kind of the token of a `remember` choice it took last; a call's is its caller's\n    /// again where it returns.\n    word: u16,\n    depth: u32,\n"),
                ("flags: 0, depth: 0,", "flags: 0, word: 0, depth: 0,"),
                ("            Some(Some(k)) => {\n                let at = self.w.push(key & !0x3ff | u32::from(k), s);", "            Some(Some(k)) => {\n                let k = if k == REMEMBERED { self.word } else { k };\n                let at = self.w.push(key & !0x3ff | u32::from(k), s);"),
            ] {
                if out.matches(a).count() != 1 {
                    return Err(format!("tier 2 as Rust: `remember`'s place in the runtime: {a:?}"));
                }
                out = out.replace(a, b);
            }
            out.push_str("\n/// A reading's way to the level's word (`remember`).\nconst REMEMBERED: u16 = u16::MAX - 2;\n");
        }
        // A grammar's `refuse M in L`: the marks the level open refuses, tested at each token it takes.
        if !self.refusing.is_empty() {
            if self.refusing.values().any(|&b| b == u32::MAX) {
                return Err("tier 2 as Rust: `refuse` of a mark tier 1 gives no bit".into());
            }
            for (a, b) in [
                ("    asks: u64,\n", "    asks: u64,\n    /// The marks the level open refuses (`refuse M in L`), their bits in a record's code: a token it takes with one\n    /// refuses the file. A call's are its own, the caller's again where it returns.\n    refusing: u32,\n"),
                ("marked: false, asks: 0,", "marked: false, asks: 0, refusing: 0,"),
                ("        if self.i >= self.n || self.open_mark() {\n            return self.refuse();\n        }\n", "        if self.i >= self.n || self.open_mark() {\n            return self.refuse();\n        }\n        // A mark the level refuses, at the token it takes (`refuse M in L`).\n        if self.k == 0 && self.refusing != 0 && self.marks_hold(self.i, self.refusing) {\n            return self.refuse();\n        }\n"),
            ] {
                if out.matches(a).count() != 1 {
                    return Err(format!("tier 2 as Rust: `refuse`'s place in the runtime: {a:?}"));
                }
                out = out.replace(a, b);
            }
        }
        // A grammar's `provisional` fields: the root closed, a node that holds one refuses the file.
        if !self.provisional.is_empty() {
            let fields: Vec<String> = self.provisional.iter().map(|f| self.field_id(f).map(|id| id.to_string())).collect::<Result<_, _>>()?;
            let a = "        self.w.set_start(0, first);\n";
            if out.matches(a).count() != 1 {
                return Err("tier 2 as Rust: `provisional`'s place in the runtime".into());
            }
            out = out.replace(a, &format!("        // A field that a later token decides (`provisional`), held still at the file's end: refused.\n        if (0..self.w.len()).any(|i| matches!((self.w.key(i) >> 10 & 0x3f) as u16, {})) {{\n            return self.refuse();\n        }}\n{a}", fields.join(" | ")));
        }
        // A grammar's `drop`: the nodes it took out, held as dead where a reading may reach them (`drop_made`).
        if rules.contains("self.drop_made(") {
            for (a, b) in [
                ("    gone: Vec<usize>,\n", "    gone: Vec<usize>,\n    /// The nodes a `drop` took out, each by the node it was in: a reading that reaches one refuses (`read_dead`).\n    dead: Vec<u32>,\n"),
                ("gone: Vec::new(), lost: None,", "gone: Vec::new(), dead: Vec::new(), lost: None,"),
                ("        self.w.insert(at, u32::from(kind) | (k & (0x3f << 10)), start);\n", "        self.w.insert(at, u32::from(kind) | (k & (0x3f << 10)), start);\n        for o in &mut self.dead {\n            if *o != NONE && *o as usize >= at {\n                *o += 1;\n            }\n        }\n"),
                ("        let nodes: Vec<(u32, u32, u32, usize)> = (at..at + n).map(|i| (self.w.key(i), self.w.start(i), self.w.end(i), self.w.size(i) as usize)).collect();\n", "        let nodes: Vec<(u32, u32, u32, usize)> = (at..at + n).map(|i| (self.w.key(i), self.w.start(i), self.w.end(i), self.w.size(i) as usize)).collect();\n        if self.dead.iter().any(|&o| (o as usize).wrapping_sub(at) < n) {\n            self.read_dead(&nodes, at, root, way)?;\n        }\n"),
            ] {
                if out.matches(a).count() != 1 {
                    return Err(format!("tier 2 as Rust: `drop`'s place in the runtime: {a:?}"));
                }
                out = out.replace(a, b);
            }
        }
        // The helpers only some grammars' rules call, each written where it is called.
        let used: Vec<&str> = MORE.iter().filter(|(name, _)| rules.contains(&format!("self.{name}("))).map(|(_, code)| *code).collect();
        if !used.is_empty() {
            out.push_str("\n#[allow(clippy::all, dead_code)]\nimpl<L: Layout> P<'_, L> {\n");
            out.push_str(&used.join("\n"));
            out.push_str("}\n");
        }
        // No mark drops or loses an extra (`drops`, `loses`, decision 0623 step 7): tier 2 keeps no extra gone or
        // lost, and a taken token checks none.
        if ["self.drop_pending()", "self.lose_pending()", "self.move_first()"].iter().all(|c| !out.contains(c)) {
            for (a, b) in [
                ("    /// The extras a mark's question dropped or lost (`drops`, `loses`), by record; the one lost,\n    /// its kind and, once its token is taken, that token's start: a pending extra of no width.\n    gone: Vec<usize>,\n", ""),
                ("    lost: Option<(u16, Option<u32>)>,\n    /// The extra a mark's `loses since` moved, by record, and where: of no width there.\n    moved: Option<(usize, u32)>,\n", ""),
                ("gone: Vec::new(), ", ""),
                ("lost: None, moved: None, ", ""),
                ("        if let Some((kind, Some(s))) = self.lost {\n            self.lost = None;\n            let at = self.w.push(u32::from(kind) | 1 << 16, s);\n            self.w.close(at, s);\n        }\n", ""),
                ("        if let Some((kind, None)) = self.lost {\n            self.lost = Some((kind, Some(self.start())));\n        }\n", ""),
                ("        // An extra a `loses since` moved and the one after it go with those before: the lean\n        // lexer's semicolon is past that one.\n        let moved = self.moved.map(|(m, _)| m).filter(|&m| m >= self.pend && m < self.i);\n        let through = moved.and_then(|m| (m + 1..self.i).find(|&k| is_extra(self.recs[k].code) && !self.gone.contains(&k)));\n", ""),
                (" && Some(j) != moved && Some(j) != through {", " {"),
                ("            if is_extra(c) && !self.gone.contains(&j) {\n                let (s, e) = self.extra_span(j);\n", "            if is_extra(c) {\n                let (s, e) = (self.recs[j].start, self.recs[j].last + 1);\n"),
                ("    /// Extra `j`'s span: its bytes, or of no width where a mark's `loses since` moved it.\n    fn extra_span(&self, j: usize) -> (u32, u32) {\n        match self.moved {\n            Some((m, s)) if m == j => (s, s),\n            _ => (self.recs[j].start, self.recs[j].last + 1),\n        }\n    }\n\n", ""),
                ("    /// `loses since`: the first extra pending of no width at the next one's start (the lean\n    /// lexer's `asi` reads a block comment through to another comment and loses its bytes there).\n    fn move_first(&mut self) {\n        let mut pending = (self.pend..self.i).filter(|&j| is_extra(self.recs[j].code) && !self.gone.contains(&j));\n        if let (Some(a), Some(b)) = (pending.next(), pending.next()) {\n            self.moved = Some((a, self.recs[b].start));\n        }\n    }\n\n", ""),
                ("    /// The extra pending last before the token (not yet written, not dropped or lost).\n    fn pending_last(&self) -> Option<usize> {\n        (self.pend..self.i).rev().find(|&j| is_extra(self.recs[j].code) && !self.gone.contains(&j))\n    }\n\n    /// `drops`: the extra pending last gone; with none left pending, the nodes closed next end\n    /// at the token, where the semicolon of no width is (the lean lexer's `Asi::Swallow`).\n    fn drop_pending(&mut self) {\n        if let Some(j) = self.pending_last() {\n            self.gone.push(j);\n            if self.pending_last().is_none() {\n                self.last_end = self.start();\n            }\n        }\n    }\n\n    /// `loses`: the extra pending last made again after the token is taken, of no width at its\n    /// start (the lean lexer's `asi` loses it); once a token.\n    fn lose_pending(&mut self) {\n        if self.lost.is_some() {\n            return;\n        }\n        if let Some(j) = self.pending_last() {\n            self.gone.push(j);\n            self.lost = Some((extra_kind(self.recs[j].code), None));\n        }\n    }\n\n", ""),
            ] {
                if !out.contains(a) {
                    return Err(format!("tier 2: the runtime's lost extras, not where they were: {a:?}"));
                }
                out = out.replace(a, b);
            }
        }
        // No region read in parts (decision 0619, as 0586's): tier 1 gives no mask, and tier 2 holds none, splits no
        // record, and writes only the helpers its code calls.
        if crate::bitlex_lex_rust::readers(self.x.lexer).is_empty() {
            for (a, b) in [
                ("    /// The pieces of record `i` where tier 2 splits it (a string, decision 0323's lowering B), as\n    /// the lean parser's string arm reads its pieces: each one's first and last byte and code;\n    /// the token is piece `k`.\n    sub: Vec<(u32, u32, u32)>,\n    k: usize,\n", ""),
                ("sub: Vec::new(), k: 0, ", ""),
                ("    /// A piece of the record split, `s..=l`, of record kind `kind` and the word its bytes are.\n    fn piece(&mut self, s: usize, l: usize, kind: u32) {\n        let word = word_in(kind, &self.src[s..=l]);\n        self.sub.push((s as u32, l as u32, kind | word << 8));\n    }\n\n", ""),
                ("    /// Record `j`'s code, its first and its last byte: the piece's at the token.\n    #[inline(always)]\n    fn code_at(&self, j: usize) -> u32 {\n        if j == self.i && self.k < self.sub.len() { self.sub[self.k].2 } else { self.recs[j].code }\n    }\n", "    /// Record `j`'s code.\n    #[inline(always)]\n    fn code_at(&self, j: usize) -> u32 {\n        self.recs[j].code\n    }\n"),
                ("        } else if self.k < self.sub.len() {\n            self.sub[self.k].0\n", ""),
                ("        if self.k < self.sub.len() {\n            let (s, l, _) = self.sub[self.k];\n            self.k += 1;\n            if self.k < self.sub.len() {\n                self.known = 0;\n                self.last_end = l + 1;\n                return Ok((s, l + 1));\n            }\n            self.sub.clear();\n            self.k = 0;\n            self.advance(l as usize + 1)?;\n            self.last_end = l + 1;\n            return Ok((s, l + 1));\n        }\n", ""),
                ("    /// Tier 1's masks, a bit a byte: those a region's parts are found by (`split`).\n    m: &'a crate::tier1::Masks,\n", ""),
                ("let (recs, m) = (&lx.recs[..], &lx.masks);", "let recs = &lx.recs[..];"),
                ("P { src, recs, m, n,", "P { src, recs, n,"),
                ("    /// At record `i`: split it where tier 2 reads its region in parts.\n", "    /// At record `i`: its code read, no mark known there yet.\n"),
                ("        if self.i < self.n {\n            self.split(self.i)?;\n        }\n", "        self.cur = if self.i < self.n { self.recs[self.i].code } else { 0 };\n"),
                // The token's code held as the token is reached (a token is its record), as the lean parser holds its
                // token: its word read of it.
                ("    known: u64,\n    holds: u64,\n", "    known: u64,\n    holds: u64,\n    /// The token's code (none past the last record), read as the token is reached.\n    cur: u32,\n"),
                ("known: 0, holds: 0, ", "known: 0, holds: 0, cur: 0, "),
                ("    fn word(&self) -> u32 {\n        self.word_at(self.i)\n    }\n", "    fn word(&self) -> u32 {\n        self.cur >> 8 & 0xff\n    }\n"),
                ("    /// Record `j` split where its first byte opens a region tier 2 reads in parts.\n    #[inline(always)]\n    fn split(&mut self, j: usize) -> R {\n        let s = self.recs[j].start as usize;\n        let _ = s;\n        Ok(())\n    }\n\n", ""),
            ] {
                if !out.contains(a) {
                    return Err(format!("tier 2: the runtime's masks, not where they were: {a:?}"));
                }
                out = out.replacen(a, b, 1);
            }
            let parts: Vec<String> = RUNTIME.split("\n\n").map(|p| p.trim_matches('\n').to_owned()).filter(|p| !p.is_empty() && p.matches('{').count() == p.matches('}').count()).collect();
            out = out.replace("if self.k == 0 && self.refusing != 0 ", "if self.refusing != 0 ");
            out = one_loop::drop_uncalled(out, &parts);
        }
        let masks = if crate::bitlex_lex_rust::readers(self.x.lexer).is_empty() { "" } else { " and masks (those a region's parts are found by)" };
        let _ = writeln!(out, "\n/// The tape of `src` from tier 1's records (each token's first and last bytes and code){masks}, finished by `finish` (the\n/// image, or the columns, as the grammar's lean parser finishes), or the byte it was refused at.\npub fn parse<F: rerail_tape_format::Finish>(src: &[u8], lx: &crate::tier1::Lexed, finish: &mut F) -> Result<F::Out, u32> {{\n    // Where tier 1 sends the file to tree-sitter.\n    if lx.refused {{\n        return Err(0);\n    }}\n    let mut widths = Widths::guess(src.len());\n    loop {{\n        // Nearly every file is parsed at the narrow widths, fixed in the parser's code, as the lean parser's.\n        let at = if widths == Narrow::WIDTHS {{ parse_at(src, lx, Narrow, finish)? }} else {{ parse_at(src, lx, widths, finish)? }};\n        match at {{\n            Ok(c) => return Ok(c),\n            Err(rerail_tape_format::Redo(w)) => {{\n                if w == widths {{\n                    return Err(u32::MAX);\n                }}\n                widths = w;\n            }}\n        }}\n    }}\n}}\n\n/// `parse` at `layout`'s widths: the tape `finish` makes of the writer, or the widths to parse at again; or the byte\n/// it was refused at.\nfn parse_at<L: Layout, F: rerail_tape_format::Finish>(src: &[u8], lx: &crate::tier1::Lexed, layout: L, finish: &mut F) -> R<Result<F::Out, rerail_tape_format::Redo>> {{\n    let mut p = P::new(src, lx, layout)?;\n    p.{entry}()?;\n    Ok(finish.finish(p.w))\n}}");
        Ok(out)
    }
}

/// The parser's helpers that only some grammars' rules call (`relabel`, `drop`), by name: each written into a parser
/// whose rules call it.
const MORE: &[(&str, &str)] = &[
    (
        "relabel_inner",
        r#"    /// `relabel … inner G`: the node the node made last at this level, a wrap, holds first takes field `G`.
    fn relabel_inner(&mut self, field: u16) -> R {
        let at = *self.made.last().unwrap_or(&NONE);
        if at == NONE || self.w.size(at as usize) < 2 {
            return self.refuse();
        }
        let at = at as usize + 1;
        let k = self.w.key(at);
        self.w.set_key(at, k & !(0x3f << 10) | u32::from(field) << 10);
        Ok(())
    }
"#,
    ),
    (
        "relabel_field",
        r#"    /// `relabel F`: the node made last at this level takes field `F`.
    fn relabel_field(&mut self, field: u16) -> R {
        let at = *self.made.last().unwrap_or(&NONE);
        if at == NONE {
            return self.refuse();
        }
        let k = self.w.key(at as usize);
        self.w.set_key(at as usize, k & !(0x3f << 10) | u32::from(field) << 10);
        Ok(())
    }
"#,
    ),
    (
        "relabel_kind",
        r#"    /// `relabel … as M`: the node made last at this level a `kind` now.
    fn relabel_kind(&mut self, kind: u16) -> R {
        let at = *self.made.last().unwrap_or(&NONE);
        if at == NONE {
            return self.refuse();
        }
        let k = self.w.key(at as usize);
        self.w.set_key(at as usize, k & !0x3ff | u32::from(kind));
        Ok(())
    }
"#,
    ),
    (
        "drop_made",
        r#"    /// `drop`: the node made last at this level goes, the nodes inside it in its place; none is made last at this
    /// level until the next.
    ///
    /// As the machine's `drop` leaves its record of kind `DEAD` in the table (left out at `ACCEPT`), the node is held
    /// as dead in the node open (`dead`), so that a reading that reaches it refuses. A dead node in it goes to that
    /// node too.
    fn drop_made(&mut self) -> R {
        let at = *self.made.last().unwrap_or(&NONE);
        let open = *self.open.last().unwrap_or(&NONE);
        for o in &mut self.dead {
            if *o == at {
                *o = open;
            }
        }
        self.reread(Some(None), |_| None, |f| f)?;
        self.dead.push(open);
        self.set_made(NONE);
        Ok(())
    }

    /// The dead nodes (`drop`) in the node read again from `at` (`nodes`, as `reread` has them): one in a node the
    /// reading reaches, not one it keeps whole, refuses, as the machine's reading refuses the `DEAD` record it
    /// reaches (it names no such kind). Each other one is then in its node where the reading writes it.
    fn read_dead(&mut self, nodes: &[(u32, u32, u32, usize)], at: usize, root: Option<Option<u16>>, way: fn(u16) -> Option<Option<u16>>) -> R {
        let (mut to, mut reached) = (vec![NONE; nodes.len()], vec![false; nodes.len()]);
        let (mut next, mut kept) = (at as u32, 0);
        for (i, &(key, _, _, size)) in nodes.iter().enumerate() {
            let way = if i < kept || key >> 16 & 1 == 1 {
                None
            } else if i == 0 {
                match root {
                    Some(Some(u16::MAX)) => way((key & 0x3ff) as u16),
                    r => r,
                }
            } else {
                way((key & 0x3ff) as u16)
            };
            match way {
                None => kept = kept.max(i + size),
                Some(_) => reached[i] = true,
            }
            if way != Some(None) {
                to[i] = next;
                next += 1;
            }
        }
        for o in &mut self.dead {
            let j = (*o as usize).wrapping_sub(at);
            if j < nodes.len() {
                if reached[j] {
                    return self.refuse();
                }
                *o = to[j];
            }
        }
        Ok(())
    }
"#,
    ),
];

/// The parser's own code: the tokens, the tree, the comments.
const RUNTIME: &str = r#"
type R<X = ()> = Result<X, u32>;

/// Mask `x`'s first bit at or after byte `from`, a word at a time.
fn first_bit(x: &[u64], from: usize) -> Option<usize> {
    let mut q = from / 64;
    let mut w = x.get(q)? & (!0u64 << (from % 64));
    while w == 0 {
        q += 1;
        w = *x.get(q)?;
    }
    Some(64 * q + w.trailing_zeros() as usize)
}

/// Whether mask `x` holds at byte `i`.
fn bit(x: &[u64], i: usize) -> bool {
    x.get(i / 64).is_some_and(|w| w >> (i % 64) & 1 == 1)
}

/// No node made yet at a level.
const NONE: u32 = u32::MAX;
/// The kind of a node open whose kind its rule gives later (`retag`).
const UNSET: u16 = 0x3ff;
/// The deepest the calls go.
const DEPTH: u32 = 400;

/// The parser: tier 1's records (the tokens and the comments), the token read next, the tree.
pub struct P<'a, L: Layout> {
    src: &'a [u8],
    /// Tier 1's records, one a token: its first and last byte and its code (the lean parser's `Tok`).
    recs: &'a [crate::tier1::Rec],
    /// Tier 1's masks, a bit a byte: those a region's parts are found by (`split`).
    m: &'a crate::tier1::Masks,
    n: usize,
    /// The next grammar token's record; the first record not written yet (the comments before it).
    i: usize,
    /// The pieces of record `i` where tier 2 splits it (a string, decision 0323's lowering B), as
    /// the lean parser's string arm reads its pieces: each one's first and last byte and code;
    /// the token is piece `k`.
    sub: Vec<(u32, u32, u32)>,
    k: usize,
    pend: usize,
    w: Writer<L>,
    /// The nodes open, and the node made last at each.
    open: Vec<u32>,
    made: Vec<u32>,
    /// The last token's end.
    last_end: u32,
    /// The marks taken or passed at this token, by their order.
    taken: u64,
    /// The marks found at this token (`held`): which, and whether each holds.
    known: u64,
    holds: u64,
    flags: u32,
    depth: u32,
    /// A mark taken last, no token or rule's end since (`G::after_mark`).
    marked: bool,
    /// The marks whose questions a level that may end at them asks (`ask_M`), by order: those
    /// of the caller's level, as the lean parser's `stmt`.
    asks: u64,
    /// The extras a mark's question dropped or lost (`drops`, `loses`), by record; the one lost,
    /// its kind and, once its token is taken, that token's start: a pending extra of no width.
    gone: Vec<usize>,
    lost: Option<(u16, Option<u32>)>,
    /// The extra a mark's `loses since` moved, by record, and where: of no width there.
    moved: Option<(usize, u32)>,
    /// The strings open over a template's code (decision 0600): their parts go on at the next
    /// opener of a reader that resumes them (`advance`).
    strings: usize,
}

#[allow(dead_code)]
impl<'a, L: Layout> P<'a, L> {
    fn new(src: &'a [u8], lx: &'a crate::tier1::Lexed, widths: L) -> R<Self> {
        let (recs, m) = (&lx.recs[..], &lx.masks);
        let n = recs.len();
        let mut p = P { src, recs, m, n, i: 0, sub: Vec::new(), k: 0, pend: 0, w: Writer::new(0, widths, src.len() / 3 + 16), open: Vec::with_capacity(64), made: Vec::with_capacity(64), last_end: 0, taken: 0, known: 0, holds: 0, flags: 0, depth: 0, marked: false, asks: 0, gone: Vec::new(), lost: None, moved: None, strings: 0 };
        p.i = p.next_tok(0);
        p.arrive()?;
        Ok(p)
    }

    /// At record `i`: split it where tier 2 reads its region in parts.
    #[inline(always)]
    fn arrive(&mut self) -> R {
        self.known = 0;
        if self.i < self.n {
            self.split(self.i)?;
        }
        Ok(())
    }

    /// A piece of the record split, `s..=l`, of record kind `kind` and the word its bytes are.
    fn piece(&mut self, s: usize, l: usize, kind: u32) {
        let word = word_in(kind, &self.src[s..=l]);
        self.sub.push((s as u32, l as u32, kind | word << 8));
    }

    /// Record `j`'s code, its first and its last byte: the piece's at the token.
    #[inline(always)]
    fn code_at(&self, j: usize) -> u32 {
        if j == self.i && self.k < self.sub.len() { self.sub[self.k].2 } else { self.recs[j].code }
    }

    /// Whether mark `order` holds at the token (`h`), found once a token, as the lean parser
    /// keeps its `asi`.
    #[inline(always)]
    fn held(&mut self, order: u32, h: fn(&Self) -> bool) -> bool {
        if self.known >> order & 1 == 0 {
            self.known |= 1 << order;
            let v = h(self);
            self.holds = self.holds & !(1 << order) | u64::from(v) << order;
        }
        self.holds >> order & 1 == 1
    }

    #[inline(always)]
    fn next_tok(&self, mut j: usize) -> usize {
        while j < self.n && is_extra(self.recs[j].code) {
            j += 1;
        }
        j.min(self.n)
    }

    /// The grammar token before record `j` (`n`: none).
    #[inline(always)]
    fn prev_tok(&self, j: usize) -> usize {
        let mut k = j;
        while k > 0 {
            k -= 1;
            if !is_extra(self.recs[k].code) {
                return k;
            }
        }
        self.n
    }

    #[inline(always)]
    fn word(&self) -> u32 {
        self.word_at(self.i)
    }

    #[inline(always)]
    fn word_at(&self, j: usize) -> u32 {
        if j < self.n { self.code_at(j) >> 8 & 0xff } else { 0 }
    }

    /// Mark `b` at record `j`: its code's bit, else, for a gap mark tier 1 leaves to tier 2 (`tier1::ASKED`), the bytes.
    #[inline(always)]
    fn bit_at(&self, j: usize, b: u32) -> bool {
        j < self.n && (self.code_at(j) >> (16 + b) & 1 == 1 || crate::tier1::ASKED >> b & 1 == 1 && self.gap(j, b))
    }

    /// Gap mark `b` at record `j`, asked of the bytes since the grammar token before's first byte, as tier 1 reads a
    /// gap and the lean parser asks whether a token is on a line.
    /// An extra keeps none of them.
    fn gap(&self, j: usize, b: u32) -> bool {
        if is_extra(self.recs[j].code) {
            return false;
        }
        let p = self.prev_tok(j);
        let (from, before) = if p < self.n { (self.recs[p].start as usize + 1, self.word_at(p)) } else { (0, 0) };
        crate::tier1::gap(self.src, from, self.recs[j].start as usize, before, b)
    }

    /// Whether one of the marks `bits` holds at record `j`: its code's, and the gap marks tier 2 asks of the bytes.
    fn marks_hold(&self, j: usize, bits: u32) -> bool {
        if self.recs[j].code >> 16 & bits != 0 {
            return true;
        }
        let mut ask = crate::tier1::ASKED & bits;
        while ask != 0 {
            if self.gap(j, ask.trailing_zeros()) {
                return true;
            }
            ask &= ask - 1;
        }
        false
    }

    #[inline(always)]
    fn start(&self) -> u32 {
        if self.i >= self.n {
            self.src.len() as u32
        } else if self.k < self.sub.len() {
            self.sub[self.k].0
        } else {
            self.recs[self.i].start
        }
    }

    #[inline(always)]
    fn at_end(&self) -> bool {
        self.i >= self.n
    }

    #[cold]
    fn refuse<X>(&self) -> R<X> {
        // `RERAIL_TIER2_TRACE`: where, as the lean parser's `RERAIL_KOTLIN_TRACE` says it.
        #[cfg(debug_assertions)]
        if std::env::var_os("RERAIL_TIER2_TRACE").is_some() {
            eprintln!("refused at {}:\n{}", self.start(), std::backtrace::Backtrace::force_capture());
        }
        Err(self.start())
    }

    fn deeper(&mut self) -> R {
        self.depth += 1;
        if self.depth > DEPTH {
            return self.refuse();
        }
        Ok(())
    }

    /// The comments before the token, written into the node open: the one a mark lost first.
    #[inline(always)]
    fn flush(&mut self) {
        if let Some((kind, Some(s))) = self.lost {
            self.lost = None;
            let at = self.w.push(u32::from(kind) | 1 << 16, s);
            self.w.close(at, s);
        }
        while self.pend < self.i {
            let j = self.pend;
            self.pend += 1;
            let c = self.recs[j].code;
            if is_extra(c) && !self.gone.contains(&j) {
                let (s, e) = self.extra_span(j);
                let at = self.w.push(u32::from(extra_kind(c)) | 1 << 16, s);
                self.w.close(at, e);
            }
        }
    }

    /// Extra `j`'s span: its bytes, or of no width where a mark's `loses since` moved it.
    fn extra_span(&self, j: usize) -> (u32, u32) {
        match self.moved {
            Some((m, s)) if m == j => (s, s),
            _ => (self.recs[j].start, self.recs[j].last + 1),
        }
    }

    /// `loses since`: the first extra pending of no width at the next one's start (the lean
    /// lexer's `asi` reads a block comment through to another comment and loses its bytes there).
    fn move_first(&mut self) {
        let mut pending = (self.pend..self.i).filter(|&j| is_extra(self.recs[j].code) && !self.gone.contains(&j));
        if let (Some(a), Some(b)) = (pending.next(), pending.next()) {
            self.moved = Some((a, self.recs[b].start));
        }
    }

    /// The comments pending before the line break of settling mark `bit` (an extra's code holds
    /// the bits of its own gap): written now, into the node open, the node's end past them.
    fn settle(&mut self, bit: u32) {
        // An extra a `loses since` moved and the one after it go with those before: the lean
        // lexer's semicolon is past that one.
        let moved = self.moved.map(|(m, _)| m).filter(|&m| m >= self.pend && m < self.i);
        let through = moved.and_then(|m| (m + 1..self.i).find(|&k| is_extra(self.recs[k].code) && !self.gone.contains(&k)));
        while self.pend < self.i {
            let j = self.pend;
            let c = self.recs[j].code;
            if is_extra(c) && c >> (16 + bit) & 1 == 1 && Some(j) != moved && Some(j) != through {
                break;
            }
            self.pend += 1;
            if is_extra(c) && !self.gone.contains(&j) {
                let (s, e) = self.extra_span(j);
                let at = self.w.push(u32::from(extra_kind(c)) | 1 << 16, s);
                self.w.close(at, e);
                self.last_end = self.last_end.max(e);
            }
        }
    }

    /// The extra pending last before the token (not yet written, not dropped or lost).
    fn pending_last(&self) -> Option<usize> {
        (self.pend..self.i).rev().find(|&j| is_extra(self.recs[j].code) && !self.gone.contains(&j))
    }

    /// `drops`: the extra pending last gone; with none left pending, the nodes closed next end
    /// at the token, where the semicolon of no width is (the lean lexer's `Asi::Swallow`).
    fn drop_pending(&mut self) {
        if let Some(j) = self.pending_last() {
            self.gone.push(j);
            if self.pending_last().is_none() {
                self.last_end = self.start();
            }
        }
    }

    /// `loses`: the extra pending last made again after the token is taken, of no width at its
    /// start (the lean lexer's `asi` loses it); once a token.
    fn lose_pending(&mut self) {
        if self.lost.is_some() {
            return;
        }
        if let Some(j) = self.pending_last() {
            self.gone.push(j);
            self.lost = Some((extra_kind(self.recs[j].code), None));
        }
    }

    /// The token taken: its span; the next token's marks not yet taken.
    #[inline(always)]
    fn bump(&mut self) -> R<(u32, u32)> {
        if self.i >= self.n || self.open_mark() {
            return self.refuse();
        }
        if let Some((kind, None)) = self.lost {
            self.lost = Some((kind, Some(self.start())));
        }
        self.taken = 0;
        self.marked = false;
        if self.k < self.sub.len() {
            let (s, l, _) = self.sub[self.k];
            self.k += 1;
            if self.k < self.sub.len() {
                self.known = 0;
                self.last_end = l + 1;
                return Ok((s, l + 1));
            }
            self.sub.clear();
            self.k = 0;
            self.advance(l as usize + 1)?;
            self.last_end = l + 1;
            return Ok((s, l + 1));
        }
        let (s, e) = (self.recs[self.i].start, self.recs[self.i].last + 1);
        self.advance(e as usize)?;
        self.last_end = e;
        Ok((s, e))
    }

    #[inline(always)]
    fn set_made(&mut self, at: u32) {
        if let Some(m) = self.made.last_mut() {
            *m = at;
        }
    }

    /// The token, which must pass `ok`, a leaf of `kind` given `field`.
    #[inline(always)]
    fn tk(&mut self, ok: bool, kind: u16, field: u16) -> R {
        if !ok {
            return self.refuse();
        }
        self.flush();
        let (s, e) = self.bump()?;
        let at = self.w.push(u32::from(kind) | u32::from(field) << 10, s);
        self.w.close(at, e);
        self.set_made(at as u32);
        Ok(())
    }

    /// The token, which must be word `w`, a leaf of `kind`.
    #[inline(always)]
    fn word_leaf(&mut self, w: u32, kind: u16) -> R {
        self.tk(self.word() == w, kind, 0)
    }

    /// The token, which must be word `w`: one the grammar implies, no leaf.
    #[inline(always)]
    fn word_skip(&mut self, w: u32) -> R {
        self.skip(self.word() == w)
    }

    /// The token, which must pass `ok`: one the grammar implies, no leaf.
    #[inline(always)]
    fn skip(&mut self, ok: bool) -> R {
        if !ok {
            return self.refuse();
        }
        self.flush();
        self.bump()?;
        Ok(())
    }

    /// The token, which must pass `ok`, joined to the leaf made last: one leaf of `kind`.
    fn merge(&mut self, ok: bool, kind: u16) -> R {
        let at = *self.made.last().unwrap_or(&NONE);
        if !ok || at == NONE || at as usize + 1 != self.w.len() {
            return self.refuse();
        }
        let (s, key) = (self.w.start(at as usize), self.w.key(at as usize));
        // The comments between are its text.
        self.pend = self.i;
        let (_, e) = self.bump()?;
        self.w.truncate(at as usize);
        let at = self.w.push(key & !0x3ff | u32::from(kind), s);
        self.w.close(at, e);
        self.set_made(at as u32);
        Ok(())
    }

    fn open_root(&mut self, kind: u16) {
        let at = self.w.push(u32::from(kind), 0);
        self.open.push(at as u32);
        self.made.push(NONE);
    }

    /// The root closed at the end of the file: from the file's start (`whole`), else (or `whole padded`) from its
    /// first token or comment, as tree-sitter's root starts past the file's leading blanks.
    fn close_root(&mut self, whole: bool, padded: bool) -> R {
        if self.i < self.n {
            return self.refuse();
        }
        self.flush();
        // From its first token or comment (none: the file's end), to the file's end (`whole`) or its last token's.
        let first = if whole && !padded {
            0
        } else if self.w.len() > 1 {
            self.w.start(1)
        } else {
            self.src.len() as u32
        };
        let end = if whole { self.src.len() as u32 } else { self.last_end };
        self.w.set_start(0, first);
        self.w.close(0, end);
        Ok(())
    }

    /// A node of `kind`, open at the token.
    #[inline(always)]
    fn open(&mut self, kind: u16) {
        self.flush();
        let at = self.w.push(u32::from(kind), self.start());
        self.open.push(at as u32);
        self.made.push(NONE);
    }

    /// The node open closed at the last token's end; the node made last at its parent's level.
    /// One still of no kind (`UNSET`, never retagged) refuses the file.
    #[inline(always)]
    fn close(&mut self) -> R {
        let at = self.open.pop().expect("a node open") as usize;
        if self.w.key(at) & 0x3ff == u32::from(UNSET) {
            return self.refuse();
        }
        self.made.pop();
        let e = self.last_end.max(self.w.start(at));
        self.w.close(at, e);
        self.set_made(at as u32);
        Ok(())
    }

    /// The node made last at this level given `field`, where it has none.
    fn field_made(&mut self, field: u16) {
        let at = *self.made.last().unwrap_or(&NONE);
        if at == NONE {
            return;
        }
        let k = self.w.key(at as usize);
        if k >> 10 & 0x3f == 0 {
            self.w.set_key(at as usize, k | u32::from(field) << 10);
        }
    }

    /// The node open, a `kind` now.
    fn retag(&mut self, kind: u16) {
        let at = *self.open.last().expect("a node open") as usize;
        let k = self.w.key(at);
        self.w.set_key(at, k & !0x3ff | u32::from(kind));
    }

    /// A node of `kind` put over the node made last at this level, open: it takes that node's
    /// field, and that node takes `field`.
    fn wrap(&mut self, kind: u16, field: u16) -> R {
        let at = *self.made.last().unwrap_or(&NONE);
        if at == NONE {
            return self.refuse();
        }
        let at = at as usize;
        let k = self.w.key(at);
        self.w.set_key(at, k & !(0x3f << 10) | u32::from(field) << 10);
        let start = self.w.start(at);
        self.w.insert(at, u32::from(kind) | (k & (0x3f << 10)), start);
        self.open.push(at as u32);
        self.made.push(at as u32 + 1);
        Ok(())
    }

    /// The node a wrap moved (the wrap's first child) a `kind` now.
    fn retype_moved(&mut self, kind: u16) {
        let at = *self.open.last().expect("a wrap open") as usize + 1;
        let k = self.w.key(at);
        self.w.set_key(at, k & !0x3ff | u32::from(kind));
    }

    /// The node made last at this level and the nodes inside it read again: `way` gives each
    /// kind's (`Some(Some(K))` read as `K`, `Some(None)` gone and its children in its place,
    /// `None` kept as it is with every node inside it; `u16::MAX - 1` refuses), `field` each
    /// field's new one, and `root` the node's own way (`u16::MAX`: its kind's).
    fn reread(&mut self, root: Option<Option<u16>>, way: fn(u16) -> Option<Option<u16>>, field: fn(u16) -> u16) -> R {
        let at = *self.made.last().unwrap_or(&NONE);
        if at == NONE {
            return self.refuse();
        }
        let at = at as usize;
        let n = self.w.size(at) as usize;
        if at + n != self.w.len() {
            return self.refuse();
        }
        // The nodes as they are, then written again in preorder.
        let nodes: Vec<(u32, u32, u32, usize)> = (at..at + n).map(|i| (self.w.key(i), self.w.start(i), self.w.end(i), self.w.size(i) as usize)).collect();
        self.w.truncate(at);
        let mut i = 0;
        while i < nodes.len() {
            i = self.reread_node(&nodes, i, root, way, field)?;
        }
        if self.w.len() == at + 1 || at < self.w.len() {
            self.set_made(at as u32);
        }
        Ok(())
    }

    /// `reread`'s node `i` and the nodes inside it: the node after them.
    fn reread_node(&mut self, nodes: &[(u32, u32, u32, usize)], i: usize, root: Option<Option<u16>>, way: fn(u16) -> Option<Option<u16>>, field: fn(u16) -> u16) -> R<usize> {
        let (key, s, e, size) = nodes[i];
        let kind = (key & 0x3ff) as u16;
        let to = if key >> 16 & 1 == 1 {
            None
        } else if i == 0 {
            match root {
                Some(Some(u16::MAX)) => way(kind),
                r => r,
            }
        } else {
            way(kind)
        };
        let key = if i == 0 { key } else { key & !(0x3f << 10) | u32::from(field((key >> 10 & 0x3f) as u16)) << 10 };
        match to {
            Some(Some(k)) if k == u16::MAX - 1 => self.refuse(),
            None => {
                let at = self.w.push(key, s);
                let mut j = i + 1;
                while j < i + size {
                    j = self.reread_node(nodes, j, Some(None), |_| None, |f| f)?;
                }
                self.w.close(at, e);
                Ok(j)
            }
            Some(None) => {
                let mut j = i + 1;
                while j < i + size {
                    j = self.reread_node(nodes, j, root, way, field)?;
                }
                Ok(j)
            }
            Some(Some(k)) => {
                let at = self.w.push(key & !0x3ff | u32::from(k), s);
                let mut j = i + 1;
                while j < i + size {
                    j = self.reread_node(nodes, j, root, way, field)?;
                }
                self.w.close(at, e);
                Ok(j)
            }
        }
    }
}
"#;

/// Tier 2's Rust, written from the tree rules.
pub fn tier2(x: &Input<'_>) -> Result<String, String> {
    G::new(x).source()
}
