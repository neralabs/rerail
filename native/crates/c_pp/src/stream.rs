//! Mode (b): a translation unit applied in one forward pass (decision 0327).
//!
//! The pass is clang's. Tokens come from a stack of sources: files (each its lexer and its open
//! conditionals) and token lists (a macro's expansion, an argument read in isolation, an `#if`'s
//! line). A macro is disabled while its expansion is read: an identifier naming it is painted
//! there, never to expand again (C17 §6.10.3.4p2), and the macro is enabled again when the reader
//! passes its expansion's end. A function-like macro's arguments are expanded in isolation before
//! they are substituted, except next to `#` or `##`.
//!
//! The outputs go to a `Sink`: tokens, the files entered, the conditionals taken, the expansions,
//! pragmas and diagnostics.

use crate::directives::Kind;
use crate::expr::{self, Item};
use crate::files::Files;
use crate::macros::*;
use crate::sink::{At, Diag, Sink};
use rerail_c_lean::T;
use rerail_c_lean::pp::{Options, P, Phase2, PpLexer, PpTok};
use std::collections::HashMap;

/// A configuration (`c/env.rr`'s rows): the dialect, the predefined macros, the answers of the
/// `__has_*` family, and the fixed date and time.
#[derive(Clone, Debug)]
pub struct Config {
    pub opts: Options,
    /// `-D` and `-U`, in order: `NAME` or `NAME(params)`, and its body (`None`: an `#undef`).
    pub predefines: Vec<(String, Option<String>)>,
    /// `(builtin, operand)` → value: `("__has_feature", "cxx_rvalue_references")` → 1. A test not
    /// here is 0, but for `__is_identifier`, 1.
    pub features: HashMap<(String, String), i64>,
    pub date: String,
    pub time: String,
    pub timestamp: String,
    /// `true` is 1 in `#if` (C++, C23).
    pub true_is_one: bool,
    pub max_depth: u32,
}

impl Config {
    pub fn new(opts: Options) -> Self {
        Config {
            opts,
            predefines: Vec::new(),
            features: HashMap::new(),
            date: "Jan  1 1970".into(),
            time: "00:00:00".into(),
            timestamp: "Thu Jan  1 00:00:00 1970".into(),
            true_is_one: opts.cpp,
            max_depth: 200,
        }
    }

    /// `-DNAME=BODY` (`=1` where no body is given, as a driver does).
    pub fn define(&mut self, d: &str) -> &mut Self {
        let (n, b) = d.split_once('=').unwrap_or((d, "1"));
        self.predefines.push((n.to_string(), Some(b.to_string())));
        self
    }

    pub fn undef(&mut self, n: &str) -> &mut Self {
        self.predefines.push((n.to_string(), None));
        self
    }
}

/// A file the pass read.
pub struct FileInfo {
    // `p2` borrows `src`'s heap bytes (never moved or changed): declared first, dropped first.
    p2: Phase2<'static>,
    src: Box<[u8]>,
    pub path: String,
    /// The search entry it was found by (`#include_next`'s start).
    pub found_by: Option<u32>,
    /// Its `\n`s' positions in `src` (made when a line is first asked).
    lines: Vec<u32>,
    lines_made: bool,
}

impl FileInfo {
    fn new(path: String, src: Vec<u8>, trigraphs: bool, found_by: Option<u32>) -> Self {
        let src = src.into_boxed_slice();
        // SAFETY: the slice is `src`'s heap allocation, which lives (unmoved, unchanged) as long as
        // this `FileInfo`, and `p2` is dropped before it.
        let bytes: &'static [u8] = unsafe { std::slice::from_raw_parts(src.as_ptr(), src.len()) };
        FileInfo { p2: Phase2::of(bytes, trigraphs), src, path, found_by, lines: Vec::new(), lines_made: false }
    }

    /// The phase-2 text.
    pub fn text(&self) -> &[u8] {
        &self.p2.text
    }

    /// The file's position of phase-2 position `at`.
    pub fn orig(&self, at: u32) -> u32 {
        self.p2.orig(at as usize) as u32
    }

    /// The line (from 1) of phase-2 position `at`.
    fn line(&mut self, at: u32) -> u32 {
        if !self.lines_made {
            let mut i = 0;
            while let Some(k) = rerail_simd8::micro::find(&self.src, i, b'\n') {
                self.lines.push(k as u32);
                i = k + 1;
            }
            self.lines_made = true;
        }
        let o = self.orig(at);
        self.lines.partition_point(|&n| n < o) as u32 + 1
    }
}

/// An open conditional of a file.
#[derive(Clone, Copy)]
struct Cond {
    /// A group of its chain was taken.
    taken: bool,
    seen_else: bool,
    /// Its `#if…`'s position.
    at: u32,
}

/// A source of tokens.
enum Src {
    File {
        file: u32,
        lx: PpLexer<'static>,
        conds: Vec<Cond>,
        /// `#line`: the presumed line of a physical line is `line + delta`.
        delta: i64,
        presumed: Option<String>,
    },
    Toks {
        toks: Vec<Tk>,
        pos: usize,
        /// The macro this list expands, enabled again when the reader passes the list's end.
        def: Option<u32>,
        /// The list is read in isolation: its end is a `STOP` token, and the reader goes no further.
        stop: bool,
    },
}

/// An expansion: its macro and the name token it expanded.
#[derive(Clone, Copy)]
pub struct Expansion {
    pub def: u32,
    pub name: Tk,
    /// The call's last token: its `)`, or the name of an object-like macro (`__LINE__` is its
    /// line, as clang reads it).
    pub end: Tk,
}

/// The symbols the pass knows by name.
struct Syms {
    defined: u32,
    va_args: u32,
    va_opt: u32,
    once: u32,
    push_macro: u32,
    pop_macro: u32,
    poison: u32,
    gcc: u32,
    clang: u32,
    limit: u32,
    prefix: u32,
    suffix: u32,
    if_empty: u32,
    /// C++'s alternative tokens, in `ALT`'s order.
    alt: [u32; 11],
}

/// C++'s alternative tokens (`and` is `&&`), as `#if` reads them.
const ALT: [(&str, T); 11] = [
    ("and", T::AmpAmp),
    ("or", T::PipePipe),
    ("not", T::Bang),
    ("bitand", T::Amp),
    ("bitor", T::Pipe),
    ("xor", T::Caret),
    ("compl", T::Tilde),
    ("not_eq", T::Ne),
    ("and_eq", T::AmpEq),
    ("or_eq", T::PipeEq),
    ("xor_eq", T::CaretEq),
];

/// The preprocessor: its configuration, files, macros and sources.
pub struct Pp<'a> {
    pub cfg: Config,
    io: &'a mut dyn Files,
    sink: &'a mut dyn Sink,
    pub files: Vec<FileInfo>,
    by_path: HashMap<String, u32>,
    /// The pastes' and strings' text (`Tk::buf` 0).
    scratch: Vec<u8>,
    pub syms: Interner,
    /// By symbol: its current definition's index plus one (0: none).
    macros: Vec<u32>,
    pub defs: Vec<Def>,
    /// `#pragma push_macro`'s saved definitions, by symbol.
    pushed: HashMap<u32, Vec<u32>>,
    poisoned: Vec<u32>,
    stack: Vec<Src>,
    pub exps: Vec<Expansion>,
    once: Vec<bool>,
    counter: u64,
    /// Reading an `#if`'s line: `defined` is an operator.
    in_if: bool,
    /// Collecting a call's arguments: a file's end ends them, not the file.
    collecting: u32,
    lines: bool,
    s: Syms,
    /// The main file.
    base: u32,
    /// The `__has_*` tests asked that the configuration does not answer (a profile's to add).
    pub unanswered: Vec<(String, String)>,
}

/// A token's spelling.
fn spell<'x>(files: &'x [FileInfo], scratch: &'x [u8], t: &Tk) -> &'x [u8] {
    let b: &[u8] = if t.buf == 0 { scratch } else { files[t.buf as usize - 1].text() };
    &b[t.start as usize..t.end as usize]
}

impl<'a> Pp<'a> {
    pub fn new(cfg: Config, io: &'a mut dyn Files, sink: &'a mut dyn Sink) -> Self {
        let mut syms = Interner::default();
        let mut g = |w: &str| syms.get(w.as_bytes());
        let s = Syms {
            defined: g("defined"),
            va_args: g("__VA_ARGS__"),
            va_opt: g("__VA_OPT__"),
            once: g("once"),
            push_macro: g("push_macro"),
            pop_macro: g("pop_macro"),
            poison: g("poison"),
            gcc: g("GCC"),
            clang: g("clang"),
            limit: g("limit"),
            prefix: g("prefix"),
            suffix: g("suffix"),
            if_empty: g("if_empty"),
            alt: ALT.map(|(w, _)| g(w)),
        };
        let lines = sink.wants_lines();
        let mut pp = Pp {
            cfg,
            io,
            sink,
            files: Vec::new(),
            by_path: HashMap::new(),
            scratch: Vec::new(),
            syms,
            macros: Vec::new(),
            defs: Vec::new(),
            pushed: HashMap::new(),
            poisoned: Vec::new(),
            stack: Vec::new(),
            exps: Vec::new(),
            once: Vec::new(),
            counter: 0,
            in_if: false,
            collecting: 0,
            lines,
            s,
            base: 0,
            unanswered: Vec::new(),
        };
        for &(name, b, fnlike) in BUILTINS {
            let sym = pp.syms.get(name.as_bytes());
            pp.set_def(sym, Def { name: sym, fnlike, params: Vec::new(), variadic: false, body: Vec::new(), builtin: Some(b), disabled: false, file: 0, at: 0 });
        }
        pp
    }

    /// Symbol `sym`'s definition.
    fn def_of(&self, sym: u32) -> Option<u32> {
        match self.macros.get(sym as usize) {
            Some(&d) if d != 0 => Some(d - 1),
            _ => None,
        }
    }

    fn set_def(&mut self, sym: u32, d: Def) {
        self.defs.push(d);
        let k = self.defs.len() as u32;
        self.set_macro(sym, k);
    }

    fn set_macro(&mut self, sym: u32, slot: u32) {
        if self.macros.len() <= sym as usize {
            self.macros.resize(sym as usize + 1, 0);
        }
        self.macros[sym as usize] = slot;
    }

    /// Whether `name` is a macro now.
    pub fn is_defined(&mut self, name: &str) -> bool {
        let s = self.syms.get(name.as_bytes());
        self.def_of(s).is_some()
    }

    /// The spelling of `t`.
    pub fn text(&self, t: &Tk) -> &[u8] {
        spell(&self.files, &self.scratch, t)
    }

    /// Preprocesses the translation unit whose main file is `path`, its tokens and events to the
    /// sink. `false` where the main file cannot be read.
    pub fn run(&mut self, path: &str) -> bool {
        let Some(src) = self.io.read(path) else { return false };
        // The predefines are a file of their own (`<built-in>`, file 0), read before the main one.
        let mut pre = String::new();
        for (n, b) in &self.cfg.predefines {
            match b {
                Some(b) => pre.push_str(&format!("#define {n} {b}\n")),
                None => pre.push_str(&format!("#undef {n}\n")),
            }
        }
        let builtin = self.add_file("<built-in>".into(), pre.into_bytes(), None);
        let main = self.add_file(path.to_string(), src, None);
        self.base = main;
        self.push_file(main, None);
        self.stack.push(self.file_src(builtin));
        loop {
            let t = self.next();
            if t.kind == P::Eof {
                break;
            }
            let at = if self.lines && t.flags & BOL != 0 && t.exp == 0 && t.buf != 0 { Some(self.at(&t)) } else { None };
            let text = spell(&self.files, &self.scratch, &t);
            self.sink.token(&t, text, at);
        }
        true
    }

    /// Where the file-read token `t` is: its file and presumed line.
    fn at(&mut self, t: &Tk) -> At {
        let file = t.buf - 1;
        let line = self.files[file as usize].line(t.start);
        let delta = self.stack.iter().rev().find_map(|s| match s {
            Src::File { file: f, delta, .. } if *f == file => Some(*delta),
            _ => None,
        });
        At { file, line: (line as i64 + delta.unwrap_or(0)) as u32 }
    }

    fn add_file(&mut self, path: String, src: Vec<u8>, found_by: Option<u32>) -> u32 {
        let id = self.files.len() as u32;
        self.by_path.insert(path.clone(), id);
        self.files.push(FileInfo::new(path, src, self.cfg.opts.trigraphs, found_by));
        self.once.push(false);
        id
    }

    fn file_src(&self, file: u32) -> Src {
        let text = self.files[file as usize].text();
        // SAFETY: a file's phase-2 text lives as long as the pass (files are never removed), and
        // its bytes never move.
        let text: &'static [u8] = unsafe { std::slice::from_raw_parts(text.as_ptr(), text.len()) };
        Src::File { file, lx: PpLexer::new(text, self.cfg.opts), conds: Vec::new(), delta: 0, presumed: None }
    }

    fn push_file(&mut self, file: u32, from: Option<(u32, u32)>) {
        let depth = self.stack.iter().filter(|s| matches!(s, Src::File { .. })).count() as u32;
        let path = self.files[file as usize].path.clone();
        self.sink.enter(file, &path, from, depth);
        self.stack.push(self.file_src(file));
    }

    /// The current file (the top file source).
    fn cur_file(&self) -> u32 {
        self.stack.iter().rev().find_map(|s| if let Src::File { file, .. } = s { Some(*file) } else { None }).unwrap_or(0)
    }

    fn diag(&mut self, file: u32, at: u32, d: Diag, text: &[u8]) {
        let o = self.files.get(file as usize).map_or(at, |f| f.orig(at));
        self.sink.diag(file, o, d, text);
    }

    /// A file's token as the pass carries it.
    fn file_tok(&mut self, t: PpTok, file: u32) -> Tk {
        let sym = if t.kind == P::Ident {
            let f = &self.files[file as usize];
            let w = &f.text()[t.start as usize..t.end as usize];
            self.syms.get(w)
        } else {
            NO_SYM
        };
        Tk { kind: t.kind, punct: t.punct, flags: t.flags & (BOL | SPACE | DIGRAPH), sym, buf: file + 1, start: t.start, end: t.end, exp: 0 }
    }

    /// The top source's file lexer and file (the top must be a file).
    fn flx(&mut self) -> (&mut PpLexer<'static>, u32) {
        match self.stack.last_mut() {
            Some(Src::File { lx, file, .. }) => (lx, *file),
            _ => unreachable!("a directive is read from a file"),
        }
    }

    /// The directive line's next token, `None` at its end.
    fn line_tok(&mut self) -> Option<Tk> {
        let (lx, file) = self.flx();
        let t = lx.next_on_line()?;
        Some(self.file_tok(t, file))
    }

    /// The rest of the directive line's tokens.
    fn line_rest(&mut self) -> Vec<Tk> {
        let mut v = Vec::new();
        while let Some(t) = self.line_tok() {
            v.push(t);
        }
        v
    }

    fn skip_line(&mut self) {
        self.flx().0.skip_line();
    }

    /// The next token, unexpanded: directives are carried out as their `#` is read, a finished
    /// expansion is left (its macro enabled again), a finished file is left.
    fn lex_raw(&mut self) -> Tk {
        loop {
            match self.stack.last_mut() {
                None => return Tk::eof(NO_SYM),
                Some(Src::Toks { toks, pos, def, stop }) => {
                    if *pos < toks.len() {
                        *pos += 1;
                        return toks[*pos - 1];
                    }
                    if *stop {
                        return Tk::eof(STOP);
                    }
                    let d = *def;
                    self.stack.pop();
                    if let Some(d) = d {
                        self.defs[d as usize].disabled = false;
                    }
                }
                Some(Src::File { lx, file, .. }) => {
                    let file = *file;
                    let pt = lx.next();
                    if pt.kind == P::Eof {
                        if self.collecting > 0 {
                            // A call's arguments end with their file (clang's reading).
                            lx.seek(pt.start as usize, true);
                            return Tk::eof(NO_SYM);
                        }
                        self.end_file();
                        continue;
                    }
                    let t = self.file_tok(pt, file);
                    if t.flags & BOL != 0 && t.kind == P::Hash {
                        self.directive(t, file);
                        continue;
                    }
                    if t.kind == P::Ident && !self.poisoned.is_empty() && self.poisoned.contains(&t.sym) {
                        self.diag(file, t.start, Diag::Poisoned, b"");
                    }
                    return t;
                }
            }
        }
    }

    fn end_file(&mut self) {
        let Some(Src::File { file, conds, .. }) = self.stack.pop() else { return };
        for c in conds {
            self.diag(file, c.at, Diag::Unterminated, b"");
        }
        let back = self.cur_file();
        if !self.stack.is_empty() && file != 0 {
            self.sink.leave(back);
        }
    }

    /// The next token, macros expanded.
    pub fn next(&mut self) -> Tk {
        loop {
            let mut t = self.lex_raw();
            if t.kind != P::Ident || t.flags & NOEXP != 0 {
                return t;
            }
            if self.in_if && t.sym == self.s.defined {
                return self.defined_op(t);
            }
            let Some(d) = self.def_of(t.sym) else { return t };
            let def = &self.defs[d as usize];
            if def.disabled {
                t.flags |= NOEXP;
                return t;
            }
            let (fnlike, builtin) = (def.fnlike, def.builtin);
            if fnlike && !self.next_is_lparen() {
                return t;
            }
            if let Some(b) = builtin {
                // `__has_include` and `__has_embed` are operators of `#if` alone (clang reads
                // them as identifiers elsewhere).
                if !self.in_if && matches!(b, Builtin::HasInclude { .. } | Builtin::HasEmbed) {
                    return t;
                }
                let toks = self.builtin(b, t);
                if !toks.is_empty() {
                    self.stack.push(Src::Toks { toks, pos: 0, def: None, stop: false });
                }
                continue;
            }
            let (args, end) = if fnlike {
                match self.collect_args(d, &t) {
                    Ok(a) => a,
                    // A call with the wrong count of arguments is its name alone (clang's
                    // output); one its file ends inside is nothing.
                    Err(true) => return Tk { flags: t.flags | NOEXP, ..t },
                    Err(false) => continue,
                }
            } else {
                (Vec::new(), t)
            };
            self.enter(d, t, end, args);
        }
    }

    /// Whether the next token is `(`, looked at without reading it (nor leaving a finished
    /// expansion).
    fn next_is_lparen(&mut self) -> bool {
        for s in self.stack.iter_mut().rev() {
            match s {
                Src::Toks { toks, pos, stop, .. } => {
                    if *pos < toks.len() {
                        return toks[*pos].is(T::LParen);
                    }
                    if *stop {
                        return false;
                    }
                }
                Src::File { lx, .. } => {
                    let (at, bol) = lx.state();
                    let t = lx.next();
                    lx.seek(at, bol);
                    return t.is(T::LParen);
                }
            }
        }
        false
    }

    /// A call's arguments after its name `name` (the next token is its `(`), and its `)`. `Err`
    /// where the call is not one: `true` for the wrong count of arguments, `false` where its file
    /// ends inside it.
    fn collect_args(&mut self, d: u32, name: &Tk) -> Result<(Vec<Vec<Tk>>, Tk), bool> {
        self.collecting += 1;
        let _lp = self.lex_raw();
        let (n, variadic) = (self.defs[d as usize].params.len(), self.defs[d as usize].variadic);
        let mut args: Vec<Vec<Tk>> = vec![Vec::new()];
        let mut depth = 0u32;
        let mut end = *name;
        let done = loop {
            let mut t = self.lex_raw();
            if t.kind == P::Eof {
                break false;
            }
            if t.is(T::LParen) {
                depth += 1;
            } else if t.is(T::RParen) {
                if depth == 0 {
                    end = t;
                    break true;
                }
                depth -= 1;
            } else if t.is(T::Comma) && depth == 0 && !(variadic && args.len() == n) {
                args.push(Vec::new());
                continue;
            }
            if t.flags & BOL != 0 {
                t.flags = (t.flags & !BOL) | SPACE;
            }
            args.last_mut().unwrap().push(t);
        };
        self.collecting -= 1;
        let file = self.cur_file();
        if !done {
            self.diag(file, name.start, Diag::UnterminatedCall, self.syms.text(name.sym).to_vec().as_slice());
            return Err(false);
        }
        if n == 0 {
            if args.len() == 1 && args[0].is_empty() {
                return Ok((Vec::new(), end));
            }
            self.diag(file, name.start, Diag::ArgCount, b"");
            return Err(true);
        }
        if args.len() + 1 == n && variadic {
            // The variadic arguments left out (C++20, C23, and GNU before them).
            args.push(Vec::new());
        }
        if args.len() != n {
            self.diag(file, name.start, Diag::ArgCount, b"");
            return Err(true);
        }
        Ok((args, end))
    }

    /// The tokens of `toks`, expanded in isolation (an argument before substitution).
    fn expand_list(&mut self, toks: Vec<Tk>) -> Vec<Tk> {
        let depth = self.stack.len();
        self.stack.push(Src::Toks { toks, pos: 0, def: None, stop: true });
        let mut out = Vec::new();
        loop {
            let t = self.next();
            if t.is_stop() {
                break;
            }
            if t.kind == P::Eof {
                // A list read in isolation ends at its `STOP`; nothing below it is read.
                break;
            }
            out.push(t);
        }
        self.stack.truncate(depth);
        out
    }

    /// A token spelled `text`, written to the scratch text.
    fn scratch_tok(&mut self, kind: P, punct: T, text: &[u8], flags: u8, exp: u32) -> Tk {
        let start = self.scratch.len() as u32;
        self.scratch.extend_from_slice(text);
        let sym = if kind == P::Ident { self.syms.get(text) } else { NO_SYM };
        Tk { kind, punct, flags, sym, buf: 0, start, end: self.scratch.len() as u32, exp }
    }

    fn number_tok(&mut self, v: i64, like: &Tk) -> Tk {
        let s = v.to_string();
        self.scratch_tok(P::Number, T::Eof, s.as_bytes(), like.flags & (BOL | SPACE), like.exp)
    }

    /// A string literal of `s`, its `"` and `\` escaped (`__FILE__`'s).
    fn string_tok(&mut self, s: &[u8], like: &Tk) -> Tk {
        let mut q = Vec::with_capacity(s.len() + 2);
        for &c in s {
            if c == b'"' || c == b'\\' {
                q.push(b'\\');
            }
            q.push(c);
        }
        self.quoted_tok(&q, like)
    }

    /// The string literal `"s"` (`s` as `#` made it, escaped already).
    fn quoted_tok(&mut self, s: &[u8], like: &Tk) -> Tk {
        let mut q = Vec::with_capacity(s.len() + 2);
        q.push(b'"');
        q.extend_from_slice(s);
        q.push(b'"');
        self.scratch_tok(P::Str, T::Eof, &q, like.flags & (BOL | SPACE), like.exp)
    }

    /// Macro `d`'s expansion of the call `name` with `args`, entered as the next source.
    fn enter(&mut self, d: u32, name: Tk, end: Tk, args: Vec<Vec<Tk>>) {
        self.exps.push(Expansion { def: d, name, end });
        let id = self.exps.len() as u32;
        {
            let def = &self.defs[d as usize];
            let (call_file, call_at) = self.root_at(&name);
            let (df, da) = (def.file, self.files.get(def.file as usize).map_or(def.at, |f| f.orig(def.at)));
            let nm = self.syms.text(def.name);
            self.sink.expansion(id, nm, df, da, call_file, call_at, name.exp);
        }
        let body = self.defs[d as usize].body.clone();
        let mut pre: Vec<Option<Vec<Tk>>> = vec![None; args.len()];
        let mut out = self.substitute(d, &body, &args, &mut pre, id);
        self.paste_all(&mut out, id);
        out.retain(|t| !t.is_placemarker());
        if let Some(f) = out.first_mut() {
            f.flags = (f.flags & !(BOL | SPACE)) | (name.flags & (BOL | SPACE));
        }
        self.defs[d as usize].disabled = true;
        self.stack.push(Src::Toks { toks: out, pos: 0, def: Some(d), stop: false });
    }

    /// The file position a token was called from: through its expansions to the token read from
    /// a file.
    fn root_at(&self, t: &Tk) -> (u32, u32) {
        let mut t = *t;
        while t.exp != 0 {
            t = self.exps[t.exp as usize - 1].name;
        }
        if t.buf == 0 {
            return (0, 0);
        }
        let f = t.buf - 1;
        (f, self.files[f as usize].orig(t.start))
    }

    /// `body`'s tokens with `args` substituted: `#` stringizes, a parameter next to `##` takes its
    /// argument unexpanded, any other its argument expanded (`pre`, made once); `__VA_OPT__`.
    fn substitute(&mut self, d: u32, body: &[Tk], args: &[Vec<Tk>], pre: &mut [Option<Vec<Tk>>], id: u32) -> Vec<Tk> {
        let def = self.defs[d as usize].clone();
        let mut out: Vec<Tk> = Vec::with_capacity(body.len());
        let va = if def.variadic { Some(def.params.len() - 1) } else { None };
        let mut i = 0;
        while i < body.len() {
            let bt = body[i];
            let next_param = body.get(i + 1).and_then(|n| def.param(n.sym));
            if def.fnlike && bt.kind == P::Hash {
                if let Some(p) = next_param {
                    let s = self.stringize(&args[p]);
                    let t = self.quoted_tok(&s, &bt);
                    out.push(Tk { exp: id, ..t });
                    i += 2;
                    continue;
                }
                if va.is_some() && body.get(i + 1).is_some_and(|n| n.sym == self.s.va_opt) {
                    // `#__VA_OPT__(…)`: the content, substituted and pasted, stringized.
                    let (content, end) = va_opt_content(body, i + 1);
                    let mut c = if self.va_present(args, pre, va.unwrap()) { self.substitute(d, content, args, pre, id) } else { Vec::new() };
                    self.paste_all(&mut c, id);
                    c.retain(|t| !t.is_placemarker());
                    let s = self.stringize(&c);
                    let t = self.quoted_tok(&s, &bt);
                    out.push(Tk { exp: id, ..t });
                    i = end;
                    continue;
                }
            }
            if va.is_some() && bt.sym == self.s.va_opt && bt.kind == P::Ident {
                let (content, end) = va_opt_content(body, i);
                if self.va_present(args, pre, va.unwrap()) {
                    let mut c = self.substitute(d, content, args, pre, id);
                    if let Some(f) = c.first_mut() {
                        f.flags = (f.flags & !SPACE) | (bt.flags & SPACE);
                    }
                    out.extend(c);
                } else {
                    out.push(Tk { flags: PLACEMARKER, ..bt });
                }
                i = end;
                continue;
            }
            if let Some(p) = def.param(bt.sym) {
                let before = i > 0 && body[i - 1].flags & PASTE != 0;
                let after = body.get(i + 1).is_some_and(|n| n.flags & PASTE != 0);
                // GNU's `, ## __VA_ARGS__`: with no variadic arguments the comma goes.
                if before && Some(p) == va && args[p].is_empty() && out.len() >= 2 && out[out.len() - 2].is(T::Comma) && out[out.len() - 1].flags & PASTE != 0 {
                    out.pop();
                    out.pop();
                    i += 1;
                    continue;
                }
                if before && Some(p) == va && out.len() >= 2 && out[out.len() - 2].is(T::Comma) && out[out.len() - 1].flags & PASTE != 0 {
                    // ...and with some, the `##` is no paste.
                    out.pop();
                }
                let toks = if before || after {
                    args[p].clone()
                } else {
                    if pre[p].is_none() {
                        pre[p] = Some(self.expand_list(args[p].clone()));
                    }
                    pre[p].clone().unwrap()
                };
                if toks.is_empty() {
                    out.push(Tk { flags: PLACEMARKER, ..bt });
                } else {
                    let k = out.len();
                    out.extend(toks);
                    out[k].flags = (out[k].flags & !(BOL | SPACE)) | (bt.flags & SPACE);
                }
                i += 1;
                continue;
            }
            out.push(Tk { exp: id, ..bt });
            i += 1;
        }
        out
    }

    /// Whether the variadic argument has tokens once expanded (`__VA_OPT__`'s test).
    fn va_present(&mut self, args: &[Vec<Tk>], pre: &mut [Option<Vec<Tk>>], va: usize) -> bool {
        if args[va].is_empty() {
            return false;
        }
        if pre[va].is_none() {
            pre[va] = Some(self.expand_list(args[va].clone()));
        }
        !pre[va].as_ref().unwrap().is_empty()
    }

    /// The spelling of `toks` as `#` makes it (C17 §6.10.3.2): a blank where there was white space,
    /// `"` and `\` escaped in string and character literals.
    fn stringize(&self, toks: &[Tk]) -> Vec<u8> {
        let mut s = Vec::new();
        for (k, t) in toks.iter().enumerate() {
            if t.is_placemarker() {
                continue;
            }
            if k > 0 && t.flags & (SPACE | BOL) != 0 && !s.is_empty() {
                s.push(b' ');
            }
            let text = self.text(t);
            if matches!(t.kind, P::Str | P::Char | P::RawStr) {
                for &c in text {
                    if c == b'"' || c == b'\\' {
                        s.push(b'\\');
                    }
                    s.push(c);
                }
            } else {
                s.extend_from_slice(text);
            }
        }
        s
    }

    /// The pastes of `out` (`##` tokens of the replacement list), left to right.
    fn paste_all(&mut self, out: &mut Vec<Tk>, id: u32) {
        if !out.iter().any(|t| t.flags & PASTE != 0) {
            return;
        }
        let mut res: Vec<Tk> = Vec::with_capacity(out.len());
        let mut i = 0;
        while i < out.len() {
            let t = out[i];
            if t.flags & PASTE != 0 && !res.is_empty() && i + 1 < out.len() {
                let l = res.pop().unwrap();
                let r = out[i + 1];
                match self.paste(l, r, id) {
                    Ok(m) => res.push(m),
                    Err(()) => {
                        res.push(l);
                        res.push(r);
                    }
                }
                i += 2;
                continue;
            }
            res.push(t);
            i += 1;
        }
        *out = res;
    }

    /// `l ## r`: one token, or `Err` where the two spell no one token.
    fn paste(&mut self, l: Tk, r: Tk, id: u32) -> Result<Tk, ()> {
        if r.is_placemarker() {
            return Ok(l);
        }
        if l.is_placemarker() {
            return Ok(Tk { flags: (r.flags & !(BOL | SPACE)) | (l.flags & (BOL | SPACE)), ..r });
        }
        let mut text = self.text(&l).to_vec();
        text.extend_from_slice(self.text(&r));
        let mut lx = PpLexer::new(&text, self.cfg.opts);
        let t = lx.next();
        let whole = t.start == 0 && t.end as usize == text.len() && t.kind != P::Eof && t.kind != P::Other;
        if !whole {
            let f = self.cur_file();
            self.diag(f, 0, Diag::BadPaste, &text);
            return Err(());
        }
        Ok(self.scratch_tok(t.kind, t.punct, &text, l.flags & (BOL | SPACE), id))
    }

    /// A built-in's tokens for the call (or use) `t`.
    fn builtin(&mut self, b: Builtin, t: Tk) -> Vec<Tk> {
        match b {
            Builtin::Line => {
                // The line of the outermost call's end (clang's: its expansion range's end).
                let mut r = t;
                while r.exp != 0 {
                    r = self.exps[r.exp as usize - 1].end;
                }
                let line = if r.buf != 0 { self.at(&r).line } else { 0 };
                vec![self.number_tok(line as i64, &t)]
            }
            Builtin::File | Builtin::FileName | Builtin::BaseFile => {
                let path = match b {
                    Builtin::BaseFile => self.files[self.base as usize].path.clone(),
                    _ => self.presumed_path(),
                };
                let path = if b == Builtin::FileName { path.rsplit('/').next().unwrap_or("").to_string() } else { path };
                vec![self.string_tok(path.as_bytes(), &t)]
            }
            Builtin::Counter => {
                self.counter += 1;
                vec![self.number_tok(self.counter as i64 - 1, &t)]
            }
            Builtin::IncludeLevel => {
                let n = self.stack.iter().filter(|s| matches!(s, Src::File { .. })).count() as i64 - 1;
                vec![self.number_tok(n, &t)]
            }
            Builtin::Date | Builtin::Time | Builtin::Timestamp => {
                let s = match b {
                    Builtin::Date => self.cfg.date.clone(),
                    Builtin::Time => self.cfg.time.clone(),
                    _ => self.cfg.timestamp.clone(),
                };
                vec![self.string_tok(s.as_bytes(), &t)]
            }
            Builtin::Pragma => {
                let args = self.builtin_operand();
                let args = self.expand_list(args);
                if let [s] = args.as_slice() {
                    if s.kind == P::Str {
                        let text = destringize(self.text(s));
                        let f = self.cur_file();
                        let at = self.root_at(&t).1;
                        self.pragma_text(f, at, text);
                        return Vec::new();
                    }
                }
                let f = self.cur_file();
                self.diag(f, t.start, Diag::Malformed, b"_Pragma");
                Vec::new()
            }
            Builtin::HasInclude { next } => {
                let op = self.builtin_operand();
                let v = match self.header_of(op) {
                    Some((name, angle)) => self.find(&name, angle, next).is_some(),
                    None => false,
                };
                vec![self.number_tok(v as i64, &t)]
            }
            Builtin::HasEmbed => {
                let op = self.builtin_operand();
                let v = match self.header_of(op) {
                    Some((name, angle)) => match self.find(&name, angle, false) {
                        // 1: found, and not empty; 2: found and empty (`__STDC_EMBED_EMPTY__`).
                        Some((path, _)) => match self.io.read(&path) {
                            Some(b) if b.is_empty() => 2,
                            Some(_) => 1,
                            None => 0,
                        },
                        None => 0,
                    },
                    None => 0,
                };
                vec![self.number_tok(v, &t)]
            }
            Builtin::Feature => {
                let op = self.builtin_operand();
                let mut name = Vec::new();
                for x in &op {
                    name.extend_from_slice(self.text(x));
                }
                let key = (String::from_utf8_lossy(self.syms.text(t.sym)).into_owned(), String::from_utf8_lossy(&name).into_owned());
                let v = match self.cfg.features.get(&key) {
                    Some(&v) => v,
                    None => {
                        let v = i64::from(key.0 == "__is_identifier");
                        self.unanswered.push(key);
                        v
                    }
                };
                vec![self.number_tok(v, &t)]
            }
        }
    }

    /// A built-in's operand: the tokens between its `(` and `)`, unexpanded.
    fn builtin_operand(&mut self) -> Vec<Tk> {
        self.collecting += 1;
        let _lp = self.lex_raw();
        let mut v = Vec::new();
        let mut depth = 0;
        loop {
            let t = self.lex_raw();
            if t.kind == P::Eof {
                break;
            }
            if t.is(T::LParen) {
                depth += 1;
            } else if t.is(T::RParen) {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            v.push(t);
        }
        self.collecting -= 1;
        v
    }

    /// The header name tokens make (`"…"`, or `<` … `>` spelled with their blanks), expanded first
    /// where they start with neither: the name and whether it is `<…>`.
    fn header_of(&mut self, toks: Vec<Tk>) -> Option<(Vec<u8>, bool)> {
        let starts = |t: Option<&Tk>| t.is_some_and(|t| t.kind == P::Str || t.kind == P::Header || t.is(T::Lt));
        let toks = if starts(toks.first()) { toks } else { self.expand_list(toks) };
        let first = *toks.first()?;
        if first.kind == P::Str || first.kind == P::Header {
            let s = self.text(&first);
            return Some((s[1..s.len() - 1].to_vec(), s[0] == b'<'));
        }
        if !first.is(T::Lt) {
            return None;
        }
        let mut name = Vec::new();
        for (k, t) in toks[1..].iter().enumerate() {
            if t.is(T::Gt) {
                return Some((name, true));
            }
            if k > 0 && t.flags & SPACE != 0 {
                name.push(b' ');
            }
            name.extend_from_slice(self.text(t));
        }
        None
    }

    /// The file a header name names, searched from the current file.
    fn find(&mut self, name: &[u8], angle: bool, next: bool) -> Option<(String, Option<u32>)> {
        let cur = self.cur_file();
        let info = &self.files[cur as usize];
        let includer = info.path.clone();
        // `#include_next` in the main file is `#include` (clang's reading).
        let next_after = (next && cur != self.base).then_some(info.found_by);
        self.io.find(name, angle, &includer, next_after)
    }

    /// The current file's path, as `#line` left it.
    fn presumed_path(&self) -> String {
        for s in self.stack.iter().rev() {
            if let Src::File { file, presumed, .. } = s {
                return presumed.clone().unwrap_or_else(|| self.files[*file as usize].path.clone());
            }
        }
        String::new()
    }

    /// `defined X` or `defined ( X )` in an `#if`: its value as a number.
    fn defined_op(&mut self, t: Tk) -> Tk {
        let mut n = self.lex_raw();
        let paren = n.is(T::LParen);
        if paren {
            n = self.lex_raw();
        }
        let v = n.kind == P::Ident && self.def_of(n.sym).is_some();
        if n.kind != P::Ident {
            let f = self.cur_file();
            self.diag(f, t.start, Diag::Malformed, b"defined");
        }
        if paren {
            let r = self.lex_raw();
            if !r.is(T::RParen) {
                let f = self.cur_file();
                self.diag(f, t.start, Diag::Malformed, b"defined");
            }
        }
        self.number_tok(v as i64, &t)
    }

    /// The directive whose `#` is `hash`, in `file`; the lexer is left at the next line.
    fn directive(&mut self, hash: Tk, file: u32) {
        let Some(name) = self.line_tok() else { return };
        let kind = match name.kind {
            P::Ident => Kind::of(self.syms.text(name.sym)),
            P::Number => Kind::LineMarker,
            _ => Kind::Unknown,
        };
        match kind {
            Kind::Define => self.define(hash, file),
            Kind::Undef => {
                if let Some(n) = self.line_tok().filter(|t| t.kind == P::Ident) {
                    if self.def_of(n.sym).is_some_and(|d| self.defs[d as usize].builtin.is_some()) {
                        self.diag(file, n.start, Diag::Redefined, b"");
                    }
                    self.set_macro(n.sym, 0);
                } else {
                    self.diag(file, hash.start, Diag::Malformed, b"undef");
                }
            }
            Kind::Include | Kind::IncludeNext | Kind::Import => return self.include(kind, hash, file),
            Kind::Embed => return self.embed(hash, file),
            Kind::If => {
                let v = self.if_expr(file, hash.start);
                return self.open_cond(file, hash.start, v);
            }
            Kind::Ifdef | Kind::Ifndef => {
                let v = match self.line_tok() {
                    Some(n) if n.kind == P::Ident => self.def_of(n.sym).is_some(),
                    _ => {
                        self.diag(file, hash.start, Diag::Malformed, b"");
                        false
                    }
                };
                self.skip_line();
                return self.open_cond(file, hash.start, v == (kind == Kind::Ifdef));
            }
            Kind::Elif | Kind::Elifdef | Kind::Elifndef | Kind::Else => {
                // In a taken group: the chain was taken, so the rest of it is skipped.
                let top = match self.stack.last_mut() {
                    Some(Src::File { conds, .. }) => conds.last_mut(),
                    _ => None,
                };
                let Some(top) = top else {
                    self.diag(file, hash.start, Diag::Unmatched, b"");
                    self.skip_line();
                    return;
                };
                let after_else = top.seen_else;
                if kind == Kind::Else {
                    top.seen_else = true;
                }
                if after_else {
                    self.diag(file, hash.start, Diag::AfterElse, b"");
                }
                self.sink.cond(file, self.files[file as usize].orig(hash.start), false);
                self.skip_line();
                return self.skip_group(file);
            }
            Kind::Endif => {
                let popped = match self.stack.last_mut() {
                    Some(Src::File { conds, .. }) => conds.pop(),
                    _ => None,
                };
                if popped.is_none() {
                    self.diag(file, hash.start, Diag::Unmatched, b"");
                }
            }
            Kind::Line | Kind::LineMarker => {
                let rest = self.line_rest();
                let mut toks = if kind == Kind::LineMarker { vec![name] } else { Vec::new() };
                toks.extend(self.expand_list(rest));
                self.line_directive(file, hash.start, &toks);
            }
            Kind::Error | Kind::Warning => {
                let (lx, _) = self.flx();
                let (s, e) = lx.rest_of_line();
                let text = self.files[file as usize].text()[s..e].to_vec();
                self.diag(file, hash.start, if kind == Kind::Error { Diag::Error } else { Diag::Warning }, &text);
                return;
            }
            Kind::Pragma => {
                let (lx, _) = self.flx();
                let (s, e) = lx.rest_of_line();
                let text = self.files[file as usize].text()[s..e].to_vec();
                let at = self.files[file as usize].orig(hash.start);
                self.pragma_text(file, at, text);
                return;
            }
            // `#ident`, `#sccs` (their strings are the object file's), GNU's `#assert`.
            Kind::Ident | Kind::Sccs | Kind::Assert | Kind::Unassert => {}
            Kind::Null => {}
            Kind::Unknown => self.diag(file, hash.start, Diag::UnknownDirective, b""),
        }
        self.skip_line();
    }

    /// `#define`.
    fn define(&mut self, hash: Tk, file: u32) {
        let Some(n) = self.line_tok().filter(|t| t.kind == P::Ident) else {
            self.diag(file, hash.start, Diag::Malformed, b"define");
            return;
        };
        let mut def = Def { name: n.sym, fnlike: false, params: Vec::new(), variadic: false, body: Vec::new(), builtin: None, disabled: false, file, at: hash.start };
        let mut first = self.line_tok();
        if first.is_some_and(|t| t.is(T::LParen) && t.flags & SPACE == 0) {
            def.fnlike = true;
            let mut want = true;
            loop {
                let Some(t) = self.line_tok() else {
                    self.diag(file, hash.start, Diag::Malformed, b"define");
                    return;
                };
                if t.is(T::RParen) && (!want || def.params.is_empty()) && !(want && def.variadic) {
                    break;
                }
                if def.variadic {
                    self.diag(file, hash.start, Diag::Malformed, b"define");
                    return;
                }
                if want && t.kind == P::Ident {
                    def.params.push(t.sym);
                    want = false;
                } else if want && t.is(T::DotDotDot) {
                    def.params.push(self.s.va_args);
                    def.variadic = true;
                    want = false;
                } else if !want && t.is(T::DotDotDot) && !def.params.is_empty() {
                    // GNU's `name...`.
                    def.variadic = true;
                } else if !want && t.is(T::Comma) {
                    want = true;
                } else {
                    self.diag(file, hash.start, Diag::Malformed, b"define");
                    return;
                }
            }
            first = self.line_tok();
        }
        let mut body = Vec::new();
        if let Some(mut f) = first {
            f.flags &= !SPACE;
            body.push(f);
        }
        body.extend(self.line_rest());
        let last = body.len().saturating_sub(1);
        for (k, t) in body.iter_mut().enumerate() {
            if t.kind == P::HashHash && k > 0 && k < last {
                t.flags |= PASTE;
            }
        }
        def.body = body;
        if let Some(old) = self.def_of(n.sym) {
            if !self.same_def(&self.defs[old as usize], &def) {
                self.diag(file, hash.start, Diag::Redefined, self.syms.text(n.sym).to_vec().as_slice());
            }
        }
        self.set_def(n.sym, def);
    }

    /// Whether two definitions are the same (C17 §6.10.3p2): parameters, and replacement lists
    /// spelled alike with white space alike.
    fn same_def(&self, a: &Def, b: &Def) -> bool {
        a.builtin.is_none()
            && a.fnlike == b.fnlike
            && a.params == b.params
            && a.variadic == b.variadic
            && a.body.len() == b.body.len()
            && a.body.iter().zip(&b.body).all(|(x, y)| self.text(x) == self.text(y) && (x.flags & SPACE == 0) == (y.flags & SPACE == 0))
    }

    /// An `#if`'s or `#elif`'s line: its value (false where it is no expression).
    fn if_expr(&mut self, file: u32, at: u32) -> bool {
        let rest = self.line_rest();
        self.in_if = true;
        let toks = self.expand_list(rest);
        self.in_if = false;
        let alt = self.s.alt;
        let cpp = self.cfg.opts.cpp;
        let items: Vec<Item> = toks
            .iter()
            .map(|t| {
                let text = spell(&self.files, &self.scratch, t);
                let k = if cpp && t.kind == P::Ident { alt.iter().position(|&a| a == t.sym) } else { None };
                match k {
                    Some(k) => Item { kind: P::Punct, punct: ALT[k].1, text },
                    None => Item { kind: t.kind, punct: t.punct, text },
                }
            })
            .collect();
        match expr::eval(&items, self.cfg.true_is_one) {
            Ok(v) => v.is_true(),
            Err(e) => {
                self.diag(file, at, Diag::BadExpr(e), b"");
                false
            }
        }
    }

    /// A conditional opened at `at` with value `v`: its group taken, or skipped.
    fn open_cond(&mut self, file: u32, at: u32, v: bool) {
        if let Some(Src::File { conds, .. }) = self.stack.last_mut() {
            conds.push(Cond { taken: v, seen_else: false, at });
        }
        self.sink.cond(file, self.files[file as usize].orig(at), v);
        if !v {
            self.skip_group(file);
        }
    }

    /// Passes a skipped group, and the groups after it, to the one taken or the chain's `#endif`.
    fn skip_group(&mut self, file: u32) {
        let start = self.flx().0.pos as u32;
        let mut depth = 0u32;
        loop {
            let (lx, _) = self.flx();
            let line_start = lx.state();
            let t = lx.next();
            if t.kind == P::Eof {
                lx.seek(line_start.0, line_start.1);
                self.skipped(file, start, t.start);
                return;
            }
            if t.flags & rerail_c_lean::pp::BOL == 0 || t.kind != P::Hash {
                lx.skip_line();
                continue;
            }
            let Some(n) = lx.next_on_line() else { continue };
            if n.kind != P::Ident {
                lx.skip_line();
                continue;
            }
            let kind = Kind::of(&self.files[file as usize].text()[n.start as usize..n.end as usize]);
            let (lx, _) = self.flx();
            match kind {
                Kind::If | Kind::Ifdef | Kind::Ifndef => {
                    depth += 1;
                    lx.skip_line();
                }
                Kind::Endif if depth > 0 => {
                    depth -= 1;
                    lx.skip_line();
                }
                Kind::Endif => {
                    lx.skip_line();
                    self.skipped(file, start, t.start);
                    if let Some(Src::File { conds, .. }) = self.stack.last_mut() {
                        conds.pop();
                    }
                    return;
                }
                Kind::Elif | Kind::Elifdef | Kind::Elifndef | Kind::Else if depth == 0 => {
                    let (taken, seen_else) = match self.stack.last() {
                        Some(Src::File { conds, .. }) => conds.last().map_or((true, false), |c| (c.taken, c.seen_else)),
                        _ => (true, false),
                    };
                    if seen_else {
                        self.diag(file, t.start, Diag::AfterElse, b"");
                    }
                    let v = if taken || seen_else {
                        self.skip_line();
                        false
                    } else {
                        match kind {
                            Kind::Else => {
                                self.skip_line();
                                true
                            }
                            Kind::Elif => self.if_expr(file, t.start),
                            _ => {
                                let d = match self.line_tok() {
                                    Some(n) if n.kind == P::Ident => self.def_of(n.sym).is_some(),
                                    _ => false,
                                };
                                self.skip_line();
                                d == (kind == Kind::Elifdef)
                            }
                        }
                    };
                    if let Some(Src::File { conds, .. }) = self.stack.last_mut() {
                        if let Some(c) = conds.last_mut() {
                            c.seen_else |= kind == Kind::Else;
                            c.taken |= v;
                        }
                    }
                    self.sink.cond(file, self.files[file as usize].orig(t.start), v);
                    if v {
                        self.skipped(file, start, t.start);
                        return;
                    }
                }
                _ => lx.skip_line(),
            }
        }
    }

    fn skipped(&mut self, file: u32, start: u32, end: u32) {
        let f = &self.files[file as usize];
        let (s, e) = (f.orig(start), f.orig(end));
        self.sink.skipped(file, s, e);
    }

    /// `#include`, `#include_next`, `#import`.
    fn include(&mut self, kind: Kind, hash: Tk, file: u32) {
        let at = self.files[file as usize].orig(hash.start);
        let header = {
            let (lx, f) = self.flx();
            match lx.header_name() {
                Some(h) => {
                    let t = self.file_tok(h, f);
                    Some(vec![t])
                }
                None => None,
            }
        };
        let toks = match header {
            Some(h) => h,
            None => self.line_rest(),
        };
        self.skip_line();
        let Some((name, angle)) = self.header_of(toks) else {
            self.diag(file, hash.start, Diag::Malformed, b"include");
            return;
        };
        let depth = self.stack.iter().filter(|s| matches!(s, Src::File { .. })).count() as u32;
        if depth > self.cfg.max_depth {
            self.diag(file, hash.start, Diag::TooDeep, &name);
            return;
        }
        let Some((path, found_by)) = self.find(&name, angle, kind == Kind::IncludeNext) else {
            self.sink.missing(file, at, &name, angle);
            self.diag(file, hash.start, Diag::NotFound, &name);
            return;
        };
        let id = match self.by_path.get(&path) {
            Some(&id) => id,
            None => {
                let Some(src) = self.io.read(&path) else {
                    self.sink.missing(file, at, &name, angle);
                    self.diag(file, hash.start, Diag::NotFound, &name);
                    return;
                };
                self.add_file(path, src, found_by)
            }
        };
        if self.once[id as usize] {
            return;
        }
        if kind == Kind::Import {
            self.once[id as usize] = true;
        }
        self.push_file(id, Some((file, at)));
    }

    /// `#embed`: the file's bytes as a list of integers, with its `prefix`, `suffix` and
    /// `if_empty` tokens, to `limit` bytes.
    fn embed(&mut self, hash: Tk, file: u32) {
        let header = {
            let (lx, f) = self.flx();
            lx.header_name().map(|h| (h, f))
        };
        let mut rest = self.line_rest();
        self.skip_line();
        let toks = match header {
            Some((h, f)) => vec![self.file_tok(h, f)],
            None => {
                // The name's tokens, then the parameters: the name ends with its `"…"` or `>`.
                let k = if rest.first().is_some_and(|t| t.is(T::Lt)) { rest.iter().position(|t| t.is(T::Gt)).map_or(rest.len(), |k| k + 1) } else { 1.min(rest.len()) };
                rest.drain(..k).collect()
            }
        };
        let Some((name, angle)) = self.header_of(toks) else {
            self.diag(file, hash.start, Diag::Malformed, b"embed");
            return;
        };
        let rest = self.expand_list(rest);
        let (mut limit, mut prefix, mut suffix, mut if_empty) = (None, Vec::new(), Vec::new(), Vec::new());
        let mut i = 0;
        while i < rest.len() {
            let p = rest[i];
            let mut arg = Vec::new();
            let mut j = i + 1;
            if rest.get(j).is_some_and(|t| t.is(T::LParen)) {
                let mut depth = 0;
                j += 1;
                while j < rest.len() {
                    if rest[j].is(T::LParen) {
                        depth += 1;
                    } else if rest[j].is(T::RParen) {
                        if depth == 0 {
                            break;
                        }
                        depth -= 1;
                    }
                    arg.push(rest[j]);
                    j += 1;
                }
                j += 1;
            }
            if p.sym == self.s.limit {
                let items: Vec<Item> = arg.iter().map(|t| Item { kind: t.kind, punct: t.punct, text: spell(&self.files, &self.scratch, t) }).collect();
                limit = expr::eval(&items, self.cfg.true_is_one).ok().map(|v| v.bits as usize);
            } else if p.sym == self.s.prefix {
                prefix = arg;
            } else if p.sym == self.s.suffix {
                suffix = arg;
            } else if p.sym == self.s.if_empty {
                if_empty = arg;
            }
            i = j;
        }
        let Some((path, _)) = self.find(&name, angle, false) else {
            let at = self.files[file as usize].orig(hash.start);
            self.sink.missing(file, at, &name, angle);
            self.diag(file, hash.start, Diag::NotFound, &name);
            return;
        };
        let bytes = self.io.read(&path).unwrap_or_default();
        let bytes = &bytes[..limit.unwrap_or(bytes.len()).min(bytes.len())];
        let mut out = Vec::new();
        if bytes.is_empty() {
            out = if_empty;
        } else {
            out.extend(prefix);
            for (k, &b) in bytes.iter().enumerate() {
                if k > 0 {
                    let c = self.scratch_tok(P::Punct, T::Comma, b",", 0, 0);
                    out.push(c);
                }
                let n = self.number_tok(b as i64, &hash);
                out.push(Tk { flags: 0, ..n });
            }
            out.extend(suffix);
        }
        if let Some(f) = out.first_mut() {
            f.flags |= BOL;
        }
        if !out.is_empty() {
            self.stack.push(Src::Toks { toks: out, pos: 0, def: None, stop: false });
        }
    }

    /// `#line N "file"` (expanded) and GNU's `# N "file" flags`.
    fn line_directive(&mut self, file: u32, at: u32, toks: &[Tk]) {
        let n = toks.first().filter(|t| t.kind == P::Number).and_then(|t| expr::number(self.text(t)));
        let Some(n) = n else {
            self.diag(file, at, Diag::Malformed, b"line");
            return;
        };
        let path = toks.get(1).filter(|t| t.kind == P::Str).map(|t| String::from_utf8_lossy(&destringize(self.text(t))).into_owned());
        // The next line is line `n`.
        let next_line = {
            let (lx, _) = self.flx();
            lx.pos as u32
        };
        let phys = self.files[file as usize].line(next_line) as i64 + 1;
        if let Some(Src::File { delta, presumed, .. }) = self.stack.last_mut() {
            *delta = n.bits as i64 - phys;
            if path.is_some() {
                *presumed = path;
            }
        }
    }

    /// A pragma's text: `once`, `push_macro`, `pop_macro` and `poison` are carried out here; every
    /// pragma goes to the sink.
    fn pragma_text(&mut self, file: u32, at: u32, text: Vec<u8>) {
        // The pragma's tokens, from a file of its own text.
        let pf = self.add_file(format!("<pragma {}>", self.files[file as usize].path), text.clone(), None);
        let src = self.file_src(pf);
        let Src::File { mut lx, .. } = src else { unreachable!() };
        let mut toks = Vec::new();
        loop {
            let t = lx.next();
            if t.kind == P::Eof {
                break;
            }
            toks.push(self.file_tok(t, pf));
        }
        let word = |k: usize| toks.get(k).map_or(NO_SYM, |t| t.sym);
        let (w0, w1) = (word(0), word(1));
        let s = Syms4 { once: self.s.once, push: self.s.push_macro, pop: self.s.pop_macro, gcc: self.s.gcc, clang: self.s.clang, poison: self.s.poison };
        let string_arg = |toks: &[Tk], k: usize| toks.get(k + 1).filter(|t| t.is(T::LParen)).and(toks.get(k + 2)).filter(|t| t.kind == P::Str).copied();
        if w0 == s.once {
            self.once[file as usize] = true;
            return;
        }
        if w0 == s.push || w0 == s.pop {
            if let Some(st) = string_arg(&toks, 0) {
                let name = destringize(self.text(&st));
                let sym = self.syms.get(&name);
                if w0 == s.push {
                    let cur = self.macros.get(sym as usize).copied().unwrap_or(0);
                    self.pushed.entry(sym).or_default().push(cur);
                } else if let Some(v) = self.pushed.get_mut(&sym).and_then(|v| v.pop()) {
                    self.set_macro(sym, v);
                }
            }
        } else if (w0 == s.gcc || w0 == s.clang) && w1 == s.poison {
            for t in &toks[2..] {
                if t.kind == P::Ident {
                    self.poisoned.push(t.sym);
                }
            }
        }
        self.sink.pragma(file, at, &text);
    }
}

/// The pragma words `pragma_text` acts on.
struct Syms4 {
    once: u32,
    push: u32,
    pop: u32,
    gcc: u32,
    clang: u32,
    poison: u32,
}

/// `__VA_OPT__ ( content )` at `i`: the content, and the index past its `)`.
fn va_opt_content(body: &[Tk], i: usize) -> (&[Tk], usize) {
    if !body.get(i + 1).is_some_and(|t| t.is(T::LParen)) {
        return (&[], i + 1);
    }
    let mut depth = 0;
    let mut j = i + 2;
    while j < body.len() {
        if body[j].is(T::LParen) {
            depth += 1;
        } else if body[j].is(T::RParen) {
            if depth == 0 {
                return (&body[i + 2..j], j + 1);
            }
            depth -= 1;
        }
        j += 1;
    }
    (&body[i + 2..], body.len())
}

/// A string literal's content (`_Pragma`'s destringizing, C17 §6.10.9): its prefix and quotes
/// taken off, `\"` and `\\` made `"` and `\`.
fn destringize(s: &[u8]) -> Vec<u8> {
    let q = s.iter().position(|&c| c == b'"').unwrap_or(0);
    let body = &s[(q + 1).min(s.len())..s.len().saturating_sub(1).max(q + 1)];
    let mut out = Vec::with_capacity(body.len());
    let mut i = 0;
    while i < body.len() {
        if body[i] == b'\\' && matches!(body.get(i + 1), Some(b'"' | b'\\')) {
            out.push(body[i + 1]);
            i += 2;
        } else {
            out.push(body[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests;
