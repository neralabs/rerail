//! The committed scope records that analysis.rr projects into public relations.

use super::{front::{Program, Value}, memo, Result};

pub(super) const INPUT: &str = "analysis_run_record";

pub(super) fn reads(program: &Program) -> bool {
    program.inputs.iter().any(|name| name == INPUT)
        && super::front::needed(program, program.outputs.clone()).contains(INPUT)
}

pub(super) fn records() -> Result<(Vec<Vec<Value>>, String)> {
    let mut rows = Vec::new();
    if let Some(store) = super::tape_cache()? {
        let id = store.identity();
        for record in store.completed_analysis_runs()? {
            let kind = if record.scope.starts_with("cwd:") { "directory" } else { "named" };
            let values = [
                format!("scope:{id}:{}", record.scope), kind.to_owned(), record.value,
                format!("run:{id}:{}", record.acquisition), record.scan_id, record.directory.unwrap_or_default(),
                record.repository.unwrap_or_default(), record.tree.unwrap_or_default(), record.commit.unwrap_or_default(),
                record.snapshot.map(|key| format!("snapshot:{id}:{key}")).unwrap_or_default(),
            ];
            rows.push(values.into_iter().map(|v| Value::Str(v.into())).collect());
        }
    }
    let digest = memo::with_rows([0; 32], std::iter::once((INPUT, &rows)));
    Ok((rows, crate::content_store::observe::hex(&digest)))
}
