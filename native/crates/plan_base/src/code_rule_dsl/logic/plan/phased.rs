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
