use super::*;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::oracle;
use crate::code_rule_dsl::value::Lang;
use ast_grep_core::tree_sitter::LanguageExt;
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const ORACLE: &str = oracle!("php/value/php.expected.json");

/// The value of the expression at `node` with nothing followed: no variable's binding, no
/// constant's declaration, the value a constant expression has in isolation.
fn decode_isolated<D: Doc>(node: &Node<'_, D>) -> Value {
    let mut spend = || true;
    let mut context = Context::new(&mut spend);
    decode(node, &mut context)
}

/// The value of the one expression statement `source` holds; `Unknown` where the grammar does not
/// read one (a syntax error).
fn decode_source(source: &str) -> Value {
    let text = format!("{source};\n");
    let root = GrammarDialect::Php.ast_grep(&text);
    let statements: Vec<_> = root.root().children().filter(|c| c.is_named()).collect();
    let [statement] = statements.as_slice() else {
        return Value::Unknown;
    };
    if statement.kind() != "expression_statement" {
        return Value::Unknown;
    }
    let Some(expression) = statement.children().find(|child| child.is_named()) else {
        return Value::Unknown;
    };
    decode_isolated(&expression)
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).expect("hex"))
        .collect()
}

fn key_agrees(key: &Key, oracle: &Json) -> bool {
    match (key, oracle[0].as_str()) {
        (Key::Int(value), Some("int")) => oracle[1].as_str() == Some(value.to_string().as_str()),
        (Key::Str(value), Some("string")) => oracle[1].as_str().map(hex).as_ref() == Some(value),
        _ => false,
    }
}

/// Whether Rerail's value is the one PHP recorded (`generate_php.php`'s tagged encoding).
pub(crate) fn agrees(value: &Value, oracle: &Json) -> bool {
    match (value, oracle[0].as_str()) {
        (Value::None, Some("null")) => true,
        (Value::Bool(value), Some("bool")) => oracle[1].as_bool() == Some(*value),
        (Value::Int(value), Some("int")) => oracle[1].as_str() == Some(value.to_string().as_str()),
        (Value::Float(value), Some("float")) => {
            let bits = u64::from_str_radix(oracle[1].as_str().unwrap_or_default(), 16)
                .map(f64::from_bits)
                .expect("float bits");
            bits.to_bits() == value.to_bits() || (bits.is_nan() && value.is_nan())
        }
        (Value::PhpStr(value), Some("string")) => {
            oracle[1].as_str().map(hex).as_ref() == Some(value)
        }
        (Value::PhpArray(items), Some("array")) => {
            let entries = oracle[1].as_array().expect("entries");
            items.len() == entries.len()
                && items.iter().zip(entries).all(|((key, value), entry)| {
                    key_agrees(key, &entry[0]) && agrees(value, &entry[1])
                })
        }
        _ => false,
    }
}

/// What the value PHP recorded tells of its elements (`partial_agrees`): an array's length, or
/// that it is no array.
fn shape(oracle: &Json) -> Shape {
    match oracle[0].as_str() {
        Some("array") => Shape::Length(oracle[1].as_array().expect("entries").len()),
        _ => Shape::NotArray,
    }
}

/// Every case in the PHP oracle (`rerail-fixtures/php/value/generate_php.php`):
/// - where PHP raises an error, Rerail knows nothing;
/// - a value Rerail knows is the value PHP gives the constant expression, or, where PHP refuses
///   it as a constant expression, the value it evaluates to at run time; of a value it knows only
///   part of, each part it claims is;
/// - where PHP evaluates the constant expression, Rerail knows all of its value, and where Rerail
///   knows part of a value it knows all of it, unless the case says why not.
#[test]
fn php_constant_expressions_decode_as_php_evaluates_them() {
    let oracle: Json = serde_json::from_str(ORACLE).expect("oracle JSON");
    assert_eq!(oracle["php"].as_str().map(|v| &v[..3]), Some("8.5"));
    assert_eq!(oracle["intSize"].as_u64(), Some(8));
    let mut failures = Vec::new();
    let mut known = 0;
    for case in oracle["cases"].as_array().expect("cases") {
        let source = case["src"].as_str().expect("src");
        let value = decode_source(source);
        let constant = &case["const"];
        let refused = constant[0].as_str() == Some("error");
        let expected = if refused { &case["eval"] } else { constant };
        let exempt = case.get("unknown").is_some();
        if value.is_known() {
            known += 1;
            if expected[0].as_str() == Some("error") {
                failures.push(format!(
                    "{source:?}: Rerail reads {value:?}, PHP raises {expected}"
                ));
            } else if value.is_constant() {
                if !agrees(&value, expected) {
                    failures.push(format!(
                        "{source:?}: Rerail reads {value:?}, PHP gives {expected}"
                    ));
                }
            } else if partial_agrees(&value, shape(expected)) != Some(true) {
                failures.push(format!(
                    "{source:?}: Rerail reads {value:?}, PHP gives {expected}"
                ));
            } else if !exempt {
                failures.push(format!(
                    "{source:?}: PHP gives {expected}, Rerail knows only {value:?}"
                ));
            }
        } else if !refused && !exempt {
            failures.push(format!(
                "{source:?}: PHP gives {constant}, Rerail knows nothing"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} disagreements:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(known > 400, "Rerail knows only {known} values");
}

/// `===`, `==` and `<=>` over every pair of the oracle's operands, as PHP computes them: `equals`
/// is `===`; `order` is `<=>`, where a pair each side of which PHP finds greater does not order.
#[test]
fn php_comparisons_agree_with_php() {
    let oracle: Json = serde_json::from_str(ORACLE).expect("oracle JSON");
    let operands: Vec<Value> = oracle["pairs"]["operands"]
        .as_array()
        .expect("operands")
        .iter()
        .map(|source| decode_source(source.as_str().expect("operand")))
        .collect();
    assert!(!operands.is_empty(), "the oracle has no operands");
    if let Some(operand) = operands.iter().find(|operand| !operand.is_constant()) {
        panic!("Rerail does not know the operand {operand:?}");
    }
    let rows = oracle["pairs"]["results"].as_array().expect("results");
    assert_eq!(rows.len(), operands.len(), "one row per operand");
    let mut failures = Vec::new();
    for (row, left) in rows.iter().zip(&operands) {
        let row = row.as_array().expect("row");
        assert_eq!(row.len(), operands.len(), "one result per operand");
        for (result, right) in row.iter().zip(&operands) {
            let identical = result[0].as_bool().expect("===");
            let loose = result[1].as_bool().expect("==");
            let spaceship = result[2].as_i64().expect("<=>");
            let order = left.order(right, Lang::Php).expect("known operands");
            let rerail_spaceship = match order {
                None => 1,
                Some(order) => order as i64,
            };
            if left.equals(right, Lang::Php) != Some(identical)
                || (order == Some(Ordering::Equal)) != loose
                || rerail_spaceship != spaceship
            {
                failures.push(format!(
                    "{left:?} vs {right:?}: PHP ===, ==, <=> give {result}, Rerail {:?}, {order:?}",
                    left.equals(right, Lang::Php)
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} disagreements:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn php_files(root: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(root).expect("corpus directory") {
        let entry = entry.expect("entry");
        let path = entry.path();
        if entry.file_type().expect("type").is_dir() {
            php_files(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "php")
            && !path.to_string_lossy().ends_with(".blade.php")
        {
            out.push(path);
        }
    }
}

/// The initializers of a file's constants and enum cases, by the byte offset of the name each
/// gives its value: a `const_element`'s second named child, an `enum_case`'s `value`.
fn initializers<'r, D: Doc>(root: Node<'r, D>) -> BTreeMap<usize, Node<'r, D>> {
    let mut found = BTreeMap::new();
    for node in root.dfs() {
        let pair = match node.kind().as_ref() {
            "const_element" => {
                let mut named = node
                    .children()
                    .filter(|child| child.is_named() && !child.kind().ends_with("comment"));
                named.next().zip(named.next())
            }
            "enum_case" => node.field("name").zip(node.field("value")),
            _ => None,
        };
        if let Some((name, value)) = pair {
            found.insert(name.range().start, value);
        }
    }
    found
}

/// What PHP tells of an initializer's elements: an array's exact count; for an array literal with
/// a key or spread that reads context, whether PHP surely finds an element in it (`false`: it may
/// find none); or that it is no array.
#[derive(Clone, Copy, Debug)]
enum Shape {
    Length(usize),
    Nonempty(bool),
    NotArray,
}

/// Whether a partial value (an array whose length or emptiness Rerail knows, not every element)
/// holds for PHP's `shape`; `None` for a whole or unknown value, or a claim `shape` cannot check.
fn partial_agrees(value: &Value, shape: Shape) -> Option<bool> {
    let Value::Container {
        kind,
        len,
        nonempty,
    } = value
    else {
        return None;
    };
    if !matches!(kind, ContainerKind::PhpArray) {
        return Some(false);
    }
    match shape {
        Shape::Length(length) => Some(
            len.is_none_or(|len| len == length)
                && nonempty.is_none_or(|nonempty| nonempty == (length > 0)),
        ),
        Shape::NotArray => Some(false),
        Shape::Nonempty(_) if len.is_some() => None,
        Shape::Nonempty(surely) => match nonempty {
            Some(true) => Some(surely),
            Some(false) if surely => Some(false),
            _ => None,
        },
    }
}

/// Where Rerail's value of a corpus initializer is not PHP's and Rerail is not wrong, by corpus, path
/// within the checkout and line, with the evidence.
const PHP_VALUE_DIVERGENCES: &[(&str, &str, u64, &str)] = &[];

#[derive(Default)]
struct ValueTally {
    initializers: usize,
    unparsed_files: usize,
    not_utf8_files: usize,
    /// Initializers of files with syntax errors or not UTF-8.
    not_compared: usize,
    without_node: usize,
    items: usize,
    decided: usize,
    agreed: usize,
    contradictions: usize,
    php_errors: usize,
    unevaluable: usize,
    reasons: BTreeMap<String, usize>,
    decided_unevaluable: Vec<String>,
    partial: usize,
    partial_agreed: usize,
    partial_unchecked: usize,
    partial_contradictions: usize,
    undecided: BTreeMap<String, (usize, Vec<String>)>,
    failures: Vec<String>,
    divergences: Vec<Json>,
}

impl ValueTally {
    /// A Rerail claim PHP contradicts: an evidenced divergence, or a failure.
    fn contradiction(&mut self, corpus: &str, path: &str, line: u64, rerail: String, php: String) {
        match PHP_VALUE_DIVERGENCES
            .iter()
            .find(|(c, p, l, _)| (*c, *p, *l) == (corpus, path, line))
        {
            Some((_, _, _, evidence)) => self.divergences.push(serde_json::json!({
                "corpus": corpus, "path": path, "line": line, "rerail": rerail,
                "reference": php, "evidence": evidence,
            })),
            None => self
                .failures
                .push(format!("{corpus}:{path}:{line}: Rerail {rerail}, PHP {php}")),
        }
    }

    /// Rerail's value of one recorded initializer against PHP's.
    fn compare<D: Doc>(&mut self, corpus: &str, path: &str, record: &Json, node: &Node<'_, D>) {
        let line = record["line"].as_u64().expect("line");
        let value = decode_isolated(node);
        let Some(php) = record.get("const") else {
            // PHP reads the initializer in its file's context.
            self.unevaluable += 1;
            for reason in record["unevaluable"].as_array().expect("reasons") {
                *self
                    .reasons
                    .entry(reason.as_str().expect("reason").to_owned())
                    .or_default() += 1;
            }
            if value.is_constant() {
                self.decided_unevaluable
                    .push(format!("{path}:{line}: {} is {value:?}", record["src"]));
            } else if value.is_known() {
                let shape = match (record["length"].as_u64(), record["nonempty"].as_bool()) {
                    (Some(length), _) => Some(Shape::Length(length as usize)),
                    (None, Some(surely)) => Some(Shape::Nonempty(surely)),
                    (None, None) => None,
                };
                self.partial_claim(corpus, path, line, &value, shape);
            }
            return;
        };
        if php[0].as_str() == Some("error") {
            self.php_errors += 1;
            if value.is_known() {
                self.contradictions += 1;
                self.contradiction(corpus, path, line, format!("{value:?}"), php.to_string());
            }
            return;
        }
        self.items += 1;
        if value.is_constant() {
            self.decided += 1;
            if agrees(&value, php) {
                self.agreed += 1;
            } else {
                self.contradictions += 1;
                self.contradiction(corpus, path, line, format!("{value:?}"), php.to_string());
            }
            return;
        }
        if value.is_known() {
            self.partial_claim(corpus, path, line, &value, Some(shape(php)));
        }
        let (count, samples) = self.undecided.entry(node.kind().to_string()).or_default();
        *count += 1;
        if samples.len() < 5 {
            samples.push(format!("{path}:{line}: {}", record["src"]));
        }
    }

    /// A partial value against what PHP tells of the initializer's elements, if anything.
    fn partial_claim(
        &mut self,
        corpus: &str,
        path: &str,
        line: u64,
        value: &Value,
        shape: Option<Shape>,
    ) {
        self.partial += 1;
        let checked = shape.and_then(|shape| Some((shape, partial_agrees(value, shape)?)));
        match checked {
            Some((_, true)) => self.partial_agreed += 1,
            None => self.partial_unchecked += 1,
            Some((shape, false)) => {
                self.partial_contradictions += 1;
                self.contradiction(
                    corpus,
                    path,
                    line,
                    format!("{value:?}"),
                    format!("{shape:?}"),
                );
            }
        }
    }
}

fn rate(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        1.0
    } else {
        part as f64 / whole as f64
    }
}

/// Every constant and enum-case initializer of whole corpora (`RERAIL_PHP_VALUES`, written by
/// `rerail-fixtures/php/value/generate_corpus.php`), Rerail's value of it in isolation against what
/// PHP 8.5.11 evaluates it to; the T3 report section goes to `RERAIL_PHP_VALUES_REPORT`. A value
/// Rerail knows must be PHP's, and a partial one (an array's length or emptiness) must hold for
/// PHP's; an initializer PHP evaluates only in its file's context (a named or class constant) is
/// not compared, but Rerail knowing it in isolation is listed.
#[test]
#[ignore = "PHP constant expressions over full PHP corpora"]
fn benchmark_php_values_corpus() {
    let input = std::env::var("RERAIL_PHP_VALUES").expect("RERAIL_PHP_VALUES");
    let output = std::env::var("RERAIL_PHP_VALUES_REPORT").expect("report path");
    let corpora: Json =
        serde_json::from_str(&std::fs::read_to_string(input).expect("corpora")).expect("JSON");
    assert_eq!(corpora["php"].as_str().map(|v| &v[..3]), Some("8.5"));
    assert_eq!(corpora["intSize"].as_u64(), Some(8));
    let mut sections = Vec::new();
    let mut divergences = Vec::new();
    let mut failures = Vec::new();
    for corpus in corpora["corpora"].as_array().expect("corpora") {
        let name = corpus["name"].as_str().expect("name");
        let root = Path::new(corpus["root"].as_str().expect("root"));
        let excluded = corpus["excluded"]
            .as_array()
            .expect("excluded")
            .iter()
            .filter_map(Json::as_str)
            .collect::<Vec<_>>();
        let mut files = Vec::new();
        for path in corpus["paths"].as_array().expect("paths") {
            php_files(Path::new(path.as_str().expect("path")), &mut files);
        }
        files.sort();
        files.retain(|file| !excluded.contains(&file.to_string_lossy().as_ref()));
        let records = corpus["records"].as_object().expect("records");
        let mut tally = ValueTally::default();
        for file in &files {
            let key = file.to_string_lossy();
            let relative = file
                .strip_prefix(root)
                .expect("file in checkout")
                .to_string_lossy();
            let found = records
                .get(key.as_ref())
                .and_then(Json::as_array)
                .map_or(&[][..], Vec::as_slice);
            tally.initializers += found.len();
            // Rerail reads source text: a file that is not UTF-8 reaches it only lossily.
            let Ok(source) = String::from_utf8(std::fs::read(file).expect("source")) else {
                tally.not_utf8_files += 1;
                tally.not_compared += found.len();
                continue;
            };
            let grep = GrammarDialect::Php.ast_grep(&source);
            // A file the grammar reads with errors is outside what Rerail decides, as tape parity
            // reports it.
            if grep
                .root()
                .dfs()
                .any(|node| node.is_error() || node.is_missing())
            {
                tally.unparsed_files += 1;
                tally.not_compared += found.len();
                continue;
            }
            let nodes = initializers(grep.root());
            for record in found {
                let at = record["at"].as_u64().expect("at") as usize;
                match nodes.get(&at) {
                    Some(node) => tally.compare(name, &relative, record, node),
                    None => tally.without_node += 1,
                }
            }
        }
        let undecided = tally
            .undecided
            .iter()
            .map(|(kind, (count, samples))| {
                (
                    kind.clone(),
                    serde_json::json!({ "count": count, "samples": samples }),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        sections.push(serde_json::json!({
            "name": name,
            "files": files.len(),
            "filesWithSyntaxErrors": tally.unparsed_files,
            "filesNotUtf8": tally.not_utf8_files,
            "unbuilt": [],
            "items": tally.items,
            "decided": tally.decided,
            "decidedRate": rate(tally.decided, tally.items),
            "referencePositives": tally.items,
            "rerailPositives": tally.decided,
            "truePositives": tally.agreed,
            "contradictions": tally.contradictions,
            "precision": 1.0 - tally.contradictions as f64 / tally.decided.max(1) as f64,
            "recall": rate(tally.agreed, tally.items),
            "initializers": tally.initializers,
            "initializersInUncomparedFiles": tally.not_compared,
            "initializersWithoutNode": tally.without_node,
            "phpErrors": tally.php_errors,
            "unevaluableInIsolation": tally.unevaluable,
            "unevaluableReasons": tally.reasons,
            "decidedButUnevaluable": tally.decided_unevaluable.len(),
            "decidedButUnevaluableItems": tally.decided_unevaluable,
            "partialClaims": tally.partial,
            "partialClaimsAgreed": tally.partial_agreed,
            "partialClaimsUnchecked": tally.partial_unchecked,
            "partialContradictions": tally.partial_contradictions,
            "undecidedByKind": undecided,
        }));
        divergences.extend(tally.divergences);
        failures.extend(tally.failures);
    }
    let report = serde_json::json!({
        "capability": "values",
        "reference": {
            "tool": "PHP",
            "version": corpora["php"],
            "signal": format!(
                "the value PHP gives `const X = <initializer>;` in a fresh `php -n` process \
                 (tagged: int, float bits, string bytes, bool, null, array with keys in order), \
                 for every constant and enum-case initializer nikic/php-parser {} finds",
                corpora["phpParser"].as_str().unwrap_or_default()
            ),
        },
        "method": "Every namespace-level `const`, class/interface/trait/enum `const` and backed \
            enum case initializer of the corpus (Blade templates and files `php -l` rejects \
            excluded) is located by nikic/php-parser. One built from literals alone (`true`, \
            `false`, `null` the only names) is evaluable in isolation: PHP 8.5.11 evaluates it as \
            the T2 value oracle does (`rerail-fixtures/php/value/evaluate.php`); the rest read their \
            file's context (named, class or magic constants, `new`, closures) and are counted \
            with the reason, not compared. Rerail's claim is the value `decode` gives the \
            initializer node with nothing followed, the T2 entrypoint (`value/php/tests.rs`); a \
            whole value is decided, an array of which Rerail knows only the length or emptiness is \
            a partial claim, checked against PHP's element count (for unevaluable array literals, \
            the count with each context-reading element value replaced by `null`, or, where a key \
            or spread reads context, only whether PHP surely finds an element once such keys are \
            `0` and such spreads `[]`; a claim that evaluation cannot check is unchecked). items = \
            evaluable initializers PHP gives a value (= referencePositives); decided = those \
            Rerail gives a whole value (= rerailPositives); truePositives = decided values equal to \
            PHP's (`agrees`, as T2 compares); contradictions = decided values PHP's differs from, \
            plus values Rerail knows where PHP raises an error. precision = 1 - \
            contradictions/decided; recall = truePositives/referencePositives, which with no \
            contradictions equals decidedRate = decided/items. Files tree-sitter-php reads with \
            errors are counted, not compared. Partial claims, and whole values Rerail gives \
            initializers unevaluable in isolation, are reported separately.",
        "corpora": sections,
        "divergences": divergences,
    });
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).expect("write report");
    assert!(
        failures.is_empty(),
        "{} contradictions:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
