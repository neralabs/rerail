//! A rule's code as the lean parser's moves (decision 0681): the generator builds them, rewrites them, and prints
//! Rust from them once.

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
    /// A take of the token; `nf`: in a rule that runs with no mark refused (`records::no_refusing`).
    Take { take: Take, nf: bool },
    /// An `if` chain: each test and its block, then the `else`.
    If { arms: Vec<(Cond, Block)>, els: Option<Block> },
    /// A `match` on `subject`: each pattern and its block, then `_`'s.
    Match { subject: String, arms: Vec<(String, Block)>, default: Block },
    Loop(Block),
    Break,
    /// `return self.refuse();`
    Refuse,
    /// The loop form's jump to a place.
    Goto(usize),
}

pub(crate) type Block = Vec<Op>;

/// Lines `s`, one move.
pub(crate) fn line(s: impl Into<String>) -> Op {
    Op::Line(s.into())
}

impl Take {
    fn print(&self, nf: bool) -> String {
        let nf = if nf { "_nf" } else { "" };
        match self {
            Take::Leaf { test, kind, field } => format!("self.tk{nf}({}, {kind}, {field})?;", test.print()),
            Take::WordLeaf { word, kind } => format!("self.word_leaf{nf}({word}, {kind})?;"),
            Take::WordSkip { word } => format!("self.word_skip{nf}({word})?;"),
            Take::Skip { test } => format!("self.skip{nf}({})?;", test.print()),
            Take::Merge { test, kind } => format!("self.merge{nf}({}, {kind})?;", test.print()),
            Take::Pass { test } => format!("self.pass{nf}({})?;", test.print()),
            Take::WordPass { word } => format!("self.word_pass{nf}({word})?;"),
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
            Op::Take { take, nf } => {
                let _ = writeln!(out, "{pad}{}", take.print(*nf));
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
            Op::Break => {
                let _ = writeln!(out, "{pad}break;");
            }
            Op::Refuse => {
                let _ = writeln!(out, "{pad}return self.refuse();");
            }
            Op::Goto(l) => {
                let _ = writeln!(out, "{pad}st = {l};\n{pad}continue 'run;");
            }
        }
    }
}

/// `b` with each take one of a rule that runs with no mark refused (`records::no_refusing`).
pub(crate) fn no_refusing(b: &mut [Op]) {
    for op in b {
        match op {
            Op::Take { nf, .. } => *nf = true,
            Op::If { arms, els } => {
                arms.iter_mut().for_each(|(_, b)| no_refusing(b));
                if let Some(e) = els {
                    no_refusing(e);
                }
            }
            Op::Match { arms, default, .. } => {
                arms.iter_mut().for_each(|(_, b)| no_refusing(b));
                no_refusing(default);
            }
            Op::Loop(b) => no_refusing(b),
            _ => {}
        }
    }
}
