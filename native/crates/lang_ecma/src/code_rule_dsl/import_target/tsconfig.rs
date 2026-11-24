//! get-tsconfig's `getTsconfig`, as eslint-plugin-n's `getTSConfigForContext` reads it: the
//! nearest `tsconfig.json` at or above a file, with the `compilerOptions` of the configurations it
//! `extends` beneath its own.

use super::fs::View;
use super::json::Json;
use super::path;

/// An object's entries, in order.
type Entries = Vec<(String, Json)>;

/// How many `extends` a configuration chain follows.
use rerail_lang::code_rule_dsl::tsconfig::MAX_EXTENDS_DEPTH;

/// A file's project configuration: where it is, and its merged `compilerOptions`.
pub(crate) struct Tsconfig {
    /// The configuration's absolute path.
    pub(crate) path: String,
    pub(crate) compiler_options: Entries,
}

impl Tsconfig {
    /// The nearest `tsconfig.json` at or above the folder of the file at `file` (absolute), never
    /// above the view's root.
    pub(crate) fn of_file(view: &View, file: &str) -> Option<Self> {
        let mut directory = path::dirname(file).to_owned();
        loop {
            let candidate = path::join(&directory, "tsconfig.json");
            if view.is_file(&candidate) {
                let compiler_options = read(view, &candidate, 0).unwrap_or_default();
                return Some(Self {
                    path: candidate,
                    compiler_options,
                });
            }
            if directory == view.base() || directory == "/" {
                return None;
            }
            directory = path::dirname(&directory).to_owned();
        }
    }

    /// `compilerOptions[name]`.
    pub(crate) fn option(&self, name: &str) -> Option<&Json> {
        self.compiler_options
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    /// `compilerOptions[name]` where it is `true`.
    pub(crate) fn flag(&self, name: &str) -> bool {
        self.option(name) == Some(&Json::Bool(true))
    }

    /// `compilerOptions.jsx`, lowercased as get-tsconfig normalizes it.
    pub(crate) fn jsx(&self) -> Option<String> {
        self.option("jsx")
            .and_then(Json::as_str)
            .map(str::to_ascii_lowercase)
    }
}

/// `{ ...base, ...own }`: the base's keys in place, own values over them, own new keys after.
fn merge(mut base: Entries, own: Entries) -> Entries {
    for (key, value) in own {
        match base.iter_mut().find(|(known, _)| *known == key) {
            Some(entry) => entry.1 = value,
            None => base.push((key, value)),
        }
    }
    base
}

/// The merged `compilerOptions` of the configuration at `file`: those of each configuration it
/// `extends` (later ones over earlier ones), its own over them. `None` where it cannot be read.
fn read(view: &View, file: &str, depth: usize) -> Option<Entries> {
    let text = view.text(file)?;
    let config = Json::parse_jsonc(&text)?;
    config.entries()?;
    let mut options = config
        .get("compilerOptions")
        .and_then(Json::entries)
        .map(<[_]>::to_vec)
        .unwrap_or_default();
    if depth >= MAX_EXTENDS_DEPTH {
        return Some(options);
    }
    let extends = match config.get("extends") {
        Some(Json::String(base)) => vec![base.as_str()],
        Some(Json::Array(bases)) => bases.iter().filter_map(Json::as_str).collect(),
        _ => Vec::new(),
    };
    let directory = path::dirname(file);
    for base in extends.into_iter().rev() {
        let Some(base) =
            extends_path(view, base, directory).and_then(|base| read(view, &base, depth + 1))
        else {
            continue;
        };
        options = merge(base, options);
    }
    Some(options)
}

/// The view's files, as the shared `extends` resolver reads them (decision 0660).
impl rerail_lang::code_rule_dsl::tsconfig::ConfigFiles for View {
    fn base(&self) -> &str {
        View::base(self)
    }
    fn is_file(&self, path: &str) -> bool {
        View::is_file(self, path)
    }
    fn is_directory(&self, path: &str) -> bool {
        View::is_directory(self, path)
    }
    fn real_path(&self, path: &str) -> String {
        View::real_path(self, path)
    }
    fn text(&self, path: &str) -> Option<String> {
        View::text(self, path).map(|text| (*text).clone())
    }
}

/// get-tsconfig's `resolveExtendsPath` (`tsconfig::extends_path`).
fn extends_path(view: &View, extends: &str, directory: &str) -> Option<String> {
    rerail_lang::code_rule_dsl::tsconfig::extends_path(view, extends, directory)
}
