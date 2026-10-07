//! Applying a finding's `suppress` to its witnesses: another linter's comments where the anchor
//! starts (the directives themselves are read in `suppress`).

use super::{Witness, anchored_span};
use crate::policy_dsl::language::Family;
use crate::policy_dsl::lines::Terminators;
use crate::policy_dsl::matcher::{
    file_eslint_directives, file_lines, file_noqa, file_phpcs, file_phpstan, file_psalm, host,
};
use crate::policy_dsl::model::FindingSpec;
use crate::policy_dsl::suppress::{
    EslintDirectives, NoqaIndex, PhpcsIndex, PhpstanIndex, PsalmIndex, parent_statement, root,
};
use ast_grep_core::{Doc, Node};

/// Whether the finding's `suppress` removes a witness: another linter's comments suppress the
/// finding at its anchor's start. `node` is the witness's node, which places the file.
pub(crate) fn suppresses<D: Doc>(
    finding: Option<&FindingSpec>,
    node: &Node<'_, D>,
    witness: &Witness,
) -> bool {
    let Some((finding, suppress)) =
        finding.and_then(|finding| Some((finding, finding.suppress.as_ref()?)))
    else {
        return false;
    };
    let Some((source, language)) = host(node) else {
        return false;
    };
    let at = anchored_span(&finding.anchor, witness).0.start_byte;
    let lines = file_lines(source, Terminators::of(language));
    // Compiling checked that the query's family honours each linter `suppress` names.
    match Family::of(language) {
        Family::Ecma => suppress.eslint.as_ref().is_some_and(|rules| {
            let directives =
                file_eslint_directives(|| EslintDirectives::read(node, source, &lines));
            directives.suppresses(rules, at, &lines)
        }),
        Family::Python => suppress.noqa.as_ref().is_some_and(|noqa| {
            let index = file_noqa(|| NoqaIndex::read(&root(node), source, &lines));
            let parent = noqa.parent.then(|| parent_statement(node, at)).flatten();
            index.suppresses(&noqa.codes, at, parent)
        }),
        // A PHP project may run PHPStan, Psalm and PHP_CodeSniffer together: any of the
        // linters a finding names may suppress it. A PHPStan identifier ignores one error, so the
        // finding's witnesses on a line spend the identifiers listed for it, as they are met.
        Family::Php => {
            suppress.phpstan.as_ref().is_some_and(|identifiers| {
                let index = file_phpstan(|| PhpstanIndex::read(&root(node), source, &lines));
                index.suppresses(&finding.code, identifiers, at, &lines)
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
