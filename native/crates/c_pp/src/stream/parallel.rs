//! The preprocessor as parallel passes: a spike of the shape (decision 0421).
//!
//! The serial pass (`Pp::run`) reads a translation unit as one machine, from its first token
//! to its last. Here the same unit is read as passes over arrays, each of them a map, a scan, a
//! sort, a search, a gather or a scatter, so that each can run on the device as it is:
//!
//! - **P0, each file once** (`files`): its tokens, its directives with their lines, and its
//!   conditionals' static nesting (chains, arms, groups). A `#define`'s definition is parsed
//!   here, once a file.
//! - **P1, the directives' fixpoint** (`rounds`): over directives only, never text tokens.
//!   Every position is a position in the unit, files included in place (prefix sums over the
//!   include tree). A name at position `p` means the latest active `#define` or `#undef` of it
//!   before `p`: one sort of the definitions by (name, position), then a search per use. Each
//!   test (`#if`, `#ifdef`, `#elif`…) is evaluated at its own position, all of them
//!   independently, and each include resolved. The rounds repeat until the active directives
//!   and the include tree are stable. Every dependency points backwards in the unit, so the
//!   fixpoint is unique and is the serial reading.
//! - **P2, candidates** (`text`): the active text's identifiers that name a macro at their
//!   position, their calls' parentheses matched within their stretch of text.
//! - **P3, invocations, each alone** (`text`): each outermost invocation expanded in isolation
//!   by the serial pass's own expansion, under the definitions of its position. An expansion
//!   that wants the tokens after it (a function-like name before a `(`, or a call left open)
//!   is merged with them and expanded again.
//! - **P4, assembly** (`text`): each piece's count of tokens, a prefix sum, and a scatter.
//!
//! `#pragma push_macro` and `pop_macro` are rows of the same table: a pop's value is the value
//! at its push, matched per name by depth. Pragmas made in text by `_Pragma` (libc++'s
//! `_LIBCPP_PUSH_MACROS`) are found in the rounds, by expanding the invocations that can reach
//! `_Pragma`, and placed between the positions of their invocation and the next token.
//!
//! What the shape does not carry yet sends the unit to the serial pass, with its reason:
//! `#embed`, `#line`, a call across a directive, and `__COUNTER__` or `_Pragma` in a directive.
//!
//! It is a spike: no product path calls it. The serial loops left are the rounds, one
//! invocation's expansion, and the host's reading of files.

use super::{Config, Pp, Src};
use crate::files::Files;
use crate::directives::Kind;
use crate::sink::Null;

mod files;
mod rounds;
mod text;
#[cfg(test)]
mod tests;

/// The definitions by position: `(symbol, position, definition + 1)`, sorted; 0 is an `#undef`.
/// The built-ins are at position 0, before every other.
pub(super) struct Vis {
    rows: Vec<(u32, u64, u32)>,
}

impl Vis {
    /// The definition of `sym` that a use at `pos` sees: the latest row of `sym` before `pos`.
    pub(super) fn at(&self, sym: u32, pos: u64) -> Option<u32> {
        let k = self.rows.partition_point(|&(s, p, _)| (s, p) < (sym, pos));
        let &(s, _, d) = self.rows.get(k.checked_sub(1)?)?;
        if s != sym {
            return None;
        }
        d.checked_sub(1)
    }
}

/// Positions have this many bits below a token's: room for the pragmas an invocation makes,
/// after it and before the next token.
pub(super) const SUB: u32 = 8;

/// A pragma that changes the reading: `once`, `push_macro`, `pop_macro`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Ev {
    Once,
    Push,
    Pop,
}

/// An isolated reading: its list's index in the stack (`usize::MAX`: no list, a directive's
/// line), and what the reading met.
pub(super) struct Isolated {
    pub outer: usize,
    /// A function-like name at the list's end looked for its `(` past it.
    pub peeked: bool,
    /// A call's arguments ran to the list's end.
    pub cut: bool,
    /// The pragmas `_Pragma` made, and their names' symbols (`NO_SYM` for `once`).
    pub events: Vec<(Ev, u32)>,
}

/// The shape of a unit's reading.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    /// P1's rounds, the last one the round that changed nothing.
    pub rounds: u32,
    /// The files read (their tables made), and the files' instances in the include tree at
    /// the end.
    pub files: u32,
    pub instances: u32,
    /// The include tree's depth (the main file's instance is 1).
    pub depth: u32,
    /// Directives of the instances, and those active.
    pub directives: u64,
    pub active_directives: u64,
    /// Tests read (a chain's where its parent group is active, its arms to the first true), and
    /// of them those evaluated: a test whose looked-up definitions are those of the last
    /// round it was evaluated in keeps its value. Includes resolved, over every round.
    pub tests: u64,
    pub evaluated: u64,
    pub includes: u64,
    /// The last round's definitions (rows of the positional table, built-ins left out).
    pub definitions: u64,
    /// Text tokens of the instances, and those active.
    pub tokens: u64,
    pub active_tokens: u64,
    /// P2: identifiers naming a macro where they are.
    pub candidates: u64,
    /// P3: outermost invocations expanded, those distinct by their tokens' spellings, the merges,
    /// the tokens they made, and the longest one's tokens in and out.
    pub invocations: u64,
    pub distinct: u64,
    pub merges: u64,
    pub expanded: u64,
    pub longest_in: u64,
    pub longest_out: u64,
    /// P4: the output's tokens.
    pub output: u64,
}

/// A unit's reading: its output tokens' spellings, or what sends it to the serial pass.
pub type Outcome = Result<Vec<Vec<u8>>, &'static str>;

/// The unit whose main file is `path`, read as passes, under `cfg` and `io`.
pub fn run(cfg: Config, io: &mut dyn Files, path: &str) -> (Outcome, Stats) {
    let mut stats = Stats::default();
    let mut null = Null;
    let mut pp = Pp::new(cfg, io, &mut null);
    let Some(src) = pp.io.read(path) else { return (Err("the main file is not read"), stats) };
    // The predefines are a file of their own, before the main one (`Pp::run`).
    let mut pre = String::new();
    for (n, b) in &pp.cfg.predefines {
        match b {
            Some(b) => pre.push_str(&format!("#define {n} {b}\n")),
            None => pre.push_str(&format!("#undef {n}\n")),
        }
    }
    let builtin = pp.add_file("<built-in>".into(), pre.into_bytes(), None);
    let main = pp.add_file(path.to_string(), src, None);
    pp.base = main;
    // The built-ins, at position 0.
    let builtins: Vec<(u32, u64, u32)> = pp.macros.iter().enumerate().filter(|&(_, &d)| d != 0).map(|(s, &d)| (s as u32, 0, d)).collect();
    let out = (|| {
        let mut tu = rounds::Tu::new(&mut pp, builtin, main);
        let (state, lay) = rounds::settle(&mut pp, &mut tu, &builtins, &mut stats)?;
        text::output(&mut pp, &tu, &state, &lay, &mut stats)
    })();
    (out, stats)
}

/// A directive of a file's P0 tables (`files::tabulate`), as the evaluator's file pass makes it
/// from the file's tape (decision 0436): its tape kind (`tape::kind_id`), the text tokens before
/// it, the group it is read in, the group after it, and its chain (`u32::MAX`: none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirRow {
    pub kind: u16,
    pub text_before: u32,
    pub group: u32,
    pub after: u32,
    pub chain: u32,
    /// The name it defines, undefines or tests (`#ifdef`, `#ifndef`, `#elifdef`, `#elifndef`).
    pub name: Option<Vec<u8>>,
}

/// A file's P0 tables, as the evaluator's file pass makes them: its directives, its groups'
/// number, and its include guard's name.
pub struct Tables {
    pub dirs: Vec<DirRow>,
    pub groups: u32,
    pub guard: Option<Vec<u8>>,
}

/// `src`'s P0 tables, read alone under `opts`: the cross-check of the evaluator's file pass.
pub fn tables(opts: crate::Options, src: &[u8]) -> Tables {
    let mut io = crate::files::Search::new(Vec::new(), |_: &str| None);
    let mut null = Null;
    let mut pp = Pp::new(Config::new(opts), &mut io, &mut null);
    let file = pp.add_file("<file>".into(), src.to_vec(), None);
    let tab = files::tabulate(&mut pp, file);
    let mut chain = vec![u32::MAX; tab.dirs.len()];
    for (c, arms) in tab.chains.iter().enumerate() {
        for &d in arms {
            chain[d as usize] = c as u32;
        }
    }
    let rows = tab
        .dirs
        .iter()
        .enumerate()
        .map(|(d, dir)| {
            let sym = match dir.act {
                files::Act::Define(k) => Some(pp.defs[k as usize].name),
                files::Act::Undef(s) => Some(s),
                files::Act::Test if !matches!(dir.kind, Kind::If | Kind::Elif | Kind::Else) => dir.line.first().filter(|n| n.kind == rerail_c_lean::pp::P::Ident).map(|n| n.sym),
                _ => None,
            };
            let name = sym.map(|s| pp.syms.text(s).to_vec());
            DirRow { kind: crate::tape::kind_id(dir.kind), text_before: dir.text_before, group: dir.group, after: dir.after, chain: chain[d], name }
        })
        .collect();
    Tables { dirs: rows, groups: tab.groups.len() as u32, guard: tab.guard.map(|g| pp.syms.text(g).to_vec()) }
}

/// Runs `f` with `pp` read as at position `pos` of the instance whose files, outermost first,
/// are `chain`: its sources the chain's files (never read: an isolated list above them stops
/// every reading), its definitions those of `pos`.
fn in_context<R>(pp: &mut Pp, chain: &[u32], pos: u64, f: impl FnOnce(&mut Pp) -> R) -> R {
    debug_assert!(pp.stack.is_empty());
    // Their lexers are over no text: nothing reads them, and a lexer over a file's text would
    // classify all of it first.
    for &file in chain {
        let lx = rerail_c_lean::pp::PpLexer::new(&[], pp.cfg.opts);
        pp.stack.push(Src::File { file, lx, conds: Vec::new(), delta: 0, presumed: None });
    }
    pp.vis_pos = pos;
    let r = f(pp);
    for s in pp.stack.drain(..) {
        if let Src::Toks { def: Some(d), .. } = s {
            pp.defs[d as usize].disabled = false;
        }
    }
    r
}
