//! Moved from `adapter.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;
use crate::code_rule_dsl::suppress::{phpcs::PhpcsIndex, phpstan::PhpstanIndex, psalm::PsalmIndex};

/// PHP's family.
pub(crate) struct Php;

impl Adapter for Php {
    const FAMILY: Family = Family::Php;
    const LABEL: &'static str = "php";
    const TERMINATORS: Terminators = Terminators::Newlines;
    const PACKAGES: bool = false;
    const PACKAGE_MANIFESTS: bool = false;
    const JSX_RUNTIMES: bool = false;
    const DECLARATION_FILES: bool = true;
    const STRIPS_BOM: bool = false;
    const COMMENT_KINDS: &'static [&'static str] = &["comment"];
    const PREAMBLE_KINDS: &'static [&'static str] = &["php_tag"];
    const BODY_KINDS: &'static [&'static str] = &[];
    type Scope = Self;
    type Value = Self;
    type Arguments = Self;
    type Calls = Self;
    type FormatArgs = Self;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        text
    }

    fn parse_input(source: &str) -> Cow<'_, str> {
        crate::code_rule_dsl::php_view::view(source)
    }

    fn module_path(_: &Path) -> Option<serde_json::Value> {
        None
    }

    /// A PHP project may run PHPStan, Psalm and PHP_CodeSniffer together: any of the linters a
    /// violation names may suppress it. A PHPStan identifier ignores one error, so the
    /// violation's witnesses on a line spend the identifiers listed for it, as they are met.
    fn suppresses<D: Doc>(
        violation: &ViolationSpec,
        suppress: &Suppress,
        node: &Node<'_, D>,
        source: &str,
        lines: &LineTable,
        anchored: Range<usize>,
        _: (usize, usize),
    ) -> bool {
        let at = anchored.start;
        suppress.get::<crate::code_rule_dsl::suppress::phpstan::Suppressions>().is_some_and(|identifiers| {
            let index = file_index(|| PhpstanIndex::read(&root(node), source, lines));
            index.suppresses(&violation.code, identifiers, at, lines)
        }) || suppress.get::<crate::code_rule_dsl::suppress::psalm::Suppressions>().is_some_and(|psalm| {
            let index = file_index(|| PsalmIndex::read(&root(node), source));
            index.suppresses(&psalm.names, psalm.suppress_all, at)
        }) || suppress.get::<crate::code_rule_dsl::suppress::phpcs::Suppressions>().is_some_and(|phpcs| {
            let index = file_index(|| PhpcsIndex::read(&root(node), source, lines));
            index.suppresses(&phpcs.codes, phpcs.version, at, lines)
        })
    }
}
