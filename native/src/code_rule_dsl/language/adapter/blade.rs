//! Moved from `adapter.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;

/// Blade templates' own tree (`blade::forte`). A template's semantics are those of the PHP it
/// embeds (`blade::view`), a guest document read with PHP's family (`embed`): a clause reads
/// them with `ast.embedded` or `embedded`. The concerns name PHP's, which the compile refuses on
/// the template's own tree (`compile::refuse_unavailable_semantics`).
pub(crate) struct Blade;

impl Adapter for Blade {
    const FAMILY: Family = Family::Blade;
    const LABEL: &'static str = "blade";
    const TERMINATORS: Terminators = Terminators::Newlines;
    const PACKAGES: bool = false;
    const PACKAGE_MANIFESTS: bool = false;
    const JSX_RUNTIMES: bool = false;
    const DECLARATION_FILES: bool = true;
    const STRIPS_BOM: bool = false;
    /// A template's `<!-- -->` (`comment`) and `{{-- --}}` (`blade_comment`); its embedded PHP's
    /// are read with PHP's family.
    const COMMENT_KINDS: &'static [&'static str] = &["comment", "blade_comment"];
    /// A `{{-- --}}` counts as a token of the template.
    const WALK_COMMENT_KINDS: &'static [&'static str] = &["comment"];
    const PARSED_BY_FORTE: bool = true;
    /// A template's `php_tag` is a whole `<?php … ?>` region, code rather than a preamble.
    const PREAMBLE_KINDS: &'static [&'static str] = &[];
    const BODY_KINDS: &'static [&'static str] = &[];
    type Scope = Php;
    type Value = Php;
    type Arguments = Php;
    type Calls = Self;
    type FormatArgs = Php;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        text
    }

    /// Forte parses a template (`blade::forte`), never tree-sitter.
    fn parse_input(source: &str) -> Cow<'_, str> {
        Cow::Borrowed(source)
    }

    fn module_path(_: &Path) -> Option<serde_json::Value> {
        None
    }

    /// No linter reads a template's `{{-- --}}` comments for directives.
    fn suppresses<D: Doc>(
        _: &ViolationSpec,
        _: &Suppress,
        _: &Node<'_, D>,
        _: &str,
        _: &LineTable,
        _: Range<usize>,
        _: (usize, usize),
    ) -> bool {
        false
    }
}
