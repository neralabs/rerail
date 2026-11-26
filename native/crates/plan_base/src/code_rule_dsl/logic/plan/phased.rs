//! The inputs a run binds beside its programs' own, shared by every program of the run (`corpus::combine` does not
//! prefix them): an earlier phase's exports (`<phase>__<export>`), and the relations the corpus runner fills from
//! Git, the analysis history and the source languages (`corpus::{git, analysis_history, source_languages}`). Their
//! names are the planner's, so that typing, under the corpus, knows them (`typing::file_local`).

/// The separator of a phase's export's name (`<phase>__<export>`).
pub const SEP: &str = "__";

/// Git's inputs (`corpus::git`).
pub mod git {
    pub const ROOT: &str = "git_root_repo";
    pub const VIEW: &str = "git_root_view";
    pub const REPOSITORY: &str = "git_repository";
    pub const LOCATION: &str = "git_location_of";
    pub const TREE: &str = "git_source_tree";
    pub const COMMIT: &str = "git_source_commit";
    pub const SELECTION: &str = "git_inventory_selection";
    pub const SOURCE_BLOB: &str = "git_selected_source_blob";
    pub const SELECTED_DIRECTORY: &str = "git_selected_directory";
    pub const INPUTS: [&str; 9] = [ROOT, VIEW, REPOSITORY, LOCATION, TREE, COMMIT, SELECTION, SOURCE_BLOB, SELECTED_DIRECTORY];
}

/// The analysis history's inputs (`corpus::analysis_history`).
pub mod analysis_history {
    pub const INPUT: &str = "analysis_run_record";
    pub const SCOPE: &str = "analysis_scope_record";

    pub fn is_input(name: &str) -> bool {
        matches!(name, INPUT | SCOPE)
    }
}

/// The host's paths and registry (`corpus::source_languages`).
pub mod source_languages {
    pub const REGISTRY: &str = "git_inventory_host_registry";
    pub const SUFFIX: &str = "git_inventory_host_suffix";
    pub const NAME: &str = "git_inventory_host_name";
    pub const PATH: &str = "workspace_language_path";

    pub fn is_input(relation: &str) -> bool {
        matches!(relation, REGISTRY | SUFFIX | NAME | PATH)
    }
}

/// Whether `rel` names an earlier phase's export (`<phase>__<export>`), or an input the runner binds with the phases'
/// (Git's, the analysis history's and the source languages'): shared by every program of a run
/// (`corpus::combine` does not prefix it).
pub fn is_phased(rel: &str) -> bool {
    analysis_history::is_input(rel) || git::INPUTS.contains(&rel) || source_languages::is_input(rel)
        || rel.split_once(SEP).is_some_and(|(p, e)| !p.is_empty() && !e.is_empty() && !p.starts_with("rr_") && !e.contains(SEP))
}

/// The prefix of a stage's name (decision 0709): a phase cut from the program it runs before, `staged_<digest>`.
pub const STAGE: &str = "staged_";

/// A stage's probe (decision 0709 §3): the values a reader of a phase's export fixes, per file.
pub const PROBE: &str = "rr_probe_";

/// Whether `rel` is a stage's export (`staged_<digest>__<relation>`, decision 0709).
pub fn is_stage_export(rel: &str) -> bool {
    rel.starts_with(STAGE) && is_phased(rel)
}

/// Whether `rel` is an earlier phase's export read by probes (decision 0709 §3): a stage's or a phase program's
/// (`<phase>__<export>`), not phase 1's (`observer__…`: the run's files) nor the runner's other inputs. A reader
/// takes it as a placeless input, and a file's memo entry is keyed by the rows of it its probes read.
pub fn is_probed(rel: &str) -> bool {
    is_phased(rel) && rel.split_once(SEP).is_some_and(|(p, _)| p != "observer" && p != "sources")
}

/// How a file's results read a setting, and so how it keys the file's memo entry (decisions 0665, 0705), as the
/// setting's maker declares it: a phase after its export (`.export reads_at by_path 1, exports by_key`), the library
/// for the project's settings (`core.rerail`'s `.keyed package_for by_path 1, package by_key, env`). A relation
/// declared with none is no setting.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Keyed {
    /// Keyed by a file's path in column 0; `Some(c)`: column `c` names keys whose `ByKey` rows the file reads too.
    ByPath(Option<usize>),
    /// Keyed by column 0, a key a `ByPath` row of the reading file names.
    ByKey,
    /// Read whole by every file (`env`, `package_export`).
    Whole,
}

/// The items of a list of declared keys (`R by_path C, S by_key, T`), each its relation and its key words as written
/// (`["by_path", "1"]`); `.` ends the list.
pub fn key_items(texts: &[String]) -> Vec<(String, Vec<String>)> {
    texts
        .iter()
        .flat_map(|t| t.split(','))
        .filter_map(|item| {
            let mut words = item.trim().trim_end_matches('.').split_whitespace().map(str::to_owned);
            Some((words.next()?, words.collect()))
        })
        .collect()
}

/// A phase's `.export` items (`key_items`), each the relation its program makes, the export's name its readers read
/// (`<phase>__<export>`), and its key's words: `asked_exports as exports by_key` hands on relation `asked_exports` as
/// export `exports` (decision 0705: an asked phase answers under the name its readers already read, where an
/// included library makes a relation of that name eagerly).
pub fn export_items(texts: &[String]) -> Vec<(String, String, Vec<String>)> {
    key_items(texts)
        .into_iter()
        .map(|(relation, words)| match words.split_first() {
            Some((w, rest)) if w == "as" && !rest.is_empty() => (relation, rest[0].clone(), rest[1..].to_vec()),
            _ => (relation.clone(), relation, words),
        })
        .collect()
}

/// The key `words` write (`by_path`, `by_path C`, `by_key`), or why they are none.
pub fn key_of(words: &[String]) -> Result<Keyed, String> {
    match words.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => Ok(Keyed::Whole),
        ["by_key"] => Ok(Keyed::ByKey),
        ["by_path"] => Ok(Keyed::ByPath(None)),
        ["by_path", c] => c.parse().map(|c| Keyed::ByPath(Some(c))).map_err(|_| format!("{c} is no column")),
        _ => Err(format!("`{}`: write `by_path`, `by_path C` or `by_key`", words.join(" "))),
    }
}

/// The declared keys of the phases' exports a run reads, by the reader's relation (`<phase>__<export>`), in the run's
/// context (decision 0442): bound once the run's phases have run (`corpus::run_observed_within`), and by an asked
/// phase's rounds for its own reads (`corpus::phases`). A phase in a run's own phase directory is known so.
#[derive(Clone, Default)]
pub struct DeclaredKeys(pub std::sync::Arc<Vec<(String, Keyed)>>);

/// The keys a file declares, read once a process for each state of the file (`declared_in`).
#[derive(Default)]
struct KeyFiles(std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, (Option<std::time::SystemTime>, u64, std::sync::Arc<Vec<(String, Keyed)>>)>>);

/// The keys the directive `directive` of the `.rerail` file at `path` declares (`.export` items with a key, or every
/// `.keyed` item): none for a file that is not there or does not parse.
fn declared_in(path: &std::path::Path, directive: &str) -> std::sync::Arc<Vec<(String, Keyed)>> {
    let meta = std::fs::metadata(path).ok();
    let stamp = (meta.as_ref().and_then(|m| m.modified().ok()), meta.as_ref().map_or(0, |m| m.len()));
    let files = rerail_prof::code_rule_dsl::logic::profile::shared::<KeyFiles>();
    if let Some((m, l, keys)) = files.0.lock().expect("the declared keys").get(path)
        && (*m, *l) == stamp
    {
        return std::sync::Arc::clone(keys);
    }
    let texts: Vec<String> = std::fs::read_to_string(path)
        .ok()
        .and_then(|src| rerail_rr_syntax::syntax::Parser::new(&src).and_then(|mut p| p.statements()).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(item, _)| match item {
            rerail_rr_syntax::syntax::Item::Setting { key, text } if key == directive => Some(text),
            _ => None,
        })
        .collect();
    let keys: Vec<(String, Keyed)> = if directive == "keyed" {
        key_items(&texts).into_iter().filter_map(|(r, w)| Some((r, key_of(&w).ok()?))).collect()
    } else {
        export_items(&texts).into_iter().filter(|(_, _, w)| !w.is_empty()).filter_map(|(_, e, w)| Some((e, key_of(&w).ok()?))).collect()
    };
    let keys = std::sync::Arc::new(keys);
    files.0.lock().expect("the declared keys").insert(path.to_path_buf(), (stamp.0, stamp.1, std::sync::Arc::clone(&keys)));
    keys
}

/// The keys the `.export` line of the phase program at `path` declares, by export (`by_path`, `by_key`): none for a file
/// that is not there or does not parse.
pub fn declared_exports_in(path: &std::path::Path) -> std::sync::Arc<Vec<(String, Keyed)>> {
    declared_in(path, "export")
}

/// The engine's own phase programs (`corpus::phases`): `<name>.rerail`.
pub fn builtin_phase_dir() -> std::path::PathBuf {
    rerail_store::source_tree::native().join("src/code_rule_dsl/typing/programs")
}

/// The switch that lets a phase declare `.rounds`, and runs an engine phase's asked program (`<name>.asked.rerail`)
/// in place of its eager one (decision 0705).
pub const ASKED_PHASES: &str = "RERAIL_ASKED_PHASES";

/// Whether asked phases run (`ASKED_PHASES`).
pub fn asked_phases_on() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::var(ASKED_PHASES).is_ok_and(|v| !v.is_empty() && v != "0" && v != "off")
}

/// The file of the engine's phase program `name`: its asked program while asked phases run and it has one
/// (`py_names.asked.rerail`), else `<name>.rerail`.
pub fn builtin_phase_file(name: &str) -> std::path::PathBuf {
    let dir = builtin_phase_dir();
    let asked = dir.join(format!("{name}.asked.rerail"));
    if asked_phases_on() && asked.exists() { asked } else { dir.join(format!("{name}.rerail")) }
}

/// How `rel` is keyed as a setting, if it is one: a phase's export as the run's phases declared it (`DeclaredKeys`),
/// else as the engine's phase program declares it (`<phase>__<export>`), else a project setting as the library's
/// `core.rerail` declares it.
pub fn declared_key(rel: &str) -> Option<Keyed> {
    if let Some(k) = rerail_prof::code_rule_dsl::logic::profile::with_bound(|k: &DeclaredKeys| k.0.iter().find(|(r, _)| r == rel).map(|(_, k)| k.clone())) {
        return Some(k);
    }
    let found = |keys: std::sync::Arc<Vec<(String, Keyed)>>, name: &str| keys.iter().find(|(r, _)| r == name).map(|(_, k)| k.clone());
    match rel.split_once(SEP) {
        Some((phase, export)) if !phase.is_empty() && !export.is_empty() && !phase.starts_with("rr_") => found(declared_in(&builtin_phase_file(phase), "export"), export),
        Some(_) => None,
        None => found(declared_in(&rerail_rr::code_rule_dsl::logic::rr::library().join("core.rerail"), "keyed"), rel),
    }
}

/// Every declared key bound here, in order of their relations.
pub fn declared_keys() -> Vec<(String, Keyed)> {
    rerail_prof::code_rule_dsl::logic::profile::with_bound(|k: &DeclaredKeys| k.0.as_ref().clone())
}

/// Binds `more` beside the keys bound already, until the guard drops: a phase's run inside a reader's keeps the
/// reader's.
pub fn bind_keys(more: impl IntoIterator<Item = (String, Keyed)>) -> rerail_prof::code_rule_dsl::logic::profile::Bound {
    let mut all = declared_keys();
    for (rel, k) in more {
        if !all.iter().any(|(r, _)| *r == rel) {
            all.push((rel, k));
        }
    }
    all.sort();
    rerail_prof::code_rule_dsl::logic::profile::bind(DeclaredKeys(std::sync::Arc::new(all)))
}
