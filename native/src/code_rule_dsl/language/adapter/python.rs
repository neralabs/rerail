//! Moved from `adapter.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;

/// Python's family: Ruff's semantics.
pub(crate) struct Python;

impl Adapter for Python {
    const FAMILY: Family = Family::Python;
    const LABEL: &'static str = "python";
    const TERMINATORS: Terminators = Terminators::Newlines;
    const PACKAGES: bool = true;
    const PACKAGE_MANIFESTS: bool = false;
    const JSX_RUNTIMES: bool = false;
    const DECLARATION_FILES: bool = false;
    const STRIPS_BOM: bool = false;
    const COMMENT_KINDS: &'static [&'static str] = &["comment"];
    const PREAMBLE_KINDS: &'static [&'static str] = &[];
    const BODY_KINDS: &'static [&'static str] = &["block"];
    type Scope = Self;
    type Value = Self;
    type Arguments = Self;
    type Calls = Self;
    type FormatArgs = Self;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        python_name(text)
    }

    fn parse_input(source: &str) -> Cow<'_, str> {
        Cow::Borrowed(source)
    }

    fn module_path(file: &Path) -> Option<serde_json::Value> {
        python_module_path_on_disk(file)
    }

    fn suppresses<D: Doc>(
        _: &ViolationSpec,
        suppress: &Suppress,
        node: &Node<'_, D>,
        source: &str,
        lines: &LineTable,
        anchored: Range<usize>,
        witness: (usize, usize),
    ) -> bool {
        let at = anchored.start;
        // A project may run Ruff and Bandit together: either linter the violation names may
        // suppress it. Bandit reads `# nosec` on the reported line and on the lines of the node it
        // visited, here the witness.
        suppress.get::<crate::code_rule_dsl::suppress::noqa::Suppressions>().is_some_and(|noqa| {
            let index = file_index(|| NoqaIndex::read(&root(node), source, lines));
            let parent = noqa.parent.then(|| parent_statement(node, at)).flatten();
            index.suppresses(&noqa.codes, at, parent)
        }) || suppress.get::<crate::code_rule_dsl::suppress::nosec::Suppressions>().is_some_and(|codes| {
            let index = file_index(|| NosecIndex::read(&root(node), source, lines));
            index.suppresses(codes, at, witness, lines)
        })
    }
}
