    use super::*;
    use crate::code_rule_dsl::{
        language::GrammarDialect,
        facts::producers::python::python_facts::{self, Fact, Qualification},
        scope::{
            Reading, ScopeTable,
            options::{DEFAULT_PYTHON_MINOR, PythonOptions},
        },
        tape::{Grammar, TapeImage},
    };
    use ast_grep_core::{AstGrep, tree_sitter::StrDoc};
    use std::sync::Arc;

    fn tape(source: &str) -> AstGrep<TapeDoc> {
        let grammar = Grammar::new(GrammarDialect::Python).unwrap();
        let parsed = StrDoc::new(source, GrammarDialect::Python);
        let image =
            TapeImage::encode(parsed.tree.root_node(), &grammar, source, &|| false).unwrap();
        AstGrep::doc(TapeDoc::restore(Arc::new(source.into()), grammar, &image, &|| false).unwrap())
    }
    fn imports(root: &AstGrep<TapeDoc>, module: Option<&ModulePath>) -> Imports {
        let mut imports = Imports::default();
        // Production supplies these nodes from its existing tape pass.
        for node in root.root().dfs() {
            if matches!(
                node.kind().as_ref(),
                "import_statement" | "import_from_statement" | "future_import_statement"
            ) {
                imports.observe(&node, module);
            }
        }
        let facts=crate::code_rule_dsl::facts::core::from_tape(root.root().get_doc(),Arc::new(crate::code_rule_dsl::lines::LineTable::new(root.root().get_doc().file_source(),crate::code_rule_dsl::lines::Terminators::of(GrammarDialect::Python)))).unwrap();
        let core=Imports::from_core(&facts,&GrammarDialect::Python.kinds(),module);
        assert_eq!(core.bindings,imports.bindings,"core/tape import facts differ");
        assert_eq!(core.opaque,imports.opaque,"core/tape uncertainty differs");
        imports
    }
    fn assert_necessary(
        source: &str,
        module: Option<&ModulePath>,
        wanted: &[&str],
        globals: &[&str],
    ) {
        let root = tape(source);
        let imports = imports(&root, module);
        if !source.contains("import *") {
            assert!(
                !imports.opaque,
                "recognized import syntax must not disable the screen"
            );
        }
        let table = ScopeTable::build_in(
            root.root(),
            GrammarDialect::Python,
            usize::MAX,
            &|| false,
            Reading::default(),
            module,
        )
        .unwrap();
        let python = PythonOptions::new(DEFAULT_PYTHON_MINOR);
        let callees: Vec<_> = root
            .root()
            .dfs()
            .filter(|n| n.kind() == "call")
            .filter_map(|n| n.field("function"))
            .collect();
        let facts = python_facts::project(
            &table,
            python_facts::Config {
                module,
                python: &python,
            },
            callees.clone(),
        )
        .unwrap();
        let by_id: BTreeMap<_, _> = facts
            .by_range
            .into_values()
            .flatten()
            .map(|f| (f.node_id, f))
            .collect();
        let wanted: BTreeSet<String> = wanted.iter().map(|s| (*s).into()).collect();
        let globals: BTreeSet<String> = globals.iter().map(|s| (*s).into()).collect();
        let mut confirmed = 0;
        for node in callees {
            let row = &by_id[&node.node_id()];
            let matches = match &row.qualification {
                Fact::Known(Qualification::Qualified { name, .. }) => wanted.contains(name),
                Fact::Known(Qualification::Unshadowed { name, .. }) => {
                    row.attributes.is_empty() && globals.contains(name)
                }
                _ => false,
            };
            if matches {
                confirmed += 1;
                assert!(
                    imports.may_qualify(&node, &wanted, &globals, module),
                    "screen excluded native fact for {}",
                    node.text()
                );
            }
        }
        assert!(confirmed > 0, "fixture must exercise native positive facts");
    }

    #[test]
    fn every_matching_native_alias_submodule_relative_global_and_definition_is_admitted() {
        let module = ModulePath::dotted("pkg.worker").unwrap();
        let source = "import os.path\nimport pathlib as paths\nfrom subprocess import run as launch\nfrom .helpers import wait as pause\nimport pkg as K\nfrom __future__ import annotations\nos.path.join('a','b')\n(paths.Path)('f')\nlaunch([])\npause()\nK.go()\nlen([])\ndef local(): pass\nlocal()\ndef hidden(launch):\n    launch([])\ndef nested():\n    import httpx as h\n    h.get('url')\n";
        assert_necessary(
            source,
            Some(&module),
            &[
                "os.path.join",
                "pathlib.Path",
                "subprocess.run",
                "pkg.helpers.wait",
                "pkg.go",
                "pkg.worker.local",
                "httpx.get",
            ],
            &["len"],
        );
    }

    #[test]
    fn negative_screen_and_deduplicated_prefixes_do_not_resolve_shadowing_or_order() {
        let source = "import requests as r\nimport requests as r\nr = object()\nr.get('url')\nunrelated.run()\nlate()\nfrom subprocess import run as late\n";
        let root = tape(source);
        let imports = imports(&root, None);
        assert!(!imports.opaque);
        assert_eq!(imports.bindings["r"].len(), 1);
        let wanted = BTreeSet::from(["requests.get".into(), "subprocess.run".into()]);
        let globals = BTreeSet::new();
        let node = |text: &str| {
            root.root()
                .dfs()
                .find(|n| {
                    n.kind() == "call" && n.field("function").is_some_and(|f| f.text() == text)
                })
                .unwrap()
                .field("function")
                .unwrap()
        };
        assert!(
            imports.may_qualify(&node("r.get"), &wanted, &globals, None),
            "shadowing is deliberately overadmitted"
        );
        assert!(
            imports.may_qualify(&node("late"), &wanted, &globals, None),
            "source order is deliberately overadmitted"
        );
        assert!(!imports.may_qualify(&node("unrelated.run"), &wanted, &globals, None));
        assert_necessary("import pkg as x\n(x.K)()\n", None, &["pkg.K"], &[]);
    }

    #[test]
    fn impossible_primary_qualifications_do_not_demand_scopes_or_remove_receiver_shapes() {
        let root = tape("from pathlib import Path\nPath('f').open()\nhandlers['open']()\n");
        let imports = imports(&root, None);
        let wanted = BTreeSet::from(["pathlib.Path".into()]);
        let globals = BTreeSet::from(["open".into()]);
        let callees: Vec<_> = root
            .root()
            .dfs()
            .filter(|n| n.kind() == "call")
            .filter_map(|n| n.field("function"))
            .collect();
        let path_method = callees
            .iter()
            .find(|n| n.text() == "Path('f').open")
            .unwrap();
        let subscript = callees
            .iter()
            .find(|n| n.text() == "handlers['open']")
            .unwrap();
        assert!(!imports.may_qualify(path_method, &wanted, &globals, None));
        assert!(!imports.may_qualify(subscript, &wanted, &globals, None));
        let constructor_callee = callees.iter().find(|n| n.text() == "Path").unwrap();
        assert!(imports.may_qualify(constructor_callee, &wanted, &globals, None));
        // Even an opaque import cannot turn a valid non-name head into a name.
        let opaque = Imports {
            opaque: true,
            ..Default::default()
        };
        assert!(!opaque.may_qualify(path_method, &wanted, &globals, None));
        let malformed = tape("broken = (\n");
        assert!(imports.may_qualify(&malformed.root(), &wanted, &globals, None));
        // The receiver call still exists in the tree. Binding/value consumers must
        // retain it independently of this qualification-only primary filter.
        assert_eq!(path_method.field("object").unwrap().kind(), "call");
    }

    #[test]
    fn stars_missing_module_and_unsupported_callees_stay_conservative() {
        let source = "from pkg import *\nimport requests as r\nr.get('url')\n";
        assert_necessary(source, None, &["requests.get"], &[]);
        let root = tape(source);
        assert!(imports(&root, None).opaque);
        let root = tape("from .helpers import wait as w\nw()\n");
        assert!(imports(&root, None).opaque);
        let root = tape("factory().open()\n");
        let callee = root.root().dfs().find(|n| n.kind() == "attribute").unwrap();
        assert!(!Imports::default().may_qualify(
            &callee,
            &BTreeSet::from(["pathlib.Path.open".into()]),
            &BTreeSet::new(),
            None
        ));
        let root = tape("import pkg as p\np.member.extra()\n");
        let imports = imports(&root, None);
        let callee = root
            .root()
            .dfs()
            .find(|n| n.kind() == "attribute" && n.text() == "p.member.extra")
            .unwrap();
        assert!(
            imports.may_qualify(
                &callee,
                &BTreeSet::from(["pkg.member".into()]),
                &BTreeSet::new(),
                None
            ),
            "native prefix options are overadmitted"
        );
    }
