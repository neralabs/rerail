use super::fs::{HeldFiles, View};
use super::{ModuleType, Style, target};
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rerail-import-target-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    fn write(&self, path: &str, text: &str) {
        let file = self.0.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn on_disk(root: &Path) -> View {
    View::new(Arc::default(), Some(root))
}

/// The upstream `no-missing` fixture's shape: relative files, packages with `exports`, a
/// types-only package and a tsconfig with `paths`.
fn no_missing() -> TempDir {
    let directory = TempDir::new("no-missing");
    for (path, text) in [
        ("a.js", ""),
        ("b.json", "{}"),
        ("d.ts", ""),
        ("e.tsx", ""),
        ("bar/main.js", ""),
        ("foo/main.js", ""),
        ("foo/package.json", r#"{ "main": "main.js" }"#),
        ("lib/index.js", ""),
        (
            "node_modules/esm-module/package.json",
            r#"{ "exports": { ".": { "import": "./entry.mjs" }, "./sub": "./sub.mjs" } }"#,
        ),
        ("node_modules/esm-module/entry.mjs", ""),
        ("node_modules/esm-module/sub.mjs", ""),
        (
            "node_modules/types-only/package.json",
            r#"{ "exports": { ".": { "types": "./types.d.ts" } } }"#,
        ),
        ("node_modules/types-only/types.d.ts", ""),
        ("node_modules/cjs/package.json", "{}"),
        ("node_modules/cjs/index.js", ""),
        (
            "ts-paths/tsconfig.json",
            r#"{ "compilerOptions": { "paths": { "@direct": ["./some/where.ts"], "@wild/*": ["./some/*"] } } }"#,
        ),
        ("ts-paths/some/where.ts", ""),
        (
            "ts-allow/tsconfig.json",
            r#"{ "compilerOptions": { "allowImportingTsExtensions": true } }"#,
        ),
        ("ts-allow/file.ts", ""),
        (
            "ts-react/base.json",
            r#"{ "compilerOptions": { "jsx": "React" } }"#,
        ),
        ("ts-react/tsconfig.json", r#"{ "extends": "./base.json" }"#),
        ("ts-react/e.tsx", ""),
        (
            "imports/package.json",
            r##"{ "imports": { "#is-ip": "./ip.js" } }"##,
        ),
        ("imports/ip.js", ""),
    ] {
        directory.write(path, text);
    }
    directory
}

/// (resolved, failed) of each target.
fn outcome(view: &View, importer: &str, specifier: &str, style: Style) -> (bool, bool) {
    let found = target(view, importer, specifier, style);
    (found.file_path.is_some(), found.error.is_some())
}

#[test]
fn targets_resolve_as_enhanced_resolve_resolves_them_for_eslint_plugin_n() {
    let directory = no_missing();
    let view = on_disk(&directory.0);
    let import =
        |importer: &str, specifier: &str| outcome(&view, importer, specifier, Style::Import);
    let require =
        |importer: &str, specifier: &str| outcome(&view, importer, specifier, Style::Require);
    // Relative files with or without an extension, and a missing one: a relative target keeps its
    // path either way.
    assert_eq!(import("test.js", "./a.js"), (true, false));
    assert_eq!(import("test.js", "./a"), (true, false));
    assert_eq!(import("test.js", "./b.json"), (true, false));
    assert_eq!(import("test.js", "./c"), (true, true));
    assert_eq!(import("test.js", "./a.json"), (true, true));
    // An import names no folder's main or index; a require does.
    assert_eq!(import("test.js", "./foo"), (true, true));
    assert_eq!(import("test.js", "./bar/"), (true, true));
    assert_eq!(require("test.js", "./foo"), (true, false));
    assert_eq!(require("test.js", "./lib"), (true, false));
    assert_eq!(import("test.js", "."), (true, true));
    // Packages: exports under the style's conditions; the main and index of a package without.
    assert_eq!(import("test.js", "esm-module"), (true, false));
    assert_eq!(require("test.js", "esm-module"), (false, true));
    assert_eq!(import("test.js", "esm-module/sub"), (true, false));
    assert_eq!(import("test.js", "esm-module/sub.mjs"), (false, true));
    assert_eq!(import("test.js", "cjs"), (true, false));
    assert_eq!(import("test.js", "no-exist-package-0"), (false, true));
    assert_eq!(import("test.js", "mocha!foo?a=b"), (false, true));
    // A type import reads the `types` condition.
    assert_eq!(
        outcome(&view, "test.ts", "types-only", Style::Type),
        (true, false)
    );
    assert_eq!(import("test.ts", "types-only"), (false, true));
    // A name below a built-in falls back to it: ignored, neither resolved nor failed.
    assert_eq!(import("test.js", "fs/foo"), (false, false));
    // Data and http imports are their own path.
    assert_eq!(import("test.js", "data:text/javascript,x"), (true, false));
    assert_eq!(import("test.js", "https://example.com/m.js"), (true, false));
    // TypeScript: a `.js` specifier names the `.ts` file (and only it, by default), a `.tsx` one
    // under a react configuration found through `extends`.
    assert_eq!(import("test.ts", "./d.js"), (true, false));
    assert_eq!(import("test.ts", "./a.js"), (true, true));
    assert_eq!(import("test.tsx", "./e.jsx"), (true, false));
    assert_eq!(import("ts-react/test.tsx", "./e.js"), (true, false));
    assert_eq!(import("ts-allow/test.ts", "./file.ts"), (true, false));
    assert_eq!(import("ts-allow/test.ts", "./file.js"), (true, true));
    // tsconfig `paths`.
    assert_eq!(import("ts-paths/test.ts", "@direct"), (true, false));
    assert_eq!(import("ts-paths/test.ts", "@wild/where.js"), (true, false));
    assert!(target(&view, "ts-paths/test.ts", "@wild/where.js", Style::Import).aliased(&view));
    assert!(!target(&view, "test.js", "@wild/where.js", Style::Import).aliased(&view));
    // The package's `imports`.
    assert_eq!(import("imports/test.js", "#is-ip"), (true, false));
    assert_eq!(import("test.js", "#is-ip"), (false, true));
}

#[test]
fn module_types_and_names_read_the_specifier() {
    let view = View::new(Arc::default(), None);
    for (specifier, module_type, name) in [
        ("./a", ModuleType::Relative, None),
        ("..", ModuleType::Relative, None),
        ("/a", ModuleType::Absolute, None),
        ("fs/promises", ModuleType::Node, Some("fs")),
        ("node:fs", ModuleType::Node, Some("fs")),
        ("data:x", ModuleType::Data, None),
        ("http://x", ModuleType::Http, None),
        ("@scope/name/sub", ModuleType::Npm, Some("@scope/name")),
        ("lodash/get", ModuleType::Npm, Some("lodash")),
        (
            "virtual:package-name",
            ModuleType::Npm,
            Some("virtual:package-name"),
        ),
        ("#internal", ModuleType::Unknown, None),
        ("@scope", ModuleType::Unknown, None),
    ] {
        let found = target(&view, "a.js", specifier, Style::Import);
        assert_eq!(
            (found.module_type, found.module_name.as_deref()),
            (module_type, name),
            "{specifier}"
        );
    }
}

#[test]
fn the_specifier_spells_the_extension_file_extension_in_import_expects() {
    let directory = TempDir::new("extension");
    for path in [
        "a.js",
        "d.ts",
        "e.tsx",
        "my-folder/index.js",
        "utils.client.ts",
        "multi.js",
        "multi.json",
    ] {
        directory.write(path, "");
    }
    let view = on_disk(&directory.0);
    let spelled = |importer: &str, specifier: &str| {
        target(&view, importer, specifier, Style::Import).extension_spelled(&view)
    };
    assert_eq!(spelled("test.js", "./a.js"), Some(true));
    assert_eq!(spelled("test.js", "./a"), Some(false));
    assert_eq!(spelled("test.js", "./my-folder"), Some(false));
    assert_eq!(spelled("test.js", "./my-folder/index.js"), Some(true));
    assert_eq!(spelled("test.js", "./missing"), Some(true));
    assert_eq!(spelled("test.ts", "./d"), Some(false));
    assert_eq!(spelled("test.ts", "./d.js"), Some(true));
    assert_eq!(spelled("test.ts", "./utils.client"), Some(false));
    assert_eq!(spelled("test.tsx", "./e.jsx"), Some(true));
    assert_eq!(spelled("test.js", "./multi"), Some(false));
    assert_eq!(spelled("test.js", "fs"), None);
}

#[test]
fn packages_decide_publication_and_listed_dependencies() {
    let directory = TempDir::new("packages");
    for (path, text) in [
        (
            "package.json",
            r#"{ "name": "root", "files": ["lib"], "dependencies": { "dep": "1" }, "devDependencies": { "dev": "1", "both": "1" }, "peerDependencies": { "both": "1" }, "workspaces": ["packages/*"] }"#,
        ),
        ("lib/a.js", ""),
        ("test/helper.js", ""),
        (
            "packages/app/package.json",
            r#"{ "dependencies": { "own": "1" } }"#,
        ),
        ("packages/app/src/index.js", ""),
        ("other/package.json", "{}"),
    ] {
        directory.write(path, text);
    }
    let view = on_disk(&directory.0);
    let published = |importer: &str, specifier: &str| {
        target(&view, importer, specifier, Style::Import).published(&view)
    };
    assert_eq!(published("lib/index.js", "./a.js"), Some(true));
    assert_eq!(published("lib/index.js", "../test/helper.js"), Some(false));
    assert_eq!(
        published("packages/app/src/index.js", "../../../lib/a.js"),
        Some(false)
    );
    assert_eq!(published("lib/index.js", "dep"), None);
    let dev = |specifier: &str| {
        target(&view, "lib/index.js", specifier, Style::Import).dev_dependency(&view)
    };
    assert_eq!(
        (dev("dev"), dev("both"), dev("dep"), dev("./x")),
        (Some(true), Some(false), Some(false), None)
    );
    let declared = |importer: &str, specifier: &str, style: Style| {
        target(&view, importer, specifier, style).declared(&view)
    };
    assert_eq!(declared("lib/index.js", "root", Style::Import), Some(true));
    assert_eq!(declared("lib/index.js", "nope", Style::Import), Some(false));
    // A workspace package reads its root's names too; a package no workspace claims does not.
    assert_eq!(
        declared("packages/app/src/index.js", "dep", Style::Import),
        Some(true)
    );
    assert_eq!(
        declared("packages/app/src/index.js", "own", Style::Import),
        Some(true)
    );
    assert_eq!(declared("other/x.js", "dep", Style::Import), Some(false));
    assert_eq!(declared("lib/index.js", "./a", Style::Import), None);
}

#[test]
fn a_file_set_without_a_root_resolves_among_its_own_files() {
    let mut held = HeldFiles::default();
    for (path, text) in [
        ("src/a.ts", ""),
        ("src/index.ts", ""),
        ("package.json", r#"{ "name": "x" }"#),
    ] {
        held.hold(path, &Arc::new(text.to_owned()), None);
    }
    let view = View::new(Arc::new(held), None);
    assert_eq!(
        outcome(&view, "src/index.ts", "./a.js", Style::Import),
        (true, false)
    );
    assert_eq!(
        outcome(&view, "src/index.ts", "./b.js", Style::Import),
        (true, true)
    );
    assert_eq!(
        outcome(&view, "src/index.ts", "lodash", Style::Import),
        (false, true)
    );
    assert_eq!(
        target(&view, "src/index.ts", "lodash", Style::Import).declared(&view),
        Some(false)
    );
}

#[test]
fn holding_a_file_changes_resolution_only_where_disk_below_the_root_differs() {
    let directory = TempDir::new("held");
    directory.write("src/a.js", "export {}");
    directory.write("package.json", r#"{ "name": "x" }"#);
    let root = Some(directory.0.as_path());
    let text = |text: &str| Arc::new(text.to_owned());
    let mut held = HeldFiles::default();
    // A source disk holds reads only as present, whatever its text.
    assert!(!held.hold("src/a.js", &text("export const a = 1"), root));
    assert!(!held.hold("src/a.js", &text("export const a = 2"), root));
    assert!(!held.release("src/a.js", root));
    // A manifest's text is read: the same as disk's changes nothing, another text does.
    assert!(!held.hold("package.json", &text(r#"{ "name": "x" }"#), root));
    assert!(held.hold("package.json", &text(r#"{ "name": "y" }"#), root));
    assert!(held.release("package.json", root));
    // A file disk lacks appears and disappears; without a root every new file does.
    assert!(held.hold("src/b.js", &text(""), root));
    assert!(held.release("src/b.js", root));
    assert!(held.hold("src/a.js", &text(""), None));
    assert!(!held.release("src/missing.js", root));
}
