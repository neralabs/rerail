//! Recompile a frozen logical candidate plan with the production SQL ownership compiler.
//! This keeps the capture's consumer ordinals for exact intermediate multiset comparisons.
use serde_json::Value;
use std::path::Path;

pub fn rewrite(
    capture: &Path,
    graph: &Path,
    output: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if output.exists() {
        return Err("output exists; use a fresh path".into());
    }
    let mut data: Value = serde_json::from_slice(&std::fs::read(capture)?)?;
    if data
        .get("setup")
        .and_then(Value::as_array)
        .is_some_and(|s| !s.is_empty())
    {
        return Err(
            "input already has shared ownership; replay its captured schedule directly".into(),
        );
    }
    let definitions: Vec<String> = serde_json::from_slice(&std::fs::read(graph)?)?;
    let stages = data["stages"].as_array().ok_or("missing stages")?;
    let inputs: Vec<_> = stages
        .iter()
        .map(|s| {
            Ok(crate::lifetime::Input {
                name: s["name"].as_str().ok_or("missing stage name")?,
                sql: s["sql"].as_str().ok_or("missing stage SQL")?,
                temporary: s["temporary"] == true,
            })
        })
        .collect::<Result<_, &str>>()?;
    let indexes: Vec<_> = stages.iter().map(|s| s["index"].as_str()).collect();
    let queries: Vec<String> = data["queries"]
        .as_array()
        .ok_or("missing result packets")?
        .iter()
        .map(|s| s.as_str().ok_or("invalid result SQL").map(String::from))
        .collect::<Result<_, _>>()?;
    let compiled = crate::ownership::compile(&inputs, &indexes, &queries, &definitions);
    for (i, stage) in data["stages"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        stage["sql"] = compiled.sql[i].clone().into();
        stage["slot"] = serde_json::to_value(compiled.layout.slot(i))?;
        stage["members"] = serde_json::to_value(compiled.layout.members(i))?;
        if compiled.layout.slot(i).is_some() {
            stage["index"] = Value::Null;
        }
    }
    let source = capture
        .parent()
        .ok_or("missing capture directory")?
        .join(data["database"].as_str().ok_or("missing source database")?);
    let parent = output.parent().ok_or("missing output directory")?;
    let target = parent.join("candidate-owner-input.sqlite");
    if target.exists() {
        return Err("output database exists; use a fresh directory".into());
    }
    std::fs::create_dir_all(parent)?;
    std::fs::copy(source, &target)?;
    data["database"] = "candidate-owner-input.sqlite".into();
    data["format"] = "rerail.candidate-replay/v3".into();
    data["setup"] = serde_json::to_value(compiled.layout.setup)?;
    data["graph"] = serde_json::to_value(definitions)?;
    data["schedule"] = serde_json::to_value(compiled.schedule.actions)?;
    data["peakLiveRelations"] = compiled.schedule.peak_relations.into();
    data["queries"] = serde_json::to_value(compiled.queries)?;
    data["rewriteNote"] =
        "Production SQL ownership compiler; frozen inputs and consumer ordinals retained".into();
    std::fs::write(output, serde_json::to_vec_pretty(&data)?)?;
    Ok(())
}
