// Inside `src/` beside `build_identity.rs`, so every copy of the native sources carries it; only
// `build.rs` compiles it.
//! The language registry (`languages/*.json`, format `rerail.language/v1`, described in
//! `docs/language-packs.md`): read, validated, and turned into the engine's language tables.
//! `generate` writes `language_registry.rs` to `OUT_DIR`; `code_rule_dsl::language` includes it. A
//! registry the engine cannot be built from fails the build here, naming the file and the fault.
//! The TypeScript host reads the same files through `src/engine/code-rule/code-rule-language-registry.generated.ts`, which
//! `scripts/generate/languages/registry.ts` writes and the host's tests keep current.

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const FORMAT: &str = "rerail.language/v1";
/// The parser implementations a dialect may name: tree-sitter, or Forte (Rerail's port,
/// `code_rule_dsl::blade::forte`).
const PARSERS: &[&str] = &["tree-sitter", "forte"];
/// The grammars Rerail implements itself (`native:<name>`): no tree-sitter grammar backs them.
const NATIVE_GRAMMARS: &[&str] = &["forte"];
/// ast-grep-language 0.45's `SupportLang` variants. A vendored grammar whose dialect variant is
/// one of them (Python, TypeScript and TSX, which Rerail vendors with patches) still lends that
/// language's pattern syntax (`GrammarDialect::support`); any other vendored grammar, and a
/// native one (Blade's Forte), has none.
const AST_GREP_LANGUAGES: &[&str] = &[
    "Bash",
    "C",
    "Cpp",
    "CSharp",
    "Css",
    "Dart",
    "Go",
    "Elixir",
    "Haskell",
    "Hcl",
    "Html",
    "Java",
    "JavaScript",
    "Json",
    "Kotlin",
    "Lua",
    "Markdown",
    "Nix",
    "Php",
    "Python",
    "Ruby",
    "Rust",
    "Scala",
    "Solidity",
    "Swift",
    "Tsx",
    "TypeScript",
    "Yaml",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Entry {
    format: String,
    id: String,
    status: Status,
    family: String,
    dialects: Vec<Dialect>,
    #[serde(default)]
    runs_on: Option<Vec<String>>,
    capabilities: BTreeMap<String, Support>,
    suppressions: Vec<String>,
    #[serde(default)]
    embeds: Vec<Embed>,
}

#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum Status {
    Supported,
    Planned,
}

#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum Support {
    Supported,
    Refused,
    NotSupportedYet,
}

impl Support {
    const fn variant(self) -> &'static str {
        match self {
            Self::Supported => "Supported",
            Self::Refused => "Refused",
            Self::NotSupportedYet => "NotSupportedYet",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Dialect {
    label: String,
    #[serde(default)]
    variant: Option<String>,
    suffixes: Vec<String>,
    #[serde(default)]
    parser: Option<String>,
    #[serde(default)]
    grammar: Option<String>,
    /// The dialect's persisted tape tag. A supported dialect needs one; a planned dialect may
    /// reserve its future tag. Either way it is unique across the registry and never reused.
    #[serde(default)]
    tape_tag: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Embed {
    language: String,
    via: String,
}

/// A supported dialect's grammar: one `build.rs` generates from a vendored `grammar.json`, one
/// Rerail implements itself (no tree-sitter grammar), or one of ast-grep's.
enum Grammar<'a> {
    Vendored(&'a str),
    Native(&'a str),
    AstGrep(&'a str),
}

/// Reads and validates every registry file in `directory`, writes `language_registry.rs` to
/// `out`, and returns each file read with its bytes (they are part of the engine's identity).
pub(crate) fn generate(directory: &Path, vendored: &[&str], out: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = std::fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("{} is unreadable: {error}", directory.display()))
        .map(|entry| entry.expect("registry entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<PathBuf>>();
    files.sort();
    let mut read = Vec::new();
    let mut entries = Vec::new();
    for path in &files {
        let name = format!(
            "languages/{}",
            path.file_name().expect("file name").to_string_lossy()
        );
        let bytes = std::fs::read(path).unwrap_or_else(|error| panic!("{name}: {error}"));
        let entry: Entry = serde_json::from_slice(&bytes)
            .unwrap_or_else(|error| panic!("{name} is not a language registry entry: {error}"));
        let stem = path.file_stem().expect("file stem").to_string_lossy();
        validate_entry(&name, &stem, &entry, vendored);
        read.push((name.clone(), bytes));
        entries.push((name, entry));
    }
    validate_registry(&entries);
    entries.sort_by_key(|(_, entry)| order(entry));
    let entries = entries
        .into_iter()
        .map(|(_, entry)| entry)
        .collect::<Vec<_>>();
    std::fs::write(out.join("language_registry.rs"), render(&entries))
        .expect("generated language registry is writable");
    read
}

/// Supported languages in the order they were added (their first dialect's tape tag), then
/// planned ones by id: the order hosts list languages in.
fn order(entry: &Entry) -> (u8, u16, String) {
    match entry.status {
        Status::Supported => (
            0,
            u16::from(entry.dialects[0].tape_tag.expect("validated")),
            entry.id.clone(),
        ),
        Status::Planned => (1, 0, entry.id.clone()),
    }
}

fn is_name(text: &str) -> bool {
    let mut characters = text.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn is_variant(text: &str) -> bool {
    let mut characters = text.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_uppercase())
        && characters.all(|character| character.is_ascii_alphanumeric())
}

fn parse_grammar(grammar: &str) -> Option<Grammar<'_>> {
    if let Some(name) = grammar.strip_prefix("vendored:") {
        return is_name(name).then_some(Grammar::Vendored(name));
    }
    if let Some(name) = grammar.strip_prefix("native:") {
        return is_name(name).then_some(Grammar::Native(name));
    }
    grammar
        .strip_prefix("ast-grep:")
        .filter(|variant| is_variant(variant))
        .map(Grammar::AstGrep)
}

fn check_grammar(name: &str, label: &str, grammar: &str, vendored: &[&str]) {
    match parse_grammar(grammar) {
        Some(Grammar::Vendored(vendored_name)) => assert!(
            vendored.contains(&vendored_name),
            "{name}: dialect {label} names vendored grammar `{vendored_name}`, which build.rs does not build (GRAMMARS)"
        ),
        Some(Grammar::Native(native)) => assert!(
            NATIVE_GRAMMARS.contains(&native),
            "{name}: dialect {label} names native grammar `{native}`, which is not one of {NATIVE_GRAMMARS:?}"
        ),
        Some(Grammar::AstGrep(_)) => {}
        None => panic!(
            "{name}: dialect {label} grammar `{grammar}` is neither vendored:<name>, native:<name> nor ast-grep:<Language>"
        ),
    }
}

fn validate_entry(name: &str, stem: &str, entry: &Entry, vendored: &[&str]) {
    assert!(
        entry.format == FORMAT,
        "{name}: format is `{}`, expected `{FORMAT}`",
        entry.format
    );
    assert!(
        is_name(&entry.id),
        "{name}: id `{}` is not lowercase letters, digits and dashes",
        entry.id
    );
    assert!(
        entry.id == stem,
        "{name}: id `{}` must equal the file name",
        entry.id
    );
    assert!(
        is_name(&entry.family),
        "{name}: family `{}` is not lowercase letters, digits and dashes",
        entry.family
    );
    assert!(
        !entry.dialects.is_empty(),
        "{name}: a language has at least one dialect"
    );
    let supported = entry.status == Status::Supported;
    for dialect in &entry.dialects {
        let label = &dialect.label;
        assert!(
            is_name(label),
            "{name}: dialect label `{label}` is not lowercase letters, digits and dashes"
        );
        assert!(
            !dialect.suffixes.is_empty(),
            "{name}: dialect {label} lists no file suffix"
        );
        for suffix in &dialect.suffixes {
            assert!(
                suffix.len() > 1
                    && suffix.starts_with('.')
                    && !suffix.contains('/')
                    && *suffix == suffix.to_ascii_lowercase(),
                "{name}: dialect {label} suffix `{suffix}` must be a lowercase `.suffix`"
            );
        }
        if let Some(parser) = &dialect.parser {
            assert!(
                PARSERS.contains(&parser.as_str()),
                "{name}: dialect {label} parser `{parser}` is not one of {PARSERS:?}"
            );
        }
        if let Some(text) = &dialect.grammar {
            check_grammar(name, label, text, vendored);
        }
        if let Some(variant) = &dialect.variant {
            assert!(
                is_variant(variant),
                "{name}: dialect {label} variant `{variant}` is not a Rust type name"
            );
        }
        assert!(
            dialect.tape_tag != Some(0),
            "{name}: dialect {label} tape tag 0 is reserved"
        );
        if supported {
            assert!(
                dialect.variant.is_some(),
                "{name}: supported dialect {label} names no engine variant"
            );
            assert!(
                dialect.parser.is_some(),
                "{name}: supported dialect {label} names no parser"
            );
            assert!(
                dialect.grammar.is_some(),
                "{name}: supported dialect {label} has no grammar; a supported language needs one for every dialect"
            );
            assert!(
                dialect.tape_tag.is_some(),
                "{name}: supported dialect {label} has no tape tag"
            );
        }
    }
    if !supported {
        assert!(
            entry.runs_on.is_none(),
            "{name}: a planned language runs on no dialect yet"
        );
        assert!(
            entry
                .capabilities
                .values()
                .all(|support| *support != Support::Supported),
            "{name}: a planned language supports no capability yet"
        );
    }
    for suppression in &entry.suppressions {
        assert!(
            is_name(suppression),
            "{name}: suppression `{suppression}` is not lowercase letters, digits and dashes"
        );
    }
    let suppressions = entry.suppressions.iter().collect::<BTreeSet<_>>();
    assert!(
        suppressions.len() == entry.suppressions.len(),
        "{name}: suppressions repeat"
    );
    for embed in &entry.embeds {
        assert!(
            is_name(&embed.via),
            "{name}: embeds {} via `{}` is not lowercase letters, digits and dashes",
            embed.language,
            embed.via
        );
        assert!(
            embed.language != entry.id,
            "{name}: a language does not embed itself"
        );
    }
}

fn validate_registry(entries: &[(String, Entry)]) {
    assert!(!entries.is_empty(), "languages/ holds no language");
    let ids = entries
        .iter()
        .map(|(_, entry)| entry.id.as_str())
        .collect::<BTreeSet<_>>();
    let capabilities = entries[0].1.capabilities.keys().collect::<Vec<_>>();
    let mut labels = BTreeMap::new();
    let mut suffixes = BTreeMap::new();
    let mut tags = BTreeMap::new();
    for (name, entry) in entries {
        assert!(
            entry.capabilities.keys().collect::<Vec<_>>() == capabilities,
            "{name}: capabilities must name the same capabilities as {}",
            entries[0].0
        );
        for dialect in &entry.dialects {
            if let Some(other) = labels.insert(dialect.label.as_str(), (name, entry)) {
                panic!(
                    "{name}: dialect label {} is also {}'s",
                    dialect.label, other.0
                );
            }
            for suffix in &dialect.suffixes {
                if let Some(other) = suffixes.insert(suffix.as_str(), name) {
                    panic!("{name}: suffix {suffix} is also {other}'s");
                }
            }
            if let Some(tag) = dialect.tape_tag
                && let Some(other) = tags.insert(tag, name)
            {
                panic!("{name}: tape tag {tag} is also {other}'s; a tape tag is never reused");
            }
        }
        for embed in &entry.embeds {
            assert!(
                ids.contains(embed.language.as_str()),
                "{name}: embeds unknown language {}",
                embed.language
            );
        }
    }
    for (name, entry) in entries
        .iter()
        .filter(|(_, entry)| entry.status == Status::Supported)
    {
        for embed in &entry.embeds {
            let (_, embedded) = entries
                .iter()
                .find(|(_, candidate)| candidate.id == embed.language)
                .expect("validated embedded language");
            assert!(
                embedded.status == Status::Supported,
                "{name}: embeds planned language {}; a supported language embeds only supported ones",
                embed.language
            );
        }
    }
    for (name, entry) in entries
        .iter()
        .filter(|(_, entry)| entry.status == Status::Supported)
    {
        let own = entry
            .dialects
            .iter()
            .map(|dialect| dialect.label.as_str())
            .collect::<Vec<_>>();
        if let Some(runs_on) = &entry.runs_on {
            assert!(
                runs_on.first().map(String::as_str) == Some(own[0]),
                "{name}: runsOn starts with the language's first dialect, {}",
                own[0]
            );
            let unique = runs_on.iter().collect::<BTreeSet<_>>();
            assert!(
                unique.len() == runs_on.len(),
                "{name}: runsOn repeats a dialect"
            );
            for label in &own {
                assert!(
                    runs_on.iter().any(|run| run == label),
                    "{name}: runsOn omits its own dialect {label}"
                );
            }
            for label in runs_on {
                let (_, owner) = labels
                    .get(label.as_str())
                    .unwrap_or_else(|| panic!("{name}: runsOn names unknown dialect {label}"));
                assert!(
                    owner.status == Status::Supported && owner.family == entry.family,
                    "{name}: runsOn dialect {label} is not a supported dialect of family {}",
                    entry.family
                );
            }
        }
        for (other_name, other) in entries {
            if other.status == Status::Supported && other.family == entry.family {
                assert!(
                    other.suppressions == entry.suppressions,
                    "{name} and {other_name} share family {} but not its suppressions",
                    entry.family
                );
            }
        }
    }
}

/// `scope` → `Scope`, `formatArgs` → `FormatArgs`, `tree-sitter` → `TreeSitter`.
fn pascal(name: &str) -> String {
    name.split('-')
        .map(|part| {
            let mut characters = part.chars();
            characters
                .next()
                .map(|first| first.to_ascii_uppercase().to_string() + characters.as_str())
                .unwrap_or_default()
        })
        .collect()
}

fn render(entries: &[Entry]) -> String {
    let supported = entries
        .iter()
        .filter(|entry| entry.status == Status::Supported)
        .collect::<Vec<_>>();
    let variant = |label: &str| -> String {
        let dialect = supported
            .iter()
            .flat_map(|entry| &entry.dialects)
            .find(|dialect| dialect.label == label)
            .expect("validated dialect");
        format!(
            "GrammarDialect::{}",
            dialect.variant.as_deref().expect("validated")
        )
    };
    let mut out = String::from(
        "// Generated by build.rs (src/build_languages.rs) from languages/*.json; do not edit.\n\n",
    );

    out.push_str("/// Declares `GrammarDialect` with the registry's supported dialects (each its entry's\n/// `variant`) followed by `language.rs`'s test-only grammars (`$test`, each with the ast-grep\n/// language that lends it pattern syntax), its `ALL`, and `support`.\n");
    out.push_str("macro_rules! grammar_dialect {\n");
    out.push_str("    ($($test:ident => $support:path),* $(,)?) => {\n");
    out.push_str("        /// Rerail's grammar dialect. `ast_grep_language::SupportLang` is closed, while vendored\n        /// grammars (Blade's, the patched Python and TypeScript) are part of Rerail's source surface.\n");
    out.push_str("        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]\n        pub(crate) enum GrammarDialect {\n");
    for dialect in supported.iter().flat_map(|entry| &entry.dialects) {
        let _ = writeln!(
            out,
            "            {},",
            dialect.variant.as_deref().expect("validated")
        );
    }
    out.push_str(
        "            $(#[cfg(test)] $test,)*\n        }\n\n        impl GrammarDialect {\n",
    );
    out.push_str("            /// Every dialect, test-only grammars included.\n            const ALL: &'static [Self] = &[\n");
    for dialect in supported.iter().flat_map(|entry| &entry.dialects) {
        let _ = writeln!(
            out,
            "                Self::{},",
            dialect.variant.as_deref().expect("validated")
        );
    }
    out.push_str("                $(#[cfg(test)] Self::$test,)*\n            ];\n\n");
    out.push_str("            /// The ast-grep language whose pattern syntax the dialect's rules use, if any.\n            const fn support(self) -> Option<SupportLang> {\n                match self {\n");
    for dialect in supported.iter().flat_map(|entry| &entry.dialects) {
        let name = dialect.variant.as_deref().expect("validated");
        let support = match parse_grammar(dialect.grammar.as_deref().expect("validated")) {
            Some(Grammar::AstGrep(language)) => Some(language),
            Some(Grammar::Vendored(_)) => AST_GREP_LANGUAGES.contains(&name).then_some(name),
            Some(Grammar::Native(_)) => None,
            None => unreachable!("validated grammar"),
        };
        let _ = match support {
            Some(language) => writeln!(
                out,
                "                    Self::{name} => Some(SupportLang::{language}),"
            ),
            None => writeln!(out, "                    Self::{name} => None,"),
        };
    }
    out.push_str("                    $(#[cfg(test)] Self::$test => Some($support),)*\n                }\n            }\n        }\n    };\n}\n\n");
    let vias = supported
        .iter()
        .flat_map(|entry| &entry.embeds)
        .map(|embed| embed.via.as_str())
        .collect::<Vec<_>>();
    let mut unique_vias = Vec::new();
    for via in vias {
        if !unique_vias.contains(&via) {
            unique_vias.push(via);
        }
    }
    out.push_str("/// How a supported language embeds another (the registry's `embeds` `via`): each names the\n/// embedder that builds the embedded program from a file's tree (`embed`).\n");
    out.push_str("#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]\n");
    out.push_str("pub(crate) enum Via {\n");
    for via in &unique_vias {
        let _ = writeln!(out, "    {},", pascal(via));
    }
    out.push_str("}\n\nimpl GrammarDialect {\n");
    out.push_str("    /// The dialects a file of this dialect embeds programs of, each with the `Via` that builds\n    /// them: every dialect of each language its registry entry `embeds`, in registry order.\n");
    out.push_str("    pub(crate) const fn embeds(self) -> &'static [(Via, GrammarDialect)] {\n        match self {\n");
    for entry in &supported {
        for dialect in &entry.dialects {
            let host = dialect.variant.as_deref().expect("validated");
            let embeds = entry
                .embeds
                .iter()
                .flat_map(|embed| {
                    entries
                        .iter()
                        .find(|candidate| candidate.id == embed.language)
                        .expect("validated embedded language")
                        .dialects
                        .iter()
                        .map(move |dialect| {
                            format!(
                                "(Via::{}, GrammarDialect::{})",
                                pascal(&embed.via),
                                dialect.variant.as_deref().expect("validated")
                            )
                        })
                })
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "            Self::{host} => &[{embeds}],");
        }
    }
    out.push_str("            #[cfg(test)]\n            _ => &[],\n        }\n    }\n}\n\n");
    out.push_str("/// Evaluates `$body` with `$e` the embedder of `$via` (`embed::<Via>`): one arm per `Via`.\n");
    out.push_str("macro_rules! with_embedder {\n    ($via:expr, $e:ident => $body:expr) => {\n        match $via {\n");
    for via in &unique_vias {
        let via = pascal(via);
        let _ = writeln!(
            out,
            "            $crate::code_rule_dsl::language::Via::{via} => {{\n                type $e = $crate::code_rule_dsl::embed::{via};\n                $body\n            }}"
        );
    }
    out.push_str("        }\n    };\n}\n\npub(crate) use with_embedder;\n\n");

    out.push_str("const DIALECTS: &[Dialect] = &[\n");
    for entry in &supported {
        for dialect in &entry.dialects {
            let _ = writeln!(
                out,
                "    Dialect {{ lang: {}, label: {:?}, #[cfg(test)] suffixes: &{:?}, tape_tag: {} }},",
                variant(&dialect.label),
                dialect.label,
                dialect.suffixes,
                dialect.tape_tag.expect("validated"),
            );
        }
    }
    out.push_str("];\n\n");

    out.push_str("/// A supported dialect's tree-sitter grammar, as the registry names it; `None` for one Rerail\n/// parses itself and for a test-only grammar.\n");
    out.push_str("fn registry_grammar(lang: GrammarDialect) -> Option<TSLanguage> {\n");
    out.push_str("    unsafe extern \"C\" {\n");
    let mut declared = BTreeSet::new();
    let mut arms = String::new();
    for dialect in supported.iter().flat_map(|entry| &entry.dialects) {
        let text = dialect.grammar.as_deref().expect("validated");
        let language = match parse_grammar(text).expect("validated grammar") {
            Grammar::Vendored(name) => {
                let symbol = format!("tree_sitter_{}", name.replace('-', "_"));
                if declared.insert(symbol.clone()) {
                    let _ = writeln!(out, "        fn {symbol}() -> *const ();");
                }
                format!(
                    "Some(unsafe {{ tree_sitter_language::LanguageFn::from_raw({symbol}) }}.into())"
                )
            }
            Grammar::Native(_) => "None".to_owned(),
            Grammar::AstGrep(language) => {
                format!("Some(SupportLang::{language}.get_ts_language())")
            }
        };
        let _ = writeln!(arms, "        {} => {language},", variant(&dialect.label));
    }
    out.push_str("    }\n    match lang {\n");
    out.push_str(&arms);
    out.push_str("        #[cfg(test)]\n        _ => None,\n    }\n}\n\n");

    out.push_str(
        "/// A code-rule language the registry marks supported: the `language` of a query, a define\n/// or a transform.\n",
    );
    out.push_str(
        "#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]\npub(crate) enum CodeRuleLanguage {\n",
    );
    for entry in &supported {
        let _ = writeln!(out, "    {},", pascal(&entry.id));
    }
    out.push_str("}\n\nimpl CodeRuleLanguage {\n");
    let _ = writeln!(
        out,
        "    pub(crate) const ALL: [Self; {}] = [",
        supported.len()
    );
    for entry in &supported {
        let _ = writeln!(out, "        Self::{},", pascal(&entry.id));
    }
    out.push_str(
        "    ];\n\n    pub(crate) const fn name(self) -> &'static str {\n        match self {\n",
    );
    for entry in &supported {
        let _ = writeln!(
            out,
            "            Self::{} => {:?},",
            pascal(&entry.id),
            entry.id
        );
    }
    out.push_str("        }\n    }\n\n");
    out.push_str("    /// The dialects its rules run on, the first being the one its own text (transform templates,\n    /// define clauses) is read in.\n");
    out.push_str("    pub(crate) const fn dialects(self) -> &'static [GrammarDialect] {\n        match self {\n");
    for entry in &supported {
        let own = entry
            .dialects
            .iter()
            .map(|dialect| dialect.label.clone())
            .collect::<Vec<_>>();
        let runs_on = entry.runs_on.clone().unwrap_or(own);
        let list = runs_on
            .iter()
            .map(|label| variant(label))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "            Self::{} => &[{list}],", pascal(&entry.id));
    }
    out.push_str("        }\n    }\n\n    pub(crate) const fn family(self) -> Family {\n        match self {\n");
    for entry in &supported {
        let _ = writeln!(
            out,
            "            Self::{} => Family::{},",
            pascal(&entry.id),
            pascal(&entry.family)
        );
    }
    out.push_str("        }\n    }\n\n");
    out.push_str(
        "    /// Whether the language reads `semantic`: what its registry entry declares.\n",
    );
    out.push_str("    pub(crate) const fn capability(self, semantic: Semantic) -> Support {\n        match (self, semantic) {\n");
    for entry in &supported {
        for (capability, support) in &entry.capabilities {
            let _ = writeln!(
                out,
                "            (Self::{}, Semantic::{}) => Support::{},",
                pascal(&entry.id),
                pascal(capability),
                support.variant()
            );
        }
    }
    out.push_str("        }\n    }\n}\n\n");

    out.push_str("/// Languages the registry plans: a rule may name one, and is refused as not supported yet.\n");
    let planned = entries
        .iter()
        .filter(|entry| entry.status == Status::Planned)
        .map(|entry| format!("{:?}", entry.id))
        .collect::<Vec<_>>();
    let _ = writeln!(
        out,
        "pub(crate) const PLANNED_LANGUAGES: &[&str] = &[{}];\n",
        planned.join(", ")
    );

    let mut families: Vec<(&str, Vec<String>, Vec<&str>)> = Vec::new();
    for entry in &supported {
        let dialects = entry.dialects.iter().map(|dialect| variant(&dialect.label));
        match families
            .iter_mut()
            .find(|(family, _, _)| *family == entry.family)
        {
            Some((_, variants, entries)) => {
                variants.extend(dialects);
                entries.push(&entry.id);
            }
            None => families.push((&entry.family, dialects.collect(), vec![entry.id.as_str()])),
        }
    }
    out.push_str("/// The semantic implementation a dialect is read with.\n");
    out.push_str("#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]\n");
    out.push_str("pub(crate) enum Family {\n");
    for (family, _, entries) in &families {
        let names = entries
            .iter()
            .map(|id| format!("languages/{id}.json"))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "    /// {names}\n    {},", pascal(family));
    }
    out.push_str("}\n\n");
    out.push_str("impl Family {\n");
    out.push_str(
        "    /// The registry's families, in the order their first languages were added.\n",
    );
    let _ = writeln!(
        out,
        "    pub(crate) const ALL: [Self; {}] = [{}];\n",
        families.len(),
        families
            .iter()
            .map(|(family, _, _)| format!("Self::{}", pascal(family)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    out.push_str("    /// The family a dialect is read with: its registry entry's.\n");
    out.push_str("    pub(crate) fn of(lang: GrammarDialect) -> Self {\n        match lang {\n");
    for (family, variants, _) in &families {
        let _ = writeln!(
            out,
            "            {} => Self::{},",
            variants.join(" | "),
            pascal(family)
        );
    }
    out.push_str(
        "            #[cfg(test)]\n            _ => TEST_DIALECT_FAMILY,\n        }\n    }\n\n",
    );
    out.push_str("    /// Whether the family's files carry `suppression`'s comments: its languages' entries list it.\n");
    out.push_str("    pub(crate) const fn honours(self, suppression: Suppression) -> bool {\n");
    let mut honoured = Vec::new();
    for (family, _, _) in &families {
        let entry = supported
            .iter()
            .find(|entry| entry.family == *family)
            .expect("family entry");
        for suppression in &entry.suppressions {
            honoured.push(format!(
                "(Self::{}, Suppression::{})",
                pascal(family),
                pascal(suppression)
            ));
        }
    }
    let body = if honoured.is_empty() {
        "let _ = (self, suppression);\n        false".to_owned()
    } else {
        format!(
            "matches!(\n            (self, suppression),\n            {}\n        )",
            honoured.join("\n                | ")
        )
    };
    let _ = writeln!(out, "        {body}\n    }}\n}}\n");

    out.push_str("/// Evaluates `$body` with `$a` the adapter of `$family` (`adapter`): one arm per registry family.\n");
    out.push_str("macro_rules! with_adapter {\n    ($family:expr, $a:ident => $body:expr) => {\n        match $family {\n");
    for (family, _, _) in &families {
        let family = pascal(family);
        let _ = writeln!(
            out,
            "            $crate::code_rule_dsl::language::Family::{family} => {{\n                type $a = $crate::code_rule_dsl::language::adapter::{family};\n                $body\n            }}"
        );
    }
    out.push_str("        }\n    };\n}\n\npub(crate) use with_adapter;\n");
    out
}

#[cfg(test)]
mod tests {
    use super::generate;
    use serde_json::{Value, json};
    use std::path::{Path, PathBuf};

    /// A registry directory and an output directory, removed when dropped.
    struct Registry(PathBuf);

    impl Registry {
        fn new(name: &str, entries: &[Value]) -> Self {
            let root = std::env::temp_dir()
                .join(format!("rerail-languages-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("languages")).expect("registry directory");
            std::fs::create_dir_all(root.join("out")).expect("output directory");
            for entry in entries {
                let id = entry["id"].as_str().expect("id");
                std::fs::write(
                    root.join("languages").join(format!("{id}.json")),
                    entry.to_string(),
                )
                .expect("registry file");
            }
            Self(root)
        }

        fn generate(&self) -> String {
            generate(&self.0.join("languages"), &["one"], &self.0.join("out"));
            std::fs::read_to_string(self.0.join("out/language_registry.rs")).expect("generated")
        }
    }

    impl Drop for Registry {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn capabilities(support: &str) -> Value {
        json!({ "scope": support, "value": support, "arguments": support, "calls": support, "formatArgs": support })
    }

    fn supported(id: &str, suffix: &str, tag: u8) -> Value {
        json!({
            "format": "rerail.language/v1", "id": id, "status": "supported", "family": id,
            "dialects": [{ "label": id, "variant": "One", "suffixes": [suffix], "parser": "tree-sitter", "grammar": "vendored:one", "tapeTag": tag }],
            "capabilities": capabilities("supported"), "suppressions": []
        })
    }

    fn planned(id: &str, suffix: &str) -> Value {
        json!({
            "format": "rerail.language/v1", "id": id, "status": "planned", "family": id,
            "dialects": [{ "label": id, "suffixes": [suffix] }],
            "capabilities": capabilities("not-supported-yet"), "suppressions": []
        })
    }
    fn planned_with_tag(id: &str, suffix: &str, tag: u8) -> Value {
        let mut entry = planned(id, suffix);
        entry["dialects"][0]["tapeTag"] = json!(tag);
        entry
    }

    #[test]
    fn planned_dialects_may_reserve_tags_without_generated_dialects() {
        let generated = Registry::new(
            "planned-tag",
            &[
                supported("one", ".one", 1),
                planned_with_tag("two", ".two", 2),
            ],
        )
        .generate();
        assert!(!generated.contains("label: \"two\""));
        assert!(!generated.contains("GrammarDialect::Two"));
    }

    #[test]
    #[should_panic(
        expected = "tape tag 1 is also languages/one.json's; a tape tag is never reused"
    )]
    fn planned_dialects_cannot_reuse_supported_tags() {
        Registry::new(
            "planned-tag-collision",
            &[
                supported("one", ".one", 1),
                planned_with_tag("two", ".two", 1),
            ],
        )
        .generate();
    }

    #[test]
    #[should_panic(
        expected = "languages/two.json: tape tag 2 is also languages/three.json's; a tape tag is never reused"
    )]
    fn planned_dialects_cannot_share_tags() {
        Registry::new(
            "planned-tag-sharing",
            &[
                supported("one", ".one", 1),
                planned_with_tag("two", ".two", 2),
                planned_with_tag("three", ".three", 2),
            ],
        )
        .generate();
    }

    #[test]
    fn embeddings_generate_via_dialects_and_dispatch() {
        let mut one = supported("one", ".one", 1);
        one["embeds"] = json!([{ "language": "two", "via": "compiled-view" }]);
        let mut two = supported("two", ".two", 2);
        two["dialects"][0]["variant"] = json!("Two");
        let generated = Registry::new("embeds", &[one, two]).generate();
        assert!(generated.contains("pub(crate) enum Via {\n    CompiledView,\n"));
        assert!(generated.contains("Self::One => &[(Via::CompiledView, GrammarDialect::Two)],"));
        assert!(generated.contains("type $e = $crate::code_rule_dsl::embed::CompiledView;"));
    }

    #[test]
    fn planned_embeddings_and_empty_via_are_not_generated() {
        let generated = Registry::new(
            "empty-embeds",
            &[supported("one", ".one", 1), planned("two", ".two")],
        )
        .generate();
        assert!(generated.contains("pub(crate) enum Via {\n}\n"));
        assert!(generated.contains("match $via {\n        }\n"));
        assert!(!generated.contains("GrammarDialect::Two"));
    }

    #[test]
    fn the_repository_registry_generates_the_engine_tables() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let out = Registry::new("repository", &[]);
        generate(
            &root.join("../languages"),
            &["python", "typescript", "tsx"],
            &out.0.join("out"),
        );
        let generated =
            std::fs::read_to_string(out.0.join("out/language_registry.rs")).expect("generated");
        assert!(generated.contains(
            "pub(crate) const PLANNED_LANGUAGES: &[&str] = &[\"rust\", \"sql\", \"vue\"];"
        ));
    }

    #[test]
    fn planned_languages_generate_no_dialect() {
        let generated = Registry::new(
            "planned",
            &[supported("one", ".one", 1), planned("two", ".two")],
        )
        .generate();
        assert!(generated.contains("label: \"one\""));
        assert!(!generated.contains("label: \"two\""));
        assert!(generated.contains("&[\"two\"]"));
    }

    #[test]
    fn each_family_dispatches_to_its_adapter() {
        let mut three = supported("three", ".three", 3);
        three["family"] = json!("one");
        let generated = Registry::new(
            "adapters",
            &[
                supported("one", ".one", 1),
                supported("two", ".two", 2),
                three,
                planned("four", ".four"),
            ],
        )
        .generate();
        assert!(generated.contains("macro_rules! with_adapter {\n    ($family:expr, $a:ident => $body:expr) => {\n        match $family {\n"));
        for family in ["One", "Two"] {
            assert_eq!(
                generated
                    .matches(&format!("            $crate::code_rule_dsl::language::Family::{family} => {{\n                type $a = $crate::code_rule_dsl::language::adapter::{family};\n                $body\n            }}\n"))
                    .count(),
                1,
                "{family}"
            );
        }
        assert!(!generated.contains("adapter::Three"));
        assert!(!generated.contains("adapter::Four"));
        assert!(generated.contains("pub(crate) use with_adapter;"));
    }
    #[test]
    fn dialect_macro_generates_registry_support_rules() {
        let mut rust = supported("rust", ".rs", 1);
        rust["dialects"][0]["variant"] = json!("Rust");
        rust["dialects"][0]["grammar"] = json!("ast-grep:Rust");
        let mut one = supported("one", ".one", 2);
        one["dialects"][0]["variant"] = json!("One");
        one["dialects"][0]["grammar"] = json!("vendored:one");
        let generated = Registry::new("dialect-macro", &[rust, one]).generate();
        assert!(generated.contains("Self::Rust => Some(SupportLang::Rust),"));
        assert!(generated.contains("Self::One => None,"));
        assert!(generated.contains("macro_rules! grammar_dialect"));
    }

    #[test]
    fn every_family_is_listed_once_in_registry_order() {
        let mut three = supported("three", ".three", 3);
        three["family"] = json!("one");
        let generated = Registry::new(
            "families",
            &[
                supported("two", ".two", 2),
                three,
                supported("one", ".one", 1),
                planned("four", ".four"),
            ],
        )
        .generate();
        assert!(
            generated.contains("    pub(crate) const ALL: [Self; 2] = [Self::One, Self::Two];\n")
        );
    }

    #[test]
    #[should_panic(expected = "supported dialect one has no grammar")]
    fn a_supported_dialect_needs_a_grammar() {
        let mut entry = supported("one", ".one", 1);
        entry["dialects"][0]
            .as_object_mut()
            .expect("dialect")
            .remove("grammar");
        Registry::new("grammar", &[entry]).generate();
    }

    #[test]
    #[should_panic(expected = "suffix .one is also languages/one.json's")]
    fn suffixes_are_unique() {
        Registry::new(
            "suffixes",
            &[supported("one", ".one", 1), planned("two", ".one")],
        )
        .generate();
    }

    #[test]
    #[should_panic(expected = "tape tag 1 is also languages/one.json's")]
    fn tape_tags_are_unique() {
        let mut two = supported("two", ".two", 1);
        two["dialects"][0]["label"] = json!("two");
        Registry::new("tags", &[supported("one", ".one", 1), two]).generate();
    }

    #[test]
    #[should_panic(expected = "a planned language supports no capability yet")]
    fn a_planned_language_supports_nothing() {
        let mut entry = planned("two", ".two");
        entry["capabilities"]["scope"] = json!("supported");
        Registry::new("capability", &[entry]).generate();
    }
    #[test]
    #[should_panic(
        expected = "languages/one.json: embeds planned language two; a supported language embeds only supported ones"
    )]
    fn supported_languages_cannot_embed_planned_languages() {
        let mut one = supported("one", ".one", 1);
        one["embeds"] = json!([{ "language": "two", "via": "compiled-view" }]);
        Registry::new("planned-embed", &[one, planned("two", ".two")]).generate();
    }
}
