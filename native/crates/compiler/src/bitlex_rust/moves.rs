//! A rule's code as the lean parser's moves (decision 0681): the generator builds them, rewrites them, and prints
//! Rust from them once.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

/// A test at the token, as the lean parser asks it: the word at a place, the token's kind, a mark, a flag, and `!`,
/// `&&` and `||` of tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Cond {
    /// A test the moves do not model yet: its text.
    Text(String),
    /// The word of the token `at` places on is one of `ws` (`w::…`): the lean parser's `is_p` and `is_p_at`.
    Word { at: usize, ws: Vec<String> },
    /// The word of the token `at` places on is one of a choice of tokens': its table (`G::anon_table`) gives it a leaf,
    /// as the lean parser's `anon()` asks a punctuation (decision 0685 step 3).
    Class { at: usize, table: String },
    /// The token is of kind `kind` (the grammar's name): its test (`G::kind_test`), the lean parser's `kind() == STR`.
    Kind { kind: String, test: String },
    /// Mark `mark` (its first definition) holds at the token: its peek's tests of the tokens (`G::peek`), or its
    /// function's call.
    Mark { mark: usize, test: Box<Cond> },
    /// Flag `FLAG_…` is clear.
    Flag(String),
    /// A bracket's scan (`G::scan_fn`) from the token `at` finds its close, and the tokens after it pass `tail`: the lean
    /// parser's scan to the `)`. `held`: asked once before the decision, its answer the local of its name (`G::decide`).
    Scan { f: String, at: String, tail: String, held: bool },
    Not(Box<Cond>),
    And(Vec<Cond>),
    Or(Vec<Cond>),
    /// One test of a mark's definitions, in parentheses among the others.
    Group(Box<Cond>),
}

impl Cond {
    pub(crate) fn text(s: impl Into<String>) -> Self {
        Cond::Text(s.into())
    }

    /// The word at the token is `w`.
    pub(crate) fn word(w: impl Into<String>) -> Self {
        Cond::Word { at: 0, ws: vec![w.into()] }
    }

    /// The test as Rust.
    pub(crate) fn print(&self) -> String {
        match self {
            Cond::Text(s) => s.clone(),
            Cond::Word { at, ws } => {
                let s = if *at == 0 { "self.word()".to_owned() } else { format!("self.word_at(self.i + {at})") };
                match ws.as_slice() {
                    [w] => format!("{s} == {w}"),
                    ws => format!("matches!({s}, {})", ws.join(" | ")),
                }
            }
            Cond::Class { at, table } => {
                let s = if *at == 0 { "self.word()".to_owned() } else { format!("self.word_at(self.i + {at})") };
                format!("{table}[{s} as usize] != 0")
            }
            Cond::Kind { test, .. } => test.clone(),
            Cond::Mark { test, .. } => test.print(),
            Cond::Flag(f) => format!("self.flags & {f} == 0"),
            Cond::Scan { f, at, tail, held } => {
                let call = if *held { f.clone() } else { format!("self.{f}({at})") };
                if tail.is_empty() { format!("{call}.is_some()") } else { format!("{call}.is_some_and(|e| {tail})") }
            }
            Cond::Not(c) if matches!(&**c, Cond::Word { ws, .. } if ws.len() == 1) => c.print().replacen(" == ", " != ", 1),
            Cond::Not(c) => {
                let t = c.print();
                if one_group(&t) || (t.starts_with("self.") && !t.contains(' ')) || matches!(&**c, Cond::Word { .. }) { format!("!{t}") } else { format!("!({t})") }
            }
            Cond::And(cs) => cs.iter().map(|c| if matches!(c, Cond::Or(_)) { format!("({})", c.print()) } else { c.print() }).collect::<Vec<_>>().join(" && "),
            Cond::Or(cs) => cs.iter().map(Cond::print).collect::<Vec<_>>().join(" || "),
            Cond::Group(c) => format!("({})", c.print()),
        }
    }

    /// Its tests, each.
    fn each(&mut self, f: &mut dyn FnMut(&mut Cond)) {
        match self {
            Cond::Mark { test: c, .. } | Cond::Not(c) | Cond::Group(c) => c.each(f),
            Cond::And(cs) | Cond::Or(cs) => cs.iter_mut().for_each(|c| c.each(f)),
            _ => {}
        }
        f(self);
    }

    /// How often it asks scan `name` at the token.
    pub(crate) fn scans(&mut self, name: &str) -> usize {
        let mut n = 0;
        self.each(&mut |c| n += usize::from(matches!(c, Cond::Scan { f, at, held: false, .. } if f == name && at == "self.i")));
        n
    }

    /// It with scan `name` at the token asked before it: its answer held (`G::decide`).
    pub(crate) fn hold_scan(&mut self, name: &str) {
        self.each(&mut |c| {
            if let Cond::Scan { f, at, held, .. } = c
                && f == name
                && at == "self.i"
            {
                *held = true;
            }
        });
    }

    /// The test itself: an `||` of one test that test, a group its test.
    pub(crate) fn core(&self) -> &Cond {
        match self {
            Cond::Or(cs) if cs.len() == 1 => cs[0].core(),
            Cond::Group(c) => c.core(),
            c => c,
        }
    }

    /// Its tests that must all hold: an `&&`'s, a mark's peek's, flattened.
    fn conjuncts(&self) -> Vec<&Cond> {
        match self.core() {
            Cond::And(cs) => cs.iter().flat_map(Cond::conjuncts).collect(),
            Cond::Mark { test, .. } if matches!(test.core(), Cond::And(_)) => test.conjuncts(),
            c => vec![c],
        }
    }

    /// The test that always holds (`true`): a take's where the code before it tested its token.
    pub(crate) fn is_true(&self) -> bool {
        matches!(self, Cond::Text(s) if s == "true")
    }

    /// Whether it holds where one of two tests does (an `||` in it, its text's too).
    pub(crate) fn has_or(&self) -> bool {
        match self {
            Cond::Text(s) | Cond::Kind { test: s, .. } | Cond::Scan { tail: s, .. } => s.contains("||"),
            Cond::Or(cs) => cs.len() > 1 || cs.iter().any(Cond::has_or),
            Cond::And(cs) => cs.iter().any(Cond::has_or),
            Cond::Mark { test: c, .. } | Cond::Not(c) | Cond::Group(c) => c.has_or(),
            Cond::Word { .. } | Cond::Class { .. } | Cond::Flag(_) => false,
        }
    }

    /// The test a take of this block's first move asks, where it is one (`G::decide`'s scans).
    pub(crate) fn first_test(b: &mut Block) -> Option<&mut Cond> {
        match b.first_mut()? {
            Op::If { arms, .. } => arms.first_mut().map(|a| &mut a.0),
            Op::Take { take, .. } => take.test_mut(),
            _ => None,
        }
    }

    /// `a`, then `b`: one `||`.
    fn or(a: Cond, b: Cond) -> Cond {
        let mut cs = match a {
            Cond::Or(cs) => cs,
            a => vec![a],
        };
        match b {
            Cond::Or(bs) => cs.extend(bs),
            b => cs.push(b),
        }
        Cond::Or(cs)
    }
}

/// A `match` arm's pattern: its words (none: `_`), and a guard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pat {
    pub words: Vec<String>,
    pub guard: Option<Cond>,
}

impl Pat {
    pub(crate) fn words(ws: Vec<String>) -> Self {
        Pat { words: ws, guard: None }
    }

    /// `_ if c`.
    pub(crate) fn guard(c: Cond) -> Self {
        Pat { words: Vec::new(), guard: Some(c) }
    }

    fn print(&self) -> String {
        let ws = if self.words.is_empty() { "_".to_owned() } else { self.words.join(" | ") };
        match &self.guard {
            Some(g) => format!("{ws} if {}", g.print()),
            None => ws,
        }
    }

    /// The test the pattern is of the word: its words', its guard after them.
    fn cond(&self) -> Cond {
        let ws = (!self.words.is_empty()).then(|| Cond::Word { at: 0, ws: self.words.clone() });
        match (ws, &self.guard) {
            (Some(w), Some(g)) => Cond::And(vec![w, g.clone()]),
            (Some(w), None) => w,
            (None, Some(g)) => g.clone(),
            (None, None) => Cond::text("true"),
        }
    }

    /// One word alone, no guard.
    fn one_word(&self) -> bool {
        self.words.len() == 1 && self.guard.is_none()
    }
}

/// A take of the token (the runtime's `tk`, `word_leaf`, …): its kind of take and its arguments, the first its test
/// or its word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Take {
    /// The token, which must pass the test: a leaf of a kind and a field.
    Leaf { test: Cond, kind: String, field: String },
    /// The token, which must be the word: a leaf of a kind.
    WordLeaf { word: String, kind: String },
    /// The token, which must be the word: no leaf.
    WordSkip { word: String },
    /// The token, which must pass the test: no leaf.
    Skip { test: Cond },
    /// The token, which must pass the test, joined to the leaf made last.
    Merge { test: Cond, kind: String },
    /// The token, which must pass the test, read into a `leaf`.
    Pass { test: Cond },
    /// The token, which must be the word, read into a `leaf`.
    WordPass { word: String },
}

/// A move of a rule's code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    /// Statements the moves do not model yet: their lines.
    Line(String),
    /// The field `f::…` set on the node made last (the lean parser's `field`).
    Field(String),
    /// A call of the rule whose method is `f` (`self.f()?;`).
    Call(String),
    /// A node of kind `k` opened (`self.open(k);`).
    Open(String),
    /// The node open closed (`self.close()?;`).
    Close,
    /// A take of the token. `nf`: in a rule that runs with no mark refused (`records::no_refusing`); `m`: whether a mark
    /// no rule takes may hold at its token (`takes`), once asked.
    Take { take: Take, nf: bool, m: Option<bool> },
    /// An `if` chain: each test and its block, then the `else`.
    If { arms: Vec<(Cond, Block)>, els: Option<Block> },
    /// A `match` on `subject`: each pattern and its block, then `_`'s.
    Match { subject: String, arms: Vec<(Pat, Block)>, default: Block },
    Loop(Block),
    While(Cond, Block),
    Break,
    /// `return self.refuse();`
    Refuse,
    /// The loop form's jump to a place.
    Goto(usize),
    /// A block at its own indentation: a rule's code in its method.
    At(String, Block),
}

pub(crate) type Block = Vec<Op>;

/// Lines `s`, one move.
pub(crate) fn line(s: impl Into<String>) -> Op {
    Op::Line(s.into())
}

/// A take, its tests not yet asked.
pub(crate) fn take(t: Take) -> Op {
    Op::Take { take: t, nf: false, m: None }
}

impl Take {
    /// The lean parser's helper the take is, and its arguments: `p` (a word the grammar implies, no leaf), `p_stored`
    /// (a word, a leaf), `implied` and `implied_if` (a token the grammar implies, untested or tested), `id` (an
    /// identifier given a field), `bump_as` (the token, a leaf of a kind), and `tk_as`, `merge_as`, `pass_as`,
    /// `word_pass_as` for the rest.
    pub(crate) fn call(&self) -> (&'static str, Vec<String>) {
        match self {
            Take::Leaf { test: Cond::Kind { test, .. }, kind, field } if test == "self.kind() == t::IDENTIFIER" && kind == "k::IDENTIFIER" => ("id", vec![field.clone()]),
            Take::Leaf { test, kind, field } if test.is_true() && field == "0" => ("bump_as", vec![kind.clone()]),
            Take::Leaf { test, kind, field } => ("tk_as", vec![test.print(), kind.clone(), field.clone()]),
            Take::WordLeaf { word, kind } => ("p_stored", vec![word.clone(), kind.clone()]),
            Take::WordSkip { word } => ("p", vec![word.clone()]),
            Take::Skip { test } if test.is_true() => ("implied", vec![]),
            Take::Skip { test } => ("implied_if", vec![test.print()]),
            Take::Merge { test, kind } => ("merge_as", vec![test.print(), kind.clone()]),
            Take::Pass { test } => ("pass_as", vec![test.print()]),
            Take::WordPass { word } => ("word_pass_as", vec![word.clone()]),
        }
    }

    /// Its test, or none for a take of a word.
    fn test(&self) -> Option<&Cond> {
        match self {
            Take::Leaf { test, .. } | Take::Skip { test } | Take::Merge { test, .. } | Take::Pass { test } => Some(test),
            Take::WordLeaf { .. } | Take::WordSkip { .. } | Take::WordPass { .. } => None,
        }
    }

    fn test_mut(&mut self) -> Option<&mut Cond> {
        match self {
            Take::Leaf { test, .. } | Take::Skip { test } | Take::Merge { test, .. } | Take::Pass { test } => Some(test),
            Take::WordLeaf { .. } | Take::WordSkip { .. } | Take::WordPass { .. } => None,
        }
    }

    /// The take as Rust: its tests `M` and `F` its helper's const parameters where the takes ask more than one pair
    /// (`Names`), else none.
    fn print(&self, nf: bool, m: Option<bool>, names: &Names) -> String {
        let (name, args) = self.call();
        match m {
            Some(m) if names.pairs.len() > 1 => format!("self.{name}::<{m}, {}>({})?;", !nf, args.join(", ")),
            _ => format!("self.{name}({})?;", args.join(", ")),
        }
    }
}

/// The takes of a parser's rules, once all of them are made: the pairs of tests (`M`, a mark no rule takes may hold
/// at the token; `F`, the level open may refuse a mark there) they ask, and the lean parser's helpers they call
/// (`Take::call`). Where they ask one pair, the helpers have no const parameters and ask only that pair, as the lean
/// parser's one `p` and `id`.
#[derive(Debug, Default)]
pub(crate) struct Names {
    pub pairs: std::collections::BTreeSet<(bool, bool)>,
    pub calls: HashSet<&'static str>,
}

impl Names {
    /// The takes of `bs`.
    pub(crate) fn of(bs: &mut [Block]) -> Names {
        let mut n = Names::default();
        for b in bs {
            takes_in(b, &mut |t, nf, m| {
                n.pairs.insert((m.unwrap_or(true), !*nf));
                n.calls.insert(t.call().0);
            });
        }
        n
    }
}

/// `b` as Rust, each line at `pad`.
pub(crate) fn print(b: &[Op], pad: &str) -> String {
    print_named(b, pad, &Names::default())
}

/// `b` as Rust, each line at `pad`, its takes as `names` has them.
pub(crate) fn print_named(b: &[Op], pad: &str, names: &Names) -> String {
    let mut out = String::new();
    print_into(b, pad, names, &mut out);
    out
}

fn print_into(b: &[Op], pad: &str, names: &Names, out: &mut String) {
    let q = format!("{pad}    ");
    for op in b {
        match op {
            Op::Line(s) => {
                for l in s.lines() {
                    if !l.is_empty() {
                        out.push_str(pad);
                    }
                    out.push_str(l);
                    out.push('\n');
                }
            }
            Op::Field(f) => {
                let _ = writeln!(out, "{pad}self.field_made({f});");
            }
            Op::Call(f) => {
                let _ = writeln!(out, "{pad}self.{f}()?;");
            }
            Op::Open(k) => {
                let _ = writeln!(out, "{pad}self.open({k});");
            }
            Op::Close => {
                let _ = writeln!(out, "{pad}self.close()?;");
            }
            Op::Take { take, nf, m } => {
                let _ = writeln!(out, "{pad}{}", take.print(*nf, *m, names));
            }
            Op::If { arms, els } => {
                for (i, (c, body)) in arms.iter().enumerate() {
                    if i == 0 {
                        let _ = writeln!(out, "{pad}if {} {{", c.print());
                    } else {
                        let _ = writeln!(out, " else if {} {{", c.print());
                    }
                    print_into(body, &q, names, out);
                    out.push_str(pad);
                    out.push('}');
                }
                match els {
                    Some(e) => {
                        out.push_str(" else {\n");
                        print_into(e, &q, names, out);
                        let _ = writeln!(out, "{pad}}}");
                    }
                    None => out.push('\n'),
                }
            }
            Op::Match { subject, arms, default } => {
                let r = format!("{q}    ");
                let _ = writeln!(out, "{pad}match {subject} {{");
                for (pat, body) in arms {
                    let _ = writeln!(out, "{q}{} => {{", pat.print());
                    print_into(body, &r, names, out);
                    let _ = writeln!(out, "{q}}}");
                }
                let _ = writeln!(out, "{q}_ => {{");
                print_into(default, &r, names, out);
                let _ = writeln!(out, "{q}}}\n{pad}}}");
            }
            Op::Loop(body) => {
                let _ = writeln!(out, "{pad}loop {{");
                print_into(body, &q, names, out);
                let _ = writeln!(out, "{pad}}}");
            }
            Op::While(c, body) => {
                let _ = writeln!(out, "{pad}while {} {{", c.print());
                print_into(body, &q, names, out);
                let _ = writeln!(out, "{pad}}}");
            }
            Op::Break => {
                let _ = writeln!(out, "{pad}break;");
            }
            Op::Refuse => {
                let _ = writeln!(out, "{pad}return self.refuse();");
            }
            Op::Goto(l) => {
                let _ = writeln!(out, "{pad}st = {l};\n{pad}continue 'run;");
            }
            Op::At(p, b) => print_into(b, p, names, out),
        }
    }
}

/// Each block in `b`, inner ones first, then `b`.
fn blocks(b: &mut Block, f: &mut dyn FnMut(&mut Block)) {
    for op in b.iter_mut() {
        match op {
            Op::If { arms, els } => {
                arms.iter_mut().for_each(|(_, x)| blocks(x, f));
                if let Some(e) = els {
                    blocks(e, f);
                }
            }
            Op::Match { arms, default, .. } => {
                arms.iter_mut().for_each(|(_, x)| blocks(x, f));
                blocks(default, f);
            }
            Op::Loop(x) | Op::While(_, x) | Op::At(_, x) => blocks(x, f),
            _ => {}
        }
    }
    f(b);
}

/// Each take in `b`.
fn takes_in(b: &mut Block, f: &mut dyn FnMut(&mut Take, &mut bool, &mut Option<bool>)) {
    blocks(b, &mut |x| {
        for op in x.iter_mut() {
            if let Op::Take { take, nf, m } = op {
                f(take, nf, m);
            }
        }
    });
}

/// `b` with each take one of a rule that runs with no mark refused (`records::no_refusing`).
pub(crate) fn no_refusing(b: &mut Block) {
    takes_in(b, &mut |_, nf, _| *nf = true);
}

/// `b` with each test of every word of a choice of tokens (`anon`: each one's table and words) that choice's test,
/// the other words after it: one test where the grammar lists the choice's words (decision 0685 step 3).
pub(crate) fn classes(b: &mut Block, anon: &[(String, HashSet<String>)]) {
    let split = |at: usize, ws: &[String]| -> Option<Cond> {
        // The choice that leaves the fewest words to list: those it tests besides the choice's, and the choice's it does
        // not. A choice of a few words is as plain as its words, and so is one that leaves as many to list.
        let listed = |c: &HashSet<String>| ws.iter().filter(|w| !c.contains(*w)).count() + c.iter().filter(|w| !ws.contains(w)).count();
        let (table, class) = anon.iter().filter(|(_, c)| c.len() >= 4).min_by_key(|(_, c)| listed(c))?;
        if listed(class) + 2 > ws.len() {
            return None;
        }
        let rest: Vec<String> = ws.iter().filter(|w| !class.contains(*w)).cloned().collect();
        let mut but: Vec<String> = class.iter().filter(|w| !ws.contains(w)).cloned().collect();
        but.sort();
        let mut c = Cond::Class { at, table: table.clone() };
        if !but.is_empty() {
            c = Cond::And(vec![c, Cond::Not(Box::new(Cond::Word { at, ws: but }))]);
        }
        Some(if rest.is_empty() { c } else { Cond::Or(vec![c, Cond::Word { at, ws: rest }]) })
    };
    let cond = |c: &mut Cond| {
        c.each(&mut |c| {
            if let Cond::Word { at, ws } = c
                && let Some(n) = split(*at, ws)
            {
                *c = n;
            }
        })
    };
    blocks(b, &mut |x| {
        for op in x.iter_mut() {
            match op {
                // A take under the arm's test takes its token untested, the test now the choice's.
                Op::If { arms, .. } => arms.iter_mut().for_each(|(c, body)| {
                    cond(c);
                    untested(body, c, None);
                }),
                Op::While(c, _) => cond(c),
                Op::Take { take, .. } => {
                    if let Some(c) = take.test_mut() {
                        cond(c);
                    }
                }
                Op::Match { subject, arms, .. } if subject == "self.word()" => {
                    for (p, body) in arms.iter_mut() {
                        if let Some(n) = split(0, &p.words) {
                            p.words.clear();
                            p.guard = Some(match p.guard.take() {
                                Some(g) => Cond::And(vec![n, g]),
                                None => n,
                            });
                        }
                        if let Some(g) = &mut p.guard {
                            cond(g);
                        }
                        if p.words.is_empty()
                            && let Some(g) = &p.guard
                        {
                            untested(body, g, None);
                        }
                    }
                }
                _ => {}
            }
        }
    });
}

/// The loops in `b` (`while` and `loop`), each, outer ones first: each alone, and with the move after it.
pub(crate) fn loops(b: &Block, out: &mut Vec<Block>) {
    for (i, op) in b.iter().enumerate() {
        if matches!(op, Op::Loop(_) | Op::While(..)) {
            out.push(vec![op.clone()]);
            if let Some(next) = b.get(i + 1) {
                out.push(vec![op.clone(), next.clone()]);
            }
        }
        match op {
            Op::If { arms, els } => {
                arms.iter().for_each(|(_, x)| loops(x, out));
                if let Some(e) = els {
                    loops(e, out);
                }
            }
            Op::Match { arms, default, .. } => {
                arms.iter().for_each(|(_, x)| loops(x, out));
                loops(default, out);
            }
            Op::Loop(x) | Op::While(_, x) | Op::At(_, x) => loops(x, out),
            _ => {}
        }
    }
}

/// The runs of moves around each loop in `b` (`while` and `loop`): the loop with up to two moves before it and one
/// after it, for `G::lists`.
pub(crate) fn runs(b: &Block, out: &mut Vec<Block>) {
    for (i, op) in b.iter().enumerate() {
        if matches!(op, Op::Loop(_) | Op::While(..)) {
            for s in i.saturating_sub(2)..=i {
                for e in i + 1..=(i + 2).min(b.len()) {
                    if e - s >= 2 {
                        out.push(b[s..e].to_vec());
                    }
                }
            }
        }
        match op {
            Op::If { arms, els } => {
                arms.iter().for_each(|(_, x)| runs(x, out));
                if let Some(e) = els {
                    runs(e, out);
                }
            }
            Op::Match { arms, default, .. } => {
                arms.iter().for_each(|(_, x)| runs(x, out));
                runs(default, out);
            }
            Op::Loop(x) | Op::While(_, x) | Op::At(_, x) => runs(x, out),
            _ => {}
        }
    }
}

/// A run of moves with the rules it calls and the fields it sets each a slot (`#c0`, `#f0`, … in the order met): the
/// run as the slots leave it, and what each slot was, the calls' then the fields'.
pub(crate) fn abstracted(seq: &[Op]) -> (Block, Vec<String>, Vec<String>) {
    let mut b = seq.to_vec();
    let (mut calls, mut fields): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    let slot = |v: &mut Vec<String>, s: &str, p: &str| -> String {
        let k = v.iter().position(|x| x == s).unwrap_or_else(|| {
            v.push(s.to_owned());
            v.len() - 1
        });
        format!("#{p}{k}")
    };
    blocks(&mut b, &mut |x| {
        for o in x.iter_mut() {
            match o {
                Op::Call(f) => *f = slot(&mut calls, f, "c"),
                Op::Field(f) => *f = slot(&mut fields, f, "f"),
                Op::Take { take: Take::Leaf { field, .. }, .. } if field != "0" => *field = slot(&mut fields, field, "f"),
                _ => {}
            }
        }
    });
    (b, calls, fields)
}

/// `b` (an `abstracted` run) with its slots filled: a call's slot by `call`'s move, a field's by `field`'s name.
pub(crate) fn filled(b: &Block, call: &dyn Fn(usize) -> Op, field: &dyn Fn(usize) -> String) -> Block {
    let mut b = b.clone();
    let k = |s: &str, p: &str| s.strip_prefix(p).and_then(|r| r.parse::<usize>().ok());
    blocks(&mut b, &mut |x| {
        for o in x.iter_mut() {
            match o {
                Op::Call(f) if let Some(i) = k(f, "#c") => *o = call(i),
                Op::Field(f) if let Some(i) = k(f, "#f") => *f = field(i),
                Op::Take { take: Take::Leaf { field: f, .. }, .. } if let Some(i) = k(f, "#f") => *f = field(i),
                _ => {}
            }
        }
    });
    b
}

/// `b` with each run of moves that is `norm` once abstracted (`abstracted`) the move `call` makes of its slots.
pub(crate) fn call_for_run(b: &mut Block, norm: &Block, call: &dyn Fn(&[String], &[String]) -> Op) {
    blocks(b, &mut |x| {
        let mut i = 0;
        while i + norm.len() <= x.len() {
            let (a, calls, fields) = abstracted(&x[i..i + norm.len()]);
            if a == *norm {
                x.splice(i..i + norm.len(), [call(&calls, &fields)]);
            }
            i += 1;
        }
    });
}

/// Whether `b` only joins tokens to a leaf of kind `k` begun before it (`merge_as`): its takes merges into `k`, its
/// calls of `merging`'s, nothing else written (decision 0685 rock 3, `G::leaves`).
pub(crate) fn merges_only(b: &[Op], k: &str, merging: &HashSet<String>) -> bool {
    b.iter().all(|op| match op {
        Op::Take { take: Take::Merge { kind, .. }, .. } => kind == k,
        Op::Call(f) => merging.contains(f),
        Op::Break | Op::Refuse => true,
        Op::If { arms, els } => arms.iter().all(|(_, x)| merges_only(x, k, merging)) && els.as_ref().is_none_or(|e| merges_only(e, k, merging)),
        Op::Match { arms, default, .. } => arms.iter().all(|(_, x)| merges_only(x, k, merging)) && merges_only(default, k, merging),
        Op::Loop(x) | Op::While(_, x) => merges_only(x, k, merging),
        _ => false,
    })
}

/// Whether `op` begins a leaf of kind `k` (a take of the token as one, no field), on each way through it that does not
/// refuse.
fn begins(op: &Op, k: &str, merging: &HashSet<String>) -> bool {
    let arm = |x: &Block| matches!(x.as_slice(), [Op::Refuse]) || (x.first().is_some_and(|f| begins(f, k, merging)) && merges_only(&x[1..], k, merging));
    match op {
        Op::Take { take: Take::Leaf { kind, field, .. }, .. } => kind == k && field == "0",
        Op::Take { take: Take::WordLeaf { kind, .. }, .. } => kind == k,
        Op::If { arms, els: Some(e) } => arms.iter().all(|(_, x)| arm(x)) && arm(e),
        Op::Match { arms, default, .. } => arms.iter().all(|(_, x)| arm(x)) && arm(default),
        _ => false,
    }
}

/// `b` with each take into a leaf of kind `k` (its beginning and its merges) a pass (`pass_as`): read into the leaf
/// `leaf_close` makes. The calls of `merging` it makes, counted in `calls`.
fn passes(b: &mut Block, k: &str, merging: &HashSet<String>, calls: &mut HashMap<String, usize>) {
    blocks(b, &mut |x| {
        for op in x.iter_mut() {
            match op {
                Op::Take { take, .. } => {
                    let new = match &*take {
                        Take::Merge { test, kind } | Take::Leaf { test, kind, .. } if kind == k => Some(Take::Pass { test: test.clone() }),
                        Take::WordLeaf { word, kind } if kind == k => Some(Take::WordPass { word: word.clone() }),
                        _ => None,
                    };
                    if let Some(n) = new {
                        *take = n;
                    }
                }
                Op::Call(f) if merging.contains(f) => *calls.entry(f.clone()).or_default() += 1,
                _ => {}
            }
        }
    });
}

/// `b` with each run that begins a leaf of kind `k` and then only joins tokens to it (`begins`, `merges_only`) one leaf
/// over the tokens it reads: `leaf_open`, the tokens passed, `leaf_close(k)`, as the lean parser's `leaf_over` after
/// its scan. The calls of `merging` in them counted in `calls`; whether it had one.
pub(crate) fn one_leaf(b: &mut Block, k: &str, merging: &HashSet<String>, calls: &mut HashMap<String, usize>) -> bool {
    // The outermost run first: a run's arms that each begin the leaf are its, not leaves of their own.
    fn walk(x: &mut Block, k: &str, merging: &HashSet<String>, calls: &mut HashMap<String, usize>, found: &mut bool) {
        let mut i = 0;
        while i < x.len() {
            if begins(&x[i], k, merging) {
                let mut j = i + 1;
                while j < x.len() && merges_only(&x[j..=j], k, merging) {
                    j += 1;
                }
                let mut run: Block = x[i..j].to_vec();
                passes(&mut run, k, merging, calls);
                let mut new = vec![Op::Line("self.leaf_open();".into())];
                new.extend(run);
                new.push(Op::Line(format!("self.leaf_close({k})?;")));
                let n = new.len();
                x.splice(i..j, new);
                *found = true;
                i += n;
            } else {
                match &mut x[i] {
                    Op::If { arms, els } => {
                        arms.iter_mut().for_each(|(_, y)| walk(y, k, merging, calls, found));
                        if let Some(e) = els {
                            walk(e, k, merging, calls, found);
                        }
                    }
                    Op::Match { arms, default, .. } => {
                        arms.iter_mut().for_each(|(_, y)| walk(y, k, merging, calls, found));
                        walk(default, k, merging, calls, found);
                    }
                    Op::Loop(y) | Op::While(_, y) | Op::At(_, y) => walk(y, k, merging, calls, found),
                    _ => {}
                }
                i += 1;
            }
        }
    }
    let mut found = false;
    walk(b, k, merging, calls, &mut found);
    found
}

/// `b` with each token taken as a leaf of no field and then joined to the next (`merge_as`), or a decision of whose
/// every arm joins it to one of another kind, one leaf over the two: `leaf_open`, both passed, `leaf_close` of the
/// kind the decision gives (`let kind = …`), as the lean parser's `leaf_over(kind, i, i + 2)` of `-1` and `.input`
/// (decision 0685 rock 6).
pub(crate) fn two_token_leaves(b: &mut Block) {
    blocks(b, &mut |x| {
        let mut i = 0;
        while i + 1 < x.len() {
            let first = match &x[i] {
                Op::Take { take: Take::Leaf { test, field, .. }, nf, m } if field == "0" => Op::Take { take: Take::Pass { test: test.clone() }, nf: *nf, m: *m },
                Op::Take { take: Take::WordLeaf { word, .. }, nf, m } => Op::Take { take: Take::WordPass { word: word.clone() }, nf: *nf, m: *m },
                _ => {
                    i += 1;
                    continue;
                }
            };
            // The second token's merge, alone, of one test on every arm: its test, its move, each arm's kind.
            let merge = |body: &Block| match body.as_slice() {
                [Op::Take { take: Take::Merge { test, kind }, nf, m }] => Some((test.clone(), *nf, *m, kind.clone())),
                _ => None,
            };
            let refuses = |body: &Block| matches!(body.as_slice(), [Op::Refuse]);
            // The second token's one test: the merges' where they are alike, else the one test they ask besides
            // `true` where each other arm's own test already holds it (`fact`).
            let one_test = |conds: Vec<&Cond>, ms: &[(Cond, bool, Option<bool>, String)]| -> Option<(Cond, bool, Option<bool>)> {
                let (nf, m) = (ms[0].1, ms[0].2);
                if ms.iter().any(|x| (x.1, x.2) != (nf, m)) {
                    return None;
                }
                let mut tests: Vec<&Cond> = ms.iter().map(|x| &x.0).filter(|t| t.print() != "true").collect();
                tests.dedup_by(|a, b| a.print() == b.print());
                match tests.as_slice() {
                    [] => Some((ms[0].0.clone(), nf, m)),
                    [t] => {
                        let f = fact(t)?;
                        conds.iter().zip(ms).all(|(c, x)| x.0.print() == t.print() || facts_of(c).contains(&f)).then(|| ((*t).clone(), nf, m))
                    }
                    _ => None,
                }
            };
            let (second, table, kind) = match &x[i + 1] {
                Op::Take { take: Take::Merge { test, kind }, nf, m } => (Op::Take { take: Take::Pass { test: test.clone() }, nf: *nf, m: *m }, None, kind.clone()),
                Op::If { arms, els: Some(e) } if refuses(e) && arms.iter().all(|(_, a)| merge(a).is_some()) => {
                    let ms: Vec<_> = arms.iter().filter_map(|(_, a)| merge(a)).collect();
                    let Some((test, nf, m)) = one_test(arms.iter().map(|(c, _)| c).collect(), &ms) else {
                        i += 1;
                        continue;
                    };
                    let mut t = "let kind = ".to_owned();
                    for ((c, _), m) in arms.iter().zip(&ms) {
                        let _ = write!(t, "if {} {{ {} }} else ", c.print(), m.3);
                    }
                    t.push_str("{ return self.refuse(); };");
                    (Op::Take { take: Take::Pass { test }, nf, m }, Some(t), "kind".to_owned())
                }
                Op::Match { subject, arms, default } if refuses(default) && arms.iter().all(|(_, a)| merge(a).is_some()) => {
                    let ms: Vec<_> = arms.iter().filter_map(|(_, a)| merge(a)).collect();
                    if ms.iter().any(|m| (&m.0, m.1, m.2) != (&ms[0].0, ms[0].1, ms[0].2)) {
                        i += 1;
                        continue;
                    }
                    let mut t = format!("let kind = match {subject} {{\n");
                    for ((p, _), m) in arms.iter().zip(&ms) {
                        let _ = writeln!(t, "    {} => {},", p.print(), m.3);
                    }
                    t.push_str("    _ => return self.refuse(),\n};");
                    (Op::Take { take: Take::Pass { test: ms[0].0.clone() }, nf: ms[0].1, m: ms[0].2 }, Some(t), "kind".to_owned())
                }
                _ => {
                    i += 1;
                    continue;
                }
            };
            let mut new = vec![Op::Line("self.leaf_open();".into()), first];
            new.extend(table.map(Op::Line));
            new.push(second);
            new.push(Op::Line(format!("self.leaf_close({kind})?;")));
            let n = new.len();
            x.splice(i..i + 2, new);
            i += n;
        }
    });
}

/// `b`, a function that only joins tokens (`merges_only`), its takes passes (`passes`): the calls of `merging` it makes
/// counted in `calls`.
pub(crate) fn passes_in(b: &mut Block, k: &str, merging: &HashSet<String>, calls: &mut HashMap<String, usize>) {
    passes(b, k, merging, calls);
}

/// The kinds the merges in `b` join tokens into (`merge_as`).
pub(crate) fn merge_kinds(b: &mut Block, out: &mut Vec<String>) {
    blocks(b, &mut |x| {
        for o in x.iter() {
            if let Op::Take { take: Take::Merge { kind, .. }, .. } = o
                && !out.contains(kind)
            {
                out.push(kind.clone());
            }
        }
    });
}

/// `b` with each call of `f` one of `g`.
pub(crate) fn rename_calls(b: &mut Block, f: &str, g: &str) {
    blocks(b, &mut |x| {
        for o in x.iter_mut() {
            match o {
                Op::Call(c) if c == f => *c = g.to_owned(),
                Op::Line(l) if l.contains(&format!("Self::{f})")) => *l = l.replace(&format!("Self::{f})"), &format!("Self::{g})")),
                _ => {}
            }
        }
    });
}

/// How many calls of `f` `b` makes.
pub(crate) fn calls_of(b: &mut Block, f: &str) -> usize {
    let mut n = 0;
    blocks(b, &mut |x| n += x.iter().filter(|o| matches!(o, Op::Call(g) if g == f)).count());
    n
}

/// A test of one token as a fact: where it is (places on from the token) and its text as a test of the token itself
/// (`self.word()`, `self.kind()`); none where it reads more than one token or other state.
fn fact(c: &Cond) -> Option<(usize, String)> {
    let t = c.print();
    let mut at = None;
    let mut out = t.clone();
    for (f, base) in [("self.word_at(", "self.word()"), ("self.kind_at(", "self.kind()")] {
        while let Some(i) = out.find(f) {
            let rest = &out[i + f.len()..];
            let close = rest.find(')')?;
            let arg = &rest[..close];
            let n = if arg == "self.i" { 0 } else { arg.strip_prefix("self.i + ")?.parse::<usize>().ok()? };
            if at.is_some_and(|a| a != n) {
                return None;
            }
            at = Some(n);
            out = format!("{}{base}{}", &out[..i], &rest[close + 1..]);
        }
    }
    let at = at.unwrap_or(0);
    if out.contains("self.i") || out.contains("_at(") || out.contains("||") || !(out.contains("self.word()") || out.contains("self.kind()")) {
        return None;
    }
    if out.contains("self.") && out.replace("self.word()", "").replace("self.kind()", "").contains("self.") {
        return None;
    }
    Some((at, out))
}

/// The facts of a test that holds: each of its conjuncts' (`fact`).
fn facts_of(c: &Cond) -> Vec<(usize, String)> {
    c.conjuncts().into_iter().filter_map(fact).collect()
}

/// Whether a method's code is one leaf over the tokens it reads (`G::leaves`): an idea of its own, as the lean
/// parser's `raw_line` and `json`.
pub(crate) fn one_leaf_only(b: &Block) -> bool {
    matches!(b.first(), Some(Op::Line(l)) if l == "self.leaf_open();") && matches!(b.last(), Some(Op::Line(l)) if l.starts_with("self.leaf_close("))
}

/// `b` with each take whose token the code before it tested (an `if`'s or a `match`'s, a `while`'s, past the moves
/// that take no token: opening a node, setting a field) taking it untested, as the lean parser's `statement` bumps
/// the `.` and the word it matched.
pub(crate) fn known_takes(b: &mut Block) {
    fn walk(b: &mut Block, mut facts: Vec<(usize, String)>) {
        for op in b.iter_mut() {
            match op {
                Op::Take { take, .. } => {
                    let held = |t: String| facts.iter().any(|(a, f)| *a == 0 && *f == t);
                    let yes = Cond::text("true");
                    let new = match &*take {
                        Take::WordLeaf { word, kind } if held(format!("self.word() == {word}")) => Some(Take::Leaf { test: yes, kind: kind.clone(), field: "0".into() }),
                        Take::WordSkip { word } if held(format!("self.word() == {word}")) => Some(Take::Skip { test: yes }),
                        Take::WordPass { word } if held(format!("self.word() == {word}")) => Some(Take::Pass { test: yes }),
                        Take::Leaf { test: Cond::Kind { test, .. }, kind, .. } if test == "self.kind() == t::IDENTIFIER" && kind == "k::IDENTIFIER" => None,
                        t => {
                            let mut t = t.clone();
                            match t.test_mut() {
                                Some(c) if !c.is_true() && held(c.print()) => {
                                    *c = yes;
                                    Some(t)
                                }
                                _ => None,
                            }
                        }
                    };
                    if let Some(n) = new {
                        *take = n;
                    }
                    facts = facts.into_iter().filter(|(a, _)| *a > 0).map(|(a, f)| (a - 1, f)).collect();
                }
                Op::Open(_) | Op::Field(_) => {}
                Op::Line(l) if ["let kind", "self.retag(", "self.marked", "self.deeper()", "self.leaf_open()", "self.open(", "self.field_made(", "self.wrap(", "self.relabel_"].iter().any(|p| l.trim_start().starts_with(p)) => {}
                Op::If { arms, els } => {
                    for (c, x) in arms.iter_mut() {
                        let mut f = facts.clone();
                        f.extend(facts_of(c));
                        walk(x, f);
                    }
                    if let Some(e) = els {
                        walk(e, facts.clone());
                    }
                    facts.clear();
                }
                Op::Match { subject, arms, default } => {
                    let at = if subject == "self.word()" { Some(0) } else { subject.strip_prefix("self.word_at(self.i + ").and_then(|r| r.strip_suffix(')')).and_then(|n| n.parse::<usize>().ok()) };
                    for (pat, x) in arms.iter_mut() {
                        let mut f = facts.clone();
                        if let (Some(a), [w]) = (at, pat.words.as_slice()) {
                            f.push((a, format!("self.word() == {w}")));
                        }
                        if let Some(g) = &pat.guard {
                            f.extend(facts_of(g));
                        }
                        walk(x, f);
                    }
                    walk(default, facts.clone());
                    facts.clear();
                }
                Op::While(c, x) => {
                    walk(x, facts_of(c));
                    facts.clear();
                }
                Op::Loop(x) | Op::At(_, x) => {
                    walk(x, Vec::new());
                    facts.clear();
                }
                _ => facts.clear(),
            }
        }
    }
    walk(b, Vec::new());
}

/// The words a test asks of the token `e` a scan found: `self.word_at(e) == w::A || …`, each of them; none where it asks
/// anything else.
fn scan_words(tail: &str) -> Option<HashSet<&str>> {
    bare(tail).split(" || ").map(|p| p.trim().strip_prefix("self.word_at(e) == ")).collect()
}

/// Whether no token passes both tests: each asks the word at one place, of words none of which the other's has, or
/// one held scan's close the same way.
fn disjoint(a: &Cond, b: &Cond) -> bool {
    a.conjuncts().iter().any(|x| {
        b.conjuncts().iter().any(|y| match (x, y) {
            (Cond::Word { at: i, ws: u }, Cond::Word { at: j, ws: v }) => i == j && !u.iter().any(|w| v.contains(w)),
            // One scan's close (`S.is_some_and(|e| …)`), asked of words none the other's asks.
            _ => {
                let (p, q) = (x.print(), y.print());
                let split = |t: &str| t.split_once(".is_some_and(|e| ").and_then(|(s, r)| Some((s.to_owned(), r.strip_suffix(')')?.to_owned())));
                matches!((split(&p), split(&q)), (Some((s, a)), Some((t, b))) if s == t && !s.contains(' ') && matches!((scan_words(&a), scan_words(&b)), (Some(u), Some(v)) if u.is_disjoint(&v)))
            }
        })
    })
}

/// `a && b`, each that holds an `||` in parentheses.
fn and(a: Cond, b: Cond) -> Cond {
    // An `||` outside any parentheses in its text (an `Or` itself `And` prints in parentheses).
    let bare_or = |t: &str| {
        let mut depth = 0i32;
        let b = t.as_bytes();
        (0..b.len()).any(|i| {
            match b[i] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                _ => {}
            }
            depth == 0 && b[i] == b'|' && b.get(i + 1) == Some(&b'|')
        })
    };
    let g = |c: Cond| if !matches!(c, Cond::Or(_)) && bare_or(&c.print()) { Cond::Group(Box::new(c)) } else { c };
    let mut cs = match a {
        Cond::And(cs) => cs,
        a => vec![g(a)],
    };
    match b {
        Cond::And(bs) => cs.extend(bs),
        b => cs.push(g(b)),
    }
    Cond::And(cs)
}

/// The decisions in `b` once their arms' code is final (after `classes` and `G::leaves`): arms next to each other whose
/// code is the same one arm, their tests `||`'d; a last arm whose code is the `else`'s gone; a `match` of the word of one
/// arm an `if`; a `while` whose code is `if d { x } else { break; }` a `while` of both tests. As the lean parser's
/// `raw_line` scans `while on_line { bump }`, not a test of each token's class whose code is the same.
pub(crate) fn joined(b: &mut Block) {
    blocks(b, &mut |x| {
        for op in x.iter_mut() {
            match op {
                Op::If { arms, els } => {
                    // An arm whose code an arm before has, where no arm between them passes its test or none passes
                    // the one before's: one arm, at the first's place or at the second's.
                    let mut out: Vec<(Cond, Block)> = Vec::new();
                    for (c, body) in std::mem::take(arms) {
                        let up = (0..out.len()).rev().find(|&i| out[i].1 == body && out[i + 1..].iter().all(|(d, _)| disjoint(d, &c)));
                        let down = || (0..out.len()).rev().find(|&i| out[i].1 == body && out[i + 1..].iter().all(|(d, _)| disjoint(d, &out[i].0)));
                        if let Some(i) = up {
                            out[i].0 = Cond::or(std::mem::replace(&mut out[i].0, Cond::text("true")), c);
                        } else if let Some(i) = down() {
                            let (d, _) = out.remove(i);
                            out.push((Cond::or(d, c), body));
                        } else {
                            out.push((c, body));
                        }
                    }
                    while out.len() > 1 && els.as_ref().is_some_and(|e| *e == out[out.len() - 1].1) {
                        out.pop();
                    }
                    *arms = out;
                }
                Op::Match { subject, arms, default } => {
                    let mut out: Vec<(Pat, Block)> = Vec::new();
                    for (pat, body) in std::mem::take(arms) {
                        match out.last_mut() {
                            Some(last) if last.1 == body && last.0.guard.is_none() && pat.guard.is_none() => last.0.words.extend(pat.words),
                            Some(last) if last.1 == body => {
                                let c = Cond::or(last.0.cond(), pat.cond());
                                last.0 = Pat::guard(c);
                            }
                            _ => out.push((pat, body)),
                        }
                    }
                    while out.last().is_some_and(|a| a.1 == *default) {
                        out.pop();
                    }
                    if subject == "self.word()" && out.len() == 1 {
                        let (pat, body) = out.pop().expect("one arm");
                        *op = Op::If { arms: vec![(pat.cond(), body)], els: Some(std::mem::take(default)) };
                    } else {
                        *arms = out;
                    }
                }
                _ => {}
            }
        }
        for op in x.iter_mut() {
            if let Op::While(c, body) = op
                && let [Op::If { arms, els: Some(e) }] = body.as_slice()
                && let [(d, inner)] = arms.as_slice()
                && matches!(e.as_slice(), [Op::Break])
            {
                *op = Op::While(and(c.clone(), d.clone()), inner.clone());
            }
        }
        x.retain(|op| !matches!(op, Op::If { arms, els: None } if arms.iter().all(|(_, b)| b.is_empty())));
        // `if A || B { x; match self.word() { A => …, B => …, _ => refuse } }`, `x` taking no token: the `match`'s `_`
        // never runs, and the `match` of the word is an `if` chain, as the lean parser asks each.
        for op in x.iter_mut() {
            let Op::If { arms, .. } = op else { continue };
            for (c, body) in arms.iter_mut() {
                let ors: Vec<String> = match c.core() {
                    Cond::Or(cs) => cs.iter().map(|c| bare(&c.print()).to_owned()).collect(),
                    c => vec![bare(&c.print()).to_owned()],
                };
                let quiet = |o: &Op| matches!(o, Op::Field(_)) || matches!(o, Op::Line(l) if l.trim_start().starts_with("self.relabel_") || l.trim_start().starts_with("self.retag("));
                let Some((Op::Match { subject, arms: m, default }, pre)) = body.split_last_mut() else { continue };
                if subject != "self.word()" || !matches!(default.as_slice(), [Op::Refuse]) || !pre.iter().all(quiet) {
                    continue;
                }
                let tests: Vec<String> = m.iter().map(|(p, _)| bare(&p.cond().print()).to_owned()).collect();
                if tests.len() == ors.len() && tests.iter().all(|t| ors.contains(t)) {
                    let chain = Op::If { arms: m.iter().map(|(p, b)| (p.cond(), b.clone())).collect(), els: None };
                    *body.last_mut().expect("a match") = chain;
                }
            }
        }
        // `if A || B { if A { x } else if B { y } else { refuse } } else …`: `if A { x } else if B { y } else …`, each
        // asked once, as the lean parser's `match self.kind()`.
        for op in x.iter_mut() {
            let Op::If { arms, .. } = op else { continue };
            let mut out = Vec::new();
            for (c, body) in std::mem::take(arms) {
                let ors: Vec<String> = match c.core() {
                    Cond::Or(cs) => cs.iter().map(|c| bare(&c.print()).to_owned()).collect(),
                    c => vec![bare(&c.print()).to_owned()],
                };
                if let [Op::If { arms: inner, els: Some(e) }] = body.as_slice()
                    && matches!(e.as_slice(), [Op::Refuse])
                    && ors.len() > 1
                    && inner.len() == ors.len()
                    && inner.iter().zip(&ors).all(|((d, _), o)| bare(&d.print()) == o)
                {
                    out.extend(inner.iter().cloned());
                } else {
                    out.push((c, body));
                }
            }
            *arms = out;
        }
        // `if c {} else { y }`: `if !c { y }`.
        for op in x.iter_mut() {
            if let Op::If { arms, els } = op
                && arms.len() == 1
                && arms[0].1.is_empty()
                && let Some(e) = els.take()
            {
                let c = std::mem::replace(&mut arms[0].0, Cond::text("true"));
                arms[0] = (Cond::Not(Box::new(c)), e);
            }
        }
    });
}

/// `b` with each run of moves `seq` a call of `f` instead.
pub(crate) fn call_for(b: &mut Block, seq: &[Op], f: &str) {
    blocks(b, &mut |x| {
        let mut i = 0;
        while i + seq.len() <= x.len() {
            if x[i..i + seq.len()] == *seq {
                x.splice(i..i + seq.len(), [Op::Call(f.to_owned())]);
            }
            i += 1;
        }
    });
}

/// What a run of moves reads, for its function's name: the one field it sets (`f::…`), else the one kind of leaf it
/// makes (`k::…`), else the one rule it calls.
pub(crate) fn what_of(seq: &[Op]) -> Option<String> {
    let mut b = seq.to_vec();
    let (mut fields, mut kinds, mut calls) = (Vec::new(), Vec::new(), Vec::new());
    blocks(&mut b, &mut |x| {
        for o in x.iter() {
            let add = |v: &mut Vec<String>, s: &str| {
                if !v.iter().any(|x| x == s) {
                    v.push(s.to_owned());
                }
            };
            match o {
                Op::Field(f) => add(&mut fields, f),
                Op::Take { take, .. } => {
                    if let Take::Leaf { field, .. } = take
                        && field != "0"
                    {
                        add(&mut fields, field);
                    }
                    if let Take::Leaf { kind, .. } | Take::WordLeaf { kind, .. } | Take::Merge { kind, .. } = take
                        && kind.starts_with("k::")
                    {
                        add(&mut kinds, kind);
                    }
                }
                Op::Call(f) => add(&mut calls, f),
                _ => {}
            }
        }
    });
    let one = |v: Vec<String>| if let [x] = v.as_slice() { Some(x.rsplit("::").next().unwrap_or(x).trim_start_matches("r_").to_ascii_lowercase()) } else { None };
    one(fields).or_else(|| one(kinds)).or_else(|| one(calls))
}

/// The arms of each decision in `b` whose arms each open a node or only call a rule of `split` (its inline form, a
/// node's): those calls opening and closing the node themselves (`G::opened_by_caller`); each such rule in `opened`.
pub(crate) fn open_arms(b: &mut Block, split: &HashMap<String, Block>, marked: bool, opened: &mut Vec<String>) {
    let kind = |body: &Block| -> Option<Option<String>> {
        match body.as_slice() {
            [Op::Open(_), ..] => Some(None),
            [Op::Call(f)] if split.contains_key(f) => Some(Some(f.clone())),
            [Op::Refuse] => Some(None),
            _ => None,
        }
    };
    blocks(b, &mut |x| {
        for op in x.iter_mut() {
            let bodies: Vec<&mut Block> = match op {
                Op::If { arms, els } => arms.iter_mut().map(|(_, b)| b).chain(els.iter_mut()).collect(),
                Op::Match { arms, default, .. } => arms.iter_mut().map(|(_, b)| b).chain(std::iter::once(default)).collect(),
                _ => continue,
            };
            let ks: Option<Vec<Option<String>>> = bodies.iter().map(|b| kind(b)).collect();
            let Some(ks) = ks else { continue };
            if !ks.iter().any(Option::is_some) || bodies.iter().filter(|b| matches!(b.first(), Some(Op::Open(_)))).count() + ks.iter().flatten().count() < 2 {
                continue;
            }
            let calls: Vec<String> = ks.into_iter().flatten().collect();
            for body in bodies {
                if let [Op::Call(f)] = body.as_slice()
                    && let Some(inl) = split.get(f)
                {
                    let n = inl.len();
                    *body = vec![inl[0].clone(), Op::Call(f.clone()), inl[n - 2].clone()];
                    if marked {
                        body.push(inl[n - 1].clone());
                    }
                }
            }
            opened.extend(calls);
        }
    });
}

/// A `match` whose arms each open a node first and close it last (`Op::Open`, `Op::Close`), its `_` one of them or a
/// refusal: the node's kind a table on the word (`let kind = match …`), the node opened before the `match` and closed
/// after it, as the lean parser's `statement` opens a directive's node once (decision 0685 step 2).
pub(crate) fn open_once(b: &mut Block) {
    blocks(b, &mut open_once_in);
}

fn open_once_in(b: &mut Block) {
    let mut i = 0;
    while i < b.len() {
        let Op::Match { subject, arms, default } = &b[i] else {
            i += 1;
            continue;
        };
        let opens = |body: &Block| matches!((body.first(), body.last()), (Some(Op::Open(_)), Some(Op::Close)) if body.len() >= 2);
        let refuses = matches!(default.as_slice(), [Op::Refuse]);
        if arms.len() < 2 || !arms.iter().all(|(_, body)| opens(body)) || !(refuses || opens(default)) {
            i += 1;
            continue;
        }
        let kind = |body: &Block| match body.first() {
            Some(Op::Open(k)) => k.clone(),
            _ => unreachable!(),
        };
        let mut table = format!("let kind = match {subject} {{\n");
        for (p, body) in arms {
            let _ = writeln!(table, "    {} => {},", p.print(), kind(body));
        }
        let _ = writeln!(table, "    _ => {},\n}};", if refuses { "return self.refuse()".to_owned() } else { kind(default) });
        let strip = |body: &Block| body[1..body.len() - 1].to_vec();
        let m = Op::Match { subject: subject.clone(), arms: arms.iter().map(|(p, body)| (p.clone(), strip(body))).collect(), default: if refuses { default.clone() } else { strip(default) } };
        b.splice(i..=i, [Op::Line(table), Op::Open("kind".into()), m, Op::Close]);
        i += 4;
    }
}

/// `b` with each call of `f` its code `body` (`G::inlined`): whether it had one.
pub(crate) fn inline_call(b: &mut Block, f: &str, body: &Block) -> bool {
    let mut found = false;
    blocks(b, &mut |x| {
        if x.iter().any(|op| matches!(op, Op::Call(g) if g == f)) {
            found = true;
            *x = std::mem::take(x).into_iter().flat_map(|op| if matches!(&op, Op::Call(g) if g == f) { body.clone() } else { vec![op] }).collect();
        }
    });
    found
}

/// What the rewrites read of the grammar.
pub(crate) struct Cx<'a> {
    /// Where a mark no rule takes may hold (`G::open_mark_words`): the words, and the kinds none of whose tokens it can
    /// be (by `ident`).
    pub marks: Option<&'a (HashSet<String>, HashSet<String>)>,
    /// Whether a decision reads `marked` (`!self.marked`, `G::after_mark`): where none does, nothing writes it.
    pub marked: bool,
}

/// A rule's moves rewritten into the lean parser's forms, in order (decision 0681 step 2).
pub(crate) fn rewrite(b: &mut Block, cx: &Cx<'_>) {
    leaf_fields(b);
    takes(b, cx);
    blocks(b, &mut or_arms);
    blocks(b, &mut one_arm_a_take);
    blocks(b, &mut no_retests);
    blocks(b, &mut ifs_and_whiles);
    if !cx.marked {
        blocks(b, &mut |x| x.retain(|op| !matches!(op, Op::Line(l) if matches!(l.trim(), "self.marked = false;" | "self.marked = true;" | "self.marked = m.marked;"))));
    }
    blocks(b, &mut no_empty_elses);
    blocks(b, &mut no_code_after_return);
    blocks(b, &mut no_taken_retests);
    blocks(b, &mut shared_test_as_match);
    blocks(b, &mut no_outer_parens);
    blocks(b, &mut loops_as_whiles);
    blocks(b, &mut no_retest_after_while);
}

/// A leaf of no field and then the field set on the node made last, which is that leaf: one take of the field, as the
/// lean parser writes a token's field with it.
fn leaf_fields(b: &mut Block) {
    blocks(b, &mut |x| {
        let mut i = 0;
        while i + 1 < x.len() {
            let field = match &x[i + 1] {
                Op::Field(f) if f != "0" => Some(f.clone()),
                _ => None,
            };
            if let (Some(f), Op::Take { take, .. }) = (field, &mut x[i]) {
                let leaf = match take {
                    Take::Leaf { test, kind, field } if field == "0" => Some(Take::Leaf { test: test.clone(), kind: kind.clone(), field: f }),
                    Take::WordLeaf { word, kind } => Some(Take::Leaf { test: Cond::word(word.clone()), kind: kind.clone(), field: f }),
                    _ => None,
                };
                if let Some(l) = leaf {
                    *take = l;
                    x.remove(i + 1);
                }
            }
            i += 1;
        }
    });
}

/// Each take with the tests its token needs, as the lean parser asks at a token only what can hold there: `m`, the
/// marks no rule takes (`open_mark`), only where its token may be one of their words.
fn takes(b: &mut Block, cx: &Cx<'_>) {
    fn may(c: &Cond, words: &HashSet<String>, free: &HashSet<String>) -> bool {
        match c {
            Cond::Word { at: 0, ws } => ws.iter().any(|w| words.contains(w)),
            Cond::Kind { kind, .. } => !free.contains(&super::ident(kind)),
            Cond::And(cs) => cs.first().is_none_or(|c| may(c, words, free)),
            Cond::Mark { test, .. } | Cond::Group(test) => may(test, words, free),
            _ => true,
        }
    }
    takes_in(b, &mut |t, _, m| {
        *m = Some(match (cx.marks, t) {
            (None, _) => true,
            (Some((words, _)), Take::WordLeaf { word, .. } | Take::WordSkip { word } | Take::WordPass { word }) => words.contains(word),
            (Some((words, free)), t) => {
                let c = t.test().expect("a take's test");
                c.has_or() || may(c, words, free)
            }
        });
    });
}

/// An arm of `a || b` whose code is `if a { x } else { y }`: two arms, `a`'s `x` and `b`'s `y`, as the lean parser asks
/// each test once (a choice of tokens' arm, `punctuation`'s words or a `#` word).
fn or_arms(b: &mut Block) {
    for op in b.iter_mut() {
        let Op::If { arms, .. } = op else { continue };
        let mut out = Vec::new();
        for (c, body) in std::mem::take(arms) {
            if let Cond::Or(cs) = c.core()
                && cs.len() > 1
                && let [Op::If { arms: inner, els: Some(y) }] = body.as_slice()
                && let [(a, x)] = inner.as_slice()
                && *a == cs[0]
            {
                let rest = if cs.len() == 2 { cs[1].clone() } else { Cond::Or(cs[1..].to_vec()) };
                out.push((a.clone(), x.clone()));
                out.push((rest, y.clone()));
            } else {
                out.push((c, body));
            }
        }
        *arms = out;
    }
}

/// A `match self.word()`'s arms as the lean parser takes a class of tokens (`raw_line`'s loop, `is_p`): an arm whose
/// take tests the very token its pattern matched takes it with no test, and the arms whose code is then the same are
/// one arm, `w::A | w::B` (a kind's guards `_ if a || b`).
fn one_arm_a_take(b: &mut Block) {
    for op in b.iter_mut() {
        let Op::Match { subject, arms, .. } = op else { continue };
        if subject != "self.word()" {
            continue;
        }
        let mut out: Vec<(Pat, Block)> = Vec::new();
        for (pat, mut body) in std::mem::take(arms) {
            // The test the pattern made: its word, or a kind's guard of one test.
            if pat.one_word() {
                untested(&mut body, &pat.cond(), Some(&pat.words[0]));
            } else if pat.words.is_empty()
                && let Some(g) = &pat.guard
                && !g.has_or()
                && !matches!(g.core(), Cond::And(_))
            {
                untested(&mut body, g.core(), None);
            }
            let same = |a: &Block| *a == body;
            if pat.one_word()
                && let Some(a) = out.iter_mut().find(|a| same(&a.1) && !a.0.words.is_empty() && a.0.guard.is_none())
            {
                a.0.words.push(pat.words[0].clone());
                continue;
            }
            // A word under a guard: with the arm of the same guard and code, where no arm before names the word.
            if pat.words.len() == 1
                && pat.guard.is_some()
                && !out.iter().any(|a| a.0.words.contains(&pat.words[0]))
                && let Some(a) = out.iter_mut().find(|a| same(&a.1) && !a.0.words.is_empty() && a.0.guard == pat.guard)
            {
                a.0.words.push(pat.words[0].clone());
                continue;
            }
            if pat.words.is_empty()
                && let Some(g) = &pat.guard
                && let Some(last) = out.last_mut()
                && same(&last.1)
                && last.0.words.is_empty()
                && let Some(lg) = last.0.guard.take()
            {
                last.0.guard = Some(Cond::or(lg, g.clone()));
                continue;
            }
            out.push((pat, body));
        }
        *arms = out;
    }
}

/// `body` whose first move takes the token `cond` tests (or word `word`'s): that test `true`, the arm's pattern having
/// made it.
fn untested(body: &mut Block, cond: &Cond, word: Option<&str>) {
    let Some(Op::Take { take, .. }) = body.first_mut() else { return };
    let yes = Cond::text("true");
    let new = match (&*take, word) {
        (Take::WordLeaf { word: x, kind }, Some(w)) => (x == w).then(|| Take::Leaf { test: yes, kind: kind.clone(), field: "0".into() }),
        (Take::WordSkip { word: x }, Some(w)) => (x == w).then_some(Take::Skip { test: yes }),
        (Take::WordPass { word: x }, Some(w)) => (x == w).then_some(Take::Pass { test: yes }),
        (Take::WordLeaf { .. } | Take::WordSkip { .. } | Take::WordPass { .. }, _) => None,
        (t, _) => (t.test().map(Cond::core) == Some(cond.core())).then(|| {
            let mut t = t.clone();
            *t.test_mut().expect("a test") = yes;
            t
        }),
    };
    if let Some(n) = new {
        *take = n;
    }
}

/// Whether test `c` passed means `x` holds: `x` is `c`, or `c`'s last conjunct, no `||` before it.
fn implies(c: &Cond, x: &Cond) -> bool {
    let (c, x) = (c.core(), x.core());
    c == x || matches!(c, Cond::And(cs) if cs.last().map(Cond::core) == Some(x) && !cs[..cs.len() - 1].iter().any(Cond::has_or))
}

/// A peek's must-hold (`if !(X) { return self.refuse(); }`) first in an arm that tested it: none, as the lean parser
/// takes the token it peeked at.
fn no_retests(b: &mut Block) {
    let retest = |body: &Block, c: &Cond| -> bool {
        matches!(body.first(), Some(Op::If { arms, els: None }) if matches!(arms.as_slice(), [(Cond::Not(x), r)] if matches!(r.as_slice(), [Op::Refuse]) && implies(c, x)))
    };
    for op in b.iter_mut() {
        match op {
            Op::If { arms, .. } => {
                for (c, body) in arms {
                    if retest(body, c) {
                        body.remove(0);
                    }
                }
            }
            Op::Match { arms, .. } => {
                for (pat, body) in arms {
                    if pat.words.is_empty()
                        && let Some(g) = &pat.guard
                        && retest(body, g)
                    {
                        body.remove(0);
                    }
                }
            }
            _ => {}
        }
    }
}

/// A decision of one arm in the lean parser's form (decision 0668 step 1): a `match self.word()` of one arm and an empty
/// `_` is an `if`, as lean's `if self.is_p(X)`; a `loop` around one whose `_` breaks is a `while`.
fn ifs_and_whiles(b: &mut Block) {
    let one = |op: &Op| -> Option<(Cond, Block, Block)> {
        let Op::Match { subject, arms, default } = op else { return None };
        match arms.as_slice() {
            [(pat, body)] if subject == "self.word()" && (!pat.words.is_empty() || pat.guard.is_some()) => Some((pat.cond(), body.clone(), default.clone())),
            _ => None,
        }
    };
    for op in b.iter_mut() {
        if let Op::Loop(body) = op
            && let [m] = body.as_slice()
            && let Some((c, arm, default)) = one(m)
            && matches!(default.as_slice(), [Op::Break])
        {
            *op = Op::While(c, arm);
            continue;
        }
        if let Some((c, arm, default)) = one(op)
            && default.is_empty()
        {
            *op = Op::If { arms: vec![(c, arm)], els: None };
        }
    }
}

/// No `else` that does nothing, as the lean parser writes none: one whose moves all went (`marked`).
fn no_empty_elses(b: &mut Block) {
    for op in b.iter_mut() {
        if let Op::If { els, .. } = op
            && els.as_ref().is_some_and(|e| print(e, "").is_empty())
        {
            *els = None;
        }
    }
}

/// Nothing after a `return` in its block, as the lean parser writes none: a refusal a place's arm ends with leaves no
/// jump after it (the loop form's `return self.refuse();` then `st = …;`).
fn no_code_after_return(b: &mut Block) {
    let returns = |op: &Op| match op {
        Op::Refuse => true,
        Op::Line(s) => s.lines().last().is_some_and(|l| l.trim().starts_with("return ") && l.trim().ends_with(';')),
        _ => false,
    };
    if let Some(i) = b.iter().position(returns) {
        b.truncate(i + 1);
    }
}

/// No take testing again what the test right before it tested: where an `if`, `else if`, `while` or arm's guard tests
/// `C` and the block's first move takes the token if `C`, the take tests `true`, as the lean parser takes a token it
/// has tested with no test (decision 0673 step 9).
fn no_taken_retests(b: &mut Block) {
    let untest = |c: &Cond, body: &mut Block| {
        if let Some(Op::Take { take, .. }) = body.first_mut()
            && let Some(t) = take.test_mut()
            && t.core() == c.core()
        {
            *t = Cond::text("true");
        }
    };
    for op in b.iter_mut() {
        match op {
            Op::If { arms, .. } => arms.iter_mut().for_each(|(c, body)| untest(c, body)),
            Op::While(c, body) => untest(c, body),
            Op::Match { arms, .. } => {
                for (pat, body) in arms {
                    if pat.words.is_empty()
                        && let Some(g) = &pat.guard
                    {
                        untest(g, body);
                    }
                }
            }
            _ => {}
        }
    }
}

/// An `if … else if …` chain whose tests each begin with one test `P` of the word and then ask the next token's word,
/// its `else` the match of `P`'s word alone: `if P { match self.word_at(self.i + 1) { … } }`, as the lean parser asks
/// `.` once and matches the directive's word (`statement`).
fn shared_test_as_match(b: &mut Block) {
    for op in b.iter_mut() {
        let Op::If { arms, els: Some(e) } = op else { continue };
        let found = (|| {
            let first = arms.first()?.0.conjuncts();
            let shared = (*first.first()?).clone();
            let Cond::Word { at: 0, ws } = &shared else { return None };
            let [word] = ws.as_slice() else { return None };
            // Each arm: `P`, then the next token's words, then tests that are no choice: its pattern.
            let pats: Vec<Pat> = arms
                .iter()
                .map(|(t, _)| {
                    let cs = t.conjuncts();
                    let (p, rest) = cs.split_first()?;
                    let (Cond::Word { at: 1, ws }, guard) = (rest.first()?, &rest[1..]) else { return None };
                    if **p != shared || guard.iter().any(|g| g.has_or()) || (guard.is_empty() && ws.is_empty()) {
                        return None;
                    }
                    let guard = match guard {
                        [] => None,
                        [g] if ws.len() == 1 => Some((*g).clone()),
                        gs if ws.len() == 1 => Some(Cond::And(gs.iter().map(|g| (*g).clone()).collect())),
                        _ => return None,
                    };
                    Some(Pat { words: ws.clone(), guard })
                })
                .collect::<Option<_>>()?;
            let [Op::Match { subject, arms: m, default }] = e.as_slice() else { return None };
            let [(p, d)] = m.as_slice() else { return None };
            (subject == "self.word()" && p.one_word() && p.words[0] == *word && matches!(default.as_slice(), [Op::Refuse])).then(|| (shared.clone(), pats, d.clone()))
        })();
        let Some((shared, pats, d)) = found else { continue };
        let arms = std::mem::take(arms).into_iter().zip(pats).map(|((_, body), p)| (p, body)).collect();
        *op = Op::If { arms: vec![(shared, vec![Op::Match { subject: "self.word_at(self.i + 1)".into(), arms, default: d }])], els: Some(vec![Op::Refuse]) };
    }
}

/// Whether `c` is one parenthesised test.
fn one_group(c: &str) -> bool {
    c.starts_with('(') && c.ends_with(')') && {
        let mut depth = 0i32;
        c.char_indices().all(|(i, ch)| {
            depth += i32::from(ch == '(') - i32::from(ch == ')');
            depth > 0 || i == c.len() - 1
        })
    }
}

/// `c` with no outer parentheses where it is one parenthesised test, as an `if`'s test.
pub(crate) fn bare(c: &str) -> &str {
    if one_group(c) { &c[1..c.len() - 1] } else { c }
}

/// `if (C)` and `while (C)`, `C` one test in parentheses: `if C`.
fn no_outer_parens(b: &mut Block) {
    let bare = |c: &mut Cond| {
        if let Cond::Group(g) = c {
            *c = (**g).clone();
        } else if let Cond::Text(t) = c
            && one_group(t)
        {
            *c = Cond::text(bare(t));
        }
    };
    for op in b.iter_mut() {
        match op {
            Op::If { arms, .. } => arms.iter_mut().for_each(|(c, _)| bare(c)),
            Op::While(c, _) => bare(c),
            _ => {}
        }
    }
}

/// Each `loop { if C { … } else { break; } }` a `while C { … }`, as the lean parser writes a repetition (decision 0673
/// step 9); `loop { if C { break; } else if D { … } … }` a `while !C { if D { … } … }`, and `loop { if C { break; } else
/// { … } }` a `while !C { … }`, the loop's end its test, as the lean parser's `while self.on_line(…)`.
fn loops_as_whiles(b: &mut Block) {
    for op in b.iter_mut() {
        let Op::Loop(body) = op else { continue };
        let Some(Op::If { arms, els }) = body.first() else { continue };
        let (c, first) = &arms[0];
        let not = || Cond::Not(Box::new(c.clone()));
        let new = match (arms.len(), els.as_deref(), body.len()) {
            (1, Some([Op::Break]), 1) => Some(Op::While(c.clone(), first.clone())),
            (n, _, _) if n > 1 && matches!(first.as_slice(), [Op::Break]) => {
                let mut rest = body.clone();
                let Op::If { arms, .. } = &mut rest[0] else { unreachable!() };
                arms.remove(0);
                Some(Op::While(not(), rest))
            }
            (1, Some(e), 1) if matches!(first.as_slice(), [Op::Break]) => Some(Op::While(not(), e.to_vec())),
            _ => None,
        };
        if let Some(n) = new {
            *op = n;
        }
    }
}

/// `while !C { if … {…} else if … {…} else { break; } }` then `if !C { return self.refuse(); }`: the loop ends only
/// where `C` holds or at its `break`, where it does not, at the token the loop's test read; so the `break` refuses and
/// the test after goes, as the lean parser's `while … on_line { word()? }`.
fn no_retest_after_while(b: &mut Block) {
    fn breaks(b: &Block) -> bool {
        b.iter().any(|op| match op {
            Op::Break => true,
            Op::If { arms, els } => arms.iter().any(|(_, x)| breaks(x)) || els.as_ref().is_some_and(breaks),
            Op::Match { arms, default, .. } => arms.iter().any(|(_, x)| breaks(x)) || breaks(default),
            Op::At(_, x) => breaks(x),
            _ => false,
        })
    }
    let negated = |c: &Cond| match c.core() {
        Cond::Not(x) => Some(x.core().clone()),
        _ => None,
    };
    let mut i = 0;
    while i + 1 < b.len() {
        let found = match (&b[i], &b[i + 1]) {
            (Op::While(c, body), Op::If { arms, els: None }) => {
                matches!(body.as_slice(), [Op::If { arms: chain, els: Some(e) }] if matches!(e.as_slice(), [Op::Break]) && chain.iter().all(|(_, x)| !breaks(x)))
                    && matches!(arms.as_slice(), [(a, r)] if matches!(r.as_slice(), [Op::Refuse]) && negated(a).is_some() && negated(a) == negated(c))
            }
            _ => false,
        };
        if found {
            if let Op::While(_, body) = &mut b[i]
                && let Some(Op::If { els: Some(e), .. }) = body.first_mut()
            {
                *e = vec![Op::Refuse];
            }
            b.remove(i + 1);
        }
        i += 1;
    }
}
