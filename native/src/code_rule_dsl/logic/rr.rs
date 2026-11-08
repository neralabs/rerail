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
mod translate;

#[cfg(test)]
mod tape_tests;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

pub(in crate::code_rule_dsl) use super::front::Error;
use super::front::Program;
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

/// A program's `.rr` text, which includes nothing.
pub(in crate::code_rule_dsl) fn parse(src: &str) -> Result<Program, Error> {
    let statements = from_tape::statements(src, &tape(src))?;
    lower::lower(statements)
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
    fn now(path: &Path) -> Self {
        use sha2::{Digest as _, Sha256};
        Self { path: path.to_owned(), sha256: std::fs::read(path).ok().map(|bytes| Sha256::digest(&bytes).into()) }
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
    // `core.rr`'s relations are the engine's (decision 0047): no other file declares or derives one.
    let core = library().join("core.rr").canonicalize().map_err(|e| Error::Syntax(format!("core.rr: {e}")))?;
    let engine: std::collections::BTreeSet<&String> = statements
        .iter()
        .filter(|(file, _)| *file == core)
        .flat_map(|(_, statements)| statements)
        .filter_map(|(item, _)| if let syntax::Item::Decl { name, .. } = item { Some(name) } else { None })
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
    // Names starting `rr_` are the engine's (`front::RESERVED`): its helpers, axes and places.
    for (file, statements) in &statements {
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
    let program = lower::lower(statements.into_iter().flat_map(|(_, statements)| statements).collect())?;
    cache.lock().expect("the program cache").insert(key, (files.clone(), program.clone()));
    Ok((program, files))
}

fn read(path: &Path) -> Result<(Vec<syntax::Statement>, LibraryFile), Error> {
    use sha2::{Digest as _, Sha256};
    let bytes = std::fs::read(path).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    // Each file's statements, kept for the process by its path and bytes' digest: programs that include one library
    // (the Python pack's files) parse it once, not once per program.
    static PARSED: OnceLock<Mutex<HashMap<PathBuf, ([u8; 32], Vec<syntax::Statement>)>>> = OnceLock::new();
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    let file = LibraryFile { path: path.to_owned(), sha256: Some(digest) };
    let cache = PARSED.get_or_init(Default::default);
    if let Some((kept, statements)) = cache.lock().expect("the parsed files").get(path)
        && *kept == digest
    {
        crate::code_rule_dsl::prof::count("rr.file_reused", 1);
        return Ok((statements.clone(), file));
    }
    let src = String::from_utf8(bytes).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    let statements = stored_statements(path, src).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    cache.lock().expect("the parsed files").insert(path.to_owned(), (digest, statements.clone()));
    Ok((statements, file))
}

/// A rule file's statements, read off its tape (decision 0062). The tape is the content store's, as any corpus
/// file's is (`facts::core::build_cached`, keyed by the source's content and the grammar's version): a file read by an
/// earlier run is not parsed again. Without a store (the browser build, `RERAIL_TAPE_CACHE=off`) the tape is made
/// here.
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

/// A file and everything it includes, each once, includes first: (path, statements without the
/// includes, the included files' `.output`s left out); and every library file read or probed, in reading order.
fn program(path: &Path) -> Result<(Files, Vec<LibraryFile>), Error> {
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
                read_files.push(LibraryFile { path: in_library, sha256: None });
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
