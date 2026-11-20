//! Canonical CPython 3.13 AST facts of a demand batch, computed natively (`code_rule_dsl::cpython`): no Python runs.
//! SQL selects the roots. A node is found as W's `pythonAst` worker (deleted, work queue Q360) found it, by its span
//! and the classes its kind may be, and each field is the worker's: `dump`, `dump_sha256`, `body_sha256`,
//! `signature_sha256`, `unparse`, `repr_unparse`, `literal_json`, `identifier`, the text fields `column` and a name's
//! `identifier`, which parse nothing, and a module's `syntax_error`: the whole `SyntaxError` (`msg`, `lineno`,
//! `offset`, `text`, `end_lineno`, `end_offset`, `cpython::syntax`), which only the gate's conformance replay asks
//! for (`pythonAstBatch`).
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Error(pub String);
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub start: usize,
    pub end: usize,
    pub kind: String,
    pub fields: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub source: String,
    pub requests: Vec<Request>,
}
pub(crate) type Row = BTreeMap<String, String>;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    version: [u8; 2],
    pub documents: Vec<Vec<Row>>,
    pub parses: usize,
    pub module_walks: usize,
}

#[cfg(any(test, feature = "test-hooks"))]
pub(crate) fn batch(documents: &[Document]) -> Result<Vec<Vec<Row>>, Error> {
    Ok(batch_with_metrics(documents)?.documents)
}
pub fn batch_with_metrics(documents: &[Document]) -> Result<Response, Error> {
    if documents.is_empty() {
        return Ok(Response {
            version: [3, 13],
            documents: Vec::new(),
            parses: 0,
            module_walks: 0,
        });
    }
    if documents.iter().map(|d| d.source.len()).sum::<usize>() > 8 * 1024 * 1024 {
        return Err(Error("Python AST batch exceeds its source capacity".into()));
    }
    native(documents)
}

/// A batch failed as the worker's did: its message after "CPython AST batch failed".
fn failed(message: impl std::fmt::Display) -> Error {
    Error(format!("CPython AST batch failed: {message}"))
}

/// Kinds a node is found by (the worker's `KINDS`); any other fails the batch.
const KINDS: &[&str] = &[
    "function_definition",
    "class_definition",
    "assignment",
    "identifier",
    "attribute",
    "call",
    "string",
    "concatenated_string",
    "integer",
    "float",
    "tuple",
    "list",
    "dictionary",
    "subscript",
    "type",
    "import_from_statement",
    "true",
    "false",
    "none",
];

/// The batch, natively: each document parsed once where a request needs its tree, its located nodes listed once
/// where a request needs a node (`module_walks`), each field as the worker's.
fn native(documents: &[Document]) -> Result<Response, Error> {
    use crate::code_rule_dsl::cpython::{ast, dump::Dump, literal, printed, unparse, value};
    let text_only = |r: &Request| {
        r.fields == ["column"] || matches!(r.kind.as_str(), "identifier" | "dotted_name" | "relative_import") && r.fields == ["identifier"]
    };
    let (mut parses, mut module_walks, mut bytes) = (0usize, 0usize, 0usize);
    let mut out = Vec::with_capacity(documents.len());
    for document in documents {
        let source = document.source.as_str();
        let tree = if document.requests.iter().all(text_only) {
            None
        } else {
            parses += 1;
            match ast::module(source) {
                Some(tree) => Some(tree),
                None => {
                    let verdict = crate::code_rule_dsl::cpython::syntax::syntax_error(source);
                    // A module that does not parse gives its `SyntaxError` where that is all that is asked of it,
                    // else the batch fails (the worker's `raise`).
                    if let Ok(Some(e)) = &verdict
                        && document.requests.iter().all(|r| r.kind == "module" && r.fields == ["syntax_error"])
                    {
                        let value = syntax_error_json(e);
                        let mut rows = Vec::with_capacity(document.requests.len());
                        for _ in &document.requests {
                            bytes += value.len();
                            if bytes > 64 * 1024 * 1024 {
                                return Err(Error("Python AST batch exceeds its output capacity".into()));
                            }
                            rows.push(Row::from([("syntax_error".to_owned(), value.clone())]));
                        }
                        out.push(rows);
                        continue;
                    }
                    return Err(match verdict {
                        Ok(Some(e)) => failed(format!("SyntaxError: {} (line {})", e.msg, e.line)),
                        Ok(None) => failed("no tree for a module that parses"),
                        Err(raised) => failed(format!("{}: {}", raised.class, raised.message)),
                    });
                }
            }
        };
        let walks = document.requests.iter().any(|r| r.kind != "module" && !text_only(r));
        module_walks += usize::from(walks);
        let located = match (&tree, walks) {
            (Some(tree), true) => printed::located(tree),
            _ => Vec::new(),
        };
        let asks = |f: &str| document.requests.iter().any(|r| r.fields.iter().any(|x| x == f));
        let dumps = tree.as_ref().filter(|_| ["dump", "dump_sha256", "body_sha256", "signature_sha256"].iter().any(|f| asks(f))).map(Dump::of);
        let literals = if asks("literal_json") { literal::literals(source) } else { Vec::new() };
        // Line starts, at CPython's terminators (`\r\n`, `\r`, `\n`), in bytes.
        let mut lines = vec![0usize];
        let b = source.as_bytes();
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'\r' if b.get(i + 1) == Some(&b'\n') => {
                    i += 2;
                    lines.push(i);
                }
                b'\r' | b'\n' => {
                    i += 1;
                    lines.push(i);
                }
                _ => i += 1,
            }
        }
        let mut rows = Vec::with_capacity(document.requests.len());
        let mut put = |row: &mut Row, field: &str, value: String| -> Result<(), Error> {
            bytes += value.len();
            if bytes > 64 * 1024 * 1024 {
                return Err(Error("Python AST batch exceeds its output capacity".into()));
            }
            row.insert(field.to_owned(), value);
            Ok(())
        };
        for r in &document.requests {
            let mut row = Row::new();
            if r.fields == ["column"] {
                let line = lines[lines.partition_point(|&s| s <= r.start) - 1];
                put(&mut row, "column", r.start.saturating_sub(line).to_string())?;
                rows.push(row);
                continue;
            }
            if text_only(r) {
                let text = source.get(r.start..r.end).ok_or_else(|| failed(format!("no text at {}..{}", r.start, r.end)))?;
                // The worker's `re.sub(r"\\\r?\n|\s+", "", text)`, then NFKC.
                let joined = text.replace("\\\r\n", "").replace("\\\n", "");
                let bare: String = joined.chars().filter(|c| !(c.is_whitespace() || ('\x1c'..='\x1f').contains(c))).collect();
                put(&mut row, "identifier", crate::code_rule_dsl::identifier::nfkc(&bare).into_owned())?;
                rows.push(row);
                continue;
            }
            let tree = tree.as_ref().expect("parsed for a request that is not text alone");
            let node: &ast::Node = if r.kind == "module" {
                tree
            } else {
                if !KINDS.contains(&r.kind.as_str()) {
                    return Err(failed(format!("Unsupported canonical AST root kind: {}", r.kind)));
                }
                let span = (r.start as u32, r.end as u32);
                let exact = located.iter().find(|n| n.span == Some(span) && printed::of_kind(n.class, &r.kind)).copied();
                let near = || {
                    let mut within =
                        located.iter().filter(|n| n.span.is_some_and(|(s, e)| s == span.0 && e <= span.1) && printed::of_kind(n.class, &r.kind));
                    match (within.next(), within.next()) {
                        (Some(one), None) => Some(*one),
                        _ => None,
                    }
                };
                exact
                    .or_else(|| if printed::by_start(&r.kind) { near() } else { None })
                    .ok_or_else(|| failed(format!("No CPython AST node for {} at {}..{}", r.kind, r.start, r.end)))?
            };
            let def = matches!(node.class, "FunctionDef" | "AsyncFunctionDef");
            let dumps = || dumps.as_ref().expect("dumps made for a field that reads them");
            for field in &r.fields {
                let none = || failed(format!("no {field} for {} at {}..{}", node.class, r.start, r.end));
                let value = match field.as_str() {
                    // The module parsed: no error.
                    "syntax_error" if r.kind == "module" => "null".to_owned(),
                    "dump" => dumps().get(node).map(str::to_owned).ok_or_else(none)?,
                    "dump_sha256" => dumps().sha256(node).ok_or_else(none)?,
                    "body_sha256" | "signature_sha256" if !def => return Err(failed("Function fingerprints require a function definition")),
                    "body_sha256" => dumps().body_sha256(node).ok_or_else(none)?,
                    "signature_sha256" => dumps().signature_sha256(node).ok_or_else(none)?,
                    "unparse" => unparse::unparse(node).ok_or_else(none)?,
                    "repr_unparse" => value::str_repr(&value::PyStr::Text(unparse::unparse(node).ok_or_else(none)?)),
                    "literal_json" => literals
                        .iter()
                        .find(|l| node.span == Some((l.start, l.end)) && l.class.name() == node.class)
                        .map(|l| l.json.clone())
                        .ok_or_else(none)?,
                    "identifier" => {
                        let name = match node.class {
                            "Name" => "id",
                            "FunctionDef" | "AsyncFunctionDef" | "ClassDef" => "name",
                            "Attribute" => "attr",
                            _ => return Err(failed("Canonical identifier requires a named AST node")),
                        };
                        match node.get(name) {
                            ast::F::Ident(id) => id.clone(),
                            _ => return Err(none()),
                        }
                    }
                    other => return Err(failed(format!("Unknown Python AST field: {other}"))),
                };
                put(&mut row, field, value)?;
            }
            rows.push(row);
        }
        out.push(rows);
    }
    Ok(Response { version: [3, 13], documents: out, parses, module_walks })
}

/// A module's `SyntaxError` as the worker gave it: `json.dumps` (`ensure_ascii`, no spaces) of its `msg`, `lineno`,
/// `offset`, `text`, `end_lineno` and `end_offset`, in that order; each `null` but `msg` where CPython gives none (a
/// null byte).
fn syntax_error_json(e: &crate::code_rule_dsl::cpython::syntax::SyntaxError) -> String {
    let mut out = String::from("{\"msg\":");
    json_ascii(&mut out, &e.msg);
    match &e.at {
        Some(at) => {
            out.push_str(&format!(",\"lineno\":{},\"offset\":{},\"text\":", at.lineno, at.offset));
            json_ascii(&mut out, &at.text);
            out.push_str(&format!(",\"end_lineno\":{},\"end_offset\":{}}}", at.end_lineno, at.end_offset));
        }
        None => out.push_str(",\"lineno\":null,\"offset\":null,\"text\":null,\"end_lineno\":null,\"end_offset\":null}"),
    }
    out
}

/// `text` as a JSON string, as Python's `json.dumps(..., ensure_ascii=True)` writes it: printable ASCII as it is, `"`
/// and `\` escaped, the short escapes of `\n`, `\r`, `\t`, `\b`, `\f`, every other character `\uxxxx` (lowercase hex;
/// past the BMP, its surrogate pair).
fn json_ascii(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out.push('"');
}
#[cfg(test)]
pub mod tests;
