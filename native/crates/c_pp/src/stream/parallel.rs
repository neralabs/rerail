//! The preprocessor as parallel passes: a spike of the shape (decision 0482).
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

/// A unit's configuration input, as the device's P1 reads it (decision 0487): the predefines'
/// text (a file of its own, `<built-in>`, before the main one, as `Pp::run` reads it) and the
/// built-in macros' names (rows at slot 0).
pub fn predefined(cfg: &Config) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut none = crate::files::Search::new(Vec::new(), |_: &str| None);
    let mut null = Null;
    let pp = Pp::new(cfg.clone(), &mut none, &mut null);
    let mut pre = String::new();
    for (n, b) in &pp.cfg.predefines {
        match b {
            Some(b) => pre.push_str(&format!("#define {n} {b}\n")),
            None => pre.push_str(&format!("#undef {n}\n")),
        }
    }
    let names = pp.macros.iter().enumerate().filter(|&(_, &d)| d != 0).map(|(s, _)| pp.syms.text(s as u32).to_vec()).collect();
    (pre.into_bytes(), names)
}

/// What a name in the text means where it stands (decision 0448, R4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// The token is no identifier.
    NotName,
    /// No macro (none defined, or `#undef`).
    None,
    /// A built-in macro.
    Builtin,
    /// The `#define` at directive `dir` of unit file `file`.
    Def { file: u32, dir: u32 },
}

/// An instance of a file in a unit's include tree, laid out (the test input of 0448's P1, until
/// P1's includes and conditionals run on the device): its file, each directive's slot (a
/// position `SUB` bits down) and whether it is active, and each stretch of text between
/// directives (`k` from 0: before directive `k`, the last after all): its first slot, whether
/// it is active, and its tokens' range in the file's text.
#[derive(Clone, Debug)]
pub struct P1Inst {
    pub file: u32,
    /// Its parent instance (an index of `P1::insts`; `u32::MAX` for a root), the parent's
    /// directive that entered it (a root's rank: 0 the predefines, 1 the main file), and its
    /// depth in the include tree (a root's 0).
    pub parent: u32,
    pub dir: u32,
    pub depth: u32,
    pub dir_slots: Vec<u64>,
    pub dir_active: Vec<bool>,
    pub gap_slots: Vec<u64>,
    pub gap_active: Vec<bool>,
    pub gap_tokens: Vec<(u32, u32)>,
}

/// A unit's P1 state at its fixpoint, as the spike reads it: its files' sources (by the spike's
/// file id; 0 is the predefines), its instances in the include tree's preorder, and every token
/// of every active stretch in the unit's order (by slot), with what it means where it stands.
pub struct P1 {
    pub files: Vec<Vec<u8>>,
    /// Each file's path as the unit opened it (`__FILE__`'s presumed path, with no `#line`).
    pub paths: Vec<String>,
    /// The built-in macros' names: rows at slot 0, before every other.
    pub builtins: Vec<Vec<u8>>,
    pub insts: Vec<P1Inst>,
    pub lookups: Vec<Lookup>,
}

/// The unit whose main file is `path`, read by the spike's P1, under `cfg` and `io`; or why it
/// is not one the device's expansion reads yet (push and pop rows, pragmas made in text).
pub fn p1_of(cfg: Config, io: &mut dyn Files, path: &str) -> Result<P1, &'static str> {
    let mut stats = Stats::default();
    let mut null = Null;
    let mut pp = Pp::new(cfg, io, &mut null);
    let src = pp.io.read(path).ok_or("the main file is not read")?;
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
    let builtins: Vec<(u32, u64, u32)> = pp.macros.iter().enumerate().filter(|&(_, &d)| d != 0).map(|(s, &d)| (s as u32, 0, d)).collect();
    let mut tu = rounds::Tu::new(&mut pp, builtin, main);
    let (state, lay) = rounds::settle(&mut pp, &mut tu, &builtins, &mut stats)?;
    // Each definition's directive.
    let mut def_at = std::collections::HashMap::new();
    for (file, tab) in tu.tabs.iter().enumerate() {
        for (d, dir) in tab.iter().flat_map(|t| t.dirs.iter()).enumerate() {
            if let files::Act::Define(k) = dir.act {
                def_at.insert(k, (file as u32, d as u32));
            }
        }
    }
    let vis = pp.vis.as_ref().ok_or("no positional table")?;
    // The active stretches' tokens' meanings, by slot: the unit's order.
    let (mut insts, mut by_slot) = (Vec::new(), Vec::new());
    // Each instance's parent and directive, by its place in the order.
    let place: std::collections::HashMap<u32, u32> = lay.order.iter().enumerate().map(|(k, &i)| (i, k as u32)).collect();
    let mut parent_of: std::collections::HashMap<u32, (u32, u32)> = std::collections::HashMap::new();
    for &i in &lay.order {
        for (d, &c) in state.child[i as usize].iter().enumerate() {
            if c != rounds::NONE {
                parent_of.insert(c, (i, d as u32));
            }
        }
    }
    let mut depth_of: Vec<u32> = Vec::new();
    for &i in &lay.order {
        let tab = tu.tab_of(i);
        if !state.events[i as usize].is_empty() {
            return Err("pragmas made in text");
        }
        let n = tab.dirs.len();
        let (parent, dir, depth) = match parent_of.get(&i) {
            Some(&(p, d)) => (place[&p], d, depth_of[place[&p] as usize] + 1),
            None => (u32::MAX, u32::from(i == tu.roots[1]), 0),
        };
        depth_of.push(depth);
        let mut inst = P1Inst { file: tu.insts[i as usize].file, parent, dir, depth, dir_slots: Vec::new(), dir_active: Vec::new(), gap_slots: Vec::new(), gap_active: Vec::new(), gap_tokens: Vec::new() };
        // A push or a pop is a row of the device's table (decision 0487 §3), as `vis` reads it.
        for d in 0..n {
            let active = state.active(tab, i, d);
            inst.dir_slots.push(lay.dir_pos(tab, i, d) >> SUB);
            inst.dir_active.push(active);
        }
        for k in 0..=n {
            let group = if k == 0 { 0 } else { tab.dirs[k - 1].after };
            let first = if k == 0 { 0 } else { tab.dirs[k - 1].text_before };
            let end = if k < n { tab.dirs[k].text_before } else { tab.text.len() as u32 };
            let pos = lay.text_pos(tab, i, k);
            let active = state.groups[i as usize][group as usize];
            inst.gap_slots.push(pos >> SUB);
            inst.gap_active.push(active);
            inst.gap_tokens.push((first, end));
            if active {
                let mut lookups = Vec::new();
                for t in &tab.text[first as usize..end as usize] {
                    lookups.push(if t.kind != rerail_c_lean::pp::P::Ident {
                        Lookup::NotName
                    } else {
                        match vis.at(t.sym, pos) {
                            None => Lookup::None,
                            Some(d) if pp.defs[d as usize].builtin.is_some() => Lookup::Builtin,
                            Some(d) => {
                                let &(file, dir) = def_at.get(&d).ok_or("a definition of no directive")?;
                                Lookup::Def { file, dir }
                            }
                        }
                    });
                }
                by_slot.push((pos, lookups));
            }
        }
        insts.push(inst);
    }
    by_slot.sort_by_key(|g| g.0);
    let lookups = by_slot.into_iter().flat_map(|g| g.1).collect();
    let files = pp.files.iter().map(|f| f.src.to_vec()).collect();
    let mut names = Vec::new();
    for &(sym, _, d) in &builtins {
        if pp.defs[d as usize - 1].builtin.is_none() {
            return Err("a definition before the predefines");
        }
        names.push(pp.syms.text(sym).to_vec());
    }
    let paths = pp.files.iter().map(|f| f.path.clone()).collect();
    Ok(P1 { files, paths, builtins: names, insts, lookups })
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
    /// A well-formed `#define`'s definition, as `Pp::parse_define` makes it.
    pub def: Option<DefView>,
}

/// A definition as the evaluator's setup makes it from the tape (decision 0448, R2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefView {
    pub fnlike: bool,
    pub variadic: bool,
    /// The parameters' spellings (`__VA_ARGS__` for a `...` of its own).
    pub params: Vec<Vec<u8>>,
    /// The body's tokens: spelling, its parameter's index, a paste operator, white space before it.
    pub body: Vec<(Vec<u8>, Option<usize>, bool, bool)>,
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
            let def = match dir.act {
                files::Act::Define(k) => {
                    let def = &pp.defs[k as usize];
                    Some(DefView {
                        fnlike: def.fnlike,
                        variadic: def.variadic,
                        params: def.params.iter().map(|&p| pp.syms.text(p).to_vec()).collect(),
                        body: def.body.iter().map(|t| (pp.text(t).to_vec(), def.param(t.sym), t.flags & crate::macros::PASTE != 0, t.flags & crate::macros::SPACE != 0)).collect(),
                    })
                }
                _ => None,
            };
            DirRow { kind: crate::tape::kind_id(dir.kind), text_before: dir.text_before, group: dir.group, after: dir.after, chain: chain[d], name, def }
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
