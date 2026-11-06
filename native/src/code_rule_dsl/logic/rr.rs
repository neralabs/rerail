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
#[cfg(test)]
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
    let files = program(path)?;
    lower::lower(files.into_iter().flat_map(|(_, statements)| statements).collect())
}

fn read(path: &Path) -> Result<Vec<syntax::Statement>, Error> {
    let src = std::fs::read_to_string(path).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    syntax::Parser::new(&src)?.statements().map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))
}

/// A file and everything it includes, each once, includes first: (path, statements without the includes).
fn program(path: &Path) -> Result<Vec<(PathBuf, Vec<syntax::Statement>)>, Error> {
    fn visit(path: &Path, out: &mut Vec<(PathBuf, Vec<syntax::Statement>)>, seen: &mut Vec<PathBuf>) -> Result<(), Error> {
        let path = path.canonicalize().map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
        if seen.contains(&path) {
            return Ok(());
        }
        seen.push(path.clone());
        let (includes, rest): (Vec<_>, Vec<_>) = read(&path)?.into_iter().partition(|(item, _)| matches!(item, syntax::Item::Include(_)));
        for (item, line) in includes {
            let syntax::Item::Include(name) = item else { unreachable!("an include") };
            let in_library = library().join(&name);
            let beside = path.parent().map(|d| d.join(&name)).unwrap_or_default();
            let found = if in_library.exists() { in_library } else { beside };
            if !found.exists() {
                return Err(Error::Syntax(format!("{}:{line}: #include {name:?}: no such file", path.display())));
            }
            visit(&found, out, seen)?;
        }
        out.push((path, rest));
        Ok(())
    }
    let mut out = Vec::new();
    // `core.rr` is every program's prelude (the base relations and built-ins), included or not.
    visit(&library().join("core.rr"), &mut out, &mut Vec::new())?;
    let mut seen = vec![library().join("core.rr").canonicalize().map_err(|e| Error::Syntax(format!("core.rr: {e}")))?];
    visit(path, &mut out, &mut seen)?;
    Ok(out)
}
