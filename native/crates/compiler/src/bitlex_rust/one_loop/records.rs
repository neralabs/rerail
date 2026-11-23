//! Tier 2 as one loop over tier 1's records: each token's first and last byte and its code (its kind, its word and its
//! marks), and the masks a region's parts are found by. The form for a grammar whose tokens tier 1 decides (marks,
//! extras, flags, readings: the `.rerail` grammar), beside the byte form (`one_loop`'s, JSON's), which reads the
//! tokens from the bytes at tier 1's index.
//!
//! - **One loop, one dispatch.** A level (a `nested` rule, and the root) is a place of `run`'s loop (`'run: loop {
//!   match st { … } }`), as the byte form's. Its call saves its caller's state the level changes (the questions it
//!   asks, its word, the marks it refuses) and the place it returns to on `P`'s stack, and goes to the level's first
//!   place; its end closes its node, takes them back and goes to that place. No call recurses: a level's depth is
//!   the stack's, bounded as the JIT's calls are (`DEPTH`).
//! - **The code after a level's call is a place.** A part of a rule that holds one is written as places, each
//!   decision's arms going on to the place after it. A rule that is no level and holds one is places too, written
//!   once: its call saves only the place it returns to. A flag a rule sets around a call is saved on the stack, as
//!   a level's caller's state.
//! - **What calls no level is the rules' code** (`G::code`): its decisions at the record's code, the marks and their
//!   questions, the comments, the flags, the tree's rewrites (`wrap`, `relabel`, `drop`, `merge`, the readings), the
//!   regions read in parts. A rule that is no level and calls none is a method of its own, called from each place.

use super::super::{First, G, Input, upper};
use ::bitlex::syntax::{Gram, Stmt};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write;

struct Rl<'g, 'a> {
    g: &'g G<'a>,
    /// The loop's places' code, by number.
    places: RefCell<Vec<String>>,
    /// Each place made of code (`to_place`), by its code.
    placed: RefCell<BTreeMap<String, usize>>,
    /// Each level's first place.
    levels: RefCell<BTreeMap<String, usize>>,
    /// Whether each rule holds a level's call (`calls`), and the rules being looked into.
    calls: RefCell<HashMap<String, bool>>,
    busy: RefCell<HashSet<String>>,
    /// What a level's call saves of its caller's: the questions it asks (some mark has them), the level's word
    /// (`remember`), the marks it refuses (`refuse … in …`).
    asks: bool,
    word: bool,
    refusing: bool,
}

/// To place `l`.
fn go(l: usize) -> String {
    format!("st = {l};\ncontinue 'run;\n")
}

impl Rl<'_, '_> {
    fn err<X>(&self, what: impl std::fmt::Display) -> Result<X, String> {
        Err(format!("tier 2 as one loop: {what}"))
    }

    /// A new place, its code `code`.
    fn place(&self, code: String) -> usize {
        let mut p = self.places.borrow_mut();
        p.push(code);
        p.len() - 1
    }

    /// Code `k` as a place: code that only goes on to a place is that place, and the same code is one place.
    fn to_place(&self, k: String) -> usize {
        if let Some(l) = k.strip_prefix("st = ").and_then(|r| r.strip_suffix(";\ncontinue 'run;\n")).and_then(|l| l.parse().ok()) {
            return l;
        }
        if let Some(&l) = self.placed.borrow().get(&k) {
            return l;
        }
        let l = self.place(k.clone());
        self.placed.borrow_mut().insert(k, l);
        l
    }

    /// Whether `g` holds a level's call, itself or through the rules written where they are used.
    fn holds_call(&self, g: &Gram) -> Result<bool, String> {
        Ok(match g {
            Gram::Sym(n) => {
                if self.g.mark_of(n).is_some() || self.g.remember.contains(n) || !self.g.rules.contains_key(n.as_str()) {
                    false
                } else if self.g.nested(n) {
                    true
                } else {
                    self.rule_calls(n)?
                }
            }
            Gram::Seq(gs) | Gram::Alt(gs) => {
                let mut any = false;
                for g in gs {
                    any |= self.holds_call(g)?;
                }
                any
            }
            Gram::Opt(a) | Gram::Star(a) | Gram::Plus(a) | Gram::Field(_, a) | Gram::Retag(_, a) | Gram::Merge(_, a) | Gram::As(_, a) => self.holds_call(a)?,
            Gram::Wrap { body, .. } | Gram::Reread { body, .. } | Gram::Flag { body, .. } | Gram::Unless { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } => self.holds_call(body)?,
            _ => false,
        })
    }

    /// Whether rule `n`, no level, holds a level's call. One that reaches itself through no level is refused: it would
    /// be a recursion of the loop's code.
    fn rule_calls(&self, n: &str) -> Result<bool, String> {
        if let Some(&c) = self.calls.borrow().get(n) {
            return Ok(c);
        }
        if !self.busy.borrow_mut().insert(n.to_owned()) {
            return self.err(format!("`{n}` reaches itself through no level"));
        }
        let body = match self.g.rules[n] {
            Stmt::Node { body, .. } | Stmt::Choice { body, .. } => body,
            _ => unreachable!(),
        };
        let c = self.holds_call(body)?;
        self.busy.borrow_mut().remove(n);
        self.calls.borrow_mut().insert(n.to_owned(), c);
        Ok(c)
    }

    /// A decision's arms among `g`'s alternatives, each before `rest` and going on to `k`: its tests, its code (an
    /// `unless F` alternative's guard is its tests'), and whether it may match nothing (`G::arms`).
    fn arms(&self, g: &Gram, rest: &[&Gram], rule: &str, k: &str) -> Result<Vec<(Vec<First>, String, bool)>, String> {
        let mut alts = Vec::new();
        G::alts(g, &mut alts);
        let mut out = Vec::new();
        for a in alts {
            let (f, e) = self.g.first(a)?;
            let body = match a {
                Gram::Unless { body, .. } => self.walk(body, rest, rule, k.to_owned())?,
                _ => self.walk(a, rest, rule, k.to_owned())?,
            };
            out.push((f, body, e));
        }
        Ok(out)
    }

    /// The code of `g` before `rest` in `rule`, then `k`: what holds no level's call is the rules' code (`G::code`);
    /// a level's call ends its place, and the code after it is a place of its own.
    fn walk(&self, g: &Gram, rest: &[&Gram], rule: &str, k: String) -> Result<String, String> {
        if !self.holds_call(g)? {
            return Ok(self.g.code(g, rest, rule, "")? + &k);
        }
        Ok(match g {
            Gram::Sym(n) if self.g.nested(n) => {
                // The caller's state the level changes, then the place it returns to; the level's own.
                let ret = self.to_place(k);
                let mut out = String::new();
                if self.asks {
                    let (set, keep, _) = self.g.call_asks(rest, rule)?;
                    let _ = write!(out, "self.stack.push(self.asks);\nself.asks = self.asks & {keep:#x} | {set:#x};\n");
                }
                if self.word {
                    out.push_str("self.stack.push(u64::from(self.word));\n");
                }
                if self.refusing {
                    let _ = write!(out, "self.stack.push(u64::from(self.refusing));\nself.refusing = {:#x};\n", self.g.refusing.get(n.as_str()).copied().unwrap_or(0));
                }
                let _ = write!(out, "self.stack.push({ret});\n{}", go(self.level(n)?));
                out
            }
            Gram::Sym(n) => {
                // A rule that is no level but calls one: its places, written once, as its method would be; the call
                // saves only the place it returns to.
                let ret = self.to_place(k);
                format!("self.stack.push({ret});\n{}", go(self.level(n)?))
            }
            Gram::Seq(gs) => {
                let mut k = k;
                for i in (0..gs.len()).rev() {
                    let r: Vec<&Gram> = gs[i + 1..].iter().chain(rest.iter().copied()).collect();
                    k = self.walk(&gs[i], &r, rule, k)?;
                }
                k
            }
            Gram::Opt(a) => {
                let after = go(self.to_place(k));
                let arms = self.arms(a, rest, rule, &after)?;
                self.decision(arms, rest, rule, &after, &after, true)?
            }
            Gram::Star(a) | Gram::Plus(a) => {
                let after = go(self.to_place(k));
                let head = self.place(String::new());
                let r: Vec<&Gram> = std::iter::once(g).chain(rest.iter().copied()).collect();
                let arms = self.arms(a, &r, rule, &go(head))?;
                let code = self.decision(arms, rest, rule, &after, &after, true)?;
                self.places.borrow_mut()[head] = code;
                if let Gram::Plus(_) = g { self.walk(a, &r, rule, go(head))? } else { go(head) }
            }
            Gram::Alt(_) => {
                let after = go(self.to_place(k));
                let arms = self.arms(g, rest, rule, &after)?;
                let empty = arms.iter().position(|a| a.2);
                let none = match empty {
                    Some(i) => arms[i].1.clone(),
                    None => "return self.refuse();\n".to_owned(),
                };
                self.decision(arms, rest, rule, &none, &none, empty.is_some())?
            }
            Gram::Field(f, a) => self.walk(a, rest, rule, format!("self.field_made({});\n{k}", self.g.field_const(f)?))?,
            Gram::Retag(kind, a) => format!("self.retag({});\n{}", self.g.kind_const(kind)?, self.walk(a, rest, rule, k)?),
            Gram::Wrap { kind, field, moved, body } => {
                let f = match field {
                    Some(f) if f != "_" => self.g.field_const(f)?,
                    _ => "0".to_owned(),
                };
                let mut out = format!("self.wrap({}, {f})?;\n", self.g.kind_const(kind)?);
                if let Some(m) = moved {
                    let _ = writeln!(out, "self.retype_moved({});", self.g.moved_kind(m)?);
                }
                out.push_str(&self.walk(body, rest, rule, format!("self.close()?;\n{k}"))?);
                out
            }
            Gram::Relabel { field, moved, inner, body } => {
                let mut out = String::new();
                if let Some(gf) = inner.as_deref().filter(|f| *f != "_") {
                    let _ = writeln!(out, "self.relabel_inner({})?;", self.g.field_const(gf)?);
                }
                if let Some(f) = field.as_deref().filter(|f| *f != "_") {
                    let _ = writeln!(out, "self.relabel_field({})?;", self.g.field_const(f)?);
                }
                match moved.as_deref() {
                    Some(::bitlex::syntax::GONE) => out.push_str("self.drop_made()?;\n"),
                    Some(m) => {
                        let _ = writeln!(out, "self.relabel_kind({})?;", self.g.moved_kind(m)?);
                    }
                    None => {}
                }
                out.push_str(&self.walk(body, rest, rule, k)?);
                out
            }
            Gram::Flag { flag, set, body } => {
                // The flag as it was, saved on the stack across the levels the body calls.
                let f = format!("FLAG_{}", upper(flag));
                let op = if *set { format!("self.flags | {f}") } else { format!("self.flags & !{f}") };
                format!("self.stack.push(u64::from(self.flags));\nself.flags = {op};\n{}", self.walk(body, rest, rule, format!("self.flags = self.stack.pop().unwrap_or(0) as u32;\n{k}"))?)
            }
            Gram::Unless { flag, body } => {
                let after = go(self.to_place(k));
                format!("if self.flags & FLAG_{} == 0 {{\n{}}}\n{after}", upper(flag), self.walk(body, rest, rule, after.clone())?)
            }
            Gram::Reread { reading, body } => {
                if !self.g.readings.contains_key(reading.as_str()) {
                    return self.err(format!("rule {rule}: no reading `{reading}`"));
                }
                format!("self.reread_{}()?;\n{}", super::super::ident(reading), self.walk(body, rest, rule, k)?)
            }
            other => return self.err(format!("rule {rule}: a level's call inside {other:?} is not written yet")),
        })
    }

    /// A decision among `arms` before `rest` in `rule` (`G::decide`, its questions first, `G::asking`): `exit` where a
    /// mark after it leaves it, `none` where no arm passes. `ending`: the level may end at the marks after it.
    fn decision(&self, arms: Vec<(Vec<First>, String, bool)>, rest: &[&Gram], rule: &str, exit: &str, none: &str, ending: bool) -> Result<String, String> {
        let ex = if ending { self.g.exits_ending(rest, rule)? } else { Vec::new() };
        let unmarked = if ending { self.g.after_mark(rest, rule)? } else { Vec::new() };
        let arms: Vec<(Vec<First>, String)> = arms.into_iter().map(|(f, b, _)| (f, b)).collect();
        Ok(self.g.asking(&arms, &ex, "") + &self.g.decide(&arms, &ex, &unmarked, exit, none, ""))
    }

    /// A node of rule `n`'s kind opened at the token: one of no tape kind (always retagged inside) a placeholder until
    /// its retag, as the rule's method opens it.
    fn open(&self, n: &str) -> Result<String, String> {
        let id = (self.g.x.kind)(n).ok_or_else(|| format!("no kind `{n}`"))?;
        Ok(if id > 0x3ff { "self.open(UNSET);\n".to_owned() } else { format!("self.open({});\n", self.g.kind_const(n)?) })
    }

    /// Rule `n`'s first place, a level's or a rule's that calls one: its depth (a level's), its node, its body, then its
    /// end, which takes back what its call saved and goes to the place the call returns to.
    fn level(&self, n: &str) -> Result<usize, String> {
        let nested = self.g.nested(n);
        if let Some(&l) = self.levels.borrow().get(n) {
            return Ok(l);
        }
        let first = self.place(String::new());
        self.levels.borrow_mut().insert(n.to_owned(), first);
        let (body, node) = match self.g.rules[n] {
            Stmt::Node { body, .. } => (body, true),
            Stmt::Choice { body, .. } => (body, false),
            _ => unreachable!(),
        };
        let mut code = String::from(if nested { "self.deeper()?;\n" } else { "" });
        let mut end = String::new();
        if node {
            code.push_str(&self.open(n)?);
            end.push_str("self.close()?;\n");
        }
        if nested {
            end.push_str("self.depth -= 1;\n");
        }
        end.push_str("self.marked = false;\n// The place the call returns to, then what a level's saved of its caller's.\nlet ret = self.stack.pop().unwrap_or(0) as usize;\n");
        // A level's call saved its caller's state too, a rule's that is no level only the place.
        if nested && self.refusing {
            end.push_str("self.refusing = self.stack.pop().unwrap_or(0) as u32;\n");
        }
        if nested && self.word {
            end.push_str("self.word = self.stack.pop().unwrap_or(0) as u16;\n");
        }
        if nested && self.asks {
            end.push_str("self.asks = self.stack.pop().unwrap_or(0);\n");
        }
        end.push_str("st = ret;\ncontinue 'run;\n");
        code.push_str(&self.walk(body, &[], n, end)?);
        self.places.borrow_mut()[first] = code;
        Ok(first)
    }
}

/// Tier 2 of the grammar as one loop over tier 1's records (the module's doc), or why it is not written.
pub(super) fn tier2(x: &Input<'_>) -> Result<(String, u32), String> {
    let g = G::new(x);
    let all = (0..x.marks.len()).filter(|&m| g.asks(m)).count();
    let rl = Rl { g: &g, places: RefCell::new(Vec::new()), placed: RefCell::new(BTreeMap::new()), levels: RefCell::new(BTreeMap::new()), calls: RefCell::new(HashMap::new()), busy: RefCell::new(HashSet::new()), asks: all > 0, word: !g.remember.is_empty(), refusing: !g.refusing.is_empty() };
    // The root: its node, its body, then the file's end, at place 0.
    let root = *g.order.first().ok_or("tier 2 as one loop: no rule")?;
    let (body, whole, padded) = match g.rules[root] {
        Stmt::Node { body, whole, padded, .. } => (body, *whole, *padded),
        _ => return rl.err("a root that is no node"),
    };
    let first = rl.place(String::new());
    let code = format!("self.open_root({});\n{}", g.kind_const(root)?, rl.walk(body, &[], root, format!("return self.close_root({whole}, {padded});\n"))?);
    rl.places.borrow_mut()[first] = code;
    // The rules that are no level and call none: methods, called from the places.
    let mut rules = String::new();
    for name in &g.order {
        if *name == root || g.nested(name) || rl.rule_calls(name)? {
            continue;
        }
        rules.push_str(&g.rule(name)?);
    }
    let mut arms = String::new();
    for (l, code) in rl.places.borrow().iter().enumerate() {
        let _ = write!(arms, "                {l} => {{\n{}                }}\n", super::super::indent(code, "                    "));
    }
    // The stack a local of the loop's, so its length and room stay in registers across the parser's calls.
    let arms = arms.replace("self.stack.", "stack.");
    let _ = write!(
        rules,
        "    /// The tree, as one loop: a place a level's part, a level's call and end through the stack (`stack`).\n    #[allow(unreachable_code, unused_labels)]\n    fn run(&mut self) -> R {{\n        let mut st: usize = 0;\n        // The stack the loop's own while it runs: its length and room in registers, not read again after each call.\n        let mut stack = std::mem::take(&mut self.stack);\n        'run: loop {{\n            match st {{\n{arms}                _ => unreachable!(),\n            }}\n        }}\n    }}\n\n"
    );
    let out = g.assemble(&rules, "run", "// Generated by native/crates/compiler/src/bitlex_rust/one_loop/records.rs; do not edit. The Bitlex compiler's Rust from the grammar's tree rules, as one loop over tier 1's records.\n\n", true)?;
    // The bits the code reads, once all of it is written.
    Ok((out, g.reads.get()))
}
