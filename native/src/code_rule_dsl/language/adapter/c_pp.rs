//! C's and C++'s directives as facts (decision 0327, mode (a)): the answer of `c_pp_file_req(path)`, a file's
//! directives whatever the configuration, by `rerail_c_pp::directives::scan`. The rules that read them are
//! `docs/rule-syntax/lib/c/preprocess.rr`. Positions are the file's bytes; `line` is from 1.

use crate::code_rule_dsl::language::{Cell, FileAnswerer};
use rerail_c_pp::Options;
use rerail_c_pp::directives::{self, Form, Problem, Span, Variadic};

/// The answerer of `c_pp_file_req`.
pub struct Directives;

const RELATIONS: &[&str] = &["c_directive", "c_define", "c_undef", "c_include", "c_cond", "c_cond_next", "c_cond_parent", "c_pragma", "c_line", "c_text", "c_guard", "c_pp_problem"];

/// How a file's directives are read: a `.c` file as C23 with GNU's extensions, every other (a header could be
/// either) as C++26 with them. The two differ only where C++ has a token C lacks (a raw string, `::` before C23).
pub fn options_for(path: &str) -> Options {
    if path.ends_with(".c") || path.ends_with(".m") { Options::c(23, true) } else { Options::cpp(26, true) }
}

fn text(b: &[u8]) -> Cell {
    Cell::Text(String::from_utf8_lossy(b).into_owned())
}

impl FileAnswerer for Directives {
    fn request(&self) -> &'static str {
        "c_pp_file_req"
    }

    fn produces(&self) -> &'static [&'static str] {
        RELATIONS
    }

    fn answer(&self, path: &str, bytes: &[u8], wanted: &dyn Fn(&str) -> bool, out: &mut dyn FnMut(&'static str, &[Cell])) {
        let d = directives::scan(bytes, options_for(path));
        let int = |v: u32| Cell::Int(i64::from(v));
        let span = |s: Span| {
            let (a, b) = d.file(s);
            [Cell::Int(i64::from(a)), Cell::Int(i64::from(b))]
        };
        let word = |s: Span| text(d.text(s));
        if wanted("c_directive") {
            for (i, x) in d.all.iter().enumerate() {
                let [a, b] = span(x.span);
                out("c_directive", &[int(i as u32), Cell::Text(x.kind.name().into()), a, b, int(x.line)]);
            }
        }
        if wanted("c_cond_parent") {
            for (i, x) in d.all.iter().enumerate() {
                if let Some(p) = x.parent {
                    out("c_cond_parent", &[int(i as u32), int(p)]);
                }
            }
        }
        if wanted("c_define") {
            for x in &d.defines {
                let params: Vec<&[u8]> = x.params.iter().map(|p| d.text(*p)).collect();
                let variadic = match x.variadic {
                    Variadic::No => "",
                    Variadic::Standard => "standard",
                    Variadic::Named => "named",
                };
                let [a, b] = span(x.body);
                out("c_define", &[int(x.i), word(x.name), Cell::Int(i64::from(x.fnlike)), text(&params.join(&b","[..])), Cell::Text(variadic.into()), a, b]);
            }
        }
        if wanted("c_undef") {
            for (i, name) in &d.undefs {
                out("c_undef", &[int(*i), word(*name)]);
            }
        }
        if wanted("c_include") {
            for x in &d.includes {
                let form = match x.form {
                    Form::Quote => "quote",
                    Form::Angle => "angle",
                    Form::Macro => "macro",
                };
                out("c_include", &[int(x.i), Cell::Text(x.kind.name().into()), Cell::Text(form.into()), word(x.spelled)]);
            }
        }
        if wanted("c_cond") || wanted("c_cond_next") {
            for x in &d.conds {
                let [a, b] = span(x.expr);
                if wanted("c_cond") {
                    out("c_cond", &[int(x.i), Cell::Text(x.kind.name().into()), a, b]);
                }
                if let (true, Some(j)) = (wanted("c_cond_next"), x.next) {
                    out("c_cond_next", &[int(x.i), int(j)]);
                }
            }
        }
        if wanted("c_pragma") {
            for x in &d.pragmas {
                out("c_pragma", &[int(x.i), word(x.first), word(x.second), word(x.text)]);
            }
        }
        if wanted("c_line") {
            for x in &d.lines {
                // A line given by macros (`n` unknown) is -1, its tokens in `file`'s place.
                let n = x.n.map_or(-1, |n| i64::try_from(n).unwrap_or(i64::MAX));
                let file = if x.n.is_some() { word(x.file) } else { word(x.rest) };
                out("c_line", &[int(x.i), Cell::Int(n), file]);
            }
        }
        if wanted("c_text") {
            for x in &d.texts {
                out("c_text", &[int(x.i), word(x.text)]);
            }
        }
        if let (true, Some((i, name))) = (wanted("c_guard"), d.guard) {
            out("c_guard", &[int(i), word(name)]);
        }
        if wanted("c_pp_problem") {
            for p in &d.problems {
                let (i, kind) = match *p {
                    Problem::Unmatched(i) => (i64::from(i), "unmatched"),
                    Problem::AfterElse(i) => (i64::from(i), "after_else"),
                    Problem::Unterminated(i) => (i64::from(i), "unterminated"),
                    Problem::Malformed(i) => (i64::from(i), "malformed"),
                    Problem::UnclosedComment => (-1, "unclosed_comment"),
                };
                out("c_pp_problem", &[Cell::Int(i), Cell::Text(kind.into())]);
            }
        }
    }
}
