//! A necessary-condition screen for explicit Python import/global qualification.
//!
//! Import syntax is observed during the caller's existing tape pass. Scope, order,
//! branches, rebinding and shadowing are deliberately ignored. This screen never
//! supplies a semantic verdict and must not screen binding/value demands.
use crate::code_rule_dsl::{
    identifier::{nfkc, python_identifier},
    matcher::python_facts::{Fact, qualification_chain},
    module_path::ModulePath,
    tape::TapeDoc,
};
use ast_grep_core::{Doc, Node};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Prefix {
    Qualified(String),
    Relative { level: usize, rest: String },
}

#[derive(Default, Debug)]
pub(crate) struct Imports {
    bindings: BTreeMap<String, BTreeSet<Prefix>>,
    /// An import could not be understood, so qualification requests are admitted.
    pub opaque: bool,
}
impl Imports {
    /// Observe only the import node's subtree, never the enclosing document.
    /// Passing any other node is harmless and does not make the file opaque.
    pub(crate) fn observe(&mut self, node: &Node<'_, TapeDoc>, module: Option<&ModulePath>) {
        match node.kind().as_ref() {
            "import_statement" | "import_from_statement" | "future_import_statement" => {}
            _ => return,
        }
        if node.dfs().any(|n| n.is_error() || n.is_missing()) {
            self.opaque = true;
            return;
        }
        if node.kind() == "import_statement" {
            self.import(node);
        } else {
            self.import_from(node, module);
        }
    }

    /// False proves no requested explicit import/global can qualify this callee.
    /// True only requests normal semantic confirmation. Targets are unioned by the
    /// caller across predicates; no rule identity is involved. Prefix targets and
    /// custom typing-module aliases are conservatively overadmitted.
    pub(crate) fn may_qualify(
        &self,
        node: &Node<'_, TapeDoc>,
        wanted_imports: &BTreeSet<String>,
        wanted_globals: &BTreeSet<String>,
        module: Option<&ModulePath>,
    ) -> bool {
        if wanted_imports.is_empty() && wanted_globals.is_empty() {
            return false;
        }
        let chain = match qualification_chain(node) {
            Fact::NotApplicable(_) => return false,
            Fact::Unknown(_) | Fact::Unsupported(_) => return true,
            Fact::Known(chain) => chain,
        };
        if self.opaque {
            return true;
        }
        let (head, tail) = (chain.head, chain.attributes);
        if tail.is_empty()
            && (wanted_globals.contains(&head)
                || wanted_globals
                    .iter()
                    .any(|name| !name.is_ascii() && nfkc(name).as_ref() == head))
        {
            return true;
        }
        if wanted_imports.is_empty() {
            return false;
        }
        if let Some(prefixes) = self.bindings.get(&head) {
            for prefix in prefixes {
                let name = match prefix {
                    Prefix::Qualified(prefix) => Some(join(prefix, &tail)),
                    Prefix::Relative { level, rest } => {
                        module.and_then(|m| m.relative(*level, rest, &tail))
                    }
                };
                if name
                    .as_deref()
                    .is_some_and(|name| wanted(name, wanted_imports))
                {
                    return true;
                }
            }
        }
        // A def/class can be qualified by its file's module, including attributes.
        // No definition scan is necessary: checking every head only overadmits.
        module
            .and_then(|m| m.definition(&head, &tail))
            .as_deref()
            .is_some_and(|name| wanted(name, wanted_imports))
    }

    fn add(&mut self, head: String, prefix: Prefix) {
        self.bindings.entry(head).or_default().insert(prefix);
    }
    fn import(&mut self, node: &Node<'_, TapeDoc>) {
        for item in node.children().filter(named) {
            match item.kind().as_ref() {
                "dotted_name" => match dotted(&item) {
                    Some(_name) => {
                        // `import a.b` binds a to a, not to a.b.
                        let Some(head) = item
                            .children()
                            .filter(named)
                            .next()
                            .and_then(|head| identifier(&head))
                        else {
                            self.opaque = true;
                            continue;
                        };
                        self.add(head.clone(), Prefix::Qualified(head));
                    }
                    None => self.opaque = true,
                },
                "aliased_import" => {
                    let binding = item
                        .field("name")
                        .zip(item.field("alias"))
                        .and_then(|(name, alias)| dotted(&name).zip(identifier(&alias)));
                    if let Some((name, alias)) = binding {
                        self.add(alias, Prefix::Qualified(name));
                    } else {
                        self.opaque = true;
                    }
                }
                _ => self.opaque = true,
            }
        }
    }
    fn import_from(&mut self, node: &Node<'_, TapeDoc>, module: Option<&ModulePath>) {
        let module_node = node.field("module_name");
        let prefix = match &module_node {
            Some(node) if node.kind() == "dotted_name" => dotted(node).map(Prefix::Qualified),
            Some(node) if node.kind() == "relative_import" => {
                let text = node.text();
                let level = text
                    .chars()
                    .take_while(|c| *c == '.' || c.is_whitespace())
                    .filter(|c| *c == '.')
                    .count();
                let rest = node
                    .children()
                    .filter(named)
                    .find(|n| n.kind() == "dotted_name");
                let rest = match rest {
                    Some(n) => dotted(&n),
                    None => Some(String::new()),
                };
                rest.filter(|_| level != 0)
                    .map(|rest| Prefix::Relative { level, rest })
            }
            // Native scope parsing uses this spelling for future imports.
            None => Some(Prefix::Qualified("__future__".into())),
            _ => None,
        };
        let Some(prefix) = prefix else {
            self.opaque = true;
            return;
        };
        for item in node.children().filter(named) {
            if module_node
                .as_ref()
                .is_some_and(|m| m.node_id() == item.node_id())
            {
                continue;
            }
            let (name, alias) = match item.kind().as_ref() {
                "dotted_name" => match dotted(&item) {
                    Some(name) => (name.clone(), name),
                    None => {
                        self.opaque = true;
                        continue;
                    }
                },
                "aliased_import" => {
                    let binding = item
                        .field("name")
                        .zip(item.field("alias"))
                        .and_then(|(name, alias)| dotted(&name).zip(identifier(&alias)));
                    let Some(binding) = binding else {
                        self.opaque = true;
                        continue;
                    };
                    binding
                }
                // Native explicit-import resolution binds no ordinary names for *.
                // Conservatively admit because this syntax can evolve or be partial.
                "wildcard_import" => {
                    self.opaque = true;
                    continue;
                }
                _ => {
                    self.opaque = true;
                    continue;
                }
            };
            let imported = match &prefix {
                Prefix::Qualified(prefix) => Prefix::Qualified(join(prefix, &[name])),
                Prefix::Relative { level, rest } => {
                    let rest = if rest.is_empty() {
                        name
                    } else {
                        format!("{rest}.{name}")
                    };
                    // Retain the relative fact even if no module is available yet.
                    // The screen is not allowed to guess a different module context.
                    if module.is_none() {
                        self.opaque = true;
                    }
                    Prefix::Relative {
                        level: *level,
                        rest,
                    }
                }
            };
            self.add(alias, imported);
        }
    }
}
fn named<D: Doc>(node: &Node<'_, D>) -> bool {
    node.is_named() && !matches!(node.kind().as_ref(), "comment" | "line_continuation")
}
fn identifier<D: Doc>(node: &Node<'_, D>) -> Option<String> {
    (node.kind() == "identifier" && !node.is_error() && !node.is_missing())
        .then(|| python_identifier(node).into_owned())
}
fn dotted<D: Doc>(node: &Node<'_, D>) -> Option<String> {
    if node.kind() != "dotted_name" {
        return None;
    }
    let parts = node
        .children()
        .filter(named)
        .map(|n| identifier(&n))
        .collect::<Option<Vec<_>>>()?;
    (!parts.is_empty()).then(|| parts.join("."))
}
fn join(prefix: &str, tail: &[String]) -> String {
    let mut name = prefix.to_owned();
    for part in tail {
        name.push('.');
        name.push_str(part);
    }
    name
}
fn wanted(name: &str, imports: &BTreeSet<String>) -> bool {
    if imports.contains(name)
        || name
            .match_indices('.')
            .any(|(at, _)| imports.contains(&name[..at]))
    {
        return true;
    }
    imports.iter().any(|target| {
        let normalized = nfkc(target);
        (!target.is_ascii() && (normalized == name || name.strip_prefix(normalized.as_ref()).is_some_and(|rest| rest.starts_with('.'))))
            // PythonOptions can map custom typing modules to `typing.<member>`.
            // Its configuration is intentionally absent from this coarse screen.
            || normalized.strip_prefix("typing.").is_some_and(|member| name.rsplit('.').next() == Some(member))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::{
        language::GrammarDialect,
        matcher::python_facts::{self, Fact, Qualification},
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
}
