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
    pub hidden: bool,
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
                _ => {}
            }
        }
        G { x, rules, order, flags, readings, firsts: Default::default(), busy: Default::default(), kinds: Default::default(), fields: Default::default(), kind_tests: Default::default(), follows: Default::default() }
    }

    fn mark_of(&self, name: &str) -> Option<usize> {
        self.x.marks.iter().position(|m| m.name == name)
    }

    fn word_of(&self, text: &str) -> Option<u8> {
        self.x.words.iter().position(|w| w.as_slice() == text.as_bytes()).map(|i| (i + 1) as u8)
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

    fn field_const(&self, name: &str) -> Result<String, String> {
        let id = (self.x.field)(name).ok_or_else(|| format!("no field `{name}`"))?;
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
            Gram::Wrap { body, .. } | Gram::Reread { body, .. } | Gram::Flag { body, .. } => self.first(body)?,
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
    /// test; then a `match` on the word, a kind's test a guard of its arm. `exits`: marks that
    /// leave the decision (`exit`, the code after it takes them, or the level ends there). None
    /// passes: `none`. An arm reached by two tests apart is numbered, its code written once.
    fn decide(&self, arms: &[(Vec<First>, String)], exits: &[usize], exit: &str, none: &str, pad: &str) -> String {
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
        for &m in exits {
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
        for t in 0..=n {
            if uses[t] > 0 {
                let _ = write!(out, "{q}{t} => {{\n{}{q}}}\n", indent(&body_of(t), &r));
            }
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
    fn exits(&self, rest: &[&Gram], rule: &str) -> Result<Vec<usize>, String> {
        let (fr, may_end) = self.rest_first(rest)?;
        let mut out: Vec<usize> = fr.iter().filter_map(|t| if let Test::Mark(m) = t.0 { Some(m) } else { None }).collect();
        if may_end {
            out.extend(self.follow(rule)?);
        }
        out.sort_unstable();
        out.dedup();
        Ok(out)
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
            Gram::As(k, a) => {
                let (test, _) = self.token_of(a).ok_or_else(|| format!("rule {rule}: `as {k}` of no token"))?;
                self.take_token(&test, Some(k), pad)?
            }
            Gram::Sym(n) if self.mark_of(n).is_some() => {
                let m = &self.x.marks[self.mark_of(n).expect("a mark")];
                format!("{pad}self.mark_{}()?;\n", ident(&m.name))
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
                let exits = self.exits(rest, rule)?;
                let arms: Vec<(Vec<First>, String)> = arms.into_iter().map(|(f, b, _)| (f, b)).collect();
                self.decide(&arms, &exits, "", "", pad)
            }
            Gram::Star(a) | Gram::Plus(a) => {
                let mut out = String::new();
                if let Gram::Plus(_) = g {
                    let r: Vec<&Gram> = std::iter::once(g).chain(rest.iter().copied()).collect();
                    out.push_str(&self.code(a, &r, rule, pad)?);
                }
                let r: Vec<&Gram> = std::iter::once(g).chain(rest.iter().copied()).collect();
                let arms = self.arms(a, &r, rule)?;
                let exits = self.exits(rest, rule)?;
                let arms: Vec<(Vec<First>, String)> = arms.into_iter().map(|(f, b, _)| (f, b)).collect();
                let d = self.decide(&arms, &exits, "break;\n", "break;\n", &deeper);
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
                let exits = if empty.is_some() { self.exits(rest, rule)? } else { Vec::new() };
                let arms: Vec<(Vec<First>, String)> = arms.into_iter().map(|(f, b, _)| (f, b)).collect();
                self.decide(&arms, &exits, &none, &none, pad)
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
                    let _ = writeln!(out, "{pad}self.retype_moved({});", self.kind_const(m)?);
                }
                out.push_str(&self.code(body, rest, rule, pad)?);
                let _ = writeln!(out, "{pad}self.close()?;");
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

    /// A token element: its test and its leaf's kind.
    fn token_of(&self, g: &Gram) -> Option<(Test, String)> {
        match g {
            Gram::Lit(t) => match self.word_of(t) {
                Some(w) => Some((Test::Word(w), t.clone())),
                None if self.is_token_kind(t) => Some((Test::Kind(t.clone()), t.clone())),
                None => None,
            },
            Gram::Sym(n) if self.mark_of(n).is_none() && !self.rules.contains_key(n.as_str()) && self.is_token_kind(n) => Some((Test::Kind(n.clone()), n.clone())),
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
        let (body, node, nested, whole) = match self.rules[name] {
            Stmt::Node { body, nested, whole, .. } => (body, true, *nested, *whole),
            Stmt::Choice { body, nested, .. } => (body, false, *nested, false),
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
            let _ = writeln!(out, "        self.close_root({whole})");
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
            out.push_str("        Ok(())\n");
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
                TokenElem::Starting(bytes) => and.push(format!("({j} < self.n && matches!(self.src[self.starts[{j}] as usize], {}))", bytes.iter().map(|b| format!("{b:#04x}")).collect::<Vec<_>>().join(" | "))),
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
        let k = self.x.marks.iter().find(|x| x.name == m && x.bit.is_some()).ok_or_else(|| format!("mark {m}: no bit in the code"))?;
        Ok(format!("{}", k.bit.expect("a bit")))
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

    /// Each mark's test, `m_NAME`: it holds at the token and is not taken there.
    fn marks(&self, out: &mut String) -> Result<(), String> {
        if self.x.marks.len() > 64 {
            return Err(format!("{} marks: a token's marks are one u64", self.x.marks.len()));
        }
        let mut done: HashSet<&str> = HashSet::new();
        let mut fns = String::new();
        for (order, m) in self.x.marks.iter().enumerate() {
            if !done.insert(m.name.as_str()) {
                continue;
            }
            let n = ident(&m.name);
            let defs: Vec<(usize, &Mark)> = self.x.marks.iter().enumerate().filter(|(_, x)| x.name == m.name).collect();
            let mut holds = Vec::new();
            for (d, (_, def)) in defs.iter().enumerate() {
                let mut c = Vec::new();
                if let Some((before, seq)) = def.tokens {
                    let pat = format!("pat_{n}_{d}");
                    self.run(seq, &pat, &mut fns)?;
                    c.push(format!("self.{pat}_0(self.i, self.prev_tok(self.i))"));
                    if !before.is_empty() {
                        let p = self.elem(before, "p")?;
                        let negated = matches!(before, [TokenElem::Not(_)]);
                        c.push(if negated { format!("{{ let p = self.prev_tok(self.i); p >= self.n || {p} }}") } else { format!("{{ let p = self.prev_tok(self.i); p < self.n && {p} }}") });
                    }
                } else if let Some(b) = def.bit {
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
                    let call = format!("self.{pat}_0(self.i, self.prev_tok(self.i))");
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
            let _ = writeln!(out, "    /// Mark `{}` holds at the token, not taken there; once a token (`held`).\n    fn m_{n}(&mut self) -> bool {{\n        self.taken >> {order} & 1 == 0 && self.held({order}, Self::h_{n})\n    }}\n\n    fn h_{n}(&self) -> bool {{\n        {}\n    }}\n", m.name, holds.join(" || "));
            let settle = if m.settles { m.bit.map(|b| format!("        self.settle({b});\n")).unwrap_or_default() } else { String::new() };
            let _ = writeln!(out, "    /// Mark `{}` taken: it and the marks before it passed at this token.\n    fn take_{n}(&mut self) {{\n{settle}        self.taken |= (2u64 << {order}) - 1;\n    }}\n", m.name);
            let _ = writeln!(out, "    /// Mark `{}` where the rule takes it: it must hold.\n    fn mark_{n}(&mut self) -> R {{\n        if !self.m_{n}() {{\n            return self.refuse();\n        }}\n        self.take_{n}();\n        Ok(())\n    }}\n", m.name);
        }
        out.push_str(&fns);
        // The marks no rule may pass: where one holds at a token taken, the file is refused.
        let open: Vec<String> = self.x.marks.iter().filter(|m| !m.hidden).map(|m| format!("self.m_{}()", ident(&m.name))).collect::<HashSet<_>>().into_iter().collect();
        let mut open = open;
        open.sort();
        let _ = writeln!(out, "    /// A mark no rule takes holds at the token (a mark not hidden).\n    fn open_mark(&mut self) -> bool {{\n        {}\n    }}\n", if open.is_empty() { "false".to_owned() } else { open.join(" || ") });
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
            let _ = writeln!(out, "    fn at_{n}_at(&self, j: usize) -> bool {{\n        if j >= self.n {{\n            return false;\n        }}\n        let c = self.codes[j];\n        {kinds}{words}\n    }}\n\n    fn at_{n}(&self) -> bool {{\n        self.at_{n}_at(self.i)\n    }}\n");
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
                    ::bitlex::syntax::ReadKind::As(m) => format!("Some(Some({}))", (self.x.kind)(m).ok_or_else(|| format!("reading {name}: no kind `{m}`"))?),
                    ::bitlex::syntax::ReadKind::Gone => "Some(None)".to_owned(),
                    ::bitlex::syntax::ReadKind::Keep => "None".to_owned(),
                };
                let _ = write!(arms, "            {id} => {to},\n");
            }
            for e in self.x.extras {
                let id = (self.x.kind)(e).ok_or_else(|| format!("no kind `{e}`"))?;
                let _ = write!(arms, "            {id} => None,\n");
            }
            let mut fields = String::new();
            for (a, b) in &r.fields {
                let (fa, fb) = ((self.x.field)(a).ok_or_else(|| format!("reading {name}: no field `{a}`"))?, (self.x.field)(b).ok_or_else(|| format!("reading {name}: no field `{b}`"))?);
                let _ = write!(fields, "            {fa} => {fb},\n");
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
        let mut marks = String::new();
        self.marks(&mut marks)?;
        let mut readings = String::new();
        self.readings(&mut readings)?;
        let mut kinds = String::new();
        self.kind_tests(&mut kinds);
        let mut extras = String::new();
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
        out.push_str("// Generated by the Bitlex compiler (`rerail_compiler::bitlex_rust`) from the grammar's tree rules. Do not edit.\n\n");
        out.push_str("use rerail_tape_format::{Widths, Writer};\n\n");
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
        out.push_str("\n#[allow(clippy::all, unused_parens, dead_code)]\nimpl P<'_> {\n");
        out.push_str(&rules);
        out.push_str(&marks);
        out.push_str(&kinds);
        out.push_str(&readings);
        out.push_str("}\n");
        let _ = writeln!(out, "\n/// The tape of `src` from tier 1's records (each token's first and last bytes and code), or the byte it was refused at.\npub fn parse(src: &[u8], starts: &[u32], lasts: &[u32], codes: &[u32]) -> Result<rerail_tape_format::Columns, u32> {{\n    let mut widths = Widths::guess(src.len());\n    loop {{\n        let mut p = P::new(src, starts, lasts, codes, widths);\n        p.{}()?;\n        match p.w.finish_columns() {{\n            Ok(c) => return Ok(c),\n            Err(rerail_tape_format::Redo(w)) => {{\n                if w == widths {{\n                    return Err(u32::MAX);\n                }}\n                widths = w;\n            }}\n        }}\n    }}\n}}", rule_fn(self.order.first().ok_or("no rule")?));
        Ok(out)
    }
}

/// The parser's own code: the tokens, the tree, the comments.
const RUNTIME: &str = r#"
type R<X = ()> = Result<X, u32>;

/// No node made yet at a level.
const NONE: u32 = u32::MAX;
/// The kind of a node open whose kind its rule gives later (`retag`).
const UNSET: u16 = 0x3ff;
/// The deepest the calls go.
const DEPTH: u32 = 400;

/// The parser: tier 1's records (the tokens and the comments), the token read next, the tree.
pub struct P<'a> {
    src: &'a [u8],
    starts: &'a [u32],
    lasts: &'a [u32],
    codes: &'a [u32],
    n: usize,
    /// The next grammar token's record; the first record not written yet (the comments before it).
    i: usize,
    pend: usize,
    w: Writer<Widths>,
    /// The nodes open, and the node made last at each.
    open: Vec<u32>,
    made: Vec<u32>,
    /// The last token's end.
    last_end: u32,
    /// The marks taken or passed at this token, by their order.
    taken: u64,
    /// The marks found at token `memo_at` (`held`): which, and whether each holds.
    memo_at: usize,
    known: u64,
    holds: u64,
    flags: u32,
    depth: u32,
}

#[allow(dead_code)]
impl<'a> P<'a> {
    fn new(src: &'a [u8], starts: &'a [u32], lasts: &'a [u32], codes: &'a [u32], widths: Widths) -> Self {
        let n = codes.len();
        let mut p = P { src, starts, lasts, codes, n, i: 0, pend: 0, w: Writer::new(0, widths, src.len() / 3 + 16), open: Vec::with_capacity(64), made: Vec::with_capacity(64), last_end: 0, taken: 0, memo_at: usize::MAX, known: 0, holds: 0, flags: 0, depth: 0 };
        p.i = p.next_tok(0);
        p
    }

    /// Whether mark `order` holds at the token (`h`), found once a token, as the lean parser
    /// keeps its `asi`.
    #[inline(always)]
    fn held(&mut self, order: u32, h: fn(&Self) -> bool) -> bool {
        if self.memo_at != self.i {
            self.memo_at = self.i;
            self.known = 0;
        }
        if self.known >> order & 1 == 0 {
            self.known |= 1 << order;
            let v = h(self);
            self.holds = self.holds & !(1 << order) | u64::from(v) << order;
        }
        self.holds >> order & 1 == 1
    }

    #[inline(always)]
    fn next_tok(&self, mut j: usize) -> usize {
        while j < self.n && is_extra(self.codes[j]) {
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
            if !is_extra(self.codes[k]) {
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
        if j < self.n { self.codes[j] >> 8 & 0xff } else { 0 }
    }

    #[inline(always)]
    fn bit_at(&self, j: usize, b: u32) -> bool {
        j < self.n && self.codes[j] >> (16 + b) & 1 == 1
    }

    #[inline(always)]
    fn start(&self) -> u32 {
        if self.i < self.n { self.starts[self.i] } else { self.src.len() as u32 }
    }

    #[inline(always)]
    fn at_end(&self) -> bool {
        self.i >= self.n
    }

    #[cold]
    fn refuse<X>(&self) -> R<X> {
        Err(self.start())
    }

    fn deeper(&mut self) -> R {
        self.depth += 1;
        if self.depth > DEPTH {
            return self.refuse();
        }
        Ok(())
    }

    /// The comments before the token, written into the node open.
    #[inline(always)]
    fn flush(&mut self) {
        while self.pend < self.i {
            let j = self.pend;
            self.pend += 1;
            let c = self.codes[j];
            if is_extra(c) {
                let at = self.w.push(u32::from(extra_kind(c)) | 1 << 16, self.starts[j]);
                self.w.close(at, self.lasts[j] + 1);
            }
        }
    }

    /// The comments pending before the line break of settling mark `bit` (an extra's code holds
    /// the bits of its own gap): written now, into the node open, the node's end past them.
    fn settle(&mut self, bit: u32) {
        while self.pend < self.i {
            let j = self.pend;
            let c = self.codes[j];
            if is_extra(c) && c >> (16 + bit) & 1 == 1 {
                break;
            }
            self.pend += 1;
            if is_extra(c) {
                let at = self.w.push(u32::from(extra_kind(c)) | 1 << 16, self.starts[j]);
                self.w.close(at, self.lasts[j] + 1);
                self.last_end = self.last_end.max(self.lasts[j] + 1);
            }
        }
    }

    /// The token taken: its span; the next token's marks not yet taken.
    #[inline(always)]
    fn bump(&mut self) -> R<(u32, u32)> {
        if self.i >= self.n || self.open_mark() {
            return self.refuse();
        }
        let (s, e) = (self.starts[self.i], self.lasts[self.i] + 1);
        self.i = self.next_tok(self.i + 1);
        self.taken = 0;
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

    /// The root closed at the end of the file: from its first token or comment, or the file's start (`whole`).
    fn close_root(&mut self, whole: bool) -> R {
        if self.i < self.n {
            return self.refuse();
        }
        self.flush();
        // From its first token or comment (none: the file's end), to the file's end (`whole`) or its last token's.
        let first = if self.w.len() > 1 { self.w.start(1) } else { self.src.len() as u32 };
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
