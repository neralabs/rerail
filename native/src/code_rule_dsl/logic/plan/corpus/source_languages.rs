//! The host's paths and registry feed the same language rules as the Git inventory.

use super::{Error, Result, ThreadStore, front, output::{Outputs, Value}, run_fact_image};
use crate::content_store::analysis_scope::AnalysisOwner;
use crate::git_workspace::snapshot::{self, SourceLanguages};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

const REGISTRY: &str = "git_inventory_host_registry";
const SUFFIX: &str = "git_inventory_host_suffix";
const NAME: &str = "git_inventory_host_name";
const PATH: &str = "workspace_language_path";

pub(super) fn is_input(relation: &str) -> bool {
    matches!(relation, REGISTRY | SUFFIX | NAME | PATH)
}

pub(super) fn inputs(program: &(impl crate::code_rule_dsl::logic::program_image::fields::ProgramFields + ?Sized), root: &Path, options: &serde_json::Value) -> BTreeMap<String, Vec<Vec<front::Value>>> {
    use front::Value::Str;
    let mut rows: BTreeMap<String, Vec<Vec<front::Value>>> = program.input_names().into_iter()
        .filter(|name| is_input(name)).map(|name| (name, Vec::new())).collect();
    if let Some(paths) = rows.get_mut(PATH)
        && let Some(input) = options.get("workspaceLanguagePaths").and_then(|paths| paths.as_array())
    {
        paths.extend(input.iter().filter_map(|path| path.as_str()).map(|path| vec![Str(path.into())]));
    }
    if let Some(registry) = rows.get_mut(REGISTRY)
        && let Some(languages) = options.get("workspaceLanguageRegistry")
            .and_then(|value| serde_json::from_value::<SourceLanguages>(value.clone()).ok())
            .or_else(|| snapshot::source_languages(root))
    {
        use sha2::Digest;
        let version = crate::content_store::observe::hex(
            &sha2::Sha256::digest(serde_json::to_vec(&languages).expect("language registry")).into(),
        );
        registry.push(vec![Str(version.as_str().into())]);
        for (relation, labels) in [(SUFFIX, languages.suffixes), (NAME, languages.names)] {
            if let Some(entries) = rows.get_mut(relation) {
                entries.extend(labels.into_iter().map(|(selector, language)| {
                    vec![Str(version.as_str().into()), Str(selector.into()), Str(language.into())]
                }));
            }
        }
    }
    rows
}

pub fn classify(
    root: &Path,
    store: &Path,
    owner: Option<AnalysisOwner>,
    paths: &[String],
    registry: &SourceLanguages,
) -> Result<HashMap<String, String>> {
    if paths.is_empty() {
        return Ok(HashMap::new());
    }
    let _store = ThreadStore::select(store, owner.map(|owner| (root.to_owned(), owner)));
    let program = concat!(
        include_str!("../../../../../../docs/rule-syntax/lib/core.rerail"), "\n",
        include_str!("../../../../../../docs/rule-syntax/lib/source-language.rerail"), "\n",
        include_str!("../../../typing/programs/workspace-source-languages.rerail"),
    );
    let basename = |path: &str| path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let names: std::collections::BTreeSet<_> = paths.iter().map(|path| basename(path)).collect();
    let options = serde_json::json!({
        "reuse": false,
        "inputFiles": [],
        "workspaceLanguagePaths": names,
        "workspaceLanguageRegistry": registry,
    });
    let image = run_fact_image(program, root, &options)?;
    let outputs = Outputs::of(&image).map_err(Error::Plan)?;
    let labels = outputs.rows("source_language").iter().map(|row| {
        let (Value::Text(path), Value::Text(language)) = (row.get(0), row.get(1)) else {
            return Err(Error::Plan("invalid source language fact".into()));
        };
        Ok((path.to_owned(), language.to_owned()))
    }).collect::<Result<HashMap<_, _>>>()?;
    Ok(paths.iter().filter_map(|path| Some((path.clone(), labels.get(&basename(path))?.clone()))).collect())
}
