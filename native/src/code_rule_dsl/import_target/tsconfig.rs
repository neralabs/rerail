//! get-tsconfig's `getTsconfig`, as eslint-plugin-n's `getTSConfigForContext` reads it: the
//! nearest `tsconfig.json` at or above a file, with the `compilerOptions` of the configurations it
//! `extends` beneath its own.

use std::collections::HashMap;

use super::fs::View;
use super::json::Json;
use super::path;

/// An object's entries, in order.
pub(crate) type Entries = Vec<(String, Json)>;

/// How many `extends` a configuration chain follows.
const MAX_EXTENDS_DEPTH: usize = 8;

/// How many configurations one configuration's chain (`Chains::of`) reads that the walk has not read before: where its
/// bases fan out past it, the configuration is none.
const MAX_CONFIGURATIONS: usize = 256;

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
                let compiler_options = Chains::new(view, false).of(&candidate).unwrap_or_default();
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

/// The merged `compilerOptions` of configurations: those of each configuration one `extends` (later ones over earlier
/// ones), its own over them. A walk reads each configuration once at each depth, however many configurations extend
/// it (a diamond of shared bases, a folder's configurations over one base). Each configuration asked for (`of`) reads
/// at most `MAX_CONFIGURATIONS` it has not read before; one whose chain fans out past that is no configuration
/// (`None`), not a partial merge, and nothing of its cut chain is kept.
pub(crate) struct Chains<'a> {
    view: &'a View,
    /// Whether each configuration's `baseUrl` is made absolute from its own folder before the merge, as TypeScript
    /// resolves one an extended configuration sets (`project`'s rows).
    based: bool,
    /// Each configuration's merged options, by its absolute path and its depth in the chain; `None` where it cannot be
    /// read.
    read: HashMap<(String, usize), Option<Entries>>,
    /// How many more configurations the chain being read may read.
    left: usize,
    /// Whether the chain being read ran out of `left`.
    cut: bool,
}

impl<'a> Chains<'a> {
    pub(crate) fn new(view: &'a View, based: bool) -> Self {
        Self {
            view,
            based,
            read: HashMap::new(),
            left: MAX_CONFIGURATIONS,
            cut: false,
        }
    }

    /// The merged `compilerOptions` of the configuration at `file` (absolute). `None` where it cannot be read, or
    /// where its chain reads more than `MAX_CONFIGURATIONS` configurations not read before.
    pub(crate) fn of(&mut self, file: &str) -> Option<Entries> {
        (self.left, self.cut) = (MAX_CONFIGURATIONS, false);
        let options = self.options(file, 0);
        options.filter(|_| !self.cut)
    }

    /// `of` for a configuration `depth` bases down a chain: read once, and kept unless the chain was cut.
    fn options(&mut self, file: &str, depth: usize) -> Option<Entries> {
        let key = (file.to_owned(), depth);
        if let Some(known) = self.read.get(&key) {
            return known.clone();
        }
        if self.left == 0 {
            self.cut = true;
            return None;
        }
        self.left -= 1;
        let made = self.merged(file, depth);
        if !self.cut {
            self.read.insert(key, made.clone());
        }
        made
    }

    /// The configuration at `file`'s own options over its bases'.
    fn merged(&mut self, file: &str, depth: usize) -> Option<Entries> {
        let config = Json::parse_jsonc(&self.view.text(file)?)?;
        config.entries()?;
        let mut options = self.own(file, &config);
        if depth >= MAX_EXTENDS_DEPTH {
            return Some(options);
        }
        let directory = path::dirname(file);
        for base in bases(&config).into_iter().rev() {
            let Some(base) = extends_path(self.view, base, directory)
                .and_then(|base| self.options(&base, depth + 1))
            else {
                continue;
            };
            options = merge(base, options);
        }
        Some(options)
    }

    /// The configuration's own `compilerOptions`, its `baseUrl` absolute from its folder where the walk is `based`.
    fn own(&self, file: &str, config: &Json) -> Entries {
        let mut options = config
            .get("compilerOptions")
            .and_then(Json::entries)
            .map(<[_]>::to_vec)
            .unwrap_or_default();
        for (_, value) in options
            .iter_mut()
            .filter(|(key, _)| self.based && key == "baseUrl")
        {
            if let Json::String(url) = &*value {
                *value = Json::String(path::resolve(path::dirname(file), url));
            }
        }
        options
    }
}

/// The configurations a configuration `extends`, as written: one, or a list.
fn bases(config: &Json) -> Vec<&str> {
    match config.get("extends") {
        Some(Json::String(base)) => vec![base.as_str()],
        Some(Json::Array(bases)) => bases.iter().filter_map(Json::as_str).collect(),
        _ => Vec::new(),
    }
}

/// get-tsconfig's `resolveExtendsPath`: a relative or absolute path (`.json` appended where it
/// names no file), or a package's configuration in a `node_modules` folder at or above
/// `directory` (its `tsconfig` field, or the subpath it names, else its `tsconfig.json`). An
/// absolute path is one `path.isAbsolute` reads as one where Rerail runs: on Windows a drive or
/// UNC path too (`path::platform_absolute`).
fn extends_path(view: &View, extends: &str, directory: &str) -> Option<String> {
    let mut target = if extends == ".." {
        "../tsconfig.json".to_owned()
    } else {
        extends.to_owned()
    };
    if target.starts_with('.') {
        target = path::resolve(directory, &target);
    } else if let Some(absolute) = path::platform_absolute(&target) {
        target = absolute;
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
        (
            format!("{first}/{name}"),
            scoped.next().unwrap_or("").to_owned(),
        )
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
    let named = path::join(
        &folder,
        if subpath.is_empty() {
            "tsconfig.json"
        } else {
            &subpath
        },
    );
    if !named.ends_with(".json") && view.is_file(&format!("{named}.json")) {
        return Some(format!("{named}.json"));
    }
    if view.is_file(&named) {
        return Some(named);
    }
    let nested = path::join(&named, "tsconfig.json");
    view.is_file(&nested).then_some(nested)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::import_target::fs::HeldFiles;
    use std::sync::Arc;

    /// A chain whose bases fan out (each of eight configurations extending the next six times over) is read once per
    /// configuration and depth, not once per path through it, and still merges the last base's options beneath the
    /// first's.
    #[test]
    fn a_fan_out_of_bases_reads_each_base_once() {
        let root = std::env::temp_dir().join(format!(
            "rerail-tsconfig-fan-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for k in 0..8 {
            let next = format!("\"./c{}.json\"", k + 1);
            std::fs::write(
                root.join(format!("c{k}.json")),
                format!(
                    "{{ \"extends\": [{}], \"compilerOptions\": {{ \"o{k}\": true }} }}",
                    [next.as_str(); 6].join(", ")
                ),
            )
            .unwrap();
        }
        std::fs::write(
            root.join("c8.json"),
            r#"{ "compilerOptions": { "strict": true, "baseUrl": "src" } }"#,
        )
        .unwrap();
        let view = View::new(Arc::new(HeldFiles::default()), Some(&root));
        let mut chains = Chains::new(&view, true);
        let options = chains.of(&view.absolute("c0.json")).expect("read");
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            chains.read.len() <= 9,
            "{} configurations read",
            chains.read.len()
        );
        assert!(
            options
                .iter()
                .any(|(k, v)| k == "strict" && *v == Json::Bool(true))
        );
        assert!(
            options
                .iter()
                .any(|(k, v)| k == "o0" && *v == Json::Bool(true))
        );
        assert!(options.iter().any(|(k, v)| k == "baseUrl" && v.as_str().is_some_and(|url| url.ends_with("/src"))));
    }

    /// Each configuration asked for has a budget of its own: a tree of more configurations than one budget, each over
    /// one base, gives every one; a configuration whose own chain reads past its budget gives none, and the next is
    /// read whole.
    #[test]
    fn each_configuration_reads_within_its_own_budget() {
        let root = std::env::temp_dir().join(format!(
            "rerail-tsconfig-budget-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("base.json"),
            r#"{ "compilerOptions": { "strict": true } }"#,
        )
        .unwrap();
        let n = MAX_CONFIGURATIONS + 44;
        for k in 0..n {
            std::fs::write(
                root.join(format!("t{k}.json")),
                r#"{ "extends": "./base.json" }"#,
            )
            .unwrap();
            std::fs::write(root.join(format!("b{k}.json")), "{}").unwrap();
        }
        let wide: Vec<String> = (0..n).map(|k| format!("\"./b{k}.json\"")).collect();
        std::fs::write(
            root.join("wide.json"),
            format!("{{ \"extends\": [{}] }}", wide.join(", ")),
        )
        .unwrap();
        let view = View::new(Arc::new(HeldFiles::default()), Some(&root));
        let mut chains = Chains::new(&view, false);
        let read: Vec<Option<Entries>> = (0..n)
            .map(|k| chains.of(&view.absolute(&format!("t{k}.json"))))
            .collect();
        let wide = chains.of(&view.absolute("wide.json"));
        let after = chains.of(&view.absolute("t0.json"));
        let _ = std::fs::remove_dir_all(&root);
        assert!(read.iter().all(|o| {
            o.as_ref()
                .is_some_and(|o| o.iter().any(|(k, _)| k == "strict"))
        }));
        assert!(wide.is_none(), "its chain reads past its budget");
        assert!(after.is_some_and(|o| o.iter().any(|(k, _)| k == "strict")));
    }
}
