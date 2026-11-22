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

/// A file's tokens (C++'s where told), or `None` where its lexer refuses it.
pub type Lexer = dyn Fn(&[u8], bool) -> Option<Vec<tokens::Tok>>;

/// How a TU's files are lexed and their directives read: by the compiled grammars, or (tests) by
/// their interpreter.
pub struct Grammars<'a> {
    pub tokens: &'a Lexer,
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
            self.units[k].lexed = Some(Lexed { read, tape, chains });
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
/// `Files`), expanded by the words on the CPU path (`native`); `Err` where a grammar or the
/// words refuse it.
pub fn expand_tu<F: rerail_c_pp::files::Files>(main: &str, io: &mut F, cpp: bool, g: &Grammars) -> Result<Vec<Out>, String> {
    expand_tu_with(main, io, cpp, g, &native)
}

/// **Deprecated** (decision 0550): `rerail-pp-aot` is Rust from the JIT's IR; its use goes with it.
/// Whether the ahead-of-time build (`rerail_pp_aot::BUILD`) is the words this process has: made
/// from the same text (its `words` fingerprint, `EXPAND`'s) for the harness's interface now
/// (`native_cpu_pp::ABI`), checked once.
pub fn aot_build() -> Option<rerail_pp_aot::Build> {
    static SAME: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let b = rerail_pp_aot::BUILD?;
    SAME.get_or_init(|| b.words == rerail_cpu_stream::clif_rust::fingerprint(EXPAND) && b.abi == rerail_cpu_pp::native_cpu_pp::ABI).then_some(b)
}

/// The words' CPU path: rustc's build of their compiled code (`rerail-pp-aot`, decision 0348 and
/// t44: 0.44× the interpreter's cycles on the corpus), else, without a build of these words,
/// Cranelift's code of the same IR, compiled once a thread. Words other than the process's
/// (`program`) run on the interpreter.
pub fn native(program: &Words, files: &[File], env: &pp_words::Env, bounds: &Bounds) -> Result<pp_words::Expanded, pp_words::Refused> {
    let ours = self::program().is_ok_and(|p| std::ptr::eq(p, program));
    if !ours {
        return pp_words::run(program, files, env, bounds);
    }
    if let Some(b) = aot_build() {
        // SAFETY: `b.run` is `rust_source`'s `pp_run` for `program`: the build was made from
        // `EXPAND`'s text (`aot_build`), and `program` is `EXPAND` assembled (`program()`).
        return unsafe { rerail_cpu_pp::native_cpu_pp::run_aot(b.run, program, files, env, bounds) };
    }
    thread_local! {
        static JIT: std::cell::OnceCell<Option<rerail_cpu_pp::native_cpu_pp::CompiledPp>> = const { std::cell::OnceCell::new() };
    }
    JIT.with(|j| match j.get_or_init(|| rerail_cpu_pp::native_cpu_pp::compile(program).ok()) {
        Some(c) => c.run(program, files, env, bounds),
        None => pp_words::run(program, files, env, bounds),
    })
}

/// The words' compiled code as UKL (`native_cpu_pp::ukl_source`, decisions 0348 stage 5, 0440 step D), made once a
/// process: the device runs the same IR as the CPU.
pub fn ukl() -> Result<&'static str, String> {
    static UKL: std::sync::OnceLock<Result<String, String>> = std::sync::OnceLock::new();
    UKL.get_or_init(|| rerail_cpu_pp::native_cpu_pp::ukl_source(program()?)).as_deref().map_err(Clone::clone)
}

/// The words on the process's device, a TU a batch (`rerail_metal::pp_on`).
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn on_device(program: &Words, files: &[File], env: &pp_words::Env, bounds: &Bounds) -> Result<pp_words::Expanded, pp_words::Refused> {
    on_device_batch(&[(program, files, env, bounds)]).pop().expect("one result")
}

/// The words on the device over a batch of TUs' rounds, a TU a thread (0408, L3): the batch
/// runner of `expand_tus_with`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn on_device_batch(jobs: &[Job<'_>]) -> Vec<Result<pp_words::Expanded, pp_words::Refused>> {
    let refused = |why: &'static str| jobs.iter().map(|_| Err(pp_words::Refused { pc: 0, why })).collect();
    let Ok(source) = ukl() else { return refused("the words do not translate to UKL") };
    match rerail_metal::pp_on::run_batch(source, jobs, jobs.len()) {
        Ok(ran) => ran.results,
        Err(_) => refused("the device refused the batch"),
    }
}

/// The words' runner: the interpreter (`pp_words::run`) or compiled words.
pub type Runner<'r> = dyn Fn(&rerail_compiler::program::Words, &[File], &pp_words::Env, &Bounds) -> Result<pp_words::Expanded, pp_words::Refused> + 'r;

/// `expand_tu`, the words run by `run`: a batch of one (`expand_tus_with`).
pub fn expand_tu_with<F: rerail_c_pp::files::Files>(main: &str, io: &mut F, cpp: bool, g: &Grammars, run: &Runner<'_>) -> Result<Vec<Out>, String> {
    let batch = |jobs: &[Job<'_>]| jobs.iter().map(|&(w, f, e, b)| run(w, f, e, b)).collect();
    expand_tus_with(&mut [(main, io, cpp)], g, &batch).pop().expect("one TU")
}

/// A round of one TU: the words, its files, its environment and its bounds.
pub type Job<'a> = (&'a Words, &'a [File<'a>], &'a pp_words::Env<'a>, &'a Bounds);

/// The words' runner over a batch of TUs' rounds (0408, L3): each job's result, in order.
pub type BatchRunner<'r> = dyn Fn(&[Job<'_>]) -> Vec<Result<pp_words::Expanded, pp_words::Refused>> + 'r;

/// Many TUs expanded together, each `(main, io, cpp)`: each round runs every TU not yet done in
/// one batch (`run`), and between rounds the host reads the files the TUs asked for. Each TU's
/// result is `expand_tu`'s.
pub fn expand_tus_with<F: rerail_c_pp::files::Files>(tus: &mut [(&str, &mut F, bool)], g: &Grammars, run: &BatchRunner<'_>) -> Vec<Result<Vec<Out>, String>> {
    let program = match program() {
        Ok(p) => p,
        Err(e) => return tus.iter().map(|_| Err(e.to_string())).collect(),
    };
    let n = tus.len();
    let mut out: Vec<Option<Result<Vec<Out>, String>>> = (0..n).map(|_| None).collect();
    let envs: Vec<pp_words::Env> = tus.iter().map(|t| pp_words::Env { cpp: t.2, ..pp_words::Env::DEFAULT }).collect();
    let mains: Vec<String> = tus.iter().map(|t| t.0.to_string()).collect();
    let mut states: Vec<Tu<'_, F>> = tus.iter_mut().map(|t| Tu { io: &mut *t.1, g, cpp: t.2, units: Vec::new(), spelled: Vec::new() }).collect();
    for (i, tu) in states.iter_mut().enumerate() {
        match tu.file(mains[i].clone(), None, true) {
            Ok(pp_words::NONE) => out[i] = Some(Err(format!("{} cannot be read", mains[i]))),
            Ok(_) => {}
            Err(e) => out[i] = Some(Err(e)),
        }
    }
    for _ in 0..ROUNDS {
        for (i, tu) in states.iter_mut().enumerate() {
            if out[i].is_none()
                && let Err(e) = tu.resolve()
            {
                out[i] = Some(Err(e));
            }
        }
        let active: Vec<usize> = (0..n).filter(|&i| out[i].is_none()).collect();
        if active.is_empty() {
            break;
        }
        let mut asked = Vec::new();
        {
            // The respelled literals' buffer is the file after the last.
            let tapes: Vec<Vec<Vec<Tok>>> = active
                .iter()
                .map(|&i| {
                    let m = states[i].units.len() as u32;
                    states[i].units.iter().map(|u| u.lexed.as_ref().map_or(Vec::new(), |l| l.tape.iter().map(|t| if t.buf == u32::MAX { Tok { buf: m + 1, ..*t } } else { *t }).collect())).collect()
                })
                .collect();
            let files: Vec<Vec<File>> = active
                .iter()
                .zip(&tapes)
                .map(|(&i, tapes)| {
                    let tu = &states[i];
                    let mut files: Vec<File> = tu.units.iter().zip(tapes).map(|(u, t)| File { path: u.path.as_bytes(), bytes: &u.src, lines: &u.lines, toks: t, chains: u.lexed.as_ref().map_or(&[][..], |l| &l.chains), includes: &u.includes, named: &u.named }).collect();
                    files.push(File { bytes: &tu.spelled, ..File::default() });
                    files
                })
                .collect();
            let bounds: Vec<Bounds> = files.iter().map(|f| Bounds::of(f)).collect();
            let jobs: Vec<Job<'_>> = active.iter().enumerate().map(|(k, &i)| (program, &files[k][..], &envs[i], &bounds[k])).collect();
            for (k, r) in run(&jobs).into_iter().enumerate() {
                let i = active[k];
                match r {
                    Err(r) => out[i] = Some(Err(format!("refused at word {}: {}", r.pc, r.why))),
                    Ok(e) if e.wanted.is_empty() => out[i] = Some(Ok(e.toks.iter().map(|t| (t.kind, t.flags & (pp_words::BOL | pp_words::SPACE), e.text(&files[k], t).to_vec())).collect())),
                    Ok(e) => asked.push((i, e.wanted)),
                }
            }
        }
        for (i, wanted) in asked {
            let tu = &mut states[i];
            for (from, mut w) in wanted {
                let from = from as usize;
                if tu.units[from].named.iter().any(|n| (&n.name, n.angle, n.next, n.embed, n.pragma) == (&w.name, w.angle, w.next, w.embed, w.pragma)) {
                    continue;
                }
                let to = if w.pragma { tu.pragma(w.name.clone()) } else { tu.find(from, &w.name, w.angle, w.next, !w.embed) };
                match to {
                    Ok(to) => {
                        w.to = to;
                        tu.units[from].named.push(w);
                    }
                    Err(e) => {
                        out[i] = Some(Err(e));
                        break;
                    }
                }
            }
        }
    }
    out.into_iter().map(|o| o.unwrap_or_else(|| Err("past the import fixpoint's rounds".into()))).collect()
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
