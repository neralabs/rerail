use super::*;

pub fn changed(before: &Option<Value>, step: &Value) -> Result<Option<Value>> {
    if let Some(snapshot) = step.get("snapshot") {
        return Ok(Some(snapshot.clone()));
    }
    let Some(patch) = step.get("file_changes") else {
        return Ok(before.clone());
    };
    let before = before
        .as_ref()
        .ok_or("file changes require an initial snapshot")?;
    if patch
        .as_object()
        .ok_or("file changes must be an object")?
        .keys()
        .any(|k| !["upsert", "delete"].contains(&k.as_str()))
    {
        return Err("unsupported file change field".into());
    }
    let mut files = BTreeMap::new();
    for file in before["files"].as_array().ok_or("missing snapshot files")? {
        files.insert(
            file["path"].as_str().ok_or("missing file path")?.to_owned(),
            file.clone(),
        );
    }
    let mut seen = std::collections::BTreeSet::new();
    for file in patch["upsert"].as_array().into_iter().flatten() {
        let path = file["path"].as_str().ok_or("missing file path")?.to_owned();
        if !seen.insert(path.clone()) {
            return Err("duplicate file change path".into());
        }
        files.insert(path, file.clone());
    }
    for path in patch["delete"].as_array().into_iter().flatten() {
        let path = path.as_str().ok_or("invalid deleted path")?;
        if !seen.insert(path.to_owned()) {
            return Err("duplicate file change path".into());
        }
        files.remove(path);
    }
    Ok(Some(
        json!({"files":files.into_values().collect::<Vec<_>>()}),
    ))
}
