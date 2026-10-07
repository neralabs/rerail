//! Destructuring from a literal: the names a declaration pattern binds, paired with the elements
//! of an object or array literal initializer they take (`const {a, b: [c]} = {a: 1, b: [2]}` gives
//! `a` the node `1` and `c` the node `2`). A name gets an element only where the literal decides
//! it: no default, no rest, no spread or computed key that could supply it, no getter.

use super::operand;
use crate::policy_dsl::value::NodeRef;
use crate::policy_dsl::value::key::{identifier_name, static_property_name};
use ast_grep_core::{Doc, Node};

/// Pairs `pattern`'s names with `value`'s elements, each as the name node's start and the
/// element as written.
pub(crate) fn destructured<D: Doc>(
    pattern: &Node<'_, D>,
    value: &Node<'_, D>,
    out: &mut Vec<(usize, NodeRef)>,
) {
    let literal = unwrapped(value);
    match (pattern.kind().as_ref(), literal.kind().as_ref()) {
        ("identifier" | "undefined", _) => out.push((pattern.range().start, NodeRef::of(value))),
        ("object_pattern", "object") => {
            for member in named(pattern) {
                match member.kind().as_ref() {
                    "shorthand_property_identifier_pattern" => {
                        if let Some(name) = identifier_name(&member.text())
                            && let Some(element) = property(&literal, &name)
                        {
                            out.push((member.range().start, NodeRef::of(&element)));
                        }
                    }
                    "pair_pattern" => {
                        if let Some(name) = static_property_name(&member)
                            && let Some(element) = property(&literal, &name)
                            && let Some(target) = member.field("value")
                        {
                            destructured(&target, &element, out);
                        }
                    }
                    _ => {}
                }
            }
        }
        ("array_pattern", "array") => {
            let elements = elements(&literal);
            for (index, target) in elements_of_pattern(pattern) {
                // Past a spread no position is known, whatever stands at this one in the pattern.
                let element = match elements.get(index) {
                    Some(Element::Spread) => break,
                    Some(Element::Node(element)) => Some(element),
                    _ => None,
                };
                let Some(target) = target else { continue };
                if target.kind() == "rest_pattern" {
                    break;
                }
                if let Some(element) = element {
                    destructured(&target, element, out);
                }
            }
        }
        _ => {}
    }
}

fn named<'a, 'r, D: Doc>(node: &'a Node<'r, D>) -> impl Iterator<Item = Node<'r, D>> + 'a {
    node.children()
        .filter(|child| child.is_named() && child.kind() != "comment")
}

/// The literal a value is, through parentheses and TypeScript's wrappers.
fn unwrapped<'r, D: Doc>(value: &Node<'r, D>) -> Node<'r, D> {
    let mut node = value.clone();
    while matches!(
        node.kind().as_ref(),
        "parenthesized_expression" | "as_expression" | "satisfies_expression"
    ) {
        match operand(&node) {
            Some(inner) => node = inner,
            None => break,
        }
    }
    node
}

/// The node an object literal's own property `name` holds after the literal is built: the last
/// member that defines it, unless a later spread, computed key or accessor may redefine it.
fn property<'r, D: Doc>(literal: &Node<'r, D>, name: &[u16]) -> Option<Node<'r, D>> {
    // `__proto__: v` sets the prototype, not a property.
    if name == "__proto__".encode_utf16().collect::<Vec<_>>() {
        return None;
    }
    let mut found = None;
    for member in named(literal) {
        match member.kind().as_ref() {
            "pair" => match static_property_name(&member) {
                Some(key) if key == name => found = member.field("value"),
                Some(_) => {}
                None => found = None,
            },
            "shorthand_property_identifier" => {
                if identifier_name(&member.text()).is_some_and(|key| key == name) {
                    found = Some(member);
                }
            }
            // A method or accessor of the name has no element; one of another name leaves it.
            "method_definition" => {
                if static_property_name(&member).is_none_or(|key| key == name) {
                    found = None;
                }
            }
            _ => found = None,
        }
    }
    found
}

enum Element<'r, D: Doc> {
    Node(Node<'r, D>),
    Hole,
    Spread,
}

/// An array literal's elements by position: holes, and a spread past which positions are not
/// known.
fn elements<'r, D: Doc>(literal: &Node<'r, D>) -> Vec<Element<'r, D>> {
    let mut out = Vec::new();
    let mut after_element = false;
    for child in literal.children() {
        match child.kind().as_ref() {
            "[" | "]" | "comment" => {}
            "," => {
                if !after_element {
                    out.push(Element::Hole);
                }
                after_element = false;
            }
            "spread_element" => {
                out.push(Element::Spread);
                after_element = true;
            }
            _ => {
                out.push(Element::Node(child));
                after_element = true;
            }
        }
    }
    out
}

/// An array pattern's targets by position (`None` for a hole).
fn elements_of_pattern<'r, D: Doc>(pattern: &Node<'r, D>) -> Vec<(usize, Option<Node<'r, D>>)> {
    let mut out = Vec::new();
    let mut after_element = false;
    for child in pattern.children() {
        match child.kind().as_ref() {
            "[" | "]" | "comment" => {}
            "," => {
                if !after_element {
                    out.push((out.len(), None));
                }
                after_element = false;
            }
            _ => {
                // A default (`[a = 1]`) is taken only when the element is `undefined`.
                let target = (child.kind() != "assignment_pattern").then_some(child);
                out.push((out.len(), target));
                after_element = true;
            }
        }
    }
    out
}
