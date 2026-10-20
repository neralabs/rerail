//! Randomly generated rules over the in-scope operators, evaluated by both engines over fixture
//! sources in Python, TypeScript, PHP and Rust: every case's findings (roots, captures, messages)
//! and failures must be identical. The semantic leaves (`resolves`, `resolves.binding`, `binds`,
//! `arguments`, `parse` with `capture` and `host`, `formatArgs`) are generated over Python,
//! TypeScript and PHP, and `calls` over Python, PHP and Rust.
//!
//! The generator is deterministic: `RERAIL_SQL_DIFF_SEED` (default `0x5e7_c0de`) seeds it and
//! `RERAIL_SQL_DIFF_CASES` (default 500) sets how many compiled cases are compared. Rules the
//! front end rejects (an invalid pattern or kind, an order conflict) are generated but not
//! counted. `RERAIL_SQL_DIFF_KEEP` names a file every differing case is appended to (its index
//! among the compared cases, rule and violation; one JSON line each), which
//! `replayed_cases_agree` replays.

use super::snapshot;
use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::language::{GrammarDialect, dialect_by_label};
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::matcher::{UtilLimit, node_at};
use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
use crate::code_rule_dsl::test_support::{CAPABILITIES, clause_document, compile_document_with};
use ast_grep_core::AstGrep;
use rayon::prelude::*;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

/// SplitMix64: small, fast and fixed, so a seed names the same cases everywhere.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound.max(1) as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// A short source fragment a pattern is built from: its text, the ranges of its identifier-like
/// leaves and of its argument list's contents.
struct Snippet {
    text: String,
    leaves: Vec<Range<usize>>,
    arguments: Option<Range<usize>>,
}

/// What rules over one language's fixtures are built from.
struct Vocabulary {
    label: &'static str,
    files: Vec<(String, String)>,
    kinds: Vec<String>,
    fields: Vec<String>,
    snippets: Vec<Snippet>,
    regexes: Vec<String>,
}

const FIXTURES: &[(&str, &[&str])] = &[
    (
        "python",
        &[
            "python/scope/usage/F841.py",
            "python/value/ruff/SIM116.py",
            "python/calls/project/pkg/models.py",
        ],
    ),
    (
        "typescript",
        &[
            "ecma/scope/typescript/bindings.ts",
            "ecma/scope/typescript/constants.ts",
            "ecma/value/eslint/no-dupe-keys.ts",
        ],
    ),
    (
        "php",
        &[
            "php/calls/php/trace.php",
            "php/calls/php/project/more.php",
            "php/scope/php/unanalysed.php",
        ],
    ),
    (
        "rust",
        &[
            "rust/calls/crate/src/lib.rs",
            "rust/calls/crate/src/util.rs",
            "rust/calls/crate/src/net/tcp.rs",
        ],
    ),
];

pub(super) fn tape(dialect: GrammarDialect, source: &str) -> TapeDoc {
    let never = || false;
    let grammar = Grammar::new(dialect).expect("grammar");
    let image = TapeImage::parse(&grammar, source, &never)
        .expect("parsed")
        .expect("tape");
    let lines = Arc::new(LineTable::new(source, Terminators::of(dialect)));
    TapeDoc::restore_with_lines(Arc::new(source.to_owned()), grammar, &image, lines, &never)
        .expect("restored")
}

const LEAF_KINDS: &[&str] = &[
    "identifier",
    "property_identifier",
    "shorthand_property_identifier",
    "type_identifier",
    "variable_name",
    "name",
    "integer",
    "number",
    "true",
    "false",
    "none",
];

const ARGUMENT_KINDS: &[&str] = &[
    "arguments",
    "argument_list",
    "formal_parameters",
    "parameters",
    "array",
    "list",
];

impl Vocabulary {
    fn of(label: &'static str, paths: &[&str]) -> Self {
        let dialect = dialect_by_label(label).expect("dialect");
        let catalog = dialect.kinds();
        let mut kinds = BTreeSet::new();
        let mut fields = BTreeSet::new();
        let mut snippets = Vec::new();
        let mut words = BTreeSet::new();
        let mut files = Vec::new();
        for path in paths {
            let source = std::fs::read_to_string(format!(
                "{}/{path}",
                crate::code_rule_dsl::fixtures_dir!()
            ))
            .unwrap_or_else(|error| panic!("{path}: {error}"));
            let doc = tape(dialect, &source);
            let root = AstGrep::doc(doc.clone());
            for id in 0..doc.transducer_len() as u32 {
                let node = node_at(&root, id);
                if node.is_named() && !node.is_error() {
                    kinds.insert(node.kind().into_owned());
                }
                let field = doc.transducer_field(id);
                if field != 0
                    && let Some(name) = catalog.field_name(field)
                {
                    fields.insert(name.to_owned());
                }
                let text = node.text();
                if node.is_named()
                    && LEAF_KINDS.contains(&node.kind().as_ref())
                    && text.len() <= 12
                    && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    words.insert(text.to_string());
                }
                let start = node.range().start;
                if node.is_named()
                    && !node.is_error()
                    && (3..=48).contains(&text.len())
                    && !text.contains('\n')
                    && !text.contains('$')
                {
                    let mut leaves = Vec::new();
                    let mut arguments = None;
                    for inner in node.dfs().skip(1) {
                        let range = inner.range();
                        let relative = range.start - start..range.end - start;
                        if inner.is_named()
                            && inner.is_named_leaf()
                            && LEAF_KINDS.contains(&inner.kind().as_ref())
                        {
                            leaves.push(relative.clone());
                        }
                        if arguments.is_none()
                            && ARGUMENT_KINDS.contains(&inner.kind().as_ref())
                            && relative.len() >= 2
                        {
                            arguments = Some(relative.start + 1..relative.end - 1);
                        }
                    }
                    if !leaves.is_empty() || arguments.is_some() {
                        snippets.push(Snippet {
                            text: text.to_string(),
                            leaves,
                            arguments,
                        });
                    }
                }
            }
            files.push((path.to_string(), source));
        }
        let mut regexes: Vec<String> = words
            .iter()
            .step_by(3)
            .take(24)
            .map(|word| format!("^{word}"))
            .collect();
        regexes.extend(
            [
                "^[a-z]", "[0-9]", "^_", "\\(", "^.$", "^\"", "[A-Z]", "\\.", "^$", "=", ",",
            ]
            .map(str::to_owned),
        );
        // PHP sources hold `$`-names: a metavariable written there is a variable, not a capture.
        if label == "php" {
            snippets.retain(|snippet| !snippet.text.contains("$"));
        }
        Self {
            label,
            files,
            kinds: kinds.into_iter().collect(),
            fields: fields.into_iter().collect(),
            snippets,
            regexes,
        }
    }
}

const SINGLE: &[&str] = &["A", "B", "C"];
const LISTS: &[&str] = &["L", "M"];

struct Generator<'v> {
    rng: Rng,
    vocabulary: &'v Vocabulary,
    /// Parameterized utilities and their parameters, as generated for the case.
    calls: Vec<(String, Vec<String>)>,
    plain: Vec<String>,
    /// Captures a parameterized utility's rule may use: its parameters.
    params: Option<Vec<String>>,
    counted: bool,
    /// Whether the case may hold three-valued leaves (`value`, `testUnknown`).
    three: bool,
    /// Whether the case's rule reaches a recursive utility.
    recursive: bool,
}

/// The relations a recursive utility recurses through, one direction per cycle.
const DIRECTIONS: &[&str] = &["has", "inside", "precedes", "follows"];

impl Generator<'_> {
    fn single(&mut self) -> String {
        match &self.params {
            Some(params) if !params.is_empty() => self.rng.pick(params).clone(),
            _ => (*self.rng.pick(SINGLE)).to_owned(),
        }
    }

    fn pattern(&mut self) -> String {
        let snippet = self.rng.pick(&self.vocabulary.snippets);
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        if let Some(arguments) = &snippet.arguments
            && self.rng.chance(40)
        {
            let list = if self.rng.chance(30) {
                "$$$".to_owned()
            } else {
                format!("$$${}", self.rng.pick(LISTS))
            };
            edits.push((arguments.clone(), list));
        }
        for leaf in &snippet.leaves {
            if edits
                .iter()
                .any(|(range, _)| range.start < leaf.end && leaf.start < range.end)
            {
                continue;
            }
            if self.rng.chance(45) {
                let name = if self.rng.chance(10) {
                    "_".to_owned()
                } else {
                    self.single()
                };
                edits.push((leaf.clone(), format!("${name}")));
            }
        }
        edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
        let mut text = snippet.text.clone();
        for (range, replacement) in edits {
            text.replace_range(range, &replacement);
        }
        if self.rng.chance(8) {
            format!("${}", self.single())
        } else {
            text
        }
    }

    /// A pattern binding each of `names` at a leaf of one snippet, in order, where some snippet
    /// has enough leaves.
    fn pattern_binding(&mut self, names: &[String]) -> Option<String> {
        let snippets: Vec<&Snippet> = self
            .vocabulary
            .snippets
            .iter()
            .filter(|snippet| snippet.leaves.len() >= names.len())
            .collect();
        if snippets.is_empty() {
            return None;
        }
        let snippet = snippets[self.rng.below(snippets.len())];
        let mut edits: Vec<(&Range<usize>, &String)> = snippet.leaves.iter().zip(names).collect();
        edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
        let mut text = snippet.text.clone();
        for (range, name) in edits {
            text.replace_range(range.clone(), &format!("${name}"));
        }
        Some(text)
    }

    /// A probe that reads a capture bound before it: `$X` binds the node, and `not` asks for a
    /// related node with its text (with `X` bound it is a test, not a fresh binding).
    fn bound_probe(&mut self, rule: &mut Map<String, Value>) {
        let name = self.single();
        let key = *self.rng.pick(&["inside", "has", "precedes", "follows"]);
        let stop = *self.rng.pick(&["end", "neighbor"]);
        rule.insert("pattern".into(), json!(format!("${name}")));
        rule.insert(
            "not".into(),
            json!({ key: { "stopBy": stop, "pattern": format!("${name}") } }),
        );
    }

    /// Three-valued: an unknown term that may bind `X`, then a term on `X`, under `not` or `any`
    /// (the second's verdict is withheld where the first may have bound `X`).
    fn withheld(&mut self, rule: &mut Map<String, Value>, depth: usize) {
        let name = format!("${}", self.single());
        let term = |generator: &mut Self, last: Value| {
            let key = *generator
                .rng
                .pick(&["inside", "has", "precedes", "follows"]);
            let stop = *generator.rng.pick(&["end", "neighbor"]);
            json!({ key: { "stopBy": stop, "all": [{ "pattern": name }, last] } })
        };
        let unknown = if self.rng.chance(50) {
            json!({ "testUnknown": "." })
        } else {
            json!({ "testUnknown": self.rng.pick(&self.vocabulary.regexes) })
        };
        let first = term(self, unknown);
        let test = self.simple(depth + 2);
        let second = term(self, test);
        let all = json!({ "all": [first, second] });
        if self.rng.chance(50) {
            rule.insert("not".into(), all);
        } else {
            let other = self.rule(depth + 1);
            rule.insert("any".into(), json!([all, other]));
        }
    }

    fn stop(&mut self, depth: usize) -> Value {
        match self.rng.below(10) {
            0..=3 => json!("end"),
            4..=6 => json!("neighbor"),
            _ => self.simple(depth + 1),
        }
    }

    /// A small capture-free rule (stop rules, `ofRule`); in a three-valued case it may be unknown.
    fn simple(&mut self, _depth: usize) -> Value {
        if self.three && self.rng.chance(35) {
            let mut rule = Map::new();
            if self.rng.chance(60) {
                rule.insert("kind".into(), json!(self.rng.pick(&self.vocabulary.kinds)));
            }
            if self.rng.chance(60) {
                rule.insert(
                    "testUnknown".into(),
                    json!(self.rng.pick(&self.vocabulary.regexes)),
                );
            } else {
                rule.insert("value".into(), self.value());
            }
            return Value::Object(rule);
        }
        match self.rng.below(4) {
            0 | 1 => json!({ "kind": self.rng.pick(&self.vocabulary.kinds) }),
            2 => json!({ "regex": self.rng.pick(&self.vocabulary.regexes) }),
            _ => json!({ "any": [
                { "kind": self.rng.pick(&self.vocabulary.kinds) },
                { "kind": self.rng.pick(&self.vocabulary.kinds) },
            ] }),
        }
    }

    fn relation(&mut self, key: &str, depth: usize) -> Value {
        let mut inner = self.rule(depth + 1);
        if self.rng.chance(70) {
            inner["stopBy"] = self.stop(depth);
        }
        if (key == "inside" || key == "has")
            && self.rng.chance(15)
            && !self.vocabulary.fields.is_empty()
        {
            inner["field"] = json!(self.rng.pick(&self.vocabulary.fields));
        }
        inner
    }

    /// A `value` test: over names and calls most values are not known, so these are often
    /// unknown; `follow` reads names through their bindings.
    fn value(&mut self) -> Value {
        let mut test = Map::new();
        match self.rng.below(8) {
            0 => {
                test.insert("truthy".into(), json!(self.rng.chance(50)));
            }
            1 => {
                let eq = match self.rng.below(4) {
                    0 => json!(self.rng.below(3)),
                    1 => json!(null),
                    2 => json!(self.rng.chance(50)),
                    _ => json!(
                        self.rng
                            .pick(&self.vocabulary.regexes)
                            .trim_start_matches('^')
                    ),
                };
                test.insert("eq".into(), eq);
            }
            2 => {
                test.insert("constant".into(), json!(self.rng.chance(50)));
            }
            3 => {
                let bound = *self.rng.pick(&["gt", "ge", "lt", "le"]);
                test.insert(bound.into(), json!(self.rng.below(4)));
            }
            4 => {
                test.insert(
                    "startsWith".into(),
                    json!(self.rng.pick(&["a", "_", "s", "f"])),
                );
            }
            5 => {
                test.insert(
                    "regex".into(),
                    json!(self.rng.pick(&["^[a-z]", "[0-9]", "^$"])),
                );
            }
            6 => {
                let types: &[&str] = match self.vocabulary.label {
                    "python" => &["str", "int", "bool", "NoneType", "list"],
                    "php" => &["string", "int", "bool", "array", "null"],
                    _ => &["string", "number", "boolean", "undefined", "object"],
                };
                test.insert("type".into(), json!(self.rng.pick(types)));
            }
            _ => {
                test.insert("bitsAny".into(), json!(1 + self.rng.below(3)));
            }
        }
        if self.rng.chance(40) {
            test.insert("follow".into(), json!(true));
        }
        Value::Object(test)
    }

    /// A `compare` of two captures, in any text mode and collation (the front end refuses the
    /// combinations a mode does not take).
    fn compare(&mut self) -> Value {
        let modes = [
            "source",
            "unquoted",
            "tokens",
            "node",
            "index",
            "value",
            "key",
            "name",
            "member",
            "reference",
            "structure",
        ];
        let text = *self.rng.pick(&modes);
        let exact = matches!(text, "tokens" | "node" | "reference" | "structure");
        let op = if exact {
            *self.rng.pick(&["eq", "ne"])
        } else {
            *self.rng.pick(&["eq", "ne", "lt", "le", "gt", "ge"])
        };
        let mut compare = Map::new();
        compare.insert("left".into(), json!(self.single()));
        compare.insert("op".into(), json!(op));
        compare.insert("right".into(), json!(self.single()));
        compare.insert("text".into(), json!(text));
        if !exact && text != "index" && self.rng.chance(50) {
            let collation = *self.rng.pick(&[
                "ordinal",
                "fold-lower",
                "fold-upper",
                "lowercase-first",
                "natural",
                "natural-fold-lower",
            ]);
            compare.insert("collation".into(), json!(collation));
        }
        Value::Object(compare)
    }

    /// A `layout` test: the node's lines and start, or a gap to a token, sibling or capture.
    fn layout(&mut self) -> Value {
        let lines = |rng: &mut Rng| match rng.below(3) {
            0 => json!({ "eq": rng.below(2) }),
            1 => json!({ "max": rng.below(2) }),
            _ => json!({ "min": 1 }),
        };
        match self.rng.below(4) {
            0 => json!({ "span": lines(&mut self.rng) }),
            1 => json!({ "start": { "max": self.rng.below(400) } }),
            _ => {
                let to = match self.rng.below(3) {
                    0 => "token".to_owned(),
                    1 => "sibling".to_owned(),
                    _ => format!("capture.{}", self.single()),
                };
                let mut gap = Map::new();
                gap.insert("to".into(), json!(to));
                gap.insert("breaks".into(), lines(&mut self.rng));
                if self.rng.chance(40) {
                    gap.insert("blank".into(), json!({ "max": self.rng.below(2) }));
                }
                if self.rng.chance(30) {
                    gap.insert("edge".into(), json!(true));
                }
                if self.rng.chance(30) {
                    gap.insert("comments".into(), json!(true));
                }
                let side = *self.rng.pick(&["before", "after"]);
                json!({ side: gap })
            }
        }
    }

    /// A sub-rule of a semantic leaf (a binding's nodes, a search's candidates): it may bind
    /// captures, and in a three-valued case be unknown.
    fn sub_rule(&mut self, depth: usize) -> Value {
        if self.rng.chance(30) {
            self.simple(depth + 2)
        } else {
            self.rule(depth + 2)
        }
    }

    /// Binding filters, as `binds`, `resolves.binding` and `redefines` read them.
    fn filters(&mut self, test: &mut Map<String, Value>) {
        if self.rng.chance(40) {
            let forms: &[&str] = match self.vocabulary.label {
                "python" => &[
                    "assignment",
                    "import",
                    "definition",
                    "parameter",
                    "for",
                    "augmented",
                ],
                "php" => &["assignment", "parameter", "for", "global", "static"],
                _ => &["assignment", "import", "definition", "parameter", "for"],
            };
            test.insert("form".into(), json!(self.rng.pick(forms)));
        }
        for key in ["used", "referenced", "latest", "exported"] {
            if self.rng.chance(12) {
                test.insert(key.into(), json!(self.rng.chance(50)));
            }
        }
    }

    /// A binding's sub-rules: its site, value and annotation, each now and then.
    fn binding_nodes(&mut self, test: &mut Map<String, Value>, depth: usize) {
        for key in ["site", "value", "annotation"] {
            if self.rng.chance(35) {
                test.insert(key.into(), self.sub_rule(depth));
            }
        }
    }

    /// A semantic leaf the language reads, added to `rule` (one that needs a key the rule already
    /// holds is left out).
    fn leaf(&mut self, rule: &mut Map<String, Value>, depth: usize) {
        let label = self.vocabulary.label;
        let calls = matches!(label, "python" | "php" | "rust");
        let leaves: &[&str] = match label {
            "rust" => &["calls"],
            "typescript" => &["resolves", "binding", "binds", "arguments", "parse"],
            _ => &[
                "resolves",
                "binding",
                "binds",
                "arguments",
                "parse",
                "formatArgs",
                "calls",
            ],
        };
        let name = *self.rng.pick(leaves);
        match name {
            "resolves" => {
                let test = match self.rng.below(4) {
                    0 => json!({ "global": true }),
                    1 => json!({ "unresolved": self.rng.chance(70) }),
                    2 => match label {
                        "python" => json!({ "import": ["os", "typing", "pkg.util", "time.sleep"] }),
                        "php" => json!({ "global": false }),
                        _ => json!({ "import": ["fs", "react", "./a"] }),
                    },
                    _ => json!({ "this": true }),
                };
                rule.insert("resolves".into(), test);
            }
            "binding" => {
                let mut binding = Map::new();
                self.filters(&mut binding);
                self.binding_nodes(&mut binding, depth);
                if label != "python" && self.rng.chance(30) {
                    binding.insert("occurrence".into(), self.sub_rule(depth));
                }
                rule.insert("resolves".into(), json!({ "binding": binding }));
            }
            "binds" => {
                let mut binds = Map::new();
                self.filters(&mut binds);
                self.binding_nodes(&mut binds, depth);
                if label != "python" && self.rng.chance(25) {
                    binds.insert("occurrence".into(), self.sub_rule(depth));
                }
                if self.rng.chance(40) {
                    let mut redefines = Map::new();
                    self.filters(&mut redefines);
                    self.binding_nodes(&mut redefines, depth);
                    if label != "python" && self.rng.chance(35) {
                        redefines.insert("every".into(), json!(true));
                    }
                    if self.rng.chance(20) {
                        redefines.insert("sameScope".into(), json!(self.rng.chance(50)));
                    }
                    binds.insert("redefines".into(), Value::Object(redefines));
                }
                rule.insert("binds".into(), Value::Object(binds));
            }
            "arguments" => {
                let signature: Vec<&str> = match (label, self.rng.below(3)) {
                    ("python", 0) => vec!["a", "/", "b", "*", "c"],
                    ("python", 1) => vec!["a", "b", "*", "c"],
                    _ => vec!["a", "b", "c"],
                };
                let slot = *self.rng.pick(&["a", "b", "c"]);
                let mut test = Map::new();
                test.insert("signature".into(), json!(signature));
                if self.rng.chance(60) {
                    test.insert("bind".into(), json!({ self.single(): slot }));
                } else {
                    test.insert("absent".into(), json!([slot]));
                }
                if label == "typescript" && self.rng.chance(50) {
                    test.insert("mode".into(), json!(self.rng.pick(&["estree", "language"])));
                }
                rule.insert("arguments".into(), Value::Object(test));
            }
            "parse" => {
                let (grammar, kinds, host): (&str, &[&str], Value) = match label {
                    "python" => match self.rng.below(3) {
                        0 => (
                            "percentFormat",
                            &["placeholder", "mapping_key", "conversion"],
                            json!({ "inside": { "kind": "call", "stopBy": "end" } }),
                        ),
                        1 => (
                            "braceFormat",
                            &["field", "field_name", "format_spec"],
                            json!({ "inside": { "kind": "call", "stopBy": "end" } }),
                        ),
                        _ => ("annotation", &["identifier", "subscript"], Value::Null),
                    },
                    "php" => (
                        "printfFormat",
                        &["placeholder", "literal"],
                        json!({ "inside": { "kind": "function_call_expression", "stopBy": "end" } }),
                    ),
                    _ => (
                        "eslintDirective",
                        &["item"],
                        json!({ "inside": { "kind": "program", "has": { "stopBy": "end",
                            "kind": "variable_declarator", "has": { "field": "name", "pattern": "$V" },
                            "compare": { "left": "V", "op": "eq", "right": "K", "text": "name" } } } }),
                    ),
                };
                let mut inner = Map::new();
                inner.insert("kind".into(), json!(self.rng.pick(kinds)));
                if self.rng.chance(60) {
                    inner.insert("stopBy".into(), json!("end"));
                }
                let captured = self.rng.chance(60);
                if captured {
                    inner.insert("capture".into(), json!("K"));
                }
                if captured && !host.is_null() && self.rng.chance(35) {
                    let host = if self.rng.chance(30) {
                        json!({ "not": host })
                    } else {
                        host
                    };
                    inner.insert("host".into(), host);
                }
                if self.rng.chance(25) {
                    inner.insert(
                        "regex".into(),
                        json!(self.rng.pick(&["^[a-z]", "s", "^\\{", "[0-9]"])),
                    );
                }
                let mut parse = Map::new();
                parse.insert("grammar".into(), json!(grammar));
                parse.insert("has".into(), Value::Object(inner));
                let literal = match label {
                    "python" => json!([{ "kind": "string" }, { "kind": "concatenated_string" }]),
                    "php" => json!([{ "kind": "string" }, { "kind": "encapsed_string" }]),
                    _ => json!([{ "kind": "comment" }]),
                };
                if !rule.contains_key("any") {
                    rule.insert("any".into(), literal);
                }
                rule.insert("parse".into(), Value::Object(parse));
            }
            "formatArgs" if !rule.contains_key("pattern") => {
                let (pattern, style, args) = match (label, self.rng.below(2)) {
                    ("python", 0) => ("$S.format($$$L)", "brace", "L"),
                    ("python", _) => ("$S % $R", "percent", "R"),
                    _ => ("sprintf($S, $$$L)", "printf", "L"),
                };
                let mut test = Map::new();
                test.insert("style".into(), json!(style));
                test.insert("string".into(), json!("S"));
                test.insert("args".into(), json!(args));
                let field = match style {
                    "brace" => "field",
                    _ => "placeholder",
                };
                let side = self.rng.below(3);
                if side != 1 {
                    let unused = if self.rng.chance(50) {
                        json!({ "pattern": "$U" })
                    } else {
                        self.sub_rule(depth)
                    };
                    test.insert("unused".into(), unused);
                }
                if side != 0 {
                    let mut missing = Map::new();
                    missing.insert("kind".into(), json!(field));
                    if self.rng.chance(50) {
                        missing.insert("capture".into(), json!("F"));
                    }
                    test.insert("missing".into(), Value::Object(missing));
                }
                rule.insert("pattern".into(), json!(pattern));
                rule.insert("formatArgs".into(), Value::Object(test));
            }
            "calls" if calls => {
                let inner = match label {
                    "python" => json!({ "kind": "function_definition",
                        "has": { "stopBy": { "kind": "function_definition" }, "kind": "call",
                            "regex": self.rng.pick(&self.vocabulary.regexes) } }),
                    "php" => {
                        json!({ "any": [{ "kind": "function_definition" }, { "kind": "method_declaration" }],
                        "regex": self.rng.pick(&self.vocabulary.regexes) })
                    }
                    _ => {
                        json!({ "kind": "function_item", "regex": self.rng.pick(&self.vocabulary.regexes) })
                    }
                };
                let mut test = inner.as_object().cloned().unwrap_or_default();
                test.insert("through".into(), json!(self.rng.pick(&["sync", "any"])));
                if self.rng.chance(40) {
                    test.insert("chain".into(), json!("CH"));
                }
                rule.insert("calls".into(), Value::Object(test));
            }
            _ => {}
        }
    }

    fn rule(&mut self, depth: usize) -> Value {
        let mut rule = Map::new();
        let terms = 1 + self.rng.below(if depth == 0 { 3 } else { 2 });
        for _ in 0..terms {
            let deep = depth < 3;
            // A semantic leaf now and then, near the root.
            if depth < 2 && self.rng.chance(18) {
                self.leaf(&mut rule, depth);
                continue;
            }
            // A shape the plain draw rarely reaches, now and then near the root: a `not` probe
            // reading a capture bound before it; three-valued, an unknown term that may bind
            // ahead of a term on its capture.
            if depth < 2 && self.rng.chance(5) {
                if self.three && self.rng.chance(50) {
                    self.withheld(&mut rule, depth);
                } else {
                    self.bound_probe(&mut rule);
                }
                continue;
            }
            let base = if deep { 22 } else { 8 };
            // `compare` and `layout` everywhere; `value` and `testUnknown` in three-valued cases.
            let extra = if self.three { 6 } else { 2 };
            let choice = self.rng.below(base + extra);
            let choice = if choice >= base {
                100 + choice - base
            } else {
                choice
            };
            match choice {
                100 => {
                    if !rule.contains_key("pattern") {
                        rule.insert("pattern".into(), json!(self.pattern()));
                    }
                    rule.insert("compare".into(), self.compare());
                }
                101 => {
                    rule.insert("layout".into(), self.layout());
                }
                102 | 103 => {
                    rule.insert(
                        "testUnknown".into(),
                        json!(self.rng.pick(&self.vocabulary.regexes)),
                    );
                }
                _ if choice >= 104 => {
                    rule.insert("value".into(), self.value());
                }
                0..=2 => {
                    let pattern = self.pattern();
                    rule.insert("pattern".into(), json!(pattern));
                }
                3..=5 => {
                    rule.insert("kind".into(), json!(self.rng.pick(&self.vocabulary.kinds)));
                }
                6 => {
                    rule.insert(
                        "regex".into(),
                        json!(self.rng.pick(&self.vocabulary.regexes)),
                    );
                }
                7 => {
                    let position = match self.rng.below(5) {
                        0 => json!(1 + self.rng.below(3)),
                        1 => json!("2n+1"),
                        2 => json!("-n+2"),
                        3 => json!({ "position": 1 + self.rng.below(2), "reverse": true }),
                        _ => {
                            let of_rule = if self.rng.chance(50) {
                                self.simple(depth + 1)
                            } else {
                                self.rule(depth + 2)
                            };
                            json!({ "position": self.rng.pick(&["1", "2", "n+1", "2n"]), "ofRule": of_rule, "reverse": self.rng.chance(30) })
                        }
                    };
                    rule.insert("nthChild".into(), position);
                }
                8 => {
                    rule.insert(
                        "all".into(),
                        json!([self.rule(depth + 1), self.rule(depth + 1)]),
                    );
                }
                9 | 10 => {
                    let branches = 2 + self.rng.below(2);
                    let any: Vec<Value> = (0..branches).map(|_| self.rule(depth + 1)).collect();
                    rule.insert("any".into(), json!(any));
                }
                11 => {
                    rule.insert("not".into(), self.rule(depth + 1));
                }
                12 => {
                    if !self.calls.is_empty() && self.rng.chance(50) {
                        let (name, params) = self.rng.pick(&self.calls).clone();
                        // Now and then every parameter on one capture: the call's bindings of
                        // them must agree.
                        let shared =
                            (params.len() > 1 && self.rng.chance(40)).then(|| self.single());
                        let args: Map<String, Value> = params
                            .iter()
                            .map(|param| {
                                let capture = shared.clone().unwrap_or_else(|| self.single());
                                (param.clone(), json!(format!("${capture}")))
                            })
                            .collect();
                        rule.insert("matches".into(), json!({ name: args }));
                    } else if !self.plain.is_empty() {
                        rule.insert("matches".into(), json!(self.rng.pick(&self.plain)));
                    }
                }
                13..=18 => {
                    let key = *self.rng.pick(&["inside", "has", "precedes", "follows"]);
                    let relation = self.relation(key, depth);
                    rule.insert(key.into(), relation);
                }
                _ => {
                    let key = *self.rng.pick(&["inside", "has", "precedes", "follows"]);
                    let mut relation = self.rule(depth + 2);
                    if self.rng.chance(70) {
                        relation["stopBy"] = self.stop(depth);
                    }
                    let mut count = Map::new();
                    count.insert(key.into(), relation);
                    let min = self.rng.below(3);
                    if min > 0 {
                        count.insert("min".into(), json!(min));
                    }
                    if min == 0 || self.rng.chance(40) {
                        count.insert("max".into(), json!(min + self.rng.below(3)));
                    }
                    if self.params.is_none() && self.rng.chance(30) {
                        count.insert("as".into(), json!("N"));
                        count.insert("offset".into(), json!(self.rng.below(3) as i64 - 1));
                        self.counted = true;
                    }
                    rule.insert("count".into(), Value::Object(count));
                }
            }
        }
        if rule.is_empty() {
            rule.insert("kind".into(), json!(self.rng.pick(&self.vocabulary.kinds)));
        }
        Value::Object(rule)
    }

    /// A recursive step: `direction`'s relation to a node `reference` matches, perhaps under a
    /// stop, a field or an extra term, perhaps negated or counted.
    fn recursive_step(&mut self, direction: &str, reference: Value) -> Value {
        let mut step = Map::new();
        step.insert("matches".into(), reference);
        if self.rng.chance(30) {
            step.insert("kind".into(), json!(self.rng.pick(&self.vocabulary.kinds)));
        }
        match self.rng.below(10) {
            0..=3 => {
                step.insert("stopBy".into(), json!("end"));
            }
            4..=5 => {
                step.insert("stopBy".into(), json!("neighbor"));
            }
            _ => {}
        }
        if (direction == "has" || direction == "inside")
            && self.rng.chance(15)
            && !self.vocabulary.fields.is_empty()
        {
            step.insert(
                "field".into(),
                json!(self.rng.pick(&self.vocabulary.fields)),
            );
        }
        let relation = json!({ direction: Value::Object(step.clone()) });
        match self.rng.below(10) {
            0 => json!({ "not": relation }),
            1 => {
                let mut count = Map::new();
                count.insert(direction.into(), Value::Object(step));
                count.insert("min".into(), json!(1 + self.rng.below(2)));
                json!({ "count": count })
            }
            _ => relation,
        }
    }

    /// A utility that recurses through relations in one direction (now and then two, whose reads
    /// may loop back): its own rule, or a call's with parameters.
    fn recursive_util(&mut self, params: Option<Vec<String>>, reference: Value) -> Value {
        let direction = *self.rng.pick(DIRECTIONS);
        self.params = params.clone();
        let mut base = self.rule(2);
        if let Some(params) = &params
            && self.rng.chance(70)
        {
            base["pattern"] = json!(format!("${}", params[0]));
        }
        let mut branches = vec![base, self.recursive_step(direction, reference.clone())];
        if self.rng.chance(6) {
            let other = *self.rng.pick(DIRECTIONS);
            branches.push(self.recursive_step(other, reference));
        }
        let body = match self.rng.below(10) {
            0 | 1 => {
                // A conjunction: the recursion holds only beside another term.
                let mut all = Map::new();
                all.insert("all".into(), json!(branches));
                Value::Object(all)
            }
            _ => json!({ "any": branches }),
        };
        self.params = None;
        match params {
            Some(params) => json!({ "params": params, "rule": body }),
            None => body,
        }
    }

    /// One case: an `ast` query (rule, utilities, constraints) and its violation.
    fn case(&mut self) -> (Value, Value) {
        self.calls.clear();
        self.plain.clear();
        self.counted = false;
        self.three = self.rng.chance(50);
        self.recursive = false;
        let mut utils = Map::new();
        let mut recursive = None;
        if self.rng.chance(35) {
            match self.rng.below(4) {
                0 => {
                    // A parameterized utility calling itself.
                    let params = vec!["P".to_owned()];
                    let reference = json!({ "r0": { "P": "$P" } });
                    let util = self.recursive_util(Some(params.clone()), reference);
                    utils.insert("r0".into(), util);
                    self.calls.push(("r0".into(), params));
                    recursive = Some(json!({ "r0": { "P": format!("${}", self.single()) } }));
                }
                1 => {
                    // Two utilities: `r1` meets `r0` at its own node, `r0` reaches `r1` through
                    // a relation.
                    let util = self.recursive_util(None, json!("r1"));
                    utils.insert("r0".into(), util);
                    let base = self.rule(2);
                    utils.insert(
                        "r1".into(),
                        json!({ "any": [base, { "matches": "r0", "kind": self.rng.pick(&self.vocabulary.kinds) }] }),
                    );
                    self.plain.push("r0".into());
                    recursive = Some(json!("r0"));
                }
                _ => {
                    let util = self.recursive_util(None, json!("r0"));
                    utils.insert("r0".into(), util);
                    self.plain.push("r0".into());
                    recursive = Some(json!("r0"));
                }
            }
        }
        for index in 0..self.rng.below(3) {
            let name = format!("u{index}");
            if self.rng.chance(50) {
                let params: Vec<String> = if self.rng.chance(70) {
                    vec!["P".into()]
                } else {
                    vec!["P".into(), "Q".into()]
                };
                self.params = Some(params.clone());
                let mut rule = self.rule(2);
                // Now and then a pattern binding every parameter, which a call mapping them to
                // one capture binds to different texts.
                let every = if params.len() > 1 && self.rng.chance(50) {
                    self.pattern_binding(&params)
                } else {
                    None
                };
                if let Some(every) = every {
                    rule["pattern"] = json!(every);
                } else if self.rng.chance(70) {
                    rule["pattern"] = json!(format!("${}", params[0]));
                }
                self.params = None;
                utils.insert(name.clone(), json!({ "params": params, "rule": rule }));
                self.calls.push((name, params));
            } else {
                let rule = self.rule(2);
                utils.insert(name.clone(), rule);
                self.plain.push(name);
            }
        }
        let mut ast = Map::new();
        ast.insert("language".into(), json!(self.vocabulary.label));
        let mut rule = self.rule(0);
        if let Some(reference) = recursive
            && self.rng.chance(85)
        {
            rule["matches"] = reference;
        }
        // Now and then the rule is a call mapping two parameters to one capture, of a utility
        // whose pattern binds them at two leaves: where their texts differ, the call fails.
        if self.rng.chance(5)
            && let Some(both) = self.pattern_binding(&["P".to_owned(), "Q".to_owned()])
        {
            let capture = format!("${}", self.single());
            utils.insert(
                "w0".into(),
                json!({ "params": ["P", "Q"], "rule": { "pattern": both } }),
            );
            rule = json!({ "matches": { "w0": { "P": capture, "Q": capture } } });
        }
        // Now and then a utility sharing the rule's scope, reached twice (one unit), comparing
        // its own capture against the rule's: it reads a capture it does not hold.
        if self.rng.chance(8) {
            let caller = self.single();
            let own = SINGLE
                .iter()
                .find(|name| **name != caller)
                .expect("two names");
            let op = *self.rng.pick(&["eq", "ne"]);
            let mut reader = json!({
                "pattern": format!("${own}"),
                "compare": { "left": own, "op": op, "right": caller, "text": "node" },
            });
            if self.rng.chance(50) {
                reader["kind"] = json!(self.rng.pick(&self.vocabulary.kinds));
            }
            utils.insert("v0".into(), reader);
            if !rule["pattern"]
                .as_str()
                .is_some_and(|pattern| pattern.contains(&format!("${caller}")))
            {
                rule["pattern"] = json!(format!("${caller}"));
            }
            let keys = ["inside", "has", "precedes", "follows"];
            let (first, second) = (*self.rng.pick(&keys), *self.rng.pick(&keys));
            rule["all"] = json!([
                { first: { "stopBy": "end", "matches": "v0" } },
                { second: { "stopBy": "end", "matches": "v0" } },
            ]);
        }
        self.recursive = serde_json::to_string(&rule).is_ok_and(|text| text.contains("\"r0\""));
        let rule_text = rule.to_string();
        ast.insert("rule".into(), rule);
        if !utils.is_empty() {
            ast.insert("utils".into(), Value::Object(utils));
        }
        if self.rng.chance(15) {
            let all_text = Value::Object(ast.clone()).to_string();
            let named = |text: &str, name: &&str| text.contains(&format!("${name}"));
            let bound: Vec<&str> = SINGLE
                .iter()
                .copied()
                .filter(|name| named(&rule_text, name))
                .collect();
            let free: Vec<&str> = SINGLE
                .iter()
                .copied()
                .filter(|name| !named(&all_text, name))
                .collect();
            let mut constraints = Map::new();
            if !bound.is_empty() && !free.is_empty() && self.rng.chance(60) {
                // A chain: one capture's constraint binds a capture the rule does not, whose own
                // constraint holds only where it is checked after the binding (in name order).
                let (bound, free) = (*self.rng.pick(&bound), *self.rng.pick(&free));
                let key = *self.rng.pick(&["inside", "has"]);
                constraints.insert(
                    bound.into(),
                    json!({ key: { "stopBy": "end", "pattern": format!("${free}") } }),
                );
                constraints.insert(free.into(), self.rule(2));
            } else {
                for _ in 0..1 + self.rng.below(2) {
                    let name = self.single();
                    constraints.insert(name, self.rule(2));
                }
            }
            ast.insert("constraints".into(), Value::Object(constraints));
        }
        let message = if self.counted { "n=$N" } else { "x" };
        let mut violation = json!({ "anchor": "witness.span", "code": "X", "message": message });
        // `violation.each` over a capture the rule names, now and then; more often in a
        // three-valued case, whose exclusion meets unknown candidates (`emit/each.rs`).
        let text = serde_json::to_string(&ast).unwrap_or_default();
        let named: Vec<&str> = SINGLE
            .iter()
            .copied()
            .filter(|name| {
                text.match_indices(&format!("${name}"))
                    .any(|(at, _)| !text[..at].ends_with('$'))
            })
            .collect();
        if !named.is_empty() && self.rng.chance(if self.three { 45 } else { 30 }) {
            violation["each"] = json!(*self.rng.pick(&named));
        }
        (Value::Object(ast), violation)
    }
}

enum Outcome {
    /// The front end refused the rule.
    Rejected,
    /// The native matcher hit a utility limit: its depth limit (256 levels or 1 MiB of stack,
    /// which depends on the build) or its work budget, limits the SQL engine does not emulate.
    NativeLimit,
    /// The SQL engine refused an operator.
    Unsupported(String),
    /// Both engines answered alike, with this many violations; whether the clause is
    /// three-valued, and whether a terminal answered unknown somewhere in it.
    Same {
        found: usize,
        three: bool,
        unknown: bool,
    },
    Differs {
        native: String,
        sql: String,
    },
}

/// Time spent evaluating cases, summed over the worker threads: native then SQL, for two-valued
/// then three-valued clauses.
static ENGINE_NANOS: [std::sync::atomic::AtomicU64; 4] = [
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
];

/// The longest SQL time of one case, in milliseconds, and how many cases took 2 s or more in
/// either engine.
static SLOWEST_CASE_MILLIS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static CASES_OVER_TWO_SECONDS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Appends `line` to `RERAIL_SQL_DIFF_SLOW_LOG`, where it is set, one whole line at a time.
fn slow_log(line: &Value) {
    append_log("RERAIL_SQL_DIFF_SLOW_LOG", line);
}

/// Appends `line` to `RERAIL_SQL_DIFF_KEEP`, where it is set, one whole line at a time.
fn keep_log(line: &Value) {
    append_log("RERAIL_SQL_DIFF_KEEP", line);
}

/// Appends `line` to the file the environment variable `variable` names, where it is set.
fn append_log(variable: &str, line: &Value) {
    use std::io::Write as _;
    static LOG: std::sync::Mutex<()> = std::sync::Mutex::new(());
    if let Ok(path) = std::env::var(variable) {
        let _held = LOG
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut file| file.write_all(format!("{line}\n").as_bytes()))
            .unwrap_or_else(|error| panic!("{variable}: {error}"));
    }
}

thread_local! {
    /// The native and SQL seconds of the thread's last case.
    static LAST_TIMES: std::cell::Cell<(f64, f64)> = const { std::cell::Cell::new((0.0, 0.0)) };
    /// The SQL engine's error on the thread's last case, if it failed.
    static LAST_SQL_ERROR: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn run_case(ast: &Value, violation: &Value, input: &Value, capabilities: &Value) -> Outcome {
    // A case known to take hours on the SQL engine (`RERAIL_SQL_DIFF_SKIP`: a fragment of its
    // rule's JSON) is skipped, and counted.
    if let Ok(fragment) = std::env::var("RERAIL_SQL_DIFF_SKIP")
        && !fragment.is_empty()
        && ast.to_string().contains(&fragment)
    {
        eprintln!("skipped case: {ast}");
        slow_log(&json!({ "ast": ast, "violation": violation, "skipped": true }));
        return Outcome::Rejected;
    }
    let document = clause_document(
        json!({ "id": "ast-match", "forbid": { "ast": ast }, "violation": violation }),
    );
    let Ok(compiled): napi::Result<CompiledCodeRuleDsl> =
        compile_document_with(document, capabilities)
    else {
        return Outcome::Rejected;
    };
    let three = crate::code_rule_dsl::test_support::grammar_matchers(&compiled)
        .iter()
        .any(|grammar| grammar.three_valued);
    let evaluate = |engine: &str| {
        compiled
            .evaluate(
                input.to_string(),
                "report".to_owned(),
                Some(engine.to_owned()),
            )
            .map_err(|error| error.reason)
    };
    let started = Instant::now();
    let native = evaluate("native");
    let unknowns = &crate::code_rule_dsl::sql::terminals::calls::UNKNOWN_VERDICTS;
    unknowns.set(0);
    let between = Instant::now();
    let sql = evaluate("sql");
    let unknown = unknowns.get() > 0;
    let at = if three { 2 } else { 0 };
    let (native_time, sql_time) = (between.duration_since(started), between.elapsed());
    LAST_TIMES.set((native_time.as_secs_f64(), sql_time.as_secs_f64()));
    LAST_SQL_ERROR.set(sql.as_ref().err().cloned());
    ENGINE_NANOS[at].fetch_add(
        native_time.as_nanos() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
    ENGINE_NANOS[at + 1].fetch_add(
        sql_time.as_nanos() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
    let slowest = sql_time.max(native_time);
    if slowest.as_secs() >= 5 {
        eprintln!(
            "slow case: native {:.1}s, sql {:.1}s: {}",
            native_time.as_secs_f64(),
            sql_time.as_secs_f64(),
            ast.to_string().chars().take(800).collect::<String>()
        );
    }
    SLOWEST_CASE_MILLIS.fetch_max(
        sql_time.as_millis() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
    if slowest >= std::time::Duration::from_secs(2) {
        CASES_OVER_TWO_SECONDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // `RERAIL_SQL_DIFF_SLOW_LOG`: every case over 2 s, whole, one JSON line each, for
        // `replayed_cases_agree`.
        slow_log(&json!({
            "ast": ast,
            "violation": violation,
            "native": native_time.as_secs_f64(),
            "sql": sql_time.as_secs_f64(),
        }));
    }
    if native.as_ref().is_ok_and(|output| {
        output.contains(UtilLimit::Depth.message()) || output.contains(UtilLimit::Work.message())
    }) {
        return Outcome::NativeLimit;
    }
    match (&native, &sql) {
        (_, Err(reason)) if reason.contains("Unsupported(") => Outcome::Unsupported(reason.clone()),
        _ if native == sql => Outcome::Same {
            found: native
                .ok()
                .and_then(|output| serde_json::from_str::<Value>(&output).ok())
                .and_then(|result| result["violations"].as_array().map(Vec::len))
                .unwrap_or(0),
            three,
            unknown,
        },
        _ => Outcome::Differs {
            native: format!("{native:?}"),
            sql: format!("{sql:?}"),
        },
    }
}

/// Counts the operators a rule uses, by key (`stopBy` by its kind, `matches` by its form, a
/// semantic leaf's keys under its name).
fn census(value: &Value, counts: &mut std::collections::BTreeMap<String, usize>) {
    census_under(value, "", counts);
}

fn census_under(
    value: &Value,
    parent: &str,
    counts: &mut std::collections::BTreeMap<String, usize>,
) {
    match value {
        Value::Object(object) => {
            for (key, item) in object {
                let name = match (parent, key.as_str(), item) {
                    (_, "stopBy", Value::String(stop)) => format!("stopBy:{stop}"),
                    (_, "stopBy", _) => "stopBy:rule".to_owned(),
                    (_, "matches", Value::Object(_)) => "matches:call".to_owned(),
                    (_, "matches", _) => "matches:shared".to_owned(),
                    (_, "nthChild", Value::Object(nth)) if nth.contains_key("ofRule") => {
                        "nthChild:ofRule".to_owned()
                    }
                    (
                        _,
                        "language" | "params" | "position" | "reverse" | "min" | "max" | "offset",
                        _,
                    ) => {
                        continue;
                    }
                    (_, "resolves", Value::Object(test)) if test.contains_key("binding") => {
                        "resolves.binding".to_owned()
                    }
                    ("binds", "redefines", _) => "binds.redefines".to_owned(),
                    (
                        "resolves.binding" | "binds" | "binds.redefines",
                        "site" | "value" | "annotation" | "occurrence" | "every",
                        _,
                    )
                    | ("formatArgs", "unused" | "missing", _)
                    | ("calls", "chain", _)
                    | ("arguments", "bind" | "absent", _) => format!("{parent}.{key}"),
                    (
                        "resolves" | "resolves.binding" | "binds" | "binds.redefines"
                        | "formatArgs" | "calls" | "arguments",
                        _,
                        _,
                    )
                    | ("parse", "grammar", _) => continue,
                    _ => key.clone(),
                };
                *counts.entry(name.clone()).or_default() += 1;
                if key == "utils" || key == "constraints" {
                    if let Value::Object(items) = item {
                        for rule in items.values() {
                            census_under(rule, "", counts);
                        }
                    }
                    continue;
                }
                match name.as_str() {
                    "resolves.binding" => census_under(&item["binding"], &name, counts),
                    "resolves" | "binds" | "binds.redefines" | "formatArgs" | "calls"
                    | "arguments" | "parse" => census_under(item, &name, counts),
                    // A capture's slot, not a rule.
                    "arguments.bind" => {}
                    _ => census_under(item, "", counts),
                }
            }
        }
        Value::Array(items) => items
            .iter()
            .for_each(|item| census_under(item, parent, counts)),
        _ => {}
    }
}

fn env_number(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .map(|value| {
            let value = value.trim();
            match value.strip_prefix("0x") {
                Some(hex) => u64::from_str_radix(hex, 16),
                None => value.parse(),
            }
            .unwrap_or_else(|_| panic!("{name} must be a number"))
        })
        .unwrap_or(default)
}

#[test]
fn generated_rules_agree_with_the_native_matcher() {
    let seed = env_number("RERAIL_SQL_DIFF_SEED", 0x5e7_c0de);
    let wanted = env_number("RERAIL_SQL_DIFF_CASES", 500) as usize;
    let started = Instant::now();
    let vocabularies: Vec<Vocabulary> = FIXTURES
        .iter()
        .map(|(label, paths)| Vocabulary::of(label, paths))
        .collect();
    let inputs: Vec<Value> = vocabularies
        .iter()
        .map(|vocabulary| {
            let files: Vec<(&str, &str, &str)> = vocabulary
                .files
                .iter()
                .map(|(path, source)| (vocabulary.label, path.as_str(), source.as_str()))
                .collect();
            snapshot(&files)
        })
        .collect();
    let mut capabilities: Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
    capabilities["limits"]["maxWitnesses"] = json!(1_000_000);
    capabilities["limits"]["maxVisitedNodes"] = json!(1_000_000_000);
    let mut generators: Vec<Generator<'_>> = vocabularies
        .iter()
        .enumerate()
        .map(|(index, vocabulary)| Generator {
            rng: Rng(seed ^ (index as u64).wrapping_mul(0x2545_f491_4f6c_dd1d)),
            vocabulary,
            calls: Vec::new(),
            plain: Vec::new(),
            params: None,
            counted: false,
            three: false,
            recursive: false,
        })
        .collect();
    let (mut compared, mut rejected, mut generated) = (0usize, 0usize, 0usize);
    let (mut with_violations, mut violations) = (0usize, 0usize);
    let (mut three_valued, mut with_unknown, mut three_recursive) = (0usize, 0usize, 0usize);
    let (mut three_each, mut three_each_unknown) = (0usize, 0usize);
    let mut operators = std::collections::BTreeMap::new();
    let (mut with_recursion, mut with_each, mut with_either, mut native_limits) =
        (0usize, 0usize, 0usize, 0usize);
    let mut unsupported = Vec::new();
    let mut differences = Vec::new();
    while compared < wanted {
        let batch: Vec<(usize, Value, Value, bool)> = (0..256)
            .map(|index| {
                let language = (generated + index) % generators.len();
                let (ast, violation) = generators[language].case();
                (language, ast, violation, generators[language].recursive)
            })
            .collect();
        generated += batch.len();
        let outcomes: Vec<Outcome> = batch
            .par_iter()
            .map(|(language, ast, violation, _)| {
                run_case(ast, violation, &inputs[*language], &capabilities)
            })
            .collect();
        for ((_, ast, violation, recursive), outcome) in batch.iter().zip(outcomes) {
            let each = violation.get("each").is_some();
            if compared >= wanted {
                break;
            }
            if !matches!(outcome, Outcome::Rejected | Outcome::NativeLimit) {
                census(ast, &mut operators);
                with_recursion += usize::from(*recursive);
                with_each += usize::from(each);
                with_either += usize::from(*recursive || each);
            }
            match outcome {
                Outcome::Rejected => rejected += 1,
                Outcome::NativeLimit => native_limits += 1,
                Outcome::Unsupported(reason) => {
                    compared += 1;
                    unsupported.push((ast.clone(), reason));
                }
                Outcome::Same {
                    found,
                    three,
                    unknown,
                } => {
                    compared += 1;
                    violations += found;
                    with_violations += usize::from(found > 0);
                    three_valued += usize::from(three);
                    with_unknown += usize::from(unknown);
                    three_recursive += usize::from(three && *recursive);
                    three_each += usize::from(three && each);
                    three_each_unknown += usize::from(three && each && unknown);
                }
                Outcome::Differs { native, sql } => {
                    // `RERAIL_SQL_DIFF_KEEP`: every differing case, whole, one JSON line each
                    // with its index among the compared cases, for `replayed_cases_agree`.
                    keep_log(&json!({ "case": compared, "ast": ast, "violation": violation }));
                    compared += 1;
                    differences.push((ast.clone(), native, sql));
                }
            }
        }
    }
    let mut refusals = std::collections::BTreeMap::<String, usize>::new();
    for (_, reason) in &unsupported {
        let name = reason
            .split("Unsupported(")
            .nth(1)
            .and_then(|rest| rest.split(')').next())
            .unwrap_or(reason);
        *refusals.entry(name.to_owned()).or_default() += 1;
    }
    eprintln!(
        "sql differential: seed {seed:#x}, {compared} cases compared ({generated} generated, {rejected} rejected by the front end), {} differences, {} unsupported {refusals:?}, {with_violations} cases with violations ({violations} violations), {three_valued} three-valued cases, {with_unknown} with an unknown verdict, {with_recursion} reaching a recursive utility ({three_recursive} three-valued), {with_each} under violation.each ({three_each} three-valued, {three_each_unknown} of them with an unknown verdict), {:.1}s",
        differences.len(),
        unsupported.len(),
        started.elapsed().as_secs_f64()
    );
    eprintln!(
        "recursion or each: {with_either} compared cases ({with_recursion} reaching a recursive utility, {with_each} under violation.each); {native_limits} cases not compared: the native matcher hit a utility limit (depth or work)"
    );
    eprintln!("operators in compared cases: {operators:?}");
    let seconds =
        |at: usize| ENGINE_NANOS[at].load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e9;
    eprintln!(
        "engine time over all threads: two-valued native {:.1}s sql {:.1}s; three-valued native {:.1}s sql {:.1}s",
        seconds(0),
        seconds(1),
        seconds(2),
        seconds(3)
    );
    eprintln!(
        "slowest case: sql {:.1}s; {} cases took 2 s or more",
        SLOWEST_CASE_MILLIS.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e3,
        CASES_OVER_TWO_SECONDS.load(std::sync::atomic::Ordering::Relaxed)
    );
    for (ast, reason) in unsupported.iter().take(5) {
        eprintln!("unsupported: {reason}\n  {ast}");
    }
    for (ast, native, sql) in differences.iter().take(5) {
        let cut = |text: &str| text.chars().take(1500).collect::<String>();
        eprintln!(
            "difference:\n  rule: {ast}\n  native: {}\n  sql: {}",
            cut(native),
            cut(sql)
        );
    }
    // The generator covers in-scope operators only; the shapes the engine refuses within them
    // are `lower::walk_leak`'s, a stop rule that recurses, a recursion whose reads loop in the
    // fixtures (where the matcher would recurse until its depth limit, or around the loop by
    // stopping at a first match), and under a three-valued `violation.each`, an `nthChild` whose
    // binding `ofRule` may be unknown (the matcher's excluding re-match leaves its unknown verdict
    // on the answer), and it must refuse them rather than answer. The semantic leaves' refusals
    // (`lower::leaves`): what a parsed tree runs per batch rather than per tree (patterns, stop
    // rules, recursive utilities, a nested parse), a tree's capture `violation.each` enumerates,
    // and `redefines.every` with binding sub-rules or under `violation.each`.
    let refused = |reason: &String| {
        [
            "Unsupported(nthChild with a binding ofRule as a relation's rule)",
            "Unsupported(recursion whose reads loop back)",
            "Unsupported(recursion deeper than 256 levels)",
            "Unsupported(recursive utility in a stop rule)",
            "Unsupported(three-valued nthChild whose excluding commit may be unknown)",
            "Unsupported(parse inside parse)",
            "Unsupported(pattern inside parse)",
            "Unsupported(stopBy rule inside parse)",
            "Unsupported(recursive utility inside parse)",
            "Unsupported(parse exporting a nested parse's capture)",
            "Unsupported(a parsed tree's capture enumerated by violation.each)",
            "Unsupported(binds.redefines.every with binding sub-rules)",
            "Unsupported(binds.redefines.every under violation.each)",
            "Unsupported(terminal binding more than 16 columns)",
        ]
        .iter()
        .any(|refusal| reason.contains(refusal))
    };
    // The recursions through a binding's nodes, which the generator above does not reach: every
    // one is in scope, so none may be refused.
    let following_rules = following_cases(seed);
    let following = following_rules
        .par_iter()
        .map(|(language, ast, violation)| {
            let at = FIXTURES
                .iter()
                .position(|(label, _)| label == language)
                .expect("a fixture language");
            (ast, run_case(ast, violation, &inputs[at], &capabilities))
        })
        .collect::<Vec<_>>();
    let mut following_failures = Vec::new();
    let (mut following_compared, mut following_found) = (0usize, 0usize);
    for (ast, outcome) in following {
        match outcome {
            Outcome::Same { found, .. } => {
                following_compared += 1;
                following_found += found;
            }
            Outcome::Rejected | Outcome::NativeLimit => {}
            Outcome::Unsupported(reason) => following_failures.push(format!("{reason}\n  {ast}")),
            Outcome::Differs { native, sql } => following_failures.push(format!(
                "difference:\n  rule: {ast}\n  native: {}\n  sql: {}",
                native.chars().take(1500).collect::<String>(),
                sql.chars().take(1500).collect::<String>()
            )),
        }
    }
    eprintln!(
        "following: {following_compared} recursions through a binding's nodes agree ({following_found} violations), {} do not",
        following_failures.len()
    );
    for failure in following_failures.iter().take(5) {
        eprintln!("following: {failure}");
    }
    assert!(
        differences.is_empty()
            && unsupported.iter().all(|(_, reason)| refused(reason))
            && following_failures.is_empty(),
        "{} differences, {} unsupported, {} recursions through a binding's nodes not agreeing",
        differences.len(),
        unsupported.len(),
        following_failures.len()
    );
}

/// Recursions through a binding's site or value (`resolves.binding`), which the matcher leaves
/// where they come back to a binding node whose sub-rule it is matching (`binding::FOLLOWING`):
/// a declaration's own name resolves to it, so most of these come back at once. A shared
/// utility, a pair of them, or a parameterized call; a leaf before or after the recursion; the
/// clause matching or negating it. Drawn from a generator of their own, so the cases above stay
/// as their seed names them.
fn following_cases(seed: u64) -> Vec<(&'static str, Value, Value)> {
    const CASES: usize = 64;
    let mut rng = Rng(seed ^ 0xf011_0e1d_0000_0001);
    let violation = json!({ "anchor": "witness.span", "code": "X", "message": "x" });
    (0..CASES)
        .map(|index| {
            let (language, roots, leaves): (&str, &[&str], &[Value]) = if index % 2 == 0 {
                (
                    "typescript",
                    &[
                        "variable_declarator",
                        "lexical_declaration",
                        "function_declaration",
                        "class_declaration",
                        "statement_block",
                        "identifier",
                    ],
                    &[
                        json!({ "kind": "number" }),
                        json!({ "kind": "string" }),
                        json!({ "kind": "arrow_function" }),
                        json!({ "kind": "call_expression", "pattern": "$X" }),
                        json!({ "has": { "stopBy": "end", "kind": "number", "pattern": "$X" } }),
                    ],
                )
            } else {
                (
                    "python",
                    &[
                        "function_definition",
                        "class_definition",
                        "assignment",
                        "call",
                        "block",
                        "identifier",
                    ],
                    &[
                        json!({ "kind": "integer" }),
                        json!({ "kind": "string" }),
                        json!({ "kind": "return_statement" }),
                        json!({ "kind": "call", "regex": "^print", "pattern": "$X" }),
                        json!({ "has": { "stopBy": "end", "kind": "integer", "pattern": "$X" } }),
                    ],
                )
            };
            let root = *rng.pick(roots);
            let leaf = rng.pick(leaves).clone();
            let node = *rng.pick(&["site", "site", "value"]);
            let parameterized = rng.chance(30);
            let mutual = !parameterized && rng.chance(40);
            let reference = |name: &str| {
                if parameterized {
                    json!({ name: { "P": "$P" } })
                } else {
                    json!(name)
                }
            };
            let step = |target: Value| {
                json!({ "kind": "identifier", "resolves": { "binding": { node: { "matches": target } } } })
            };
            let walk = if mutual {
                json!({ "has": { "stopBy": "end", "matches": "hop" } })
            } else {
                let mut walk = step(reference("known"));
                if rng.chance(30) {
                    walk["pattern"] = json!("$Y");
                }
                json!({ "has": { "stopBy": "end", "all": [walk] } })
            };
            let branches = if rng.chance(50) {
                vec![leaf, walk]
            } else {
                vec![walk, leaf]
            };
            let body = json!({ "any": branches });
            let mut utils = Map::new();
            if parameterized {
                utils.insert("known".into(), json!({ "params": ["P"], "rule": body }));
            } else {
                utils.insert("known".into(), body);
            }
            if mutual {
                utils.insert("hop".into(), step(json!("known")));
            }
            let called = if parameterized {
                json!({ "known": { "P": "$X" } })
            } else {
                json!("known")
            };
            let rule = if rng.chance(25) {
                json!({ "kind": root, "not": { "matches": called } })
            } else {
                json!({ "kind": root, "matches": called })
            };
            let ast = json!({ "language": language, "rule": rule, "utils": utils });
            (language, ast, violation.clone())
        })
        .collect()
}

/// The replayed cases, one at a time on the differential's inputs: each one's native and SQL
/// time, the rows its statements wrote, and whether the engines agree; every case must be
/// compared and agree. By default the cases of `tests/replay.jsonl`, which kill every seeded
/// mutant (`sql::mutant`); `RERAIL_SQL_DIFF_REPLAY` names another file of them (lines of
/// `RERAIL_SQL_DIFF_SLOW_LOG` or `RERAIL_SQL_DIFF_KEEP`):
/// `RERAIL_SQL_DIFF_REPLAY=<file> cargo test --profile loop sql::tests::differential::replayed -- --nocapture`.
#[test]
fn replayed_cases_agree() {
    use crate::code_rule_dsl::sql::exec::STATEMENT_PROFILE;
    let path = std::env::var("RERAIL_SQL_DIFF_REPLAY").unwrap_or_else(|_| {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/code_rule_dsl/sql/tests/replay.jsonl"
        )
        .to_owned()
    });
    let inputs: Vec<(&str, Value)> = FIXTURES
        .iter()
        .map(|(label, paths)| {
            let vocabulary = Vocabulary::of(label, paths);
            let files: Vec<(&str, &str, &str)> = vocabulary
                .files
                .iter()
                .map(|(path, source)| (*label, path.as_str(), source.as_str()))
                .collect();
            (*label, snapshot(&files))
        })
        .collect();
    let mut capabilities: Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
    capabilities["limits"]["maxWitnesses"] = json!(1_000_000);
    capabilities["limits"]["maxVisitedNodes"] = json!(1_000_000_000);
    let (mut differ, mut uncompared) = (0, 0);
    for line in std::fs::read_to_string(&path).expect("replay file").lines() {
        let case: Value = serde_json::from_str(line).expect("a JSON case");
        let (ast, violation) = (&case["ast"], &case["violation"]);
        let language = ast["language"].as_str().expect("a language");
        let input = &inputs
            .iter()
            .find(|(label, _)| *label == language)
            .expect("a fixture language")
            .1;
        STATEMENT_PROFILE.set(Some(Default::default()));
        LAST_TIMES.set((0.0, 0.0));
        let outcome = run_case(ast, violation, input, &capabilities);
        let profile = STATEMENT_PROFILE.take().unwrap_or_default();
        let rows: usize = profile.values().map(|(_, rows, _)| rows).sum();
        let statements: usize = profile.values().map(|(_, _, runs)| runs).sum();
        let verdict = match &outcome {
            Outcome::Same { found, .. } => format!("same, {found} violations"),
            Outcome::Differs { .. } => {
                differ += 1;
                "DIFFERS".to_owned()
            }
            Outcome::Unsupported(reason) => {
                uncompared += 1;
                reason.clone()
            }
            Outcome::NativeLimit => {
                uncompared += 1;
                "native limit".to_owned()
            }
            Outcome::Rejected => {
                uncompared += 1;
                "rejected".to_owned()
            }
        };
        let (native, sql) = LAST_TIMES.get();
        let failed = LAST_SQL_ERROR
            .take()
            .map_or(String::new(), |error| format!(" (sql failed: {error})"));
        eprintln!(
            "replayed: native {native:.3}s, sql {sql:.3}s (logged {:.1}s); {statements} statements, {rows} rows; {verdict}{failed}: {}",
            case["sql"].as_f64().unwrap_or(-1.0),
            ast.to_string().chars().take(300).collect::<String>()
        );
        // Where the time of a slow case went: its costliest statements, with their rows and runs.
        if sql >= 1.0 {
            let mut costly: Vec<_> = profile.iter().collect();
            costly.sort_by_key(|(_, (nanos, _, _))| std::cmp::Reverse(*nanos));
            let shown = env_number("RERAIL_SQL_DIFF_REPLAY_STATEMENTS", 6) as usize;
            for (statement, (nanos, rows, runs)) in costly.into_iter().take(shown) {
                eprintln!(
                    "  {:>8.3}s {rows:>10} rows {runs:>6} runs  {}",
                    *nanos as f64 / 1e9,
                    statement.chars().take(400).collect::<String>()
                );
            }
        }
    }
    assert_eq!(differ, 0, "{differ} replayed cases differ");
    assert_eq!(uncompared, 0, "{uncompared} replayed cases not compared");
}

/// The loops of a statement's `EXPLAIN QUERY PLAN` rows `(id, parent, detail)` that read a whole
/// table once per row of an outer loop: a `SCAN` after the first loop of its join, or in a
/// correlated subquery. A `node` table loop is bounded where a constraint on `id`, `parent` or
/// `last` drives it (its index string: `i`, `p`, `l` codes); a terminal's where it has a term
/// (a nonzero index number). `SEARCH` loops (an index, a primary key, an automatic index) are
/// bounded by their key.
pub(super) fn inner_scans(plan: &[(i64, i64, String)]) -> Vec<String> {
    let parent_of: std::collections::HashMap<i64, i64> =
        plan.iter().map(|(id, parent, _)| (*id, *parent)).collect();
    let detail_of: std::collections::HashMap<i64, &str> = plan
        .iter()
        .map(|(id, _, detail)| (*id, detail.as_str()))
        .collect();
    let is_loop = |detail: &str| {
        (detail.starts_with("SCAN ") || detail.starts_with("SEARCH "))
            && detail != "SCAN CONSTANT ROW"
    };
    let mut found = Vec::new();
    for (id, parent, detail) in plan {
        if !detail.starts_with("SCAN ") || !is_loop(detail) {
            continue;
        }
        let after_another = plan
            .iter()
            .any(|(other, up, text)| up == parent && other < id && is_loop(text));
        let mut correlated = false;
        let mut at = *parent;
        while let Some(text) = detail_of.get(&at) {
            correlated |= text.starts_with("CORRELATED ");
            at = parent_of[&at];
        }
        if !after_another && !correlated {
            continue;
        }
        let bounded = match detail.split_once("VIRTUAL TABLE INDEX ") {
            Some((_, index)) => {
                let (number, codes) = index.split_once(':').unwrap_or((index, ""));
                if codes.is_empty() {
                    number.trim().parse::<i64>().unwrap_or(0) != 0
                } else if codes.starts_with('@') {
                    // A frame's lookup on its constrained columns (`frames`).
                    true
                } else {
                    ["i=", "p=", "i>", "i}", "i<", "i{", "l}", "l>"]
                        .iter()
                        .any(|code| {
                            codes
                                .as_bytes()
                                .chunks(2)
                                .any(|pair| pair == code.as_bytes())
                        })
                }
            }
            None => false,
        };
        if !bounded {
            found.push(detail.clone());
        }
    }
    found
}

/// No statement the engine emits reads a whole table on the inner side of a join: every join
/// back to a frame goes through a primary key or an index, and every walk through a bounded
/// `node` constraint. And none is a recursive CTE: walks, stops and chains are structural
/// primitives behind virtual tables. Checked on the plans of the differential's generated clauses
/// (every operator, two- and three-valued, recursion and `violation.each`), as their sessions
/// prepare them.
#[test]
fn emitted_joins_never_scan_a_whole_table_on_the_inner_side() {
    use crate::code_rule_dsl::test_support::grammar_matchers;
    let vocabularies: Vec<Vocabulary> = FIXTURES
        .iter()
        .map(|(label, paths)| Vocabulary::of(label, paths))
        .collect();
    let mut capabilities: Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
    capabilities["limits"]["maxWitnesses"] = json!(1_000_000);
    let mut generators: Vec<Generator<'_>> = vocabularies
        .iter()
        .enumerate()
        .map(|(index, vocabulary)| Generator {
            rng: Rng(0x91a7 ^ (index as u64).wrapping_mul(0x2545_f491_4f6c_dd1d)),
            vocabulary,
            calls: Vec::new(),
            plain: Vec::new(),
            params: None,
            counted: false,
            three: false,
            recursive: false,
        })
        .collect();
    let (mut clauses, mut statements) = (0usize, 0usize);
    let mut offenders: std::collections::BTreeMap<String, (usize, String)> =
        std::collections::BTreeMap::new();
    for case in 0..1500 {
        let generator = &mut generators[case % 3];
        let (ast, violation) = generator.case();
        let document = clause_document(
            json!({ "id": "ast-match", "forbid": { "ast": ast }, "violation": violation }),
        );
        let Ok(compiled) = compile_document_with(document, &capabilities) else {
            continue;
        };
        let grammars = grammar_matchers(&compiled);
        let grammars: Vec<_> = grammars.iter().collect();
        let each = violation["each"].as_str();
        let Ok(program) = crate::code_rule_dsl::sql::compile_enumerating(&grammars, each) else {
            continue;
        };
        clauses += 1;
        let mut session = program.session().expect("a session");
        for (sql, plan) in session.plans().expect("plans") {
            statements += 1;
            assert!(!sql.contains("RECURSIVE"), "a recursive CTE: {sql}");
            for detail in inner_scans(&plan) {
                // One example statement per offending plan line, with the tables' numbers cut.
                let shape: String = detail
                    .split(' ')
                    .map(|word| {
                        word.trim_end_matches(|c: char| c.is_ascii_digit())
                            .to_owned()
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                let entry = offenders.entry(shape).or_insert((0, sql.clone()));
                entry.0 += 1;
            }
        }
    }
    eprintln!("plan audit: {clauses} clauses, {statements} statements");
    for (shape, (count, sql)) in &offenders {
        eprintln!("inner scan `{shape}` in {count} statements, e.g.:\n  {sql}");
    }
    assert!(clauses > 500, "{clauses} clauses audited");
    assert!(
        offenders.is_empty(),
        "{} inner scan shapes",
        offenders.len()
    );
}
