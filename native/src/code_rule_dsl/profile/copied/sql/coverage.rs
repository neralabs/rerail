//! Connection receipts keep a missing entire trace from passing the job budget.
use super::{Path, io_error, json};
use std::{fs::OpenOptions, io::Write, sync::Mutex};

static WRITE: Mutex<()> = Mutex::new(());

pub(super) fn record(stem: &Path, event: &str) -> rusqlite::Result<()> {
    let _guard = WRITE.lock().unwrap_or_else(|error| error.into_inner());
    let pid = std::process::id();
    let path = stem
        .parent()
        .expect("profile directory")
        .join(format!("coverage-{pid}.jsonl"));
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(io_error)?;
    let receipt = json!({"version":1,"pid":pid,"session":stem.file_name().expect("profile stem").to_string_lossy(),"event":event});
    let mut bytes =
        serde_json::to_vec(&receipt).map_err(|error| io_error(std::io::Error::other(error)))?;
    bytes.push(b'\n');
    file.write_all(&bytes).map_err(io_error)
}
