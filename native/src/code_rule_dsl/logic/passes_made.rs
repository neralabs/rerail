//! The passes a program was made into (decision 0597): what a made program's `.pass` settings say, for the passes
//! that read it below the contexts' own crate (hiding: a context's guests are read by its rules alone). `contexts`
//! makes them and re-exports these.

use super::program_image::Version;
use super::program_image::fields::ProgramFields;

/// The prefix of a context's instance of a relation (`contexts::instance`).
pub const PREFIX: &str = "rr_cx_";

/// Whether relation `rel` is a context's instance of one: its name is an instance's, under any prefix a later pass
/// gives a program's relations (a pack's `p0_`, a file-local part's `fl<k>_`).
pub fn of_a_context(rel: &str) -> bool {
    rel.contains(PREFIX)
}

/// A setting's words, a comma one of them.
pub fn words(text: &str) -> Vec<String> {
    text.replace(',', " , ").split_whitespace().map(str::to_owned).collect()
}

/// A pass made (`contexts::passes`): a context, its host and its roots, as the made program's `.pass` settings hold
/// them (`.pass annotation of python roots ann_root [holes REL] exports ann_type, …`: the roots as the made program
/// names them).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Made {
    pub name: String,
    pub host: String,
    /// The host's relation of its roots, as the made program names it.
    pub roots: String,
}

/// The passes program `v` was made into (`contexts::passes`), from its `.pass` settings.
pub fn made(v: &Version) -> Vec<Made> {
    ProgramFields::settings(v)
        .into_iter()
        .filter(|(k, _)| k == "pass")
        .filter_map(|(_, text)| match words(&text).as_slice() {
            [name, of, host, roots, rel, exports, ..] if of == "of" && roots == "roots" && exports == "exports" => Some(Made { name: name.clone(), host: host.clone(), roots: rel.clone() }),
            [name, of, host, roots, rel, holes, _, exports, ..] if of == "of" && roots == "roots" && holes == "holes" && exports == "exports" => Some(Made { name: name.clone(), host: host.clone(), roots: rel.clone() }),
            _ => None,
        })
        .collect()
}
