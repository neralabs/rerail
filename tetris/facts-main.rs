//! Generic JSON-lines transport. All application behavior belongs to the .rr program.
use std::io::{self, BufRead, Write};

fn main() {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in io::stdin().lock().lines() {
        let result = line.map_err(|e| e.to_string()).and_then(|line| {
            let request: serde_json::Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
            let source = request["source"].as_str().ok_or("source must be a string")?;
            rerail_engine::tools::evaluate_facts(source, request["trace"].as_bool().unwrap_or(false))
        });
        let response = match result {
            Ok(response) => response,
            Err(error) => serde_json::json!({"error": error}),
        };
        writeln!(out, "{response}").expect("write response");
        out.flush().expect("flush response");
    }
}
