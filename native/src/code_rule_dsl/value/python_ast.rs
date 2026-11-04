//! Canonical CPython 3.13 AST facts, produced by one isolated process per demand batch.
//! SQL selects the roots. No source module, hook or site configuration is imported.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::Write,
    process::{Command, Stdio},
};

#[derive(Debug, Clone)]
pub(crate) struct Error(pub String);
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub start: usize,
    pub end: usize,
    pub kind: String,
    pub fields: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Document {
    pub source: String,
    pub requests: Vec<Request>,
}
pub(crate) type Row = BTreeMap<String, String>;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Response {
    version: [u8; 2],
    pub documents: Vec<Vec<Row>>,
    pub parses: usize,
    pub module_walks: usize,
}

#[cfg(test)]
pub(crate) fn batch(documents: &[Document]) -> Result<Vec<Vec<Row>>, Error> {
    Ok(batch_with_metrics(documents)?.documents)
}
pub(crate) fn batch_with_metrics(documents: &[Document]) -> Result<Response, Error> {
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
    let input = serde_json::to_vec(
        &serde_json::json!({"format":"rerail.python-ast/v1","documents":documents}),
    )
    .map_err(|e| Error(e.to_string()))?;
    let executable = if cfg!(windows) {
        "python.exe"
    } else {
        "python3.13"
    };
    let mut child = Command::new(executable)
        .args(["-I", "-S", "-c", include_str!("python_ast/worker.py")])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            Error(format!(
                "Canonical Python AST facts require CPython 3.13 ({executable}): {e}"
            ))
        })?;
    let write = child.stdin.take().expect("piped stdin").write_all(&input);
    let output = child.wait_with_output().map_err(|e| Error(e.to_string()))?;
    if !output.status.success() {
        return Err(Error(format!(
            "CPython AST batch failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    write.map_err(|e| Error(e.to_string()))?;
    if output.stdout.len() > 64 * 1024 * 1024 {
        return Err(Error("Python AST batch exceeds its output capacity".into()));
    }
    let response: Response =
        serde_json::from_slice(&output.stdout).map_err(|e| Error(e.to_string()))?;
    if response.version != [3, 13] || response.documents.len() != documents.len() {
        return Err(Error(
            "Canonical Python AST response shape differs from its request".into(),
        ));
    }
    for (document, rows) in documents.iter().zip(&response.documents) {
        if rows.len() != document.requests.len() {
            return Err(Error(
                "Canonical Python AST row count differs from its demand".into(),
            ));
        }
        for (request, row) in document.requests.iter().zip(rows) {
            if row.len() != request.fields.len()
                || request.fields.iter().any(|f| !row.contains_key(f))
            {
                return Err(Error(
                    "Canonical Python AST fields differ from their demand".into(),
                ));
            }
        }
    }
    Ok(response)
}
#[cfg(test)]
mod tests;
