// Inside `src/` beside `build_identity.rs`, so every copy of the native sources carries it; only
// `build.rs` compiles it.
//! The language registry (`languages/*.json`, format `rerail.language/v1`, described in
//! `docs/language-packs.md`): read, validated, and turned into the engine's language tables.
//! `generate` writes `language_registry.rs` to `OUT_DIR`; `code_rule_dsl::language` includes it. A
//! registry the engine cannot be built from fails the build here, naming the file and the fault.
//! The TypeScript host reads the same files through `src/code-rule-language-registry.generated.ts`, which
//! `scripts/generate-language-registry.ts` writes and the host's tests keep current.

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const FORMAT: &str = "rerail.language/v1";
/// The parser implementations a dialect may name.
const PARSERS: &[&str] = &["tree-sitter"];

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
    #[serde(default)]
    tape_tag: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Embed {
    language: String,
    via: String,
}

/// A supported dialect's grammar: one `build.rs` generates from a vendored `grammar.json`, or one
/// of ast-grep's.
enum Grammar<'a> {
    Vendored(&'a str),
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
        Some(Grammar::AstGrep(_)) => {}
        None => panic!(
            "{name}: dialect {label} grammar `{grammar}` is neither vendored:<name> nor ast-grep:<Language>"
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

    out.push_str("/// A supported dialect's grammar, as the registry names it; `None` for a test-only grammar.\n");
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
                format!("unsafe {{ tree_sitter_language::LanguageFn::from_raw({symbol}) }}.into()")
            }
            Grammar::AstGrep(language) => format!("SupportLang::{language}.get_ts_language()"),
        };
        let _ = writeln!(
            arms,
            "        {} => Some({language}),",
            variant(&dialect.label)
        );
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

    let mut families: Vec<(&str, Vec<String>)> = Vec::new();
    for entry in &supported {
        let dialects = entry.dialects.iter().map(|dialect| variant(&dialect.label));
        match families
            .iter_mut()
            .find(|(family, _)| *family == entry.family)
        {
            Some((_, variants)) => variants.extend(dialects),
            None => families.push((&entry.family, dialects.collect())),
        }
    }
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
            .map(|(family, _)| format!("Self::{}", pascal(family)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    out.push_str("    /// The family a dialect is read with: its registry entry's.\n");
    out.push_str("    pub(crate) fn of(lang: GrammarDialect) -> Self {\n        match lang {\n");
    for (family, variants) in &families {
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
    for (family, _) in &families {
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
    for (family, _) in &families {
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

    #[test]
    fn the_repository_registry_generates_the_engine_tables() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let out = Registry::new("repository", &[]);
        generate(
            &root.join("../languages"),
            &["blade", "python", "typescript", "tsx"],
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
}
