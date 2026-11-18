//! Mode (b) compiled (decision 0348): a translation unit's macro expansion as preprocessing
//! words (`expand/expand.rir`), run over the files' token tapes (`tokens`), held to the lean
//! preprocessor (`rerail-c-pp`).

use crate::tokens::{self, BOL, SPACE};
use rerail_compiler::pp_words::{self, Bounds, File, Tok};
use rerail_compiler::program::Words;

pub const EXPAND: &str = include_str!("../expand/expand.rir");

static PROGRAM: std::sync::OnceLock<Result<Words, String>> = std::sync::OnceLock::new();

/// The process's expansion program, assembled and verified the first time: one for C and C++,
/// the dialect a run's (`pp_words::Env::cpp`).
pub fn program() -> Result<&'static Words, &'static str> {
    PROGRAM.get_or_init(|| rerail_compiler::rir_asm::assemble(EXPAND)).as_ref().map_err(String::as_str)
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
            let mut x = Tok { kind: kind(t.kind), flags: flag(t, BOL, pp_words::BOL) | flag(t, SPACE, pp_words::SPACE), buf: file + 1, start: t.start, end: t.end, exp: 0 };
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

/// A directive's `#` on a file's token tape: its row's start, a `#` token's.
fn hash_at(read: &crate::Read, toks: &[Tok], row: u32) -> Option<u32> {
    let start = read.rows.get(row as usize)?.start;
    let k = toks.binary_search_by_key(&start, |t| t.start).ok()?;
    (toks[k].kind == pp_words::HASH).then_some(k as u32)
}

/// A file's conditionals' chains on its token tape (`File::chains`): each conditional's `#` and
/// its chain's next directive's, by mode (a)'s `c_cond_next`; `None` where a directive's `#` is
/// no token's start.
pub fn chains(read: &crate::Read, toks: &[Tok]) -> Option<Vec<(u32, u32)>> {
    let mut out = read.next.iter().map(|&(a, b)| Some((hash_at(read, toks, a)?, hash_at(read, toks, b)?))).collect::<Option<Vec<_>>>()?;
    out.sort_unstable();
    Some(out)
}

/// How a TU's files are lexed and their directives read: by the compiled grammars, or (tests) by
/// their interpreter.
pub struct Grammars<'a> {
    pub tokens: &'a dyn Fn(&[u8], bool) -> Option<Vec<tokens::Tok>>,
    pub read: &'a dyn Fn(&[u8]) -> Option<crate::Read>,
}

/// The compiled grammars.
pub fn compiled_grammars() -> Grammars<'static> {
    Grammars { tokens: &tokens::tokens, read: &|src| crate::compiled().ok().and_then(|c| c.read(src)) }
}

/// A file of the TU, as the host holds it.
struct Unit {
    path: String,
    src: Vec<u8>,
    lines: Vec<u32>,
    found_by: Option<u32>,
    /// Its directives, tape and chains: `None` for a file only embedded (data, never lexed).
    lexed: Option<Lexed>,
    includes: Vec<(u32, u32)>,
    named: Vec<pp_words::Named>,
    /// Its includes mode (a) read are resolved.
    resolved: bool,
}

struct Lexed {
    read: crate::Read,
    tape: Vec<Tok>,
    chains: Vec<(u32, u32)>,
    if_depth: u32,
}

/// The most calls `#if`'s evaluator (`expand.lab`'s `cond`) makes on any `#if` or `#elif` of a
/// file (mode (a)'s `c_cond` rows, their expressions' tokens on the tape), counted on the tokens
/// as written (an expansion's tokens can take it deeper; then the compiled words use the return
/// stack past their depth, as at depth 0). A primary is 3 calls past its `cond` (`cond`,
/// `ternary`, `binary`); a prefix operator 1 more; a binary operator 2 (its operand's `binary`
/// and `unary`, at most 10 a level: the precedences), counted from above; `(` 4 past the
/// `unary` it is in; `?`'s operand 2 past its level's primary, `:`'s 1.
pub fn if_depth(read: &crate::Read, src: &[u8], tape: &[Tok]) -> u32 {
    #[derive(Clone, Copy, PartialEq)]
    enum Open {
        Paren,
        Query,
        Colon,
    }
    let facts = crate::Facts::of(read, src);
    let mut most = 0;
    for c in facts.conds.iter().filter(|c| c.kind == "if" || c.kind == "elif") {
        let lo = tape.partition_point(|t| t.start < c.expr.0);
        let hi = tape.partition_point(|t| t.start < c.expr.1);
        // Each level open: its primary's calls, its binary operators met, what opened it.
        let mut open: Vec<(u32, u32, Open)> = vec![(3, 0, Open::Paren)];
        let (mut unary, mut operand) = (0, true);
        for t in &tape[lo..hi] {
            // A respelled literal's spelling is not the file's: an operand all the same.
            let text = src.get(t.start as usize..t.end as usize).unwrap_or_default();
            let punct = t.kind == pp_words::PUNCT;
            let (base, ops, _) = *open.last().unwrap();
            let at = base + 2 * ops.min(10) + unary;
            match text {
                b"(" if punct => {
                    open.push((at + 4, 0, Open::Paren));
                    (unary, operand) = (0, true);
                }
                b")" if punct => {
                    while open.len() > 1 && open.pop().unwrap().2 != Open::Paren {}
                    operand = false;
                }
                b"?" if punct => {
                    let level = open.iter().rposition(|o| o.2 != Open::Query).unwrap();
                    open.push((open[level].0 + 2, 0, Open::Query));
                    (unary, operand) = (0, true);
                }
                b":" if punct => {
                    while open.len() > 1 && open.last().unwrap().2 == Open::Colon {
                        open.pop();
                    }
                    if open.len() > 1 && open.last().unwrap().2 == Open::Query {
                        let q = open.pop().unwrap();
                        open.push((q.0 - 1, 0, Open::Colon));
                    }
                    (unary, operand) = (0, true);
                }
                b"," if punct => {
                    while open.len() > 1 && open.last().unwrap().2 != Open::Paren {
                        open.pop();
                    }
                    open.last_mut().unwrap().1 = 0;
                    (unary, operand) = (0, true);
                }
                b"+" | b"-" | b"~" | b"!" if punct && operand => unary += 1,
                _ if punct => {
                    open.last_mut().unwrap().1 += 1;
                    (unary, operand) = (0, true);
                }
                _ => {
                    unary = 0;
                    operand = false;
                }
            }
            let (base, ops, _) = *open.last().unwrap();
            most = most.max(base + 2 * ops.min(10) + unary);
        }
    }
    most
}

/// The compiled words, each depth `#if`'s evaluator is compiled at made the first time a TU
/// needs it (`compile_k`); the caller keeps it, so its programs serve every TU it expands.
pub struct Compiled {
    by_k: std::cell::RefCell<Vec<Option<rerail_compiler::native_cpu_pp::CompiledPp>>>,
}

impl Default for Compiled {
    fn default() -> Self {
        Self::new()
    }
}

impl Compiled {
    pub fn new() -> Self {
        Compiled { by_k: std::cell::RefCell::new((0..=rerail_compiler::native_cpu_pp::MAX_K.trailing_zeros() + 1).map(|_| None).collect()) }
    }

    /// The depth a TU's files take: their deepest `#if`'s calls, a power of two (0 for none),
    /// at most `MAX_K`.
    pub fn depth(files: &[File]) -> usize {
        let d = files.iter().map(|f| f.if_depth).max().unwrap_or(0) as usize;
        if d == 0 { 0 } else { d.next_power_of_two().min(rerail_compiler::native_cpu_pp::MAX_K) }
    }

    /// `pp_words::run`'s work by the compiled words at the files' depth.
    pub fn run(&self, program: &rerail_compiler::program::Words, files: &[File], env: &pp_words::Env, bounds: &Bounds) -> Result<pp_words::Expanded, pp_words::Refused> {
        let k = Self::depth(files);
        let slot = if k == 0 { 0 } else { k.trailing_zeros() as usize + 1 };
        let mut by_k = self.by_k.borrow_mut();
        if by_k[slot].is_none() {
            let c = rerail_compiler::native_cpu_pp::compile_k(program, k).map_err(|_| pp_words::Refused { pc: 0, why: "the words do not compile" })?;
            by_k[slot] = Some(c);
        }
        by_k[slot].as_ref().unwrap().run(program, files, env, bounds)
    }
}

/// The TU's files, found and read as the words need them.
struct Tu<'a, F: rerail_c_pp::files::Files> {
    io: &'a mut F,
    g: &'a Grammars<'a>,
    cpp: bool,
    units: Vec<Unit>,
    /// Literals respelled (`tape`), every file's: the file after the last.
    spelled: Vec<u8>,
}

impl<F: rerail_c_pp::files::Files> Tu<'_, F> {
    /// The file at `path` (read the first time, and lexed the first time it is included, `lex`),
    /// or `NONE` where it cannot be read.
    fn file(&mut self, path: String, found_by: Option<u32>, lex: bool) -> Result<u32, String> {
        let k = match self.units.iter().position(|u| u.path == path) {
            Some(k) => k,
            None => {
                let Some(src) = self.io.read(&path) else { return Ok(pp_words::NONE) };
                self.unit(path, src, found_by)
            }
        };
        if lex {
            self.lex(k)?;
        }
        Ok(k as u32)
    }

    /// A `_Pragma`'s text as a file of its own (the lean pass's `pragma_text`), lexed: its tokens
    /// are the pragma's. One unit a text.
    fn pragma(&mut self, text: Vec<u8>) -> Result<u32, String> {
        let k = match self.units.iter().position(|u| u.path == PRAGMA && u.src == text) {
            Some(k) => k,
            None => self.unit(PRAGMA.to_string(), text, None),
        };
        self.lex(k)?;
        Ok(k as u32)
    }

    /// A unit of `src`, not lexed yet.
    fn unit(&mut self, path: String, src: Vec<u8>, found_by: Option<u32>) -> usize {
        let mut lines = Vec::new();
        let mut i = 0;
        while let Some(k) = rerail_simd8::micro::find(&src, i, b'\n') {
            lines.push(k as u32);
            i = k + 1;
        }
        self.units.push(Unit { path, src, lines, found_by, lexed: None, includes: Vec::new(), named: Vec::new(), resolved: false });
        self.units.len() - 1
    }

    /// Unit `k` lexed (once): its token tape, mode (a)'s read, its chains and `#if` depth.
    fn lex(&mut self, k: usize) -> Result<(), String> {
        if self.units[k].lexed.is_none() {
            let (src, path) = (&self.units[k].src, &self.units[k].path);
            let toks = (self.g.tokens)(src, self.cpp).ok_or_else(|| format!("the token grammar refuses {path}"))?;
            let read = (self.g.read)(src).ok_or_else(|| format!("the directives' grammar refuses {path}"))?;
            // The spelled buffer's index is set when the files are laid out (`expand_tu`).
            let tape = tape(src, &toks, k as u32, &mut self.spelled, u32::MAX - 1);
            let chains = chains(&read, &tape).ok_or("a directive's `#` is no token's")?;
            let if_depth = if_depth(&read, src, &tape);
            self.units[k].lexed = Some(Lexed { read, tape, chains, if_depth });
        }
        Ok(())
    }

    /// The file a header name names from file `from`.
    fn find(&mut self, from: usize, name: &[u8], angle: bool, next: bool, lex: bool) -> Result<u32, String> {
        let next_after = next.then_some(self.units[from].found_by);
        let includer = self.units[from].path.clone();
        match self.io.find(name, angle, &includer, next_after) {
            Some((path, found_by)) => self.file(path, found_by, lex),
            None => Ok(pp_words::NONE),
        }
    }

    /// The includes and embeds mode (a) read the names of, of the lexed files not yet resolved
    /// (each may add files).
    fn resolve(&mut self) -> Result<(), String> {
        while let Some(k) = self.units.iter().position(|u| u.lexed.is_some() && !u.resolved) {
            self.units[k].resolved = true;
            let lx = self.units[k].lexed.as_ref().unwrap();
            let facts = crate::Facts::of(&lx.read, &self.units[k].src);
            for inc in facts.includes {
                if inc.form == "macro" || inc.spelled.len() < 2 {
                    continue;
                }
                let name = inc.spelled[1..inc.spelled.len() - 1].to_vec();
                let next = inc.kind == "include_next" && k != 0;
                let to = self.find(k, &name, inc.form == "angle", next, inc.kind != "embed")?;
                let lx = self.units[k].lexed.as_ref().unwrap();
                let at = hash_at(&lx.read, &lx.tape, inc.i).ok_or("an include's `#` is no token's")?;
                self.units[k].includes.push((at, to));
            }
            self.units[k].includes.sort_unstable();
        }
        Ok(())
    }
}

/// A `_Pragma`'s file's path (`Tu::pragma`), as the lean pass names it.
const PRAGMA: &str = "<pragma>";

/// The rounds of the import fixpoint a TU may take: each finds the includes whose names its
/// expansion makes.
const ROUNDS: usize = 16;

/// The TU whose main file is `main`, its files found and read by `io` (the lean pass's
/// `Files`), expanded by the words; `Err` where a grammar or the words refuse it.
pub fn expand_tu<F: rerail_c_pp::files::Files>(main: &str, io: &mut F, cpp: bool, g: &Grammars) -> Result<Vec<Out>, String> {
    expand_tu_with(main, io, cpp, g, &pp_words::run)
}

/// The words' runner: the interpreter (`pp_words::run`) or compiled words.
pub type Runner<'r> = dyn Fn(&rerail_compiler::program::Words, &[File], &pp_words::Env, &Bounds) -> Result<pp_words::Expanded, pp_words::Refused> + 'r;

/// `expand_tu`, the words run by `run`.
pub fn expand_tu_with<F: rerail_c_pp::files::Files>(main: &str, io: &mut F, cpp: bool, g: &Grammars, run: &Runner<'_>) -> Result<Vec<Out>, String> {
    let program = program()?;
    let env = pp_words::Env { cpp, ..pp_words::Env::DEFAULT };
    let mut tu = Tu { io, g, cpp, units: Vec::new(), spelled: Vec::new() };
    if tu.file(main.to_string(), None, true)? == pp_words::NONE {
        return Err(format!("{main} cannot be read"));
    }
    for _ in 0..ROUNDS {
        tu.resolve()?;
        let n = tu.units.len() as u32;
        // The respelled literals' buffer is the file after the last.
        let tapes: Vec<Vec<Tok>> = tu.units.iter().map(|u| u.lexed.as_ref().map_or(Vec::new(), |l| l.tape.iter().map(|t| if t.buf == u32::MAX { Tok { buf: n + 1, ..*t } } else { *t }).collect())).collect();
        let mut files: Vec<File> = tu.units.iter().zip(&tapes).map(|(u, t)| File { path: u.path.as_bytes(), bytes: &u.src, lines: &u.lines, toks: t, chains: u.lexed.as_ref().map_or(&[][..], |l| &l.chains), includes: &u.includes, named: &u.named, if_depth: u.lexed.as_ref().map_or(0, |l| l.if_depth) }).collect();
        files.push(File { bytes: &tu.spelled, ..File::default() });
        let e = run(program, &files, &env, &Bounds::of(&files)).map_err(|r| format!("refused at word {}: {}", r.pc, r.why))?;
        if e.wanted.is_empty() {
            return Ok(e.toks.iter().map(|t| (t.kind, t.flags & (pp_words::BOL | pp_words::SPACE), e.text(&files, t).to_vec())).collect());
        }
        drop(files);
        for (from, mut w) in e.wanted {
            let from = from as usize;
            if tu.units[from].named.iter().any(|n| (&n.name, n.angle, n.next, n.embed, n.pragma) == (&w.name, w.angle, w.next, w.embed, w.pragma)) {
                continue;
            }
            w.to = if w.pragma { tu.pragma(w.name.clone())? } else { tu.find(from, &w.name, w.angle, w.next, !w.embed)? };
            tu.units[from].named.push(w);
        }
    }
    Err("past the import fixpoint's rounds".into())
}

/// A TU's files by path, as `Search` reads them from a map (`dirs` its search path).
pub fn search<'a>(files: &'a [(&'a str, &'a [u8])], dirs: &[(&str, rerail_c_pp::files::DirKind)]) -> rerail_c_pp::files::Search<impl FnMut(&str) -> Option<Vec<u8>> + 'a> {
    let dirs = dirs.iter().map(|(d, k)| (d.to_string(), *k)).collect();
    rerail_c_pp::files::Search::new(dirs, move |p: &str| files.iter().find(|f| f.0 == p).map(|f| f.1.to_vec()))
}

/// `src` (a TU of one file, `main.c`) expanded by the words.
pub fn expand(src: &[u8], cpp: bool) -> Result<Vec<Out>, String> {
    let files = [("main.c", src)];
    expand_tu("main.c", &mut search(&files, &[]), cpp, &compiled_grammars())
}

/// `src` expanded by the lean preprocessor, in the same terms.
pub fn lean(src: &[u8], cpp: bool) -> Vec<Out> {
    let files = [("main.c", src)];
    lean_tu("main.c", &mut search(&files, &[]), cpp)
}

/// The TU whose main file is `main` expanded by the lean preprocessor, in the same terms.
pub fn lean_tu<F: rerail_c_pp::files::Files>(main: &str, io: &mut F, cpp: bool) -> Vec<Out> {
    use rerail_c_lean::pp::{DIGRAPH, P};
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
    let mut sink = Toks(Vec::new());
    let mut pp = Pp::new(Config::new(opts), io, &mut sink);
    pp.run(main);
    drop(pp);
    sink.0
}

#[cfg(test)]
mod tests;
