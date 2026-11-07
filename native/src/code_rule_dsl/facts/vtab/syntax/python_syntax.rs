//! Native grammar validation for Python AST consumers. Raw source remains available on a rejected parse.
//! The tape owns the demanded, cached verdict; no Python interpreter is linked or invoked.
use ruff_python_ast::PythonVersion;
use ruff_python_parser::{Mode, ParseOptions, parse_unchecked};

pub(super) fn error(source: &str) -> Option<(u32, String)> {
    // ast.parse(str) does not apply the file decoder's UTF-8 signature removal.
    if source.starts_with('\u{feff}') {
        return Some((1, "invalid non-printable character U+FEFF".to_owned()));
    }
    let parsed = parse_unchecked(source, ParseOptions::from(Mode::Module).with_target_version(PythonVersion::PY313));
    let ordinary = parsed.errors().iter().map(|e| (u32::from(e.location.start()), e.error.to_string()));
    let version = parsed.unsupported_syntax_errors().iter().map(|e| (u32::from(e.range.start()), e.to_string()));
    let (offset, message) = ordinary.chain(version).min_by_key(|e| e.0)?;
    Some((super::core_rr::Lines::new(source, true).line(offset), message))
}
