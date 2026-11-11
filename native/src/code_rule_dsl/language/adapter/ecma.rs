//! Moved from `adapter.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;
use crate::code_rule_dsl::suppress::{biome::BiomeSuppressions, eslint::EslintDirectives, oxlint::OxlintDirectives};

/// TypeScript, TSX and JavaScript: ESLint's semantics.
pub(crate) struct Ecma;

impl Adapter for Ecma {
    const FAMILY: Family = Family::Ecma;
    const LABEL: &'static str = "typescript";
    const TERMINATORS: Terminators = Terminators::Eslint;
    const PACKAGES: bool = false;
    const PACKAGE_MANIFESTS: bool = true;
    const JSX_RUNTIMES: bool = true;
    const DECLARATION_FILES: bool = true;
    const STRIPS_BOM: bool = true;
    /// `html_comment`: a script's `<!--` and line-leading `-->` comments.
    const COMMENT_KINDS: &'static [&'static str] = &["comment", "html_comment"];
    const IMPORT_TARGETS: bool = true;
    const PREAMBLE_KINDS: &'static [&'static str] = &["hash_bang_line"];
    const BODY_KINDS: &'static [&'static str] = &[];
    type Scope = Self;
    type Value = Self;
    type Arguments = Self;
    type Calls = Refused;
    type FormatArgs = Refused;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        ecma_name(text)
    }

    fn parse_input(source: &str) -> Cow<'_, str> {
        Cow::Borrowed(source)
    }

    fn module_path(_: &Path) -> Option<serde_json::Value> {
        None
    }

    /// A project may run ESLint, oxlint (which also reads `eslint-` directives) or Biome: any
    /// linter the violation names may suppress it. Oxlint and Biome ask about the anchored span:
    /// an oxlint block covers a problem that overlaps it, a `biome-ignore-start` range one it
    /// holds, a `biome-ignore` one that starts in what it covers.
    fn suppresses<D: Doc>(
        _: &ViolationSpec,
        suppress: &Suppress,
        node: &Node<'_, D>,
        source: &str,
        lines: &LineTable,
        anchored: Range<usize>,
        _: (usize, usize),
    ) -> bool {
        suppress.get::<crate::code_rule_dsl::suppress::eslint::Suppressions>().is_some_and(|rules| {
            let directives = file_index(|| EslintDirectives::read(node, source, lines));
            directives.suppresses(rules, anchored.start, lines)
        }) || suppress.get::<crate::code_rule_dsl::suppress::oxlint::Suppressions>().is_some_and(|rules| {
            let directives = file_index(|| OxlintDirectives::read(&root(node), source));
            directives.suppresses(rules, anchored.clone())
        }) || suppress.get::<crate::code_rule_dsl::suppress::biome::Suppressions>().is_some_and(|rules| {
            let suppressions =
                file_index(|| BiomeSuppressions::read(&root(node), source));
            suppressions.suppresses(rules, anchored.clone())
        })
    }

    /// `.cjs` and `.cts` (`.d.cts` declarations included) are CommonJS; `.mjs`, `.mts`, `.tsx`
    /// and other declaration files modules. A `.js`, `.jsx` or `.ts` file follows its package's
    /// `type`, as Node reads `.ts` files the same way as `.js` ones (type stripping), and is a
    /// module without a manifest (ESLint's default config, which upstream fixtures assume); where
    /// the manifest sets no valid `type`, Node's syntax detection decides.
    fn source_type(path: &str, package: Option<PackageType>) -> Option<SourceType> {
        let name = path.rsplit('/').next().unwrap_or(path);
        match name.rsplit_once('.').map(|(_, extension)| extension) {
            Some("cjs" | "cts") => Some(SourceType::CommonJs),
            Some("ts") if is_declaration_file(name) => Some(SourceType::Module),
            Some("js" | "jsx" | "ts") => match package {
                None | Some(PackageType::Module) => Some(SourceType::Module),
                Some(PackageType::CommonJs) => Some(SourceType::CommonJs),
                Some(PackageType::Unset) => None,
            },
            _ => Some(SourceType::Module),
        }
    }
}

impl Ecma {
    /// The module system of a file by its extension and its nearest `package.json`'s `type` alone
    /// (`SourceType::by_package_type`).
    pub(crate) fn source_type_by_package_type(
        path: &str,
        package: Option<PackageType>,
    ) -> SourceType {
        SourceType::by_package_type(path, package)
    }
}
