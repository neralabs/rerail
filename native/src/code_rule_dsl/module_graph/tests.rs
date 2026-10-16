use super::*;

/// The server graph of a file set (path, text) whose `package.json` entries are its manifests and
/// whose configuration entries the package's `Config` reads.
fn graph(folder: &str, files: &[(&str, &str)]) -> Vec<String> {
    let manifests = files
        .iter()
        .filter_map(|(path, _)| {
            path.strip_suffix("/package.json")
                .or_else(|| (*path == "package.json").then_some(""))
        })
        .collect::<HashSet<_>>();
    let read = |path: &str| {
        files
            .iter()
            .find(|(entry, _)| *entry == path)
            .map(|(_, text)| (*text).to_owned())
    };
    in_file_set(
        folder,
        files.iter().copied(),
        &|directory| manifests.contains(directory),
        &Config::read(folder, &read),
    )
    .into_iter()
    .collect()
}

const HOOK: &str =
    "import { useState } from 'react';\nexport function useThing() { return useState(0); }\n";

#[test]
fn a_summary_reads_the_directive_prologue_and_every_import_form() {
    let summary = summarize(
        "a.tsx",
        "#!/usr/bin/env node\n// note\n'use strict';\n\"use client\";\nimport a from './a';\nimport type { T } from './types';\nimport './side';\nexport { b } from './b';\nexport type { U } from './u';\nexport * from './star';\nimport x = require('./cjs');\nconst lazy = () => import('./lazy');\nconst c = require('./c');\nconst d = require(name);\n",
    )
    .unwrap();
    assert!(summary.client);
    assert_eq!(
        summary.specifiers,
        ["./a", "./side", "./b", "./star", "./cjs", "./lazy", "./c"]
    );
    // A directive after the prologue is a plain string statement.
    assert!(
        !summarize("a.ts", "import a from 'a';\n'use client';\n")
            .unwrap()
            .client
    );
    assert!(
        !summarize("a.js", "const x = 'use client';\n")
            .unwrap()
            .client
    );
    assert!(
        summarize("a.mjs", "'use client'\nexport default 1;\n")
            .unwrap()
            .client
    );
    assert!(summarize("a.json", "{}").is_none());
    // Comments inside a directive or before a literal argument are no part of it.
    assert!(
        summarize(
            "a.tsx",
            "'use client' /* browser entry */;\nexport default 1;\n"
        )
        .unwrap()
        .client
    );
    assert_eq!(
        summarize(
            "a.ts",
            "import(/* webpackChunkName: \"hook\" */ './hook');\nrequire(/* why */ './c');\n"
        )
        .unwrap()
        .specifiers,
        ["./hook", "./c"]
    );
}

#[test]
fn a_module_imported_only_from_a_client_module_is_client_code() {
    // zechub: the hook module is imported only from a "use client" widget, which the page imports.
    let files = [
        ("package.json", r#"{ "dependencies": { "next": "15" } }"#),
        (
            "app/page.tsx",
            "import Widget from './widget';\nexport default function Page() { return <Widget />; }\n",
        ),
        (
            "app/widget.tsx",
            "'use client';\nimport { useThing } from './adapters/react';\nexport default function Widget() { useThing(); return null; }\n",
        ),
        ("app/adapters/react.ts", HOOK),
    ];
    assert_eq!(graph("", &files), ["app/page.tsx"]);
    // The same module imported by the page too is in the server graph, as Next bundles it there.
    let mut shared = files.to_vec();
    shared[1].1 = "import Widget from './widget';\nimport { useThing } from './adapters/react';\nexport default function Page() { useThing(); return <Widget />; }\n";
    assert_eq!(
        graph("", &shared),
        ["app/adapters/react.ts", "app/page.tsx"]
    );
}

#[test]
fn the_graph_follows_imports_out_of_app_and_through_every_resolution() {
    let files = [
        ("package.json", r#"{ "dependencies": { "next": "15" } }"#),
        (
            "tsconfig.json",
            "{\n  // Next's default alias, through a base configuration\n  \"extends\": \"./tsconfig.base\",\n  \"compilerOptions\": { \"strict\": true, },\n}\n",
        ),
        (
            "tsconfig.base.json",
            r#"{ "compilerOptions": { "paths": { "@/*": ["./src/*"], "~lib": ["./lib/index.ts"] } } }"#,
        ),
        (
            "src/app/(marketing)/page.tsx",
            "import { a } from '@/components/a';\nimport l from '~lib';\nexport default function Page() {}\n",
        ),
        ("src/components/a.ts", "export * from './b.js';\n"),
        (
            "src/components/b.ts",
            "import c from '../util';\nexport const b = 1;\n",
        ),
        ("src/util/index.tsx", "export default 1;\n"),
        ("lib/index.ts", "export default 2;\n"),
        // A private folder holds no route, and a client page starts no server graph.
        ("src/app/_parts/page.tsx", HOOK),
        (
            "src/app/client/page.tsx",
            "'use client';\nimport '@/components/only-client';\n",
        ),
        ("src/components/only-client.ts", HOOK),
        // Unreferenced, and a module of another package by name.
        ("src/components/unused.ts", HOOK),
    ];
    assert_eq!(
        graph("", &files),
        [
            "lib/index.ts",
            "src/app/(marketing)/page.tsx",
            "src/components/a.ts",
            "src/components/b.ts",
            "src/util/index.tsx",
        ]
    );
}

#[test]
fn base_url_resolves_bare_specifiers_and_a_nested_package_is_no_edge() {
    let files = [
        (
            "apps/web/package.json",
            r#"{ "dependencies": { "next": "15" } }"#,
        ),
        (
            "apps/web/jsconfig.json",
            r#"{ "compilerOptions": { "baseUrl": "." } }"#,
        ),
        (
            "apps/web/app/layout.jsx",
            "import Nav from 'components/nav';\nimport Ui from './ui/button';\nexport default function Layout() {}\n",
        ),
        (
            "apps/web/components/nav.jsx",
            "export default function Nav() {}\n",
        ),
        ("apps/web/app/ui/package.json", "{}"),
        ("apps/web/app/ui/button.jsx", HOOK),
        (
            "apps/docs/app/page.tsx",
            "export default function Page() {}\n",
        ),
    ];
    assert_eq!(
        graph("apps/web", &files),
        ["app/layout.jsx", "components/nav.jsx"]
    );
}

/// A fresh directory below the system's temporary folder.
fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rerail-module-graph-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// Writes `bytes` to `path` below `root`, creating its folders.
fn write_file(root: &Path, path: &str, bytes: impl AsRef<[u8]>) {
    let file = root.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, bytes).unwrap();
}

/// The members `on_disk` gives the package in `folder` below `root`.
fn disk_graph(root: &Path, folder: &str) -> Vec<String> {
    on_disk(root, folder, &config_on_disk(root, folder), None)
        .into_iter()
        .collect()
}

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
        .members(),
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

/// A file set reads the files a disk walk does: none in `node_modules` or a hidden folder, and no
/// graph past the size bound; a root `app` folder hides `src/app` from Next.
#[test]
fn a_file_set_reads_what_a_disk_walk_reads() {
    let mut files = vec![
        (
            "package.json".to_owned(),
            r#"{ "dependencies": { "next": "15" } }"#.to_owned(),
        ),
        (
            "app/page.tsx".to_owned(),
            "export default function Page() {}\n".to_owned(),
        ),
        ("node_modules/x/app/page.tsx".to_owned(), HOOK.to_owned()),
        (".next/server/app/page.tsx".to_owned(), HOOK.to_owned()),
        ("src/app/page.tsx".to_owned(), HOOK.to_owned()),
    ];
    let members = |files: &[(String, String)]| {
        graph(
            "",
            &files
                .iter()
                .map(|(path, text)| (path.as_str(), text.as_str()))
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(members(&files), ["app/page.tsx"]);
    // Without the root app folder, src/app holds the entries.
    files.remove(1);
    assert_eq!(members(&files), ["src/app/page.tsx"]);
    // A root app folder of no source, or one that is another package, still hides src/app.
    files.push(("app/globals.css".to_owned(), String::new()));
    assert!(members(&files).is_empty());
    files.pop();
    files.push(("app/package.json".to_owned(), "{}".to_owned()));
    files.push(("app/x.ts".to_owned(), String::new()));
    assert!(members(&files).is_empty());
    files.pop();
    files.pop();
    // A source larger than a disk walk reads imports nothing, here as there.
    let oversized = format!(
        "export * from '../lib/hook';\n//{}\n",
        "x".repeat(MAX_SOURCE_BYTES as usize)
    );
    files.push(("src/app/layout.tsx".to_owned(), oversized));
    files.push(("lib/hook.ts".to_owned(), HOOK.to_owned()));
    assert_eq!(members(&files), ["src/app/layout.tsx", "src/app/page.tsx"]);
    // Past MAX_GRAPH_FILES sources, no graph.
    files.extend((0..MAX_GRAPH_FILES).map(|index| (format!("lib/f{index}.ts"), String::new())));
    assert!(members(&files).is_empty());
}

/// The `jsxImportSource` the configuration entries `files` give the package in `folder`.
fn jsx_import_source(folder: &str, files: &[(String, String)]) -> Option<String> {
    let read = |path: &str| {
        files
            .iter()
            .find(|(entry, _)| entry == path)
            .map(|(_, text)| text.clone())
    };
    Config::read(folder, &read)
        .jsx_import_source()
        .map(str::to_owned)
}

#[test]
fn a_configuration_reads_as_typescript_reads_jsonc() {
    // A trailing comma before a comment and the closing bracket, of either comment form.
    for text in [
        "{ \"compilerOptions\": { \"jsxImportSource\": \"preact\", /* runtime */ }, }",
        "{ \"compilerOptions\": { \"jsxImportSource\": \"preact\", // runtime\n }, // done\n }",
        "{ \"compilerOptions\": { \"jsxImportSource\": \"preact\" /* a, } */ } }",
    ] {
        let files = [("tsconfig.json".to_owned(), text.to_owned())];
        assert_eq!(
            jsx_import_source("", &files).as_deref(),
            Some("preact"),
            "{text}"
        );
    }
    // A string holding comment or comma text is a string.
    let files = [(
        "tsconfig.json".to_owned(),
        r#"{ "compilerOptions": { "jsxImportSource": "a//b,}" } }"#.to_owned(),
    )];
    assert_eq!(jsx_import_source("", &files).as_deref(), Some("a//b,}"));
}

#[test]
fn extends_follows_only_project_configuration_names() {
    let base = r#"{ "compilerOptions": { "jsxImportSource": "preact" } }"#;
    for (name, followed) in [
        ("tsconfig.base.json", true),
        ("tsconfig-base.json", true),
        ("jsconfig.shared.json", true),
        ("base.json", false),
    ] {
        let files = [
            (
                "apps/web/tsconfig.json".to_owned(),
                format!(r#"{{ "extends": "../../config/{name}" }}"#),
            ),
            (format!("config/{name}"), base.to_owned()),
        ];
        assert_eq!(
            jsx_import_source("apps/web", &files).as_deref(),
            followed.then_some("preact"),
            "{name}"
        );
        assert_eq!(
            is_module_config(&format!("config/{name}")),
            followed,
            "{name}"
        );
    }
}

#[test]
fn an_oversized_configuration_is_absent_whichever_path_reads_it() {
    let oversized = format!(
        "{{ \"compilerOptions\": {{ \"jsxImportSource\": \"react\" }} }}{}",
        " ".repeat(MAX_SOURCE_BYTES as usize)
    );
    let files = [
        ("tsconfig.json".to_owned(), oversized),
        (
            "jsconfig.json".to_owned(),
            r#"{ "compilerOptions": { "jsxImportSource": "preact" } }"#.to_owned(),
        ),
    ];
    assert_eq!(jsx_import_source("", &files).as_deref(), Some("preact"));
}

#[test]
fn a_configuration_with_a_byte_order_mark_reads_as_typescript_reads_it() {
    let files = [(
        "tsconfig.json".to_owned(),
        "\u{feff}{ \"compilerOptions\": { \"jsxImportSource\": \"preact\" } }".to_owned(),
    )];
    assert_eq!(jsx_import_source("", &files).as_deref(), Some("preact"));
}

#[test]
fn an_extends_naming_a_drive_or_share_reads_nothing_outside_the_workspace() {
    // Each base would be read on Windows from a drive or an SMB share, not the workspace: the
    // chain reads none of them, so their `jsxImportSource` is absent, whatever the reader holds.
    let base = "{ \"compilerOptions\": { \"jsxImportSource\": \"preact\" } }";
    for extends in [
        "./x/../C:/Users/v/tsconfig.json",
        "./x/../\\\\\\\\host\\\\share/tsconfig.json",
        "./x/..\\\\..\\\\tsconfig.base.json",
        "./tsconfig\\u0000.json",
    ] {
        let tsconfig = format!("{{ \"extends\": \"{extends}\" }}");
        let read = |path: &str| {
            Some(if path == "tsconfig.json" {
                tsconfig.clone()
            } else {
                base.to_owned()
            })
        };
        let config = Config::read("", &read);
        assert_eq!(config.jsx_import_source(), None, "{extends}");
        assert_eq!(config.files(), ["tsconfig.json"], "{extends}");
    }
}

#[test]
fn paths_beyond_the_pattern_and_target_bounds_are_ignored() {
    // Patterns past the first MAX_PATHS_PATTERNS (in key order) map nothing.
    let patterns = (0..MAX_PATHS_PATTERNS + 6)
        .map(|index| format!(r#""@p{index:02}/*": ["./p{index:02}/*"]"#))
        .collect::<Vec<_>>()
        .join(", ");
    let config = format!(r#"{{ "compilerOptions": {{ "paths": {{ {patterns} }} }} }}"#);
    let last = MAX_PATHS_PATTERNS - 1;
    let past = MAX_PATHS_PATTERNS;
    let page = format!(
        "import '@p00/x';\nimport '@p{last:02}/x';\nimport '@p{past:02}/x';\nexport default 1;\n"
    );
    let first = "p00/x.ts";
    let kept = format!("p{last:02}/x.ts");
    let dropped = format!("p{past:02}/x.ts");
    let files = [
        ("package.json", r#"{ "dependencies": { "next": "15" } }"#),
        ("tsconfig.json", config.as_str()),
        ("app/page.tsx", page.as_str()),
        (first, ""),
        (kept.as_str(), ""),
        (dropped.as_str(), ""),
    ];
    assert_eq!(graph("", &files), ["app/page.tsx", first, &kept]);
    // Targets past the first MAX_PATHS_TARGETS of a pattern are not probed.
    let targets = (0..MAX_PATHS_TARGETS + 4)
        .map(|index| format!(r#""./t{index:02}/*""#))
        .collect::<Vec<_>>()
        .join(", ");
    let config = format!(r#"{{ "compilerOptions": {{ "paths": {{ "@t/*": [{targets}] }} }} }}"#);
    let last = format!("t{:02}/b.ts", MAX_PATHS_TARGETS - 1);
    let past = format!("t{:02}/c.ts", MAX_PATHS_TARGETS);
    let files = [
        ("package.json", r#"{ "dependencies": { "next": "15" } }"#),
        ("tsconfig.json", config.as_str()),
        (
            "app/page.tsx",
            "import '@t/a';\nimport '@t/b';\nimport '@t/c';\nexport default 1;\n",
        ),
        ("t00/a.ts", ""),
        (last.as_str(), ""),
        (past.as_str(), ""),
    ];
    assert_eq!(graph("", &files), ["app/page.tsx", "t00/a.ts", &last]);
}

#[test]
fn a_specifier_is_read_once_per_module_and_resolved_per_directory() {
    assert_eq!(
        summarize(
            "a.ts",
            "import a from './a';\nexport { b } from './a';\nconst c = require('./a');\nimport('./b');\nimport './b';\n"
        )
        .unwrap()
        .specifiers,
        ["./a", "./b"]
    );
    // The same specifier from another directory names another module.
    let files = [
        ("package.json", r#"{ "dependencies": { "next": "15" } }"#),
        (
            "app/page.tsx",
            "import './x';\nimport './sub/y';\nexport default 1;\n",
        ),
        ("app/x.ts", ""),
        ("app/sub/y.ts", "import './x';\n"),
        ("app/sub/x.ts", ""),
    ];
    assert_eq!(
        graph("", &files),
        ["app/page.tsx", "app/sub/x.ts", "app/sub/y.ts", "app/x.ts"]
    );
}

/// Next's default `pageExtensions` (tsx, ts, jsx, js) name the entries; an `.mjs` or `.mts`
/// module is still an import target.
#[test]
fn only_next_page_extensions_name_entries() {
    let files = [
        ("package.json", r#"{ "dependencies": { "next": "15" } }"#),
        ("app/page.mjs", HOOK),
        ("app/other/page.mts", HOOK),
        (
            "app/layout.tsx",
            "import './helper';\nimport './typed.mjs';\n",
        ),
        ("app/helper.mjs", "export {};\n"),
        ("app/typed.mts", "export {};\n"),
    ];
    assert_eq!(
        graph("", &files),
        ["app/helper.mjs", "app/layout.tsx", "app/typed.mts"]
    );
}

#[test]
fn on_disk_decodes_a_source_lossily() {
    let root = temp_root("lossy");
    write_file(
        &root,
        "package.json",
        r#"{ "dependencies": { "next": "15" } }"#,
    );
    write_file(
        &root,
        "app/page.tsx",
        b"import './hook';\n// \xff\xfe not UTF-8\nexport default function Page() {}\n",
    );
    write_file(&root, "app/hook.ts", HOOK);
    assert_eq!(disk_graph(&root, ""), ["app/hook.ts", "app/page.tsx"]);
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
        assert!(disk_graph(&root, "").is_empty());
        chmod("lib/a.ts", 0o644);
        assert_eq!(disk_graph(&root, ""), complete);
        chmod("parts", 0o000);
        assert!(disk_graph(&root, "").is_empty());
        chmod("parts", 0o755);
        assert_eq!(disk_graph(&root, ""), complete);
    }
    chmod("lib/a.ts", 0o644);
    std::fs::remove_dir_all(&root).unwrap();
}
