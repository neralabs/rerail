use super::*;

/// The server graph of a file set (path, text) whose `package.json` entries are its manifests and
/// whose configuration entries the package's `Config` reads; `None` without one.
fn graph(folder: &str, files: &[(&str, &str)]) -> Option<Vec<String>> {
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
        &mut HashMap::new(),
        &|directory| manifests.contains(directory),
        &Config::read(folder, &read),
    )
    .map(|members| members.into_iter().collect())
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
    assert_eq!(graph("", &files).unwrap(), ["app/page.tsx"]);
    // The same module imported by the page too is in the server graph, as Next bundles it there.
    let mut shared = files.to_vec();
    shared[1].1 = "import Widget from './widget';\nimport { useThing } from './adapters/react';\nexport default function Page() { useThing(); return <Widget />; }\n";
    assert_eq!(
        graph("", &shared).unwrap(),
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
        graph("", &files).unwrap(),
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
        graph("apps/web", &files).unwrap(),
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

/// The members `on_disk` gives the package in `folder` below `root`; `None` without a graph.
fn disk_graph(root: &Path, folder: &str) -> Option<Vec<String>> {
    on_disk(root, folder, &config_on_disk(root, folder), None)
        .map(|members| members.into_iter().collect())
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
    assert_eq!(members(&files).unwrap(), ["app/page.tsx"]);
    // Without the root app folder, src/app holds the entries.
    files.remove(1);
    assert_eq!(members(&files).unwrap(), ["src/app/page.tsx"]);
    // A root app folder of no source, or one that is another package, still hides src/app.
    files.push(("app/globals.css".to_owned(), String::new()));
    assert!(members(&files).unwrap().is_empty());
    files.pop();
    files.push(("app/package.json".to_owned(), "{}".to_owned()));
    files.push(("app/x.ts".to_owned(), String::new()));
    assert!(members(&files).unwrap().is_empty());
    files.pop();
    files.pop();
    // A source larger than a disk walk reads imports nothing, here as there.
    let oversized = format!(
        "export * from '../lib/hook';\n//{}\n",
        "x".repeat(MAX_SOURCE_BYTES as usize)
    );
    files.push(("src/app/layout.tsx".to_owned(), oversized));
    files.push(("lib/hook.ts".to_owned(), HOOK.to_owned()));
    assert_eq!(
        members(&files).unwrap(),
        ["src/app/layout.tsx", "src/app/page.tsx"]
    );
    // Past MAX_GRAPH_FILES sources, no graph.
    files.extend((0..MAX_GRAPH_FILES).map(|index| (format!("lib/f{index}.ts"), String::new())));
    assert!(members(&files).is_none());
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

/// A configuration that does not parse sets nothing, and nothing reports it: no `paths` alias or
/// `jsxImportSource`, no `extends` base read, and the `jsconfig.json` beside a `tsconfig.json`
/// that does not parse is not read in its place.
#[test]
fn a_configuration_that_does_not_parse_sets_nothing_and_is_not_replaced() {
    let tsconfig = |closing: &str| {
        format!(
            r#"{{ "extends": "./tsconfig.base.json", "compilerOptions": {{ "jsxImportSource": "preact", "paths": {{ "@/*": ["./src/*"] }} }}{closing}"#
        )
    };
    let base = r#"{ "compilerOptions": { "baseUrl": "." } }"#;
    let jsconfig = r#"{ "compilerOptions": { "jsxImportSource": "solid-js", "paths": { "@/*": ["./src/*"] } } }"#;
    for (closing, parses) in [(" }", true), ("", false)] {
        let tsconfig = tsconfig(closing);
        let files = [
            ("package.json", r#"{ "dependencies": { "next": "15" } }"#),
            ("tsconfig.json", tsconfig.as_str()),
            ("tsconfig.base.json", base),
            ("jsconfig.json", jsconfig),
            (
                "app/page.tsx",
                "import { load } from '@/load';\nexport default 1;\n",
            ),
            ("src/load.ts", "export const load = 1;\n"),
        ];
        let reads = std::cell::RefCell::new(Vec::new());
        let read = |path: &str| {
            reads.borrow_mut().push(path.to_owned());
            files
                .iter()
                .find(|(entry, _)| *entry == path)
                .map(|(_, text)| (*text).to_owned())
        };
        let config = Config::read("", &read);
        if parses {
            assert_eq!(config.jsx_import_source(), Some("preact"));
            assert_eq!(config.files(), ["tsconfig.json", "tsconfig.base.json"]);
            assert_eq!(config.candidates("@/load"), ["./src/load", "@/load"]);
        } else {
            assert_eq!(
                config,
                Config {
                    files: vec!["tsconfig.json".to_owned()],
                    ..Config::default()
                }
            );
            assert_eq!(*reads.borrow(), ["tsconfig.json"]);
        }
        // The page reaches the module only through the alias.
        let members = graph("", &files).expect("a graph");
        let expected: &[&str] = if parses {
            &["app/page.tsx", "src/load.ts"]
        } else {
            &["app/page.tsx"]
        };
        assert_eq!(members, expected, "{closing:?}");
    }
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
    assert_eq!(graph("", &files).unwrap(), ["app/page.tsx", first, &kept]);
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
    assert_eq!(
        graph("", &files).unwrap(),
        ["app/page.tsx", "t00/a.ts", &last]
    );
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
        graph("", &files).unwrap(),
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
        graph("", &files).unwrap(),
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
    assert_eq!(
        disk_graph(&root, "").unwrap(),
        ["app/hook.ts", "app/page.tsx"]
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

const NEXT_MANIFEST: &str = r#"{ "dependencies": { "next": "15" } }"#;

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

/// Past the walk's byte budget a package has no graph, as past its file bound; a source over
/// 1 MiB, which it does not read, counts nothing.
#[test]
fn the_walk_stops_past_its_byte_budget() {
    let root = temp_root("budget");
    write_file(&root, "app/page.tsx", "x".repeat(600));
    write_file(&root, "lib/a.ts", "x".repeat(600));
    write_file(
        &root,
        "lib/big.ts",
        "x".repeat(MAX_SOURCE_BYTES as usize + 1),
    );
    let walked = |mut budget: u64| {
        let mut entries = MAX_GRAPH_ENTRIES;
        walk(&root, &root, &mut Vec::new(), &mut budget, &mut entries)
    };
    assert!(walked(1_200));
    assert!(!walked(1_199));
    std::fs::remove_dir_all(&root).unwrap();
}

/// Past the walk's entry bound a package has no graph: every entry it lists counts, a file of no
/// ECMA dialect, a folder and one it skips included, but not what a skipped folder holds.
#[test]
fn the_walk_stops_past_its_entry_bound() {
    let root = temp_root("entries");
    write_file(&root, "app/page.tsx", "export default 1;\n");
    write_file(&root, "lib/a.ts", "");
    write_file(&root, "lib/notes.txt", "");
    write_file(&root, "node_modules/x/index.js", "");
    write_file(&root, "node_modules/y/index.js", "");
    // `app`, `lib`, `node_modules`, `app/page.tsx`, `lib/a.ts` and `lib/notes.txt`.
    let walked = |mut entries: usize| {
        let mut budget = MAX_GRAPH_BYTES;
        walk(&root, &root, &mut Vec::new(), &mut budget, &mut entries)
    };
    assert!(walked(6));
    assert!(!walked(5));
    std::fs::remove_dir_all(&root).unwrap();
}

/// A rewrite that keeps a source's length and modification time (`'use server'` for
/// `'use client'`) is read again: a file modified a moment ago is not cached, and a settled one's
/// change time or file id moves (on unix its status-change time or inode, on Windows NTFS's change
/// time or file id).
#[test]
fn a_rewrite_restoring_length_and_modification_time_is_read_again() {
    let root = temp_root("restamped");
    write_file(&root, "package.json", NEXT_MANIFEST);
    write_file(&root, "app/page.tsx", "import './a';\nexport default 1;\n");
    write_file(&root, "app/b.ts", "");
    let rewrite = |directive: &str, modified: SystemTime| {
        let file = root.join("app/a.ts");
        std::fs::write(&file, format!("'{directive}';\nimport './b';\n")).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(modified)
            .unwrap();
    };
    // The stamp the walk reads for `app/a.ts`.
    let stamp = || {
        listing::list(&root.join("app"))
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| entry.name() == "a.ts")
            .and_then(|entry| Stamp::of(&entry.facts().unwrap()))
    };
    let server = ["app/a.ts", "app/b.ts", "app/page.tsx"];
    let tick = SystemTime::now();
    rewrite("use server", tick);
    assert_eq!(disk_graph(&root, "").unwrap(), server);
    rewrite("use client", tick);
    assert_eq!(disk_graph(&root, "").unwrap(), ["app/page.tsx"]);
    {
        let settled = SystemTime::now() - Duration::from_secs(3600);
        // On Windows a summary is cached once the clock has left its change time: each settled
        // rewrite there takes a change time of its own in the past, as a later walk sees it.
        let rewrites = std::cell::Cell::new(0_u64);
        let settle = || {
            rewrites.set(rewrites.get() + 1);
            #[cfg(windows)]
            crate::git_workspace::set_change_time(
                &root.join("app/a.ts"),
                settled + Duration::from_secs(rewrites.get()),
            );
        };
        rewrite("use server", settled);
        settle();
        assert_eq!(disk_graph(&root, "").unwrap(), server);
        // The change time is as coarse as a scheduler tick, so a rewrite within the tick of the
        // last one leaves it: the rewrite repeats until the stamp the walk reads moved.
        let cached = stamp();
        (0..10_000)
            .find(|_| {
                rewrite("use client", settled);
                settle();
                stamp() != cached
            })
            .expect("a rewrite moves the stamp");
        assert_eq!(disk_graph(&root, "").unwrap(), ["app/page.tsx"]);
        // A file renamed over the source (as `rsync -t` writes one) is another file.
        write_file(&root, "app/a.tmp", "'use server';\nimport './b';\n");
        std::fs::File::options()
            .write(true)
            .open(root.join("app/a.tmp"))
            .unwrap()
            .set_modified(settled)
            .unwrap();
        std::fs::rename(root.join("app/a.tmp"), root.join("app/a.ts")).unwrap();
        assert_eq!(disk_graph(&root, "").unwrap(), server);
    }
    std::fs::remove_dir_all(&root).unwrap();
}

/// A source whose change time the file clock had not left when the walk read it is read again by
/// the next walk: a rewrite in that change's own tick, keeping the length and modification time,
/// leaves the whole stamp as it was. A change time ahead of the clock stands for that tick.
#[cfg(windows)]
#[test]
fn a_source_changed_in_the_tick_of_its_read_is_read_again() {
    let root = temp_root("same-tick");
    write_file(&root, "package.json", NEXT_MANIFEST);
    write_file(&root, "app/page.tsx", "import './a';\nexport default 1;\n");
    write_file(&root, "app/b.ts", "");
    let source = root.join("app/a.ts");
    let settled = SystemTime::now() - Duration::from_secs(3600);
    let ahead = SystemTime::now() + Duration::from_secs(3600);
    let rewrite = |directive: &str| {
        std::fs::write(&source, format!("'{directive}';\nimport './b';\n")).unwrap();
        crate::git_workspace::backdate(&source, settled);
        crate::git_workspace::set_change_time(&source, ahead);
    };
    rewrite("use server");
    assert_eq!(
        disk_graph(&root, "").unwrap(),
        ["app/a.ts", "app/b.ts", "app/page.tsx"]
    );
    rewrite("use client");
    assert_eq!(disk_graph(&root, "").unwrap(), ["app/page.tsx"]);
    std::fs::remove_dir_all(&root).unwrap();
}

/// A source whose read a person denied after the walk cached its summary keeps the summary of its
/// last write, as its modification time and length show it: the denial moves NTFS's change time
/// alone. A write of another length or at another time has no summary until it is read.
#[cfg(windows)]
#[test]
fn a_source_whose_read_is_denied_keeps_the_summary_of_its_last_write() {
    let root = temp_root("denied");
    write_file(&root, "package.json", NEXT_MANIFEST);
    write_file(&root, "app/page.tsx", "import './a';\nexport default 1;\n");
    write_file(&root, "app/b.ts", "");
    let source = root.join("app/a.ts");
    let write = |text: &str, modified: SystemTime| {
        std::fs::write(&source, text).unwrap();
        crate::git_workspace::backdate(&source, modified);
    };
    let settled = SystemTime::now() - Duration::from_secs(3600);
    let server = ["app/a.ts", "app/b.ts", "app/page.tsx"];
    write("'use server';\nimport './b';\n", settled);
    assert_eq!(disk_graph(&root, "").unwrap(), server);
    let denied = crate::git_workspace::ReadDenied::new(&source);
    assert!(std::fs::read(&source).is_err(), "the read is denied");
    assert_eq!(disk_graph(&root, "").unwrap(), server);
    assert_eq!(disk_graph(&root, "").unwrap(), server);
    drop(denied);
    for (text, modified) in [
        ("'use server';\nimport './b';\n\n", settled),
        (
            "'use client';\nimport './b';\n",
            settled + Duration::from_secs(1),
        ),
    ] {
        write(text, modified);
        let _denied = crate::git_workspace::ReadDenied::new(&source);
        assert_eq!(disk_graph(&root, ""), None, "{text:?}");
    }
    std::fs::remove_dir_all(&root).unwrap();
}

/// A source rewritten at the same length with its modification time put back, which another
/// handle locks when the walk comes, keeps the summary of its last write while the lock holds, and
/// is read once the lock is let go: the summary kept for a source the walk could not read is not
/// cached under the source's new stamp.
#[cfg(windows)]
#[test]
fn a_rewrite_locked_when_the_walk_comes_is_read_once_let_go() {
    let root = temp_root("locked");
    write_file(&root, "package.json", NEXT_MANIFEST);
    write_file(&root, "app/page.tsx", "import './a';\nexport default 1;\n");
    write_file(&root, "app/b.ts", "");
    let source = root.join("app/a.ts");
    let settled = SystemTime::now() - Duration::from_secs(3600);
    // Each rewrite's change time is one the clock has left, as a later walk sees it.
    let rewrite = |directive: &str, changed: SystemTime| {
        std::fs::write(&source, format!("'{directive}';\nimport './b';\n")).unwrap();
        crate::git_workspace::backdate(&source, settled);
        crate::git_workspace::set_change_time(&source, changed);
    };
    let server = ["app/a.ts", "app/b.ts", "app/page.tsx"];
    rewrite("use server", settled);
    assert_eq!(disk_graph(&root, "").unwrap(), server);
    rewrite("use client", settled + Duration::from_secs(1));
    let lock = std::fs::File::open(&source).unwrap();
    lock.lock().unwrap();
    assert!(std::fs::read(&source).is_err(), "the lock denies the read");
    assert_eq!(disk_graph(&root, "").unwrap(), server);
    drop(lock);
    assert_eq!(disk_graph(&root, "").unwrap(), ["app/page.tsx"]);
    std::fs::remove_dir_all(&root).unwrap();
}

/// A source whose data a provider moved off this disk after the walk cached its summary (a cloud
/// file placeholder freed, a file moved to offline storage) keeps the summary of its last write
/// with no read: the attribute moves NTFS's change time, and a read would make the provider
/// download it. Once its data is back, the walk reads it once.
#[cfg(windows)]
#[test]
fn a_source_whose_data_is_offline_keeps_the_summary_of_its_last_write_with_no_read() {
    use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_OFFLINE};
    let root = temp_root("offline");
    write_file(&root, "package.json", NEXT_MANIFEST);
    let settled = SystemTime::now() - Duration::from_secs(3600);
    for (path, text) in [
        ("app/page.tsx", "import './a';\nexport default 1;\n"),
        ("app/a.ts", "'use server';\nimport './b';\n"),
        ("app/b.ts", ""),
    ] {
        write_file(&root, path, text);
        crate::git_workspace::backdate(&root.join(path), settled);
    }
    let source = root.join("app/a.ts");
    let server = ["app/a.ts", "app/b.ts", "app/page.tsx"];
    let parsed = |why: &str, sources: usize| {
        SUMMARIES.set(0);
        assert_eq!(disk_graph(&root, "").unwrap(), server, "{why}");
        assert_eq!(SUMMARIES.get(), sources, "{why}");
    };
    parsed("the first walk reads every source", 3);
    crate::git_workspace::set_attributes(&source, FILE_ATTRIBUTE_OFFLINE);
    parsed("the offline source is not read", 0);
    crate::git_workspace::set_attributes(&source, FILE_ATTRIBUTE_NORMAL);
    // The change time the clearing gave, as a later walk sees it: one the clock has left.
    crate::git_workspace::set_change_time(&source, settled + Duration::from_secs(60));
    parsed("the source back on disk is read once", 1);
    parsed("and then taken from the cache", 0);
    std::fs::remove_dir_all(&root).unwrap();
}

/// On unix a change to a source's permissions moves its status-change time as a write does, so no
/// cached summary stands for a source that can no longer be read: the package has no graph.
#[cfg(unix)]
#[test]
fn a_source_whose_read_is_denied_keeps_no_summary_on_unix() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let root = temp_root("denied");
    write_file(&root, "package.json", NEXT_MANIFEST);
    write_file(&root, "app/page.tsx", "import './a';\nexport default 1;\n");
    write_file(&root, "app/a.ts", "'use server';\nimport './b';\n");
    write_file(&root, "app/b.ts", "");
    let source = root.join("app/a.ts");
    std::fs::File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(3600))
        .unwrap();
    let server = ["app/a.ts", "app/b.ts", "app/page.tsx"];
    assert_eq!(disk_graph(&root, "").unwrap(), server);
    // The status-change time is as coarse as a scheduler tick, so a change of permissions within
    // the tick of the time set above leaves it: the change repeats until the time moved.
    let changed = || {
        let metadata = std::fs::metadata(&source).unwrap();
        (metadata.ctime(), metadata.ctime_nsec())
    };
    let cached = changed();
    (0..10_000)
        .find(|_| {
            std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o000)).unwrap();
            changed() != cached
        })
        .expect("a change of permissions moves the status-change time");
    // Permissions do not bind the superuser.
    if std::fs::read(&source).is_err() {
        assert_eq!(disk_graph(&root, ""), None);
    }
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
}

/// The walk lists a folder a page at a time as it takes the folder's entries: past its entry
/// bound it asks for no page after the one that holds the bound.
#[cfg(windows)]
#[test]
fn the_walk_reads_no_folder_page_past_its_entry_bound() {
    let root = temp_root("pages");
    // A page holds a few hundred entries of names this long.
    for number in 0..1_500 {
        write_file(&root, &format!("lib/{number:04}.ts"), "");
    }
    let pages = |bound: usize| {
        listing::PAGES_READ.set(0);
        let mut entries = bound;
        let listed = listed(&root.join("lib"), &mut entries).is_some();
        (listed, listing::PAGES_READ.get())
    };
    let (whole, all) = pages(MAX_GRAPH_ENTRIES);
    assert!(whole);
    assert!(all > 2, "the folder takes {all} pages");
    assert_eq!(pages(10), (false, 1));
    std::fs::remove_dir_all(&root).unwrap();
}

/// The walk follows no junction: one to a folder of the package would read its sources again
/// under another path, and one to a folder outside it sources the package does not hold.
#[cfg(windows)]
#[test]
fn the_walk_follows_no_junction() {
    let root = temp_root("junctions");
    let outside = temp_root("junctions-outside");
    write_file(&root, "package.json", NEXT_MANIFEST);
    write_file(&root, "app/page.tsx", "export default 1;\n");
    write_file(&root, "lib/a.ts", "");
    write_file(&outside, "b.ts", "");
    // Each link is named with the system's separator: `mklink` reads a `/` as one of its switches.
    for (link, target) in [("lib", root.join("lib")), ("outside", outside.clone())] {
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(root.join("app").join(link))
            .arg(target)
            .output()
            .unwrap();
        assert!(
            made.status.success(),
            "mklink /J app\\{link}: {}{}",
            String::from_utf8_lossy(&made.stdout),
            String::from_utf8_lossy(&made.stderr)
        );
    }
    let mut sources = Vec::new();
    let (mut budget, mut entries) = (MAX_GRAPH_BYTES, MAX_GRAPH_ENTRIES);
    assert!(walk(&root, &root, &mut sources, &mut budget, &mut entries));
    let walked = sources
        .into_iter()
        .map(|(relative, _, _)| relative)
        .collect::<Vec<_>>();
    assert_eq!(walked, ["app/page.tsx", "lib/a.ts"]);
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::remove_dir_all(&outside).unwrap();
}

/// An `extends` base already read, one past the chain's depth and an entry past the first
/// `MAX_EXTENDS_ENTRIES` of an array are not read.
#[test]
fn extends_reads_each_base_once_within_its_bounds() {
    let count = |files: &[(&str, String)]| {
        let reads = std::cell::Cell::new(0);
        let read = |path: &str| {
            reads.set(reads.get() + 1);
            files
                .iter()
                .find(|(entry, _)| *entry == path)
                .map(|(_, text)| text.clone())
        };
        let config = Config::read("", &read);
        (reads.get(), config.files().to_vec())
    };
    let base = r#"{ "compilerOptions": { "jsxImportSource": "preact" } }"#.to_owned();
    // A self-referencing array: the configuration is read once, and the base past the cap never.
    let selves = vec![r#""./tsconfig.json""#; 2_000].join(", ");
    let files = [
        (
            "tsconfig.json",
            format!(r#"{{ "extends": [{selves}, "./tsconfig.base.json"] }}"#),
        ),
        ("tsconfig.base.json", base.clone()),
    ];
    assert_eq!(count(&files), (1, vec!["tsconfig.json".to_owned()]));
    // Within the cap, a base named twice is read once.
    let files = [
        (
            "tsconfig.json",
            r#"{ "extends": ["./tsconfig.json", "./tsconfig.base.json", "./tsconfig.base"] }"#
                .to_owned(),
        ),
        ("tsconfig.base.json", base),
    ];
    assert_eq!(
        count(&files),
        (
            2,
            vec!["tsconfig.json".to_owned(), "tsconfig.base.json".to_owned()]
        )
    );
    // A chain longer than MAX_EXTENDS: the base past it is not read.
    let chain = (0..MAX_EXTENDS + 4)
        .map(|index| {
            let name = if index == 0 {
                "tsconfig.json".to_owned()
            } else {
                format!("tsconfig.{index}.json")
            };
            (
                name,
                format!(r#"{{ "extends": "./tsconfig.{}.json" }}"#, index + 1),
            )
        })
        .collect::<Vec<_>>();
    let chain = chain
        .iter()
        .map(|(name, text)| (name.as_str(), text.clone()))
        .collect::<Vec<_>>();
    let (reads, files) = count(&chain);
    assert_eq!((reads, files.len()), (MAX_EXTENDS + 1, MAX_EXTENDS + 1));
}

/// A configuration reached through a symlinked folder that leads out of the workspace is not
/// read; one inside it still is.
#[cfg(unix)]
#[test]
fn extends_through_a_symlinked_folder_reads_nothing_outside_the_workspace() {
    let base = r#"{ "compilerOptions": { "jsxImportSource": "preact" } }"#;
    let outside = temp_root("outside");
    write_file(&outside, "tsconfig.base.json", base);
    let root = temp_root("symlinked");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "extends": "./shared/tsconfig.base.json" }"#,
    );
    std::os::unix::fs::symlink(&outside, root.join("shared")).unwrap();
    let config = config_on_disk(&root, "");
    assert_eq!(config.jsx_import_source(), None);
    assert_eq!(config.files(), ["tsconfig.json"]);
    write_file(&root, "real/tsconfig.base.json", base);
    std::fs::remove_file(root.join("shared")).unwrap();
    std::os::unix::fs::symlink(root.join("real"), root.join("shared")).unwrap();
    assert_eq!(
        config_on_disk(&root, "").jsx_import_source(),
        Some("preact")
    );
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::remove_dir_all(&outside).unwrap();
}

/// A `tsconfig.json` that is a symlink is read where it resolves inside the workspace, not
/// replaced by the `jsconfig.json` beside it, and not where it resolves outside.
#[cfg(unix)]
#[test]
fn a_symlinked_configuration_is_read_only_inside_the_workspace() {
    let shared = r#"{ "compilerOptions": { "jsxImportSource": "preact" } }"#;
    let outside = temp_root("outside-shared");
    write_file(&outside, "tsconfig.shared.json", shared);
    let root = temp_root("symlinked-config");
    write_file(&root, "tsconfig.shared.json", shared);
    write_file(
        &root,
        "web/jsconfig.json",
        r#"{ "compilerOptions": { "jsxImportSource": "solid-js" } }"#,
    );
    std::os::unix::fs::symlink("../tsconfig.shared.json", root.join("web/tsconfig.json")).unwrap();
    let config = config_on_disk(&root, "web");
    assert_eq!(config.jsx_import_source(), Some("preact"));
    assert_eq!(config.files(), ["web/tsconfig.json"]);
    std::fs::remove_file(root.join("web/tsconfig.json")).unwrap();
    std::os::unix::fs::symlink(
        outside.join("tsconfig.shared.json"),
        root.join("web/tsconfig.json"),
    )
    .unwrap();
    assert_ne!(
        config_on_disk(&root, "web").jsx_import_source(),
        Some("preact")
    );
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::remove_dir_all(&outside).unwrap();
}
