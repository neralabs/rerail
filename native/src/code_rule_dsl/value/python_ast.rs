//! Canonical CPython 3.13 AST facts, produced by one isolated process per demand batch.
//! SQL selects the roots. No source module, hook or site configuration is imported.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<Vec<String>>,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RowFrame {
    document: usize,
    request: usize,
    row: Row,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EndFrame {
    version: [u8; 2],
    parses: usize,
    module_walks: usize,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Frame {
    Row(RowFrame),
    End(EndFrame),
}

fn read_frames(reader: impl BufRead, documents: &[Document]) -> Result<Response, Error> {
    let mut response = Response {
        version: [3, 13],
        documents: documents.iter().map(|_| Vec::new()).collect(),
        parses: 0,
        module_walks: 0,
    };
    let mut expected = documents.iter().enumerate().flat_map(|(document, value)| {
        (0..value.requests.len()).map(move |request| (document, request))
    });
    let mut reader = reader;
    let mut ended = false;
    loop {
        let mut bytes = Vec::new();
        let count = reader
            .by_ref()
            .take(64 * 1024 * 1024 + 1)
            .read_until(b'\n', &mut bytes)
            .map_err(|error| Error(error.to_string()))?;
        if count == 0 {
            break;
        }
        if count > 64 * 1024 * 1024 || bytes.last() != Some(&b'\n') {
            return Err(Error(
                "Python AST projection exceeds its output capacity".into(),
            ));
        }
        if ended {
            return Err(Error(
                "Canonical Python AST frames follow the completion frame".into(),
            ));
        }
        match serde_json::from_slice::<Frame>(&bytes).map_err(|error| Error(error.to_string()))? {
            Frame::Row(frame) => {
                if expected.next() != Some((frame.document, frame.request)) {
                    return Err(Error(
                        "Canonical Python AST frame differs from its demand order".into(),
                    ));
                }
                let request = &documents[frame.document].requests[frame.request];
                if frame.row.len() != request.fields.len()
                    || request
                        .fields
                        .iter()
                        .any(|field| !frame.row.contains_key(field))
                {
                    return Err(Error(
                        "Canonical Python AST fields differ from their demand".into(),
                    ));
                }
                response.documents[frame.document].push(frame.row);
            }
            Frame::End(frame) => {
                if frame.version != [3, 13] || expected.next().is_some() {
                    return Err(Error("Canonical Python AST response is incomplete".into()));
                }
                response.parses = frame.parses;
                response.module_walks = frame.module_walks;
                ended = true;
            }
        }
    }
    if !ended {
        return Err(Error(
            "Canonical Python AST completion frame is missing".into(),
        ));
    }
    Ok(response)
}

#[cfg(test)]
pub(crate) fn batch(documents: &[Document]) -> Result<Vec<Vec<Row>>, Error> {
    Ok(batch_with_metrics(documents)?.documents)
}
pub(crate) fn batch_with_metrics(documents: &[Document]) -> Result<Response, Error> {
    // Capacity partitions are transport batches, not independent rule scopes.
    // Keep each document intact so partitioning never reparses it, and retain demand order.
    let mut response = Response {
        version: [3, 13],
        documents: Vec::new(),
        parses: 0,
        module_walks: 0,
    };
    let mut start = 0;
    while start < documents.len() {
        let mut end = start;
        let mut bytes = 0usize;
        while end < documents.len() && bytes + documents[end].source.len() <= 8 * 1024 * 1024 {
            bytes += documents[end].source.len();
            end += 1;
        }
        if end == start {
            return Err(Error(
                "Python AST document exceeds its source capacity".into(),
            ));
        }
        let part = batch_part_with_metrics(&documents[start..end])?;
        response.documents.extend(part.documents);
        response.parses += part.parses;
        response.module_walks += part.module_walks;
        start = end;
    }
    Ok(response)
}
fn batch_part_with_metrics(documents: &[Document]) -> Result<Response, Error> {
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
        &serde_json::json!({"format":"rerail.python-ast/v2","documents":documents}),
    )
    .map_err(|e| Error(e.to_string()))?;
    let executable = if cfg!(windows) {
        "python.exe"
    } else {
        "python3.13"
    };
    let mut child = Command::new(executable)
        .args([
            "-I",
            "-S",
            "-c",
            &format!(
                "{}\n{}",
                include_str!("python_ast/metadata.py"),
                include_str!("python_ast/worker.py")
            ),
        ])
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
    let frames = read_frames(
        BufReader::new(child.stdout.take().expect("piped stdout")),
        documents,
    );
    if frames.is_err() {
        let _ = child.kill();
    }
    let output = child.wait_with_output().map_err(|e| Error(e.to_string()))?;
    if !output.status.success() {
        return Err(Error(format!(
            "CPython AST batch failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    write.map_err(|e| Error(e.to_string()))?;
    let response = frames?;
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
