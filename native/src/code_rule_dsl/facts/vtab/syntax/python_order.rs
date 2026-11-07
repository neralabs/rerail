//! CPython field order over the existing tape projection. Geometry and node ids stay unchanged.
//! Only a demanded visit column builds this dense rank array, once per document. This is one
//! iterative tree pass, with temporary child overrides for fields whose source order differs.
use super::{Dialect, NONE, Projected};
use std::collections::HashMap;

struct Walker<'a> {
    p: &'a Projected,
    dialect: &'a Dialect,
    overrides: HashMap<u32, Vec<u32>>,
}
impl Walker<'_> {
    fn kind(&self, n: u32) -> &str {
        self.dialect.name(&self.p.facts, self.p.core[n as usize])
    }
    fn field(&self, n: u32, name: &str) -> Option<u32> {
        let core = self
            .dialect
            .field_child(&self.p.facts, self.p.core[n as usize], name)?;
        let local = self.p.local[core as usize];
        (local != NONE).then_some(local)
    }
    fn children(&self, n: u32) -> Vec<u32> {
        self.p.children(n).collect()
    }
    fn decorators(&self, n: u32) -> Vec<u32> {
        let parent = self.p.parent[n as usize];
        if parent == NONE || self.kind(parent) != "decorated_definition" {
            return Vec::new();
        }
        self.p
            .children(parent)
            .filter(|&c| self.kind(c) == "decorator")
            .collect()
    }
    fn definition(&self, n: u32, class: bool) -> Vec<u32> {
        let fields: &[&str] = if class {
            &["name", "superclasses", "body", "type_parameters"]
        } else {
            &[
                "name",
                "parameters",
                "body",
                "return_type",
                "type_parameters",
            ]
        };
        let mut out = Vec::new();
        // Comments and other CST-only nodes are retained, but cannot move semantic expressions.
        for c in self.p.children(n) {
            if !fields.iter().any(|name| self.field(n, name) == Some(c)) {
                out.push(c);
            }
        }
        for name in fields {
            if *name == "return_type" || class && *name == "type_parameters" {
                out.extend(self.decorators(n));
            }
            if let Some(child) = self.field(n, name) {
                out.push(child);
            }
        }
        out
    }
    fn parameters(&mut self, n: u32) -> Vec<u32> {
        let (mut positional, mut vararg, mut keyword, mut kwarg) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let (mut defaults, mut kw_defaults, mut other) = (Vec::new(), Vec::new(), Vec::new());
        let mut keyword_only = false;
        for param in self.children(n) {
            let kind = self.kind(param);
            if kind == "keyword_separator" {
                keyword_only = true;
                other.push(param);
                continue;
            }
            if matches!(kind, "positional_separator" | "comment") {
                other.push(param);
                continue;
            }
            let star = match kind {
                "list_splat_pattern" => 1,
                "dictionary_splat_pattern" => 2,
                "typed_parameter" => self
                    .p
                    .children(param)
                    .find_map(|c| match self.kind(c) {
                        "list_splat_pattern" => Some(1),
                        "dictionary_splat_pattern" => Some(2),
                        _ => None,
                    })
                    .unwrap_or(0),
                _ => 0,
            };
            if let Some(value) = self.field(param, "value") {
                // ast.arg owns the annotation; arguments owns defaults, in separate lists.
                let header = self.p.children(param).filter(|&c| c != value).collect();
                self.overrides.insert(param, header);
                if keyword_only {
                    kw_defaults.push(value)
                } else {
                    defaults.push(value)
                }
            }
            match star {
                1 => {
                    vararg.push(param);
                    keyword_only = true;
                }
                2 => kwarg.push(param),
                _ if keyword_only => keyword.push(param),
                _ => positional.push(param),
            }
        }
        other.extend(positional);
        other.extend(vararg);
        other.extend(keyword);
        other.extend(kw_defaults);
        other.extend(kwarg);
        other.extend(defaults);
        other
    }
    fn dictionary(&mut self, n: u32) -> Vec<u32> {
        let (mut keys, mut values) = (Vec::new(), Vec::new());
        for child in self.children(n) {
            if self.kind(child) == "pair" {
                if let Some(value) = self.field(child, "value") {
                    let key = self.p.children(child).filter(|&c| c != value).collect();
                    self.overrides.insert(child, key);
                    values.push(value);
                }
                keys.push(child);
            } else if self.kind(child) == "dictionary_splat" {
                values.push(child);
            } else {
                keys.push(child);
            }
        }
        keys.extend(values);
        keys
    }
    fn changed_children(&mut self, n: u32) -> Option<Vec<u32>> {
        if let Some(children) = self.overrides.remove(&n) {
            return Some(children);
        }
        match self.kind(n) {
            "decorated_definition"
                if self.field(n, "definition").is_some_and(|c| {
                    matches!(self.kind(c), "function_definition" | "class_definition")
                }) =>
            {
                Some(
                    self.p
                        .children(n)
                        .filter(|&c| self.kind(c) != "decorator")
                        .collect(),
                )
            }
            "function_definition" => Some(self.definition(n, false)),
            "class_definition" => Some(self.definition(n, true)),
            "parameters" | "lambda_parameters" => Some(self.parameters(n)),
            "dictionary" => Some(self.dictionary(n)),
            "argument_list" => {
                let children = self.children(n);
                let is_keyword =
                    |c: u32| matches!(self.kind(c), "keyword_argument" | "dictionary_splat");
                Some(
                    children
                        .iter()
                        .copied()
                        .filter(|&c| !is_keyword(c))
                        .chain(children.iter().copied().filter(|&c| is_keyword(c)))
                        .collect(),
                )
            }
            _ => None,
        }
    }
}

pub(super) fn ranks(p: &Projected, dialect: &Dialect) -> Vec<u32> {
    let _profile = crate::code_rule_dsl::logic::profile::span(|| "Python AST visit ranks".into());
    let mut walker = Walker {
        p,
        dialect,
        overrides: HashMap::new(),
    };
    let mut ranks = vec![NONE; p.len() as usize];
    let mut stack = if p.len() == 0 { Vec::new() } else { vec![0] };
    let mut next = 0;
    while let Some(n) = stack.pop() {
        assert_eq!(
            ranks[n as usize], NONE,
            "Python visit projection reached a node twice"
        );
        ranks[n as usize] = next;
        next += 1;
        if let Some(children) = walker.changed_children(n) {
            stack.extend(children.into_iter().rev());
        } else {
            // Most nodes need no temporary vector or hash lookup insertion.
            for &core in p.facts.children(p.core[n as usize]).iter().rev() {
                let local = p.local[core as usize];
                if local != NONE {
                    stack.push(local);
                }
            }
        }
    }
    assert_eq!(next, p.len(), "Python visit projection omitted nodes");
    ranks
}
