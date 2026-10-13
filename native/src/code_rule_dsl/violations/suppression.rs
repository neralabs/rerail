//! Applying a violation's `suppress` to its witnesses: another linter's comments where the anchor
//! starts (the directives themselves are read in `suppress`).

use super::{Witness, anchored_span};
use crate::code_rule_dsl::language::Family;
use crate::code_rule_dsl::language::adapter::{Adapter, with_adapter};
use crate::code_rule_dsl::lines::Terminators;
use crate::code_rule_dsl::matcher::{file_lines, host};
use crate::code_rule_dsl::model::ViolationSpec;
use ast_grep_core::{Doc, Node};

/// Whether the violation's `suppress` removes a witness: another linter's comments suppress the
/// violation at its anchor's start. `node` is the witness's node, which places the file.
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
    let at = anchored_span(&violation.anchor, witness).0.start_byte;
    let lines = file_lines(source, Terminators::of(language));
    // Compiling checked that the query's family honours each linter `suppress` names.
    let span = (witness.span.start_byte, witness.span.end_byte);
    with_adapter!(Family::of(language), A => A::suppresses(violation, suppress, node, source, &lines, at, span))
}
