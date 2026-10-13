//! Applying a violation's `suppress` to its witnesses: another linter's comments where the anchor
//! starts, or over the anchored span (the directives themselves are read in `suppress`); and
//! Rerail's own `rerail-ignore` comments, which every reported violation answers to.

use super::{Witness, anchored_span};
use crate::code_rule_dsl::language::Family;
use crate::code_rule_dsl::language::adapter::{Adapter, with_adapter};
use crate::code_rule_dsl::lines::Terminators;
use crate::code_rule_dsl::matcher::{file_lines, file_rerail_ignores, host};
use crate::code_rule_dsl::model::ViolationSpec;
use crate::code_rule_dsl::suppress::{RerailIgnores, root};
use ast_grep_core::{Doc, Node};
use std::rc::Rc;

/// Whether the violation's `suppress` removes a witness: another linter's comments suppress the
/// violation at its anchor's start (oxlint's and Biome's: over the anchored span). `node` is the
/// witness's node, which places the file.
pub(crate) fn suppresses<D: Doc>(
    violation: Option<&ViolationSpec>,
    node: &Node<'_, D>,
    witness: &Witness,
) -> bool {
    let Some((violation, suppress)) =
        violation.and_then(|violation| Some((violation, violation.suppress.as_ref()?)))
    else {
        return false;
    };
    let Some((source, language)) = host(node) else {
        return false;
    };
    let anchored = anchored_span(&violation.anchor, witness).0;
    let lines = file_lines(source, Terminators::of(language));
    // Compiling checked that the query's family honours each linter `suppress` names.
    let span = (witness.span.start_byte, witness.span.end_byte);
    with_adapter!(Family::of(language), A => A::suppresses(violation, suppress, node, source, &lines, anchored.start_byte..anchored.end_byte, span))
}

/// Whether a `rerail-ignore` comment naming the violation's code rule covers a witness's anchor
/// start: always honoured, whatever `suppress` and the `suppressions` setting. `node` is the
/// witness's node, which places the file.
pub(crate) fn rerail_ignores<D: Doc>(
    violation: Option<&ViolationSpec>,
    node: &Node<'_, D>,
    witness: &Witness,
) -> bool {
    let Some((violation, rule)) =
        violation.and_then(|violation| Some((violation, violation.ignored_by.as_deref()?)))
    else {
        return false;
    };
    file_ignores(node).is_some_and(|ignores| {
        ignores.ignores(rule, anchored_span(&violation.anchor, witness).0.start_byte)
    })
}

/// Whether the file `node` belongs to holds a `rerail-ignore` comment naming the violation's code
/// rule, which may then drop some of its witnesses.
pub(crate) fn names_rerail_ignore<D: Doc>(violation: &ViolationSpec, node: &Node<'_, D>) -> bool {
    violation
        .ignored_by
        .as_deref()
        .is_some_and(|rule| file_ignores(node).is_some_and(|ignores| ignores.names(rule)))
}

/// The `rerail-ignore` directives of the file `node` belongs to; none for a parsed literal's tree.
fn file_ignores<D: Doc>(node: &Node<'_, D>) -> Option<Rc<RerailIgnores>> {
    let (source, language) = host(node)?;
    Some(file_rerail_ignores(|| {
        let lines = file_lines(source, Terminators::of(language));
        RerailIgnores::read(&root(node), source, &lines)
    }))
}
