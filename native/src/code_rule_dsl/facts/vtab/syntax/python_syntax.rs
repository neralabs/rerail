//! Native grammar validation for Python AST consumers. Raw source remains available on a rejected parse.
//! The tape owns the demanded, cached verdict; no Python interpreter is linked or invoked.
use ruff_python_ast::PythonVersion;
use ruff_python_parser::{Mode, ParseOptions, parse_unchecked};
mod diagnostic;

pub(super) fn error(source: &str) -> Option<(u32, String)> {
    if source.contains('\0') {
        return Some((1, "source code string cannot contain null bytes".to_owned()));
    }
    // ast.parse(str) does not apply the file decoder's UTF-8 signature removal.
    if source.starts_with('\u{feff}') {
        return Some((1, "invalid non-printable character U+FEFF".to_owned()));
    }
    let parsed = parse_unchecked(source, ParseOptions::from(Mode::Module).with_target_version(PythonVersion::PY313));
    // Choose the parser error before adjusting its displayed position. An unclosed group can
    // point back to its opener, but must not hide an earlier malformed parameter.
    let ordinary = parsed.errors().iter().min_by_key(|e| e.location.start());
    let version = parsed.unsupported_syntax_errors().iter().min_by_key(|e| e.range.start());
    let (offset, message) = match (ordinary, version) {
        (Some(e), v) if v.is_none_or(|v| e.location.start() <= v.range.start()) =>
            diagnostic::describe(source, e, parsed.tokens()),
        (_, Some(e)) => (u32::from(e.range.start()), e.to_string()),
        _ => return None,
    };
    Some((super::core_rr::Lines::new(source, true).line(offset), message))
}

#[cfg(test)]
mod tests {
    #[test]
    fn malformed_parameter_list_has_the_python_syntax_message() {
        assert_eq!(super::error("def broken(:\n"), Some((1, "invalid syntax".to_owned())));
    }

    #[test]
    fn native_diagnostics_match_recorded_python_313_errors() {
        let cases: serde_json::Value = serde_json::from_str(include_str!("python_syntax/python-errors.json")).unwrap();
        for case in cases.as_array().unwrap() {
            let source = case["source"].as_str().unwrap();
            // NUL has no SyntaxError location; the relation uses the document's first line.
            let expected = (case["line"].as_u64().unwrap_or(1) as u32, case["message"].as_str().unwrap().to_owned());
            assert_eq!(super::error(source), Some(expected), "{source:?}");
        }
    }
}
