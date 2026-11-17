//! Mode (b) compiled (decision 0348): a translation unit's macro expansion as preprocessing
//! words (`expand/expand.rir`), run over the files' token tapes (`tokens`), held to the lean
//! preprocessor (`rerail-c-pp`).

use crate::tokens::{self, BOL, SPACE};
use rerail_compiler::pp_words::{self, Bounds, File, Tok};
use rerail_compiler::program::Words;

pub const EXPAND: &str = include_str!("../expand/expand.rir");

/// The program's text for C (its `; C++ only` lines left out) or C++.
pub fn text(cpp: bool) -> String {
    EXPAND.lines().filter(|l| cpp || !l.trim_end().ends_with("; C++ only")).map(|l| format!("{l}\n")).collect()
}

static C: std::sync::OnceLock<Result<Words, String>> = std::sync::OnceLock::new();
static CPP: std::sync::OnceLock<Result<Words, String>> = std::sync::OnceLock::new();

/// The process's expansion program for C or C++, assembled and verified the first time.
pub fn program(cpp: bool) -> Result<&'static Words, &'static str> {
    let cell = if cpp { &CPP } else { &C };
    cell.get_or_init(|| rerail_compiler::rir_asm::assemble(&text(cpp))).as_ref().map_err(String::as_str)
}

/// A token tape's kind, as the words know it.
fn kind(k: &str) -> u8 {
    match k {
        "identifier" => pp_words::IDENT,
        "number" => pp_words::NUMBER,
        "literal" => pp_words::LITERAL,
        "punct" => pp_words::PUNCT,
        "digraph" => pp_words::DIGRAPH,
        "hash" => pp_words::HASH,
        "hashhash" => pp_words::HASHHASH,
        _ => pp_words::OTHER,
    }
}

/// A file's tape as the words read it (`buf`: the file's index + 1). A token a splice is inside
/// (only a literal can hold one, `tokens`) is spelled as phase 2 spells it, in `spelled` (buffer
/// `spelled_buf`).
pub fn tape(src: &[u8], toks: &[tokens::Tok], file: u32, spelled: &mut Vec<u8>, spelled_buf: u32) -> Vec<Tok> {
    let flag = |t: &tokens::Tok, f: u8, to: u8| if t.flags & f != 0 { to } else { 0 };
    toks.iter()
        .map(|t| {
            let mut x = Tok { kind: kind(t.kind), flags: flag(t, BOL, pp_words::BOL) | flag(t, SPACE, pp_words::SPACE), buf: file + 1, start: t.start, end: t.end };
            let b = &src[t.start as usize..t.end as usize];
            if x.kind == pp_words::LITERAL && b.windows(2).any(|w| w == b"\\\n" || w == b"\\\r") {
                let start = spelled.len() as u32;
                let mut k = 0;
                while k < b.len() {
                    let n = if b[k] != b'\\' { 0 } else if b.get(k + 1) == Some(&b'\n') { 2 } else if b.get(k + 1..k + 3) == Some(b"\r\n") { 3 } else { 0 };
                    if n == 0 {
                        spelled.push(b[k]);
                    }
                    k += n.max(1);
                }
                (x.buf, x.start, x.end) = (spelled_buf + 1, start, spelled.len() as u32);
            }
            x
        })
        .collect()
}

/// An output token: its kind, `BOL` and `SPACE` (the words'), and its spelling.
pub type Out = (u8, u8, Vec<u8>);

/// `src` (a TU of one file, no includes) expanded by the words; `Err` where its tokens or the
/// words refuse it.
pub fn expand(src: &[u8], cpp: bool) -> Result<Vec<Out>, String> {
    expand_tokens(src, cpp, tokens::tokens(src, cpp))
}

/// `src` expanded over its tokens `toks` (`None`: the token grammar refused it).
pub fn expand_tokens(src: &[u8], cpp: bool, toks: Option<Vec<tokens::Tok>>) -> Result<Vec<Out>, String> {
    let program = program(cpp)?;
    let toks = toks.ok_or("the token grammar refuses the file")?;
    let mut spelled = Vec::new();
    let tape = tape(src, &toks, 0, &mut spelled, 1);
    let files = [File { bytes: src, toks: &tape }, File { bytes: &spelled, toks: &[] }];
    let e = pp_words::run(program, &files, &Bounds::of(&files)).map_err(|r| format!("refused at word {}: {}", r.pc, r.why))?;
    Ok(e.toks.iter().map(|t| (t.kind, t.flags & (pp_words::BOL | pp_words::SPACE), e.text(&files, t).to_vec())).collect())
}

/// `src` expanded by the lean preprocessor, in the same terms.
pub fn lean(src: &[u8], cpp: bool) -> Vec<Out> {
    use rerail_c_lean::pp::{DIGRAPH, P};
    use rerail_c_pp::files::Search;
    use rerail_c_pp::macros::Tk;
    use rerail_c_pp::sink::{At, Sink};
    use rerail_c_pp::{Config, Options, Pp};
    struct Toks(Vec<Out>);
    impl Sink for Toks {
        fn token(&mut self, t: &Tk, text: &[u8], _at: Option<At>, _off: u32) {
            let kind = match t.kind {
                P::Ident => pp_words::IDENT,
                P::Number => pp_words::NUMBER,
                P::Char | P::Str | P::RawStr => pp_words::LITERAL,
                P::Hash => pp_words::HASH,
                P::HashHash => pp_words::HASHHASH,
                P::Punct if t.flags & DIGRAPH != 0 => pp_words::DIGRAPH,
                P::Punct => pp_words::PUNCT,
                _ => pp_words::OTHER,
            };
            let flags = if t.flags & BOL != 0 { pp_words::BOL } else { 0 } | if t.flags & SPACE != 0 { pp_words::SPACE } else { 0 };
            self.0.push((kind, flags, text.to_vec()));
        }
    }
    let opts = if cpp { Options::cpp(26, true) } else { Options::c(23, true) };
    let src = src.to_vec();
    let mut io = Search::new(Vec::new(), move |p: &str| (p == "main.c").then(|| src.clone()));
    let mut sink = Toks(Vec::new());
    let mut pp = Pp::new(Config::new(opts), &mut io, &mut sink);
    pp.run("main.c");
    drop(pp);
    sink.0
}

#[cfg(test)]
mod tests;
