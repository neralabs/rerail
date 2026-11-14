//! Apply the production private-owner projection rewrite to an immutable SQL capture.
use serde_json::Value;
use std::path::Path;

pub fn rewrite(capture: &Path, output: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if output.exists() {
        return Err("output already exists; use a fresh output path".into());
    }
    let mut data: Value = serde_json::from_slice(&std::fs::read(capture)?)?;
    if !matches!(
        data["format"].as_str(),
        Some("rerail.candidate-replay/v1" | "rerail.candidate-replay/v2")
    ) {
        return Err("expected a candidate SQL capture".into());
    }
    let source = capture
        .parent()
        .ok_or("capture needs a parent")?
        .join(data["database"].as_str().ok_or("missing database")?);
    let parent = output.parent().ok_or("output needs a parent")?;
    let copy = parent.join("owner-input.sqlite");
    if copy.exists() {
        return Err("output database already exists; use a fresh directory".into());
    }
    for stage in data["stages"].as_array_mut().ok_or("missing stages")? {
        if stage["temporary"] != true {
            continue;
        }
        let name = stage["name"].as_str().ok_or("missing owner name")?;
        let sql = stage["sql"].as_str().ok_or("missing owner SQL")?;
        stage["sql"] = crate::render::owned(sql.into(), name).into();
    }
    std::fs::create_dir_all(parent)?;
    std::fs::copy(source, &copy)?;
    data["database"] = "owner-input.sqlite".into();
    data["rewriteNote"]="Production private-owner projection rewrite; logical bodies and all other stages remain unchanged".into();
    std::fs::write(output, serde_json::to_vec_pretty(&data)?)?;
    Ok(())
}
