//! Applying a violation's `suppress` to its witnesses: another linter's comments where the anchor
//! starts (the directives themselves are read in `suppress`).

use super::{Witness, anchored_span};
use crate::code_rule_dsl::language::Family;
use crate::code_rule_dsl::lines::Terminators;
use crate::code_rule_dsl::matcher::{
    file_eslint_directives, file_lines, file_noqa, file_nosec, file_phpcs, file_phpstan,
    file_psalm, host,
};
use crate::code_rule_dsl::model::ViolationSpec;
use crate::code_rule_dsl::suppress::{
    EslintDirectives, NoqaIndex, NosecIndex, PhpcsIndex, PhpstanIndex, PsalmIndex,
    parent_statement, root,
};
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
    match Family::of(language) {
        Family::Ecma => suppress.eslint.as_ref().is_some_and(|rules| {
            let directives =
                file_eslint_directives(|| EslintDirectives::read(node, source, &lines));
            directives.suppresses(rules, at, &lines)
        }),
        // A project may run Ruff and Bandit together: either linter the violation names may
        // suppress it. Bandit reads `# nosec` on the reported line and on the lines of the node it
        // visited, here the witness.
        Family::Python => {
            suppress.noqa.as_ref().is_some_and(|noqa| {
                let index = file_noqa(|| NoqaIndex::read(&root(node), source, &lines));
                let parent = noqa.parent.then(|| parent_statement(node, at)).flatten();
                index.suppresses(&noqa.codes, at, parent)
            }) || suppress.nosec.as_ref().is_some_and(|codes| {
                let index = file_nosec(|| NosecIndex::read(&root(node), source, &lines));
                let span = (witness.span.start_byte, witness.span.end_byte);
                index.suppresses(codes, at, span, &lines)
            })
        }
        // A PHP project may run PHPStan, Psalm and PHP_CodeSniffer together: any of the
        // linters a violation names may suppress it. A PHPStan identifier ignores one error, so the
        // violation's witnesses on a line spend the identifiers listed for it, as they are met.
        Family::Php => {
            suppress.phpstan.as_ref().is_some_and(|identifiers| {
                let index = file_phpstan(|| PhpstanIndex::read(&root(node), source, &lines));
                index.suppresses(&violation.code, identifiers, at, &lines)
            }) || suppress.psalm.as_ref().is_some_and(|psalm| {
                let index = file_psalm(|| PsalmIndex::read(&root(node), source));
                index.suppresses(&psalm.names, psalm.suppress_all, at)
            }) || suppress.phpcs.as_ref().is_some_and(|phpcs| {
                let index = file_phpcs(|| PhpcsIndex::read(&root(node), source, &lines));
                index.suppresses(&phpcs.codes, phpcs.version, at, &lines)
            })
        }
        Family::Blade => false,
    }
}
