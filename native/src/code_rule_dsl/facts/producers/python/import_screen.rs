//! A necessary-condition screen for explicit Python import/global qualification.
//!
//! Import syntax is observed during the caller's existing tape pass. Scope, order,
//! branches, rebinding and shadowing are deliberately ignored. This screen never
//! supplies a semantic verdict and must not screen binding/value demands.
use crate::code_rule_dsl::{
    identifier::{nfkc, python_identifier},
    facts::producers::python::python_facts::{Fact, qualification_chain},
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
        self.may_qualify_parts(chain.head,chain.attributes,wanted_imports,wanted_globals,module)
    }
    pub(crate) fn may_qualify_parts(&self,head:String,tail:Vec<String>,wanted_imports:&BTreeSet<String>,wanted_globals:&BTreeSet<String>,module:Option<&ModulePath>)->bool {
        if self.opaque { return true; }

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
pub mod tests;

/// The same conservative import screen over existing core facts. No tape restoration or scope table.
impl Imports {
    pub(crate) fn from_core(f: &crate::code_rule_dsl::facts::core::Facts, catalog: &crate::code_rule_dsl::language::KindCatalog, module: Option<&ModulePath>) -> Self {
        use crate::code_rule_dsl::facts::core::{kind_atom, kind_code, facts_tag};
        let kind = |n: u32| catalog.kind_name(kind_code(f.kind[n as usize])).unwrap_or("");
        let named = |n: u32| f.flags[n as usize] & 1 != 0 && !matches!(kind(n), "comment" | "line_continuation");
        let field = |n: u32, name: &str| catalog.field_id(name).and_then(|id| f.children(n).iter().copied().find(|&c| f.field[c as usize] == id));
        let ident = |n: u32| (kind(n)=="identifier").then(|| crate::code_rule_dsl::identifier::python_name(f.text(n).into()).into_owned());
        let dotted = |n: u32| -> Option<String> {
            if kind(n)!="dotted_name" { return None; }
            let parts=f.children(n).iter().copied().filter(|&c|named(c)).map(ident).collect::<Option<Vec<_>>>()?;
            (!parts.is_empty()).then(||parts.join("."))
        };
        let binding = |n:u32| -> Option<(String,String)> {
            match kind(n) {
                "dotted_name" => dotted(n).map(|s|(s.clone(),s)),
                "aliased_import" => Some((dotted(field(n,"name")?)?,ident(field(n,"alias")?)?)),
                _=>None,
            }
        };
        let mut out=Self::default();
        out.opaque=f.syntax_errors;
        for name in ["import_statement","import_from_statement","future_import_statement"] {
            let code=catalog.kind_id(name); if code==0 {continue;}
            for &n in f.of_kind(kind_atom(facts_tag(f),code)) {
                if name=="import_statement" {
                    for c in f.children(n).iter().copied().filter(|&c|named(c)) {
                        match kind(c) {
                            "dotted_name" => match dotted(c).and_then(|s|s.split('.').next().map(str::to_owned)) {
                                Some(head)=>out.add(head.clone(),Prefix::Qualified(head)), None=>out.opaque=true,
                            },
                            "aliased_import"=>match binding(c) {Some((q,a))=>out.add(a,Prefix::Qualified(q)),None=>out.opaque=true},
                            _=>out.opaque=true,
                        }
                    }
                    continue;
                }
                let module_node=field(n,"module_name");
                let prefix=match module_node {
                    Some(m) if kind(m)=="dotted_name"=>dotted(m).map(Prefix::Qualified),
                    Some(m) if kind(m)=="relative_import"=> {
                        let level=f.text(m).chars().take_while(|c|*c=='.'||c.is_whitespace()).filter(|c|*c=='.').count();
                        let rest=f.children(m).iter().copied().find(|&c|kind(c)=="dotted_name").map(dotted).unwrap_or(Some(String::new()));
                        rest.filter(|_|level!=0).map(|rest|Prefix::Relative{level,rest})
                    },
                    None=>Some(Prefix::Qualified("__future__".into())), _=>None,
                };
                let Some(prefix)=prefix else {out.opaque=true;continue;};
                for c in f.children(n).iter().copied().filter(|&c|named(c)&&Some(c)!=module_node) {
                    let Some((q,a))=binding(c) else {out.opaque=true;continue;};
                    let imported=match &prefix {
                        Prefix::Qualified(p)=>Prefix::Qualified(join(p,&[q])),
                        Prefix::Relative{level,rest}=> {
                            if module.is_none(){out.opaque=true;}
                            Prefix::Relative{level:*level,rest:if rest.is_empty(){q}else{format!("{rest}.{q}")}}
                        }
                    };
                    out.add(a,imported);
                }
            }
        }
        out
    }
}
