//! A file's mode (a) facts (decision 0327): the rows the engine's `c_pp_file_req` answer gives,
//! made from the compiled grammar's tape (`Compiled::facts`) or from the lean scan
//! (`Facts::lean`), which is the reference and the fall-back. Positions are the file's bytes; a
//! text is the phase-2 text (its `\`-newlines removed), as the lean scan's `Directives::text`.

use crate::{Operand, Read, What};
use rerail_c_pp::directives::{self, Form, Problem, Span, Variadic};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Facts {
    pub directives: Vec<Directive>,
    pub defines: Vec<Define>,
    pub undefs: Vec<(u32, Vec<u8>)>,
    pub includes: Vec<Include>,
    pub conds: Vec<Cond>,
    pub pragmas: Vec<Pragma>,
    pub lines: Vec<Line>,
    pub texts: Vec<(u32, Vec<u8>)>,
    /// The include guard: its `#ifndef` (or `#if !defined`) and its macro.
    pub guard: Option<(u32, Vec<u8>)>,
    /// Each problem: its directive (-1: the file's) and its name (`c_pp_problem`).
    pub problems: Vec<(i64, &'static str)>,
}

/// `c_directive` and `c_cond_parent`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directive {
    pub kind: &'static str,
    pub start: u32,
    pub end: u32,
    pub line: u32,
    pub parent: Option<u32>,
}

/// `c_define`: `variadic` is "", "standard" or "named"; `body` the replacement list's bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Define {
    pub i: u32,
    pub name: Vec<u8>,
    pub fnlike: bool,
    pub params: Vec<Vec<u8>>,
    pub variadic: &'static str,
    pub body: (u32, u32),
}

/// `c_include`: `form` is "quote", "angle" or "macro".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Include {
    pub i: u32,
    pub kind: &'static str,
    pub form: &'static str,
    pub spelled: Vec<u8>,
}

/// `c_cond` and `c_cond_next`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cond {
    pub i: u32,
    pub kind: &'static str,
    pub expr: (u32, u32),
    pub next: Option<u32>,
}

/// `c_pragma`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pragma {
    pub i: u32,
    pub first: Vec<u8>,
    pub second: Vec<u8>,
    pub text: Vec<u8>,
}

/// `c_line`: `n` where the line's number is one, else its tokens in `rest`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub i: u32,
    pub n: Option<u64>,
    pub file: Vec<u8>,
    pub rest: Vec<u8>,
}

impl Facts {
    /// The lean scan's facts of `src` (`rerail_c_pp::directives::scan`).
    pub fn lean(src: &[u8], opts: rerail_c_pp::Options) -> Facts {
        let d = directives::scan(src, opts);
        let word = |s: Span| d.text(s).to_vec();
        let span = |s: Span| d.file(s);
        Facts {
            directives: d.all.iter().map(|x| {
                let (start, end) = span(x.span);
                Directive { kind: x.kind.name(), start, end, line: x.line, parent: x.parent }
            }).collect(),
            defines: d.defines.iter().map(|x| Define {
                i: x.i,
                name: word(x.name),
                fnlike: x.fnlike,
                params: x.params.iter().map(|p| word(*p)).collect(),
                variadic: match x.variadic {
                    Variadic::No => "",
                    Variadic::Standard => "standard",
                    Variadic::Named => "named",
                },
                body: span(x.body),
            }).collect(),
            undefs: d.undefs.iter().map(|(i, n)| (*i, word(*n))).collect(),
            includes: d.includes.iter().map(|x| Include {
                i: x.i,
                kind: x.kind.name(),
                form: match x.form {
                    Form::Quote => "quote",
                    Form::Angle => "angle",
                    Form::Macro => "macro",
                },
                spelled: word(x.spelled),
            }).collect(),
            conds: d.conds.iter().map(|x| Cond { i: x.i, kind: x.kind.name(), expr: span(x.expr), next: x.next }).collect(),
            pragmas: d.pragmas.iter().map(|x| Pragma { i: x.i, first: word(x.first), second: word(x.second), text: word(x.text) }).collect(),
            lines: d.lines.iter().map(|x| Line { i: x.i, n: x.n, file: word(x.file), rest: word(x.rest) }).collect(),
            texts: d.texts.iter().map(|x| (x.i, word(x.text))).collect(),
            guard: d.guard.map(|(i, n)| (i, word(n))),
            problems: d.problems.iter().map(|p| match *p {
                Problem::Unmatched(i) => (i64::from(i), "unmatched"),
                Problem::AfterElse(i) => (i64::from(i), "after_else"),
                Problem::Unterminated(i) => (i64::from(i), "unterminated"),
                Problem::Malformed(i) => (i64::from(i), "malformed"),
                Problem::UnclosedComment => (-1, "unclosed_comment"),
            }).collect(),
        }
    }

    /// The facts of `src` from its tape's reading (`read`).
    pub fn of(r: &Read, src: &[u8]) -> Facts {
        let text = |a: u32, b: u32| unsplice(&src[a as usize..b as usize]);
        let mut f = Facts { directives: Vec::new(), defines: Vec::new(), undefs: Vec::new(), includes: Vec::new(), conds: Vec::new(), pragmas: Vec::new(), lines: Vec::new(), texts: Vec::new(), guard: None, problems: Vec::new() };
        // Each directive's operands, in order (`read` gives them by directive).
        let mut ops: Vec<&[Operand]> = vec![&[]; r.rows.len()];
        let mut at = 0;
        while at < r.operands.len() {
            let i = r.operands[at].i as usize;
            let n = r.operands[at..].iter().take_while(|o| o.i as usize == i).count();
            ops[i] = &r.operands[at..at + n];
            at += n;
        }
        let next: std::collections::HashMap<u32, u32> = r.next.iter().copied().collect();
        let mut lines = rerail_c_pp::tape::LineCount::default();
        let mut malformed = Vec::new();
        for (i, row) in r.rows.iter().enumerate() {
            let i = i as u32;
            let o = ops[i as usize];
            let get = |w: What| o.iter().find(|x| x.what == w);
            let bytes = |w: What| get(w).map_or_else(Vec::new, |x| text(x.start, x.end));
            // An operand missing: an empty span where the word ends (the word's own: the lean
            // scan's `name.end`).
            let after_word = r.word_ends[i as usize];
            f.directives.push(Directive { kind: row.kind, start: row.start, end: row.end, line: lines.of(src, row.start as usize), parent: row.parent });
            match row.kind {
                "define" => match get(What::Name) {
                    Some(name) => {
                        let params: Vec<Vec<u8>> = o.iter().filter(|x| x.what == What::Param).map(|x| text(x.start, x.end)).collect();
                        let variadic = if get(What::NamedVariadic).is_some() {
                            "named"
                        } else if get(What::Variadic).is_some() {
                            "standard"
                        } else {
                            ""
                        };
                        let body = get(What::Body).map_or((name.end, name.end), |b| (b.start, b.end));
                        f.defines.push(Define { i, name: text(name.start, name.end), fnlike: get(What::Params).is_some(), params, variadic, body });
                    }
                    None => malformed.push(i),
                },
                "undef" => match get(What::Name) {
                    Some(n) => f.undefs.push((i, text(n.start, n.end))),
                    None => malformed.push(i),
                },
                "include" | "include_next" | "import" | "embed" => match o.iter().find(|x| matches!(x.what, What::Quote | What::Angle | What::Macro)) {
                    Some(s) => {
                        let form = match s.what {
                            What::Quote => "quote",
                            What::Angle => "angle",
                            _ => "macro",
                        };
                        f.includes.push(Include { i, kind: row.kind, form, spelled: text(s.start, s.end) });
                    }
                    None => malformed.push(i),
                },
                "if" | "ifdef" | "ifndef" | "elif" | "elifdef" | "elifndef" | "else" | "endif" => {
                    let expr = get(What::Expr).map_or((after_word, after_word), |x| (x.start, x.end));
                    f.conds.push(Cond { i, kind: row.kind, expr, next: next.get(&i).copied() });
                }
                "pragma" => f.pragmas.push(Pragma { i, first: bytes(What::First), second: bytes(What::Second), text: bytes(What::Text) }),
                "line" | "line_marker" => {
                    let n = get(What::Number).and_then(|x| std::str::from_utf8(&src[x.start as usize..x.end as usize]).ok()?.parse().ok());
                    f.lines.push(Line { i, n, file: bytes(What::File), rest: bytes(What::Rest) });
                }
                "null" => {}
                _ => f.texts.push((i, bytes(What::Text))),
            }
        }
        // The lean scan's order: unmatched and after-`#else` as met, the malformed as met.
        let mut problems: Vec<(u32, &'static str)> = r.unmatched.iter().map(|&i| (i, "unmatched")).chain(r.after_else.iter().map(|&i| (i, "after_else"))).chain(malformed.into_iter().map(|i| (i, "malformed"))).collect();
        problems.sort();
        f.problems = problems.into_iter().map(|(i, k)| (i64::from(i), k)).collect();
        f.guard = guard(r, src, &f);
        f
    }
}

/// The include guard: the file's top level one group and nothing else (no code, no other
/// directive), its chain its opener and its `#endif`, its opener `#ifndef X`, `#if !defined X`
/// or `#if !defined(X)`.
fn guard(r: &Read, src: &[u8], f: &Facts) -> Option<(u32, Vec<u8>)> {
    if r.top_code || r.top.len() != 1 || r.top[0] != 0 {
        return None;
    }
    let opener = f.conds.iter().find(|c| c.i == 0)?;
    let end = f.conds.iter().find(|c| Some(c.i) == opener.next)?;
    if end.kind != "endif" {
        return None;
    }
    let leaf = |k: usize| r.first_expr.get(k).map(|&(kind, a, b)| (kind, &src[a as usize..b as usize]));
    let name = |k: usize| leaf(k).filter(|l| l.0 == "identifier").map(|l| l.1.to_vec());
    let is = |k: usize, s: &[u8]| leaf(k).is_some_and(|l| l.1 == s);
    match (opener.kind, r.first_expr.len()) {
        ("ifndef", 1) => name(0),
        ("if", 3) if is(0, b"!") && is(1, b"defined") => name(2),
        ("if", 5) if is(0, b"!") && is(1, b"defined") && is(2, b"(") && is(4, b")") => name(3),
        _ => None,
    }
    .map(|n| (0, n))
}

/// `b` with its `\`-newlines removed (`\` then `\n` or `\r\n`, phase 2; the grammar refuses the
/// other splices): the
/// runs between them copied, found a `\` at a time.
fn unsplice(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut from = 0;
    let mut at = 0;
    while let Some(k) = rerail_simd8::micro::find(b, at, b'\\') {
        // `\` and `\n`, or `\` and `\r\n`.
        let past = match b.get(k + 1..k + 3) {
            Some([b'\r', b'\n']) => Some(k + 3),
            _ if b.get(k + 1) == Some(&b'\n') => Some(k + 2),
            _ => None,
        };
        if let Some(past) = past {
            out.extend_from_slice(&b[from..k]);
            from = past;
        }
        at = k + 1;
    }
    out.extend_from_slice(&b[from..]);
    out
}

