//! `py_binding(node, …)` (SCHEMA.md § 1.4): the bindings whose name node is a node of the
//! content, taken from the native engine's scope table (`scope/python.rs`, Ruff 0.16.9's semantic
//! model) by calling its builder, not by re-implementing it. Per content: the table reads one
//! file under the default reading (no `languageOptions`, not a stub), as native's `binds` reads it
//! for a Python query (`matcher/resolve.rs` `file_table`, `table_reading`).
//!
//! What `binds` reads of a binding at its name node (native `matcher/binds.rs` `NameSite::named`,
//! `BindingFilter::verdict`): its form, its scope's kind, its index among its scope's bindings of
//! the name (Ruff's order) and how many there are (`ScopeTable::chain`), and `outer`. Native finds
//! a node's bindings by its byte range and kind id (`Usage::named_at` over `name_sites`); the same
//! key maps each binding to its fact node here, once per content.
use super::facts;
use ast_grep_core::source::SgNode;
use ast_grep_core::AstGrep;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::scope::{Form, Reading, ScopeKind, ScopeTable};
use crate::code_rule_dsl::tape::TapeDoc;

/// One binding of the content, at its name node.
#[derive(Clone, Copy, Debug)]
pub struct Binding {
    /// The fact node of its name (the tape node with the name's range and kind id).
    pub node: u32,
    pub form: Form,
    pub scope_kind: ScopeKind,
    /// Its index among its scope's bindings of the name, and their number (deletions included).
    pub index: u32,
    pub chain_len: u32,
    /// Made under a `global` or `nonlocal` declaration.
    pub outer: bool,
}

/// The content's bindings that have a name node, sorted by node (stable within a node).
pub fn build(doc: &TapeDoc) -> Result<Vec<Binding>, String> {
    let grep = AstGrep::doc(doc.clone());
    let table = ScopeTable::build_in(grep.root(), GrammarDialect::Python, usize::MAX, &|| false, Reading::default(), None).map_err(|e| format!("scope table: {e:?}"))?;
    // Native's name kinds for Python (`ScopeRules::NAME_KINDS`, language/adapter/scope.rs): a node
    // of another kind names no binding for `binds`.
    let name_kinds = [facts::intern("identifier"), facts::intern("dotted_name")];
    let mut by_site: rustc_hash::FxHashMap<(u32, u32, u16), Vec<Binding>> = Default::default();
    for scope in &table.scopes {
        for bindings in scope.bindings.values() {
            for (index, b) in bindings.iter().enumerate() {
                let Some(name) = b.name else { continue };
                by_site.entry((name.start, name.end, name.kind)).or_default().push(Binding {
                    node: facts::NONE,
                    form: b.site.form,
                    scope_kind: scope.kind,
                    index: index as u32,
                    chain_len: bindings.len() as u32,
                    outer: b.outer,
                });
            }
        }
    }
    let mut out = Vec::new();
    if by_site.is_empty() {
        return Ok(out);
    }
    for id in 0..doc.transducer_len() as u32 {
        let span = doc.transducer_span(id);
        let Some(found) = by_site.get(&(span.start as u32, span.end as u32, doc.transducer_kind(id))) else { continue };
        if !name_kinds.contains(&facts::intern(&doc.node(id).kind())) {
            continue;
        }
        out.extend(found.iter().map(|b| Binding { node: id, ..*b }));
    }
    Ok(out)
}

/// The bindings named at `node`.
pub fn at(all: &[Binding], node: u32) -> &[Binding] {
    let a = all.partition_point(|b| b.node < node);
    let b = all.partition_point(|b| b.node <= node);
    &all[a..b]
}

/// Native's binding forms by filter name (`LEXICAL_FORMS`, language/adapter/scope.rs; Python's
/// `ScopeRules::FORMS`).
pub const FORMS: &[(&str, Form)] = &[
    ("assignment", Form::Assignment),
    ("annotated", Form::Annotated),
    ("annotation", Form::Annotation),
    ("augmented", Form::Augmented),
    ("named", Form::Named),
    ("for", Form::For),
    ("with", Form::With),
    ("except", Form::Except),
    ("import", Form::Import),
    ("definition", Form::Definition),
    ("parameter", Form::Parameter),
    ("global", Form::Global),
    ("match", Form::Match),
    ("typeParameter", Form::TypeParameter),
    ("typeAlias", Form::TypeAlias),
    ("implicit", Form::Implicit),
    ("deletion", Form::Deletion),
    ("field", Form::Field),
    ("method", Form::Method),
    ("accessor", Form::Accessor),
];

/// Python's scope names (`ScopeRules::SCOPES`).
pub const SCOPES: &[(&str, ScopeKind)] = &[
    ("module", ScopeKind::Module),
    ("class", ScopeKind::Class),
    ("function", ScopeKind::Function),
    ("lambda", ScopeKind::Lambda),
    ("comprehension", ScopeKind::Comprehension),
    ("typeParameters", ScopeKind::TypeParams),
];

/// A `binds` test without usage facts or sub-rules (native `Binds` with `declaration_only` and
/// no site/value/annotation/occurrence/redefines): its filters only, so its verdict at a node is
/// two-valued and binds nothing.
#[derive(Clone, Debug)]
pub struct Filter {
    pub forms: Option<Vec<Form>>,
    pub scopes: Option<Vec<ScopeKind>>,
    pub latest: Option<bool>,
    pub outer: Option<bool>,
    /// `bindings` without `within`: (min, max).
    pub bindings: Option<(u32, Option<u32>)>,
}

impl Filter {
    /// Native `BindingFilter::verdict` for Python without usage: every condition is known.
    pub fn passes(&self, b: &Binding) -> bool {
        self.forms.as_ref().is_none_or(|f| f.contains(&b.form))
            && self.scopes.as_ref().is_none_or(|s| s.contains(&b.scope_kind))
            && self.latest.is_none_or(|w| w == (b.index + 1 == b.chain_len))
            && self.outer.is_none_or(|w| w == b.outer)
            && self.bindings.is_none_or(|(min, max)| min <= b.chain_len && max.is_none_or(|m| b.chain_len <= m))
    }

    /// Compiles a `binds` object (native compile/binds.rs `validate_binds`, `compile_filter`);
    /// the keys that read the usage index, a capture's region or a sub-rule are refused by name.
    pub fn compile(v: &serde_json::Value, at: &str) -> Result<Filter, String> {
        use serde_json::Value as Json;
        let o = v.as_object().ok_or_else(|| format!("{at} must be an object"))?;
        for k in o.keys() {
            match k.as_str() {
                "form" | "scope" | "latest" | "outer" | "bindings" => {}
                "used" | "referenced" | "nestedUse" | "exported" => return Err(format!("{at}/{k}: binds usage filter unsupported (needs the usage index)")),
                "site" | "value" | "annotation" | "occurrence" | "redefines" => return Err(format!("{at}/{k}: binds sub-rule unsupported")),
                // Native refuses both for Python (`ScopeRules::REFUSED_FILTERS`).
                "builtin" | "directives" => return Err(format!("{at}/{k}: not read for python")),
                _ => return Err(format!("unknown field `{k}` at {at}")),
            }
        }
        fn names<T: Copy>(v: &Json, at: &str, known: &[(&str, T)], what: &str) -> Result<Vec<T>, String> {
            let items = match v {
                Json::String(_) => std::slice::from_ref(v),
                Json::Array(a) if !a.is_empty() => a.as_slice(),
                _ => return Err(format!("{at} must be a {what} name or a non-empty array of them")),
            };
            items
                .iter()
                .map(|i| {
                    let n = i.as_str().unwrap_or_default();
                    known.iter().find(|(k, _)| *k == n).map(|(_, x)| *x).ok_or_else(|| format!("{at}: `{i}` is not a python {what}"))
                })
                .collect()
        }
        let boolean = |k: &str| -> Result<Option<bool>, String> { o.get(k).map(|v| v.as_bool().ok_or_else(|| format!("{at}/{k} must be true or false"))).transpose() };
        let bindings = match o.get("bindings") {
            None => None,
            Some(b) => {
                let at = format!("{at}/bindings");
                let b = b.as_object().ok_or_else(|| format!("{at} must be an object"))?;
                if let Some(k) = b.keys().find(|k| !["min", "max", "within"].contains(&k.as_str())) {
                    return Err(format!("unknown field `{k}` at {at}"));
                }
                if b.contains_key("within") {
                    return Err(format!("{at}/within: binds bindings.within unsupported (reads a capture's region)"));
                }
                let bound = |k: &str| -> Result<Option<u32>, String> { b.get(k).map(|v| v.as_u64().and_then(|x| u32::try_from(x).ok()).ok_or_else(|| format!("{at}/{k} must be a non-negative integer"))).transpose() };
                let (min, max) = (bound("min")?, bound("max")?);
                if min.unwrap_or(0) == 0 && max.is_none() {
                    return Err(format!("{at} must set min to at least 1, or set max; otherwise every binding passes"));
                }
                let min = min.unwrap_or(0);
                if max.is_some_and(|m| m < min) {
                    return Err(format!("{at}: min must not exceed max"));
                }
                Some((min, max))
            }
        };
        Ok(Filter {
            forms: o.get("form").map(|f| names(f, &format!("{at}/form"), FORMS, "binding form")).transpose()?,
            scopes: o.get("scope").map(|s| names(s, &format!("{at}/scope"), SCOPES, "scope kind")).transpose()?,
            latest: boolean("latest")?,
            outer: boolean("outer")?,
            bindings,
        })
    }
}
