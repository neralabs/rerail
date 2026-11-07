//! Repository policy files become typed facts once, before document partitioning.
//! Readers cache bytes and decoded documents across selectors; a failed selector publishes no prefix.
use super::{Error, Result, front};
use front::Value;
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::path::{Component, Path};

#[derive(Clone, Debug)]
struct Failure {
    kind: &'static str,
    detail: String,
}
type ReadResult<T> = std::result::Result<T, Failure>;
fn malformed(detail: impl Into<String>) -> Failure { Failure { kind: "malformed", detail: detail.into() } }

fn parameters(text: &str) -> Result<BTreeMap<String, String>> {
    let mut parts = Vec::new();
    let (mut start, mut quoted, mut escaped) = (0, false, false);
    for (i, c) in text.char_indices() {
        if escaped { escaped = false; continue; }
        match c {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            ',' if !quoted => { parts.push(&text[start..i]); start = i + 1; }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    let mut out = BTreeMap::new();
    for part in parts {
        let Some((key, value)) = part.split_once('=') else { return Err(Error::Plan(format!("invalid input parameter: {part}"))); };
        let (key, value) = (key.trim(), value.trim());
        if !["IO", "filename", "dir", "path", "records", "at", "artifact"].contains(&key) {
            return Err(Error::Plan(format!("unsupported input parameter: {key}")));
        }
        let value = if value.starts_with('"') {
            serde_json::from_str::<String>(value).map_err(|e| Error::Plan(format!("invalid input parameter {key}: {e}")))?
        } else { value.to_owned() };
        if out.insert(key.to_owned(), value).is_some() { return Err(Error::Plan(format!("duplicate input parameter: {key}"))); }
    }
    Ok(out)
}

/// JSON pointer selectors retain the key or index of each wildcard as a leading column.
fn select<'a>(value: &'a Json, steps: &[&str], prefix: Vec<Json>, out: &mut Vec<(Vec<Json>, &'a Json)>) {
    let Some((step, rest)) = steps.split_first() else { out.push((prefix, value)); return; };
    if *step == "*" {
        let members: Vec<(Json, &Json)> = match value {
            Json::Object(map) => map.iter().map(|(k, v)| (Json::String(k.clone()), v)).collect(),
            Json::Array(items) => items.iter().enumerate().map(|(i, v)| (Json::from(i), v)).collect(),
            _ => Vec::new(),
        };
        for (key, child) in members {
            let mut next = prefix.clone(); next.push(key);
            select(child, rest, next, out);
        }
    } else {
        let key = step.replace("~1", "/").replace("~0", "~");
        if let Some(child) = match value {
            Json::Object(map) => map.get(&key),
            Json::Array(items) => key.parse::<usize>().ok().and_then(|i| items.get(i)),
            _ => None,
        } { select(child, rest, prefix, out); }
    }
}

fn scalar(value: &Json, ty: &str, text: bool) -> ReadResult<Value> {
    match (ty, value) {
        ("sym", Json::String(s)) => Ok(Value::Str(s.clone())),
        ("int", Json::Number(n)) => n.as_i64().map(Value::Int).ok_or_else(|| malformed("number is outside the signed integer domain")),
        ("int", Json::Bool(b)) => Ok(Value::Int(i64::from(*b))),
        ("int", Json::String(s)) if text => s.parse().map(Value::Int).map_err(|_| malformed("invalid integer field")),
        _ => Err(malformed(format!("expected a {ty} scalar, found {value}"))),
    }
}

fn typed(row: Vec<Json>, columns: &[(String, String)], text: bool) -> ReadResult<Vec<Value>> {
    if row.len() != columns.len() { return Err(malformed(format!("expected {} columns, found {}", columns.len(), row.len()))); }
    row.iter().zip(columns).map(|(v, (_, ty))| scalar(v, ty, text)).collect()
}

fn json_rows(value: &Json, pointer: &str, records: bool, prefix: Vec<Json>, columns: &[(String, String)]) -> ReadResult<Vec<Vec<Value>>> {
    if !pointer.is_empty() && !pointer.starts_with('/') { return Err(malformed("path must be a JSON pointer")); }
    let steps: Vec<_> = if pointer.is_empty() { Vec::new() } else { pointer[1..].split('/').collect() };
    let mut selected = Vec::new();
    select(value, &steps, prefix, &mut selected);
    let mut out = Vec::new();
    for (prefix, value) in selected {
        let rows: Vec<Vec<Json>> = if records {
            let Json::Array(items) = value else { return Err(malformed("records=true requires a list of objects")); };
            items.iter().map(|item| {
                let Json::Object(map) = item else { return Err(malformed("record is not an object")); };
                columns[prefix.len().min(columns.len())..].iter().map(|(key, _)|
                    map.get(key).cloned().ok_or_else(|| malformed(format!("record is missing {key}")))).collect()
            }).collect::<ReadResult<_>>()?
        } else { match value {
            Json::Object(map) => map.iter().map(|(k, v)| vec![Json::String(k.clone()), v.clone()]).collect(),
            Json::Array(items) => items.iter().map(|v| match v { Json::Array(tuple) => tuple.clone(), _ => vec![v.clone()] }).collect(),
            scalar => vec![vec![scalar.clone()]],
        }};
        for row in rows { out.push(typed(prefix.iter().cloned().chain(row).collect(), columns, false)?); }
    }
    Ok(out)
}

#[derive(Default)]
struct Readers {
    bytes: BTreeMap<String, ReadResult<String>>,
    documents: BTreeMap<(String, String), ReadResult<Json>>,
}
impl Readers {
    fn read<'a>(&'a mut self, path: &str, read: &mut impl FnMut(&str) -> ReadResult<String>) -> ReadResult<&'a str> {
        self.bytes.entry(path.to_owned()).or_insert_with(|| read(path)).as_ref().map(String::as_str).map_err(Clone::clone)
    }
    fn rows(&mut self, path: &str, io: &str, pointer: &str, records: bool, prefix: Vec<Json>, columns: &[(String, String)], read: &mut impl FnMut(&str) -> ReadResult<String>) -> ReadResult<Vec<Vec<Value>>> {
        if matches!(io, "json" | "yaml") {
            let key = (path.to_owned(), io.to_owned());
            if !self.documents.contains_key(&key) {
                let decoded = self.read(path, read).and_then(|s| if io == "json" {
                    serde_json::from_str(s).map_err(|e| malformed(e.to_string()))
                } else { serde_yaml_ng::from_str::<Json>(s).map_err(|e| malformed(e.to_string())) });
                self.documents.insert(key.clone(), decoded);
            }
            let document = self.documents[&key].as_ref().map_err(Clone::clone)?;
            return json_rows(document, pointer, records, prefix, columns);
        }
        let source = self.read(path, read)?;
        // Repository text readers use universal newlines. No line is manufactured after a final separator.
        let source = source.replace("\r\n", "\n").replace('\r', "\n");
        let mut out = Vec::new();
        for (i, line) in source.split_terminator('\n').enumerate() {
            let mut row = prefix.clone();
            if matches!(io, "lines" | "jsonl") { row.push(Json::from(i + 1)); }
            if io == "jsonl" {
                let value = serde_json::from_str(line).map_err(|e| malformed(format!("line {}: {e}", i + 1)))?;
                out.extend(json_rows(&value, pointer, records, row, columns)?);
            } else {
                row.extend(if io == "lines" { vec![Json::String(line.to_owned())] }
                           else { line.split('\t').map(|s| Json::String(s.to_owned())).collect() });
                out.push(typed(row, columns, true)?);
            }
        }
        Ok(out)
    }
}

pub(super) fn populate(program: &mut front::Program, root: &Path) -> Result<()> {
    populate_with(program, root, &mut |name| std::fs::read_to_string(root.join(name)).map_err(|e| Failure {
        kind: if e.kind() == std::io::ErrorKind::NotFound { "missing" } else { "unreadable" }, detail: e.to_string(),
    }))
}

fn populate_with(program: &mut front::Program, root: &Path, read: &mut impl FnMut(&str) -> ReadResult<String>) -> Result<()> {
    let settings: Vec<_> = program.settings.iter().filter(|(k, _)| k == "input").map(|(_, s)| s.clone()).collect();
    let mut readers = Readers::default();
    let mut errors = Vec::new();
    for setting in settings {
        let Some((rel, params)) = setting.strip_suffix(')').and_then(|s| s.split_once('(')) else { return Err(Error::Plan(format!("invalid input setting: {setting}"))); };
        let params = parameters(params)?;
        if params.contains_key("at") || params.contains_key("artifact") {
            return Err(Error::Plan(format!("{rel}: revision and stamped-artifact readers are not implemented")));
        }
        let io = params.get("IO").map_or("file", String::as_str);
        if !["file", "lines", "json", "yaml", "jsonl"].contains(&io) { return Err(Error::Plan(format!("{rel}: unsupported input reader {io}"))); }
        let records = match params.get("records").map(String::as_str) {
            None | Some("false") => false, Some("true") => true,
            _ => return Err(Error::Plan(format!("{rel}: records must be true or false"))),
        };
        let pointer = params.get("path").map_or("", String::as_str);
        let filename = params.get("filename");
        let directory = params.get("dir");
        if filename.is_some() == directory.is_some() { return Err(Error::Plan(format!("{rel}: supply exactly one filename or dir"))); }
        let path = filename.or(directory).unwrap();
        if Path::new(path).components().any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_))) {
            return Err(Error::Plan(format!("{rel}: input path must be repository-relative")));
        }
        let columns = program.decls.get(rel).ok_or_else(|| Error::Plan(format!("{rel}: input has no declaration")))?;
        let result = if directory.is_some() {
            if io != "lines" { return Err(Error::Plan(format!("{rel}: dir is supported only by IO=lines"))); }
            match std::fs::read_dir(root.join(path)) {
                Err(e) => Err(Failure { kind: if e.kind() == std::io::ErrorKind::NotFound { "missing" } else { "unreadable" }, detail: e.to_string() }),
                Ok(entries) => {
                    let mut paths = entries.map(|entry| entry.map(|e| e.path())).collect::<std::io::Result<Vec<_>>>()
                        .map_err(|e| Error::Plan(format!("{rel}: listing {path}: {e}")))?;
                    paths.sort();
                    let mut rows = Vec::new();
                    let mut failure = None;
                    for file in paths {
                        if !file.is_file() { continue; }
                        let name = file.strip_prefix(root).unwrap().to_string_lossy().into_owned();
                        match readers.rows(&name, io, pointer, records, vec![Json::String(name.clone())], columns, read) {
                            Ok(part) => rows.extend(part), Err(e) => { failure = Some(e); break; }
                        }
                    }
                    failure.map_or(Ok(rows), Err)
                }
            }
        } else { readers.rows(path, io, pointer, records, Vec::new(), columns, read) };
        program.inputs.retain(|name| name != rel);
        match result {
            Ok(rows) => program.facts.entry(rel.to_owned()).or_default().extend(rows),
            Err(error) => errors.push(vec![Value::Str(path.clone()), Value::Str(error.kind.to_owned()), Value::Str(error.detail)]),
        }
    }
    if !errors.is_empty() {
        program.inputs.retain(|name| name != "input_error");
        program.facts.entry("input_error".to_owned()).or_default().extend(errors);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rows(io: &str, source: &str, pointer: &str, records: bool, columns: &[(&str, &str)]) -> ReadResult<Vec<Vec<Value>>> {
        let cols = columns.iter().map(|(name, ty)| ((*name).to_owned(), (*ty).to_owned())).collect::<Vec<_>>();
        Readers::default().rows("fixture", io, pointer, records, Vec::new(), &cols, &mut |_| Ok(source.to_owned()))
    }

    #[test]
    fn line_and_tab_readers_preserve_empty_fields_and_universal_newlines() {
        assert_eq!(rows("lines", "first\r\n\rthird\n", "", false, &[("line", "int"), ("text", "sym")]).unwrap(), vec![
            vec![Value::Int(1), Value::Str("first".into())], vec![Value::Int(2), Value::Str("".into())],
            vec![Value::Int(3), Value::Str("third".into())]]);
        assert!(rows("lines", "", "", false, &[("line", "int"), ("text", "sym")]).unwrap().is_empty());
        assert_eq!(rows("file", "\t7\n", "", false, &[("name", "sym"), ("limit", "int")]).unwrap(),
            vec![vec![Value::Str("".into()), Value::Int(7)]]);
    }

    #[test]
    fn yaml_and_json_lines_share_strict_typed_projection() {
        assert_eq!(rows("yaml", "counts:\n  first: 7\n", "/counts", false, &[("key", "sym"), ("n", "int")]).unwrap(),
            vec![vec![Value::Str("first".into()), Value::Int(7)]]);
        assert_eq!(rows("jsonl", "{\"n\":7}\n{\"n\":8}\n", "/n", false, &[("line", "int"), ("n", "int")]).unwrap(),
            vec![vec![Value::Int(1), Value::Int(7)], vec![Value::Int(2), Value::Int(8)]]);
        assert!(rows("jsonl", "7\nnot json\n", "", false, &[("line", "int"), ("n", "int")]).is_err());
        assert!(rows("json", "[7,\"8\"]", "", false, &[("n", "int")]).is_err());
        assert!(rows("json", "[7,9223372036854775808]", "", false, &[("n", "int")]).is_err());
    }

    #[test]
    fn selectors_decode_pointer_escapes_and_reject_incomplete_records() {
        assert_eq!(rows("json", r#"{"a/b":{"~":["one","two"]}}"#, "/a~1b/~0/*", false, &[("index", "int"), ("name", "sym")]).unwrap(),
            vec![vec![Value::Int(0), Value::Str("one".into())], vec![Value::Int(1), Value::Str("two".into())]]);
        assert!(rows("json", r#"[{"name":"one"},{}]"#, "", true, &[("name", "sym")]).is_err());
        assert!(parameters(r#"IO=json, filename="a,b.json", filename="other.json""#).is_err());
        assert_eq!(parameters(r#"IO=json, filename = "a,b\".json""#).unwrap()["filename"], "a,b\".json");
    }

    #[test]
    fn unsupported_artifact_or_revision_inputs_fail_closed() {
        for option in [r#"artifact="collection""#, r#"filename="policy.json", at="base""#] {
            let mut program = crate::code_rule_dsl::logic::rr::parse(&format!(".decl a(n: number)\n.input a(IO=json, {option})\n")).unwrap();
            assert!(populate_with(&mut program, Path::new("."), &mut |_| panic!("unsupported input must not be read")).is_err());
        }
    }

    #[test]
    fn multiple_selectors_read_and_decode_one_document_once() {
        let mut program = crate::code_rule_dsl::logic::rr::parse(r#"
.decl a(n: number)
.input a(IO=json, filename="shared.json", path="/a")
.decl b(s: symbol)
.input b(IO=json, filename="shared.json", path="/b")
"#).unwrap();
        let mut reads = 0;
        populate_with(&mut program, Path::new("."), &mut |_| { reads += 1; Ok(r#"{"a":7,"b":"kept"}"#.to_owned()) }).unwrap();
        assert_eq!(reads, 1);
        assert_eq!(program.facts["a"], vec![vec![Value::Int(7)]]);
        assert_eq!(program.facts["b"], vec![vec![Value::Str("kept".into())]]);
    }
}
