//! Rules in `.rr`, the engine's rule language: Soufflé Datalog with the additions of
//! `docs/rule-syntax/lib/core.rr` (decision 0043). Every rule the engine runs is read here.
//!
//! - `syntax`: tokens and statements as written;
//! - `lower`: statements to the engine's `Program`, the form the planner, magic sets and the backends take;
//! - `print`: a `Program` as `.rr` text, read back as the same program;
//! - `check`: the checks of `docs/rule-syntax/site/check_rr.py` (grounded, declared once, stratified);
//! - `translate`: the earlier `.rdl` dialect's text in `.rr`, comments kept, for the move and for the converter.
//!
//! `#include "x.rr"` reads `docs/rule-syntax/lib/x.rr`, or else `x.rr` beside the including file; each file once.
//! A file read with `parse_file` has `core.rr` before it, included or not. An included file's `.output`s are its own,
//! not the program's: they are left out (`python/calls.rr`'s).

pub use rerail_rr::code_rule_dsl::logic::rr::check;
#[path = "rr/aggregate_demand"]
pub mod aggregate_demand {
    pub use rerail_rr::code_rule_dsl::logic::rr::aggregate_demand::*;
    #[cfg(any(test, feature = "test-hooks"))]
    pub mod tests;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::logic::front::{self, AggOp, Atom, Literal, Program, Rule, Term};
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, BTreeSet};
}
pub mod from_tape;
pub use rerail_rr::code_rule_dsl::logic::rr::lower;
/// The text tests `.rr` names, and the engine's test each lowers to (the coverage gate probes each).
pub use rerail_rr::code_rule_dsl::logic::rr::lower::TESTS as RR_TESTS;
pub use rerail_rr::code_rule_dsl::logic::rr::print;
// The parser and the tape's tree, the `.rr` parser crate's (`rerail_rr_syntax`).
use rerail_rr_syntax::syntax;
pub use rerail_rr_syntax::tape;

/// `src`'s tape as nodes: the lean parser's image (decision 0140) read back (`tape::nodes`), what
/// the statements are read off as a stored tape's are (decision 0062).
pub fn nodes(src: &str) -> Vec<tape::Node> {
    tape::nodes(&tape(src)).expect("the lean parser's image decodes")
}
#[path = "rr"]
pub mod tape_type {
    pub use rerail_rr::code_rule_dsl::logic::rr::tape_type::*;
    #[cfg(any(test, feature = "test-hooks"))]
    #[path = "tape_type_tests.rs"]
    pub mod tests;
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, BTreeSet};
    #[allow(unused_imports)]
    use rerail_rr_syntax::syntax::{Head, Item, Lit, Statement, Term};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::logic::front::Error;
}
pub mod token_reads;
pub use rerail_rr::code_rule_dsl::logic::rr::translate;

// cratify: `tape_tests` is rerail-engine's (it mounts it)
// cratify: `tests` is rerail-engine's (it mounts it)

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub use super::front::Error;
use super::front::Program;
#[cfg(any(test, feature = "test-hooks"))]
pub use lower::drop_unread_core;
pub use aggregate_demand::retype_helpers;
pub use lower::is_core;
pub use lower::WORLD;
pub use print::print;
pub use syntax::quote;
pub use translate::translate;

/// The relations `core.rr` declares: the engine's names (decision 0047). Empty when `core.rr` does not read.
pub fn core_names() -> &'static std::collections::BTreeSet<String> {
    static CORE: std::sync::OnceLock<std::collections::BTreeSet<String>> = std::sync::OnceLock::new();
    CORE.get_or_init(|| {
        std::fs::read_to_string(library().join("core.rr")).ok().and_then(|src| parse(&src).ok()).map(|p| p.decls.into_keys().collect()).unwrap_or_default()
    })
}

/// The library `#include` reads from.
pub fn library() -> PathBuf {
    // `RERAIL_RR_LIB` names another copy: the browser build (native/wasm) mounts the library in its own filesystem.
    std::env::var_os("RERAIL_RR_LIB")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join("../docs/rule-syntax/lib"))
}

/// A program's `.rr` text. Its `#include`s name library files (decision 0089), read as `parse_file` reads them:
/// `core.rr` first unless the text is in T's shape, each file once, includes first. A text that includes nothing is
/// read as it is, without `core.rr`.
pub fn parse(src: &str) -> Result<Program, Error> {
    parse_with(src, true)
}

/// A program's `.rr` text as `parse` reads it, but with only its libraries' declarations, not their clauses: what a
/// run that holds the libraries once (decision 0089 §5) needs of a program that adds to no library relation. Its own
/// rules are lowered against the library's names; the library's rules are the run's, lowered once.
pub fn parse_own(src: &str) -> Result<Program, Error> {
    parse_with(src, false)
}

// A run handed a program it read is handed the program itself (`corpus::run_read`), not its print to lower again.
fn parse_with(src: &str, clauses: bool) -> Result<Program, Error> {
    read_text(src)?.lower(clauses)
}

/// A program text read once (its tape, its statements, its includes resolved), for what a run needs of it: its
/// library modules (`modules`) and its program (`lower`). A run of a pack asks both of every program; each read of
/// a text makes its tape and resolves its includes again.
pub struct Text(Read);

enum Read {
    /// A text that includes nothing: read as it is.
    Plain(Vec<syntax::Statement>),
    /// A text's library files (includes first, without their includes: canonical path, bytes' digest, statements as
    /// `read` keeps them) and its own statements.
    Modular { files: Vec<(PathBuf, [u8; 32], std::sync::Arc<Vec<syntax::Statement>>)>, own: Vec<syntax::Statement> },
}

/// `src` read once, its includes resolved (`Text`).
pub fn read_text(src: &str) -> Result<Text, Error> {
    let statements = from_tape::statements(src, &nodes(src))?;
    if !statements.iter().any(|(item, _)| matches!(item, syntax::Item::Include(_))) {
        refuse_ungrounded(&statements, &BTreeSet::new())?;
        return Ok(Text(Read::Plain(statements)));
    }
    let (files, own) = resolve_shared(statements)?;
    let given: BTreeSet<String> = files.iter().flat_map(|(_, _, s)| check::fix_relations(s)).collect();
    refuse_ungrounded(&own, &given)?;
    Ok(Text(Read::Modular { files, own }))
}

/// A program's own rules must be grounded (`check::grounded`, which the library's files pass by test): every head
/// variable, and every variable a negation, comparison or aggregate reads from outside, bound by a positive literal
/// of the body. An ungrounded rule otherwise fails late and far from its cause — "unplaced literals", "$x is unbound
/// in SQL", a statement over SQLite's 65535 references — or, when no rule reads the unbound column, is silently
/// projected away.
fn refuse_ungrounded(statements: &[syntax::Statement], given: &BTreeSet<String>) -> Result<(), Error> {
    // A program in T's shape (it declares `doc` itself: a lowered program printed and read again, `combine`) holds
    // the lowering's own rules, which are not source rules: only a source text is checked.
    if statements.iter().any(|(item, _)| matches!(item, syntax::Item::Decl { name, .. } if name == "doc")) {
        return Ok(());
    }
    let bad = check::grounded(statements, given);
    if bad.is_empty() {
        return Ok(());
    }
    let lines: Vec<String> = bad.iter().take(8).map(|(line, vars)| format!("line {line}: {}", vars.join(", "))).collect();
    Err(Error::Syntax(format!(
        "{} ungrounded rule{} (a variable no positive body literal binds): {}{}",
        bad.len(),
        if bad.len() == 1 { "" } else { "s" },
        lines.join("; "),
        if bad.len() > 8 { "; …" } else { "" }
    )))
}

impl Text {
    /// The text's library modules (`modules`).
    pub fn modules(&self) -> Option<Modules> {
        let Read::Modular { files, own } = &self.0 else { return None };
        let declares = |statements: &[syntax::Statement]| -> std::collections::BTreeSet<String> {
            statements
                .iter()
                .filter_map(|(item, _)| match item {
                    syntax::Item::Decl { name, .. } | syntax::Item::Violation { name, .. } => Some(name.clone()),
                    _ => None,
                })
                .collect()
        };
        let heads = own
            .iter()
            .flat_map(|(item, _)| match item {
                syntax::Item::Clause { heads, edits, .. } if edits.is_empty() => heads.iter().map(|h| h.rel.clone()).collect(),
                _ => Vec::new(),
            })
            .collect();
        Some(Modules { files: files.iter().map(|(path, digest, statements)| (path.clone(), *digest, declares(statements))).collect(), heads, declared: declares(own) })
    }

    /// The text's program, as `parse` (`clauses`) or `parse_own` reads it.
    pub fn lower(self, clauses: bool) -> Result<Program, Error> {
        let (files, own) = match self.0 {
            Read::Plain(statements) => return lower::lower(statements),
            Read::Modular { files, own } => (files, own),
        };
        // The text's own rules and declarations; its facts may be the runner's (`env`, decision 0089 §1).
        let checked: Vec<syntax::Statement> = own.iter().filter(|(item, _)| !matches!(item, syntax::Item::Clause { body, .. } if body.is_empty())).cloned().collect();
        // `core.rr`'s names, unless the text is in T's shape and reads no `core.rr` (`resolve`).
        let reads_core = files.first().is_some_and(|(path, _, _)| path.ends_with("core.rr"));
        check_names(&vec![(PathBuf::from("<program>"), checked)], reads_core)?;
        // Lowering reads one kind of clause statically: a kind set's (`fscope("module")`, `lfscope(k) :- fscope(k)`),
        // whose relation has one column. Those are kept; the rest are the run's. Only what is kept is copied.
        let unary: std::collections::BTreeSet<&str> = if clauses {
            Default::default()
        } else {
            files
                .iter()
                .flat_map(|(_, _, statements)| statements.iter())
                .filter_map(|(item, _)| match item {
                    syntax::Item::Decl { name, cols, .. } if cols.len() == 1 => Some(name.as_str()),
                    _ => None,
                })
                .collect()
        };
        let kept = |item: &syntax::Item| match item {
            syntax::Item::Include(_) => false,
            syntax::Item::Clause { heads, .. } => clauses || heads.iter().all(|h| unary.contains(h.rel.as_str())),
            _ => true,
        };
        let mut statements: Vec<syntax::Statement> = files.iter().flat_map(|(_, _, statements)| statements.iter().filter(|(item, _)| kept(item)).cloned()).collect();
        statements.extend(own);
        lower::lower(statements)
    }
}

/// A program text's library modules (decision 0089): the files its `#include`s name, as `parse` reads them, and
/// what the text itself declares and derives. `None` for a text that includes nothing (a printed program).
#[derive(Clone, Debug)]
pub struct Modules {
    /// Each library file, includes first: its canonical path, its bytes' digest and the relations it declares.
    pub files: Vec<(PathBuf, [u8; 32], std::collections::BTreeSet<String>)>,
    /// The relations the text's own clauses derive or state facts of.
    pub heads: std::collections::BTreeSet<String>,
    /// The relations the text itself declares.
    pub declared: std::collections::BTreeSet<String>,
}

pub fn modules(src: &str) -> Result<Option<Modules>, Error> {
    Ok(read_text(src)?.modules())
}

/// The library files `paths` (canonical, includes first, as `Modules::files` lists them) as one program, lowered
/// once for every program of a run that includes them (decision 0089). Kept for the process by the files' digests.
pub(in crate::code_rule_dsl) fn lower_library(paths: &[PathBuf]) -> Result<Program, Error> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static LIBRARIES: OnceLock<Mutex<HashMap<Vec<(PathBuf, [u8; 32])>, Program>>> = OnceLock::new();
    let mut files = Vec::new();
    let mut key = Vec::new();
    for path in paths {
        let (statements, file) = read(path)?;
        key.push((path.clone(), file.sha256.unwrap_or_default()));
        files.push((path.clone(), statements.into_iter().filter(|(item, _)| !matches!(item, syntax::Item::Include(_))).collect()));
    }
    let cache = LIBRARIES.get_or_init(Default::default);
    if let Some(program) = cache.lock().expect("the lowered libraries").get(&key) {
        crate::code_rule_dsl::prof::count("rr.library_reused", 1);
        return Ok(program.clone());
    }
    check_names(&files, false)?;
    let program = lower::lower(files.into_iter().flat_map(|(_, statements): (PathBuf, Vec<syntax::Statement>)| statements).collect())?;
    let mut kept = cache.lock().expect("the lowered libraries");
    if kept.len() >= 8 {
        kept.clear();
    }
    kept.insert(key, program.clone());
    Ok(program)
}

/// A program text read once for what a run needs of it besides its rules (`plan::corpus::run_with_options`): its
/// statements, and the seed phase 0 derives its include graph from. Once every text's graph is derived
/// (`phase0::graphs`, one run for all of them), [`TextRead::resolved`] gives its library files, the relations it
/// declares and the phase it names. The whole-run cutoff, its record and the texts' digest take these; none reads a
/// text again (each parsed a pack's 278 texts again, about eight times a process).
pub struct TextRead {
    /// `None` for a text that includes nothing, or while phase 0 itself runs (`text_seed`).
    pub seed: Option<super::plan::corpus::phase0::Seed>,
    statements: Vec<syntax::Statement>,
}

/// What [`TextRead::resolved`] reads off a text: its library files (includes resolved, `core.rr` first unless the
/// text is in T's shape, and the engine's own library programs), the relations it and its includes declare, and the
/// phase it names itself (`.phase`).
pub struct TextNames {
    pub included: Vec<PathBuf>,
    pub names: Vec<String>,
    pub phase: Option<String>,
}

/// `src` parsed, and its seed (`TextRead`).
pub fn read_seed(src: &str) -> Result<TextRead, Error> {
    use super::plan::corpus::phase0;
    let statements = from_tape::statements(src, &nodes(src))?;
    let includes: Vec<(i64, String)> = statements
        .iter()
        .filter_map(|(item, _)| match item {
            syntax::Item::Include(name) => Some(name.clone()),
            _ => None,
        })
        .enumerate()
        .map(|(k, n)| (k as i64, n))
        .collect();
    let seed = (!includes.is_empty() && !phase0::active()).then(|| {
        let t_shape = statements.iter().any(|(item, _)| matches!(item, syntax::Item::Decl { name, .. } if name == "doc"));
        phase0::Seed::Text { t_shape, includes }
    });
    Ok(TextRead { seed, statements })
}

impl TextRead {
    /// The text's library files, names and phase, its includes resolved in the graph phase 0 derived for its seed.
    pub fn resolved(self) -> Result<TextNames, Error> {
        let (files, own) = if self.statements.iter().any(|(item, _)| matches!(item, syntax::Item::Include(_))) { resolve(self.statements)? } else { (Vec::new(), self.statements) };
        let (mut names, mut phase) = (Vec::new(), None);
        for (item, _) in files.iter().flat_map(|(_, st)| st.iter()).chain(own.iter()) {
            match item {
                syntax::Item::Decl { name, .. } => names.push(name.clone()),
                syntax::Item::Setting { key, text } if key == "phase" && phase.is_none() => phase = Some(text.trim().to_owned()),
                _ => {}
            }
        }
        names.sort();
        names.dedup();
        let mut included: Vec<PathBuf> = files.into_iter().map(|(p, _)| p).collect();
        // The engine's own library programs, which the runner adds to a run (decision 0076's components).
        included.extend(ENGINE_FILES.iter().filter_map(|f| library().join(f).canonicalize().ok()));
        Ok(TextNames { included, names, phase })
    }
}

/// The library files a program's text reads (`TextNames::included`): what a record of the run must check besides
/// the text (phase 0, t23). For a text read alone; a run reads its programs once (`read_seed`).
pub fn included_files(src: &str) -> Result<Vec<PathBuf>, Error> {
    Ok(read_seed(src)?.resolved()?.included)
}

/// The relations a program's text declares, its includes' with its own, and the phase it names itself
/// (`TextNames`): for a text read alone (a phase program found by name, `corpus::phases::phase_files`).
pub fn declared_names(src: &str) -> Result<(Vec<String>, Option<String>), Error> {
    let t = read_seed(src)?.resolved()?;
    Ok((t.names, t.phase))
}

/// `statements`' `#include`s resolved in the library: its files (`core.rr` first unless the statements are in T's
/// shape, each once, includes first) and the statements without the includes.
///
/// Phase 0 resolves them (`plan::corpus::phase0`): `bootstrap.rr`'s run loads each file as a document, and the
/// files are read here off their stored tapes, in the graph's order.
fn resolve(statements: Vec<syntax::Statement>) -> Result<(Files, Vec<syntax::Statement>), Error> {
    let (files, own) = resolve_shared(statements)?;
    Ok((files.into_iter().map(|(path, _, statements)| (path, statements.iter().filter(|(item, _)| !matches!(item, syntax::Item::Include(_))).cloned().collect())).collect(), own))
}

/// `resolve`, each file's statements as `read` keeps them (its includes still in them) with its bytes' digest.
type SharedFiles = Vec<(PathBuf, [u8; 32], std::sync::Arc<Vec<syntax::Statement>>)>;

fn resolve_shared(statements: Vec<syntax::Statement>) -> Result<(SharedFiles, Vec<syntax::Statement>), Error> {
    use super::plan::corpus::phase0;
    if phase0::active() {
        let (files, own) = builtin_resolve(statements)?;
        let mut out = Vec::new();
        for (path, statements) in files {
            let (_, file) = read_shared(&path)?;
            out.push((path, file.sha256.unwrap_or_default(), std::sync::Arc::new(statements)));
        }
        return Ok((out, own));
    }
    let (includes, own): (Vec<_>, Vec<_>) = statements.into_iter().partition(|(item, _)| matches!(item, syntax::Item::Include(_)));
    let t_shape = own.iter().any(|(item, _)| matches!(item, syntax::Item::Decl { name, .. } if name == "doc"));
    let names: Vec<(i64, String, usize)> = includes
        .iter()
        .enumerate()
        .filter_map(|(k, (item, line))| match item {
            syntax::Item::Include(name) => Some((k as i64, name.clone(), *line)),
            _ => None,
        })
        .collect();
    let g = phase0::graph(&phase0::Seed::Text { t_shape, includes: names.iter().map(|(k, n, _)| (*k, n.clone())).collect() })?;
    if let Some((_, at, name)) = g.missing.iter().find(|(f, _, _)| f == phase0::TEXT) {
        let line = names.iter().find(|(k, _, _)| k == at).map_or(0, |(_, _, l)| *l);
        return Err(Error::Syntax(format!("line {line}: #include {name:?}: no such file in the library")));
    }
    let mut order = Vec::new();
    g.order(phase0::TEXT, &mut order)?;
    let files = order
        .into_iter()
        .map(|p| {
            let (statements, file) = read_shared(&p)?;
            Ok((p, file.sha256.unwrap_or_default(), statements))
        })
        .collect::<Result<_, Error>>()?;
    Ok((files, own))
}

/// Each module's statements, without its includes (the graph placed them), read off its stored tape.

/// The seed phase 0 starts from for a program text: whether it is in T's shape, and the names it includes. `None`
/// for a text that includes nothing, or while phase 0 itself runs.
pub fn text_seed(src: &str) -> Result<Option<super::plan::corpus::phase0::Seed>, Error> {
    use super::plan::corpus::phase0;
    // An include is the lexer's `#include` (`syntax::Tok::Hash`): a text without those letters includes nothing.
    // A printed program, its includes resolved, is not read for it; its parse errors are its parse's to report.
    if phase0::active() || !src.contains("#include") {
        return Ok(None);
    }
    // Read once a run: the run hands its texts' graph digests to its combines (`corpus::run_handed`).
    read_text_seed(src)
}

fn read_text_seed(src: &str) -> Result<Option<super::plan::corpus::phase0::Seed>, Error> {
    use super::plan::corpus::phase0;
    // The letters may stand in a string, a pattern or a comment, which are not directives: the lexer's tokens say,
    // before another syntax tree is made for a program without includes.
    let lexed = syntax::lex(src);
    if lexed.error.is_none() && !lexed.toks.iter().any(|(t,_,_)| matches!(t, syntax::Tok::Hash(_))) {
        return Ok(None);
    }
    let statements = from_tape::statements(src, &nodes(src))?;
    let names: Vec<(i64, String)> = statements
        .iter()
        .filter_map(|(item, _)| match item {
            syntax::Item::Include(name) => Some(name.clone()),
            _ => None,
        })
        .enumerate()
        .map(|(k, n)| (k as i64, n))
        .collect();
    if names.is_empty() || phase0::active() {
        return Ok(None);
    }
    let t_shape = statements.iter().any(|(item, _)| matches!(item, syntax::Item::Decl { name, .. } if name == "doc"));
    Ok(Some(phase0::Seed::Text { t_shape, includes: names }))
}

/// The program's module graph's digest (`phase0::Graph::digest`): what its text is a function of besides itself.
/// `None` for a text that includes nothing.
pub(in crate::code_rule_dsl) fn graph_digest(src: &str) -> Result<Option<[u8; 32]>, Error> {
    match text_seed(src)? {
        Some(seed) => Ok(Some(super::plan::corpus::phase0::graph(&seed)?.digest())),
        None => Ok(None),
    }
}

/// The resolution phase 0's own program (`bootstrap.rr`) is read with: the library, then beside the includer.
fn builtin_resolve(statements: Vec<syntax::Statement>) -> Result<(Files, Vec<syntax::Statement>), Error> {
    let (includes, own): (Vec<_>, Vec<_>) = statements.into_iter().partition(|(item, _)| matches!(item, syntax::Item::Include(_)));
    let core = library().join("core.rr").canonicalize().map_err(|e| Error::Syntax(format!("core.rr: {e}")))?;
    let t_shape = own.iter().any(|(item, _)| matches!(item, syntax::Item::Decl { name, .. } if name == "doc"));
    let (mut out, mut read_files, mut seen) = (Vec::new(), Vec::new(), Vec::new());
    if t_shape {
        seen.push(core);
    } else {
        visit(&core, &mut out, &mut read_files, &mut seen, false)?;
    }
    for (item, line) in includes {
        let syntax::Item::Include(name) = item else { unreachable!("an include") };
        let path = library().join(&name);
        if !path.exists() {
            return Err(Error::Syntax(format!("line {line}: #include {name:?}: no such file in the library")));
        }
        visit(&path, &mut out, &mut read_files, &mut seen, true)?;
    }
    Ok((out, own))
}


/// The first error a `.rr` file's parse gives, as running it would report it: its line and message
/// (`syntax_error/3`). `None` when it parses.
pub fn syntax_error(src: &str) -> Option<(usize, String)> {
    let error = match syntax::Parser::new(src).and_then(|mut parser| parser.statements()) {
        Ok(_) => return None,
        Err(error) => error,
    };
    let mut parser = syntax::Parser::taped(src);
    let _ = parser.statements();
    let at = parser.into_tape().1.map_or(0, |(at, _)| at);
    let rerail_rr_syntax::Error::Syntax(message) = error;
    Some((syntax::line_of(src, at), message))
}

/// A `.rr` string literal's value, as the rule engine reads it (`"a\nb"` is `a`, newline, `b`); `None` for text
/// that is no whole string literal.
pub(in crate::code_rule_dsl) fn string_value(text: &str) -> Option<String> {
    match syntax::tokens(text).ok()?.as_slice() {
        [(syntax::Tok::Str(value), 0, end)] if *end == text.len() => Some(value.clone()),
        _ => None,
    }
}

/// A backtick literal's body, without its fences (and, for a fence of two or more, the one space inside each).
pub(in crate::code_rule_dsl) fn code_body(text: &str) -> &str {
    let n = text.bytes().take_while(|&b| b == b'`').count();
    let body = text.get(n..text.len().saturating_sub(n)).unwrap_or("");
    if n > 1 { body.strip_prefix(' ').unwrap_or(body).strip_suffix(' ').unwrap_or(body) } else { body }
}

/// The compiler's named policy headers and settings, without reparsing rule source in a host adapter.
pub fn metadata(program: &Program) -> Result<serde_json::Value, Error> {
    use syntax::Tok;
    let mut policies = program.policies.clone();
    for (_, text) in program.settings.iter().filter(|(key, _)| key == "policy") {
        let tokens = syntax::tokens(text)?;
        let Some((Tok::Str(id), _, _)) = tokens.first() else {
            return Err(Error::Syntax("policy metadata requires a quoted identifier".to_owned()));
        };
        let revision = tokens.windows(2).find_map(|pair| match (&pair[0].0, &pair[1].0) {
            (Tok::Id(key), Tok::Num(n)) if key == "revision" => Some(*n),
            _ => None,
        });
        policies.push(serde_json::json!({"id": id, "revision": revision}));
    }
    Ok(serde_json::json!({"policies": policies, "settings": program.settings}))
}

/// A lowered program run over files a caller gives (a test's), not found by walking a repository: its
/// `.scope` filter on findings is taken out, as `.scope given` leaves it out when lowering.
pub fn files_given(program: &mut Program) {
    let filter = ["rr_scope_drop", "rr_scope_in", "rr_report_in"];
    program.rules.retain(|r| !filter.contains(&r.head.rel.as_str()));
    for r in &mut program.rules {
        r.body.retain(|l| !l.atom().is_some_and(|a| filter.contains(&a.rel.as_str())));
    }
    filter.iter().for_each(|f| {
        program.decls.remove(*f);
    });
    program.settings.push(("scope".to_owned(), "given".to_owned()));
    program.doc_scopes = None;
}

/// A program's `.rr` file, with what it includes.
pub fn parse_file(path: &Path) -> Result<Program, Error> {
    read_file(path).map(|(program, _)| program)
}

/// A library file a program read: decision 0044's seeded fact `library_file(path, hash)`. `sha256`
/// is of the file's bytes, or `None` where an `#include` looked and found no file (a file
/// appearing there changes what the program reads).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryFile {
    pub path: PathBuf,
    pub sha256: Option<[u8; 32]>,
}

impl LibraryFile {
    /// The file as it is now, by phase 0's observer: one `stat`, and a read only when it changed.
    fn now(path: &Path) -> Self {
        Self { path: path.to_owned(), sha256: super::plan::corpus::observe(|o| o.digest(path)) }
    }

    /// The seeded fact, for a file that exists.
    // Read by the runner's record when it seeds the store (0044 step 2).
    #[allow(dead_code)]
    pub fn fact(&self) -> Option<String> {
        let hash = self.sha256?;
        let hex = hash.iter().map(|b| format!("{b:02x}")).collect::<String>();
        Some(format!("library_file({}, \"sha256:{hex}\").", quote(&self.path.to_string_lossy())))
    }
}

/// The program of a `.rr` file and the library files it read or probed, in reading order (core.rr, then
/// each file before what it includes).
///
/// The lowered program is a derivation of those files (0044 §4): it is kept for the process and
/// reused while every file it read hashes the same, so only an edit to a file it read (or a
/// file appearing where an include looked) reads and lowers it again.
/// The relations `core.rr` declares for libraries to derive: the engine reads them (decision 0055's embeds).
/// And the documents they load (t21 E2).
const LIBRARY_DERIVED: [&str; 3] = ["embed", "embed_value", "load"];
/// The library's programs the engine adds to a run itself (decision 0076's import components).
const ENGINE_FILES: [&str; 1] = ["components.rr"];

pub fn read_file(path: &Path) -> Result<(Program, Vec<LibraryFile>), Error> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static PROGRAMS: OnceLock<Mutex<HashMap<PathBuf, (Vec<LibraryFile>, Program)>>> = OnceLock::new();
    let key = path.canonicalize().map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    let cache = PROGRAMS.get_or_init(Default::default);
    if let Some((files, program)) = cache.lock().expect("the program cache").get(&key)
        && files.iter().all(|file| LibraryFile::now(&file.path) == *file)
    {
        crate::code_rule_dsl::prof::count("rr.program_reused", 1);
        return Ok((program.clone(), files.clone()));
    }
    // The kept statements alone copied from the process's (`read_shared`): each policy copied its libraries whole,
    // to drop most of them (`unreached_dropped`).
    let (shared, files) = program_shared(path)?;
    let own = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    let statements = reached_statements(&own, &shared.iter().collect::<Vec<_>>());
    let program = lower_files_dropping(path, statements, false)?;
    cache.lock().expect("the program cache").insert(key, (files.clone(), program.clone()));
    Ok((program, files))
}

/// A file's program from its modules' statements (`program`): its names checked, its own rules grounded, and lowered,
/// as `read_file` reads it. Testing mode (`rr::testing`, decision 0130) gives the file's statements with a test's
/// spliced in.
pub fn lower_files(path: &Path, statements: Files) -> Result<Program, Error> {
    lower_files_dropping(path, statements, true)
}

/// `lower_files`, with the unreached clauses of included files dropped first (`drop`) or lowered too.
pub fn lower_files_dropping(path: &Path, statements: Files, drop: bool) -> Result<Program, Error> {
    let key = path.canonicalize().map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    check_names(&statements, false)?;
    // The file's own rules grounded (`refuse_ungrounded`), its includes' `fix` relations given.
    let own_path = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    let given: BTreeSet<String> = statements.iter().filter(|(p, _)| *p != own_path).flat_map(|(_, s)| check::fix_relations(s)).collect();
    for (_, own) in statements.iter().filter(|(p, _)| *p == own_path) {
        refuse_ungrounded(own, &given)?;
    }
    // The included files' clauses nothing in the program reaches: dropped before they are checked and lowered.
    let statements = if drop { unreached_dropped(&own_path, statements) } else { statements };
    token_reads::refuse(&statements)?;
    // The relations an included file declares are the library's: canonical, one copy however many policies of a
    // combined run include it (decision 0091). Printed as `.global library …`, so a program's text keeps that.
    let core = library().join("core.rr").canonicalize().map_err(|e| Error::Syntax(format!("core.rr: {e}")))?;
    let library: Vec<String> = statements
        .iter()
        .filter(|(file, _)| *file != core && *file != key)
        .flat_map(|(_, statements)| statements)
        .filter_map(|(item, _)| if let syntax::Item::Decl { name, .. } = item { Some(name.clone()) } else { None })
        .collect();
    let mut program = lower::lower(statements.into_iter().flat_map(|(_, statements)| statements).collect())?;
    if !library.is_empty() {
        program.settings.push((super::front::GLOBAL.to_owned(), format!("{} {}", super::front::LIBRARY, library.join(", "))));
    }
    Ok(program)
}

/// `statements` less the included files' clauses that nothing in the program reaches. Of the 44 data-layer
/// policies' rules, 76% were lowered (with the libraries' whole) and then pruned (`front::prune`): reading a policy
/// was the largest part of a batch's setup (`lower_files`, 16% of the main thread). Kept whole: the file itself,
/// `core.rr` and the engine's files (lowering reads their relations by name), and every item but a clause. A clause
/// of another file is kept when one of its heads is reached: named by the file itself (each identifier of each of
/// its items, so none is missed), by another file's output, violation or setting, or read by a kept clause. Every clause for a relation is kept with any, so a library relation is the same in each program that
/// reaches it (decision 0091). `front::prune` of the program lowered from these is the same as of the whole
/// (`dropping_unreached_clauses_prunes_to_the_same_program`).
pub fn unreached_dropped(own: &Path, statements: Files) -> Files {
    reached_statements(own, &statements.iter().map(|(p, s)| (p, s.iter().collect())).collect::<Vec<_>>())
}

/// `unreached_dropped` of statements held elsewhere (`read_shared`'s): only the kept are copied.
fn reached_statements(own: &Path, statements: &[(&PathBuf, Vec<&syntax::Statement>)]) -> Files {
    fn idents(text: &str, out: &mut BTreeSet<String>) {
        out.extend(text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|w| w.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')).map(str::to_owned));
    }
    fn term(t: &syntax::Term, out: &mut BTreeSet<String>) {
        match t {
            syntax::Term::Call { name, args, .. } => {
                out.insert(name.clone());
                args.iter().for_each(|a| term(a, out));
            }
            syntax::Term::Bin { l, r, .. } => {
                term(l, out);
                term(r, out);
            }
            syntax::Term::Agg { over, by, body, .. } => {
                over.iter().for_each(|o| term(o, out));
                by.iter().for_each(|b| term(b, out));
                lits(body, out);
            }
            syntax::Term::Named(_, t) => term(t, out),
            // A relation may be an argument (`nearest(n, p, block_node)`, a set of nodes): every name in a term may
            // be one.
            syntax::Term::Json(text) | syntax::Term::Str(text) | syntax::Term::Tick(text) => idents(text, out),
            syntax::Term::Var(v) => {
                out.insert(v.clone());
            }
            _ => {}
        }
    }
    fn lits(body: &[syntax::Lit], out: &mut BTreeSet<String>) {
        for l in body {
            match l {
                syntax::Lit::Atom { rel, args, .. } => {
                    out.insert(rel.clone());
                    args.iter().for_each(|a| term(a, out));
                }
                syntax::Lit::Cmp { l, r, .. } => {
                    term(l, out);
                    term(r, out);
                }
                syntax::Lit::Pattern { subject, .. } => term(subject, out),
                syntax::Lit::Group { alts, .. } => alts.iter().for_each(|a| lits(a, out)),
            }
        }
    }
    let whole: Vec<PathBuf> = ["core.rr"].iter().chain(ENGINE_FILES.iter()).filter_map(|f| library().join(f).canonicalize().ok()).chain(std::iter::once(own.to_owned())).collect();
    let mut reached: BTreeSet<String> = BTreeSet::new();
    // Each other file's clauses by head: (file, statement) indices.
    let mut by_head: std::collections::HashMap<&str, Vec<(usize, usize)>> = std::collections::HashMap::new();
    for (f, (file, items)) in statements.iter().enumerate() {
        let kept_whole = whole.contains(file);
        for (s, (item, _)) in items.iter().copied().enumerate() {
            match item {
                syntax::Item::Clause { heads, .. } if !kept_whole => heads.iter().for_each(|h| by_head.entry(h.rel.as_str()).or_default().push((f, s))),
                syntax::Item::Clause { heads, body, edits } => {
                    heads.iter().for_each(|h| {
                        reached.insert(h.rel.clone());
                        h.args.iter().for_each(|a| term(a, &mut reached));
                    });
                    lits(body, &mut reached);
                    edits.iter().for_each(|e| term(e, &mut reached));
                }
                // What another file declares reads nothing (`.decl`, `.input`, `.stored`, `.functor`, `.type`, `.edit`):
                // its relations are reached only when read. Its outputs, violations and settings are read.
                syntax::Item::Decl { .. } | syntax::Item::Functor { .. } | syntax::Item::Type { .. } | syntax::Item::Edit { .. } | syntax::Item::Lattice { .. } | syntax::Item::Include(_) | syntax::Item::Io { output: false, .. } if !kept_whole => {}
                other => idents(&format!("{other:?}"), &mut reached),
            }
        }
    }
    // The engine's relations (`core.rr`'s) are read by the engine: a library clause deriving one asks it for
    // something (`load(f, q, "python")`, python/modules.rr), though no rule reads it.
    let core = library().join("core.rr").canonicalize().ok();
    let listed: Vec<&String> = statements
        .iter()
        .filter(|(file, _)| core.as_ref() == Some(*file))
        .flat_map(|(_, items)| items.iter())
        .filter_map(|(item, _)| if let syntax::Item::Decl { name, .. } = item { Some(name) } else { None })
        .collect();
    if listed.is_empty() {
        reached.extend(core_names().iter().cloned());
    } else {
        reached.extend(listed.into_iter().cloned());
    }
    // A request rule: its head is read by no clause, but its body reads the file system (`dir_entry`, `file_line`,
    // `file_digest`), which asks the engine for what it reads (`py_listed(d) :- py_file_at(_, _, d, _),
    // dir_entry(d, _, _)`, python/modules.rr). It is kept once what it reads is reached.
    let mut read_by_some: BTreeSet<String> = BTreeSet::new();
    for (item, _) in statements.iter().flat_map(|(_, items)| items.iter()) {
        if let syntax::Item::Clause { body, .. } = item {
            lits(body, &mut read_by_some);
        }
    }
    let mut askers: std::collections::HashMap<&str, Vec<(usize, usize)>> = std::collections::HashMap::new();
    for (f, (file, items)) in statements.iter().enumerate() {
        if whole.contains(file) {
            continue;
        }
        for (s, (item, _)) in items.iter().copied().enumerate() {
            let syntax::Item::Clause { heads, body, .. } = item else { continue };
            if heads.iter().any(|h| read_by_some.contains(&h.rel)) {
                continue;
            }
            for l in body {
                if let syntax::Lit::Atom { rel, .. } = l
                    && let Some((world, _)) = lower::WORLD.iter().find(|(w, _)| *w == rel.as_str())
                {
                    askers.entry(world).or_default().push((f, s));
                }
            }
        }
    }
    let mut keep: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut todo: Vec<String> = reached.iter().cloned().collect();
    while let Some(rel) = todo.pop() {
        for &(f, s) in by_head.get(rel.as_str()).into_iter().flatten().chain(askers.get(rel.as_str()).into_iter().flatten()) {
            if !keep.insert((f, s)) {
                continue;
            }
            let syntax::Item::Clause { heads, body, edits } = &statements[f].1[s].0 else { continue };
            let mut more = BTreeSet::new();
            heads.iter().for_each(|h| {
                more.insert(h.rel.clone());
                h.args.iter().for_each(|a| term(a, &mut more));
            });
            lits(body, &mut more);
            edits.iter().for_each(|e| term(e, &mut more));
            todo.extend(more.into_iter().filter(|r| reached.insert(r.clone())));
        }
    }
    statements
        .iter()
        .enumerate()
        .map(|(f, (file, items))| {
            let kept_whole = whole.contains(*file);
            let items = items.iter().copied().enumerate().filter(|(s, (item, _))| kept_whole || !matches!(item, syntax::Item::Clause { .. }) || keep.contains(&(f, *s))).map(|(_, i)| i.clone()).collect();
            ((*file).clone(), items)
        })
        .collect()
}

/// The names a program's files may not declare or derive: `core.rr`'s, but in `core.rr` (decision 0047), and names
/// starting `rr_`, but in the engine's own library programs.
fn check_names(statements: &Files, core_unlisted: bool) -> Result<(), Error> {
    // `core.rr`'s relations are the engine's (decision 0047): no other file declares or derives one.
    let core = library().join("core.rr").canonicalize().map_err(|e| Error::Syntax(format!("core.rr: {e}")))?;
    // From `core.rr` among the files, else (`core_unlisted`: the program reads it) as `core_names` reads it.
    let listed = statements.iter().any(|(file, _)| *file == core);
    let engine: std::collections::BTreeSet<&String> = statements
        .iter()
        .filter(|(file, _)| *file == core)
        .flat_map(|(_, statements)| statements)
        .filter_map(|(item, _)| if let syntax::Item::Decl { name, .. } = item { Some(name) } else { None })
        .chain(core_names().iter().filter(|_| core_unlisted && !listed))
        .filter(|name| !LIBRARY_DERIVED.contains(&name.as_str()))
        .collect();
    for (file, statements) in statements.iter().filter(|(file, _)| *file != core) {
        for (item, line) in statements {
            let names: Vec<&String> = match item {
                syntax::Item::Decl { name, .. } => vec![name],
                syntax::Item::Clause { heads, .. } => heads.iter().map(|h| &h.rel).collect(),
                _ => Vec::new(),
            };
            if let Some(name) = names.into_iter().find(|n| engine.contains(n)) {
                return Err(Error::Syntax(format!("{}:{line}: {name} is core.rr's: a program reads it and cannot declare or derive it", file.display())));
            }
        }
    }
    // Names starting `rr_` are the engine's (`front::RESERVED`): its helpers, axes and places. The engine's own
    // library programs (`ENGINE_FILES`, which the runner adds; no rule includes them) may use them.
    let own: Vec<PathBuf> = ENGINE_FILES.iter().filter_map(|f| library().join(f).canonicalize().ok()).collect();
    for (file, statements) in statements.iter().filter(|(file, _)| !own.contains(file)) {
        for (item, line) in statements {
            let names: Vec<&String> = match item {
                syntax::Item::Decl { name, .. } | syntax::Item::Violation { name, .. } => vec![name],
                syntax::Item::Clause { heads, .. } => heads.iter().map(|h| &h.rel).collect(),
                _ => Vec::new(),
            };
            if let Some(name) = names.into_iter().find(|n| n.starts_with(super::front::RESERVED)) {
                return Err(Error::Syntax(format!("{}:{line}: {name}: names starting `{}` are the engine's", file.display(), super::front::RESERVED)));
            }
        }
    }
    Ok(())
}

thread_local! {
    /// The library files read or probed while `recording` runs on this thread.
    static READS: std::cell::RefCell<Option<Vec<LibraryFile>>> = const { std::cell::RefCell::new(None) };
}

/// `f`'s result and every library file it read or probed, misses included, each once in reading order: what a
/// program read from the library, so a later run can tell whether reading it again would read the same (decision
/// 0094 §6: the loader records what it read).
pub(in crate::code_rule_dsl) fn recording<T>(f: impl FnOnce() -> T) -> (T, Vec<LibraryFile>) {
    let outer = READS.with(|r| r.borrow_mut().replace(Vec::new()));
    let out = f();
    let read = READS.with(|r| std::mem::replace(&mut *r.borrow_mut(), outer)).unwrap_or_default();
    // An enclosing recording sees what this one read too.
    READS.with(|r| {
        if let Some(outer) = r.borrow_mut().as_mut() {
            outer.extend(read.iter().cloned());
        }
    });
    let mut seen = std::collections::HashSet::new();
    (out, read.into_iter().filter(|f| seen.insert(f.path.clone())).collect())
}

fn noted(file: &LibraryFile) {
    READS.with(|r| {
        if let Some(read) = r.borrow_mut().as_mut() {
            read.push(file.clone());
        }
    });
}

impl LibraryFile {
    /// Whether the file still reads as recorded: the same bytes, or still no file.
    pub(in crate::code_rule_dsl) fn unchanged(&self) -> bool {
        Self::now(&self.path) == *self
    }
}

pub fn read(path: &Path) -> Result<(Vec<syntax::Statement>, LibraryFile), Error> {
    read_shared(path).map(|(statements, file)| ((*statements).clone(), file))
}

/// A library file's statements as kept for the process, and the file as read.
fn read_shared(path: &Path) -> Result<(std::sync::Arc<Vec<syntax::Statement>>, LibraryFile), Error> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    // Each file's statements, kept for the process by its path and bytes' digest: programs that include one library
    // (the Python pack's files) parse it once, not once per program. Phase 0's observer gives the digest by a `stat`
    // while the file is unchanged, so a kept file is not read again. Held shared: a reader copies what it keeps.
    static PARSED: OnceLock<Mutex<HashMap<PathBuf, ([u8; 32], Arc<Vec<syntax::Statement>>)>>> = OnceLock::new();
    let cache = PARSED.get_or_init(Default::default);
    if let Some(digest) = super::plan::corpus::observe(|o| o.digest(path))
        && let Some((kept, statements)) = cache.lock().expect("the parsed files").get(path)
        && *kept == digest
    {
        crate::code_rule_dsl::prof::count("rr.file_reused", 1);
        let file = LibraryFile { path: path.to_owned(), sha256: Some(digest) };
        noted(&file);
        return Ok((Arc::clone(statements), file));
    }
    let (bytes, digest) = super::plan::corpus::observe(|o| o.read(path)).ok_or_else(|| Error::Syntax(format!("{}: cannot read the file", path.display())))?;
    let file = LibraryFile { path: path.to_owned(), sha256: Some(digest) };
    noted(&file);
    let src = String::from_utf8(bytes).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    let statements = Arc::new(stored_statements(path, src).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?);
    cache.lock().expect("the parsed files").insert(path.to_owned(), (digest, Arc::clone(&statements)));
    Ok((statements, file))
}

/// A rule file's statements, read off its tape (decision 0062). The tape is the content store's, as any corpus
/// file's is (`facts::core::build_cached`, keyed by the source's content and the grammar's version): a file read by an
/// earlier run is not parsed again. Without a store (the browser build, `RERAIL_TAPE_CACHE=off`) the tape is made
/// here.
/// `src` (a document named `name`) as a program, read off the tape the store keeps for it, as a rule file's is
/// (`stored_statements`): a prepared program's document (decision 0074).
pub(in crate::code_rule_dsl) fn parse_stored(name: &str, src: String) -> Result<Program, Error> {
    lower::lower(stored_statements(Path::new(name), src)?)
}

fn stored_statements(_path: &Path, src: String) -> Result<Vec<syntax::Statement>, Error> {
    let store = if cfg!(feature = "tape-cache") { super::plan::corpus::phases::reading_store() } else { None };
    if let Some(store) = store {
        let src = std::sync::Arc::new(src);
        let file = crate::code_rule_dsl::facts::core::Source { language: "rr".to_owned(), source: std::sync::Arc::clone(&src) };
        if let Ok(crate::code_rule_dsl::facts::core::Built::Facts(facts)) = crate::code_rule_dsl::facts::core::build_cached(&file, &store) {
            crate::code_rule_dsl::prof::count("rr.statements_from_stored_tape", 1);
            return from_tape::statements(&src, &from_tape::nodes_of_facts(&facts));
        }
        return from_tape::statements(&src, &nodes(&src));
    }
    from_tape::statements(&src, &nodes(&src))
}

pub type Files = Vec<(PathBuf, Vec<syntax::Statement>)>;

/// `path` and what it includes, each once (`seen`), includes first, added to `out`; every library file read or
/// probed, to `read_files`. An `included` file's `.output`s are left out, as are those of what it includes.
fn visit(path: &Path, out: &mut Files, read_files: &mut Vec<LibraryFile>, seen: &mut Vec<PathBuf>, included: bool) -> Result<(), Error> {
    let path = path.canonicalize().map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    if seen.contains(&path) {
        return Ok(());
    }
    seen.push(path.clone());
    let (statements, file) = read(&path)?;
    read_files.push(file);
    let (includes, mut rest): (Vec<_>, Vec<_>) = statements.into_iter().partition(|(item, _)| matches!(item, syntax::Item::Include(_)));
    if included {
        rest.retain(|(item, _)| !matches!(item, syntax::Item::Io { output: true, .. }));
    }
    for (item, line) in includes {
        let syntax::Item::Include(name) = item else { unreachable!("an include") };
        let in_library = library().join(&name);
        let beside = path.parent().map(|d| d.join(&name)).unwrap_or_default();
        let found = if in_library.exists() {
            in_library
        } else {
            let miss = LibraryFile { path: in_library, sha256: None };
            noted(&miss);
            read_files.push(miss);
            beside
        };
        if !found.exists() {
            return Err(Error::Syntax(format!("{}:{line}: #include {name:?}: no such file", path.display())));
        }
        visit(&found, out, read_files, seen, true)?;
    }
    out.push((path, rest));
    Ok(())
}

/// The files of `path`'s program, each once, includes first (`path` last); and every library file read or probed,
/// in reading order.
fn program_order(path: &Path) -> Result<(Vec<PathBuf>, Vec<LibraryFile>), Error> {
    use super::plan::corpus::phase0;
    let path = path.canonicalize().map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    let g = phase0::graph(&phase0::Seed::File(path.clone()))?;
    let mut order = Vec::new();
    g.order(&path.to_string_lossy(), &mut order)?;
    order.retain(|p| *p != path);
    order.push(path.clone());
    // The files in reading order, as `LibraryFile`s check them: `core.rr` (an include at offset -1), then the
    // file, then each include, its library probe first where it was found beside the file.
    let library = library();
    let mut files = Vec::new();
    let mut seen = Vec::new();
    let key = path.to_string_lossy().into_owned();
    let (first, rest): (Vec<_>, Vec<_>) = g.includes.get(&key).cloned().unwrap_or_default().into_iter().partition(|(at, _, _)| *at < 0);
    for (_, _, target) in first {
        trace(&g, &library, &target, &mut files, &mut seen);
    }
    seen.push(path.clone());
    files.push(LibraryFile { path: path.clone(), sha256: g.digests.get(&path).copied() });
    for (_, name, target) in rest {
        probe(&library, &name, &target, &mut files);
        trace(&g, &library, &target, &mut files, &mut seen);
    }
    Ok((order, files))
}

/// A file and everything it includes, each once, includes first: (path, statements without the
/// includes, the included files' `.output`s left out); and every library file read or probed, in reading order.
pub fn program(path: &Path) -> Result<(Files, Vec<LibraryFile>), Error> {
    let (shared, files) = program_shared(path)?;
    Ok((shared.iter().map(|(p, s)| (p.clone(), s.iter().map(|s| (*s).clone()).collect())).collect(), files))
}

/// The modules of a program (`program_order`), each its statements as the process holds them (`read_shared`) and
/// those a program reads of them: without its includes, and an included file without its `.output`s (its own, not
/// the program's; `core.rr` is the prelude, read as the file is).
struct Shared(Vec<(PathBuf, std::sync::Arc<Vec<syntax::Statement>>)>);

impl Shared {
    fn iter(&self) -> impl Iterator<Item = (&PathBuf, Vec<&syntax::Statement>)> {
        let core = library().join("core.rr").canonicalize().ok();
        let own = self.0.last().map(|(p, _)| p.clone());
        self.0.iter().map(move |(p, statements)| {
            let included = Some(p) != own.as_ref() && Some(p) != core.as_ref();
            (p, statements.iter().filter(|(item, _)| !matches!(item, syntax::Item::Include(_)) && !(included && matches!(item, syntax::Item::Io { output: true, .. }))).collect())
        })
    }
}

fn program_shared(path: &Path) -> Result<(Shared, Vec<LibraryFile>), Error> {
    if super::plan::corpus::phase0::active() {
        let (modules, files) = builtin_program(path)?;
        return Ok((Shared(modules.into_iter().map(|(p, s)| (p, std::sync::Arc::new(s))).collect()), files));
    }
    let (order, files) = program_order(path)?;
    // One handle on the tape cache for the files read (`phases::Reading`), not one each.
    let _reading = super::plan::corpus::phases::Reading::enter();
    let modules = order.into_iter().map(|p| Ok((p.clone(), read_shared(&p)?.0))).collect::<Result<Vec<_>, Error>>()?;
    Ok((Shared(modules), files))
}

/// `path` and what it includes, each once, as phase 0 found them: the reading order of the builtin reader.
fn trace(g: &super::plan::corpus::phase0::Graph, library: &Path, path: &Path, files: &mut Vec<LibraryFile>, seen: &mut Vec<PathBuf>) {
    if seen.iter().any(|p| p == path) {
        return;
    }
    seen.push(path.to_path_buf());
    files.push(LibraryFile { path: path.to_path_buf(), sha256: g.digests.get(path).copied() });
    for (_, name, target) in g.includes.get(&*path.to_string_lossy()).cloned().unwrap_or_default() {
        probe(library, &name, &target, files);
        trace(g, library, &target, files, seen);
    }
}

/// The library candidate an include probed and did not find, when its file is the one beside the includer.
fn probe(library: &Path, name: &str, target: &Path, files: &mut Vec<LibraryFile>) {
    let in_library = library.join(name);
    if in_library.canonicalize().ok().as_deref() != Some(target) {
        files.push(LibraryFile { path: in_library, sha256: None });
    }
}

/// `program` for phase 0's own reads.
fn builtin_program(path: &Path) -> Result<(Files, Vec<LibraryFile>), Error> {
    let (mut out, mut read_files) = (Vec::new(), Vec::new());
    // `core.rr` is every program's prelude (the base relations and built-ins), included or not; except a program in
    // T's shape (decision 0046), which declares its base relations itself (`doc` among them), and whose libraries
    // read only the relations both shapes have.
    let t_shape = read(&path.canonicalize().map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?)?
        .0
        .iter()
        .any(|(item, _)| matches!(item, syntax::Item::Decl { name, .. } if name == "doc"));
    if !t_shape {
        visit(&library().join("core.rr"), &mut out, &mut read_files, &mut Vec::new(), false)?;
    }
    let mut seen = vec![library().join("core.rr").canonicalize().map_err(|e| Error::Syntax(format!("core.rr: {e}")))?];
    visit(path, &mut out, &mut read_files, &mut seen, false)?;
    Ok((out, read_files))
}

pub use crate::code_rule_dsl::facts::vtab::syntax::core_rr::single_metavars;
