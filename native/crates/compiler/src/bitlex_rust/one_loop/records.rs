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

use super::super::moves::{self, Block, Cond, Op, line};
use super::super::{First, G, Input, upper};
use ::bitlex::syntax::{Gram, Stmt};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write;

struct Rl<'g, 'a> {
    g: &'g G<'a>,
    /// The loop's places' code, by number.
    places: RefCell<Vec<Block>>,
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
    /// The rules whose code may run while a level refuses a mark (`refuse M in L`): those levels, and the rules that
    /// are no level their code reaches. The others' code runs with none refused, and takes its tokens without the
    /// test (`moves::no_refusing`).
    hot: HashSet<String>,
    /// The levels whose call saves its caller's refused marks: a level that refuses one, or one a hot rule calls.
    saves: HashSet<String>,
    /// The levels whose own code may change the level's word (`remember`), which their call saves with the place it
    /// returns to: another leaves its caller's word as it found it.
    words: HashSet<String>,
}

/// The levels whose own code (theirs and the rules that are no level it reaches) takes a `remember` choice: what may
/// set the level's word (`Rl::words`). A reading only reads it (`REMEMBERED`), as the machine's `REMEMBER` alone sets it.
fn word_setters(g: &G<'_>) -> HashSet<String> {
    let mut out = HashSet::new();
    for &level in g.rules.keys().filter(|n| g.nested(n)) {
        let mut seen: HashSet<&str> = HashSet::from([level]);
        let mut todo = vec![level];
        let mut sets = false;
        while let Some(r) = todo.pop() {
            let body = match g.rules.get(r) {
                Some(Stmt::Node { body, .. } | Stmt::Choice { body, .. }) => body,
                _ => continue,
            };
            let mut ns = Vec::new();
            names(body, &mut ns);
            for n in ns {
                if g.remember.iter().any(|r| r == n) {
                    sets = true;
                } else if g.mark_of(n).is_none() && g.rules.contains_key(n) && !g.nested(n) && seen.insert(n) {
                    todo.push(n);
                }
            }
        }
        if sets {
            out.insert(level.to_owned());
        }
    }
    out
}

/// The rules `g` names (`Gram::Sym`), itself and inside.
fn names<'g>(g: &'g Gram, out: &mut Vec<&'g str>) {
    match g {
        Gram::Sym(n) | Gram::Leaf { rule: n, .. } => out.push(n),
        Gram::Seq(gs) | Gram::Alt(gs) => gs.iter().for_each(|g| names(g, out)),
        Gram::Opt(a) | Gram::Star(a) | Gram::Plus(a) | Gram::Field(_, a) | Gram::Retag(_, a) | Gram::Merge(_, a) | Gram::As(_, a) => names(a, out),
        Gram::Wrap { body, .. } | Gram::Reread { body, .. } | Gram::Flag { body, .. } | Gram::Unless { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } => names(body, out),
        _ => {}
    }
}

/// What may run while a level refuses a mark (`Rl::hot`), and the levels whose calls save the marks refused
/// (`Rl::saves`).
fn refusers(g: &G<'_>) -> (HashSet<String>, HashSet<String>) {
    let mut hot: HashSet<String> = g.refusing.iter().filter(|(_, b)| **b != 0).map(|(n, _)| n.clone()).collect();
    let mut saves: HashSet<String> = hot.iter().filter(|n| g.nested(n)).cloned().collect();
    let mut todo: Vec<String> = hot.iter().cloned().collect();
    while let Some(r) = todo.pop() {
        let body = match g.rules.get(r.as_str()) {
            Some(Stmt::Node { body, .. } | Stmt::Choice { body, .. }) => body,
            _ => continue,
        };
        let mut ns = Vec::new();
        names(body, &mut ns);
        for n in ns {
            if g.mark_of(n).is_some() || g.remember.iter().any(|r| r == n) || !g.rules.contains_key(n) {
                continue;
            }
            if g.nested(n) {
                saves.insert(n.to_owned());
            } else if hot.insert(n.to_owned()) {
                todo.push(n.to_owned());
            }
        }
    }
    (hot, saves)
}

/// To place `l`.
fn go(l: usize) -> Block {
    vec![Op::Goto(l)]
}

/// `a`, then `b`.
fn then(mut a: Block, b: Block) -> Block {
    a.extend(b);
    a
}

impl Rl<'_, '_> {
    fn err<X>(&self, what: impl std::fmt::Display) -> Result<X, String> {
        Err(format!("tier 2 as one loop: {what}"))
    }

    /// A new place, its code `code`.
    fn place(&self, code: Block) -> usize {
        let mut p = self.places.borrow_mut();
        p.push(code);
        p.len() - 1
    }

    /// Code `k` as a place: code that only goes on to a place is that place, and the same code is one place.
    fn to_place(&self, k: Block) -> usize {
        if let [Op::Goto(l)] = k.as_slice() {
            return *l;
        }
        let key = moves::print(&k, "");
        if let Some(&l) = self.placed.borrow().get(&key) {
            return l;
        }
        let l = self.place(k);
        self.placed.borrow_mut().insert(key, l);
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
            Gram::Leaf { rule, .. } => self.holds_call(&Gram::Sym(rule.clone()))?,
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
    fn arms(&self, g: &Gram, rest: &[&Gram], rule: &str, k: &Block) -> Result<Vec<(Vec<First>, Block, bool)>, String> {
        let mut alts = Vec::new();
        G::alts(g, &mut alts);
        let mut out = Vec::new();
        for a in alts {
            let (f, e) = self.g.first(a)?;
            let body = match a {
                Gram::Unless { body, .. } => self.walk(body, rest, rule, k.clone())?,
                _ => self.walk(a, rest, rule, k.clone())?,
            };
            out.push((f, body, e));
        }
        Ok(out)
    }

    /// The code of `g` before `rest` in `rule`, then `k`: what holds no level's call is the rules' code (`G::code`);
    /// a level's call ends its place, and the code after it is a place of its own.
    fn walk(&self, g: &Gram, rest: &[&Gram], rule: &str, k: Block) -> Result<Block, String> {
        if !self.holds_call(g)? {
            let mut code = self.g.code(g, rest, rule)?;
            if !self.hot.contains(rule) {
                moves::no_refusing(&mut code);
            }
            return Ok(then(code, k));
        }
        Ok(match g {
            Gram::Sym(n) if self.g.nested(n) => {
                // The caller's state the level changes, then the place it returns to; the level's own.
                let ret = self.to_place(k);
                let mut out = Block::new();
                if self.asks {
                    let (set, keep, _) = self.g.call_asks(rest, rule)?;
                    out.push(line(format!("self.stack.push(self.asks);\nself.asks = self.asks & {keep:#x} | {set:#x};")));
                }
                if self.refusing && self.saves.contains(n.as_str()) {
                    out.push(line(format!("self.stack.push(u64::from(self.refusing));\nself.refusing = {:#x};", self.g.refusing.get(n.as_str()).copied().unwrap_or(0))));
                }
                // The word, where the level may change it, in one entry with the place: one push a call.
                if self.word && self.words.contains(n.as_str()) {
                    out.push(line(format!("self.stack.push({ret} | u64::from(self.word) << 32);")));
                } else {
                    out.push(line(format!("self.stack.push({ret});")));
                }
                then(out, go(self.level(n)?))
            }
            Gram::Sym(n) => {
                // A rule that is no level but calls one: its places, written once, as its method would be; the call
                // saves only the place it returns to.
                let ret = self.to_place(k);
                then(vec![line(format!("self.stack.push({ret});"))], go(self.level(n)?))
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
                let head = self.place(Block::new());
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
                    None => vec![Op::Refuse],
                };
                self.decision(arms, rest, rule, &none, &none, empty.is_some())?
            }
            Gram::Field(f, a) => self.walk(a, rest, rule, then(vec![line(format!("self.field_made({});", self.g.field_const(f)?))], k))?,
            Gram::Retag(kind, a) => then(vec![line(format!("self.retag({});", self.g.kind_const(kind)?))], self.walk(a, rest, rule, k)?),
            Gram::Wrap { kind, field, moved, body } => {
                let f = match field {
                    Some(f) if f != "_" => self.g.field_const(f)?,
                    _ => "0".to_owned(),
                };
                let mut out = vec![line(format!("self.wrap({}, {f})?;", self.g.kind_const(kind)?))];
                if let Some(m) = moved {
                    out.push(line(format!("self.retype_moved({});", self.g.moved_kind(m)?)));
                }
                then(out, self.walk(body, rest, rule, then(vec![line("self.close()?;")], k))?)
            }
            Gram::Relabel { field, moved, inner, body } => {
                let mut out = Block::new();
                if let Some(gf) = inner.as_deref().filter(|f| *f != "_") {
                    out.push(line(format!("self.relabel_inner({})?;", self.g.field_const(gf)?)));
                }
                if let Some(f) = field.as_deref().filter(|f| *f != "_") {
                    out.push(line(format!("self.relabel_field({})?;", self.g.field_const(f)?)));
                }
                match moved.as_deref() {
                    Some(::bitlex::syntax::GONE) => out.push(line("self.drop_made()?;")),
                    Some(m) => out.push(line(format!("self.relabel_kind({})?;", self.g.moved_kind(m)?))),
                    None => {}
                }
                then(out, self.walk(body, rest, rule, k)?)
            }
            Gram::Flag { flag, set, body } => {
                // The flag as it was, saved on the stack across the levels the body calls.
                let f = format!("FLAG_{}", upper(flag));
                let op = if *set { format!("self.flags | {f}") } else { format!("self.flags & !{f}") };
                then(vec![line(format!("self.stack.push(u64::from(self.flags));\nself.flags = {op};"))], self.walk(body, rest, rule, then(vec![line("self.flags = self.stack.pop().unwrap_or(0) as u32;")], k))?)
            }
            Gram::Unless { flag, body } => {
                let after = go(self.to_place(k));
                then(vec![Op::If { arms: vec![(Cond::text(format!("self.flags & FLAG_{} == 0", upper(flag))), self.walk(body, rest, rule, after.clone())?)], els: None }], after)
            }
            Gram::Reread { reading, body } => {
                if !self.g.readings.contains_key(reading.as_str()) {
                    return self.err(format!("rule {rule}: no reading `{reading}`"));
                }
                then(vec![line(format!("self.reread_{}()?;", super::super::ident(reading)))], self.walk(body, rest, rule, k)?)
            }
            // `leaf K R`: `R` reads the tokens, then one leaf of `K` over them (`leaf_open` holds where it starts).
            Gram::Leaf { kind, rule: r } => then(vec![line("self.leaf_open();")], self.walk(&Gram::Sym(r.clone()), rest, rule, then(vec![line(format!("self.leaf_close({})?;", self.g.kind_const(kind)?))], k))?),
            other => return self.err(format!("rule {rule}: a level's call inside {other:?} is not written yet")),
        })
    }

    /// A decision among `arms` before `rest` in `rule` (`G::decide`, its questions first, `G::asking`): `exit` where a
    /// mark after it leaves it, `none` where no arm passes. `ending`: the level may end at the marks after it.
    fn decision(&self, arms: Vec<(Vec<First>, Block, bool)>, rest: &[&Gram], rule: &str, exit: &Block, none: &Block, ending: bool) -> Result<Block, String> {
        let ex = if ending { self.g.exits_ending(rest, rule)? } else { Vec::new() };
        let unmarked = if ending { self.g.after_mark(rest, rule)? } else { Vec::new() };
        let arms: Vec<(Vec<First>, Block)> = arms.into_iter().map(|(f, b, _)| (f, b)).collect();
        Ok(then(self.g.asking(&arms, &ex), self.g.decide(&arms, &ex, &unmarked, exit, none)))
    }

    /// A node of rule `n`'s kind opened at the token: one of no tape kind (always retagged inside) a placeholder until
    /// its retag, as the rule's method opens it.
    fn open(&self, n: &str) -> Result<Op, String> {
        let id = (self.g.x.kind)(n).ok_or_else(|| format!("no kind `{n}`"))?;
        Ok(if id > 0x3ff { line("self.open(UNSET);") } else { line(format!("self.open({});", self.g.kind_const(n)?)) })
    }

    /// Rule `n`'s first place, a level's or a rule's that calls one: its depth (a level's), its node, its body, then its
    /// end, which takes back what its call saved and goes to the place the call returns to.
    fn level(&self, n: &str) -> Result<usize, String> {
        let nested = self.g.nested(n);
        if let Some(&l) = self.levels.borrow().get(n) {
            return Ok(l);
        }
        let first = self.place(Block::new());
        self.levels.borrow_mut().insert(n.to_owned(), first);
        let (body, node) = match self.g.rules[n] {
            Stmt::Node { body, .. } => (body, true),
            Stmt::Choice { body, .. } => (body, false),
            _ => unreachable!(),
        };
        let counts = nested && self.g.counts_depth(n);
        let mut code = if counts { vec![line("self.deeper()?;")] } else { Block::new() };
        // Its endings first (`ending`), as the rule's method has them before its close.
        let mut end = self.g.ending_code(n)?;
        if node {
            code.push(self.open(n)?);
            end.push(line("self.close()?;"));
        }
        if counts {
            end.push(line("self.depth -= 1;"));
        }
        end.push(line("self.marked = false;"));
        // The place the call returns to, then what a level's saved of its caller's: one pop where it saved only the place,
        // as the lean parser's `return`.
        let restores = nested && ((self.refusing && self.saves.contains(n)) || self.asks);
        if nested && self.word && self.words.contains(n) {
            end.push(line("let top = self.stack.pop().unwrap_or(0);\nlet ret = top as u32 as usize;\nself.word = (top >> 32) as u16;"));
        } else if restores {
            end.push(line("let ret = self.stack.pop().unwrap_or(0) as usize;"));
        }
        // A level's call saved its caller's state too, a rule's that is no level only the place.
        if nested && self.refusing && self.saves.contains(n) {
            end.push(line("self.refusing = self.stack.pop().unwrap_or(0) as u32;"));
        }
        if nested && self.asks {
            end.push(line("self.asks = self.stack.pop().unwrap_or(0);"));
        }
        if nested && self.word && self.words.contains(n) || restores {
            end.push(line("st = ret;\ncontinue 'run;"));
        } else {
            end.push(line("st = self.stack.pop().unwrap_or(0) as usize;\ncontinue 'run;"));
        }
        code.extend(self.walk(body, &[], n, end)?);
        self.places.borrow_mut()[first] = code;
        Ok(first)
    }
}

/// Tier 2 of the grammar as one loop over tier 1's records (the module's doc), or why it is not written.
pub(super) fn tier2(x: &Input<'_>) -> Result<(String, u32), String> {
    let g = G::new(x);
    let (hot, saves) = refusers(&g);
    let words = word_setters(&g);
    let all = (0..x.marks.len()).filter(|&m| g.asks(m)).count();
    let rl = Rl { g: &g, places: RefCell::new(Vec::new()), placed: RefCell::new(BTreeMap::new()), levels: RefCell::new(BTreeMap::new()), calls: RefCell::new(HashMap::new()), busy: RefCell::new(HashSet::new()), asks: all > 0, word: !g.remember.is_empty(), refusing: !g.refusing.is_empty(), hot, saves, words };
    // The root: its node, its body, then the file's end, at place 0.
    let root = *g.order.first().ok_or("tier 2 as one loop: no rule")?;
    let (body, whole, padded) = match g.rules[root] {
        Stmt::Node { body, whole, padded, .. } => (body, *whole, *padded),
        _ => return rl.err("a root that is no node"),
    };
    let first = rl.place(Block::new());
    // Where the root recovers (`recover`, decision 0650), the loop is one statement's, from the place 0 to its end, and
    // `run` the statement loop, as the calls form's: a statement that fails returns to it, its stack cut back.
    let one = g.statements(root, body)?;
    let code = match one {
        Some(one) => rl.walk(one, &[], root, vec![line("return Ok(());")])?,
        None => then(vec![line(format!("self.open_root({});", g.kind_const(root)?))], rl.walk(body, &[], root, vec![line(format!("return self.close_root({whole}, {padded});"))])?),
    };
    rl.places.borrow_mut()[first] = code;
    // The rules that are no level and call none: methods, called from the places.
    let mut made = Vec::new();
    for name in &g.order {
        if *name == root || g.nested(name) || rl.rule_calls(name)? {
            continue;
        }
        made.push(g.rule(name, !rl.hot.contains(*name))?);
    }
    // Printed once every rule's and place's moves are made (`G::rule`).
    let mut rules: String = made.into_iter().map(|b| g.printed(b, "")).collect();
    // A place that only goes to another is that other wherever it is named (decision 0673 step 11): no arm of its own,
    // as the lean parser has no statement that only goes on to the next.
    let places: Vec<String> = rl.places.borrow().iter().map(|b| g.printed(b.clone(), "")).collect();
    let jump = |code: &str| code.strip_prefix("st = ").and_then(|r| r.strip_suffix(";\ncontinue 'run;\n")).and_then(|n| n.parse::<usize>().ok());
    let mut to: Vec<usize> = (0..places.len()).collect();
    for (l, code) in places.iter().enumerate() {
        if l > 0
            && let Some(m) = jump(code)
        {
            to[l] = m;
        }
    }
    let target = |mut l: usize| {
        for _ in 0..to.len() {
            if to[l] == l {
                break;
            }
            l = to[l];
        }
        l
    };
    let mut arms = String::new();
    for (l, code) in places.iter().enumerate() {
        if target(l) != l {
            continue;
        }
        let mut code = code.clone();
        for n in (0..places.len()).filter(|&n| target(n) != n) {
            code = code.replace(&format!("st = {n};\n"), &format!("st = {};\n", target(n))).replace(&format!("push({n});"), &format!("push({});", target(n))).replace(&format!("push({n} | "), &format!("push({} | ", target(n)));
        }
        let _ = write!(arms, "                {l} => {{\n{}                }}\n", super::super::indent(&code, "                    "));
    }
    // The stack a local of the loop's, so its length and room stay in registers across the parser's calls.
    let arms = arms.replace("self.stack.", "stack.");
    if one.is_some() {
        let _ = write!(
            rules,
            "    /// The tree: its statements, each one run of the loop (`statement`), which the stack is the loop's own across.\n    fn run(&mut self) -> R {{\n        let mut stack = std::mem::take(&mut self.stack);\n{}    }}\n\n    /// A statement, as one loop: a place a level's part, a level's call and end through the stack (`stack`).\n    #[allow(unreachable_code, unused_labels)]\n    fn statement(&mut self, stack: &mut Vec<u64>) -> R {{\n        let mut st: usize = 0;\n        'run: loop {{\n            match st {{\n{arms}                _ => unreachable!(),\n            }}\n        }}\n    }}\n\n",
            g.statement_loop(root, "self.statement(&mut stack)", "                stack.clear();\n")?
        );
        let out = g.assemble(&rules, "run", "// Generated by native/crates/compiler/src/bitlex_rust/one_loop/records.rs; do not edit. The Bitlex compiler's Rust from the grammar's tree rules, as one loop over tier 1's records.\n\n", true)?;
        return Ok((out, g.reads.get()));
    }
    let _ = write!(
        rules,
        "    /// The tree, as one loop: a place a level's part, a level's call and end through the stack (`stack`).\n    #[allow(unreachable_code, unused_labels)]\n    fn run(&mut self) -> R {{\n        let mut st: usize = 0;\n        // The stack the loop's own while it runs: its length and room in registers, not read again after each call.\n        let mut stack = std::mem::take(&mut self.stack);\n        'run: loop {{\n            match st {{\n{arms}                _ => unreachable!(),\n            }}\n        }}\n    }}\n\n"
    );
    let out = g.assemble(&rules, "run", "// Generated by native/crates/compiler/src/bitlex_rust/one_loop/records.rs; do not edit. The Bitlex compiler's Rust from the grammar's tree rules, as one loop over tier 1's records.\n\n", true)?;
    // The bits the code reads, once all of it is written.
    Ok((out, g.reads.get()))
}

/// Tier 2 of the grammar as calls (the CPU's form, beside the loop, the device's): the same levels, a level's call a
/// call of its method, which saves in its locals what the loop's call saves on the stack, where the same level may
/// change it (`refusers`); a level that may set the level's word keeps its caller's itself (`word_setters`, `G::rule`).
/// The code after it the code that follows. Its depth bounded as the loop's
/// (`deeper`, `DEPTH`).
pub(super) fn tier2_calls(x: &Input<'_>) -> Result<(String, u32), String> {
    let g = G::new(x);
    let (hot, saves) = refusers(&g);
    *g.call_saves.borrow_mut() = Some((saves, word_setters(&g)));
    let made = g.order.iter().map(|name| g.rule(name, !hot.contains(*name))).collect::<Result<Vec<_>, _>>()?;
    let rules: String = made.into_iter().map(|b| g.printed(b, "")).collect();
    let root = *g.order.first().ok_or("tier 2 as calls: no rule")?;
    let out = g.assemble(&rules, &super::super::rule_fn(root), "// Generated by native/crates/compiler/src/bitlex_rust/one_loop/records.rs; do not edit. The Bitlex compiler's Rust from the grammar's tree rules, as calls over tier 1's records.\n\n", false)?;
    Ok((out, g.reads.get()))
}
