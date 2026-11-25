//! A rule's code as the lean parser's moves (decision 0681): the generator builds them, rewrites them, and prints
//! Rust from them once.

use std::collections::HashSet;
use std::fmt::Write;

/// A test at the token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Cond {
    /// A test the moves do not model yet: its text.
    Text(String),
    Not(Box<Cond>),
    Or(Vec<Cond>),
}

impl Cond {
    pub(crate) fn text(s: impl Into<String>) -> Self {
        Cond::Text(s.into())
    }

    /// The test as Rust.
    pub(crate) fn print(&self) -> String {
        match self {
            Cond::Text(s) => s.clone(),
            Cond::Not(c) => format!("!({})", c.print()),
            Cond::Or(cs) => cs.iter().map(Cond::print).collect::<Vec<_>>().join(" || "),
        }
    }

    /// The test with each `a` in its text `b`.
    pub(crate) fn replace(&mut self, a: &str, b: &str) {
        match self {
            Cond::Text(s) => *s = s.replace(a, b),
            Cond::Not(c) => c.replace(a, b),
            Cond::Or(cs) => cs.iter_mut().for_each(|c| c.replace(a, b)),
        }
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
    /// A take of the token. `nf`: in a rule that runs with no mark refused (`records::no_refusing`); `m`: whether a mark
    /// no rule takes may hold at its token (`takes`), once asked.
    Take { take: Take, nf: bool, m: Option<bool> },
    /// An `if` chain: each test and its block, then the `else`.
    If { arms: Vec<(Cond, Block)>, els: Option<Block> },
    /// A `match` on `subject`: each pattern and its block, then `_`'s.
    Match { subject: String, arms: Vec<(String, Block)>, default: Block },
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
                    let _ = writeln!(out, "{q}{pat} => {{");
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
    /// Where a mark no rule takes may hold (`G::mark_words`): the words, and the kinds none of whose tokens it can be.
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

/// The test a take's first argument is: its test, or none for a take of a word.
fn take_test(t: &mut Take) -> Option<&mut Cond> {
    match t {
        Take::Leaf { test, .. } | Take::Skip { test } | Take::Merge { test, .. } | Take::Pass { test } => Some(test),
        Take::WordLeaf { .. } | Take::WordSkip { .. } | Take::WordPass { .. } => None,
    }
}

/// No take testing again what the test right before it tested: where an `if`, `else if`, `while` or arm's guard tests
/// `C` and the block's first move takes the token if `C`, the take tests `true`, as the lean parser takes a token it
/// has tested with no test (decision 0673 step 9).
fn no_taken_retests(b: &mut Block) {
    let untest = |c: &str, body: &mut Block| {
        if let Some(Op::Take { take, .. }) = body.first_mut()
            && let Some(t) = take_test(take)
            && t.print() == c
        {
            *t = Cond::text("true");
        }
    };
    for op in b.iter_mut() {
        match op {
            Op::If { arms, .. } => arms.iter_mut().for_each(|(c, body)| untest(&c.print(), body)),
            Op::While(c, body) => untest(&c.print(), body),
            Op::Match { arms, .. } => {
                for (pat, body) in arms {
                    if let Some(c) = pat.strip_prefix("_ if ") {
                        untest(c, body);
                    }
                }
            }
            _ => {}
        }
    }
}

/// An `if … else if …` chain whose tests each begin with one test `P` and then ask the next token's word, its `else`
/// the match of `P`'s word alone: `if P { match self.word_at(self.i + 1) { … } }`, as the lean parser asks `.` once and
/// matches the directive's word (`statement`).
fn shared_test_as_match(b: &mut Block) {
    let next = "self.word_at(self.i + 1) == ";
    // A test's words after `P`, as a pattern: `w::X`, `w::X if R` or `w::X | w::Y`.
    let pattern = |q: &str| -> Option<String> {
        if let Some(inner) = q.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
            let ws: Option<Vec<&str>> = inner.split(" || ").map(|t| t.strip_prefix(next).filter(|w| !w.contains(' '))).collect();
            return ws.filter(|v| v.len() > 1).map(|v| v.join(" | "));
        }
        let r = q.strip_prefix(next)?;
        match r.split_once(" && ") {
            Some((w, rest)) if !w.contains(' ') && !rest.contains("||") => Some(format!("{w} if {rest}")),
            None if !r.contains(' ') => Some(r.to_owned()),
            _ => None,
        }
    };
    for op in b.iter_mut() {
        let Op::If { arms, els: Some(e) } = op else { continue };
        let found = (|| {
            let c = arms.first()?.0.print();
            let (shared, _) = c.split_once(" && ")?;
            let w = shared.strip_prefix("self.word() == ")?;
            let word = w.split_once(' ').map_or(w, |x| x.0);
            let pats: Vec<String> = arms.iter().map(|(t, _)| pattern(t.print().strip_prefix(shared)?.strip_prefix(" && ")?)).collect::<Option<_>>()?;
            let [Op::Match { subject, arms: m, default }] = e.as_slice() else { return None };
            let [(p, d)] = m.as_slice() else { return None };
            (subject == "self.word()" && p == word && matches!(default.as_slice(), [Op::Refuse])).then(|| (shared.to_owned(), pats, d.clone()))
        })();
        let Some((shared, pats, d)) = found else { continue };
        let arms = std::mem::take(arms).into_iter().zip(pats).map(|((_, body), p)| (p, body)).collect();
        *op = Op::If { arms: vec![(Cond::text(shared), vec![Op::Match { subject: "self.word_at(self.i + 1)".into(), arms, default: d }])], els: Some(vec![Op::Refuse]) };
    }
}

/// `!c`: with no second parentheses where `c` is one call or one parenthesised test.
fn negate(c: &str) -> String {
    if one_group(c) || (c.starts_with("self.") && !c.contains(' ')) { format!("!{c}") } else { format!("!({c})") }
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

/// The test `!c` negates, its outer parentheses gone: `!((a || b))` and `!(a || b)` the same.
fn negated(c: &str) -> Option<&str> {
    let mut c = c.strip_prefix('!')?;
    while one_group(c) {
        c = &c[1..c.len() - 1];
    }
    Some(c)
}

/// `c` with no outer parentheses where it is one parenthesised test, as an `if`'s test.
pub(crate) fn bare(c: &str) -> &str {
    if one_group(c) { &c[1..c.len() - 1] } else { c }
}

/// `if (C)` and `while (C)`, `C` one parenthesised test: `if C`.
fn no_outer_parens(b: &mut Block) {
    let bare = |c: &mut Cond| {
        let t = c.print();
        if one_group(&t) {
            *c = Cond::text(bare(&t));
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
        let c = c.print();
        let new = match (arms.len(), els.as_deref(), body.len()) {
            (1, Some([Op::Break]), 1) => Some(Op::While(Cond::text(c), first.clone())),
            (n, _, _) if n > 1 && matches!(first.as_slice(), [Op::Break]) => {
                let mut rest = body.clone();
                let Op::If { arms, .. } = &mut rest[0] else { unreachable!() };
                arms.remove(0);
                Some(Op::While(Cond::text(negate(&c)), rest))
            }
            (1, Some(e), 1) if matches!(first.as_slice(), [Op::Break]) => Some(Op::While(Cond::text(negate(&c)), e.to_vec())),
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
    let has_break = |b: &Block| print(b, "").lines().any(|l| l.trim().starts_with("break"));
    let mut i = 0;
    while i + 1 < b.len() {
        let found = match (&b[i], &b[i + 1]) {
            (Op::While(c, body), Op::If { arms, els: None }) => {
                let c = c.print();
                matches!(body.as_slice(), [Op::If { arms: chain, els: Some(e) }] if matches!(e.as_slice(), [Op::Break]) && chain.iter().all(|(_, x)| !has_break(x)))
                    && matches!(arms.as_slice(), [(a, r)] if matches!(r.as_slice(), [Op::Refuse]) && negated(&a.print()).is_some() && negated(&a.print()) == negated(&c))
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

/// A leaf of no field and then the field set on the node made last, which is that leaf: one take of the field, as the
/// lean parser writes a token's field with it.
fn leaf_fields(b: &mut Block) {
    blocks(b, &mut |x| {
        let mut i = 0;
        while i + 1 < x.len() {
            let field = match &x[i + 1] {
                Op::Line(s) => s.strip_prefix("self.field_made(").and_then(|r| r.strip_suffix(");")).filter(|f| f.starts_with("f::") && !f.contains(' ')).map(str::to_owned),
                _ => None,
            };
            if let (Some(f), Op::Take { take, .. }) = (field, &mut x[i]) {
                let leaf = match take {
                    Take::Leaf { test, kind, field } if field == "0" => Some(Take::Leaf { test: test.clone(), kind: kind.clone(), field: f }),
                    Take::WordLeaf { word, kind } => Some(Take::Leaf { test: Cond::text(format!("self.word() == {word}")), kind: kind.clone(), field: f }),
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
    let may = |a: &str| -> bool {
        let Some((words, free)) = cx.marks else { return true };
        if a.contains("||") {
            return true;
        }
        let first = a.split(" && ").next().unwrap_or(a).trim();
        if let Some(ws) = first.strip_prefix("matches!(self.word(), ").and_then(|r| r.strip_suffix(')')) {
            return ws.split(" | ").any(|w| words.contains(w.trim()));
        }
        let w = first.strip_prefix("self.word() == ").unwrap_or(first);
        if w.starts_with("w::") && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':') {
            return words.contains(w);
        }
        let k = first.strip_prefix("self.kind() == t::").or_else(|| first.strip_prefix("matches!(self.kind(), t::")).map(|r| r.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next().unwrap_or(r));
        match k {
            Some(k) if !k.is_empty() => !free.contains(&k.to_ascii_lowercase()),
            _ => true,
        }
    };
    takes_in(b, &mut |t, _, m| *m = Some(may(&t.args()[0])));
}

fn is_word(p: &str) -> bool {
    p.starts_with("w::") && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
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
        let mut out: Vec<(String, Block)> = Vec::new();
        for (pat, mut body) in std::mem::take(arms) {
            let cond = if is_word(&pat) { Some(format!("self.word() == {pat}")) } else { pat.strip_prefix("_ if ").filter(|c| !c.contains("||") && !c.contains(" && ")).map(str::to_owned) };
            if let Some(c) = &cond {
                untested(&mut body, c, is_word(&pat).then_some(pat.as_str()));
            }
            let same = |a: &Block| print(a, "") == print(&body, "");
            if is_word(&pat)
                && let Some(a) = out.iter_mut().find(|a| same(&a.1) && a.0.split(" | ").all(is_word))
            {
                a.0 = format!("{} | {pat}", a.0);
                continue;
            }
            // A word under a guard: with the arm of the same guard and code, where no arm before names the word.
            if let Some((w, g)) = pat.split_once(" if ").filter(|(w, _)| is_word(w))
                && !out.iter().any(|a| a.0.split(" if ").next().is_some_and(|ws| ws.split(" | ").any(|x| x == w)))
                && let Some(a) = out.iter_mut().find(|a| same(&a.1) && a.0.split_once(" if ").is_some_and(|(ws, h)| h == g && ws.split(" | ").all(is_word)))
            {
                let (ws, _) = a.0.split_once(" if ").expect("a guard");
                a.0 = format!("{ws} | {w} if {g}");
                continue;
            }
            if let Some(c) = pat.strip_prefix("_ if ")
                && let Some(last) = out.last_mut()
                && same(&last.1)
                && last.0.starts_with("_ if ")
            {
                last.0 = format!("{} || {c}", last.0);
                continue;
            }
            out.push((pat, body));
        }
        *arms = out;
    }
}

/// `body` whose first move takes the token `cond` tests (or word `word`'s): that test `true`, the arm's pattern having
/// made it.
fn untested(body: &mut Block, cond: &str, word: Option<&str>) {
    let Some(Op::Take { take, .. }) = body.first_mut() else { return };
    let new = match (&*take, word) {
        (Take::WordLeaf { word: x, kind }, Some(w)) => (x == w).then(|| Take::Leaf { test: Cond::text("true"), kind: kind.clone(), field: "0".into() }),
        (Take::WordSkip { word: x }, Some(w)) => (x == w).then(|| Take::Skip { test: Cond::text("true") }),
        (Take::WordPass { word: x }, Some(w)) => (x == w).then(|| Take::Pass { test: Cond::text("true") }),
        (Take::WordLeaf { .. } | Take::WordSkip { .. } | Take::WordPass { .. }, _) => None,
        (Take::Leaf { test, kind, field }, _) => (test.print() == cond).then(|| Take::Leaf { test: Cond::text("true"), kind: kind.clone(), field: field.clone() }),
        (Take::Merge { test, kind }, _) => (test.print() == cond).then(|| Take::Merge { test: Cond::text("true"), kind: kind.clone() }),
        (Take::Skip { test }, _) => (test.print() == cond).then(|| Take::Skip { test: Cond::text("true") }),
        (Take::Pass { test }, _) => (test.print() == cond).then(|| Take::Pass { test: Cond::text("true") }),
    };
    if let Some(n) = new {
        *take = n;
    }
}

/// Whether test `c` passed means `x` holds: `x` is `c`, or `c`'s last conjunct, no `||` before it.
fn implies(c: &str, x: &str) -> bool {
    c == x || c.strip_suffix(x).and_then(|h| h.strip_suffix(" && ")).is_some_and(|h| !h.contains("||"))
}

/// A peek's must-hold (`if !(X) { return self.refuse(); }`) first in an arm that tested it: none, as the lean parser
/// takes the token it peeked at.
fn no_retests(b: &mut Block) {
    let retest = |body: &Block, c: &str| -> bool {
        let Some(first) = body.first() else { return false };
        let text = print(std::slice::from_ref(first), "");
        let mut ls = text.lines();
        matches!((ls.next(), ls.next(), ls.next(), ls.next()), (Some(a), Some(r), Some("}"), None) if r.trim() == "return self.refuse();" && a.strip_prefix("if !(").and_then(|x| x.strip_suffix(") {")).is_some_and(|x| implies(c, x)))
    };
    for op in b.iter_mut() {
        match op {
            Op::If { arms, .. } => {
                for (c, body) in arms {
                    if retest(body, &c.print()) {
                        body.remove(0);
                    }
                }
            }
            Op::Match { arms, .. } => {
                for (pat, body) in arms {
                    if let Some(c) = pat.strip_prefix("_ if ")
                        && retest(body, c)
                    {
                        body.remove(0);
                    }
                }
            }
            _ => {}
        }
    }
}

/// A `match self.word()` arm's pattern as a test: `self.word() == w::A`, `matches!(self.word(), w::A | w::B)`, a guard
/// joined by `&&`; a kind's guard (`_ if c`) its test.
fn pattern_cond(pat: &str) -> String {
    if let Some(c) = pat.strip_prefix("_ if ") {
        return c.to_owned();
    }
    let (words, guard) = match pat.split_once(" if ") {
        Some((w, g)) => (w, Some(g)),
        None => (pat, None),
    };
    let test = if words.contains(" | ") { format!("matches!(self.word(), {words})") } else { format!("self.word() == {words}") };
    match guard {
        Some(g) => format!("{test} && {g}"),
        None => test,
    }
}

/// A decision of one arm in the lean parser's form (decision 0668 step 1): a `match self.word()` of one arm and an empty
/// `_` is an `if`, as lean's `if self.is_p(X)`; a `loop` around one whose `_` breaks is a `while`.
fn ifs_and_whiles(b: &mut Block) {
    let one = |op: &Op| -> Option<(String, Block, String)> {
        let Op::Match { subject, arms, default } = op else { return None };
        match arms.as_slice() {
            [(pat, body)] if subject == "self.word()" && !pat.starts_with(' ') && pat != "_" && print(default, "").lines().count() <= 1 => Some((pat.clone(), body.clone(), print(default, ""))),
            _ => None,
        }
    };
    for op in b.iter_mut() {
        if let Op::Loop(body) = op
            && let [m] = body.as_slice()
            && let Some((pat, arm, default)) = one(m)
            && default.trim() == "break;"
        {
            *op = Op::While(Cond::text(pattern_cond(&pat)), arm);
            continue;
        }
        if let Some((pat, arm, default)) = one(op)
            && default.trim().is_empty()
        {
            *op = Op::If { arms: vec![(Cond::text(pattern_cond(&pat)), arm)], els: None };
        }
    }
}
