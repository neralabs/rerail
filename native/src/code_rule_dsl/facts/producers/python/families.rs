//! The `py.*` fact families (list step 25, decision 0029): a document's call facts, read from its
//! core facts (`facts::core`, the tape's columns) starting at the kind postings of `call`, not by
//! scanning the tape. The rows are rule-independent and keyed by node, as t11's targets
//! (`notes/t11-streams-since.md`, `py.call`, `py.call_site`, `py.exec_context`, `py.call_arg`):
//!
//! ```text
//! py_call(d, call, callee)                       every call with a `function` field
//! py_call_site(d, report, call, ref)             `report` the callee less transparent parens; `ref` it
//!                                                where it is a name or attribute, else -1
//! py_exec_context(d, call, owner, kind)          the call's nearest execution boundary: the function,
//!                                                class or lambda whose body holds it (-1: the module)
//! py_call_arg(d, call, arg, ordinal, keyword, spread, value)
//!                                                each named argument; `keyword` its name node or -1
//! py_attribute(d, node, object, name)            each attribute callee's object and name nodes
//! ```
//!
//! `kind` and `spread` are name atoms ([`CONTEXTS`], [`SPREADS`]). The semantics are
//! `call_batch::collect` and `collect_arguments`' (S's producers, kept as the oracle): the tests
//! check these rows equal theirs. Rows are in core ids here; `facts::vtab::syntax` maps them to the
//! wave's node ids and serves them.

use crate::code_rule_dsl::facts::core::{Facts, NONE, kind_atom};
use crate::code_rule_dsl::language::KindCatalog;

/// The rows' version: a change to what they hold changes the families' producer (`families::py_producer`).
pub(crate) const VERSION: u32 = 3;

pub(crate) use crate::code_rule_dsl::logic::front::PYTHON_CONTEXTS as CONTEXTS;
/// `py_call_arg.spread`'s names: none, `*x`, `**x`.
pub(crate) const SPREADS: [&str; 3] = ["none", "positional", "keywords"];

/// What a kind code is to the producer, by name (a grammar gives some names several codes).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Other,
    Call,
    Function,
    Class,
    Lambda,
    Async,
    Name,
    Attribute,
    Parens,
    Generator,
    Keyword,
    Splat,
    DictionarySplat,
    Comment,
    Continuation,
}

/// The dialect's roles and fields the producer reads.
pub(crate) struct Grammar {
    tag: u8,
    roles: Vec<Role>,
    calls: Vec<u32>,
    function: u16,
    body: u16,
    object: u16,
    attribute: u16,
    arguments: u16,
    name: u16,
    value: u16,
}

impl Grammar {
    pub(crate) fn new(tag: u8, catalog: &KindCatalog) -> Self {
        let roles: Vec<Role> = (0..catalog.kind_count() as u16)
            .map(|code| match (catalog.kind_name(code), catalog.kind_is_named(code)) {
                (Some("call"), true) => Role::Call,
                (Some("function_definition"), true) => Role::Function,
                (Some("class_definition"), true) => Role::Class,
                (Some("lambda"), true) => Role::Lambda,
                (Some("async"), false) => Role::Async,
                (Some("identifier"), true) => Role::Name,
                (Some("attribute"), true) => Role::Attribute,
                (Some("parenthesized_expression"), true) => Role::Parens,
                (Some("generator_expression"), true) => Role::Generator,
                (Some("keyword_argument"), true) => Role::Keyword,
                (Some("list_splat" | "parenthesized_list_splat"), true) => Role::Splat,
                (Some("dictionary_splat"), true) => Role::DictionarySplat,
                (Some("comment"), _) => Role::Comment,
                (Some("line_continuation"), _) => Role::Continuation,
                _ => Role::Other,
            })
            .collect();
        let calls = roles.iter().enumerate().filter(|(_, r)| **r == Role::Call).map(|(code, _)| kind_atom(tag, code as u16)).collect();
        let field = |name: &str| catalog.field_id(name).unwrap_or(0);
        Self {
            tag,
            roles,
            calls,
            function: field("function"),
            body: field("body"),
            object: field("object"),
            attribute: field("attribute"),
            arguments: field("arguments"),
            name: field("name"),
            value: field("value"),
        }
    }
    pub(crate) fn tag(&self) -> u8 {
        self.tag
    }
    fn role(&self, f: &Facts, id: u32) -> Role {
        self.atom_role(f.kind[id as usize])
    }
    fn atom_role(&self, atom: u32) -> Role {
        self.roles.get(usize::from(crate::code_rule_dsl::facts::core::kind_code(atom))).copied().unwrap_or(Role::Other)
    }
}

/// One document's rows, in core ids (`NONE`: none). `kind` and `spread` index [`CONTEXTS`] and [`SPREADS`].
#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct Rows {
    pub call: Vec<[u32; 2]>,
    pub site: Vec<[u32; 3]>,
    pub context: Vec<[u32; 3]>,
    pub arg: Vec<[u32; 6]>,
    pub attribute: Vec<[u32; 3]>,
}

/// The first child of `id` under `field` (the tape's `child_by_field_id`).
fn field(f: &Facts, id: u32, field: u16) -> Option<u32> {
    (field != 0).then(|| f.children(id).iter().copied().find(|&c| f.field[c as usize] == field)).flatten()
}

/// `node` less transparent parens: a parenthesized expression's first named child that is not a comment
/// or line continuation, repeatedly.
pub(crate) fn unparen(g: &Grammar, f: &Facts, mut node: u32) -> u32 {
    while g.role(f, node) == Role::Parens {
        match f.children(node).iter().copied().find(|&c| f.named(c) && !matches!(g.role(f, c), Role::Comment | Role::Continuation)) {
            Some(child) => node = child,
            None => break,
        }
    }
    node
}

/// A named node that is not a comment (`call_batch::named`).
fn named(g: &Grammar, f: &Facts, id: u32) -> bool {
    f.named(id) && g.role(f, id) != Role::Comment
}

/// The nearest execution boundary of `call`: the innermost function, class or lambda whose body holds
/// it, and its context kind (an index of [`CONTEXTS`]); `(NONE, 0)` the module.
fn context(g: &Grammar, f: &Facts, call: u32) -> (u32, u32) {
    let mut at = f.parent(call);
    while let Some(owner) = at {
        let role = g.role(f, owner);
        if matches!(role, Role::Function | Role::Class | Role::Lambda) {
            if let Some(body) = field(f, owner, g.body) {
                if body <= call && call < f.subtree_end[body as usize] {
                    let kind = match role {
                        Role::Class => 3,
                        Role::Lambda => 4,
                        _ if f.tokens(owner).iter().any(|t| g.atom_role(t.kind) == Role::Async) => 2,
                        _ => 1,
                    };
                    return (owner, kind);
                }
            }
        }
        at = f.parent(owner);
    }
    (NONE, 0)
}

/// The rows of one document's facts, from its calls' postings.
pub(crate) fn rows(g: &Grammar, f: &Facts) -> Rows { collect_rows(g,f,true) }
pub(crate) fn structure(g: &Grammar, f: &Facts) -> Rows { collect_rows(g,f,false) }
fn collect_rows(g: &Grammar, f: &Facts, with_arguments: bool) -> Rows {
    let mut calls: Vec<u32> = g.calls.iter().flat_map(|&atom| f.of_kind(atom).iter().copied()).collect();
    calls.sort_unstable();
    let mut out = Rows::default();
    for call in calls {
        let Some(callee) = field(f, call, g.function) else { continue };
        let (owner, kind) = context(g, f, call);
        let report = unparen(g, f, callee);
        let reference = matches!(g.role(f, report), Role::Name | Role::Attribute).then_some(report).unwrap_or(NONE);
        out.call.push([call, callee]);
        out.site.push([report, call, reference]);
        out.context.push([call, owner, kind]);
        if g.role(f, report) == Role::Attribute {
            if let (Some(object), Some(name)) = (field(f, report, g.object), field(f, report, g.attribute)) {
                out.attribute.push([report, object, name]);
            }
        }
        if with_arguments { append_arguments(g,f,call,&mut out); }
    }
    out
}

/// The digest of a document's rows. Rows hold no byte positions, so an edit that adds or removes no node
/// (whitespace, a comment's or a string's text) keeps it, and its readers short-circuit.
pub(crate) fn digest(rows: &Rows) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"rerail.py-rows/v1\0");
    let mut family = |cells: &[u32], width: usize| {
        hash.update(((cells.len() / width) as u64).to_le_bytes());
        cells.iter().for_each(|c| hash.update(c.to_le_bytes()));
    };
    family(rows.call.as_flattened(), 2);
    family(rows.site.as_flattened(), 3);
    family(rows.context.as_flattened(), 3);
    family(rows.arg.as_flattened(), 6);
    family(rows.attribute.as_flattened(), 3);
    hash.finalize().into()
}

#[cfg(test)]
mod tests;

/// Extract only calls requested by the consumer; no all-call scan or context calculation.
pub(crate) fn arguments(g:&Grammar,f:&Facts,calls:&[u32])->Vec<[u32;6]> {
    let mut out=Rows::default();
    for &call in calls { if g.role(f,call)==Role::Call { append_arguments(g,f,call,&mut out); } }
    out.arg
}
fn append_arguments(g:&Grammar,f:&Facts,call:u32,out:&mut Rows) {
        let Some(arguments) = field(f, call, g.arguments) else { return };
        if g.role(f, arguments) == Role::Generator {
            out.arg.push([call, arguments, 0, NONE, 0, arguments]);
            return;
        }
        let listed = f.children(arguments).iter().copied().filter(|&a| named(g, f, a));
        for (ordinal, argument) in listed.enumerate() {
            let role = g.role(f, argument);
            let spread = match role {
                Role::Splat => 1,
                Role::DictionarySplat => 2,
                _ => 0,
            };
            let (keyword, value) = if role == Role::Keyword {
                (field(f, argument, g.name).unwrap_or(NONE), field(f, argument, g.value).unwrap_or(argument))
            } else if spread != 0 {
                (NONE, f.children(argument).iter().copied().find(|&c| named(g, f, c)).unwrap_or(argument))
            } else {
                (NONE, argument)
            };
            out.arg.push([call, argument, ordinal as u32, keyword, spread, value]);
        }
}

pub(crate) fn qualification_chain(g:&Grammar,f:&Facts,n:u32)->Option<(String,Vec<String>)> {
    let mut n=unparen(g,f,n); let mut tail=Vec::new();
    while g.role(f,n)==Role::Attribute {
        let a=field(f,n,g.attribute)?;
        tail.push(crate::code_rule_dsl::identifier::python_name(f.text(a).into()).into_owned());
        n=unparen(g,f,field(f,n,g.object)?);
    }
    if g.role(f,n)!=Role::Name {return None;}
    tail.reverse();
    Some((crate::code_rule_dsl::identifier::python_name(f.text(n).into()).into_owned(),tail))
}

pub(crate) mod candidates;
#[cfg(test)]
pub(crate) use candidates as candidate_spike;
