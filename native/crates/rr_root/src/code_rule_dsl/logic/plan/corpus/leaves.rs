//! The leaves a run reads, and what each is now (t33 K4): one table, by the leaf's kind. A leaf is what a run read
//! from outside its program, each kept as `(kind, path, digest)` (`Report::consulted`, a frame's `frame_read`):
//! - the documents it read (`doc`, by their bytes) and the rule files its text includes (`rule`);
//! - what its resolution rules listed, read, loaded or found missing (`world::World::consulted`);
//! - what its input readers read: repository files, directories, artifacts, a base revision, Git's state
//!   (`inputs::populate`).
//!
//! A leaf is as it was when its reader answers now what the run recorded. A frame whose every leaf is as it was is
//! the empty leaf delta, and the run takes the frame (`reuse`, `phases::phase_one`).

use super::world::World;
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
use std::collections::BTreeMap;
use std::path::Path;

/// The leaf readers of a run of `prog` under `root`: the world's (made once, as a run's requests are), and the input
/// readers' declarations (read once, at the first artifact).
pub(super) struct Leaves<'p, P: ProgramFields + ?Sized> {
    root: &'p Path,
    /// None for leaves read before any program is (a command's rule files): no world's leaf among them.
    prog: Option<&'p P>,
    world: Option<World>,
    artifacts: Option<Option<BTreeMap<String, Vec<String>>>>,
}

/// What a leaf is now, against what it was.
enum Now {
    Is(String),
    Still(bool),
}

impl<'p, P: ProgramFields + ?Sized> Leaves<'p, P> {
    pub(super) fn new(root: &'p Path, prog: &'p P) -> Self {
        Leaves { root, prog: Some(prog), world: None, artifacts: None }
    }

    /// Whether leaf `(kind, path, digest)` reads as it did. A kind no reader knows is not.
    pub(super) fn still(&mut self, (kind, path, digest): &(String, String, String)) -> bool {
        match self.now(kind, path, digest) {
            Some(Now::Is(now)) => now == *digest,
            Some(Now::Still(still)) => still,
            None => false,
        }
    }

    fn now(&mut self, kind: &str, path: &str, digest: &str) -> Option<Now> {
        use super::inputs;
        let root = self.root;
        Some(Now::Is(match kind {
            // A document listed unread (none of the run's rules reads one) is as it was whatever its bytes.
            "doc" if digest.is_empty() => return Some(Now::Still(true)),
            "doc" | "rule" => super::world::bytes_digest(&root.join(path)),
            // A directory of rule files, by its entries (a command's: the files it runs are those listed).
            "rule_dir" => super::observe(|o| o.listing_digest(&root.join(path))).map(|d| crate::content_store::observe::hex(&d)).unwrap_or_default(),
            "package_marker" => crate::git_workspace::snapshot::is_file(root.join(path).join("__init__.py")).to_string(),
            "input" => inputs::of_result(&inputs::read_repository_file(root, path)),
            "input_dir" => inputs::listing(root, path),
            "artifact" => {
                let prog = self.prog?;
                let declared = self.artifacts.get_or_insert_with(|| inputs::artifacts(&prog.settings()).ok()).as_ref()?;
                let (text, read) = inputs::read_artifact(root, declared, path);
                inputs::artifact_entry(&text, &read)
            }
            "base" => {
                let (rev, file) = path.split_once('\t')?;
                inputs::of_result(&inputs::base_reader(root, (!rev.is_empty()).then_some(rev))(file))
            }
            super::git::SELECTED_DIRECTORY => super::git::selection_stamp(root),
            "git_object_predicate" => return Some(Now::Still(crate::code_rule_dsl::logic::strings::git::predicate_still(root, path, digest))),
            "git_root_repo" => return Some(Now::Still(super::git::still(root, path, digest))),
            "git_rev" | "git_index" | "git_tracking_index" | "git_revision_rows" => return Some(Now::Still(crate::code_rule_dsl::logic::strings::git::still(root, kind, path, digest))),
            super::analysis_history::INPUT => return Some(Now::Still(super::analysis_history::Read::from_key(path).is_some_and(|read| read.records().is_ok_and(|(_, current)| current == digest)))),
            // The world's: what the resolution rules asked.
            _ => {
                let prog = self.prog?;
                self.world.get_or_insert_with(|| World::new(root, prog)).observation(kind, path)?
            }
        }))
    }
}

impl<'p> Leaves<'p, super::Summary> {
    /// The leaf readers of what is read before a program: rule files and their directories, documents.
    pub(super) fn files(root: &'p Path) -> Self {
        Leaves { root, prog: None, world: None, artifacts: None }
    }
}
