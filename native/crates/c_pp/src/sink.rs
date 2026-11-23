//! Mode (b)'s outputs. A sink is told each token and each event as the pass makes it; any number
//! of sinks can be fed from one pass (`Tee`).

use crate::macros::{BOL, SPACE, Tk};
use rerail_c_lean::pp::P;

/// What applying finds wrong (`c_pp_diag`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Diag {
    /// `#error`'s text.
    Error,
    /// `#warning`'s text.
    Warning,
    /// A header not found (its spelling).
    NotFound,
    /// Includes nested past the limit.
    TooDeep,
    /// An `#if…` its file does not close; an `#elif…`, `#else` or `#endif` with none open; one
    /// after `#else`.
    Unterminated,
    Unmatched,
    AfterElse,
    /// An `#if`'s expression that is none.
    BadExpr(crate::expr::ExprError),
    /// A function-like macro's call its file ends inside, or with the wrong count of arguments.
    UnterminatedCall,
    ArgCount,
    /// `##` that made no one token (the two are kept).
    BadPaste,
    /// A directive's operands that are none it takes.
    Malformed,
    /// A directive's name that names none.
    UnknownDirective,
    /// A macro defined again, differently.
    Redefined,
    /// A poisoned identifier used.
    Poisoned,
}

/// Where a token came from, for sinks that ask (`Sink::wants_lines`): its file and line, where it
/// was read from its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct At {
    pub file: u32,
    pub line: u32,
}

/// A pass's outputs.
#[allow(unused_variables)]
pub trait Sink {
    /// Whether `token` is to be told each line's first token's line.
    fn wants_lines(&self) -> bool {
        false
    }

    /// An output token, its spelling, (`wants_lines`) where a line's first token came from, and
    /// its file position where it was read from a file (`t.exp == 0`, `t.buf != 0`; else 0).
    fn token(&mut self, t: &Tk, text: &[u8], at: Option<At>, off: u32) {}

    /// File `file` entered, from directive `from` (the includer and its `#`'s file position; `None`
    /// for the main file), at include depth `depth`.
    fn enter(&mut self, file: u32, path: &str, from: Option<(u32, u32)>, depth: u32) {}

    /// The pass is back in `file` (its includer), or past its end.
    fn leave(&mut self, file: u32) {}

    /// A header that was not found, by its spelling (`c_pp_include_wanted`).
    fn missing(&mut self, from: u32, at: u32, spelled: &[u8], angle: bool) {}

    /// A conditional directive at `at` in `file`, and whether its group was taken.
    fn cond(&mut self, file: u32, at: u32, taken: bool) {}

    /// A skipped group: `file`'s bytes from `start` to `end`.
    fn skipped(&mut self, file: u32, start: u32, end: u32) {}

    /// A macro's expansion (`Expansion`).
    fn expansion(&mut self, e: Expansion<'_>) {}

    /// Whether `entered` is to be told each macro invocation (a measurement's: what an expansion
    /// depends on, decision 0613).
    fn wants_entries(&self) -> bool {
        false
    }

    /// A macro invocation entered (`wants_entries`): what its expansion depends on.
    fn entered(&mut self, e: Entered) {}

    /// The unit's expansions as requests (`wants_entries`, decision 0613's measurement), at its end.
    fn requests(&mut self, r: &[Request], deps: &[u32]) {}

    /// A pragma (a `#pragma` line's text after `pragma`, or `_Pragma`'s string).
    fn pragma(&mut self, file: u32, at: u32, text: &[u8]) {}

    /// A diagnostic, at `at` in `file`.
    fn diag(&mut self, file: u32, at: u32, d: Diag, text: &[u8]) {}
}

/// A macro invocation as its expansion depends on it (`Sink::entered`).
/// - `key`: the definition (its spelling), its arguments' spellings, and the names of the macros
///   disabled where it is entered: equal in two units where their expansions are.
/// - `epoch_key`: `key` and the count of `#define`s and `#undef`s before it, so that every name its
///   rescan looks up resolves alike (a key equal in this one is equal at any place).
/// - `depth`: the expansions it is inside (its rescans' and its arguments').
/// - `size`: its tokens after substitution, before their rescan.
#[derive(Clone, Copy, Debug)]
pub struct Entered {
    pub def: u32,
    pub key: u64,
    pub epoch_key: u64,
    pub depth: u32,
    pub size: u32,
}

/// An expansion as a request (decision 0613's measurement): request 0 is the top (the files'
/// tokens); the others are macro invocations and arguments' expansions, in the order entered.
#[derive(Clone, Copy, Debug, Default)]
pub struct Request {
    /// The request whose reading posted this one (an argument's: its invocation).
    pub parent: u32,
    /// The parent's tokens read when this was posted (an argument's: 0).
    pub pos: u32,
    /// The tokens this request's own reading took from its list (its children's are theirs).
    pub own: u32,
    /// `Entered::epoch_key` (an argument's: its tokens, the disabled names, the definitions' count).
    pub key: u64,
    /// An argument's expansion before substitution.
    pub arg: bool,
    /// A place-dependent built-in read here (`__LINE__`, `__COUNTER__`, ...): not memoized.
    pub place: bool,
    /// Calls whose name is this request's last token and whose `(` comes after it (open tails).
    pub tails: u32,
    /// `Entered::key` (no definitions' count): the key of a memo across units.
    pub skey: u64,
    /// An invocation's expansion id (its body's tokens'); an argument's, 0.
    pub exp: u32,
    /// The step model of 0588's loop: the steps of the reading so far (`run` tokens not yet a step).
    pub steps: u32,
    pub run: u32,
    /// The parent's steps when this was posted.
    pub pos_steps: u32,
    /// An invocation whose name or arguments come from its parent's arguments' expansions.
    pub needs_args: bool,
    /// An argument's expansion with an edge its rescan can't decide before it is done.
    pub blocks: bool,
    /// An invocation's holes with such an edge: before (a function-like name, then a macro's
    /// name first) and after (a macro's name or `)` last, then the body's `(`).
    pub edge_first: u32,
    pub edge_last: u32,
    /// Calls in the rescan whose name and `(` came from different parts (an argument and the body).
    pub taken: u32,
    /// An invocation's dependencies, `deps[dep_at..dep_at + dep_n]` of `Sink::requests`: the
    /// expansions (`Tk::exp`) whose tokens its name and its call's shape came from (its `(` and `)`,
    /// the commas that split it, parentheses a producer left unpaired). A child taking its
    /// arguments as references waits for these alone.
    pub dep_at: u32,
    pub dep_n: u32,
}

/// No outputs.
pub struct Null;

impl Sink for Null {}

/// Two sinks fed from one pass.
pub struct Tee<'a>(pub &'a mut dyn Sink, pub &'a mut dyn Sink);

impl Sink for Tee<'_> {
    fn wants_lines(&self) -> bool {
        self.0.wants_lines() || self.1.wants_lines()
    }
    fn token(&mut self, t: &Tk, text: &[u8], at: Option<At>, off: u32) {
        self.0.token(t, text, at, off);
        self.1.token(t, text, at, off);
    }
    fn enter(&mut self, file: u32, path: &str, from: Option<(u32, u32)>, depth: u32) {
        self.0.enter(file, path, from, depth);
        self.1.enter(file, path, from, depth);
    }
    fn leave(&mut self, file: u32) {
        self.0.leave(file);
        self.1.leave(file);
    }
    fn missing(&mut self, from: u32, at: u32, spelled: &[u8], angle: bool) {
        self.0.missing(from, at, spelled, angle);
        self.1.missing(from, at, spelled, angle);
    }
    fn cond(&mut self, file: u32, at: u32, taken: bool) {
        self.0.cond(file, at, taken);
        self.1.cond(file, at, taken);
    }
    fn skipped(&mut self, file: u32, start: u32, end: u32) {
        self.0.skipped(file, start, end);
        self.1.skipped(file, start, end);
    }
    fn expansion(&mut self, e: Expansion<'_>) {
        self.0.expansion(e);
        self.1.expansion(e);
    }
    fn wants_entries(&self) -> bool {
        self.0.wants_entries() || self.1.wants_entries()
    }
    fn entered(&mut self, e: Entered) {
        self.0.entered(e);
        self.1.entered(e);
    }
    fn requests(&mut self, r: &[Request], deps: &[u32]) {
        self.0.requests(r, deps);
        self.1.requests(r, deps);
    }
    fn pragma(&mut self, file: u32, at: u32, text: &[u8]) {
        self.0.pragma(file, at, text);
        self.1.pragma(file, at, text);
    }
    fn diag(&mut self, file: u32, at: u32, d: Diag, text: &[u8]) {
        self.0.diag(file, at, d.clone(), text);
        self.1.diag(file, at, d, text);
    }
}

/// `-E` text: each source line's tokens on a line, white space as one blank, a line marker
/// (`# N "path"`) where the file or line jumps, and pragmas on their own lines.
pub struct TextSink {
    pub out: Vec<u8>,
    /// File paths, by file (told by `enter`).
    paths: Vec<String>,
    /// The output's current file and line.
    file: u32,
    line: u32,
    /// The last token's spelling's last byte and first, and its kind (`P::Eof`: none on the line).
    last: (u8, P),
    started: bool,
}

impl Default for TextSink {
    fn default() -> Self {
        TextSink { out: Vec::new(), paths: Vec::new(), file: 0, line: 0, last: (0, P::Eof), started: false }
    }
}

impl TextSink {
    fn marker(&mut self, file: u32, line: u32) {
        if !self.out.is_empty() && self.out.last() != Some(&b'\n') {
            self.out.push(b'\n');
        }
        let path = self.paths.get(file as usize).map_or("", |s| s.as_str());
        self.out.extend_from_slice(format!("# {line} \"{}\"\n", path.replace('\\', "\\\\").replace('"', "\\\"")).as_bytes());
        self.file = file;
        self.line = line;
        self.last = (0, P::Eof);
    }

    /// Whether `b` after `a` (with no blank) would read as other tokens than `a` and `b`.
    pub(crate) fn would_join(a: (u8, P), b0: u8, b: P) -> bool {
        let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80 || c == b'\\';
        let (a1, ak) = a;
        if ak == P::Eof {
            return false;
        }
        (word(a1) && (word(b0) || matches!(b, P::Str | P::Char | P::RawStr) || b0 == b'.' && ak == P::Number))
            || (ak == P::Number && (matches!(b0, b'.' | b'+' | b'-' | b'\'') || word(b0)))
            || (b"+-*/%<>=&|^!.#:~?".contains(&a1) && b"+-*/%<>=&|^!.#:".contains(&b0))
    }
}

impl Sink for TextSink {
    fn wants_lines(&self) -> bool {
        true
    }

    fn token(&mut self, t: &Tk, text: &[u8], at: Option<At>, _off: u32) {
        if let Some(at) = at {
            if !self.started || at.file != self.file || at.line < self.line || at.line > self.line + 8 {
                self.started = true;
                self.marker(at.file, at.line);
            } else {
                while self.line < at.line {
                    self.out.push(b'\n');
                    self.line += 1;
                }
                self.last = (0, P::Eof);
            }
        } else if t.flags & BOL != 0 && self.last.1 != P::Eof {
            // A line's first token with no line to tell (an expansion's): a blank.
            self.out.push(b' ');
        }
        if self.last.1 != P::Eof && (t.flags & SPACE != 0 || Self::would_join(self.last, text.first().copied().unwrap_or(0), t.kind)) && self.out.last() != Some(&b' ') {
            self.out.push(b' ');
        }
        self.out.extend_from_slice(text);
        self.last = (text.last().copied().unwrap_or(0), t.kind);
    }

    fn enter(&mut self, file: u32, path: &str, _from: Option<(u32, u32)>, _depth: u32) {
        if self.paths.len() <= file as usize {
            self.paths.resize(file as usize + 1, String::new());
        }
        self.paths[file as usize] = path.to_string();
    }

    fn pragma(&mut self, _file: u32, _at: u32, text: &[u8]) {
        if self.out.last().is_some_and(|&c| c != b'\n') {
            self.out.push(b'\n');
        }
        self.out.extend_from_slice(b"#pragma ");
        self.out.extend_from_slice(text);
        self.out.push(b'\n');
        self.line += 1;
        self.last = (0, P::Eof);
    }
}

/// Expansion `id` of macro `name`: where the macro is defined and where it is called (file, position), and the
/// expansion it was called from (`parent`, 0 for none).
#[derive(Clone, Copy, Debug)]
pub struct Expansion<'a> {
    pub id: u32,
    pub name: &'a [u8],
    pub defined: (u32, u32),
    pub called: (u32, u32),
    pub parent: u32,
}

/// An `Expansion` kept (`FactSink`), its name owned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpansionRow {
    pub id: u32,
    pub name: Vec<u8>,
    pub defined: (u32, u32),
    pub called: (u32, u32),
    pub parent: u32,
}

/// Rows (mode (b)'s relations of 0327), by file id; the paths are `paths`.
#[derive(Default, Debug)]
pub struct FactSink {
    pub paths: Vec<String>,
    /// `c_pp_included`: from (file, `#`'s position), to, depth.
    pub included: Vec<(u32, u32, u32, u32)>,
    /// `c_pp_include_wanted`: from (file, position), spelling, angle.
    pub missing: Vec<(u32, u32, Vec<u8>, bool)>,
    /// `c_pp_cond_taken`: (file, position, taken).
    pub conds: Vec<(u32, u32, bool)>,
    /// The skipped ranges (`c_pp_active` is each entered file's bytes less these).
    pub skipped: Vec<(u32, u32, u32)>,
    /// `c_pp_expansion`: each expansion, its name owned.
    pub expansions: Vec<ExpansionRow>,
    pub pragmas: Vec<(u32, u32, Vec<u8>)>,
    pub diags: Vec<(u32, u32, Diag, Vec<u8>)>,
    /// How many tokens were output.
    pub tokens: u64,
}

impl Sink for FactSink {
    fn token(&mut self, _t: &Tk, _text: &[u8], _at: Option<At>, _off: u32) {
        self.tokens += 1;
    }
    fn enter(&mut self, file: u32, path: &str, from: Option<(u32, u32)>, depth: u32) {
        if self.paths.len() <= file as usize {
            self.paths.resize(file as usize + 1, String::new());
        }
        self.paths[file as usize] = path.to_string();
        if let Some((f, at)) = from {
            self.included.push((f, at, file, depth));
        }
    }
    fn missing(&mut self, from: u32, at: u32, spelled: &[u8], angle: bool) {
        self.missing.push((from, at, spelled.to_vec(), angle));
    }
    fn cond(&mut self, file: u32, at: u32, taken: bool) {
        self.conds.push((file, at, taken));
    }
    fn skipped(&mut self, file: u32, start: u32, end: u32) {
        self.skipped.push((file, start, end));
    }
    fn expansion(&mut self, e: Expansion<'_>) {
        self.expansions.push(ExpansionRow { id: e.id, name: e.name.to_vec(), defined: e.defined, called: e.called, parent: e.parent });
    }
    fn pragma(&mut self, file: u32, at: u32, text: &[u8]) {
        self.pragmas.push((file, at, text.to_vec()));
    }
    fn diag(&mut self, file: u32, at: u32, d: Diag, text: &[u8]) {
        self.diags.push((file, at, d, text.to_vec()));
    }
}
