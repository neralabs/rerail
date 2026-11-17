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

    /// An output token, its spelling, and (`wants_lines`) where a line's first token came from.
    fn token(&mut self, t: &Tk, text: &[u8], at: Option<At>) {}

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

    /// Expansion `id` of macro `name` (defined in `def_file` at `def_at`), called at `at` in `file`
    /// (or from expansion `parent`).
    fn expansion(&mut self, id: u32, name: &[u8], def_file: u32, def_at: u32, file: u32, at: u32, parent: u32) {}

    /// A pragma (a `#pragma` line's text after `pragma`, or `_Pragma`'s string).
    fn pragma(&mut self, file: u32, at: u32, text: &[u8]) {}

    /// A diagnostic, at `at` in `file`.
    fn diag(&mut self, file: u32, at: u32, d: Diag, text: &[u8]) {}
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
    fn token(&mut self, t: &Tk, text: &[u8], at: Option<At>) {
        self.0.token(t, text, at);
        self.1.token(t, text, at);
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
    fn expansion(&mut self, id: u32, name: &[u8], def_file: u32, def_at: u32, file: u32, at: u32, parent: u32) {
        self.0.expansion(id, name, def_file, def_at, file, at, parent);
        self.1.expansion(id, name, def_file, def_at, file, at, parent);
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
    fn would_join(a: (u8, P), b0: u8, b: P) -> bool {
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

    fn token(&mut self, t: &Tk, text: &[u8], at: Option<At>) {
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
    /// `c_pp_expansion`: id, name, defined at (file, position), called at (file, position), parent.
    pub expansions: Vec<(u32, Vec<u8>, u32, u32, u32, u32, u32)>,
    pub pragmas: Vec<(u32, u32, Vec<u8>)>,
    pub diags: Vec<(u32, u32, Diag, Vec<u8>)>,
    /// How many tokens were output.
    pub tokens: u64,
}

impl Sink for FactSink {
    fn token(&mut self, _t: &Tk, _text: &[u8], _at: Option<At>) {
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
    fn expansion(&mut self, id: u32, name: &[u8], def_file: u32, def_at: u32, file: u32, at: u32, parent: u32) {
        self.expansions.push((id, name.to_vec(), def_file, def_at, file, at, parent));
    }
    fn pragma(&mut self, file: u32, at: u32, text: &[u8]) {
        self.pragmas.push((file, at, text.to_vec()));
    }
    fn diag(&mut self, file: u32, at: u32, d: Diag, text: &[u8]) {
        self.diags.push((file, at, d, text.to_vec()));
    }
}
