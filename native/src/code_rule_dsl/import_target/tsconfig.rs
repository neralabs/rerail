//! get-tsconfig's `getTsconfig`, as eslint-plugin-n's `getTSConfigForContext` reads it: the
//! nearest `tsconfig.json` at or above a file, with the `compilerOptions` of the configurations it
//! `extends` beneath its own.

use super::fs::View;
use super::path;
use super::json::Json;

/// An object's entries, in order.
type Entries = Vec<(String, Json)>;

/// How many `extends` a configuration chain follows.
const MAX_EXTENDS_DEPTH: usize = 8;

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
        let Some(base) = extends_path(view, base, directory)
            .and_then(|base| read(view, &base, depth + 1))
        else {
            continue;
        };
        options = merge(base, options);
    }
    Some(options)
}

/// get-tsconfig's `resolveExtendsPath`: a relative or absolute path (`.json` appended where it
/// names no file), or a package's configuration in a `node_modules` folder at or above
/// `directory` (its `tsconfig` field, or the subpath it names, else its `tsconfig.json`).
fn extends_path(view: &View, extends: &str, directory: &str) -> Option<String> {
    let mut target = if extends == ".." {
        "../tsconfig.json".to_owned()
    } else {
        extends.to_owned()
    };
    if target.starts_with('.') {
        target = path::resolve(directory, &target);
    }
    if target.starts_with('/') {
        if view.is_file(&target) {
            return Some(target);
        }
        if !target.ends_with(".json") && view.is_file(&format!("{target}.json")) {
            return Some(format!("{target}.json"));
        }
        return None;
    }
    let mut parts = extends.splitn(2, '/');
    let first = parts.next()?;
    let rest = parts.next().unwrap_or("");
    let (package, subpath) = if first.starts_with('@') {
        let mut scoped = rest.splitn(2, '/');
        let name = scoped.next().unwrap_or("");
        (format!("{first}/{name}"), scoped.next().unwrap_or("").to_owned())
    } else {
        (first.to_owned(), rest.to_owned())
    };
    let mut search = directory.to_owned();
    let folder = loop {
        let candidate = path::join(&search, &format!("node_modules/{package}"));
        if view.is_directory(&candidate) {
            break view.real_path(&candidate);
        }
        if search == view.base() || search == "/" {
            return None;
        }
        search = path::dirname(&search).to_owned();
    };
    let manifest = path::join(&folder, "package.json");
    if subpath.is_empty()
        && let Some(Ok(json)) = view.json(&manifest)
        && let Some(field) = json.get("tsconfig").and_then(Json::as_str)
    {
        let named = path::join(&folder, field);
        if view.is_file(&named) {
            return Some(named);
        }
    }
    let named = path::join(&folder, if subpath.is_empty() { "tsconfig.json" } else { &subpath });
    if !named.ends_with(".json") && view.is_file(&format!("{named}.json")) {
        return Some(format!("{named}.json"));
    }
    if view.is_file(&named) {
        return Some(named);
    }
    let nested = path::join(&named, "tsconfig.json");
    view.is_file(&nested).then_some(nested)
}
