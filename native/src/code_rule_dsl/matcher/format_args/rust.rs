//! `formatArgs` for Rust: a `format_args!`-family macro call read from tree-sitter-rust's token
//! tree, as rustc 1.99.0 splits its arguments. The macro's arguments are tokens, not expressions:
//! the call is split at the token tree's top-level commas, so a comma inside a turbofish
//! (`HashMap::<K, V>::new()`) or after `as` (`x as Foo<A, B>`) is kept by tracking the angle
//! brackets that open there; any other bare angle brackets, and a closure's parameter list
//! written as a whole argument (`|a, b| ..`), are not told from comparisons and parameters.

use crate::code_rule_dsl::value::rust::{RustValue, literal};
use ast_grep_core::{Doc, Node};

/// How many arguments a macro takes before its format string, by name.
fn leading_arguments(name: &str) -> Option<usize> {
    match name {
        "format" | "print" | "println" | "eprint" | "eprintln" | "panic" | "format_args"
        | "unreachable" | "todo" | "unimplemented" => Some(0),
        "write" | "writeln" | "assert" | "debug_assert" => Some(1),
        "assert_eq" | "assert_ne" | "debug_assert_eq" | "debug_assert_ne" => Some(2),
        _ => None,
    }
}

/// One argument after the format string.
#[derive(Clone)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct CallArgument<'r, D: Doc> {
    /// The identifier of `name = value`.
    pub(crate) name: Option<Node<'r, D>>,
    /// The first token of the argument's value.
    pub(crate) first: Node<'r, D>,
    /// The last token of the argument's value.
    pub(crate) last: Node<'r, D>,
}

/// A format macro call whose format string is a literal.
#[derive(Clone)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct FormatCall<'r, D: Doc> {
    /// The macro's name without its path (`println`).
    pub(crate) macro_name: String,
    /// The format string's literal token.
    pub(crate) template: Node<'r, D>,
    /// The format string's value.
    pub(crate) value: String,
    /// The arguments after the format string, in source order.
    pub(crate) arguments: Vec<CallArgument<'r, D>>,
    /// How many arguments are not named.
    pub(crate) positional: usize,
    /// The names of the named arguments, in source order.
    pub(crate) named: Vec<String>,
    /// Whether no positional argument follows a named one (rustc refuses the call otherwise).
    pub(crate) well_ordered: bool,
}

/// The call `invocation` makes when it is a format macro with a string literal as its template;
/// `None` for any other macro, for a template that is not a literal (`concat!(..)`, a path), and
/// for a call without a template (`println!()`, `assert!(cond)`).
pub(crate) fn format_call<'r, D: Doc>(invocation: &Node<'r, D>) -> Option<FormatCall<'r, D>> {
    if invocation.kind() != "macro_invocation" {
        return None;
    }
    let macro_name = macro_name(&invocation.field("macro")?.text());
    let leading = leading_arguments(&macro_name)?;
    let tree = invocation
        .children()
        .find(|child| child.kind() == "token_tree")?;
    // A comment is no token of the call: rustc splits the arguments without it.
    let tokens: Vec<Node<'r, D>> = tree
        .children()
        .filter(|token| !matches!(token.kind().as_ref(), "line_comment" | "block_comment"))
        .collect();
    let inner = tokens.get(1..tokens.len().checked_sub(1)?)?;
    let mut segments = split_arguments(inner)?;
    if segments.len() <= leading {
        return None;
    }
    let mut rest = segments.split_off(leading);
    let template_segment = rest.remove(0);
    let [template] = template_segment.as_slice() else {
        return None;
    };
    let value = match literal(template) {
        RustValue::Str(value) => value,
        _ => return None,
    };
    let mut arguments = Vec::new();
    let mut named = Vec::new();
    let mut well_ordered = true;
    for segment in rest {
        let argument = call_argument(&segment)?;
        match &argument.name {
            // A raw identifier names the argument without its `r#` (`r#type` is `type`).
            Some(name) => {
                let name = name.text();
                named.push(name.strip_prefix("r#").unwrap_or(&name).to_string())
            }
            None => well_ordered &= named.is_empty(),
        }
        arguments.push(argument);
    }
    Some(FormatCall {
        macro_name,
        template: template.clone(),
        value,
        positional: arguments.len() - named.len(),
        named,
        arguments,
        well_ordered,
    })
}

/// A macro path's last segment when the path is the standard library's (`std::println`) or a bare
/// name; any other path keeps its text, which no format macro is named.
fn macro_name(path: &str) -> String {
    let path: String = path.chars().filter(|c| !c.is_whitespace()).collect();
    let path = path.strip_prefix("::").unwrap_or(&path);
    for root in ["std::", "core::", "alloc::"] {
        if let Some(name) = path.strip_prefix(root) {
            return name.to_string();
        }
    }
    path.to_string()
}

/// The tokens between a token tree's delimiters split at its top-level commas; a trailing comma
/// ends the last argument. `None` for an empty argument between two commas.
fn split_arguments<'r, D: Doc>(tokens: &[Node<'r, D>]) -> Option<Vec<Vec<Node<'r, D>>>> {
    let mut segments: Vec<Vec<Node<'r, D>>> = Vec::new();
    let mut current: Vec<Node<'r, D>> = Vec::new();
    let mut angles = 0usize;
    let mut after_as = false;
    for token in tokens {
        let kind = token.kind();
        let opens_angle = kind == "<"
            && (after_as
                || angles > 0
                || current
                    .last()
                    .is_some_and(|previous| previous.kind() == "::"));
        match kind.as_ref() {
            "," if angles == 0 => {
                if current.is_empty() {
                    return None;
                }
                segments.push(std::mem::take(&mut current));
                after_as = false;
                continue;
            }
            "as" => after_as = true,
            ">" => angles = angles.saturating_sub(1),
            ">>" => angles = angles.saturating_sub(2),
            _ if opens_angle => angles += 1,
            _ => {}
        }
        current.push(token.clone());
    }
    if !current.is_empty() {
        segments.push(current);
    }
    Some(segments)
}

/// An argument's tokens as `name = value` or a bare value.
fn call_argument<'r, D: Doc>(tokens: &[Node<'r, D>]) -> Option<CallArgument<'r, D>> {
    if let [name, equals, value @ .., last] = tokens
        && name.kind() == "identifier"
        && equals.kind() == "="
    {
        return Some(CallArgument {
            name: Some(name.clone()),
            first: value.first().unwrap_or(last).clone(),
            last: last.clone(),
        });
    }
    Some(CallArgument {
        name: None,
        first: tokens.first()?.clone(),
        last: tokens.last()?.clone(),
    })
}

#[cfg(test)]
mod tests;
