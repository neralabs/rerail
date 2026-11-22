//! The tests of the engine's napi items (`code_rule_dsl/mod.rs`): the package facts a host reads through
//! `packageFacts` and `PackageFactsReader`, against the repository ingestion's and a file set's derivations, and the
//! server graph `serverGraphAsync` reads on disk. Every other test of the module graph and the workspace context is a
//! test crate's (`rerail-tests-scope`, `rerail-tests-workspace`).

use crate::code_rule_dsl::module_graph::fixtures::{HOOK, NEXT_MANIFEST, disk_graph, graph, temp_root, write_file};
use crate::code_rule_dsl::module_graph::{config_on_disk, on_disk};
use crate::code_rule_dsl::workspace::fixtures::{TempDir, git, package_repository};
use crate::code_rule_dsl::workspace::index::{repository_module, repository_package};
use crate::git_workspace;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc as StdArc;

#[test]
fn on_disk_reads_the_package_and_follows_changes() {
    let root = temp_root("changes");
    let write = |path: &str, text: &str| write_file(&root, path, text);
    write(
        "web/package.json",
        r#"{ "dependencies": { "next": "15" } }"#,
    );
    write(
        "web/app/page.tsx",
        "import './hook';\nexport default function Page() {}\n",
    );
    write("web/app/hook.ts", HOOK);
    write("web/node_modules/x/page.tsx", HOOK);
    write("web/.next/app/page.tsx", HOOK);
    // Through the binding hosts call: members relative to the package's folder.
    assert_eq!(
        crate::code_rule_dsl::ServerGraphTask::new(
            root.to_string_lossy().into_owned(),
            "web".to_owned(),
            None,
            None
        )
        .members()
        .unwrap(),
        ["app/hook.ts", "app/page.tsx"]
    );
    // A page that now imports through a client boundary drops the hook module.
    write(
        "web/app/page.tsx",
        "import './client';\nexport default function Page() {}\n",
    );
    write("web/app/client.tsx", "'use client';\nimport './hook';\n");
    assert_eq!(
        on_disk(&root, "web", &config_on_disk(&root, "web"), None)
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        ["app/page.tsx"]
    );
    // An overlay reads a source as a host has it: an unsaved page importing the hook again, a new
    // page, or the page deleted.
    let overlaid = |path: &str, source: Option<&str>| {
        crate::code_rule_dsl::ServerGraphTask::new(
            root.to_string_lossy().into_owned(),
            "web".to_owned(),
            Some(path.to_owned()),
            source.map(str::to_owned),
        )
        .members()
        .unwrap()
    };
    assert_eq!(
        overlaid("web/app/page.tsx", Some("import './hook';\n")),
        ["app/hook.ts", "app/page.tsx"]
    );
    assert_eq!(
        overlaid("web/app/new/page.tsx", Some("export default 1;\n")),
        ["app/new/page.tsx", "app/page.tsx"]
    );
    assert_eq!(overlaid("web/app/page.tsx", None), Vec::<String>::new());
    // An overlay is read as the walk reads a file: not in a hidden folder.
    assert_eq!(
        overlaid("web/app/.generated/page.tsx", Some("export default 1;\n")),
        ["app/page.tsx"]
    );
    // An overlay making the root app folder present hides src/app.
    write(
        "web/src/app/page.tsx",
        "export default function Page() {}\n",
    );
    std::fs::remove_dir_all(root.join("web/app")).unwrap();
    assert_eq!(overlaid("web/app/x.ts", None), ["src/app/page.tsx"]);
    assert_eq!(
        overlaid("web/app/x.ts", Some("export {};\n")),
        Vec::<String>::new()
    );
    // Even an overlay the graph does not read as a source (a hidden folder) makes the folder present.
    assert_eq!(
        overlaid("web/app/.generated/page.tsx", Some("export {};\n")),
        Vec::<String>::new()
    );
    // The configuration chain the package reads, as conformance forwards it with a corpus.
    write(
        "web/tsconfig.json",
        r#"{ "extends": "../shared/tsconfig.base.json" }"#,
    );
    write("shared/tsconfig.base.json", "{}");
    assert_eq!(
        crate::code_rule_dsl::module_config_files(
            root.to_string_lossy().into_owned(),
            "web".to_owned()
        ),
        ["web/tsconfig.json", "shared/tsconfig.base.json"]
    );
    std::fs::remove_dir_all(&root).unwrap();
}

/// A source or folder that cannot be read leaves the package without a graph, and the failure is
/// not remembered: once it reads, the graph is complete.
#[cfg(unix)]
#[test]
fn on_disk_gives_no_graph_while_a_source_or_folder_cannot_be_read() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_root("unreadable");
    write_file(
        &root,
        "package.json",
        r#"{ "dependencies": { "next": "15" } }"#,
    );
    write_file(
        &root,
        "app/page.tsx",
        "import '../lib/a';\nimport '../parts/c';\nexport default 1;\n",
    );
    write_file(&root, "lib/a.ts", "import './b';\n");
    write_file(&root, "lib/b.ts", "");
    write_file(&root, "parts/c.ts", "");
    let complete = ["app/page.tsx", "lib/a.ts", "lib/b.ts", "parts/c.ts"];
    let chmod = |path: &str, mode: u32| {
        std::fs::set_permissions(root.join(path), std::fs::Permissions::from_mode(mode)).unwrap();
    };
    chmod("lib/a.ts", 0o000);
    // Permissions do not bind the superuser.
    if std::fs::read(root.join("lib/a.ts")).is_err() {
        assert!(disk_graph(&root, "").is_none());
        // The binding resolves no members (null), not an empty graph.
        let task = crate::code_rule_dsl::ServerGraphTask::new(
            root.to_string_lossy().into_owned(),
            String::new(),
            None,
            None,
        );
        assert_eq!(task.members(), None);
        chmod("lib/a.ts", 0o644);
        assert_eq!(disk_graph(&root, "").unwrap(), complete);
        chmod("parts", 0o000);
        assert!(disk_graph(&root, "").is_none());
        chmod("parts", 0o755);
        assert_eq!(disk_graph(&root, "").unwrap(), complete);
    }
    chmod("lib/a.ts", 0o644);
    std::fs::remove_dir_all(&root).unwrap();
}

/// The walk reads no folder hosts ignore (`build`, `vendor`) and no nested checkout (a folder
/// holding `.git`), an overlay there included; a file set skips the same ignored folders.
#[test]
fn the_graph_reads_no_ignored_folder_or_nested_checkout() {
    let root = temp_root("ignored");
    let files = [
        ("package.json", NEXT_MANIFEST),
        (
            "app/page.tsx",
            "import '../build/a';\nimport '../vendor/b';\nimport '../checkout/c';\nimport '../lib/d';\nexport default 1;\n",
        ),
        ("build/a.ts", ""),
        ("vendor/b.ts", ""),
        ("checkout/c.ts", ""),
        ("lib/d.ts", ""),
    ];
    for (path, text) in files {
        write_file(&root, path, text);
    }
    write_file(&root, "checkout/.git", "gitdir: ../.git/modules/checkout\n");
    assert_eq!(disk_graph(&root, "").unwrap(), ["app/page.tsx", "lib/d.ts"]);
    let overlaid = |path: &str| {
        crate::code_rule_dsl::ServerGraphTask::new(
            root.to_string_lossy().into_owned(),
            String::new(),
            Some(path.to_owned()),
            Some("export {};\n".to_owned()),
        )
        .members()
        .unwrap()
    };
    assert_eq!(overlaid("build/a.ts"), ["app/page.tsx", "lib/d.ts"]);
    assert_eq!(overlaid("checkout/c.ts"), ["app/page.tsx", "lib/d.ts"]);
    let file_set = files
        .iter()
        .filter(|(path, _)| !path.starts_with("checkout/"))
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(graph("", &file_set).unwrap(), ["app/page.tsx", "lib/d.ts"]);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn hosts_repository_ingestion_and_file_sets_share_one_package_derivation() {
    let directory = TempDir::new("package-facts");
    let root = directory.0.join("repo");
    let oversized = format!(
        "{{\"type\":\"module\",\"description\":\"{}\"}}",
        "x".repeat(1024 * 1024)
    );
    let root_manifest = r#"{ "name": "root", "type": "esm", "bin": { "a": "./bin/../cli.js", "b": "/abs.js", "c": "../out.js" },
        "engines": { "node": "not a range" }, "devEngines": { "runtime": [{ "name": "bun" }, { "name": "node", "version": "^20.9" }] },
        "dependencies": { "react": "19" }, "devDependencies": { "vitest": "4" } }"#;
    let files = [
        ("package.json", root_manifest),
        ("a.js", ""),
        (
            "esm/package.json",
            r#"{ "type": "module", "bin": "x.js", "engines": { "node": ">=22" } }"#,
        ),
        ("esm/deep/a.js", ""),
        (
            "esm/cjs/package.json",
            r#"{ "type": "commonjs", "peerDependencies": { "vue": "3" } }"#,
        ),
        ("esm/cjs/a.js", ""),
        ("broken/package.json", "{ \"type\": \"module\""),
        ("broken/a.js", ""),
        ("large/package.json", oversized.as_str()),
        ("large/a.js", ""),
        ("linked/a.js", ""),
        ("sub/a.js", ""),
    ];
    for (path, source) in files {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, source).unwrap();
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        root.join("esm/package.json"),
        root.join("linked/package.json"),
    )
    .unwrap();
    git(&root, &["init", "-q"]);
    git(&root, &["add", "."]);
    git(
        &root,
        &[
            "-c",
            "user.name=rerail",
            "-c",
            "user.email=rerail@example.invalid",
            "commit",
            "-q",
            "-m",
            "fixture",
        ],
    );
    // What a host sends: `packageFacts`, in the shape of the wire's `package`.
    let host_as = |root: &std::path::Path, path: &str, label: &str| {
        super::package_facts(
            label.to_owned(),
            root.join(path).to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
        )
        .map(|facts| {
            let mut wire = json!({ "folder": facts.folder, "type": facts.package_type });
            if !facts.bin.is_empty() {
                wire["bin"] = json!(facts.bin);
            }
            if let Some(version) = facts.node_version {
                wire["nodeVersion"] = json!(version);
            }
            if !facts.dependencies.is_empty() {
                wire["dependencies"] = json!(facts.dependencies);
            }
            wire
        })
    };
    let host = |root: &std::path::Path, path: &str| host_as(root, path, "javascript");
    let repository = git_workspace::discover(&root).unwrap();
    let mut manifests = crate::code_rule_dsl::package_manifest::Manifests::new(root.clone());
    let mut indexed = |path: &str, language: &str| {
        repository_package(&mut manifests, &repository, path, language).map(|manifest| {
            crate::code_rule_dsl::package_manifest::of_file(&manifest)
                .expect("an ECMA manifest")
                .to_wire()
        })
    };
    // A workspace or snapshot holding the same texts (the symlink is not one of its entries).
    let entries = files
        .iter()
        .filter_map(|(path, source)| {
            let directory = crate::code_rule_dsl::package_manifest::manifest_directory(path)?;
            Some((
                directory,
                StdArc::new(crate::code_rule_dsl::package_manifest::Manifest::parse(
                    directory.to_owned(),
                    source.as_bytes(),
                )),
            ))
        })
        .collect::<HashMap<_, _>>();
    let in_file_set = |path: &str| {
        crate::code_rule_dsl::package_manifest::manifest_in(path, |directory| {
            entries.get(directory)
        })
        .map(|manifest| {
            crate::code_rule_dsl::package_manifest::of_file(&manifest)
                .expect("an ECMA manifest")
                .to_wire()
        })
    };
    // The nearest manifest decides. `bin` targets resolve against its folder (an absolute one or
    // one outside it never names a file below it); an unparsable `engines.node` falls through to
    // `devEngines.runtime`'s `node` entry. One without a valid `type`, malformed, oversized or a
    // symlink has nothing else either.
    let unreadable = |folder: &str| json!({ "folder": folder, "type": "unset" });
    for (path, expected) in [
        (
            "a.js",
            json!({ "folder": "", "type": "unset", "bin": ["cli.js"], "nodeVersion": "^20.9",
                "dependencies": ["react", "vitest"] }),
        ),
        (
            "esm/deep/a.js",
            json!({ "folder": "esm", "type": "module", "bin": ["x.js"], "nodeVersion": ">=22" }),
        ),
        (
            "esm/cjs/a.js",
            json!({ "folder": "esm/cjs", "type": "commonjs", "dependencies": ["vue"] }),
        ),
        ("broken/a.js", unreadable("broken")),
        ("large/a.js", unreadable("large")),
        ("sub/a.js", indexed("a.js", "javascript").unwrap()),
    ] {
        assert_eq!(host(&root, path), Some(expected.clone()), "{path}");
        assert_eq!(
            indexed(path, "javascript"),
            Some(expected.clone()),
            "{path}"
        );
        assert_eq!(in_file_set(path), Some(expected), "{path}");
    }
    #[cfg(unix)]
    for read in [
        host(&root, "linked/a.js"),
        indexed("linked/a.js", "javascript"),
    ] {
        assert_eq!(read, Some(unreadable("linked")));
    }
    // Only ECMA files carry one, and no manifest above the root counts.
    assert_eq!(indexed("a.js", "python"), None);
    assert_eq!(host_as(&root, "a.py", "python"), None);
    assert!(
        super::reads_package_manifests("javascript".to_owned())
            && !super::reads_package_manifests("python".to_owned())
    );
    assert_eq!(host(&root.join("sub"), "a.js"), None);
}

/// On disk, a host's `package` facts carry the manifest's `private` and each file's own
/// `published`, read from the manifest's `files` and `main` and its folder's `.npmignore` (a
/// regular file, never a symlink): one reader reads each manifest once for every file below it,
/// and the manifest's digest, which reuse keys carry, follows its ignore files.
#[test]
fn host_package_facts_read_private_and_each_files_publication_on_disk() {
    let directory = TempDir::new("package-publication");
    let root = directory.0.clone();
    std::fs::create_dir_all(root.join("lib")).unwrap();
    std::fs::create_dir_all(root.join("pkg")).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{ "files": ["lib"], "main": "main.js", "private": true }"#,
    )
    .unwrap();
    std::fs::write(root.join(".npmignore"), "lib/x.js\n").unwrap();
    std::fs::write(root.join("pkg/package.json"), "{}").unwrap();
    let absolute = |path: &str| root.join(path).to_string_lossy().into_owned();
    let reader = crate::code_rule_dsl::PackageFactsReader::new(root.to_string_lossy().into_owned());
    let facts = |path: &str| {
        reader
            .facts("javascript".to_owned(), absolute(path))
            .map(|facts| (facts.folder, facts.private, facts.published))
    };
    assert_eq!(
        facts("lib/a.js"),
        Some((String::new(), Some(true), Some(true)))
    );
    assert_eq!(
        facts("lib/x.js"),
        Some((String::new(), Some(true), Some(false)))
    );
    assert_eq!(
        facts("src/b.js"),
        Some((String::new(), Some(true), Some(false)))
    );
    assert_eq!(
        facts("main.js"),
        Some((String::new(), Some(true), Some(true)))
    );
    assert_eq!(
        facts("pkg/a.js"),
        Some(("pkg".to_owned(), None, Some(true)))
    );
    let one_shot = crate::code_rule_dsl::package_facts(
        "javascript".to_owned(),
        absolute("lib/x.js"),
        root.to_string_lossy().into_owned(),
    )
    .expect("the root manifest");
    assert_eq!(
        (one_shot.private, one_shot.published),
        (Some(true), Some(false))
    );
    // The digest follows the ignore files.
    let digest = || {
        *crate::code_rule_dsl::package_manifest::Manifests::new(root.clone())
            .of_file(&root.join("pkg/a.js"))
            .expect("the package's manifest")
            .digest()
    };
    let before = digest();
    std::fs::write(root.join("pkg/.gitignore"), "a.js\n").unwrap();
    assert_ne!(digest(), before);
    let reader = crate::code_rule_dsl::PackageFactsReader::new(root.to_string_lossy().into_owned());
    assert_eq!(
        reader
            .facts("javascript".to_owned(), absolute("pkg/a.js"))
            .and_then(|facts| facts.published),
        Some(false)
    );
    // A symlinked ignore file is not read.
    #[cfg(unix)]
    {
        std::fs::remove_file(root.join("pkg/.gitignore")).unwrap();
        std::fs::write(root.join("ignore-all"), "*\n").unwrap();
        std::os::unix::fs::symlink(root.join("ignore-all"), root.join("pkg/.npmignore")).unwrap();
        let reader =
            crate::code_rule_dsl::PackageFactsReader::new(root.to_string_lossy().into_owned());
        assert_eq!(
            reader
                .facts("javascript".to_owned(), absolute("pkg/a.js"))
                .and_then(|facts| facts.published),
            Some(true)
        );
    }
}

#[test]
fn hosts_and_repository_ingestion_share_one_python_module_derivation() {
    let repository = package_repository("module-path");
    let root = repository.0.join("repo");
    let module = |path: &str| match super::module_path(
        "python".to_owned(),
        root.join(path).to_string_lossy().into_owned(),
    ) {
        Some(napi::Either::A(dotted)) => Some(dotted),
        Some(napi::Either::B(names)) => Some(names.join("|")),
        None => None,
    };
    assert_eq!(module("alpha/x.py").as_deref(), Some("alpha.x"));
    assert_eq!(
        module("alpha/__init__.py").as_deref(),
        Some("alpha.__init__")
    );
    assert_eq!(module("loose/x.py"), None);
    let repository_module =
        |path: &str| repository_module(&git_workspace::discover(&root).unwrap(), path, "python");
    assert_eq!(repository_module("beta/x.py"), Some(json!("beta.x")));
    assert_eq!(repository_module("loose/x.py"), None);
}
