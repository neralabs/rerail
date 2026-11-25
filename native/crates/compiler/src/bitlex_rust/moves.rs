//! A rule's code as the lean parser's moves (decision 0681): the generator builds them, rewrites them, and prints
//! Rust from them once.

use std::collections::HashSet;
use std::fmt::Write;

/// A test at the token, as the lean parser asks it: the word at a place, the token's kind, a mark, a flag, and `!`,
/// `&&` and `||` of tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Cond {
    /// A test the moves do not model yet: its text.
    Text(String),
    /// The word of the token `at` places on is one of `ws` (`w::…`): the lean parser's `is_p` and `is_p_at`.
    Word { at: usize, ws: Vec<String> },
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
            Cond::Kind { test, .. } => test.clone(),
            Cond::Mark { test, .. } => test.print(),
            Cond::Flag(f) => format!("self.flags & {f} == 0"),
            Cond::Scan { f, at, tail, held } => {
                let call = if *held { f.clone() } else { format!("self.{f}({at})") };
                if tail.is_empty() { format!("{call}.is_some()") } else { format!("{call}.is_some_and(|e| {tail})") }
            }
            Cond::Not(c) => {
                let t = c.print();
                if one_group(&t) || (t.starts_with("self.") && !t.contains(' ')) { format!("!{t}") } else { format!("!({t})") }
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

    /// Whether it holds where one of two tests does (an `||` in it, its text's too).
    pub(crate) fn has_or(&self) -> bool {
        match self {
            Cond::Text(s) | Cond::Kind { test: s, .. } | Cond::Scan { tail: s, .. } => s.contains("||"),
            Cond::Or(cs) => cs.len() > 1 || cs.iter().any(Cond::has_or),
            Cond::And(cs) => cs.iter().any(Cond::has_or),
            Cond::Mark { test: c, .. } | Cond::Not(c) | Cond::Group(c) => c.has_or(),
            Cond::Word { .. } | Cond::Flag(_) => false,
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
    fn name(&self) -> &'static str {
        match self {
            Take::Leaf { .. } => "tk",
            Take::WordLeaf { .. } => "word_leaf",
            Take::WordSkip { .. } => "word_skip",
            Take::Skip { .. } => "skip",
            Take::Merge { .. } => "merge",
            Take::Pass { .. } => "pass",
            Take::WordPass { .. } => "word_pass",
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

    /// Its arguments, the first its test or its word.
    fn args(&self) -> Vec<String> {
        match self {
            Take::Leaf { test, kind, field } => vec![test.print(), kind.clone(), field.clone()],
            Take::WordLeaf { word, kind } => vec![word.clone(), kind.clone()],
            Take::WordSkip { word } | Take::WordPass { word } => vec![word.clone()],
            Take::Skip { test } | Take::Pass { test } => vec![test.print()],
            Take::Merge { test, kind } => vec![test.print(), kind.clone()],
        }
    }

    fn print(&self, nf: bool, m: Option<bool>) -> String {
        let args = self.args().join(", ");
        match m {
            Some(m) => format!("self.{}_as::<{m}, {}>({args})?;", self.name(), !nf),
            None => format!("self.{}{}({args})?;", self.name(), if nf { "_nf" } else { "" }),
        }
    }
}

/// `b` as Rust, each line at `pad`.
pub(crate) fn print(b: &[Op], pad: &str) -> String {
    let mut out = String::new();
    print_into(b, pad, &mut out);
    out
}

fn print_into(b: &[Op], pad: &str, out: &mut String) {
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
            Op::Take { take, nf, m } => {
                let _ = writeln!(out, "{pad}{}", take.print(*nf, *m));
            }
            Op::If { arms, els } => {
                for (i, (c, body)) in arms.iter().enumerate() {
                    if i == 0 {
                        let _ = writeln!(out, "{pad}if {} {{", c.print());
                    } else {
                        let _ = writeln!(out, " else if {} {{", c.print());
                    }
                    print_into(body, &q, out);
                    out.push_str(pad);
                    out.push('}');
                }
                match els {
                    Some(e) => {
                        out.push_str(" else {\n");
                        print_into(e, &q, out);
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
                    print_into(body, &r, out);
                    let _ = writeln!(out, "{q}}}");
                }
                let _ = writeln!(out, "{q}_ => {{");
                print_into(default, &r, out);
                let _ = writeln!(out, "{q}}}\n{pad}}}");
            }
            Op::Loop(body) => {
                let _ = writeln!(out, "{pad}loop {{");
                print_into(body, &q, out);
                let _ = writeln!(out, "{pad}}}");
            }
            Op::While(c, body) => {
                let _ = writeln!(out, "{pad}while {} {{", c.print());
                print_into(body, &q, out);
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
            Op::At(p, b) => print_into(b, p, out),
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
