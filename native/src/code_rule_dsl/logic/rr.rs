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

mod check;
mod from_tape;
mod lower;
mod print;
mod syntax;
pub(in crate::code_rule_dsl) mod tape;
mod token_reads;
mod translate;

#[cfg(test)]
mod tape_tests;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

pub(in crate::code_rule_dsl) use super::front::Error;
use super::front::Program;
#[cfg(test)]
pub(in crate::code_rule_dsl) use lower::drop_unread_core;
pub(in crate::code_rule_dsl) use lower::is_core;
pub(in crate::code_rule_dsl) use lower::WORLD;
pub(in crate::code_rule_dsl) use print::print;
pub(in crate::code_rule_dsl) use syntax::quote;
pub(in crate::code_rule_dsl) use translate::translate;

/// The relations `core.rr` declares: the engine's names (decision 0047). Empty when `core.rr` does not read.
pub(in crate::code_rule_dsl) fn core_names() -> &'static std::collections::BTreeSet<String> {
    static CORE: std::sync::OnceLock<std::collections::BTreeSet<String>> = std::sync::OnceLock::new();
    CORE.get_or_init(|| {
        std::fs::read_to_string(library().join("core.rr")).ok().and_then(|src| parse(&src).ok()).map(|p| p.decls.into_keys().collect()).unwrap_or_default()
    })
}

/// The library `#include` reads from.
pub(in crate::code_rule_dsl) fn library() -> PathBuf {
    // `RERAIL_RR_LIB` names another copy: the browser build (native/wasm) mounts the library in its own filesystem.
    std::env::var_os("RERAIL_RR_LIB")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/lib"))
}

/// A program's `.rr` text. Its `#include`s name library files (decision 0089), read as `parse_file` reads them:
/// `core.rr` first unless the text is in T's shape, each file once, includes first. A text that includes nothing is
/// read as it is, without `core.rr`.
pub(in crate::code_rule_dsl) fn parse(src: &str) -> Result<Program, Error> {
    parse_with(src, true)
}

/// A program's `.rr` text as `parse` reads it, but with only its libraries' declarations, not their clauses: what a
/// run that holds the libraries once (decision 0089 §5) needs of a program that adds to no library relation. Its own
/// rules are lowered against the library's names; the library's rules are the run's, lowered once.
pub(in crate::code_rule_dsl) fn parse_own(src: &str) -> Result<Program, Error> {
    parse_with(src, false)
}

fn parse_with(src: &str, clauses: bool) -> Result<Program, Error> {
    let statements = from_tape::statements(src, &tape(src))?;
    if !statements.iter().any(|(item, _)| matches!(item, syntax::Item::Include(_))) {
        return lower::lower(statements);
    }
    let (mut files, own) = resolve(statements)?;
    // The text's own rules and declarations; its facts may be the runner's (`env`, decision 0089 §1).
    let checked: Vec<syntax::Statement> = own.iter().filter(|(item, _)| !matches!(item, syntax::Item::Clause { body, .. } if body.is_empty())).cloned().collect();
    // `core.rr`'s names, unless the text is in T's shape and reads no `core.rr` (`resolve`).
    let reads_core = files.first().is_some_and(|(path, _)| path.ends_with("core.rr"));
    check_names(&vec![(PathBuf::from("<program>"), checked)], reads_core)?;
    // Lowering reads one kind of clause statically: a kind set's (`fscope("module")`, `lfscope(k) :- fscope(k)`),
    // whose relation has one column. Those are kept; the rest are the run's.
    if !clauses {
        let unary: std::collections::BTreeSet<String> = files
            .iter()
            .flat_map(|(_, statements)| statements)
            .filter_map(|(item, _)| match item {
                syntax::Item::Decl { name, cols, .. } if cols.len() == 1 => Some(name.clone()),
                _ => None,
            })
            .collect();
        for (_, statements) in &mut files {
            statements.retain(|(item, _)| match item {
                syntax::Item::Clause { heads, .. } => heads.iter().all(|h| unary.contains(&h.rel)),
                _ => true,
            });
        }
    }
    files.push((PathBuf::from("<program>"), own));
    lower::lower(files.into_iter().flat_map(|(_, statements)| statements).collect())
}

/// A program text's library modules (decision 0089): the files its `#include`s name, as `parse` reads them, and
/// what the text itself declares and derives. `None` for a text that includes nothing (a printed program).
#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) struct Modules {
    /// Each library file, includes first: its canonical path, its bytes' digest and the relations it declares.
    pub files: Vec<(PathBuf, [u8; 32], std::collections::BTreeSet<String>)>,
    /// The relations the text's own clauses derive or state facts of.
    pub heads: std::collections::BTreeSet<String>,
    /// The relations the text itself declares.
    pub declared: std::collections::BTreeSet<String>,
}

pub(in crate::code_rule_dsl) fn modules(src: &str) -> Result<Option<Modules>, Error> {
    let statements = from_tape::statements(src, &tape(src))?;
    if !statements.iter().any(|(item, _)| matches!(item, syntax::Item::Include(_))) {
        return Ok(None);
    }
    let (files, own) = resolve(statements)?;
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
    let mut out = Vec::new();
    for (path, statements) in &files {
        let (_, file) = read(path)?;
        out.push((path.clone(), file.sha256.unwrap_or_default(), declares(statements)));
    }
    Ok(Some(Modules { files: out, heads, declared: declares(&own) }))
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

/// The library files a program's text reads, includes resolved (`core.rr` first unless the text is in T's shape):
/// what a record of the run must check besides the text (phase 0, t23). Empty for a text that includes nothing.
pub(in crate::code_rule_dsl) fn included_files(src: &str) -> Result<Vec<PathBuf>, Error> {
    let statements = from_tape::statements(src, &tape(src))?;
    let mut paths: Vec<PathBuf> = if statements.iter().any(|(item, _)| matches!(item, syntax::Item::Include(_))) {
        resolve(statements)?.0.into_iter().map(|(p, _)| p).collect()
    } else {
        Vec::new()
    };
    // The engine's own library programs, which the runner adds to a run (decision 0076's components).
    paths.extend(ENGINE_FILES.iter().filter_map(|f| library().join(f).canonicalize().ok()));
    Ok(paths)
}

/// `statements`' `#include`s resolved in the library: its files (`core.rr` first unless the statements are in T's
/// shape, each once, includes first) and the statements without the includes.
///
/// Phase 0 resolves them (`plan::corpus::phase0`): `bootstrap.rr`'s run loads each file as a document, and the
/// files are read here off their stored tapes, in the graph's order.
fn resolve(statements: Vec<syntax::Statement>) -> Result<(Files, Vec<syntax::Statement>), Error> {
    use super::plan::corpus::phase0;
    if phase0::active() {
        return builtin_resolve(statements);
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
    Ok((modules_of(order)?, own))
}

/// Each module's statements, without its includes (the graph placed them), read off its stored tape.
fn modules_of(paths: Vec<PathBuf>) -> Result<Files, Error> {
    paths
        .into_iter()
        .map(|p| {
            let (statements, _) = read(&p)?;
            Ok((p, statements.into_iter().filter(|(item, _)| !matches!(item, syntax::Item::Include(_))).collect()))
        })
        .collect()
}

/// The seed phase 0 starts from for a program text: whether it is in T's shape, and the names it includes. `None`
/// for a text that includes nothing, or while phase 0 itself runs.
pub(in crate::code_rule_dsl) fn text_seed(src: &str) -> Result<Option<super::plan::corpus::phase0::Seed>, Error> {
    use super::plan::corpus::phase0;
    let statements = from_tape::statements(src, &tape(src))?;
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

/// A `.rr` file's tree, as its tape holds it (decision 0061): its nodes in preorder, comments as extras, and an ERROR
/// leaf for each statement that does not parse. Never fails: a file the engine refuses to run still has a tape.
pub(in crate::code_rule_dsl) fn tape(src: &str) -> Vec<tape::Node> {
    let mut parser = syntax::Parser::taped(src);
    // A taped parse reads on past an error; what it could not read is in the tree.
    let _ = parser.statements();
    parser.into_tape().0
}

/// The first error a `.rr` file's parse gives, as running it would report it: its line and message
/// (`syntax_error/3`). `None` when it parses.
pub(in crate::code_rule_dsl) fn syntax_error(src: &str) -> Option<(usize, String)> {
    let error = match syntax::Parser::new(src).and_then(|mut parser| parser.statements()) {
        Ok(_) => return None,
        Err(error) => error,
    };
    let mut parser = syntax::Parser::taped(src);
    let _ = parser.statements();
    let at = parser.into_tape().1.map_or(0, |(at, _)| at);
    let message = match error {
        Error::Syntax(message) => message,
        other => format!("{other:?}"),
    };
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
pub(in crate::code_rule_dsl) fn metadata(program: &Program) -> Result<serde_json::Value, Error> {
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
pub(in crate::code_rule_dsl) fn files_given(program: &mut Program) {
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
pub(in crate::code_rule_dsl) fn parse_file(path: &Path) -> Result<Program, Error> {
    read_file(path).map(|(program, _)| program)
}

/// A library file a program read: decision 0044's seeded fact `library_file(path, hash)`. `sha256`
/// is of the file's bytes, or `None` where an `#include` looked and found no file (a file
/// appearing there changes what the program reads).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct LibraryFile {
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
    pub(in crate::code_rule_dsl) fn fact(&self) -> Option<String> {
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

pub(in crate::code_rule_dsl) fn read_file(path: &Path) -> Result<(Program, Vec<LibraryFile>), Error> {
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
    let (statements, files) = program(path)?;
    check_names(&statements, false)?;
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
    cache.lock().expect("the program cache").insert(key, (files.clone(), program.clone()));
    Ok((program, files))
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

fn read(path: &Path) -> Result<(Vec<syntax::Statement>, LibraryFile), Error> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    // Each file's statements, kept for the process by its path and bytes' digest: programs that include one library
    // (the Python pack's files) parse it once, not once per program. Phase 0's observer gives the digest by a `stat`
    // while the file is unchanged, so a kept file is not read again.
    static PARSED: OnceLock<Mutex<HashMap<PathBuf, ([u8; 32], Vec<syntax::Statement>)>>> = OnceLock::new();
    let cache = PARSED.get_or_init(Default::default);
    if let Some(digest) = super::plan::corpus::observe(|o| o.digest(path))
        && let Some((kept, statements)) = cache.lock().expect("the parsed files").get(path)
        && *kept == digest
    {
        crate::code_rule_dsl::prof::count("rr.file_reused", 1);
        let file = LibraryFile { path: path.to_owned(), sha256: Some(digest) };
        noted(&file);
        return Ok((statements.clone(), file));
    }
    let (bytes, digest) = super::plan::corpus::observe(|o| o.read(path)).ok_or_else(|| Error::Syntax(format!("{}: cannot read the file", path.display())))?;
    let file = LibraryFile { path: path.to_owned(), sha256: Some(digest) };
    noted(&file);
    let src = String::from_utf8(bytes).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    let statements = stored_statements(path, src).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    cache.lock().expect("the parsed files").insert(path.to_owned(), (digest, statements.clone()));
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

fn stored_statements(path: &Path, src: String) -> Result<Vec<syntax::Statement>, Error> {
    let store = if cfg!(feature = "tape-cache") { super::plan::corpus::tape_cache().ok().flatten() } else { None };
    if let Some(store) = store {
        let src = std::sync::Arc::new(src);
        let file = crate::code_rule_dsl::model::FileInput::new(path.display().to_string(), "rr".to_owned(), std::sync::Arc::clone(&src), None, None, "core")
            .map_err(|e| Error::Syntax(format!("{e:?}")))?;
        if let Ok(crate::code_rule_dsl::facts::core::Built::Facts(facts)) = crate::code_rule_dsl::facts::core::build_cached(&file, &store) {
            crate::code_rule_dsl::prof::count("rr.statements_from_stored_tape", 1);
            return from_tape::statements(&src, &from_tape::nodes_of_facts(&facts));
        }
        return from_tape::statements(&src, &tape(&src));
    }
    from_tape::statements(&src, &tape(&src))
}

type Files = Vec<(PathBuf, Vec<syntax::Statement>)>;

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

/// A file and everything it includes, each once, includes first: (path, statements without the
/// includes, the included files' `.output`s left out); and every library file read or probed, in reading order.
fn program(path: &Path) -> Result<(Files, Vec<LibraryFile>), Error> {
    use super::plan::corpus::phase0;
    if phase0::active() {
        return builtin_program(path);
    }
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
    // An included file's `.output`s are its own, not the program's; `core.rr` is the prelude, read as the file is.
    let core = library.join("core.rr").canonicalize().ok();
    let mut modules = modules_of(order)?;
    for (p, statements) in &mut modules {
        if *p != path && Some(&*p) != core.as_ref() {
            statements.retain(|(item, _)| !matches!(item, syntax::Item::Io { output: true, .. }));
        }
    }
    Ok((modules, files))
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

/// A pattern's single-node metavariables (`$X`, not `$$$X` nor `$_X`), each once, in order.
pub(in crate::code_rule_dsl) fn single_metavars(pattern: &str) -> Vec<String> {
    let b = pattern.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'$' {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < b.len() && b[j] == b'$' {
            j += 1;
        }
        let k = j + b[j..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == b'_').count();
        let name = &pattern[j..k];
        if j - i == 1 && name.starts_with(|c: char| c.is_ascii_uppercase()) && !out.iter().any(|x| x == name) {
            out.push(name.to_owned());
        }
        i = k.max(i + 1);
    }
    out
}
