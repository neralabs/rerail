//! The preprocessing words (decision 0348): tier 2's words over a re-entrant token source, and
//! their interpreter, which is their meaning.
//!
//! A translation unit's expansion is a program of these words (`rerail-c-directives`'
//! `expand.rir`). The machine they act on:
//! - **sources**, a stack of cursors: a file's token tape (read-only), or a range of the TU's token
//!   arena. An ended source is left as `READ` passes it; one that carries a macro enables it again
//!   (clang's painting). A stop source's end reads as `EOF` with `STOP` and is never passed: an
//!   argument expanded before substitution ends there.
//! - **the arena**, append-only: arguments, substituted bodies, and the tokens of `##` and `#`,
//!   their spellings in the scratch bytes (`buf` 0).
//! - **the macro table**, by open addressing on a word-at-a-time key of the name, then its bytes.
//!   A definition's body is a range of its file's tape, never copied, with a mark per token (its
//!   parameter, a `__VA_OPT__`, a paste).
//! - **frames**, one per call being made: its arguments (arena ranges), and each argument's
//!   expansion once it is made (`PRE`), in the order the substitution first asks for it.
//! - **levels**, one per argument being expanded: the tokens it gave so far (`KEEP`), moved to the
//!   arena when it ends, so an argument's expansion is one range however deep its calls nest.
//! - **the conditional stack** of each file source: a group's chain taken, its `#else` seen. A group
//!   not taken is passed by mode (a)'s chains (`File::chains`), never read.
//! - the registers: `T`, the token read last, `D`, the macro found last, and whether an `#if`'s
//!   line is being expanded (`defined` is an operator there).
//!
//! Every store has a bound from the work (`Bounds::of`); past one the TU is refused, to the lean
//! pass. Expansion's worst case is exponential, and a refusal is the answer to it.

use crate::Error;
use crate::program::*;
use std::collections::HashMap;

/// Whether a word is one of the preprocessing words.
pub fn is_pp(opcode: u8) -> bool {
    (READ..=LINEDIR).contains(&opcode)
}

// A token's kind.
pub const IDENT: u8 = 0;
pub const NUMBER: u8 = 1;
/// A string or character literal, its prefix and (C++) suffix included.
pub const LITERAL: u8 = 2;
pub const PUNCT: u8 = 3;
/// `<:` `:>` `<%` `%>`.
pub const DIGRAPH: u8 = 4;
pub const HASH: u8 = 5;
pub const HASHHASH: u8 = 6;
pub const OTHER: u8 = 7;
/// The input's end, or (with `STOP`) a stop source's.
pub const EOF: u8 = 8;
pub const KINDS: u8 = 9;

// A token's flags.
/// A line's first token.
pub const BOL: u8 = 1;
/// White space before it.
pub const SPACE: u8 = 2;
/// Painted: it named a macro being expanded where it was read, and is never expanded.
pub const NOEXP: u8 = 4;
/// A `##` of a replacement list, the paste operator (an argument's `##` is none).
pub const PASTE: u8 = 8;
/// An empty argument, until the pastes are done.
pub const PLACEMARKER: u8 = 16;
/// An `EOF` that is a stop source's end.
pub const STOP: u8 = 32;

/// A token: its kind, flags and spelling (`buf` 0: the scratch bytes; `f + 1`: file `f`'s), and
/// the expansion it came from (`exp`: 0 none, else an index + 1 of `Expanded::exps`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tok {
    pub kind: u8,
    pub flags: u8,
    pub buf: u32,
    pub start: u32,
    pub end: u32,
    pub exp: u32,
}

/// The built-in macros (the lean pass's, `rerail_c_pp::macros::BUILTINS`, in its order): name,
/// and whether it takes an operand.
pub const BUILTINS: &[(&str, bool)] = &[
    ("__LINE__", false),
    ("__FILE__", false),
    ("__FILE_NAME__", false),
    ("__BASE_FILE__", false),
    ("__COUNTER__", false),
    ("__INCLUDE_LEVEL__", false),
    ("__DATE__", false),
    ("__TIME__", false),
    ("__TIMESTAMP__", false),
    ("_Pragma", true),
    ("__has_include", true),
    ("__has_include_next", true),
    ("__has_embed", true),
    ("__has_feature", true),
    ("__has_extension", true),
    ("__has_builtin", true),
    ("__has_attribute", true),
    ("__has_cpp_attribute", true),
    ("__has_c_attribute", true),
    ("__has_declspec_attribute", true),
    ("__has_warning", true),
    ("__is_identifier", true),
    ("__is_target_arch", true),
    ("__is_target_vendor", true),
    ("__is_target_os", true),
    ("__is_target_environment", true),
    ("__is_target_variant_os", true),
    ("__is_target_variant_environment", true),
    ("__has_constexpr_builtin", true),
    ("__building_module", true),
];

/// `BUILTINS`' indexes the words tell apart (the rest from `__has_feature` on are features).
const B_LINE: u8 = 0;
const B_FILE: u8 = 1;
const B_FILE_NAME: u8 = 2;
const B_BASE_FILE: u8 = 3;
const B_COUNTER: u8 = 4;
const B_INCLUDE_LEVEL: u8 = 5;
const B_DATE: u8 = 6;
const B_TIME: u8 = 7;
const B_TIMESTAMP: u8 = 8;
const B_PRAGMA: u8 = 9;
const B_HAS_INCLUDE: u8 = 10;
const B_HAS_INCLUDE_NEXT: u8 = 11;
const B_HAS_EMBED: u8 = 12;

/// The configuration the built-ins read: `__DATE__`, `__TIME__` and `__TIMESTAMP__`, fixed so
/// runs are deterministic (`c_fixed`), and the `__has_feature` family's answers (`c_feature`:
/// built-in, operand, value; a test not there is 0, but for `__is_identifier`, 1).
#[derive(Clone, Copy)]
pub struct Env<'a> {
    pub date: &'a [u8],
    pub time: &'a [u8],
    pub timestamp: &'a [u8],
    pub features: &'a [(&'a [u8], &'a [u8], i64)],
}

impl Env<'static> {
    /// The lean pass's defaults (`rerail_c_pp::Config::new`).
    pub const DEFAULT: Env<'static> = Env { date: b"Jan  1 1970", time: b"00:00:00", timestamp: b"Thu Jan  1 00:00:00 1970", features: &[] };
}

/// A file of the TU: its bytes, its token tape (`buf` its index + 1), its conditionals' chains
/// (each conditional's `#`, a token's index, and its chain's next directive's, rising), and
/// its includes resolved: those whose header name mode (a) read (their `#` and file, `NONE`
/// not found, rising), and those whose name is made by expanding their line, by name.
#[derive(Clone, Copy, Default)]
pub struct File<'a> {
    /// Its path, as `__FILE__` spells it (before a `#line`).
    pub path: &'a [u8],
    pub bytes: &'a [u8],
    /// Its newlines' offsets (`__LINE__`'s, `#line`'s).
    pub lines: &'a [u32],
    pub toks: &'a [Tok],
    pub chains: &'a [(u32, u32)],
    pub includes: &'a [(u32, u32)],
    pub named: &'a [Named],
}

/// An include whose header name is made by expanding its line: the name (its quotes or brackets
/// taken off), `<…>`, `#include_next`, and the file it names (`NONE`: not found).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Named {
    pub name: Vec<u8>,
    pub angle: bool,
    pub next: bool,
    pub to: u32,
}

/// The file includes nest to at most (clang's bound).
pub const MAX_INCLUDE_DEPTH: usize = 200;

/// The stores' bounds.
#[derive(Clone, Debug)]
pub struct Bounds {
    pub arena: usize,
    pub scratch: usize,
    /// Sources, frames, levels and calls: each.
    pub depth: usize,
    pub macros: usize,
    pub output: usize,
    pub steps: u64,
}

impl Bounds {
    /// The bounds the work sets: the arena and output a multiple of the TU's tokens, the table
    /// its directives (each could be a `#define`), the depths clang's nesting limit.
    pub fn of(files: &[File]) -> Bounds {
        let toks: usize = files.iter().map(|f| f.toks.len()).sum();
        let bytes: usize = files.iter().map(|f| f.bytes.len()).sum();
        let hashes = files.iter().flat_map(|f| f.toks.iter()).filter(|t| t.kind == HASH && t.flags & BOL != 0).count();
        Bounds { arena: 64 * toks + 4096, scratch: 64 * bytes + 4096, depth: 256, macros: hashes + 64, output: 64 * toks + 4096, steps: 4096 * (toks as u64 + 64) }
    }
}

/// The expansion: the output tokens and their made spellings (`buf` 0), and the includes whose
/// names no file's `named` has (the including file and the name, as `Named` keys them): the host
/// resolves them and runs the TU again (the import fixpoint).
pub struct Expanded {
    pub toks: Vec<Tok>,
    pub scratch: Vec<u8>,
    pub wanted: Vec<(u32, Named)>,
    /// The expansions (`Tok::exp`): each call's name and its last token (its `)`, or the name).
    pub exps: Vec<(Tok, Tok)>,
    /// The `__has_feature` family's tests `Env` does not answer: built-in and operand.
    pub unanswered: Vec<(Vec<u8>, Vec<u8>)>,
}

impl Expanded {
    /// A token's spelling.
    pub fn text<'a>(&'a self, files: &'a [File], t: &Tok) -> &'a [u8] {
        let b = if t.buf == 0 { &self.scratch[..] } else { files[t.buf as usize - 1].bytes };
        &b[t.start as usize..t.end as usize]
    }
}

/// A TU refused: the word, and why.
#[derive(Debug)]
pub struct Refused {
    pub pc: usize,
    pub why: &'static str,
}

/// The standard's names the words know.
const VA_ARGS: &[u8] = b"__VA_ARGS__";
const VA_OPT: &[u8] = b"__VA_OPT__";

/// Whether a word is a test (it runs the next word where it holds, skips it where not).
fn is_test(opcode: u8, mode: u8) -> bool {
    matches!(opcode, LINE | IS | PEEK | LOOKUP | MACRO | DEFINED) || (opcode == INCLUDE && mode <= 1) || (opcode == BUILTIN && mode >= 1) || (opcode == ARG && (mode == 2 || mode == 4)) || (opcode == PRE && mode == 0) || (opcode == COND && (mode == 2 || mode == 3)) || (opcode == IF && mode == 1)
}

/// A preprocessing program's rules: only these words, `GOTO`, `PUSH` (each followed by its
/// callee's `GOTO`), `RETURN`, `ACCEPT` and `STOP`; operands in range; no word falls off the end;
/// and every cycle reads a token (`READ`, `LINE`), a call counting as one where every way through
/// its callee does.
pub fn validate(program: &Words) -> Result<(), Error> {
    let words = &program.words;
    let n = words.len();
    let string = |s: u16| program.strings.get(usize::from(s)).is_some_and(|s| !s.is_empty());
    for (pc, &w) in words.iter().enumerate() {
        let (op, mode, operand) = split(w);
        let ok = match op {
            READ | LINE | LOOKUP | PAINT | PUT | FRAME | KEEP | SUBST | DEFINE | UNDEF | SKIPLINE | SKIP | DEFINED | PRAGMA => mode == 0 && operand == 0,
            INCLUDE => mode <= 2 && operand == 0,
            IS => match mode {
                0 => operand < u16::from(KINDS),
                1 => string(operand),
                3 | 4 => operand == 0,
                _ => false,
            },
            COND => mode <= 6 && operand == 0,
            IF => mode <= 2 && operand == 0,
            PEEK => mode == 1 && string(operand),
            MACRO => mode <= 2 && operand == 0,
            BUILTIN => mode <= 4 && operand == 0,
            LINEDIR => mode <= 1 && operand == 0,
            ARG => mode <= 5 && operand == 0,
            PRE => mode <= 1 && operand == 0,
            PUSH => mode == 0 && usize::from(operand) < n && words.get(pc + 1).is_some_and(|&w| split(w).0 == GOTO),
            GOTO => mode == 0 && usize::from(operand) < n,
            RETURN | ACCEPT | STOP => mode == 0 && operand == 0,
            _ => return Err(Error::InvalidOpcode(pc)),
        };
        if !ok {
            return Err(Error::InvalidOperand(pc));
        }
        let ends = matches!(op, GOTO | RETURN | ACCEPT | STOP);
        if (is_test(op, mode) && pc + 2 >= n) || (!ends && pc + 1 >= n) {
            return Err(Error::InvalidTarget(pc));
        }
    }
    if program.pp.ops.iter().any(|&s| program.strings.get(usize::from(s)).is_none_or(|b| !(1..=4).contains(&b.len()))) {
        return Err(Error::InvalidOperand(0));
    }
    let mut memo: HashMap<usize, Option<bool>> = HashMap::new();
    // The words after `pc` with no token read between: a read's are none; a call's return place
    // only where its callee can return without reading.
    let mut edges: Vec<Vec<usize>> = Vec::with_capacity(n);
    for pc in 0..n {
        edges.push(after(words, pc, &mut memo));
    }
    // No cycle in that graph.
    let mut color = vec![0u8; n];
    for root in 0..n {
        if color[root] != 0 {
            continue;
        }
        let mut stack = vec![(root, 0usize)];
        color[root] = 1;
        while let Some(&mut (pc, ref mut k)) = stack.last_mut() {
            if let Some(&q) = edges[pc].get(*k) {
                *k += 1;
                match color[q] {
                    0 => {
                        color[q] = 1;
                        stack.push((q, 0));
                    }
                    1 => return Err(Error::NonProgressingCycle(q)),
                    _ => {}
                }
            } else {
                color[pc] = 2;
                stack.pop();
            }
        }
    }
    Ok(())
}

/// The words that can follow `pc` with no token read.
fn after(words: &[u32], pc: usize, memo: &mut HashMap<usize, Option<bool>>) -> Vec<usize> {
    let (op, mode, operand) = split(words[pc]);
    match op {
        READ | LINE | RETURN | ACCEPT | STOP => vec![],
        // `defined` reads its operand where it holds, and nothing where it does not.
        DEFINED => vec![pc + 2],
        GOTO => vec![usize::from(operand)],
        PUSH => {
            let callee = usize::from(split(words[pc + 1]).2);
            if reads(words, callee, memo) { vec![pc + 1] } else { vec![pc + 1, usize::from(operand)] }
        }
        _ if is_test(op, mode) => vec![pc + 1, pc + 2],
        _ => vec![pc + 1],
    }
}

/// Whether every way from `entry` to a `RETURN` reads a token (a callee still being asked about
/// counts as not).
fn reads(words: &[u32], entry: usize, memo: &mut HashMap<usize, Option<bool>>) -> bool {
    match memo.get(&entry) {
        Some(&Some(r)) => return r,
        Some(None) => return false,
        None => {}
    }
    memo.insert(entry, None);
    let mut seen = vec![false; words.len()];
    let mut work = vec![entry];
    let mut ok = true;
    while let Some(pc) = work.pop() {
        if std::mem::replace(&mut seen[pc], true) {
            continue;
        }
        if split(words[pc]).0 == RETURN {
            ok = false;
            break;
        }
        work.extend(after(words, pc, memo));
    }
    memo.insert(entry, Some(ok));
    ok
}

/// No file, definition or source.
pub const NONE: u32 = u32::MAX;
/// A body token's mark: no parameter.
const NO_PARAM: u32 = 0xffff;
/// ...a `__VA_OPT__`.
const VA_OPT_MARK: u32 = 0xfffe;
/// ...and with this bit, a paste operator.
const PASTE_MARK: u32 = 1 << 16;

#[derive(Clone, Copy)]
struct Src {
    /// The file, or `NONE`: the arena.
    file: u32,
    pos: u32,
    end: u32,
    /// The macro enabled again where it is left, or `NONE`.
    def: u32,
    stop: bool,
    /// A file's: the token index of the `#` of the directive read last.
    hash: u32,
    /// A file's, as `#line` set them: its presumed line less its physical line, and its
    /// presumed name (scratch bytes; `NONE`: its path).
    delta: i32,
    presumed: (u32, u32),
}

impl Src {
    fn file(file: u32, end: u32) -> Src {
        Src { file, pos: 0, end, def: NONE, stop: false, hash: NONE, delta: 0, presumed: (NONE, NONE) }
    }

    fn arena(pos: u32, end: u32, def: u32, stop: bool) -> Src {
        Src { file: NONE, pos, end, def, stop, hash: NONE, delta: 0, presumed: (NONE, NONE) }
    }
}

#[derive(Clone, Copy)]
struct Def {
    fnlike: bool,
    variadic: bool,
    params: u16,
    /// The body: file `file`'s tokens `first..end`, and their marks from `marks`.
    file: u32,
    first: u32,
    end: u32,
    marks: u32,
    /// The arguments to expand before substitution, in the order it asks: `uses[pre.0..pre.1]`.
    pre: (u32, u32),
    disabled: bool,
    /// A built-in's index in `BUILTINS` + 1, or 0.
    builtin: u8,
}

#[derive(Clone, Copy)]
struct Slot {
    key: u64,
    name: Tok,
    /// The definition, or `NONE` (undefined).
    def: u32,
}

#[derive(Clone, Copy)]
struct Frame {
    def: u32,
    name: Tok,
    /// Its arguments: `argv[args..]`; their expansions: `pres[pres..pres + params]`.
    args: u32,
    pres: u32,
    depth: u32,
    next_use: u32,
    /// Its arguments are being read (to its `)`).
    open: bool,
    /// Its last token: its `)`, or its name.
    end: Tok,
}

struct Machine<'a> {
    program: &'a Words,
    files: &'a [File<'a>],
    bounds: &'a Bounds,
    arena: Vec<Tok>,
    scratch: Vec<u8>,
    sources: Vec<Src>,
    defs: Vec<Def>,
    marks: Vec<u32>,
    /// A definition's argument uses (`Def::pre`): the parameter, and whether it is inside a
    /// `__VA_OPT__` (expanded only where the variadic argument is present).
    uses: Vec<(u16, bool)>,
    params: Vec<Tok>,
    table: Vec<Option<Slot>>,
    frames: Vec<Frame>,
    argv: Vec<(u32, u32)>,
    /// An argument's expansion, or `(NONE, NONE)` while it is not made.
    pres: Vec<(u32, u32)>,
    /// The open levels: the parameter, and where its tokens start in `kept`.
    levels: Vec<(u16, u32)>,
    kept: Vec<Tok>,
    out: Vec<Tok>,
    t: Tok,
    d: u32,
    /// `T` was read from a file.
    from_file: bool,
    /// The open conditionals: the file source's index, the chain taken, its `#else` seen.
    conds: Vec<(u32, bool, bool)>,
    /// An `#if`'s line is being expanded.
    in_if: bool,
    /// The frames whose arguments are being read: a file's end ends them, not the file.
    collecting: u32,
    /// The files `#pragma once` or `#import` has entered once.
    once: Vec<bool>,
    /// `push_macro`'s stack: the slot and its definition then.
    pushed: Vec<(u32, u32)>,
    /// The include read last: its word (0 `include`, 1 `include_next`, 2 `import`) and file.
    inc: (u8, u32),
    wanted: Vec<(u32, Named)>,
    env: &'a Env<'a>,
    exps: Vec<(Tok, Tok)>,
    unanswered: Vec<(Vec<u8>, Vec<u8>)>,
    counter: u32,
    /// The built-in whose operand is read: its index and name, and the operand's depth.
    bi: (u8, Tok, u32),
    /// The line marker's number (`LINEDIR m=0`).
    marker: Option<Tok>,
    /// The directive line's token read last.
    line_last: Tok,
}

type R<T> = Result<T, &'static str>;

fn eof(flags: u8) -> Tok {
    Tok { kind: EOF, flags, ..Tok::default() }
}

/// A name's key: its bytes eight at a time, and its length.
pub fn key(b: &[u8]) -> u64 {
    let mut h = (b.len() as u64) ^ 0x9e37_79b9_7f4a_7c15;
    for c in b.chunks(8) {
        let mut w = [0u8; 8];
        w[..c.len()].copy_from_slice(c);
        h = (h ^ u64::from_le_bytes(w)).wrapping_mul(0xff51_afd7_ed55_8ccd).rotate_left(31);
    }
    h
}

fn word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

/// Runs a preprocessing program over a TU (file 0 its main file), in bounds.
pub fn run(program: &Words, files: &[File], env: &Env, bounds: &Bounds) -> Result<Expanded, Refused> {
    let table = bounds.macros.next_power_of_two() * 2;
    let mut m = Machine {
        program,
        files,
        bounds,
        arena: Vec::new(),
        scratch: Vec::new(),
        sources: vec![Src::file(0, files.first().map_or(0, |f| f.toks.len() as u32))],
        defs: Vec::new(),
        marks: Vec::new(),
        uses: Vec::new(),
        params: Vec::new(),
        table: vec![None; table],
        frames: Vec::new(),
        argv: Vec::new(),
        pres: Vec::new(),
        levels: Vec::new(),
        kept: Vec::new(),
        out: Vec::new(),
        t: eof(0),
        d: NONE,
        from_file: false,
        conds: Vec::new(),
        in_if: false,
        collecting: 0,
        once: vec![false; files.len()],
        pushed: Vec::new(),
        inc: (0, NONE),
        wanted: Vec::new(),
        env,
        exps: Vec::new(),
        unanswered: Vec::new(),
        counter: 0,
        bi: (0, eof(0), 0),
        marker: None,
        line_last: eof(0),
    };
    m.install_builtins().map_err(|why| Refused { pc: 0, why })?;
    let words = &program.words;
    let mut returns: Vec<usize> = Vec::new();
    let mut pc = 0usize;
    let mut steps = 0u64;
    loop {
        steps += 1;
        if steps > bounds.steps {
            return Err(Refused { pc, why: "past the steps' bound" });
        }
        let (op, mode, operand) = split(words[pc]);
        let at = pc;
        pc += 1;
        let fail = |why| Refused { pc: at, why };
        let held = match op {
            GOTO => {
                pc = usize::from(operand);
                None
            }
            PUSH => {
                if returns.len() >= bounds.depth {
                    return Err(fail("past the calls' bound"));
                }
                returns.push(usize::from(operand));
                None
            }
            RETURN => {
                pc = returns.pop().ok_or_else(|| fail("a return with no call"))?;
                None
            }
            ACCEPT => return Ok(Expanded { toks: m.out, scratch: m.scratch, wanted: m.wanted, exps: m.exps, unanswered: m.unanswered }),
            STOP => return Err(fail("the program stopped")),
            READ => {
                m.read();
                None
            }
            LINE => Some(m.line().map_err(fail)?),
            SKIPLINE => {
                while m.line().map_err(fail)? {}
                None
            }
            IS => Some(match mode {
                0 => m.t.kind == operand as u8,
                1 => m.text(&m.t) == &program.strings[usize::from(operand)][..],
                3 => m.t.kind == HASH && m.t.flags & BOL != 0 && m.from_file,
                _ => m.in_if,
            }),
            SKIP => {
                m.skip().map_err(fail)?;
                None
            }
            COND => m.cond(mode).map_err(fail)?,
            IF if mode != 1 => {
                m.if_open(mode == 0).map_err(fail)?;
                None
            }
            INCLUDE => m.include(mode).map_err(fail)?,
            PRAGMA => {
                m.pragma().map_err(fail)?;
                None
            }
            IF => Some(m.if_close().map_err(fail)?),
            DEFINED => Some(m.defined().map_err(fail)?),
            PEEK => Some(m.peek(&program.strings[usize::from(operand)])),
            LOOKUP => Some(m.lookup().map_err(fail)?),
            MACRO => Some(match mode {
                0 => m.defs[m.d as usize].disabled,
                1 => m.defs[m.d as usize].fnlike,
                _ => m.defs[m.d as usize].builtin != 0,
            }),
            BUILTIN => m.builtin(mode).map_err(fail)?,
            LINEDIR => {
                m.linedir(mode).map_err(fail)?;
                None
            }
            PAINT => {
                m.t.flags |= NOEXP;
                None
            }
            PUT => {
                if m.out.len() >= bounds.output {
                    return Err(fail("past the output's bound"));
                }
                m.out.push(m.t);
                None
            }
            FRAME => {
                m.frame().map_err(fail)?;
                None
            }
            ARG => m.arg(mode).map_err(fail)?,
            PRE if mode == 0 => Some(m.pre().map_err(fail)?),
            PRE => {
                m.pre_end().map_err(fail)?;
                None
            }
            KEEP => {
                if m.kept.len() >= bounds.arena {
                    return Err(fail("past the arena's bound"));
                }
                m.kept.push(m.t);
                None
            }
            SUBST => {
                m.subst().map_err(fail)?;
                None
            }
            DEFINE => {
                m.define().map_err(fail)?;
                None
            }
            UNDEF => {
                m.undef().map_err(fail)?;
                None
            }
            _ => return Err(fail("not a preprocessing word")),
        };
        if held == Some(false) {
            pc += 1;
        }
    }
}

impl Machine<'_> {
    fn text(&self, t: &Tok) -> &[u8] {
        let b = if t.buf == 0 { &self.scratch[..] } else { self.files[t.buf as usize - 1].bytes };
        &b[t.start as usize..t.end as usize]
    }

    fn src_tok(&self, s: &Src, at: u32) -> Tok {
        if s.file == NONE { self.arena[at as usize] } else { self.files[s.file as usize].toks[at as usize] }
    }

    fn read(&mut self) {
        loop {
            let Some(&s) = self.sources.last() else {
                (self.t, self.from_file) = (eof(0), false);
                return;
            };
            if s.pos < s.end {
                self.t = self.src_tok(&s, s.pos);
                self.from_file = s.file != NONE;
                let top = self.sources.last_mut().unwrap();
                if s.file != NONE && self.t.kind == HASH && self.t.flags & BOL != 0 {
                    top.hash = s.pos;
                }
                top.pos += 1;
                return;
            }
            // A stop source's end, the main file's, and a file's while a call's arguments are read
            // (they end with it), are read again and again.
            if s.stop || (s.file != NONE && (self.sources.len() == 1 || self.collecting > 0)) {
                (self.t, self.from_file) = (eof(if s.stop { STOP } else { 0 }), false);
                return;
            }
            let i = self.sources.len() as u32 - 1;
            self.sources.pop();
            if s.file != NONE {
                while self.conds.last().is_some_and(|c| c.0 == i) {
                    self.conds.pop();
                }
            }
            if s.def != NONE {
                self.defs[s.def as usize].disabled = false;
            }
        }
    }

    /// The directive line's next token, from the file on top.
    fn line(&mut self) -> R<bool> {
        let s = *self.sources.last().ok_or("a directive with no file")?;
        if s.file == NONE {
            return Err("a directive's line not in a file");
        }
        if s.pos >= s.end {
            return Ok(false);
        }
        let t = self.src_tok(&s, s.pos);
        if t.flags & BOL != 0 {
            return Ok(false);
        }
        self.t = t;
        self.line_last = t;
        self.sources.last_mut().unwrap().pos += 1;
        Ok(true)
    }

    fn line_tok(&mut self) -> R<Option<Tok>> {
        Ok(if self.line()? { Some(self.t) } else { None })
    }

    fn peek(&self, s: &[u8]) -> bool {
        for src in self.sources.iter().rev() {
            if src.pos < src.end {
                return self.text(&self.src_tok(src, src.pos)) == s;
            }
            if src.stop || src.file != NONE {
                return false;
            }
        }
        false
    }

    /// The slot of the name spelled `b`, or the free slot where it would go.
    fn find(&self, b: &[u8]) -> R<Result<usize, usize>> {
        let k = key(b);
        let mask = self.table.len() - 1;
        let mut i = k as usize & mask;
        for _ in 0..self.table.len() {
            match &self.table[i] {
                None => return Ok(Err(i)),
                Some(s) if s.key == k && self.text(&s.name) == b => return Ok(Ok(i)),
                _ => i = (i + 1) & mask,
            }
        }
        Err("past the macro table's bound")
    }

    fn lookup(&mut self) -> R<bool> {
        if self.t.kind != IDENT || self.t.flags & NOEXP != 0 {
            return Ok(false);
        }
        let b = self.text(&self.t).to_vec();
        Ok(match self.find(&b)? {
            Ok(i) if self.table[i].unwrap().def != NONE => {
                self.d = self.table[i].unwrap().def;
                true
            }
            _ => false,
        })
    }

    fn frame(&mut self) -> R<()> {
        if self.frames.len() >= self.bounds.depth {
            return Err("past the frames' bound");
        }
        let def = self.defs[self.d as usize];
        self.frames.push(Frame { def: self.d, name: self.t, args: self.argv.len() as u32, pres: self.pres.len() as u32, depth: 0, next_use: 0, open: def.fnlike, end: self.t });
        self.collecting += u32::from(def.fnlike);
        self.pres.extend(std::iter::repeat_n((NONE, NONE), usize::from(def.params)));
        if def.fnlike {
            let a = self.arena.len() as u32;
            self.argv.push((a, a));
        }
        Ok(())
    }

    fn drop_frame(&mut self) {
        let f = self.frames.pop().unwrap();
        self.argv.truncate(f.args as usize);
        self.pres.truncate(f.pres as usize);
    }

    fn push_arena(&mut self, t: Tok) -> R<()> {
        if self.arena.len() >= self.bounds.arena {
            return Err("past the arena's bound");
        }
        self.arena.push(t);
        Ok(())
    }

    /// `T` appended to the open argument (a line's first token is one with space before it).
    fn append(&mut self) -> R<()> {
        let mut t = self.t;
        if t.flags & BOL != 0 {
            t.flags = (t.flags & !BOL) | SPACE;
        }
        self.push_arena(t)?;
        self.argv.last_mut().ok_or("an argument with no frame")?.1 = self.arena.len() as u32;
        Ok(())
    }

    fn arg(&mut self, mode: u8) -> R<Option<bool>> {
        let f = *self.frames.last().ok_or("an argument with no frame")?;
        let def = self.defs[f.def as usize];
        match mode {
            0 => self.append()?,
            1 => {
                self.frames.last_mut().unwrap().depth += 1;
                self.append()?;
            }
            2 => {
                if f.depth == 0 {
                    self.frames.last_mut().unwrap().end = self.t;
                    self.close_args();
                    return Ok(Some(true));
                }
                self.frames.last_mut().unwrap().depth -= 1;
                self.append()?;
                return Ok(Some(false));
            }
            3 => {
                let nargs = self.argv.len() - f.args as usize;
                if f.depth == 0 && !(def.variadic && nargs == usize::from(def.params)) {
                    let a = self.arena.len() as u32;
                    self.argv.push((a, a));
                } else {
                    self.append()?;
                }
            }
            4 => {
                let n = usize::from(def.params);
                let nargs = self.argv.len() - f.args as usize;
                let ok = if n == 0 {
                    let (a, b) = self.argv[f.args as usize];
                    nargs == 1 && a == b
                } else {
                    // The variadic arguments left out (C++20, C23, and GNU before them).
                    nargs == n || (nargs + 1 == n && def.variadic)
                };
                if !ok {
                    // A call with the wrong count of arguments is its name alone (clang's).
                    self.drop_frame();
                    self.t = Tok { flags: f.name.flags | NOEXP, ..f.name };
                    return Ok(Some(false));
                }
                if n == 0 {
                    self.argv.truncate(f.args as usize);
                } else if nargs + 1 == n {
                    let a = self.arena.len() as u32;
                    self.argv.push((a, a));
                }
                return Ok(Some(true));
            }
            _ => {
                self.close_args();
                self.drop_frame();
            }
        }
        Ok(None)
    }

    fn close_args(&mut self) {
        let f = self.frames.last_mut().unwrap();
        if f.open {
            f.open = false;
            self.collecting -= 1;
        }
    }

    fn arg_range(&self, f: &Frame, p: usize) -> (u32, u32) {
        self.argv[f.args as usize + p]
    }

    /// Whether the variadic argument has tokens once expanded (`__VA_OPT__`'s test).
    fn va_present(&self, f: &Frame) -> bool {
        let def = &self.defs[f.def as usize];
        let va = usize::from(def.params) - 1;
        let (a, b) = self.arg_range(f, va);
        let (x, y) = self.pres[f.pres as usize + va];
        a != b && x != NONE && x != y
    }

    fn pre(&mut self) -> R<bool> {
        let f = *self.frames.last().ok_or("an expansion with no frame")?;
        let def = self.defs[f.def as usize];
        let mut u = f.next_use;
        while def.pre.0 + u < def.pre.1 {
            let (p, inside) = self.uses[(def.pre.0 + u) as usize];
            u += 1;
            let p = usize::from(p);
            if self.pres[f.pres as usize + p].0 != NONE || (inside && !self.va_present(&f)) {
                continue;
            }
            let (a, b) = self.arg_range(&f, p);
            if a == b {
                self.pres[f.pres as usize + p] = (b, b);
                continue;
            }
            self.frames.last_mut().unwrap().next_use = u;
            if self.sources.len() >= self.bounds.depth || self.levels.len() >= self.bounds.depth {
                return Err("past the sources' bound");
            }
            self.sources.push(Src::arena(a, b, NONE, true));
            self.levels.push((p as u16, self.kept.len() as u32));
            return Ok(true);
        }
        self.frames.last_mut().unwrap().next_use = u;
        Ok(false)
    }

    fn pre_end(&mut self) -> R<()> {
        let (p, at) = self.levels.pop().ok_or("no level open")?;
        if !self.sources.pop().is_some_and(|s| s.stop) {
            return Err("a level's source not on top");
        }
        let a = self.arena.len() as u32;
        for k in at as usize..self.kept.len() {
            self.push_arena(self.kept[k])?;
        }
        self.kept.truncate(at as usize);
        let f = *self.frames.last().ok_or("a level with no frame")?;
        self.pres[f.pres as usize + usize::from(p)] = (a, self.arena.len() as u32);
        Ok(())
    }

    // The body.

    fn body_tok(&self, def: &Def, k: u32) -> Tok {
        let mut t = self.files[def.file as usize].toks[(def.first + k) as usize];
        if k == 0 {
            t.flags &= !SPACE;
        }
        if self.marks[(def.marks + k) as usize] & PASTE_MARK != 0 {
            t.flags |= PASTE;
        }
        t
    }

    fn mark(&self, def: &Def, k: u32) -> u32 {
        self.marks[(def.marks + k) as usize] & 0xffff
    }

    fn param(&self, def: &Def, k: u32) -> Option<usize> {
        let m = self.mark(def, k);
        (m < VA_OPT_MARK).then_some(m as usize)
    }

    fn spelled(&self, t: &Tok, s: &[u8]) -> bool {
        self.text(t) == s
    }

    /// `__VA_OPT__`'s content at body token `i` (its `(`'s match, else the body's end), and the
    /// token after it.
    fn va_opt_content(&self, def: &Def, i: u32, hi: u32) -> (u32, u32, u32) {
        if !(i + 1 < hi && self.spelled(&self.body_tok(def, i + 1), b"(")) {
            return (i + 1, i + 1, i + 1);
        }
        let mut depth = 0;
        for j in i + 2..hi {
            let t = self.body_tok(def, j);
            if self.spelled(&t, b"(") {
                depth += 1;
            } else if self.spelled(&t, b")") {
                if depth == 0 {
                    return (i + 2, j, j + 1);
                }
                depth -= 1;
            }
        }
        (i + 2, hi, hi)
    }

    fn arena_toks(&self, (a, b): (u32, u32)) -> Vec<Tok> {
        self.arena[a as usize..b as usize].to_vec()
    }

    /// The body's tokens `lo..hi` with the top frame's arguments substituted.
    fn substitute(&mut self, f: &Frame, lo: u32, hi: u32, id: u32) -> R<Vec<Tok>> {
        let def = self.defs[f.def as usize];
        let va = def.variadic.then(|| usize::from(def.params) - 1);
        let mut out: Vec<Tok> = Vec::new();
        let mut i = lo;
        while i < hi {
            let bt = self.body_tok(&def, i);
            let next_param = if i + 1 < hi { self.param(&def, i + 1) } else { None };
            if def.fnlike && bt.kind == HASH {
                if let Some(p) = next_param {
                    let toks = self.arena_toks(self.arg_range(f, p));
                    let t = self.stringized(&toks, &bt)?;
                    out.push(Tok { exp: id, ..t });
                    i += 2;
                    continue;
                }
                if va.is_some() && i + 1 < hi && self.mark(&def, i + 1) == VA_OPT_MARK {
                    // `#__VA_OPT__(…)`: the content, substituted and pasted, stringized.
                    let (clo, chi, end) = self.va_opt_content(&def, i + 1, hi);
                    let mut c = if self.va_present(f) { self.substitute(f, clo, chi, id)? } else { Vec::new() };
                    self.paste_all(&mut c, id)?;
                    c.retain(|t| t.flags & PLACEMARKER == 0);
                    let t = self.stringized(&c, &bt)?;
                    out.push(Tok { exp: id, ..t });
                    i = end;
                    continue;
                }
            }
            if va.is_some() && self.mark(&def, i) == VA_OPT_MARK {
                let (clo, chi, end) = self.va_opt_content(&def, i, hi);
                if self.va_present(f) {
                    let mut c = self.substitute(f, clo, chi, id)?;
                    if let Some(x) = c.first_mut() {
                        x.flags = (x.flags & !SPACE) | (bt.flags & SPACE);
                    }
                    out.extend(c);
                } else {
                    out.push(Tok { flags: PLACEMARKER, ..bt });
                }
                i = end;
                continue;
            }
            if let Some(p) = self.param(&def, i) {
                let before = i > lo && self.body_tok(&def, i - 1).flags & PASTE != 0;
                let after = i + 1 < hi && self.body_tok(&def, i + 1).flags & PASTE != 0;
                let raw = self.arg_range(f, p);
                let comma_paste = |out: &Vec<Tok>, m: &Self| out.len() >= 2 && m.spelled(&out[out.len() - 2], b",") && out[out.len() - 2].kind == PUNCT && out[out.len() - 1].flags & PASTE != 0;
                // GNU's `, ## __VA_ARGS__`: with no variadic arguments the comma goes...
                if before && Some(p) == va && raw.0 == raw.1 && comma_paste(&out, self) {
                    out.pop();
                    out.pop();
                    i += 1;
                    continue;
                }
                // ...and with some, the `##` is no paste.
                if before && Some(p) == va && comma_paste(&out, self) {
                    out.pop();
                }
                let toks = if before || after {
                    self.arena_toks(raw)
                } else {
                    let r = self.pres[f.pres as usize + p];
                    if r.0 == NONE {
                        return Err("an argument substituted before its expansion");
                    }
                    self.arena_toks(r)
                };
                if toks.is_empty() {
                    out.push(Tok { flags: PLACEMARKER, ..bt });
                } else {
                    let k = out.len();
                    out.extend(toks);
                    out[k].flags = (out[k].flags & !(BOL | SPACE)) | (bt.flags & SPACE);
                }
                i += 1;
                continue;
            }
            out.push(Tok { exp: id, ..bt });
            i += 1;
        }
        Ok(out)
    }

    fn scratch_tok(&mut self, kind: u8, text: &[u8], flags: u8) -> R<Tok> {
        if self.scratch.len() + text.len() > self.bounds.scratch {
            return Err("past the scratch bytes' bound");
        }
        let start = self.scratch.len() as u32;
        self.scratch.extend_from_slice(text);
        Ok(Tok { kind, flags, buf: 0, start, end: self.scratch.len() as u32, exp: 0 })
    }

    /// `#`'s literal of `toks` (C17 §6.10.3.2): a blank where there was white space, `"` and `\`
    /// escaped in literals; its flags `like`'s.
    fn stringized(&mut self, toks: &[Tok], like: &Tok) -> R<Tok> {
        let mut s = vec![b'"'];
        for (k, t) in toks.iter().enumerate() {
            if t.flags & PLACEMARKER != 0 {
                continue;
            }
            if k > 0 && t.flags & (SPACE | BOL) != 0 && s.len() > 1 {
                s.push(b' ');
            }
            let text = self.text(t);
            if t.kind == LITERAL {
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
        s.push(b'"');
        self.scratch_tok(LITERAL, &s, like.flags & (BOL | SPACE))
    }

    /// The pastes of `out`, left to right.
    fn paste_all(&mut self, out: &mut Vec<Tok>, id: u32) -> R<()> {
        if !out.iter().any(|t| t.flags & PASTE != 0) {
            return Ok(());
        }
        let mut res: Vec<Tok> = Vec::with_capacity(out.len());
        let mut i = 0;
        while i < out.len() {
            let t = out[i];
            if t.flags & PASTE != 0 && !res.is_empty() && i + 1 < out.len() {
                let l = res.pop().unwrap();
                let r = out[i + 1];
                match self.paste(l, r, id)? {
                    Some(m) => res.push(m),
                    None => {
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
        Ok(())
    }

    /// `l ## r`: one token, or `None` where the two spell no one token.
    fn paste(&mut self, l: Tok, r: Tok, id: u32) -> R<Option<Tok>> {
        if r.flags & PLACEMARKER != 0 {
            return Ok(Some(l));
        }
        if l.flags & PLACEMARKER != 0 {
            return Ok(Some(Tok { flags: (r.flags & !(BOL | SPACE)) | (l.flags & (BOL | SPACE)), ..r }));
        }
        let (lb, rb) = (self.text(&l).to_vec(), self.text(&r).to_vec());
        let mut text = lb.clone();
        text.extend_from_slice(&rb);
        let Some(kind) = self.pasted_kind(&l, &r, &lb, &rb, &text)? else { return Ok(None) };
        Ok(Some(Tok { exp: id, ..self.scratch_tok(kind, &text, l.flags & (BOL | SPACE))? }))
    }

    /// The kind of the one token `l`'s and `r`'s bytes spell, `None` where they spell none.
    fn pasted_kind(&self, l: &Tok, r: &Tok, lb: &[u8], rb: &[u8], text: &[u8]) -> R<Option<u8>> {
        let op_like = |k: u8| matches!(k, PUNCT | DIGRAPH | HASH | HASHHASH);
        Ok(match (l.kind, r.kind) {
            (IDENT, IDENT) => Some(IDENT),
            (IDENT, NUMBER) => rb.iter().all(|&c| word_byte(c)).then_some(IDENT),
            (IDENT, LITERAL) if matches!(rb.first(), Some(b'"' | b'\'')) => {
                if matches!(lb, b"L" | b"u" | b"U" | b"u8") {
                    Some(LITERAL)
                } else if matches!(lb, b"R" | b"LR" | b"uR" | b"UR" | b"u8R") {
                    return Err("a raw string's prefix pasted");
                } else {
                    None
                }
            }
            (NUMBER, IDENT | NUMBER) => Some(NUMBER),
            (NUMBER, PUNCT) => (rb.iter().all(|&c| c == b'.') || (matches!(rb, b"+" | b"-") && matches!(lb.last(), Some(b'e' | b'E' | b'p' | b'P')))).then_some(NUMBER),
            (PUNCT, NUMBER) => (lb == b"." && rb.first().is_some_and(u8::is_ascii_digit)).then_some(NUMBER),
            (LITERAL, IDENT) => self.program.pp.suffix.then_some(LITERAL),
            (a, b) if op_like(a) && op_like(b) => {
                let strings = &self.program.strings;
                self.program.pp.ops.iter().any(|&s| strings[usize::from(s)] == text).then(|| match text {
                    b"#" | b"%:" => HASH,
                    b"##" | b"%:%:" => HASHHASH,
                    b"<:" | b":>" | b"<%" | b"%>" => DIGRAPH,
                    _ => PUNCT,
                })
            }
            _ => None,
        })
    }

    fn subst(&mut self) -> R<()> {
        let f = *self.frames.last().ok_or("a substitution with no frame")?;
        let def = self.defs[f.def as usize];
        if self.exps.len() >= self.bounds.output {
            return Err("past the expansions' bound");
        }
        self.exps.push((f.name, f.end));
        let id = self.exps.len() as u32;
        let mut out = self.substitute(&f, 0, def.end - def.first, id)?;
        self.paste_all(&mut out, id)?;
        out.retain(|t| t.flags & PLACEMARKER == 0);
        if let Some(x) = out.first_mut() {
            x.flags = (x.flags & !(BOL | SPACE)) | (f.name.flags & (BOL | SPACE));
        }
        let a = self.arena.len() as u32;
        for t in out {
            self.push_arena(t)?;
        }
        self.drop_frame();
        if self.sources.len() >= self.bounds.depth {
            return Err("past the sources' bound");
        }
        self.defs[f.def as usize].disabled = true;
        self.sources.push(Src::arena(a, self.arena.len() as u32, f.def, false));
        Ok(())
    }

    // The conditionals.

    /// The top source, a file's index.
    fn file_src(&self) -> R<usize> {
        let i = self.sources.len().checked_sub(1).ok_or("a directive with no file")?;
        if self.sources[i].file == NONE {
            return Err("a directive's line not in a file");
        }
        Ok(i)
    }

    fn skip(&mut self) -> R<()> {
        let i = self.file_src()?;
        let s = self.sources[i];
        let chains = self.files[s.file as usize].chains;
        let next = chains.binary_search_by_key(&s.hash, |c| c.0).ok().map(|k| chains[k].1);
        self.sources[i].pos = next.filter(|&n| n > s.hash).unwrap_or(s.end);
        Ok(())
    }

    fn cond(&mut self, mode: u8) -> R<Option<bool>> {
        let i = self.file_src()? as u32;
        let top = self.conds.last().copied().filter(|c| c.0 == i);
        match mode {
            0 | 1 => {
                if self.conds.len() >= self.bounds.depth * 16 {
                    return Err("past the conditionals' bound");
                }
                self.conds.push((i, mode == 1, false));
            }
            2 => return Ok(Some(top.is_some())),
            3 => return Ok(Some(top.is_none_or(|c| c.1 || c.2))),
            4 | 5 => {
                if let Some(c) = self.conds.last_mut().filter(|c| c.0 == i) {
                    if mode == 4 {
                        c.1 = true;
                    } else {
                        c.2 = true;
                    }
                }
            }
            _ => {
                if top.is_some() {
                    self.conds.pop();
                }
            }
        }
        Ok(None)
    }

    fn if_open(&mut self, in_if: bool) -> R<()> {
        let a = self.arena.len() as u32;
        while self.line()? {
            let t = self.t;
            self.push_arena(t)?;
        }
        if self.sources.len() >= self.bounds.depth || self.levels.len() >= self.bounds.depth {
            return Err("past the sources' bound");
        }
        self.sources.push(Src::arena(a, self.arena.len() as u32, NONE, true));
        self.levels.push((u16::MAX, self.kept.len() as u32));
        self.in_if = in_if;
        Ok(())
    }

    /// The level `if_open` opened closed: its tokens.
    fn line_close(&mut self) -> R<Vec<Tok>> {
        let (_, at) = self.levels.pop().ok_or("no level open")?;
        if !self.sources.pop().is_some_and(|s| s.stop) {
            return Err("a line's source not on top");
        }
        self.in_if = false;
        Ok(self.kept.drain(at as usize..).collect())
    }

    fn if_close(&mut self) -> R<bool> {
        let toks = self.line_close()?;
        let items: Vec<crate::pp_expr::Item> = toks.iter().map(|t| crate::pp_expr::item(t.kind, self.text(t), self.program.pp.cpp)).collect();
        Ok(crate::pp_expr::eval(&items, self.program.pp.cpp).is_ok_and(|v| v.is_true()))
    }

    /// `defined X` or `defined ( X )`: `T` its value, a number.
    fn defined(&mut self) -> R<bool> {
        let t = self.t;
        if t.kind != IDENT || t.flags & NOEXP != 0 || self.text(&t) != b"defined" {
            return Ok(false);
        }
        self.read();
        let paren = self.spelled(&self.t, b"(");
        if paren {
            self.read();
        }
        let n = self.t;
        let v = n.kind == IDENT && {
            let b = self.text(&n).to_vec();
            matches!(self.find(&b)?, Ok(i) if self.table[i].unwrap().def != NONE)
        };
        if paren {
            self.read();
        }
        self.t = self.scratch_tok(NUMBER, if v { b"1" } else { b"0" }, t.flags & (BOL | SPACE))?;
        Ok(true)
    }

    // The includes.

    fn include(&mut self, mode: u8) -> R<Option<bool>> {
        // `m=1`'s line, its stop source on top, closed first.
        let toks = if mode == 1 { self.line_close()? } else { Vec::new() };
        let i = self.file_src()?;
        let s = self.sources[i];
        match mode {
            0 => {
                self.inc.0 = match self.text(&self.t) {
                    b"include_next" => 1,
                    b"import" => 2,
                    _ => 0,
                };
                let incs = self.files[s.file as usize].includes;
                let Ok(k) = incs.binary_search_by_key(&s.hash, |x| x.0) else { return Ok(Some(false)) };
                self.inc.1 = incs[k].1;
                self.skip_line()?;
                Ok(Some(true))
            }
            1 => {
                let Some((name, angle)) = self.header_name(&toks) else { return Ok(Some(false)) };
                // `#include_next` in the main file is `#include` (clang's reading).
                let next = self.inc.0 == 1 && s.file != 0;
                let named = self.files[s.file as usize].named;
                match named.iter().find(|n| n.name == name && n.angle == angle && n.next == next) {
                    Some(n) => {
                        self.inc.1 = n.to;
                        Ok(Some(true))
                    }
                    None => {
                        self.wanted.push((s.file, Named { name, angle, next, to: NONE }));
                        Ok(Some(false))
                    }
                }
            }
            _ => {
                let f = self.inc.1;
                let depth = self.sources.iter().filter(|s| s.file != NONE).count();
                if f == NONE || self.once[f as usize] || depth > MAX_INCLUDE_DEPTH {
                    return Ok(None);
                }
                if self.inc.0 == 2 {
                    self.once[f as usize] = true;
                }
                if self.sources.len() >= self.bounds.depth {
                    return Err("past the sources' bound");
                }
                let end = self.files[f as usize].toks.len() as u32;
                self.sources.push(Src::file(f, end));
                Ok(None)
            }
        }
    }

    /// The header name tokens make: a string literal's inside, or `<` … `>` spelled with their
    /// blanks (the lean pass's `header_of`).
    fn header_name(&self, toks: &[Tok]) -> Option<(Vec<u8>, bool)> {
        let first = toks.first()?;
        let s = self.text(first);
        if first.kind == LITERAL && s.first() == Some(&b'"') && s.len() >= 2 {
            return Some((s[1..s.len() - 1].to_vec(), false));
        }
        if s != b"<" {
            return None;
        }
        let mut name = Vec::new();
        for (k, t) in toks[1..].iter().enumerate() {
            if self.text(t) == b">" && t.kind == PUNCT {
                return Some((name, true));
            }
            if k > 0 && t.flags & SPACE != 0 {
                name.push(b' ');
            }
            name.extend_from_slice(self.text(t));
        }
        None
    }

    fn pragma(&mut self) -> R<()> {
        let i = self.file_src()?;
        let file = self.sources[i].file;
        let mut toks = Vec::new();
        while let Some(t) = self.line_tok()? {
            if toks.len() < 4 {
                toks.push(t);
            }
        }
        let word = toks.first().filter(|t| t.kind == IDENT).map_or(Vec::new(), |t| self.text(t).to_vec());
        // The lean pass's string argument: `( "…" )`, a string literal, not a raw one.
        let arg = toks.get(1).filter(|t| self.spelled(t, b"(")).and(toks.get(2)).filter(|t| t.kind == LITERAL).and_then(|t| destringize(self.text(t)));
        self.pragma_do(file, &word, arg)
    }

    /// A pragma carried out: its first word, and its string argument destringized.
    fn pragma_do(&mut self, file: u32, word: &[u8], arg: Option<Vec<u8>>) -> R<()> {
        if word == b"once" {
            self.once[file as usize] = true;
            return Ok(());
        }
        let (push, pop) = (word == b"push_macro", word == b"pop_macro");
        let Some(name) = arg.filter(|_| push || pop) else { return Ok(()) };
        let slot = match self.find(&name)? {
            Ok(i) => i,
            Err(i) if push => {
                let n = self.scratch_tok(IDENT, &name, 0)?;
                self.table[i] = Some(Slot { key: key(&name), name: n, def: NONE });
                i
            }
            Err(_) => return Ok(()),
        };
        if push {
            if self.pushed.len() >= self.bounds.macros * 4 {
                return Err("past push_macro's bound");
            }
            self.pushed.push((slot as u32, self.table[slot].unwrap().def));
        } else if let Some(k) = self.pushed.iter().rposition(|p| p.0 == slot as u32) {
            let (_, d) = self.pushed.remove(k);
            self.table[slot].as_mut().unwrap().def = d;
        }
        Ok(())
    }

    // The built-ins.

    fn install_builtins(&mut self) -> R<()> {
        for (k, &(name, fnlike)) in BUILTINS.iter().enumerate() {
            let n = self.scratch_tok(IDENT, name.as_bytes(), 0)?;
            self.defs.push(Def { fnlike, variadic: false, params: 0, file: 0, first: 0, end: 0, marks: 0, pre: (0, 0), disabled: false, builtin: k as u8 + 1 });
            let d = (self.defs.len() - 1) as u32;
            match self.find(name.as_bytes())? {
                Err(i) => self.table[i] = Some(Slot { key: key(name.as_bytes()), name: n, def: d }),
                Ok(_) => return Err("a built-in twice"),
            }
        }
        Ok(())
    }

    fn builtin(&mut self, mode: u8) -> R<Option<bool>> {
        let b = self.defs[self.d as usize].builtin.wrapping_sub(1);
        match mode {
            0 => {
                let t = self.t;
                self.t = self.builtin_value(b, &t, &[])?;
                Ok(None)
            }
            1 => {
                if !self.in_if && matches!(b, B_HAS_INCLUDE | B_HAS_INCLUDE_NEXT | B_HAS_EMBED) {
                    return Ok(Some(false));
                }
                self.bi = (b, self.t, 0);
                self.levels.push((u16::MAX, self.kept.len() as u32));
                self.collecting += 1;
                Ok(Some(true))
            }
            2 => {
                let t = self.t;
                if self.spelled(&t, b")") && t.kind == PUNCT {
                    if self.bi.2 == 0 {
                        return Ok(Some(true));
                    }
                    self.bi.2 -= 1;
                } else if self.spelled(&t, b"(") && t.kind == PUNCT {
                    self.bi.2 += 1;
                }
                if self.kept.len() >= self.bounds.arena {
                    return Err("past the arena's bound");
                }
                self.kept.push(t);
                Ok(Some(false))
            }
            3 => {
                self.collecting -= 1;
                let (_, at) = self.levels.pop().ok_or("no level open")?;
                let op: Vec<Tok> = self.kept.drain(at as usize..).collect();
                let (b, name, _) = self.bi;
                let header = matches!(b, B_HAS_INCLUDE | B_HAS_INCLUDE_NEXT | B_HAS_EMBED);
                let starts = |m: &Self| op.first().is_some_and(|t| (t.kind == LITERAL && m.text(t).first() == Some(&b'"')) || (t.kind == PUNCT && m.spelled(t, b"<")));
                if b == B_PRAGMA || (header && !starts(self)) {
                    // The operand expanded first: a stop source of its tokens, and a level.
                    let a = self.arena.len() as u32;
                    for t in op {
                        self.push_arena(t)?;
                    }
                    if self.sources.len() >= self.bounds.depth || self.levels.len() >= self.bounds.depth {
                        return Err("past the sources' bound");
                    }
                    self.sources.push(Src::arena(a, self.arena.len() as u32, NONE, true));
                    self.levels.push((u16::MAX, self.kept.len() as u32));
                    return Ok(Some(true));
                }
                self.t = self.builtin_value(b, &name, &op)?;
                Ok(Some(false))
            }
            _ => {
                let toks = self.line_close()?;
                let (b, name, _) = self.bi;
                if b != B_PRAGMA {
                    self.t = self.builtin_value(b, &name, &toks)?;
                    return Ok(Some(true));
                }
                // `_Pragma("…")`: one string literal, its text the pragma's.
                if let [s] = toks.as_slice()
                    && s.kind == LITERAL
                    && let Some(text) = destringize(self.text(s))
                {
                    let file = self.cur_file();
                    let (word, arg) = pragma_words(&text);
                    self.pragma_do(file, &word, arg)?;
                }
                Ok(Some(false))
            }
        }
    }

    /// The file on top: the innermost file source's.
    fn cur_file(&self) -> u32 {
        self.sources.iter().rev().find(|s| s.file != NONE).map_or(0, |s| s.file)
    }

    /// A file token's presumed line: its physical line, moved by the innermost `#line` of its
    /// file.
    fn line_of(&self, t: &Tok) -> u32 {
        if t.buf == 0 || t.buf as usize > self.files.len() {
            return 0;
        }
        let f = t.buf - 1;
        let phys = self.files[f as usize].lines.partition_point(|&n| n < t.start) as i64 + 1;
        let delta = self.sources.iter().rev().find(|s| s.file == f).map_or(0, |s| s.delta);
        (phys + i64::from(delta)) as u32
    }

    fn presumed_path(&self) -> Vec<u8> {
        match self.sources.iter().rev().find(|s| s.file != NONE) {
            Some(s) if s.presumed.0 != NONE => self.scratch[s.presumed.0 as usize..s.presumed.1 as usize].to_vec(),
            Some(s) => self.files[s.file as usize].path.to_vec(),
            None => Vec::new(),
        }
    }

    fn number_tok(&mut self, v: i64, like: &Tok) -> R<Tok> {
        let t = self.scratch_tok(NUMBER, v.to_string().as_bytes(), like.flags & (BOL | SPACE))?;
        Ok(Tok { exp: like.exp, ..t })
    }

    /// A string literal of `s`, its `"` and `\` escaped.
    fn string_tok(&mut self, s: &[u8], like: &Tok) -> R<Tok> {
        let mut q = vec![b'"'];
        for &c in s {
            if c == b'"' || c == b'\\' {
                q.push(b'\\');
            }
            q.push(c);
        }
        q.push(b'"');
        let t = self.scratch_tok(LITERAL, &q, like.flags & (BOL | SPACE))?;
        Ok(Tok { exp: like.exp, ..t })
    }

    /// Built-in `b`'s value for its use `t` and its operand `op`.
    fn builtin_value(&mut self, b: u8, t: &Tok, op: &[Tok]) -> R<Tok> {
        match b {
            B_LINE => {
                // The line of the outermost call's end (clang's: its expansion range's end).
                let mut r = *t;
                while r.exp != 0 {
                    r = self.exps[r.exp as usize - 1].1;
                }
                let line = self.line_of(&r);
                self.number_tok(i64::from(line), t)
            }
            B_FILE | B_FILE_NAME | B_BASE_FILE => {
                let path = if b == B_BASE_FILE { self.files.first().map_or(Vec::new(), |f| f.path.to_vec()) } else { self.presumed_path() };
                let path = if b == B_FILE_NAME { path.rsplit(|&c| c == b'/').next().unwrap_or(b"").to_vec() } else { path };
                self.string_tok(&path, t)
            }
            B_COUNTER => {
                self.counter += 1;
                self.number_tok(i64::from(self.counter) - 1, t)
            }
            B_INCLUDE_LEVEL => {
                let n = self.sources.iter().filter(|s| s.file != NONE).count() as i64 - 1;
                self.number_tok(n, t)
            }
            B_DATE | B_TIME | B_TIMESTAMP => {
                let s = match b {
                    B_DATE => self.env.date,
                    B_TIME => self.env.time,
                    _ => self.env.timestamp,
                };
                self.string_tok(s, t)
            }
            B_HAS_INCLUDE | B_HAS_INCLUDE_NEXT | B_HAS_EMBED => {
                let file = self.cur_file();
                let v = match self.header_name(op) {
                    Some((name, angle)) => {
                        let next = b == B_HAS_INCLUDE_NEXT && file != 0;
                        let named = self.files[file as usize].named;
                        match named.iter().find(|n| n.name == name && n.angle == angle && n.next == next) {
                            Some(n) if n.to == NONE => 0,
                            // 1: found, and not empty; 2: found and empty (`__STDC_EMBED_EMPTY__`).
                            Some(n) if b == B_HAS_EMBED && self.files[n.to as usize].bytes.is_empty() => 2,
                            Some(_) => 1,
                            None => {
                                self.wanted.push((file, Named { name, angle, next, to: NONE }));
                                0
                            }
                        }
                    }
                    None => 0,
                };
                self.number_tok(v, t)
            }
            _ => {
                let builtin = BUILTINS[usize::from(b)].0.as_bytes();
                let mut name = Vec::new();
                for x in op {
                    name.extend_from_slice(self.text(x));
                }
                let v = match self.env.features.iter().find(|f| f.0 == builtin && f.1 == &name[..]) {
                    Some(f) => f.2,
                    None => {
                        self.unanswered.push((builtin.to_vec(), name));
                        i64::from(builtin == b"__is_identifier")
                    }
                };
                self.number_tok(v, t)
            }
        }
    }

    /// `#line N "file"` and `# N "file" flags`: the next line is line `N` (of `file`).
    fn linedir(&mut self, mode: u8) -> R<()> {
        if mode == 0 {
            self.marker = Some(self.t);
            return Ok(());
        }
        let last = self.line_last;
        let mut toks = self.line_close()?;
        if let Some(m) = self.marker.take() {
            toks.insert(0, m);
        }
        let i = self.file_src()?;
        let Some(n) = toks.first().filter(|t| t.kind == NUMBER).and_then(|t| crate::pp_expr::number(self.text(t))) else { return Ok(()) };
        let path = toks.get(1).filter(|t| t.kind == LITERAL).and_then(|t| destringize(self.text(t)));
        // The directive's last line: the line after it is line `n`.
        let s = self.sources[i];
        let phys = self.files[s.file as usize].lines.partition_point(|&k| k < last.end) as i64 + 2;
        self.sources[i].delta = (n.bits as i64 - phys) as i32;
        if let Some(p) = path {
            let t = self.scratch_tok(LITERAL, &p, 0)?;
            self.sources[i].presumed = (t.start, t.end);
        }
        Ok(())
    }

    // The directives.

    fn define(&mut self) -> R<()> {
        let Some(name) = self.line_tok()?.filter(|t| t.kind == IDENT) else {
            return self.skip_line();
        };
        let s = *self.sources.last().unwrap();
        let p0 = self.params.len();
        let (mut fnlike, mut variadic) = (false, false);
        let mut first = self.line_tok()?;
        if first.is_some_and(|t| self.spelled(&t, b"(") && t.flags & SPACE == 0) {
            fnlike = true;
            let mut want = true;
            loop {
                let Some(t) = self.line_tok()? else {
                    self.params.truncate(p0);
                    return Ok(());
                };
                let n = self.params.len() - p0;
                if self.spelled(&t, b")") && (!want || n == 0) && !(want && variadic) {
                    break;
                }
                let malformed = if variadic {
                    true
                } else if want && t.kind == IDENT {
                    self.params.push(t);
                    want = false;
                    false
                } else if want && self.spelled(&t, b"...") {
                    // `__VA_ARGS__`'s spelling, made once.
                    let va = self.scratch_tok(IDENT, VA_ARGS, 0)?;
                    self.params.push(va);
                    variadic = true;
                    want = false;
                    false
                } else if !want && self.spelled(&t, b"...") && n > 0 {
                    // GNU's `name...`.
                    variadic = true;
                    false
                } else if !want && self.spelled(&t, b",") {
                    want = true;
                    false
                } else {
                    true
                };
                if malformed {
                    self.params.truncate(p0);
                    return self.skip_line();
                }
            }
            first = self.line_tok()?;
        }
        // The body: the line's tokens from `first`, a range of the file's tape.
        let top = *self.sources.last().unwrap();
        let first_at = if first.is_some() { top.pos - 1 } else { top.pos };
        while self.line()? {}
        let end = self.sources.last().unwrap().pos;
        let def_params: Vec<Tok> = self.params[p0..].to_vec();
        self.params.truncate(p0);
        if def_params.len() > 0xfff0 {
            return Err("past the parameters' bound");
        }
        let file = s.file;
        let marks = self.marks.len() as u32;
        let n = end - first_at;
        for k in 0..n {
            let t = self.files[file as usize].toks[(first_at + k) as usize];
            let text = self.text(&t);
            let mut m = NO_PARAM;
            if t.kind == IDENT {
                if variadic && text == VA_OPT {
                    m = VA_OPT_MARK;
                } else if fnlike && let Some(p) = def_params.iter().position(|p| self.text(p) == text) {
                    m = p as u32;
                }
            }
            if t.kind == HASHHASH && k > 0 && k + 1 < n {
                m |= PASTE_MARK;
            }
            self.marks.push(m);
        }
        let mut def = Def { fnlike, variadic, params: def_params.len() as u16, file, first: first_at, end, marks, pre: (0, 0), disabled: false, builtin: 0 };
        let u0 = self.uses.len() as u32;
        if fnlike {
            self.walk_uses(&def, 0, n, false);
        }
        def.pre = (u0, self.uses.len() as u32);
        if self.defs.len() >= self.bounds.macros * 4 {
            return Err("past the definitions' bound");
        }
        self.defs.push(def);
        let d = (self.defs.len() - 1) as u32;
        let b = self.text(&name).to_vec();
        match self.find(&b)? {
            Ok(i) => self.table[i].as_mut().unwrap().def = d,
            Err(i) => self.table[i] = Some(Slot { key: key(&b), name, def: d }),
        }
        Ok(())
    }

    /// The arguments substitution expands, in the order it first asks for each (`substitute`'s
    /// walk): `(parameter, inside a __VA_OPT__)`.
    fn walk_uses(&mut self, def: &Def, lo: u32, hi: u32, inside: bool) {
        let va = def.params.wrapping_sub(1);
        let mut i = lo;
        while i < hi {
            let bt = self.body_tok(def, i);
            if bt.kind == HASH {
                if i + 1 < hi && self.param(def, i + 1).is_some() {
                    i += 2;
                    continue;
                }
                if def.variadic && i + 1 < hi && self.mark(def, i + 1) == VA_OPT_MARK {
                    self.uses.push((va, inside));
                    let (clo, chi, end) = self.va_opt_content(def, i + 1, hi);
                    self.walk_uses(def, clo, chi, true);
                    i = end;
                    continue;
                }
            }
            if def.variadic && self.mark(def, i) == VA_OPT_MARK {
                self.uses.push((va, inside));
                let (clo, chi, end) = self.va_opt_content(def, i, hi);
                self.walk_uses(def, clo, chi, true);
                i = end;
                continue;
            }
            if let Some(p) = self.param(def, i) {
                let before = i > lo && self.body_tok(def, i - 1).flags & PASTE != 0;
                let after = i + 1 < hi && self.body_tok(def, i + 1).flags & PASTE != 0;
                if !(before || after) {
                    self.uses.push((p as u16, inside));
                }
            }
            i += 1;
        }
    }

    fn undef(&mut self) -> R<()> {
        if let Some(n) = self.line_tok()?.filter(|t| t.kind == IDENT) {
            let b = self.text(&n).to_vec();
            if let Ok(i) = self.find(&b)? {
                self.table[i].as_mut().unwrap().def = NONE;
            }
        }
        self.skip_line()
    }

    fn skip_line(&mut self) -> R<()> {
        while self.line()? {}
        Ok(())
    }
}

/// A string literal's characters, its `\"` and `\\` unescaped (the lean pass's `destringize`):
/// `None` where it is no string literal (a character constant, a raw string).
fn destringize(s: &[u8]) -> Option<Vec<u8>> {
    let q = s.iter().position(|&c| c == b'"' || c == b'\'')?;
    if s[q] != b'"' || s[..q].contains(&b'R') || s.len() < q + 2 {
        return None;
    }
    let body = &s[q + 1..s.len() - 1];
    let mut out = Vec::with_capacity(body.len());
    let mut k = 0;
    while k < body.len() {
        if body[k] == b'\\' && matches!(body.get(k + 1), Some(b'"' | b'\\')) {
            k += 1;
        }
        out.push(body[k]);
        k += 1;
    }
    Some(out)
}

/// A pragma's text (`_Pragma`'s) read as `#pragma`'s tokens are: its first word, and the string
/// literal of `( "…" )` after it.
fn pragma_words(text: &[u8]) -> (Vec<u8>, Option<Vec<u8>>) {
    let blank = |c: u8| c == b' ' || c == b'\t' || c == b'\x0b' || c == b'\x0c';
    let mut k = 0;
    let skip = |k: &mut usize| {
        while *k < text.len() && blank(text[*k]) {
            *k += 1;
        }
    };
    skip(&mut k);
    let w = k;
    while k < text.len() && word_byte(text[k]) {
        k += 1;
    }
    let word = text[w..k].to_vec();
    skip(&mut k);
    if text.get(k) != Some(&b'(') {
        return (word, None);
    }
    k += 1;
    skip(&mut k);
    let s = k;
    while k < text.len() && word_byte(text[k]) {
        k += 1;
    }
    if text.get(k) != Some(&b'"') {
        return (word, None);
    }
    k += 1;
    while k < text.len() && text[k] != b'"' {
        k += if text[k] == b'\\' { 2 } else { 1 };
    }
    if k >= text.len() {
        return (word, None);
    }
    (word, destringize(&text[s..=k]))
}
