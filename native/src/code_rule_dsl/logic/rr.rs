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
//! A file read with `parse_file` has `core.rr` before it, included or not.

mod check;
mod lower;
mod print;
mod syntax;
mod translate;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

pub(in crate::code_rule_dsl) use super::front::Error;
use super::front::Program;
pub(in crate::code_rule_dsl) use print::print;
pub(in crate::code_rule_dsl) use syntax::quote;
pub(in crate::code_rule_dsl) use translate::translate;

/// The library `#include` reads from.
pub(in crate::code_rule_dsl) fn library() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/lib")
}

/// A program's `.rr` text, which includes nothing.
pub(in crate::code_rule_dsl) fn parse(src: &str) -> Result<Program, Error> {
    let statements = syntax::Parser::new(src)?.statements()?;
    lower::lower(statements)
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
    let file = LibraryFile { path: path.to_owned(), sha256: Some(Sha256::digest(&bytes).into()) };
    let src = String::from_utf8(bytes).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    let statements = syntax::Parser::new(&src)?.statements().map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    Ok((statements, file))
}

type Files = Vec<(PathBuf, Vec<syntax::Statement>)>;

/// A file and everything it includes, each once, includes first: (path, statements without the
/// includes); and every library file read or probed, in reading order.
fn program(path: &Path) -> Result<(Files, Vec<LibraryFile>), Error> {
    fn visit(path: &Path, out: &mut Files, read_files: &mut Vec<LibraryFile>, seen: &mut Vec<PathBuf>) -> Result<(), Error> {
        let path = path.canonicalize().map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
        if seen.contains(&path) {
            return Ok(());
        }
        seen.push(path.clone());
        let (statements, file) = read(&path)?;
        read_files.push(file);
        let (includes, rest): (Vec<_>, Vec<_>) = statements.into_iter().partition(|(item, _)| matches!(item, syntax::Item::Include(_)));
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
            visit(&found, out, read_files, seen)?;
        }
        out.push((path, rest));
        Ok(())
    }
    let (mut out, mut read_files) = (Vec::new(), Vec::new());
    // `core.rr` is every program's prelude (the base relations and built-ins), included or not.
    visit(&library().join("core.rr"), &mut out, &mut read_files, &mut Vec::new())?;
    let mut seen = vec![library().join("core.rr").canonicalize().map_err(|e| Error::Syntax(format!("core.rr: {e}")))?];
    visit(path, &mut out, &mut read_files, &mut seen)?;
    Ok((out, read_files))
}
