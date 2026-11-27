//! A `check KIND = G` as Rust: a function of the token's bytes, `G` over them, its names byte classes, its literals
//! bytes, each decision at the next byte (LL(1)), as the lean lexer reads a number by its grammar. Tier 2 as one loop
//! (`bitlex_rust::one_loop`) and tier 1 as Rust (`bitlex_lex_rust`) write it alike.

use ::bitlex::syntax::{Gram, Stmt};
use std::fmt::Write;

/// The writer of a lexer's checks: its classes, and whether a match may read the bytes past the token unchecked (the
/// source padded with spaces, which no check's byte is: `direct`).
pub struct Checks<'a> {
    pub lexer: &'a [Stmt],
    /// The writer named in an error (`tier 2 as one loop`, `tier 1 as Rust`).
    pub who: &'a str,
    direct: std::cell::Cell<bool>,
}

/// A byte set as a `match` pattern, its runs as ranges.
pub fn pattern(bytes: &[u8]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut k = 0;
    while k < bytes.len() {
        let mut j = k;
        while j + 1 < bytes.len() && bytes[j + 1] == bytes[j] + 1 {
            j += 1;
        }
        out.push(if j == k { format!("{:#04x}", bytes[k]) } else { format!("{:#04x}..={:#04x}", bytes[k], bytes[j]) });
        k = j + 1;
    }
    out.join(" | ")
}

impl<'a> Checks<'a> {
    pub fn new(lexer: &'a [Stmt], who: &'a str) -> Checks<'a> {
        Checks { lexer, who, direct: std::cell::Cell::new(false) }
    }

    fn err<X>(&self, what: impl std::fmt::Display) -> Result<X, String> {
        Err(format!("{}: {what}", self.who))
    }

    /// `check {kind}`'s function, `check_{name}(b) -> Option<usize>` (`name`: the kind as the writer's identifier): the end of the grammar's match from the start of
    /// `b`, or none. `padded`: `b` runs on past the token to padding of spaces, read without a bound where no byte of
    /// the check's is a space.
    pub fn check_fn(&self, name: &str, kind: &str, body: &Gram, padded: bool) -> Result<String, String> {
        self.direct.set(padded && !self.check_bytes(body)?.contains(&b' '));
        let code = self.check_code(body, false)?;
        self.direct.set(false);
        let mut out = String::new();
        let _ = write!(out, "/// `check {kind}`: the end of the check's grammar's match from the start of `b` (the bytes from the token's,\n/// to the {} end), or none.\nfn check_{}(b: &[u8]) -> Option<usize> {{\nlet mut j = 0usize;\n{}Some(j)\n}}\n\n", if padded { "padded source's" } else { "source's" }, name, code);
        Ok(out)
    }

    /// Every byte a check's grammar may read.
    pub fn check_bytes(&self, g: &Gram) -> Result<Vec<u8>, String> {
        Ok(match g {
            Gram::Lit(_) | Gram::Sym(_) => self.check_first(g)?.0,
            Gram::Seq(gs) | Gram::Alt(gs) => gs.iter().map(|g| self.check_bytes(g)).collect::<Result<Vec<_>, _>>()?.concat(),
            Gram::Opt(a) | Gram::Star(a) | Gram::Plus(a) => self.check_bytes(a)?,
            other => return self.err(format!("a check's {other:?}")),
        })
    }

    /// The bytes a check's grammar may start with, and whether it may match none.
    pub fn check_first(&self, g: &Gram) -> Result<(Vec<u8>, bool), String> {
        Ok(match g {
            Gram::Lit(t) if t.len() == 1 => (vec![t.as_bytes()[0]], false),
            Gram::Sym(n) => (self.class(n)?, false),
            Gram::Seq(gs) => {
                let mut out = Vec::new();
                for g in gs {
                    let (f, e) = self.check_first(g)?;
                    out.extend(f);
                    if !e {
                        return Ok((out, false));
                    }
                }
                (out, true)
            }
            Gram::Alt(gs) => {
                let (mut out, mut empty) = (Vec::new(), false);
                for g in gs {
                    let (f, e) = self.check_first(g)?;
                    out.extend(f);
                    empty |= e;
                }
                (out, empty)
            }
            Gram::Opt(a) | Gram::Star(a) => (self.check_first(a)?.0, true),
            Gram::Plus(a) => self.check_first(a)?,
            other => return self.err(format!("a check's {other:?}")),
        })
    }

    fn class(&self, name: &str) -> Result<Vec<u8>, String> {
        for s in self.lexer {
            if let Stmt::Class(n, items) = s
                && n == name
            {
                let mut out: Vec<u8> = items.iter().flat_map(|&(lo, hi)| (lo..=hi).map(|b| b as u8)).collect();
                out.sort_unstable();
                out.dedup();
                return Ok(out);
            }
        }
        self.err(format!("a check's `{name}`: no class"))
    }

    fn at_byte(&self, g: &Gram) -> Result<String, String> {
        let (mut f, _) = self.check_first(g)?;
        f.sort_unstable();
        f.dedup();
        Ok(if self.direct.get() { format!("matches!(b[j], {})", pattern(&f)) } else { format!("matches!(b.get(j), Some({}))", pattern(&f)) })
    }

    /// The code of a check's `g`; `tested`, whether the test before it found its first byte (an `[A]`, `A*`, `A+` or
    /// `A | B` found it there, so its code reads it as found).
    fn check_code(&self, g: &Gram, tested: bool) -> Result<String, String> {
        Ok(match g {
            Gram::Lit(_) | Gram::Sym(_) if tested => "j += 1;\n".to_owned(),
            Gram::Lit(_) | Gram::Sym(_) => format!("if !{} {{\nreturn None;\n}}\nj += 1;\n", self.at_byte(g)?),
            Gram::Seq(gs) => gs.iter().enumerate().map(|(k, g)| self.check_code(g, tested && k == 0)).collect::<Result<String, String>>()?,
            Gram::Opt(a) => format!("if {} {{\n{}}}\n", self.at_byte(a)?, self.check_code(a, true)?),
            Gram::Star(a) => format!("while {} {{\n{}}}\n", self.at_byte(a)?, self.check_code(a, true)?),
            Gram::Plus(a) => format!("{}while {} {{\n{}}}\n", self.check_code(a, tested)?, self.at_byte(a)?, self.check_code(a, true)?),
            // Each alternative one byte: one test of them all (`walk`'s exponent sign), none where the test before found it.
            Gram::Alt(gs) if gs.iter().all(|g| matches!(g, Gram::Lit(_) | Gram::Sym(_))) => {
                if tested {
                    "j += 1;\n".to_owned()
                } else {
                    format!("if !{} {{\nreturn None;\n}}\nj += 1;\n", self.at_byte(g)?)
                }
            }
            Gram::Alt(gs) => {
                let mut out = String::new();
                for g in gs {
                    let _ = write!(out, "if {} {{\n{}}} else ", self.at_byte(g)?, self.check_code(g, true)?);
                }
                out + "{\nreturn None;\n}\n"
            }
            other => return self.err(format!("a check's {other:?}")),
        })
    }
}
