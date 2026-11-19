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
    (READ..=EMBED).contains(&opcode) || (SUB..=EXPR).contains(&opcode)
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
#[repr(C)]
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
pub(crate) const B_LINE: u8 = 0;
pub(crate) const B_FILE: u8 = 1;
pub(crate) const B_FILE_NAME: u8 = 2;
pub(crate) const B_BASE_FILE: u8 = 3;
pub(crate) const B_COUNTER: u8 = 4;
pub(crate) const B_INCLUDE_LEVEL: u8 = 5;
pub(crate) const B_DATE: u8 = 6;
pub(crate) const B_TIME: u8 = 7;
pub(crate) const B_TIMESTAMP: u8 = 8;
pub(crate) const B_PRAGMA: u8 = 9;
pub(crate) const B_HAS_INCLUDE: u8 = 10;
pub(crate) const B_HAS_INCLUDE_NEXT: u8 = 11;
pub(crate) const B_HAS_EMBED: u8 = 12;

/// The configuration the built-ins read: `__DATE__`, `__TIME__` and `__TIMESTAMP__`, fixed so
/// runs are deterministic (`c_fixed`), and the `__has_feature` family's answers (`c_feature`:
/// built-in, operand, value; a test not there is 0, but for `__is_identifier`, 1).
#[derive(Clone, Copy)]
pub struct Env<'a> {
    /// The dialect: C++ where set, else C (`PpTables`' C++ rows apply).
    pub cpp: bool,
    pub date: &'a [u8],
    pub time: &'a [u8],
    pub timestamp: &'a [u8],
    pub features: &'a [(&'a [u8], &'a [u8], i64)],
    /// `__COUNTER__`'s first value: a reading of one element (decision 0430) starts from the
    /// count of the uses before it.
    pub counter: u32,
    /// Record each name a lookup does not find (`Expanded::missed`): an element read alone is
    /// given the definitions it reaches, and asks for a name it was not given (one a paste made).
    pub asks: bool,
}

impl Env<'static> {
    /// The lean pass's defaults (`rerail_c_pp::Config::new`).
    pub const DEFAULT: Env<'static> = Env { cpp: false, date: b"Jan  1 1970", time: b"00:00:00", timestamp: b"Thu Jan  1 00:00:00 1970", features: &[], counter: 0, asks: false };
}

/// The built-in index of an `Asked` row that is a name a lookup missed (`Env::asks`).
pub(crate) const MISS: u32 = BUILTINS.len() as u32;

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
/// taken off), `<…>`, `#include_next`, an `#embed`'s (or `__has_embed`'s: its file is data, never
/// lexed), and the file it names (`NONE`: not found). Or a `_Pragma`'s text, and the file of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Named {
    pub name: Vec<u8>,
    pub angle: bool,
    pub next: bool,
    pub embed: bool,
    /// A `_Pragma`'s text, not a header name: `to` is that text as a file of its own, lexed by
    /// the token grammar (the lean pass's `pragma_text`).
    pub pragma: bool,
    pub to: u32,
}

impl Named {
    fn is(&self, name: &[u8], angle: bool, next: bool, embed: bool) -> bool {
        !self.pragma && self.name == name && self.angle == angle && self.next == next && self.embed == embed
    }
}

/// An `#include` being read (`INCLUDE`): how (0 `#include`, 1 `#include_next`, 2 `#import`),
/// and the file it names (`NONE`: none).
#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct Inc {
    pub(crate) how: u32,
    pub(crate) to: u32,
}

/// A file wanted and not found (`Expanded::wanted`'s row): the file that names it, its name's
/// bytes (`wanted_bytes[lo..hi]`), and how it is named.
#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct Want {
    pub(crate) file: u32,
    pub(crate) lo: u32,
    pub(crate) hi: u32,
    pub(crate) angle: bool,
    pub(crate) next: bool,
    pub(crate) embed: bool,
    pub(crate) pragma: bool,
}

/// `#embed`'s state as the compiled words keep it (`Machine::embed` and `em`): the name's
/// parameters' range (`rest`), the name (its bytes, `<…>` or not) and the file mode (a) found
/// (`to`); the parameters read (`Em`'s fields, `limit` where `has_limit`); the digits' place in
/// scratch where they are there.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub(crate) struct EmC {
    pub(crate) rest_lo: u64,
    pub(crate) rest_hi: u64,
    pub(crate) has_name: u64,
    pub(crate) name: u64,
    pub(crate) nname: u64,
    pub(crate) angle: u64,
    pub(crate) has_to: u64,
    pub(crate) to: u64,
    pub(crate) lo: u64,
    pub(crate) i: u64,
    pub(crate) hi: u64,
    pub(crate) p: Tok,
    pub(crate) arg0: u64,
    pub(crate) arg1: u64,
    pub(crate) depth: u64,
    pub(crate) prefix0: u64,
    pub(crate) prefix1: u64,
    pub(crate) suffix0: u64,
    pub(crate) suffix1: u64,
    pub(crate) if_empty0: u64,
    pub(crate) if_empty1: u64,
    pub(crate) has_limit: u64,
    pub(crate) limit: u64,
    pub(crate) file: u64,
    pub(crate) k: u64,
    pub(crate) n: u64,
    pub(crate) a: u64,
    pub(crate) has_digits: u64,
    pub(crate) digits: u64,
}

/// A built-in call being read (`BUILTIN`): which, its name's token, its parentheses' depth.
#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct Bi {
    pub(crate) b: u32,
    pub(crate) name: Tok,
    pub(crate) depth: u32,
}

/// A feature asked and not answered (`Expanded::unanswered`'s row): its built-in (`BUILTINS`),
/// and its operand's bytes (`asked_bytes[lo..hi]`).
#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct Asked {
    pub(crate) b: u32,
    pub(crate) lo: u32,
    pub(crate) hi: u32,
}

/// A feature's answer as the compiled words read it (`Env::features`): its built-in's index
/// (`BUILTINS`, `NONE` for none of them), its operand's bytes, and its value.
#[repr(C)]
pub(crate) struct FeatureC {
    pub(crate) b: u32,
    pub(crate) name: *const u8,
    pub(crate) len: u64,
    pub(crate) value: i64,
}

/// A file's named target as the compiled words read it (`Named`).
#[repr(C)]
pub(crate) struct NamedC {
    pub(crate) name: *const u8,
    pub(crate) len: u64,
    pub(crate) to: u32,
    pub(crate) angle: bool,
    pub(crate) next: bool,
    pub(crate) embed: bool,
    pub(crate) pragma: bool,
}

/// The file includes nest to at most (clang's bound).
pub const MAX_INCLUDE_DEPTH: usize = 200;

thread_local! {
    static HOST_RUNS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The words' runs on the host so far, on this thread: the interpreter's (`run`) and the
/// compiled code's on the CPU (`run_native`). The device's path makes none (0408, L3): it
/// lays a machine out (`prepare`) and reads it back (`complete`) only.
pub fn host_runs() -> u64 {
    HOST_RUNS.with(std::cell::Cell::get)
}

/// The device's rungs of `Bounds` (`Bounds::rung`), the last `Bounds::of`.
pub const RUNGS: usize = 5;

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
    /// The rooms of the calls' lists (`params`, `marks`, `kept`, `argv`, `pres`, `uses`), and of
    /// the lists a TU rarely fills (`exps`, `wanted`, `asked`, and the bytes of the last two):
    /// rooms only, which the interpreter's bounds (`arena`, `output`, `scratch`) are at `of`.
    pub calls: usize,
    pub lists: usize,
    pub list_bytes: usize,
}

impl Bounds {
    /// The bounds the work sets: the arena and output a multiple of the TU's tokens, the table
    /// its directives (each could be a `#define`), the depths clang's nesting limit.
    pub fn of(files: &[File]) -> Bounds {
        let toks: usize = files.iter().map(|f| f.toks.len()).sum();
        let bytes: usize = files.iter().map(|f| f.bytes.len()).sum();
        let hashes = files.iter().flat_map(|f| f.toks.iter()).filter(|t| t.kind == HASH && t.flags & BOL != 0).count();
        // An `#embed` makes two tokens a byte.
        let (arena, output, scratch) = (64 * toks + 2 * bytes + 4096, 64 * toks + 2 * bytes + 4096, 64 * bytes + 4096);
        Bounds { arena, scratch, depth: 256, macros: hashes + 64, output, steps: 4096 * (toks as u64 + 64) + 64 * bytes as u64, calls: arena, lists: output, list_bytes: scratch }
    }

    /// The device's rungs (decision 0348, stage L1), smallest first: rooms from a TU's use, for a
    /// batch of TUs on Metal, where every room is pages taken. A run refused past a room's bound
    /// (`is_room`) below the last rung is run again at the next. The last is `of`, so a TU's
    /// answer, and its refusal at a real bound, are the CPU's. Each rung's tokens a token of the
    /// TU, and its depths; an embedded file (data, no tokens) still makes two tokens a byte.
    pub fn rung(files: &[File], r: usize) -> Bounds {
        const RUNG: [(usize, usize); RUNGS - 1] = [(2, 32), (4, 64), (8, 64), (16, 128)];
        let of = Bounds::of(files);
        let Some(&(per, depth)) = RUNG.get(r) else { return of };
        let toks: usize = files.iter().map(|f| f.toks.len()).sum();
        let bytes: usize = files.iter().map(|f| f.bytes.len()).sum();
        let data: usize = files.iter().filter(|f| f.toks.is_empty()).map(|f| f.bytes.len()).sum();
        let tokens = per * toks + 2 * data + 256;
        Bounds { arena: tokens, scratch: per * bytes + 256, depth, macros: of.macros, output: tokens, steps: of.steps, calls: tokens / 8 + 256, lists: tokens / 16 + 64, list_bytes: per * 256 }
    }

    /// Whether a refusal is a room's: past a store's bound, which a larger rung may hold. The
    /// steps' bound is the work's, the same on every rung.
    pub fn is_room(why: &str) -> bool {
        why.starts_with("past the ") && why.ends_with(" bound") && why != "past the steps' bound"
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
    /// The names lookups did not find, in order, where the run asks (`Env::asks`).
    pub missed: Vec<Vec<u8>>,
    /// `__COUNTER__`'s next value.
    pub counter: u32,
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
pub(crate) const VA_ARGS: &[u8] = b"__VA_ARGS__";
pub(crate) const VA_OPT: &[u8] = b"__VA_OPT__";

/// Whether a word is a test (it runs the next word where it holds, skips it where not).
pub(crate) fn is_test(opcode: u8, mode: u8) -> bool {
    matches!(opcode, LINE | IS | PEEK | LOOKUP | MACRO | DEFINED) || (opcode == INCLUDE && mode <= 1) || (opcode == EMBED && matches!(mode, 0 | 3 | 4 | 5 | 7 | 8)) || (opcode == SUB && matches!(mode, 1..=4 | 6 | 7)) || (opcode == STR && mode == 1) || opcode == GLUE || (opcode == DEFINE && matches!(mode, 0..=3 | 5 | 7)) || (opcode == EXPR && matches!(mode, 0 | 1 | 3 | 6 | 8 | 11 | 14)) || (opcode == BUILTIN && mode >= 1) || (opcode == ARG && (mode == 2 || mode == 4)) || (opcode == PRE && mode == 0) || (opcode == COND && (mode == 2 || mode == 3)) 
}

/// A preprocessing program's rules: only these words, `GOTO`, `PUSH` (each followed by its
/// callee's `GOTO`), `RETURN`, `ACCEPT` and `STOP`; operands in range; no word falls off the end;
/// and every cycle steps a stated cursor, a call counting as one where every way through its
/// callee does. The steps: a token read (`READ`, `LINE`); on the held side of their tests, a
/// body token taken (`SUB m=1`), a `__VA_OPT__`'s content closed (`SUB m=7`), a token
/// stringized (`STR m=1`), a paste's step (`GLUE`), a `#define`'s token read or walked
/// (`DEFINE m=2, 5, 7`), an expression's token (`EXPR m=0, 6, 8, 11`), an `#embed`'s parameter, argument token or byte
/// (`EMBED m=3, 4, 8`). Each is a backend's loop
/// head.
pub fn validate(program: &Words) -> Result<(), Error> {
    let words = &program.words;
    let n = words.len();
    let string = |s: u16| program.strings.get(usize::from(s)).is_some_and(|s| !s.is_empty());
    for (pc, &w) in words.iter().enumerate() {
        let (op, mode, operand) = split(w);
        let ok = match op {
            READ | LINE | LOOKUP | PAINT | PUT | FRAME | KEEP | UNDEF | SKIPLINE | SKIP | DEFINED | PRAGMA => mode == 0 && operand == 0,
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
            SUB => mode <= 10 && operand == 0,
            DEFINE => mode <= 8 && operand == 0,
            EXPR => mode <= 14 && operand == 0,
            EMBED => mode <= 10 && operand == 0,
            STR => mode <= 3 && operand == 0,
            GLUE => mode == 0 && operand == 0,
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
    if program.pp.ops.iter().chain(&program.pp.cpp_ops).any(|&s| program.strings.get(usize::from(s)).is_none_or(|b| !(1..=4).contains(&b.len()))) {
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

/// The tests that step a cursor where they hold (`after`: their no-step successor is the
/// failed one alone). A backend's held branch is the step.
pub(crate) fn steps_where_held(op: u8, mode: u8) -> bool {
    match op {
        // `defined` reads its operand where it holds, and nothing where it does not; a step
        // test steps where it holds.
        DEFINED | GLUE => true,
        SUB => mode == 1 || mode == 7,
        STR => mode == 1,
        // A parameter list's token, a body token read, a use walked.
        DEFINE => matches!(mode, 2 | 5 | 7),
        // An operand's token, a `,`, a binary operator, a `?`.
        EXPR => matches!(mode, 0 | 6 | 8 | 11),
        // An `#embed`'s parameter or argument token, a byte.
        EMBED => matches!(mode, 3 | 4 | 8),
        _ => false,
    }
}

/// The words that can follow `pc` with no step (`validate`).
fn after(words: &[u32], pc: usize, memo: &mut HashMap<usize, Option<bool>>) -> Vec<usize> {
    let (op, mode, operand) = split(words[pc]);
    match op {
        READ | LINE | RETURN | ACCEPT | STOP => vec![],
        _ if steps_where_held(op, mode) => vec![pc + 2],
        GOTO => vec![usize::from(operand)],
        PUSH => {
            let callee = usize::from(split(words[pc + 1]).2);
            if reads(words, callee, memo) { vec![pc + 1] } else { vec![pc + 1, usize::from(operand)] }
        }
        _ if is_test(op, mode) => vec![pc + 1, pc + 2],
        _ => vec![pc + 1],
    }
}

/// Whether every way from `entry` to a `RETURN` steps (a callee still being asked about counts
/// as not).
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
pub(crate) const NO_PARAM: u32 = 0xffff;
/// ...a `__VA_OPT__`.
pub(crate) const VA_OPT_MARK: u32 = 0xfffe;
/// ...and with this bit, a paste operator.
pub(crate) const PASTE_MARK: u32 = 1 << 16;

#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct Src {
    /// The file, or `NONE`: the arena.
    pub(crate) file: u32,
    pub(crate) pos: u32,
    pub(crate) end: u32,
    /// The macro enabled again where it is left, or `NONE`.
    pub(crate) def: u32,
    pub(crate) stop: bool,
    /// A file's: the token index of the `#` of the directive read last.
    pub(crate) hash: u32,
    /// A file's, as `#line` set them: its presumed line less its physical line, and its
    /// presumed name (scratch bytes; `NONE`: its path).
    pub(crate) delta: i32,
    pub(crate) presumed: (u32, u32),
}

impl Src {
    fn file(file: u32, end: u32) -> Src {
        Src { file, pos: 0, end, def: NONE, stop: false, hash: NONE, delta: 0, presumed: (NONE, NONE) }
    }

    fn arena(pos: u32, end: u32, def: u32, stop: bool) -> Src {
        Src { file: NONE, pos, end, def, stop, hash: NONE, delta: 0, presumed: (NONE, NONE) }
    }
}

#[derive(Clone, Copy, Default)]
#[repr(C)]
pub(crate) struct Def {
    pub(crate) fnlike: bool,
    pub(crate) variadic: bool,
    pub(crate) params: u16,
    /// The body: file `file`'s tokens `first..end`, and their marks from `marks`.
    pub(crate) file: u32,
    pub(crate) first: u32,
    pub(crate) end: u32,
    pub(crate) marks: u32,
    /// The arguments to expand before substitution, in the order it asks: `uses[pre.0..pre.1]`.
    pub(crate) pre: (u32, u32),
    pub(crate) disabled: bool,
    /// A built-in's index in `BUILTINS` + 1, or 0.
    pub(crate) builtin: u8,
}

/// A macro table's slot: a name's key, the name, and its definition (`NONE`: undefined). An
/// empty slot's name is of kind `EOF` (`Slot::EMPTY`).
#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct Slot {
    pub(crate) key: u64,
    pub(crate) name: Tok,
    pub(crate) def: u32,
}

impl Slot {
    pub(crate) const EMPTY: Slot = Slot { key: 0, name: Tok { kind: EOF, flags: 0, buf: 0, start: 0, end: 0, exp: 0 }, def: NONE };
}

#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct Frame {
    pub(crate) def: u32,
    pub(crate) name: Tok,
    /// Its arguments: `argv[args..]`; their expansions: `pres[pres..pres + params]`.
    pub(crate) args: u32,
    pub(crate) pres: u32,
    pub(crate) depth: u32,
    pub(crate) next_use: u32,
    /// Its arguments are being read (to its `)`).
    pub(crate) open: bool,
    /// Its last token: its `)`, or its name.
    pub(crate) end: Tok,
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
    table: Vec<Slot>,
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
    inc: Inc,
    /// The files wanted and not found: rows, their names' bytes in `wanted_bytes` (scratch is
    /// cut back after each lookup).
    wanted: Vec<Want>,
    wanted_bytes: Vec<u8>,
    env: &'a Env<'a>,
    exps: Vec<(Tok, Tok)>,
    /// The features asked and not answered: rows, their operands' bytes in `asked_bytes`.
    asked: Vec<Asked>,
    asked_bytes: Vec<u8>,
    counter: u32,
    /// The built-in whose operand is read: its index and name, and the operand's depth.
    bi: Bi,
    /// The line marker's number (`LINEDIR m=0`).
    marker: Tok,
    has_marker: bool,
    /// The directive line's token read last.
    line_last: Tok,
    /// The `#embed` read: its parameters' tokens until they are pushed, its name, and its file
    /// where mode (a) read its name (`Some(NONE)`: not found).
    embed: ((u32, u32), Option<(Spell<'a>, bool)>, Option<u32>),
    sub: Sub,
    def: DefState,
    ex: Ex,
    em: Em,
    /// Where scratch spells the numbers 0 to 255 and `,` (`#embed`'s bytes), once written.
    digits: Option<u32>,
}

/// A spelling: a range of scratch, or the input's bytes (a file's, its path, the environment's).
#[derive(Clone, Copy)]
enum Spell<'a> {
    Scratch(u32, u32),
    Bytes(&'a [u8]),
}

impl<'a> Spell<'a> {
    /// Bytes `a..b` of it.
    fn sub(self, a: usize, b: usize) -> Spell<'a> {
        match self {
            Spell::Scratch(lo, _) => Spell::Scratch(lo + a as u32, lo + b as u32),
            Spell::Bytes(x) => Spell::Bytes(&x[a..b]),
        }
    }
}

/// Tokens in place: a range of the kept tokens, or of the arena.
#[derive(Clone, Copy)]
enum Toks {
    Kept(u32, u32),
    Arena(u32, u32),
}

/// The `#embed` whose parameters are read and bytes pushed (`EMBED m=2` on): its parameters'
/// tokens `kept[lo..hi]` and the next, the parameter read, its argument's start and depth, the
/// arguments kept (ranges of `kept`), `limit`, and the bytes: the file, the next, the end; the
/// tokens pushed start at `a`.
#[derive(Clone, Copy, Default)]
struct Em {
    lo: u32,
    i: u32,
    hi: u32,
    p: Tok,
    arg: (u32, u32),
    depth: u32,
    prefix: (u32, u32),
    suffix: (u32, u32),
    if_empty: (u32, u32),
    limit: Option<usize>,
    file: u32,
    k: u32,
    n: u32,
    a: u32,
}

/// `#if`'s expression being evaluated (`EXPR`): its tokens `kept[lo..hi]` and the next one,
/// the token taken, and whether an error was met (the value is then false; the rules unwind
/// with no more tokens read). Values, the operators pending (an operator and where it was), the
/// binary operands' least precedences, and whether an operand is evaluated (`&&`, `||` and `?:`
/// evaluate only the operands they take).
#[derive(Default)]
#[repr(C)]
pub(crate) struct Ex {
    pub(crate) lo: u32,
    pub(crate) at: u32,
    pub(crate) hi: u32,
    pub(crate) t: Tok,
    pub(crate) err: bool,
    vals: Vec<crate::pp_expr::Value>,
    /// Indices into `OPS`.
    ops: Vec<u8>,
    mins: Vec<u8>,
    live: Vec<bool>,
}

/// `#if`'s operators (an `EXPR` operator's index); `OPS.len()` an operator of no expression
/// (`&=`, `[`).
pub(crate) const OPS: &[&[u8]] = &[b"||", b"&&", b"|", b"^", b"&", b"==", b"!=", b"<", b">", b"<=", b">=", b"<<", b">>", b"+", b"-", b"*", b"/", b"%", b"~", b"!", b"(", b")", b"?", b":", b","];

/// An operator's index in `OPS`.
pub(crate) const fn op_index(op: &[u8]) -> u8 {
    let mut i = 0;
    while i < OPS.len() {
        let o = OPS[i];
        if o.len() == op.len() && (o.len() < 1 || o[0] == op[0]) && (o.len() < 2 || o[1] == op[1]) {
            return i as u8;
        }
        i += 1;
    }
    OPS.len() as u8
}

/// The operator of index `i` (`OPS`), `\0` for one of no expression.
fn op_bytes(i: u8) -> &'static [u8] {
    OPS.get(usize::from(i)).copied().unwrap_or(b"\0")
}

/// The `#define` being read (`DEFINE`).
#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct DefState {
    pub(crate) name: Tok,
    /// Its parameters: `params[p0..]`.
    pub(crate) p0: u32,
    pub(crate) fnlike: bool,
    pub(crate) variadic: bool,
    /// A parameter list: a parameter is wanted next; it closed.
    pub(crate) want: bool,
    pub(crate) closed: bool,
    /// The token after the name or the list, read already (`EOF`: none).
    pub(crate) first: Tok,
    /// The definition made so far (its body's tokens, marks).
    pub(crate) d: Def,
    /// The walk of the uses: the range's start, its next token, its end, and an open
    /// `__VA_OPT__`'s content (the walk after it: its next token and end).
    pub(crate) lo: u32,
    pub(crate) i: u32,
    pub(crate) hi: u32,
    /// `.0` `NONE`: no content open.
    pub(crate) content: (u32, u32),
}

impl Default for DefState {
    fn default() -> Self {
        DefState { name: Tok::default(), p0: 0, fnlike: false, variadic: false, want: false, closed: false, first: eof(0), d: Def::default(), lo: 0, i: 0, hi: 0, content: (NONE, NONE) }
    }
}

/// The substitution being made (`SUB`, `STR`, `GLUE`): one at a time, as a call's arguments
/// are expanded before it begins.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub(crate) struct Sub {
    /// The expansion's index + 1 (`Tok::exp`).
    pub(crate) id: u32,
    /// The body's range walked (all of it, or a `__VA_OPT__`'s content) and its next token.
    pub(crate) lo: u32,
    pub(crate) i: u32,
    pub(crate) hi: u32,
    /// The output's first arena index, and the range's (a content's) being made.
    pub(crate) base: u32,
    pub(crate) out: u32,
    /// The body token taken last (`B`), and its index.
    pub(crate) bt: Tok,
    pub(crate) at: u32,
    /// An open `__VA_OPT__`'s content (`va_open`): the walk after it, its output's start,
    /// whether it is stringized, and its `#` or `__VA_OPT__` token.
    pub(crate) va_open: bool,
    pub(crate) va_lo: u32,
    pub(crate) va_i: u32,
    pub(crate) va_hi: u32,
    pub(crate) va_out: u32,
    pub(crate) va_str: bool,
    pub(crate) va_bt: Tok,
    /// The content closed last: its output's start, whether it is stringized, its token.
    pub(crate) closed_at: u32,
    pub(crate) closed_str: bool,
    pub(crate) closed_bt: Tok,
    /// The pastes: the range's start, the read and write cursors, and the range's end.
    pub(crate) ps: u32,
    pub(crate) r: u32,
    pub(crate) w: u32,
    pub(crate) end: u32,
    /// The range to stringize, its first token, the literal's first scratch byte, and the token
    /// whose flags it takes.
    pub(crate) s: (u32, u32),
    pub(crate) s_first: u32,
    pub(crate) sb: u32,
    pub(crate) like: Tok,
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

/// `0` … `255` spelled one after another, then `,`: `#embed`'s bytes' spellings.
pub(crate) const DIGITS: [u8; 659] = {
    let (mut b, mut i, mut d) = ([0u8; 659], 0, 0u32);
    while d < 256 {
        if d >= 100 {
            b[i] = b'0' + (d / 100) as u8;
            i += 1;
        }
        if d >= 10 {
            b[i] = b'0' + (d / 10 % 10) as u8;
            i += 1;
        }
        b[i] = b'0' + (d % 10) as u8;
        i += 1;
        d += 1;
    }
    b[658] = b',';
    b
};

fn word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

/// Runs a preprocessing program over a TU (file 0 its main file), in bounds.
impl<'a> Machine<'a> {
    /// The machine at a TU's start: the main file its source, the built-ins defined.
    fn new(program: &'a Words, files: &'a [File<'a>], env: &'a Env<'a>, bounds: &'a Bounds) -> Result<Machine<'a>, Refused> {
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
        table: vec![Slot::EMPTY; table],
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
        inc: Inc { how: 0, to: NONE },
        wanted: Vec::new(),
        wanted_bytes: Vec::new(),
        env,
        exps: Vec::new(),
        asked: Vec::new(),
        asked_bytes: Vec::new(),
        counter: env.counter,
        bi: Bi { b: 0, name: eof(0), depth: 0 },
        marker: eof(0),
        has_marker: false,
        line_last: eof(0),
        embed: ((0, 0), None, None),
        sub: Sub::default(),
        def: DefState::default(),
        ex: Ex::default(),
        em: Em::default(),
        digits: None,
    };
    m.install_builtins().map_err(|why| Refused { pc: 0, why })?;
    Ok(m)
    }

    fn finish(self) -> Expanded {
        let bytes = &self.wanted_bytes;
        let wanted = self.wanted.iter().map(|w| (w.file, Named { name: bytes[w.lo as usize..w.hi as usize].to_vec(), angle: w.angle, next: w.next, embed: w.embed, pragma: w.pragma, to: NONE })).collect();
        let bytes = &self.asked_bytes;
        let (miss, asked): (Vec<&Asked>, Vec<&Asked>) = self.asked.iter().partition(|a| a.b == MISS);
        let unanswered = asked.iter().map(|a| (BUILTINS[a.b as usize].0.as_bytes().to_vec(), bytes[a.lo as usize..a.hi as usize].to_vec())).collect();
        let missed = miss.iter().map(|a| bytes[a.lo as usize..a.hi as usize].to_vec()).collect();
        Expanded { toks: self.out, scratch: self.scratch, wanted, exps: self.exps, unanswered, missed, counter: self.counter }
    }
}

/// The compiled words' view of the machine (`native_cpu_pp`): each buffer's start, length and
/// room (its bound; the vector is reserved to it ahead, so a write within the room never
/// moves it), and the scalars the words read. The code reads and writes them in place, and the
/// lengths are given back to the vectors when it returns.
#[repr(C)]
pub(crate) struct Ctx {
    pub files: *const FileC,
    pub nfiles: u64,
    /// A token the compiled words keep aside (a built-in's name while its value is made).
    pub like: Tok,
    /// `#embed`'s state (the compiled words' own).
    pub em: EmC,
    pub sources: *mut Src,
    pub nsources: u64,
    pub room_sources: u64,
    pub arena: *mut Tok,
    pub narena: u64,
    pub room_arena: u64,
    pub out: *mut Tok,
    pub nout: u64,
    pub room_out: u64,
    pub scratch: *mut u8,
    pub nscratch: u64,
    pub room_scratch: u64,
    /// `(u32, bool, bool)`s (`Machine::conds`), at their Rust layout: the code is made in this
    /// process (`COND_*`).
    pub conds: *mut u8,
    pub nconds: u64,
    pub room_conds: u64,
    pub defs: *mut Def,
    pub ndefs: u64,
    pub room_defs: u64,
    pub params: *mut Tok,
    pub nparams: u64,
    pub room_params: u64,
    pub marks: *mut u32,
    pub nmarks: u64,
    pub room_marks: u64,
    /// The `#define` being read (`Machine::def`).
    pub def_state: *mut DefState,
    pub kept: *mut Tok,
    pub nkept: u64,
    pub room_kept: u64,
    pub frames: *mut Frame,
    pub nframes: u64,
    pub room_frames: u64,
    /// `(u32, u32)`s, `(u32, u32)`s, `(u16, bool)`s and `(u16, u32)`s, at their Rust layouts
    /// (`PAIR`, `USE`, `LEVEL`).
    pub argv: *mut u8,
    pub nargv: u64,
    pub room_argv: u64,
    pub pres: *mut u8,
    pub npres: u64,
    pub room_pres: u64,
    pub uses: *mut u8,
    pub nuses: u64,
    pub room_uses: u64,
    pub levels: *mut u8,
    pub nlevels: u64,
    pub room_levels: u64,
    /// The macro table: its slots, and its length less one (a power of two).
    pub table: *mut Slot,
    pub table_mask: u64,
    /// The definition `LOOKUP` found.
    pub d: u64,
    /// `(Tok, Tok)`s, at their Rust layout (`EXP`).
    pub exps: *mut u8,
    pub nexps: u64,
    pub room_exps: u64,
    /// The substitution being made (`Machine::sub`).
    pub sub: *mut Sub,
    /// The `#include` being read, and the files read once (`#pragma once`, `#import`).
    pub inc: *mut Inc,
    pub once: *mut u8,
    /// The files wanted and not found, and their names' bytes.
    pub wanted: *mut Want,
    pub nwanted: u64,
    pub room_wanted: u64,
    pub wanted_bytes: *mut u8,
    pub nwanted_bytes: u64,
    pub room_wanted_bytes: u64,
    /// `#if`'s expression (`Machine::ex`), and its stacks: `Value`s, `OPS` indices, least
    /// precedences, and whether an operand is evaluated.
    pub ex: *mut Ex,
    pub vals: *mut crate::pp_expr::Value,
    pub nvals: u64,
    pub room_vals: u64,
    pub ops: *mut u8,
    pub nops: u64,
    pub room_ops: u64,
    pub mins: *mut u8,
    pub nmins: u64,
    pub room_mins: u64,
    pub live: *mut u8,
    pub nlive: u64,
    pub room_live: u64,
    pub returns: *mut u32,
    pub nreturns: u64,
    pub room_returns: u64,
    pub collecting: u64,
    /// The dialect (`Env::cpp`): 1 C++.
    pub cpp: u64,
    pub t: Tok,
    pub line_last: Tok,
    /// The built-in call being read, `__COUNTER__`'s count, and a line marker's number token.
    pub bi: *mut Bi,
    pub counter: u64,
    pub marker: Tok,
    pub has_marker: u8,
    /// The features asked and not answered, and their operands' bytes.
    pub asked: *mut Asked,
    pub nasked: u64,
    pub room_asked: u64,
    pub asked_bytes: *mut u8,
    pub nasked_bytes: u64,
    pub room_asked_bytes: u64,
    /// The environment: `__DATE__`'s, `__TIME__`'s and `__TIMESTAMP__`'s bytes, the features.
    pub date: *const u8,
    pub ndate: u64,
    pub time: *const u8,
    pub ntime: u64,
    pub timestamp: *const u8,
    pub ntimestamp: u64,
    pub features: *const FeatureC,
    pub nfeatures: u64,
    /// `Env::asks`: 1, record the names lookups miss.
    pub asks: u64,
    /// `#pragma`'s first three tokens (the compiled words' own).
    pub pragma_toks: [Tok; 3],
    /// `push_macro`'s stack: `(u32, u32)`s (`PAIR`), a slot and its definition.
    pub pushed: *mut u8,
    pub npushed: u64,
    pub room_pushed: u64,
    pub from_file: u8,
    pub in_if: u8,
}

/// A file as the compiled words read it.
#[repr(C)]
pub(crate) struct FileC {
    pub toks: *const Tok,
    pub ntoks: u64,
    pub bytes: *const u8,
    pub nbytes: u64,
    /// Its `#` chains (`File::chains`): `(u32, u32)`s (`PAIR`), sorted by the first.
    pub chains: *const u8,
    pub nchains: u64,
    /// Its includes mode (a) resolved (`File::includes`): `(u32, u32)`s (`PAIR`), rising.
    pub includes: *const u8,
    pub nincludes: u64,
    /// Its includes named by expanding their line (`File::named`).
    pub named: *const NamedC,
    pub nnamed: u64,
    /// Its newlines' offsets, and its path (`File::lines`, `File::path`).
    pub lines: *const u32,
    pub nlines: u64,
    pub path: *const u8,
    pub npath: u64,
}

/// A conditional's layout (`Machine::conds`): its size and its file source's offset.
pub(crate) const COND_SIZE: usize = std::mem::size_of::<(u32, bool, bool)>();
pub(crate) const COND_SRC: usize = std::mem::offset_of!((u32, bool, bool), 0);
/// Its chain taken, and its `#else` seen.
pub(crate) const COND_TAKEN: usize = std::mem::offset_of!((u32, bool, bool), 1);
pub(crate) const COND_ELSE: usize = std::mem::offset_of!((u32, bool, bool), 2);
/// An argument's or an expansion's range: its size, and its two ends' offsets.
pub(crate) const PAIR: (usize, usize, usize) = (std::mem::size_of::<(u32, u32)>(), std::mem::offset_of!((u32, u32), 0), std::mem::offset_of!((u32, u32), 1));
/// An argument use: its size, its parameter's offset, its "inside `__VA_OPT__`"'s.
pub(crate) const USE: (usize, usize, usize) = (std::mem::size_of::<(u16, bool)>(), std::mem::offset_of!((u16, bool), 0), std::mem::offset_of!((u16, bool), 1));
/// An expansion's name and end tokens: its size, and their offsets.
pub(crate) const EXP: (usize, usize, usize) = (std::mem::size_of::<(Tok, Tok)>(), std::mem::offset_of!((Tok, Tok), 0), std::mem::offset_of!((Tok, Tok), 1));
/// A level: its size, its parameter's offset, its start's in `kept`.
pub(crate) const LEVEL: (usize, usize, usize) = (std::mem::size_of::<(u16, u32)>(), std::mem::offset_of!((u16, u32), 0), std::mem::offset_of!((u16, u32), 1));

/// Runs compiled words (`native_cpu_pp`): `code` over the machine's buffers, the result `0`
/// (accepted) or `pc + 1` and a reason's index into `reasons` above bit 32.
///
/// # Safety
/// `code` reads and writes only within the `Ctx` it is given: each buffer below its room.
pub(crate) unsafe fn run_native(program: &Words, files: &[File], env: &Env, bounds: &Bounds, code: unsafe extern "C" fn(*mut Ctx) -> u64, reasons: &[&'static str]) -> Result<Expanded, Refused> {
    HOST_RUNS.with(|n| n.set(n.get() + 1));
    let mut p = prepare(program, files, env, bounds, &mut |b: Buf| b.ptr as u64)?;
    // SAFETY: the caller's.
    let r = unsafe { code(&mut p.c) };
    // SAFETY: the code wrote each buffer below its room, which its vector has reserved.
    unsafe { complete(p, r, reasons) }
}

/// A buffer the compiled words reach, as `prepare` hands it to a placement: its start, its
/// bytes written so far, its bytes reserved (its room), and whether the words write it.
#[derive(Clone, Copy, Debug)]
pub struct Buf {
    pub ptr: *mut u8,
    pub len: usize,
    pub room: usize,
    pub write: bool,
}

/// A TU's machine laid out for compiled words: the machine, the context the code reads (each
/// buffer's address where the placement put it), and what the context points at.
pub struct Prepared<'a> {
    m: Box<Machine<'a>>,
    pub(crate) c: Ctx,
    _keep: (Vec<Vec<NamedC>>, Vec<FileC>, Vec<FeatureC>, Vec<u32>),
}

impl Prepared<'_> {
    /// The context's bytes, to copy where the code runs.
    pub fn ctx_bytes(&self) -> &[u8] {
        // The device lays contexts out by 8-byte slots (0408, L2).
        const _: () = assert!(std::mem::size_of::<Ctx>() % 8 == 0);
        // SAFETY: `Ctx` is plain old data (`repr(C)`), read as bytes.
        unsafe { std::slice::from_raw_parts((&raw const self.c).cast::<u8>(), std::mem::size_of::<Ctx>()) }
    }

    /// The context back from where the code ran (its counts and scalars).
    ///
    /// # Safety
    /// `bytes` must be this context as the code left it.
    pub unsafe fn ctx_from(&mut self, bytes: &[u8]) {
        assert_eq!(bytes.len(), std::mem::size_of::<Ctx>());
        // SAFETY: the caller's; `Ctx` is plain old data.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), (&raw mut self.c).cast::<u8>(), bytes.len()) };
    }
}

/// A TU's machine laid out for compiled words: each buffer the code reaches handed to `place`,
/// which gives the address the code finds it at (on the CPU its own; on a device, a copy's).
pub fn prepare<'a>(program: &'a Words, files: &'a [File<'a>], env: &'a Env<'a>, bounds: &'a Bounds, place: &mut dyn FnMut(Buf) -> u64) -> Result<Prepared<'a>, Refused> {
    let mut m = Box::new(Machine::new(program, files, env, bounds)?);
    // A read-only slice, and a vector the words write (its room reserved, its written part).
    fn ro<T>(place: &mut dyn FnMut(Buf) -> u64, v: &[T]) -> u64 {
        let n = std::mem::size_of_val(v);
        place(Buf { ptr: v.as_ptr().cast_mut().cast(), len: n, room: n, write: false })
    }
    fn rw<T>(place: &mut dyn FnMut(Buf) -> u64, v: &mut Vec<T>, room: u64, extra: usize) -> u64 {
        let size = std::mem::size_of::<T>();
        place(Buf { ptr: v.as_mut_ptr().cast(), len: v.len() * size, room: (room as usize * size + extra).max(v.len() * size), write: true })
    }
    fn one<T>(place: &mut dyn FnMut(Buf) -> u64, x: &mut T) -> u64 {
        let n = std::mem::size_of::<T>();
        place(Buf { ptr: (x as *mut T).cast(), len: n, room: n, write: true })
    }
    let named: Vec<Vec<NamedC>> = files.iter().map(|f| f.named.iter().map(|n| NamedC { name: ro(place, &n.name) as *const u8, len: n.name.len() as u64, to: n.to, angle: n.angle, next: n.next, embed: n.embed, pragma: n.pragma }).collect()).collect();
    let filesc: Vec<FileC> = files
        .iter()
        .zip(&named)
        .map(|(f, n)| FileC {
            toks: ro(place, f.toks) as *const Tok,
            ntoks: f.toks.len() as u64,
            bytes: ro(place, f.bytes) as *const u8,
            nbytes: f.bytes.len() as u64,
            chains: ro(place, f.chains) as *const u8,
            nchains: f.chains.len() as u64,
            includes: ro(place, f.includes) as *const u8,
            nincludes: f.includes.len() as u64,
            named: ro(place, n) as *const NamedC,
            nnamed: n.len() as u64,
            lines: ro(place, f.lines) as *const u32,
            nlines: f.lines.len() as u64,
            path: ro(place, f.path) as *const u8,
            npath: f.path.len() as u64,
        })
        .collect();
    fn room<T>(v: &mut Vec<T>, n: usize) -> u64 {
        v.reserve_exact(n.saturating_sub(v.len()));
        n as u64
    }
    // A byte buffer: its room, and eight bytes more of capacity, so the words store a run's last
    // bytes as a whole word (`native_cpu_pp`'s `store_bytes`).
    fn room_bytes(v: &mut Vec<u8>, n: usize) -> u64 {
        v.reserve_exact((n + 8).saturating_sub(v.len()));
        n as u64
    }
    let mut returns = vec![0u32; bounds.depth];
    let features: Vec<FeatureC> = env
        .features
        .iter()
        .map(|f| FeatureC { b: BUILTINS.iter().position(|b| b.0.as_bytes() == f.0).map_or(NONE, |i| i as u32), name: ro(place, &f.1) as *const u8, len: f.1.len() as u64, value: f.2 })
        .collect();
    // Each vector's room reserved first, then placed.
    let rooms = [
        room(&mut m.sources, bounds.depth), room(&mut m.arena, bounds.arena), room(&mut m.out, bounds.output), room_bytes(&mut m.scratch, bounds.scratch),
        room(&mut m.conds, bounds.depth * 16), room(&mut m.defs, bounds.macros * 4), room(&mut m.params, bounds.calls), room(&mut m.marks, bounds.calls),
        room(&mut m.kept, bounds.calls), room(&mut m.frames, bounds.depth), room(&mut m.argv, bounds.calls), room(&mut m.pres, bounds.calls),
        room(&mut m.uses, bounds.calls), room(&mut m.levels, bounds.depth), room(&mut m.exps, bounds.lists), room(&mut m.wanted, bounds.lists),
        room_bytes(&mut m.wanted_bytes, bounds.list_bytes), room(&mut m.ex.vals, bounds.depth * 4), room(&mut m.ex.ops, bounds.depth * 4), room(&mut m.ex.mins, bounds.depth * 4),
        room(&mut m.ex.live, bounds.depth * 4), room(&mut m.asked, bounds.lists), room_bytes(&mut m.asked_bytes, bounds.list_bytes), room(&mut m.pushed, bounds.macros * 4),
    ];
    let c = Ctx {
        files: ro(place, &filesc) as *const FileC,
        nfiles: filesc.len() as u64,
        like: Tok::default(),
        em: EmC { has_digits: u64::from(m.digits.is_some()), digits: u64::from(m.digits.unwrap_or(0)), ..EmC::default() },
        room_sources: rooms[0],
        sources: rw(place, &mut m.sources, rooms[0], 0) as *mut Src,
        nsources: m.sources.len() as u64,
        room_arena: rooms[1],
        arena: rw(place, &mut m.arena, rooms[1], 0) as *mut Tok,
        narena: m.arena.len() as u64,
        room_out: rooms[2],
        out: rw(place, &mut m.out, rooms[2], 0) as *mut Tok,
        nout: m.out.len() as u64,
        room_scratch: rooms[3],
        scratch: rw(place, &mut m.scratch, rooms[3], 8) as *mut u8,
        nscratch: m.scratch.len() as u64,
        room_conds: rooms[4],
        conds: rw(place, &mut m.conds, rooms[4], 0) as *mut u8,
        nconds: m.conds.len() as u64,
        room_defs: rooms[5],
        defs: rw(place, &mut m.defs, rooms[5], 0) as *mut Def,
        ndefs: m.defs.len() as u64,
        room_params: rooms[6],
        params: rw(place, &mut m.params, rooms[6], 0) as *mut Tok,
        nparams: m.params.len() as u64,
        room_marks: rooms[7],
        marks: rw(place, &mut m.marks, rooms[7], 0) as *mut u32,
        nmarks: m.marks.len() as u64,
        def_state: one(place, &mut m.def) as *mut DefState,
        room_kept: rooms[8],
        kept: rw(place, &mut m.kept, rooms[8], 0) as *mut Tok,
        nkept: m.kept.len() as u64,
        room_frames: rooms[9],
        frames: rw(place, &mut m.frames, rooms[9], 0) as *mut Frame,
        nframes: m.frames.len() as u64,
        room_argv: rooms[10],
        argv: rw(place, &mut m.argv, rooms[10], 0) as *mut u8,
        nargv: m.argv.len() as u64,
        room_pres: rooms[11],
        pres: rw(place, &mut m.pres, rooms[11], 0) as *mut u8,
        npres: m.pres.len() as u64,
        room_uses: rooms[12],
        uses: rw(place, &mut m.uses, rooms[12], 0) as *mut u8,
        nuses: m.uses.len() as u64,
        room_levels: rooms[13],
        levels: rw(place, &mut m.levels, rooms[13], 0) as *mut u8,
        nlevels: m.levels.len() as u64,
        table_mask: m.table.len() as u64 - 1,
        table: { let n = m.table.len() as u64; rw(place, &mut m.table, n, 0) as *mut Slot },
        d: u64::from(m.d),
        room_exps: rooms[14],
        exps: rw(place, &mut m.exps, rooms[14], 0) as *mut u8,
        nexps: m.exps.len() as u64,
        sub: one(place, &mut m.sub) as *mut Sub,
        inc: one(place, &mut m.inc) as *mut Inc,
        once: { let n = m.once.len() as u64; rw(place, &mut m.once, n, 0) as *mut u8 },
        room_wanted: rooms[15],
        wanted: rw(place, &mut m.wanted, rooms[15], 0) as *mut Want,
        nwanted: m.wanted.len() as u64,
        room_wanted_bytes: rooms[16],
        wanted_bytes: rw(place, &mut m.wanted_bytes, rooms[16], 8) as *mut u8,
        nwanted_bytes: m.wanted_bytes.len() as u64,
        room_vals: rooms[17],
        vals: rw(place, &mut m.ex.vals, rooms[17], 0) as *mut crate::pp_expr::Value,
        nvals: m.ex.vals.len() as u64,
        room_ops: rooms[18],
        ops: rw(place, &mut m.ex.ops, rooms[18], 0) as *mut u8,
        nops: m.ex.ops.len() as u64,
        room_mins: rooms[19],
        mins: rw(place, &mut m.ex.mins, rooms[19], 0) as *mut u8,
        nmins: m.ex.mins.len() as u64,
        room_live: rooms[20],
        live: rw(place, &mut m.ex.live, rooms[20], 0) as *mut u8,
        nlive: m.ex.live.len() as u64,
        ex: one(place, &mut m.ex) as *mut Ex,
        returns: { let n = returns.len() as u64; rw(place, &mut returns, n, 0) as *mut u32 },
        nreturns: 0,
        room_returns: bounds.depth as u64,
        collecting: u64::from(m.collecting),
        cpp: u64::from(env.cpp),
        t: m.t,
        line_last: m.line_last,
        pragma_toks: [Tok::default(); 3],
        bi: one(place, &mut m.bi) as *mut Bi,
        counter: u64::from(m.counter),
        marker: m.marker,
        has_marker: u8::from(m.has_marker),
        room_asked: rooms[21],
        asked: rw(place, &mut m.asked, rooms[21], 0) as *mut Asked,
        nasked: m.asked.len() as u64,
        room_asked_bytes: rooms[22],
        asked_bytes: rw(place, &mut m.asked_bytes, rooms[22], 8) as *mut u8,
        nasked_bytes: m.asked_bytes.len() as u64,
        date: ro(place, env.date) as *const u8,
        ndate: env.date.len() as u64,
        time: ro(place, env.time) as *const u8,
        ntime: env.time.len() as u64,
        timestamp: ro(place, env.timestamp) as *const u8,
        ntimestamp: env.timestamp.len() as u64,
        features: ro(place, &features) as *const FeatureC,
        nfeatures: features.len() as u64,
        asks: u64::from(env.asks),
        room_pushed: rooms[23],
        pushed: rw(place, &mut m.pushed, rooms[23], 0) as *mut u8,
        npushed: m.pushed.len() as u64,
        from_file: u8::from(m.from_file),
        in_if: u8::from(m.in_if),
    };
    // The machine's box moves in, its heap place kept: the pointers into it stay good.
    Ok(Prepared { m, c, _keep: (named, filesc, features, returns) })
}

/// A TU's run finished: the counts the code left in the context given back to the machine's
/// vectors (each buffer placed back where it was), then its result, `r` the code's.
///
/// # Safety
/// The code must have written each buffer below its room, and its context be `p.c`.
pub unsafe fn complete(p: Prepared<'_>, r: u64, reasons: &[&'static str]) -> Result<Expanded, Refused> {
    let Prepared { mut m, c, _keep } = p;
    // SAFETY: the caller's: each count is within its vector's reserved room.
    unsafe {
        m.sources.set_len(c.nsources as usize);
        m.arena.set_len(c.narena as usize);
        m.out.set_len(c.nout as usize);
        m.scratch.set_len(c.nscratch as usize);
        m.conds.set_len(c.nconds as usize);
        m.kept.set_len(c.nkept as usize);
        m.frames.set_len(c.nframes as usize);
        m.defs.set_len(c.ndefs as usize);
        m.params.set_len(c.nparams as usize);
        m.marks.set_len(c.nmarks as usize);
        m.argv.set_len(c.nargv as usize);
        m.pres.set_len(c.npres as usize);
        m.uses.set_len(c.nuses as usize);
        m.levels.set_len(c.nlevels as usize);
        m.exps.set_len(c.nexps as usize);
        m.wanted.set_len(c.nwanted as usize);
        m.pushed.set_len(c.npushed as usize);
        m.asked.set_len(c.nasked as usize);
        m.asked_bytes.set_len(c.nasked_bytes as usize);
        m.wanted_bytes.set_len(c.nwanted_bytes as usize);
        m.ex.vals.set_len(c.nvals as usize);
        m.ex.ops.set_len(c.nops as usize);
        m.ex.mins.set_len(c.nmins as usize);
        m.ex.live.set_len(c.nlive as usize);
    }
    (m.collecting, m.t, m.from_file, m.in_if, m.d, m.line_last) = (c.collecting as u32, c.t, c.from_file != 0, c.in_if != 0, c.d as u32, c.line_last);
    (m.counter, m.marker, m.has_marker) = (c.counter as u32, c.marker, c.has_marker != 0);
    if r == 0 {
        return Ok(m.finish());
    }
    Err(Refused { pc: (r & 0xffff_ffff) as usize - 1, why: reasons.get((r >> 32) as usize).copied().unwrap_or("the compiled words refuse") })
}

pub fn run(program: &Words, files: &[File], env: &Env, bounds: &Bounds) -> Result<Expanded, Refused> {
    HOST_RUNS.with(|n| n.set(n.get() + 1));
    let mut m = Machine::new(program, files, env, bounds)?;
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
            ACCEPT => return Ok(m.finish()),
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
            IF => {
                m.expr_begin().map_err(fail)?;
                None
            }
            EXPR => m.expr(mode).map_err(fail)?,
            INCLUDE => m.include(mode).map_err(fail)?,
            PRAGMA => {
                m.pragma().map_err(fail)?;
                None
            }
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
            EMBED => m.embed(mode).map_err(fail)?,
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
            SUB => m.sub(mode).map_err(fail)?,
            STR => m.str_word(mode).map_err(fail)?,
            GLUE => Some(m.paste_step().map_err(fail)?),
            DEFINE => m.define(mode).map_err(fail)?,
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

impl<'a> Machine<'a> {
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
            let s = &self.table[i];
            if s.name.kind == EOF {
                return Ok(Err(i));
            }
            if s.key == k && self.text(&s.name) == b {
                return Ok(Ok(i));
            }
            i = (i + 1) & mask;
        }
        Err("past the macro table's bound")
    }

    fn lookup(&mut self) -> R<bool> {
        if self.t.kind != IDENT || self.t.flags & NOEXP != 0 {
            return Ok(false);
        }
        Ok(match self.find(self.text(&self.t))? {
            Ok(i) if self.table[i].def != NONE => {
                self.d = self.table[i].def;
                true
            }
            _ => {
                self.missed(self.t)?;
                false
            }
        })
    }

    /// Name `n` not found: recorded where the run asks (`Env::asks`), as a row of `MISS`.
    fn missed(&mut self, n: Tok) -> R<()> {
        if !self.env.asks {
            return Ok(());
        }
        let len = (n.end - n.start) as usize;
        if self.asked.len() >= self.bounds.output || self.asked_bytes.len() + len > self.bounds.scratch {
            return Err("past the unanswered features' bound");
        }
        let lo = self.asked_bytes.len() as u32;
        let (a, b) = (n.start as usize, n.end as usize);
        if n.buf == 0 {
            self.asked_bytes.extend_from_slice(&self.scratch[a..b]);
        } else {
            let files = self.files;
            self.asked_bytes.extend_from_slice(&files[n.buf as usize - 1].bytes[a..b]);
        }
        self.asked.push(Asked { b: MISS, lo, hi: self.asked_bytes.len() as u32 });
        Ok(())
    }

    fn frame(&mut self) -> R<()> {
        if self.frames.len() >= self.bounds.depth {
            return Err("past the frames' bound");
        }
        let def = self.defs[self.d as usize];
        self.frames.push(Frame { def: self.d, name: self.t, args: self.argv.len() as u32, pres: self.pres.len() as u32, depth: 0, next_use: 0, open: def.fnlike, end: self.t });
        self.collecting += u32::from(def.fnlike);
        if self.pres.len() + usize::from(def.params) > self.bounds.arena {
            return Err("past the arena's bound");
        }
        self.pres.extend(std::iter::repeat_n((NONE, NONE), usize::from(def.params)));
        if def.fnlike {
            let a = self.arena.len() as u32;
            self.push_argv((a, a))?;
        }
        Ok(())
    }

    /// An argument opened: its range in the arena.
    fn push_argv(&mut self, a: (u32, u32)) -> R<()> {
        if self.argv.len() >= self.bounds.arena {
            return Err("past the arena's bound");
        }
        self.argv.push(a);
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
                    self.push_argv((a, a))?;
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
                    self.push_argv((a, a))?;
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


    fn scratch_tok(&mut self, kind: u8, text: &[u8], flags: u8) -> R<Tok> {
        if self.scratch.len() + text.len() > self.bounds.scratch {
            return Err("past the scratch bytes' bound");
        }
        let start = self.scratch.len() as u32;
        self.scratch.extend_from_slice(text);
        Ok(Tok { kind, flags, buf: 0, start, end: self.scratch.len() as u32, exp: 0 })
    }

    /// `l ## r`: one token, or `None` where the two spell no one token.
    fn paste(&mut self, l: Tok, r: Tok, id: u32) -> R<Option<Tok>> {
        if r.flags & PLACEMARKER != 0 {
            return Ok(Some(l));
        }
        if l.flags & PLACEMARKER != 0 {
            return Ok(Some(Tok { flags: (r.flags & !(BOL | SPACE)) | (l.flags & (BOL | SPACE)), ..r }));
        }
        // The two spelled together in scratch: the token's spelling, where they make one.
        let s0 = self.scratch.len();
        self.scratch_copy(&l, false)?;
        let m = self.scratch.len();
        self.scratch_copy(&r, false)?;
        let kind = self.pasted_kind(&l, &r, &self.scratch[s0..m], &self.scratch[m..], &self.scratch[s0..]);
        let Ok(Some(kind)) = kind else {
            self.scratch.truncate(s0);
            return kind.map(|_| None);
        };
        Ok(Some(Tok { kind, flags: l.flags & (BOL | SPACE), buf: 0, start: s0 as u32, end: self.scratch.len() as u32, exp: id }))
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
            (LITERAL, IDENT) => (self.program.pp.suffix && self.env.cpp).then_some(LITERAL),
            (a, b) if op_like(a) && op_like(b) => {
                let strings = &self.program.strings;
                let cpp_ops: &[u16] = if self.env.cpp { &self.program.pp.cpp_ops } else { &[] };
                self.program.pp.ops.iter().chain(cpp_ops).any(|&s| strings[usize::from(s)] == text).then(|| match text {
                    b"#" | b"%:" => HASH,
                    b"##" | b"%:%:" => HASHHASH,
                    b"<:" | b":>" | b"<%" | b"%>" => DIGRAPH,
                    _ => PUNCT,
                })
            }
            _ => None,
        })
    }

    // The substitution.

    fn sub(&mut self, mode: u8) -> R<Option<bool>> {
        let f = *self.frames.last().ok_or("a substitution with no frame")?;
        let def = self.defs[f.def as usize];
        let v = self.sub;
        let held = match mode {
            0 => {
                if self.exps.len() >= self.bounds.output {
                    return Err("past the expansions' bound");
                }
                self.exps.push((f.name, f.end));
                let a = self.arena.len() as u32;
                self.sub = Sub { id: self.exps.len() as u32, hi: def.end - def.first, base: a, out: a, ..Sub::default() };
                return Ok(None);
            }
            1 => {
                if v.i >= v.hi {
                    return Ok(Some(false));
                }
                self.sub.bt = self.body_tok(&def, v.i);
                self.sub.at = v.i;
                self.sub.i += 1;
                true
            }
            2 => match (def.fnlike && v.bt.kind == HASH && v.at + 1 < v.hi).then(|| self.param(&def, v.at + 1)).flatten() {
                Some(p) => {
                    let (a, b) = self.arg_range(&f, p);
                    self.sub.i = v.at + 2;
                    self.sub.s = (a, b);
                    self.sub.like = v.bt;
                    true
                }
                None => false,
            },
            3 | 4 => {
                // `#__VA_OPT__(…)` (3), `__VA_OPT__(…)` (4).
                let k = if mode == 3 { v.at + 1 } else { v.at };
                let is = def.variadic && k < v.hi && self.mark(&def, k) == VA_OPT_MARK && (mode == 4 || (def.fnlike && v.bt.kind == HASH));
                if !is {
                    return Ok(Some(false));
                }
                if self.sub.va_open {
                    return Err("a __VA_OPT__ inside a __VA_OPT__");
                }
                let (clo, chi, end) = self.va_opt_content(&def, k, v.hi);
                let present = self.va_present(&f);
                if mode == 4 && !present {
                    self.push_arena(Tok { flags: PLACEMARKER, ..v.bt })?;
                    self.sub.i = end;
                    return Ok(Some(true));
                }
                let a = self.arena.len() as u32;
                (self.sub.va_open, self.sub.va_lo, self.sub.va_i, self.sub.va_hi, self.sub.va_out, self.sub.va_str, self.sub.va_bt) = (true, v.lo, end, v.hi, v.out, mode == 3, v.bt);
                // Not present, stringized: its content is empty, its literal `""`.
                let (lo, hi) = if present { (clo, chi) } else { (chi, chi) };
                (self.sub.lo, self.sub.i, self.sub.hi, self.sub.out) = (lo, lo, hi, a);
                true
            }
            5 => {
                self.substitute_one(&f, &def)?;
                return Ok(None);
            }
            6 => {
                let (at, stringized) = (v.closed_at, v.closed_str);
                if stringized {
                    (self.sub.ps, self.sub.r, self.sub.w, self.sub.end) = (at, at, at, self.arena.len() as u32);
                }
                stringized
            }
            7 => {
                if !v.va_open {
                    return Ok(Some(false));
                }
                let at = v.out;
                if !v.va_str && (at as usize) < self.arena.len() {
                    let x = &mut self.arena[at as usize];
                    x.flags = (x.flags & !SPACE) | (v.va_bt.flags & SPACE);
                }
                self.sub = Sub { lo: v.va_lo, i: v.va_i, hi: v.va_hi, out: v.va_out, va_open: false, closed_at: at, closed_str: v.va_str, closed_bt: v.va_bt, ..v };
                true
            }
            8 => {
                let at = v.closed_at;
                self.retain_from(at);
                self.sub.s = (at, self.arena.len() as u32);
                self.sub.like = v.closed_bt;
                return Ok(None);
            }
            9 => {
                (self.sub.ps, self.sub.r, self.sub.w, self.sub.end) = (v.base, v.base, v.base, self.arena.len() as u32);
                return Ok(None);
            }
            _ => {
                self.retain_from(v.base);
                if let Some(x) = self.arena.get_mut(v.base as usize) {
                    x.flags = (x.flags & !(BOL | SPACE)) | (f.name.flags & (BOL | SPACE));
                }
                self.drop_frame();
                if self.sources.len() >= self.bounds.depth {
                    return Err("past the sources' bound");
                }
                self.defs[f.def as usize].disabled = true;
                self.sources.push(Src::arena(v.base, self.arena.len() as u32, f.def, false));
                return Ok(None);
            }
        };
        Ok(Some(held))
    }

    /// `B` substituted (`SUB m=5`): a parameter's argument, or `B` itself.
    fn substitute_one(&mut self, f: &Frame, def: &Def) -> R<()> {
        let v = self.sub;
        let (bt, i) = (v.bt, v.at);
        let Some(p) = self.param(def, i) else {
            return self.push_arena(Tok { exp: v.id, ..bt });
        };
        let va = def.variadic.then(|| usize::from(def.params) - 1);
        let before = i > v.lo && self.body_tok(def, i - 1).flags & PASTE != 0;
        let after = i + 1 < v.hi && self.body_tok(def, i + 1).flags & PASTE != 0;
        let raw = self.arg_range(f, p);
        let n = self.arena.len();
        let comma_paste = n >= v.out as usize + 2 && self.spelled(&self.arena[n - 2], b",") && self.arena[n - 2].kind == PUNCT && self.arena[n - 1].flags & PASTE != 0;
        // GNU's `, ## __VA_ARGS__`: with no variadic arguments the comma goes...
        if before && Some(p) == va && raw.0 == raw.1 && comma_paste {
            self.arena.truncate(n - 2);
            return Ok(());
        }
        // ...and with some, the `##` is no paste.
        if before && Some(p) == va && comma_paste {
            self.arena.pop();
        }
        let (a, b) = if before || after {
            raw
        } else {
            let r = self.pres[f.pres as usize + p];
            if r.0 == NONE {
                return Err("an argument substituted before its expansion");
            }
            r
        };
        if a == b {
            return self.push_arena(Tok { flags: PLACEMARKER, ..bt });
        }
        let k = self.arena.len();
        for x in a..b {
            let t = self.arena[x as usize];
            self.push_arena(t)?;
        }
        self.arena[k].flags = (self.arena[k].flags & !(BOL | SPACE)) | (bt.flags & SPACE);
        Ok(())
    }

    /// The arena's placemarkers from `at` on dropped.
    fn retain_from(&mut self, at: u32) {
        let mut w = at as usize;
        for r in at as usize..self.arena.len() {
            if self.arena[r].flags & PLACEMARKER == 0 {
                self.arena[w] = self.arena[r];
                w += 1;
            }
        }
        self.arena.truncate(w);
    }

    /// One step of the pastes of `arena[r..end]`, written from `w` (the lean pass's `paste_all`,
    /// left to right: a `##` joins the token written last and the one after it).
    fn paste_step(&mut self) -> R<bool> {
        let v = self.sub;
        if v.r >= v.end {
            self.arena.truncate(v.w as usize);
            return Ok(false);
        }
        let t = self.arena[v.r as usize];
        if t.flags & PASTE != 0 && v.w > v.ps && v.r + 1 < v.end {
            let l = self.arena[v.w as usize - 1];
            let r = self.arena[v.r as usize + 1];
            match self.paste(l, r, v.id)? {
                Some(m) => self.arena[v.w as usize - 1] = m,
                None => {
                    self.arena[v.w as usize] = r;
                    self.sub.w += 1;
                }
            }
            self.sub.r += 2;
            return Ok(true);
        }
        self.arena[v.w as usize] = t;
        self.sub.w += 1;
        self.sub.r += 1;
        Ok(true)
    }

    fn str_word(&mut self, mode: u8) -> R<Option<bool>> {
        let v = self.sub;
        match mode {
            0 => {
                self.sub.s_first = v.s.0;
                self.sub.sb = self.scratch.len() as u32;
                self.scratch_push(b"\"")?;
                Ok(None)
            }
            1 => {
                if v.s.0 >= v.s.1 {
                    return Ok(Some(false));
                }
                let t = self.arena[v.s.0 as usize];
                self.sub.s.0 += 1;
                if t.flags & PLACEMARKER != 0 {
                    return Ok(Some(true));
                }
                if v.s.0 > v.s_first && t.flags & (SPACE | BOL) != 0 && self.scratch.len() as u32 > v.sb + 1 {
                    self.scratch_push(b" ")?;
                }
                self.scratch_copy(&t, t.kind == LITERAL)?;
                Ok(Some(true))
            }
            _ => {
                self.scratch_push(b"\"")?;
                if mode == 3 {
                    self.arena.truncate(v.s_first as usize);
                }
                let t = Tok { kind: LITERAL, flags: v.like.flags & (BOL | SPACE), buf: 0, start: v.sb, end: self.scratch.len() as u32, exp: v.id };
                self.push_arena(t)?;
                Ok(None)
            }
        }
    }

    /// Token `t`'s spelling appended to scratch, with `"` and `\\` escaped where `escape` (a
    /// literal stringized): the runs between them copied whole.
    fn scratch_copy(&mut self, t: &Tok, escape: bool) -> R<()> {
        let s = if t.buf == 0 { Spell::Scratch(t.start, t.end) } else { Spell::Bytes(&self.files[t.buf as usize - 1].bytes[t.start as usize..t.end as usize]) };
        self.copy_spell(s, escape)
    }

    /// A spelling appended to scratch, `"` and `\\` escaped where `escape`: the runs between
    /// them copied whole.
    fn copy_spell(&mut self, s: Spell<'a>, escape: bool) -> R<()> {
        let (start, end) = match s {
            Spell::Scratch(lo, hi) => (lo as usize, hi as usize),
            Spell::Bytes(b) => (0, b.len()),
        };
        let worst = if escape { 2 * (end - start) } else { end - start };
        if self.scratch.len() + worst > self.bounds.scratch {
            return Err("past the scratch bytes' bound");
        }
        let mut at = start;
        while at < end {
            let b = match s {
                Spell::Scratch(..) => &self.scratch[at..end],
                Spell::Bytes(b) => &b[at..end],
            };
            let run = if escape { b.iter().position(|&c| c == b'"' || c == b'\\').unwrap_or(b.len()) } else { b.len() };
            match s {
                Spell::Scratch(..) => self.scratch.extend_from_within(at..at + run),
                Spell::Bytes(b) => self.scratch.extend_from_slice(&b[at..at + run]),
            }
            at += run;
            if at < end {
                let c = match s {
                    Spell::Scratch(..) => self.scratch[at],
                    Spell::Bytes(b) => b[at],
                };
                self.scratch.extend_from_slice(&[b'\\', c]);
                at += 1;
            }
        }
        Ok(())
    }

    fn scratch_push(&mut self, b: &[u8]) -> R<()> {
        if self.scratch.len() + b.len() > self.bounds.scratch {
            return Err("past the scratch bytes' bound");
        }
        self.scratch.extend_from_slice(b);
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

    /// The level `if_open` opened closed: its tokens' range of `kept`, left in place (the caller
    /// cuts `kept` back once it has read them).
    fn line_end(&mut self) -> R<(u32, u32)> {
        let (_, at) = self.levels.pop().ok_or("no level open")?;
        if !self.sources.pop().is_some_and(|s| s.stop) {
            return Err("a line's source not on top");
        }
        self.in_if = false;
        Ok((at, self.kept.len() as u32))
    }

    /// A spelling's bytes.
    fn spell(&self, s: Spell<'a>) -> &[u8] {
        match s {
            Spell::Scratch(lo, hi) => &self.scratch[lo as usize..hi as usize],
            Spell::Bytes(b) => b,
        }
    }

    /// A token's spelling, in place.
    fn tok_spell(&self, t: &Tok) -> Spell<'a> {
        let files: &'a [File<'a>] = self.files;
        if t.buf == 0 { Spell::Scratch(t.start, t.end) } else { Spell::Bytes(&files[t.buf as usize - 1].bytes[t.start as usize..t.end as usize]) }
    }

    fn tok_in(&self, ts: Toks, k: u32) -> Tok {
        match ts {
            Toks::Kept(..) => self.kept[k as usize],
            Toks::Arena(..) => self.arena[k as usize],
        }
    }

    /// A string literal's text (its escapes of `"` and `\\` undone) appended to scratch: its range;
    /// `None` for a raw literal or no string literal.
    fn destringize(&mut self, s: Spell<'a>) -> R<Option<Spell<'a>>> {
        let b = self.spell(s);
        let Some(q) = b.iter().position(|&c| c == b'"' || c == b'\'') else { return Ok(None) };
        if b[q] != b'"' || b[..q].contains(&b'R') || b.len() < q + 2 {
            return Ok(None);
        }
        let (lo, hi) = (q + 1, b.len() - 1);
        if self.scratch.len() + (hi - lo) > self.bounds.scratch {
            return Err("past the scratch bytes' bound");
        }
        let start = self.scratch.len() as u32;
        let mut k = lo;
        while k < hi {
            let b = self.spell(s);
            let j = (k..hi).find(|&j| b[j] == b'\\' && j + 1 < hi && matches!(b[j + 1], b'"' | b'\\')).unwrap_or(hi);
            self.copy_spell(s.sub(k, j), false)?;
            if j < hi {
                let c = self.spell(s)[j + 1];
                self.scratch.push(c);
                k = j + 2;
            } else {
                k = hi;
            }
        }
        Ok(Some(Spell::Scratch(start, self.scratch.len() as u32)))
    }

    /// The level `if_open` opened closed, its tokens left for `EXPR`.
    fn expr_begin(&mut self) -> R<()> {
        let (at, hi) = self.line_end()?;
        let mut ex = std::mem::take(&mut self.ex);
        (ex.lo, ex.at, ex.hi, ex.err) = (at, at, hi, false);
        (ex.vals.clear(), ex.ops.clear(), ex.mins.clear(), ex.live.clear());
        ex.live.push(true);
        self.ex = ex;
        Ok(())
    }

    /// The operator of the expression's next token, if it has one.
    fn expr_peek(&self) -> Option<u8> {
        let ex = &self.ex;
        if ex.err || ex.at >= ex.hi {
            return None;
        }
        self.expr_op(&self.kept[ex.at as usize])
    }

    /// A token's operator: a punctuator's spelling (or an alternative token's punctuator), as
    /// one of the evaluator's operators; `None` for any other token.
    fn expr_op(&self, t: &Tok) -> Option<u8> {
        let it = crate::pp_expr::item(t.kind, self.text(t), self.program.pp.cpp && self.env.cpp);
        let op = it.op?;
        Some(OPS.iter().position(|o| *o == op).unwrap_or(OPS.len()) as u8)
    }

    /// Room for one more on one of the expression's stacks (`len` long).
    fn ex_room(&self, len: usize) -> R<()> {
        if len >= self.bounds.depth * 4 {
            return Err("past the expression's bound");
        }
        Ok(())
    }

    fn live(&self) -> bool {
        *self.ex.live.last().unwrap_or(&false)
    }

    fn expr_push(&mut self, v: crate::pp_expr::Value) -> R<()> {
        if self.ex.vals.len() >= self.bounds.depth * 4 {
            return Err("past the expression's bound");
        }
        self.ex.vals.push(v);
        Ok(())
    }

    fn pop2(&mut self) -> R<(crate::pp_expr::Value, crate::pp_expr::Value)> {
        let r = self.ex.vals.pop().ok_or("no value")?;
        let l = self.ex.vals.pop().ok_or("no value")?;
        Ok((l, r))
    }

    fn expr(&mut self, mode: u8) -> R<Option<bool>> {
        use crate::pp_expr::{binary, prefix, Value};
        let zero = Value::int(0);
        Ok(Some(match mode {
            0 => {
                let ex = &mut self.ex;
                if ex.err || ex.at >= ex.hi {
                    ex.err = true;
                    self.expr_push(zero)?;
                    return Ok(Some(false));
                }
                ex.t = self.kept[ex.at as usize];
                ex.at += 1;
                true
            }
            1 => {
                let t = self.ex.t;
                match self.expr_op(&t) {
                    Some(op) if matches!(op_bytes(op), b"+" | b"-" | b"~" | b"!") => {
                        self.ex_room(self.ex.ops.len())?;
                        self.ex.ops.push(op);
                        true
                    }
                    _ => false,
                }
            }
            2 => {
                let op = self.ex.ops.pop().ok_or("no operator")?;
                let v = self.ex.vals.last_mut().ok_or("no value")?;
                *v = prefix(op_bytes(op), *v);
                return Ok(None);
            }
            3 => {
                let t = self.ex.t;
                self.expr_op(&t) == Some(op_index(b"("))
            }
            4 => {
                if self.expr_peek() == Some(op_index(b")")) {
                    self.ex.at += 1;
                } else {
                    self.ex.err = true;
                }
                return Ok(None);
            }
            5 => {
                let t = self.ex.t;
                let cpp = self.program.pp.cpp && self.env.cpp;
                let it = crate::pp_expr::item(t.kind, self.text(&t), cpp);
                let v = crate::pp_expr::primary(&it, cpp);
                if v.is_none() {
                    self.ex.err = true;
                }
                self.expr_push(v.unwrap_or(zero))?;
                return Ok(None);
            }
            6 => {
                if self.expr_peek() != Some(op_index(b",")) {
                    return Ok(Some(false));
                }
                self.ex.at += 1;
                self.ex.vals.pop().ok_or("no value")?;
                true
            }
            7 => {
                self.ex_room(self.ex.mins.len())?;
                self.ex.mins.push(1);
                return Ok(None);
            }
            8 => {
                let min = *self.ex.mins.last().ok_or("no precedence")?;
                let Some(op) = self.expr_peek() else { return Ok(Some(false)) };
                let Some(p) = binary(op_bytes(op)).filter(|&p| p >= min) else { return Ok(Some(false)) };
                self.ex_room(self.ex.mins.len())?;
                self.ex_room(self.ex.ops.len())?;
                self.ex_room(self.ex.live.len())?;
                self.ex.at += 1;
                let l = *self.ex.vals.last().ok_or("no value")?;
                let live = self.live()
                    && match op_bytes(op) {
                        b"&&" => l.is_true(),
                        b"||" => !l.is_true(),
                        _ => true,
                    };
                (self.ex.ops.push(op), self.ex.mins.push(p + 1), self.ex.live.push(live));
                true
            }
            9 => {
                let op = self.ex.ops.pop().ok_or("no operator")?;
                self.ex.live.pop();
                let live = self.live();
                let (l, r) = self.pop2()?;
                let v = match op_bytes(op) {
                    b"&&" => Value::int((l.is_true() && r.is_true()) as i64),
                    b"||" => Value::int((l.is_true() || r.is_true()) as i64),
                    op => crate::pp_expr::apply(op, l, r, live).unwrap_or_else(|| {
                        self.ex.err = true;
                        zero
                    }),
                };
                self.expr_push(v)?;
                return Ok(None);
            }
            10 => {
                self.ex.mins.pop().ok_or("no precedence")?;
                return Ok(None);
            }
            11 => {
                if self.expr_peek() != Some(op_index(b"?")) {
                    return Ok(Some(false));
                }
                self.ex_room(self.ex.live.len())?;
                self.ex.at += 1;
                let c = *self.ex.vals.last().ok_or("no value")?;
                let live = self.live() && c.is_true();
                self.ex.live.push(live);
                true
            }
            12 => {
                self.ex.live.pop();
                if self.expr_peek() == Some(op_index(b":")) {
                    self.ex.at += 1;
                } else {
                    self.ex.err = true;
                }
                let n = self.ex.vals.len();
                let c = *self.ex.vals.get(n.wrapping_sub(2)).ok_or("no value")?;
                let live = self.live() && !c.is_true();
                self.ex.live.push(live);
                return Ok(None);
            }
            13 => {
                self.ex.live.pop();
                let (a, b) = self.pop2()?;
                let c = self.ex.vals.pop().ok_or("no value")?;
                let v = if c.is_true() { a } else { b };
                self.expr_push(Value { bits: v.bits, unsigned: a.unsigned || b.unsigned })?;
                return Ok(None);
            }
            _ => {
                let ex = &self.ex;
                let ok = !ex.err && ex.at == ex.hi && ex.vals.last().is_some_and(|v| v.is_true());
                self.kept.truncate(ex.lo as usize);
                ok
            }
        }))
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
            matches!(self.find(self.text(&n))?, Ok(i) if self.table[i].def != NONE)
        };
        if n.kind == IDENT && !v {
            self.missed(n)?;
        }
        if paren {
            self.read();
        }
        self.t = self.scratch_tok(NUMBER, if v { b"1" } else { b"0" }, t.flags & (BOL | SPACE))?;
        Ok(true)
    }

    // The includes.

    fn include(&mut self, mode: u8) -> R<Option<bool>> {
        // `m=1`'s line, its stop source on top, closed first.
        let line = if mode == 1 { self.line_end()? } else { (0, 0) };
        let i = self.file_src()?;
        let s = self.sources[i];
        match mode {
            0 => {
                self.inc.how = match self.text(&self.t) {
                    b"include_next" => 1,
                    b"import" => 2,
                    _ => 0,
                };
                let incs = self.files[s.file as usize].includes;
                let Ok(k) = incs.binary_search_by_key(&s.hash, |x| x.0) else { return Ok(Some(false)) };
                self.inc.to = incs[k].1;
                self.skip_line()?;
                Ok(Some(true))
            }
            1 => {
                let s0 = self.scratch.len();
                let hn = self.header_name(Toks::Kept(line.0, line.1))?;
                self.kept.truncate(line.0 as usize);
                let Some((name, angle)) = hn else { return Ok(Some(false)) };
                // `#include_next` in the main file is `#include` (clang's reading).
                let next = self.inc.how == 1 && s.file != 0;
                let named = self.files[s.file as usize].named;
                let found = named.iter().find(|n| n.is(self.spell(name), angle, next, false)).map(|n| n.to);
                let held = match found {
                    Some(to) => {
                        self.inc.to = to;
                        true
                    }
                    None => {
                        self.want(s.file, name, angle, next, false)?;
                        false
                    }
                };
                self.scratch.truncate(s0);
                Ok(Some(held))
            }
            _ => {
                let f = self.inc.to;
                let depth = self.sources.iter().filter(|s| s.file != NONE).count();
                if f == NONE || self.once[f as usize] || depth > MAX_INCLUDE_DEPTH {
                    return Ok(None);
                }
                if self.inc.how == 2 {
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

    /// The header name tokens make: a string literal's inside (in place), or `<` … `>` spelled
    /// with their blanks, in scratch (the lean pass's `header_of`).
    /// A file wanted and not found, recorded: its name's bytes copied to `wanted_bytes`.
    fn want(&mut self, file: u32, name: Spell<'a>, angle: bool, next: bool, embed: bool) -> R<()> {
        self.want_as(file, name, angle, next, embed, false)
    }

    /// `want`, or a `_Pragma`'s text wanted as a file (`pragma`).
    fn want_as(&mut self, file: u32, name: Spell<'a>, angle: bool, next: bool, embed: bool, pragma: bool) -> R<()> {
        let n = self.spell(name).len();
        if self.wanted.len() >= self.bounds.output || self.wanted_bytes.len() + n > self.bounds.scratch {
            return Err("past the wanted files' bound");
        }
        let lo = self.wanted_bytes.len() as u32;
        match name {
            Spell::Scratch(a, b) => self.wanted_bytes.extend_from_slice(&self.scratch[a as usize..b as usize]),
            Spell::Bytes(b) => self.wanted_bytes.extend_from_slice(b),
        }
        self.wanted.push(Want { file, lo, hi: self.wanted_bytes.len() as u32, angle, next, embed, pragma });
        Ok(())
    }

    fn header_name(&mut self, ts: Toks) -> R<Option<(Spell<'a>, bool)>> {
        let (Toks::Kept(lo, hi) | Toks::Arena(lo, hi)) = ts;
        if lo >= hi {
            return Ok(None);
        }
        let first = self.tok_in(ts, lo);
        let n = self.text(&first).len();
        if first.kind == LITERAL && self.text(&first).first() == Some(&b'"') && n >= 2 {
            return Ok(Some((self.tok_spell(&first).sub(1, n - 1), false)));
        }
        if self.text(&first) != b"<" {
            return Ok(None);
        }
        // `<` … `>`: spelled in scratch.
        let start = self.scratch.len();
        for k in lo + 1..hi {
            let t = self.tok_in(ts, k);
            if self.text(&t) == b">" && t.kind == PUNCT {
                return Ok(Some((Spell::Scratch(start as u32, self.scratch.len() as u32), true)));
            }
            if k > lo + 1 && t.flags & SPACE != 0 {
                self.scratch_push(b" ")?;
            }
            self.scratch_copy(&t, false)?;
        }
        self.scratch.truncate(start);
        Ok(None)
    }

    fn pragma(&mut self) -> R<()> {
        let i = self.file_src()?;
        let file = self.sources[i].file;
        let (mut toks, mut n) = ([Tok::default(); 3], 0);
        while let Some(t) = self.line_tok()? {
            if n < 3 {
                toks[n] = t;
            }
            n += 1;
        }
        self.pragma_of(file, toks, n)
    }

    /// A pragma read from its first three tokens and its count (`#pragma`'s line, or a
    /// `_Pragma`'s text lexed as a file): its word and its string argument carried out.
    fn pragma_of(&mut self, file: u32, toks: [Tok; 3], n: usize) -> R<()> {
        let word = if n > 0 && toks[0].kind == IDENT { self.tok_spell(&toks[0]) } else { Spell::Bytes(b"") };
        // The lean pass's string argument: `( "…" )`, a string literal, not a raw one.
        let lit = (n > 2 && self.spelled(&toks[1], b"(") && toks[2].kind == LITERAL).then(|| self.tok_spell(&toks[2]));
        self.pragma_do(file, word, lit)
    }

    /// A pragma carried out: its first word, and its string argument (destringized here, where
    /// it is `push_macro`'s or `pop_macro`'s).
    fn pragma_do(&mut self, file: u32, word: Spell<'a>, lit: Option<Spell<'a>>) -> R<()> {
        let w = self.spell(word);
        if w == b"once" {
            self.once[file as usize] = true;
            return Ok(());
        }
        let (push, pop) = (w == b"push_macro", w == b"pop_macro");
        if !(push || pop) {
            return Ok(());
        }
        let Some(lit) = lit else { return Ok(()) };
        let Some(name @ Spell::Scratch(n0, n1)) = self.destringize(lit)? else { return Ok(()) };
        let slot = match self.find(self.spell(name))? {
            Ok(i) => i,
            Err(i) if push => {
                let n = Tok { kind: IDENT, flags: 0, buf: 0, start: n0, end: n1, exp: 0 };
                self.table[i] = Slot { key: key(self.spell(name)), name: n, def: NONE };
                i
            }
            Err(_) => return Ok(()),
        };
        if push {
            if self.pushed.len() >= self.bounds.macros * 4 {
                return Err("past push_macro's bound");
            }
            self.pushed.push((slot as u32, self.table[slot].def));
        } else if let Some(k) = self.pushed.iter().rposition(|p| p.0 == slot as u32) {
            let (_, d) = self.pushed.remove(k);
            self.table[slot].def = d;
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
                Err(i) => self.table[i] = Slot { key: key(name.as_bytes()), name: n, def: d },
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
                self.t = self.builtin_value(b, &t, Toks::Kept(0, 0))?;
                Ok(None)
            }
            1 => {
                if !self.in_if && matches!(b, B_HAS_INCLUDE | B_HAS_INCLUDE_NEXT | B_HAS_EMBED) {
                    return Ok(Some(false));
                }
                self.bi = Bi { b: u32::from(b), name: self.t, depth: 0 };
                if self.levels.len() >= self.bounds.depth {
                    return Err("past the sources' bound");
                }
                self.levels.push((u16::MAX, self.kept.len() as u32));
                self.collecting += 1;
                Ok(Some(true))
            }
            2 => {
                let t = self.t;
                if self.spelled(&t, b")") && t.kind == PUNCT {
                    if self.bi.depth == 0 {
                        return Ok(Some(true));
                    }
                    self.bi.depth -= 1;
                } else if self.spelled(&t, b"(") && t.kind == PUNCT {
                    self.bi.depth += 1;
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
                let hi = self.kept.len() as u32;
                let (b, name) = (self.bi.b as u8, self.bi.name);
                let header = matches!(b, B_HAS_INCLUDE | B_HAS_INCLUDE_NEXT | B_HAS_EMBED);
                let starts = at < hi && {
                    let t = self.kept[at as usize];
                    (t.kind == LITERAL && self.text(&t).first() == Some(&b'"')) || (t.kind == PUNCT && self.spelled(&t, b"<"))
                };
                if b == B_PRAGMA || (header && !starts) {
                    // The operand expanded first: a stop source of its tokens, and a level.
                    let a = self.arena.len() as u32;
                    self.push_kept((at, hi))?;
                    self.kept.truncate(at as usize);
                    if self.sources.len() >= self.bounds.depth || self.levels.len() >= self.bounds.depth {
                        return Err("past the sources' bound");
                    }
                    self.sources.push(Src::arena(a, self.arena.len() as u32, NONE, true));
                    self.levels.push((u16::MAX, self.kept.len() as u32));
                    return Ok(Some(true));
                }
                self.t = self.builtin_value(b, &name, Toks::Kept(at, hi))?;
                self.kept.truncate(at as usize);
                Ok(Some(false))
            }
            _ => {
                let (lo, hi) = self.line_end()?;
                let (b, name) = (self.bi.b as u8, self.bi.name);
                if b != B_PRAGMA {
                    self.t = self.builtin_value(b, &name, Toks::Kept(lo, hi))?;
                    self.kept.truncate(lo as usize);
                    return Ok(Some(true));
                }
                // `_Pragma("…")`: one string literal, its text the pragma's.
                let one = (hi - lo == 1).then(|| self.kept[lo as usize]).filter(|s| s.kind == LITERAL);
                self.kept.truncate(lo as usize);
                // Its text is a file of its own, lexed by the token grammar as the host lexes
                // every file: the file on top's named one, else wanted (the host lexes it, and
                // the TU runs again).
                if let Some(s) = one
                    && let Some(text) = self.destringize(self.tok_spell(&s))?
                {
                    let file = self.cur_file();
                    let named = self.files[file as usize].named;
                    match named.iter().find(|n| n.pragma && n.name == self.spell(text)).map(|n| n.to) {
                        Some(to) if to != NONE => {
                            let ts = self.files[to as usize].toks;
                            let mut toks = [Tok::default(); 3];
                            for (k, t) in ts.iter().take(3).enumerate() {
                                toks[k] = *t;
                            }
                            self.pragma_of(file, toks, ts.len())?;
                        }
                        Some(_) => return Err("a _Pragma's text the token grammar refuses"),
                        None => self.want_as(file, text, false, false, false, true)?,
                    }
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

    fn presumed_path(&self) -> Spell<'a> {
        match self.sources.iter().rev().find(|s| s.file != NONE) {
            Some(s) if s.presumed.0 != NONE => Spell::Scratch(s.presumed.0, s.presumed.1),
            Some(s) => Spell::Bytes(self.files[s.file as usize].path),
            None => Spell::Bytes(b""),
        }
    }

    fn number_tok(&mut self, v: i64, like: &Tok) -> R<Tok> {
        // The digits from the right, in a word's worth of bytes (at most 20, and a sign).
        let (mut b, mut i, mut u) = ([0u8; 21], 21, v.unsigned_abs());
        loop {
            i -= 1;
            b[i] = b'0' + (u % 10) as u8;
            u /= 10;
            if u == 0 {
                break;
            }
        }
        if v < 0 {
            i -= 1;
            b[i] = b'-';
        }
        let t = self.scratch_tok(NUMBER, &b[i..], like.flags & (BOL | SPACE))?;
        Ok(Tok { exp: like.exp, ..t })
    }

    /// A string literal of `s`, its `"` and `\` escaped.
    fn string_tok(&mut self, s: Spell<'a>, like: &Tok) -> R<Tok> {
        let start = self.scratch.len() as u32;
        self.scratch_push(b"\"")?;
        self.copy_spell(s, true)?;
        self.scratch_push(b"\"")?;
        Ok(Tok { kind: LITERAL, flags: like.flags & (BOL | SPACE), buf: 0, start, end: self.scratch.len() as u32, exp: like.exp })
    }

    /// Built-in `b`'s value for its use `t` and its operand `op`.
    fn builtin_value(&mut self, b: u8, t: &Tok, op: Toks) -> R<Tok> {
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
                let path = if b == B_BASE_FILE { Spell::Bytes(self.files.first().map_or(&b""[..], |f| f.path)) } else { self.presumed_path() };
                // The name after the last `/`.
                let path = match path {
                    _ if b != B_FILE_NAME => path,
                    Spell::Bytes(p) => Spell::Bytes(p.rsplit(|&c| c == b'/').next().unwrap_or(b"")),
                    Spell::Scratch(lo, hi) => Spell::Scratch(self.scratch[lo as usize..hi as usize].iter().rposition(|&c| c == b'/').map_or(lo, |k| lo + k as u32 + 1), hi),
                };
                self.string_tok(path, t)
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
                self.string_tok(Spell::Bytes(s), t)
            }
            B_HAS_INCLUDE | B_HAS_INCLUDE_NEXT | B_HAS_EMBED => {
                let file = self.cur_file();
                let s0 = self.scratch.len();
                let v = match self.header_name(op)? {
                    Some((name, angle)) => {
                        let next = b == B_HAS_INCLUDE_NEXT && file != 0;
                        let embed = b == B_HAS_EMBED;
                        let named = self.files[file as usize].named;
                        match named.iter().find(|n| n.is(self.spell(name), angle, next, embed)) {
                            Some(n) if n.to == NONE => 0,
                            // 1: found, and not empty; 2: found and empty (`__STDC_EMBED_EMPTY__`).
                            Some(n) if b == B_HAS_EMBED && self.files[n.to as usize].bytes.is_empty() => 2,
                            Some(_) => 1,
                            None => {
                                self.want(file, name, angle, next, embed)?;
                                0
                            }
                        }
                    }
                    None => 0,
                };
                self.scratch.truncate(s0);
                self.number_tok(v, t)
            }
            _ => {
                let builtin = BUILTINS[usize::from(b)].0.as_bytes();
                // The operand's spellings, run together in scratch.
                let s0 = self.scratch.len();
                let (Toks::Kept(lo, hi) | Toks::Arena(lo, hi)) = op;
                for k in lo..hi {
                    let x = self.tok_in(op, k);
                    self.scratch_copy(&x, false)?;
                }
                let found = self.env.features.iter().find(|f| f.0 == builtin && f.1 == &self.scratch[s0..]).map(|f| f.2);
                let v = match found {
                    Some(v) => v,
                    None => {
                        let n = self.scratch.len() - s0;
                        if self.asked.len() >= self.bounds.output || self.asked_bytes.len() + n > self.bounds.scratch {
                            return Err("past the unanswered features' bound");
                        }
                        let lo = self.asked_bytes.len() as u32;
                        self.asked_bytes.extend_from_slice(&self.scratch[s0..]);
                        self.asked.push(Asked { b: u32::from(b), lo, hi: self.asked_bytes.len() as u32 });
                        i64::from(builtin == b"__is_identifier")
                    }
                };
                self.scratch.truncate(s0);
                self.number_tok(v, t)
            }
        }
    }

    /// `#line N "file"` and `# N "file" flags`: the next line is line `N` (of `file`).
    fn linedir(&mut self, mode: u8) -> R<()> {
        if mode == 0 {
            (self.marker, self.has_marker) = (self.t, true);
            return Ok(());
        }
        let last = self.line_last;
        let (lo, hi) = self.line_end()?;
        // The line's tokens, after the marker's (a line marker's number) where there is one.
        let marker = self.has_marker.then_some(self.marker);
        self.has_marker = false;
        let tok = |m: &Self, k: u32| match marker {
            Some(t) if k == 0 => Some(t),
            Some(_) => (lo + k - 1 < hi).then(|| m.kept[(lo + k - 1) as usize]),
            None => (lo + k < hi).then(|| m.kept[(lo + k) as usize]),
        };
        let (first, second) = (tok(self, 0), tok(self, 1));
        self.kept.truncate(lo as usize);
        let i = self.file_src()?;
        let Some(n) = first.filter(|t| t.kind == NUMBER).and_then(|t| crate::pp_expr::number(self.text(&t))) else { return Ok(()) };
        // The directive's last line: the line after it is line `n`.
        let s = self.sources[i];
        let phys = self.files[s.file as usize].lines.partition_point(|&k| k < last.end) as i64 + 2;
        self.sources[i].delta = (n.bits as i64 - phys) as i32;
        if let Some(t) = second.filter(|t| t.kind == LITERAL)
            && let Some(Spell::Scratch(p0, p1)) = self.destringize(self.tok_spell(&t))?
        {
            self.sources[i].presumed = (p0, p1);
        }
        Ok(())
    }

    // `#embed`.

    /// `toks` pushed to be expanded: a stop source, and a level.
    fn push_expand(&mut self, r: (u32, u32)) -> R<()> {
        if self.sources.len() >= self.bounds.depth || self.levels.len() >= self.bounds.depth {
            return Err("past the sources' bound");
        }
        self.sources.push(Src::arena(r.0, r.1, NONE, true));
        self.levels.push((u16::MAX, self.kept.len() as u32));
        Ok(())
    }

    fn embed(&mut self, mode: u8) -> R<Option<bool>> {
        match mode {
            0 => {
                let i = self.file_src()?;
                let s = self.sources[i];
                // The line's tokens to the arena: the name's, then the parameters'.
                let a = self.arena.len() as u32;
                while let Some(t) = self.line_tok()? {
                    self.push_arena(t)?;
                }
                let end = self.arena.len() as u32;
                // The name ends with its `"…"` or `>`.
                let angle = a < end && self.arena[a as usize].kind == PUNCT && self.spelled(&self.arena[a as usize], b"<");
                let k = if angle { (a..end).find(|&k| self.arena[k as usize].kind == PUNCT && self.spelled(&self.arena[k as usize], b">")).map_or(end, |k| k + 1) } else { (a + 1).min(end) };
                let incs = self.files[s.file as usize].includes;
                self.embed = ((k, end), None, incs.binary_search_by_key(&s.hash, |x| x.0).ok().map(|k| incs[k].1));
                let spelled = angle || (a < k && self.arena[a as usize].kind == LITERAL && self.text(&self.arena[a as usize]).first() == Some(&b'"'));
                if spelled || a == k {
                    self.embed.1 = self.header_name(Toks::Arena(a, k))?;
                    // Malformed: nothing, its parameters not expanded (the lean pass's order).
                    let rest = if self.embed.1.is_some() { (k, end) } else { (end, end) };
                    self.push_expand(rest)?;
                    return Ok(Some(false));
                }
                self.push_expand((a, k))?;
                Ok(Some(true))
            }
            1 => {
                let (lo, hi) = self.line_end()?;
                self.embed.1 = self.header_name(Toks::Kept(lo, hi))?;
                self.kept.truncate(lo as usize);
                let rest = if self.embed.1.is_some() { self.embed.0 } else { (0, 0) };
                self.push_expand(rest)?;
                Ok(None)
            }
            2 => {
                let (at, hi) = self.line_end()?;
                // No name: nothing read, nothing pushed.
                let i = if self.embed.1.is_some() { at } else { hi };
                self.em = Em { lo: at, i, hi, ..Em::default() };
                Ok(None)
            }
            3 => {
                let v = self.em;
                if v.i >= v.hi {
                    return Ok(Some(false));
                }
                self.em.p = self.kept[v.i as usize];
                let paren = v.i + 1 < v.hi && self.spelled(&self.kept[v.i as usize + 1], b"(");
                self.em.i = v.i + 1 + u32::from(paren);
                // No `(`: an empty argument, closed already.
                self.em.arg = (self.em.i, if paren { u32::MAX } else { self.em.i });
                self.em.depth = 0;
                Ok(Some(true))
            }
            4 => {
                let v = self.em;
                if v.arg.1 != u32::MAX {
                    return Ok(Some(false));
                }
                if v.i < v.hi {
                    let t = self.kept[v.i as usize];
                    if self.spelled(&t, b"(") {
                        self.em.depth += 1;
                    } else if self.spelled(&t, b")") {
                        if v.depth == 0 {
                            self.em.arg.1 = v.i;
                            self.em.i = v.i + 1;
                            return Ok(Some(false));
                        }
                        self.em.depth -= 1;
                    }
                    self.em.i = v.i + 1;
                    return Ok(Some(true));
                }
                // Unclosed: the argument runs to the end.
                (self.em.arg.1, self.em.i) = (v.hi, v.hi + 1);
                Ok(Some(false))
            }
            5 => {
                let v = self.em;
                if v.p.kind != IDENT {
                    return Ok(Some(false));
                }
                match self.text(&v.p) {
                    b"limit" => {
                        let mut ex = std::mem::take(&mut self.ex);
                        (ex.lo, ex.at, ex.hi, ex.err) = (v.arg.0, v.arg.0, v.arg.1, false);
                        (ex.vals.clear(), ex.ops.clear(), ex.mins.clear(), ex.live.clear());
                        ex.live.push(true);
                        self.ex = ex;
                        return Ok(Some(true));
                    }
                    b"prefix" => self.em.prefix = v.arg,
                    b"suffix" => self.em.suffix = v.arg,
                    b"if_empty" => self.em.if_empty = v.arg,
                    _ => {}
                }
                Ok(Some(false))
            }
            6 => {
                let ex = &self.ex;
                self.em.limit = (!ex.err && ex.at == ex.hi).then(|| ex.vals.last().map_or(0, |v| v.bits as usize));
                Ok(None)
            }
            7 => {
                let v = self.em;
                self.em.a = self.arena.len() as u32;
                let Some((name, angle)) = self.embed.1.take() else { return Ok(Some(false)) };
                let file = self.cur_file();
                let to = match self.embed.2.take() {
                    Some(to) => to,
                    None => match self.files[file as usize].named.iter().find(|n| n.is(self.spell(name), angle, false, true)) {
                        Some(n) => n.to,
                        None => {
                            self.want(file, name, angle, false, true)?;
                            NONE
                        }
                    },
                };
                if to == NONE {
                    return Ok(Some(false));
                }
                let len = self.files[to as usize].bytes.len();
                let n = v.limit.unwrap_or(len).min(len) as u32;
                (self.em.file, self.em.k, self.em.n) = (to, 0, n);
                self.push_kept(if n == 0 { v.if_empty } else { v.prefix })?;
                if n > 0 && self.digits.is_none() {
                    // `0` … `255`, then `,`.
                    let base = self.scratch.len() as u32;
                    self.scratch_push(&DIGITS)?;
                    self.digits = Some(base);
                }
                Ok(Some(n > 0))
            }
            8 => {
                let v = self.em;
                if v.k >= v.n {
                    return Ok(Some(false));
                }
                let base = self.digits.ok_or("no digits")?;
                if v.k > 0 {
                    let c = base + 658;
                    self.push_arena(Tok { kind: PUNCT, flags: 0, buf: 0, start: c, end: c + 1, exp: 0 })?;
                }
                let d = u32::from(self.files[v.file as usize].bytes[v.k as usize]);
                let (off, len) = if d < 10 { (d, 1) } else if d < 100 { (10 + 2 * (d - 10), 2) } else { (190 + 3 * (d - 100), 3) };
                self.push_arena(Tok { kind: NUMBER, flags: 0, buf: 0, start: base + off, end: base + off + len, exp: 0 })?;
                self.em.k = v.k + 1;
                Ok(Some(true))
            }
            9 => {
                let v = self.em;
                self.push_kept(v.suffix)?;
                Ok(None)
            }
            _ => {
                let v = self.em;
                self.kept.truncate(v.lo as usize);
                let end = self.arena.len() as u32;
                if end > v.a {
                    self.arena[v.a as usize].flags |= BOL;
                    if self.sources.len() >= self.bounds.depth {
                        return Err("past the sources' bound");
                    }
                    self.sources.push(Src::arena(v.a, end, NONE, false));
                }
                Ok(None)
            }
        }
    }

    /// Tokens `kept[r.0..r.1]` pushed to the arena.
    fn push_kept(&mut self, r: (u32, u32)) -> R<()> {
        for k in r.0..r.1 {
            let t = self.kept[k as usize];
            self.push_arena(t)?;
        }
        Ok(())
    }

    // The directives.

    fn define(&mut self, mode: u8) -> R<Option<bool>> {
        let v = self.def;
        let held = match mode {
            0 => {
                let Some(name) = self.line_tok()?.filter(|t| t.kind == IDENT) else {
                    self.skip_line()?;
                    return Ok(Some(false));
                };
                self.def = DefState { name, p0: self.params.len() as u32, want: true, ..DefState::default() };
                true
            }
            1 => {
                let first = self.line_tok()?;
                if first.is_some_and(|t| self.spelled(&t, b"(") && t.flags & SPACE == 0) {
                    self.def.fnlike = true;
                    true
                } else {
                    self.def.first = first.unwrap_or(eof(0));
                    false
                }
            }
            2 => {
                let p0 = v.p0 as usize;
                let Some(t) = self.line_tok()? else {
                    // Unterminated: no definition.
                    self.params.truncate(p0);
                    return Ok(Some(false));
                };
                let n = self.params.len() - p0;
                if self.spelled(&t, b")") && (!v.want || n == 0) && !(v.want && v.variadic) {
                    self.def.closed = true;
                    return Ok(Some(false));
                }
                let malformed = if v.variadic {
                    true
                } else if v.want && t.kind == IDENT {
                    self.push_param(t)?;
                    self.def.want = false;
                    false
                } else if v.want && self.spelled(&t, b"...") {
                    // `__VA_ARGS__`'s spelling, made once.
                    let va = self.scratch_tok(IDENT, VA_ARGS, 0)?;
                    self.push_param(va)?;
                    (self.def.variadic, self.def.want) = (true, false);
                    false
                } else if !v.want && self.spelled(&t, b"...") && n > 0 {
                    // GNU's `name...`.
                    self.def.variadic = true;
                    false
                } else if !v.want && self.spelled(&t, b",") {
                    self.def.want = true;
                    false
                } else {
                    true
                };
                if malformed {
                    self.params.truncate(p0);
                    self.skip_line()?;
                    return Ok(Some(false));
                }
                true
            }
            3 => {
                if v.closed {
                    self.def.first = self.line_tok()?.unwrap_or(eof(0));
                }
                v.closed
            }
            4 => {
                // The body: the line's tokens from `first`, a range of the file's tape.
                let top = *self.sources.last().ok_or("a directive with no file")?;
                let first_at = if v.first.kind != EOF { top.pos - 1 } else { top.pos };
                if self.params.len() - v.p0 as usize > 0xfff0 {
                    return Err("past the parameters' bound");
                }
                let params = (self.params.len() - v.p0 as usize) as u16;
                self.def.d = Def { fnlike: v.fnlike, variadic: v.variadic, params, file: top.file, first: first_at, end: first_at, marks: self.marks.len() as u32, pre: (0, 0), disabled: false, builtin: 0 };
                if v.first.kind != EOF {
                    self.mark_body(v.first)?;
                }
                return Ok(None);
            }
            5 => {
                if let Some(t) = self.line_tok()? {
                    self.mark_body(t)?;
                    return Ok(Some(true));
                }
                // A `##` last is no paste operator.
                if v.d.end > v.d.first {
                    let k = (v.d.marks + v.d.end - v.d.first - 1) as usize;
                    self.marks[k] &= !PASTE_MARK;
                }
                false
            }
            6 => {
                let u0 = self.uses.len() as u32;
                self.def.d.pre = (u0, u0);
                (self.def.lo, self.def.i, self.def.hi, self.def.content) = (0, 0, if v.d.fnlike { v.d.end - v.d.first } else { 0 }, (NONE, NONE));
                return Ok(None);
            }
            7 => self.walk_use()?,
            _ => {
                let mut d = v.d;
                d.pre.1 = self.uses.len() as u32;
                self.params.truncate(v.p0 as usize);
                if self.defs.len() >= self.bounds.macros * 4 {
                    return Err("past the definitions' bound");
                }
                self.defs.push(d);
                let k = (self.defs.len() - 1) as u32;
                match self.find(self.text(&v.name))? {
                    Ok(i) => self.table[i].def = k,
                    Err(i) => self.table[i] = Slot { key: key(self.text(&v.name)), name: v.name, def: k },
                }
                return Ok(None);
            }
        };
        Ok(Some(held))
    }

    /// Body token `t` (the next of the definition being read) marked: a parameter's index,
    /// `__VA_OPT__`, and `##` (but first).
    fn push_param(&mut self, t: Tok) -> R<()> {
        if self.params.len() >= self.bounds.arena {
            return Err("past the arena's bound");
        }
        self.params.push(t);
        Ok(())
    }

    fn mark_body(&mut self, t: Tok) -> R<()> {
        let v = self.def;
        let k = v.d.end - v.d.first;
        let text = self.text(&t);
        let mut m = NO_PARAM;
        if t.kind == IDENT {
            if v.d.variadic && text == VA_OPT {
                m = VA_OPT_MARK;
            } else if v.d.fnlike && let Some(p) = self.params[v.p0 as usize..].iter().position(|p| self.text(p) == text) {
                m = p as u32;
            }
        }
        if t.kind == HASHHASH && k > 0 {
            m |= PASTE_MARK;
        }
        if self.marks.len() >= self.bounds.arena {
            return Err("past the arena's bound");
        }
        self.marks.push(m);
        self.def.d.end += 1;
        Ok(())
    }

    /// One step of the walk of the definition's argument uses, in the order substitution first
    /// asks for each (`SUB`'s walk): `(parameter, inside a __VA_OPT__)`.
    fn walk_use(&mut self) -> R<bool> {
        let v = self.def;
        let def = v.d;
        if v.i >= v.hi {
            // A content's end: the walk after it.
            if v.content.0 == NONE {
                return Ok(false);
            }
            (self.def.lo, self.def.i, self.def.hi, self.def.content) = (0, v.content.0, v.content.1, (NONE, NONE));
            return Ok(true);
        }
        let (i, hi, inside) = (v.i, v.hi, v.content.0 != NONE);
        let va = def.params.wrapping_sub(1);
        let bt = self.body_tok(&def, i);
        let opens = |m: &Self, k: u32| def.variadic && k < hi && m.mark(&def, k) == VA_OPT_MARK;
        let at = if bt.kind == HASH && i + 1 < hi && self.param(&def, i + 1).is_some() {
            self.def.i = i + 2;
            return Ok(true);
        } else if bt.kind == HASH && opens(self, i + 1) {
            Some(i + 1)
        } else if opens(self, i) {
            Some(i)
        } else {
            None
        };
        if let Some(k) = at {
            if inside {
                return Err("a __VA_OPT__ inside a __VA_OPT__");
            }
            if self.uses.len() >= self.bounds.arena {
                return Err("past the arena's bound");
            }
            self.uses.push((va, false));
            let (clo, chi, end) = self.va_opt_content(&def, k, hi);
            (self.def.lo, self.def.i, self.def.hi, self.def.content) = (clo, clo, chi, (end, hi));
            return Ok(true);
        }
        if let Some(p) = self.param(&def, i) {
            let before = i > v.lo && self.body_tok(&def, i - 1).flags & PASTE != 0;
            let after = i + 1 < hi && self.body_tok(&def, i + 1).flags & PASTE != 0;
            if !(before || after) {
                if self.uses.len() >= self.bounds.arena {
                    return Err("past the arena's bound");
                }
                self.uses.push((p as u16, inside));
            }
        }
        self.def.i = i + 1;
        Ok(true)
    }

    fn undef(&mut self) -> R<()> {
        if let Some(n) = self.line_tok()?.filter(|t| t.kind == IDENT) {
            if let Ok(i) = self.find(self.text(&n))? {
                self.table[i].def = NONE;
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

#[cfg(test)]
#[test]
fn the_digits_spell_0_to_255_and_a_comma() {
    let mut want: Vec<u8> = (0..=255u32).flat_map(|d| d.to_string().into_bytes()).collect();
    want.push(b',');
    assert_eq!(&DIGITS[..], &want[..]);
}
