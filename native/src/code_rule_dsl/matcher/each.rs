//! `violation.each` (a Rerail extension): one violation per distinct binding of a capture at one root.
//! The root is matched again with each binding found so far excluded: a relation's search and an
//! `any` pass over a candidate that would bind the capture to an excluded range, so the next match
//! binds another. Negations, counts, stop rules and `nthChild` probes read their own bindings and
//! exclude nothing.
//!
//! A binding is its source range in the host file. A capture a parsed sub-tree exported (M11a's
//! `parse`) is bound in the host as a reserved span variable, and inside the sub-tree to a node of
//! the sub-tree's own document: a matcher that runs a sub-tree's rule enters `HostRanges` so that
//! exclusion reads the sub-tree's nodes by their host ranges.

use super::exported::exported_span;
use ast_grep_core::Doc;
use ast_grep_core::meta_var::MetaVarEnv;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

/// A parsed sub-tree's coordinates: `node` maps one of its node ids to its range in the document
/// the sub-tree was parsed from; `span` maps a range of the sub-tree there.
#[derive(Clone)]
pub(crate) struct HostMap {
    pub(crate) node: Arc<dyn Fn(usize) -> Option<Range<usize>>>,
    pub(crate) span: Arc<dyn Fn(Range<usize>) -> Option<Range<usize>>>,
}

thread_local! {
    /// Whether an enumeration is excluding bindings here (false inside a negation or count).
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// The enumerated capture and the ranges it was bound to so far, as `(start, end)`.
    static EXCLUDED: RefCell<(String, HashSet<(usize, usize)>)> =
        RefCell::new((String::new(), HashSet::new()));
    /// The sub-trees being matched, innermost last.
    static HOSTS: RefCell<Vec<HostMap>> = const { RefCell::new(Vec::new()) };
    /// Bumped whenever the excluded ranges change: what a memoized verdict read of them.
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

/// What an excluding verdict may read of the enumeration here: `(0, 0)` where nothing is excluded,
/// else the generation of the excluded ranges and the excluded capture's name, hashed.
pub(super) fn exclusion_key() -> (u64, u64) {
    if !ACTIVE.get() {
        return (0, 0);
    }
    EXCLUDED.with_borrow(|(name, _)| {
        (
            GENERATION.get(),
            std::hash::BuildHasher::hash_one(&rustc_hash::FxBuildHasher, name.as_str()),
        )
    })
}

/// Whether bindings of `name` are being excluded here.
pub(super) fn excludes(name: &str) -> bool {
    ACTIVE.get() && EXCLUDED.with_borrow(|(excluded, _)| excluded == name)
}

#[cfg(test)]
thread_local! {
    static EXCLUSION_PROBES: Cell<usize> = const { Cell::new(0) };
}

/// A sub-tree's rule being matched: while it lives, bindings read through its coordinates. `parse`
/// and `formatArgs`' `missing` (M11a) enter one around their sub-tree's match.
pub(crate) struct HostRanges;

impl HostRanges {
    pub(crate) fn enter(map: HostMap) -> Self {
        HOSTS.with_borrow_mut(|hosts| hosts.push(map));
        Self
    }
}

impl Drop for HostRanges {
    fn drop(&mut self) {
        HOSTS.with_borrow_mut(|hosts| hosts.pop());
    }
}

/// Whether bindings are being excluded here.
pub(super) fn active() -> bool {
    ACTIVE.get()
}

/// A capture's binding as a range of the host file: a node capture's range, mapped out of the
/// sub-trees being matched, or an exported capture's host range.
pub(crate) fn binding_range<D: Doc>(env: &MetaVarEnv<'_, D>, name: &str) -> Option<Range<usize>> {
    match env.get_match(name) {
        Some(node) => HOSTS.with_borrow(|hosts| {
            let Some((innermost, outer)) = hosts.split_last() else {
                return Some(node.range());
            };
            let range = (innermost.node)(node.node_id())?;
            outer
                .iter()
                .rev()
                .try_fold(range, |range, host| (host.span)(range))
        }),
        None => {
            let range = exported_span(env, name)?;
            HOSTS.with_borrow(|hosts| {
                hosts
                    .iter()
                    .rev()
                    .try_fold(range, |range, host| (host.span)(range))
            })
        }
    }
}

/// Whether the bindings bind the enumerated capture to a range already reported.
pub(super) fn excluded<D: Doc>(env: &MetaVarEnv<'_, D>) -> bool {
    EXCLUDED.with_borrow(|(name, ranges)| {
        let Some(range) = binding_range(env, name) else {
            return false;
        };
        #[cfg(test)]
        EXCLUSION_PROBES.set(EXCLUSION_PROBES.get() + 1);
        ranges.contains(&(range.start, range.end))
    })
}

/// Runs `inner` with exclusion suspended: its bindings are its own.
#[inline]
pub(super) fn suspended<T>(inner: impl FnOnce() -> T) -> T {
    if !ACTIVE.get() {
        return inner();
    }
    let saved = ACTIVE.replace(false);
    let result = inner();
    ACTIVE.set(saved);
    result
}

/// Runs a parameterized call's utility (`inner`) in its frame: the enumerated capture is excluded
/// there under the parameter it is passed as, and nothing is excluded where it is not passed.
#[inline]
pub(super) fn framed<T>(args: &[(String, String)], inner: impl FnOnce() -> T) -> T {
    if !ACTIVE.get() {
        return inner();
    }
    let param = EXCLUDED.with_borrow(|(name, _)| {
        args.iter()
            .find(|(_, caller)| caller == name)
            .map(|(param, _)| param.clone())
    });
    let Some(param) = param else {
        return suspended(inner);
    };
    let caller = EXCLUDED.with_borrow_mut(|(name, _)| std::mem::replace(name, param));
    let result = inner();
    EXCLUDED.with_borrow_mut(|(name, _)| *name = caller);
    result
}

/// An enumeration in progress; dropping it ends the exclusion, also on an early return.
pub(super) struct Enumeration;

impl Enumeration {
    pub(super) fn start(name: &str) -> Self {
        EXCLUDED.with_borrow_mut(|(known, ranges)| {
            known.clear();
            known.push_str(name);
            ranges.clear();
        });
        GENERATION.set(GENERATION.get() + 1);
        ACTIVE.set(true);
        Self
    }

    /// Records a match's binding; `false` where it binds nothing new (unbound or already seen),
    /// which ends the enumeration.
    pub(super) fn record(&self, range: Option<Range<usize>>) -> bool {
        let Some(range) = range else {
            return false;
        };
        let inserted =
            EXCLUDED.with_borrow_mut(|(_, ranges)| ranges.insert((range.start, range.end)));
        if inserted {
            GENERATION.set(GENERATION.get() + 1);
        }
        inserted
    }
}

impl Drop for Enumeration {
    fn drop(&mut self) {
        ACTIVE.set(false);
        EXCLUDED.with_borrow_mut(|(_, ranges)| ranges.clear());
        GENERATION.set(GENERATION.get() + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::{HostMap, HostRanges, binding_range};
    use crate::code_rule_dsl::CompiledCodeRuleDsl;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::test_support::*;
    use crate::code_rule_dsl::violations;
    use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
    use ast_grep_language::LanguageExt;
    use serde_json::{Value, json};
    use std::sync::Arc;

    fn compile(rule: Value, each: Option<&str>, capabilities: &Value) -> CompiledCodeRuleDsl {
        compile_with(json!({ "rule": rule }), each, capabilities)
            .unwrap_or_else(|error| panic!("{}", error.reason))
    }

    fn compile_with(
        ast: Value,
        each: Option<&str>,
        capabilities: &Value,
    ) -> napi::Result<CompiledCodeRuleDsl> {
        let mut ast = ast;
        ast["language"] = json!("typescript");
        let mut violation = violation();
        if let Some(each) = each {
            violation["each"] = json!(each);
        }
        compile_document_with(
            clause_document(json!({ "id": "c", "forbid": { "ast": ast }, "violation": violation })),
            capabilities,
        )
    }

    fn defaults() -> Value {
        serde_json::from_str(CAPABILITIES).expect("capabilities")
    }

    /// Violations at one witness sort by id; the texts sorted.
    fn texts(compiled: &CompiledCodeRuleDsl, source: &str) -> Vec<String> {
        let mut texts = matched_texts(
            &evaluate_file(compiled, "typescript", "a.ts", source),
            source,
        );
        texts.sort();
        texts
    }

    fn ids(result: &Value) -> Vec<String> {
        result["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|violation| violation["id"].as_str().expect("id").to_owned())
            .collect()
    }

    #[test]
    fn each_reports_one_violation_per_distinct_binding() {
        let rule = json!({ "kind": "call_expression", "has": { "kind": "arguments", "has": { "kind": "identifier", "pattern": "$X" } } });
        let source = "f(a, b, a);\ng();\n";
        let plain = evaluate_file(
            &compile(rule.clone(), None, &defaults()),
            "typescript",
            "a.ts",
            source,
        );
        let each = evaluate_file(
            &compile(rule, Some("X"), &defaults()),
            "typescript",
            "a.ts",
            source,
        );
        assert_eq!(matched_texts(&plain, source), ["f(a, b, a) X=a"]);
        let mut each_texts = matched_texts(&each, source);
        each_texts.sort();
        assert_eq!(
            each_texts,
            ["f(a, b, a) X=a", "f(a, b, a) X=a", "f(a, b, a) X=b"]
        );
        let each_ids = ids(&each);
        assert_eq!(
            each_ids
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3
        );
        // Without `each` a violation's id is the witness's, as before.
        assert!(!each_ids.contains(&ids(&plain)[0]));
        // A binding at the root has one value: one violation.
        let root = compile(json!({ "pattern": "f($X, $$$)" }), Some("X"), &defaults());
        assert_eq!(
            matched_texts(&evaluate_file(&root, "typescript", "a.ts", source), source),
            ["f(a, b, a) X=a"]
        );
        // A match that leaves the capture unbound is reported once, and only without a bound one.
        let optional = compile(
            json!({ "kind": "call_expression", "any": [
                { "has": { "kind": "arguments", "has": { "kind": "identifier", "pattern": "$X" } } },
                { "kind": "call_expression" },
            ] }),
            Some("X"),
            &defaults(),
        );
        assert_eq!(
            texts(&optional, source),
            ["f(a, b, a) X=a", "f(a, b, a) X=a", "f(a, b, a) X=b", "g()"]
        );
    }

    #[test]
    fn each_explores_branches_and_nested_relations_but_not_negations() {
        let source = "f(g(a), g(b));\n";
        // Each `has` on the path is a choice: every identifier two levels down is reached.
        let nested = compile(
            json!({ "kind": "arguments", "inside": { "kind": "call_expression", "regex": "^f" },
                "has": { "kind": "call_expression", "has": { "kind": "arguments", "has": { "kind": "identifier", "pattern": "$X" } } } }),
            Some("X"),
            &defaults(),
        );
        assert_eq!(
            texts(&nested, source),
            ["(g(a), g(b)) X=a", "(g(a), g(b)) X=b"]
        );
        // `any` branches bind in turn.
        let branches = compile(
            json!({ "pattern": "f($P, $Q)", "any": [
                { "has": { "stopBy": "end", "regex": "^a$", "pattern": "$X" } },
                { "has": { "stopBy": "end", "regex": "^b$", "pattern": "$X" } },
            ] }),
            Some("X"),
            &defaults(),
        );
        assert_eq!(
            texts(&branches, source)
                .iter()
                .map(|text| text.rsplit(' ').next().unwrap_or_default().to_owned())
                .collect::<Vec<_>>(),
            ["X=a", "X=b"]
        );
        // A negation binds for itself: it excludes nothing.
        let negated = compile(
            json!({ "kind": "call_expression", "regex": "^f",
                "has": { "stopBy": "end", "kind": "identifier", "pattern": "$X" },
                "not": { "has": { "stopBy": "end", "kind": "identifier", "regex": "^z$", "pattern": "$X" } } }),
            Some("X"),
            &defaults(),
        );
        assert_eq!(
            texts(&negated, source)
                .iter()
                .map(|text| text.rsplit(' ').next().unwrap_or_default().to_owned())
                .collect::<Vec<_>>(),
            ["X=a", "X=b", "X=f", "X=g", "X=g"]
        );
    }

    #[test]
    fn bindings_read_as_host_ranges() {
        let root = GrammarDialect::TypeScript.ast_grep("f(abc);\n");
        let node = root
            .root()
            .find("abc")
            .expect("identifier")
            .get_node()
            .clone();
        let mut env = MetaVarEnv::new();
        env.insert("C", node.clone());
        assert_eq!(binding_range(&env, "C"), Some(2..5));
        // Inside a sub-tree, a node reads through the sub-tree's coordinates, innermost first.
        let shift = |by: usize| HostMap {
            node: Arc::new(move |_| Some(10 + by..11 + by)),
            span: Arc::new(move |range: std::ops::Range<usize>| {
                Some(range.start + by..range.end + by)
            }),
        };
        {
            let _outer = HostRanges::enter(shift(100));
            let _inner = HostRanges::enter(shift(0));
            assert_eq!(binding_range(&env, "C"), Some(110..111));
        }
        assert_eq!(binding_range(&env, "C"), Some(2..5));
        // An exported capture's reserved span variable (M11a's `parse`), mapped out as well.
        let mut exported = MetaVarEnv::<ast_grep_core::tree_sitter::StrDoc<GrammarDialect>>::new();
        let span = "\0E".to_owned();
        exported.insert_transformation(
            &MetaVariable::Capture(span.clone(), false),
            &span,
            b"5:7".to_vec(),
        );
        assert_eq!(binding_range(&exported, "E"), Some(5..7));
        let _host = HostRanges::enter(shift(20));
        assert_eq!(binding_range(&exported, "E"), Some(25..27));
    }

    /// `each` naming a named group of a regex that tests the witness itself: one violation per
    /// match of the regex that binds the group, with that match's groups in the message, the
    /// anchor and `fix.range`, and an id per range. A metavariable of the same name wins.
    #[test]
    fn each_enumerates_a_regex_groups_matches_in_the_witness_text() {
        let rule = json!({ "kind": "string", "regex": "(?<D>[0-9])(?<N>[a-z])?" });
        let ast = |violation: Value| {
            compile_document_with(
                clause_document(json!({ "id": "c", "forbid": { "ast": {
                    "language": "typescript", "rule": rule } }, "violation": violation })),
                &defaults(),
            )
            .unwrap_or_else(|error| panic!("{}", error.reason))
        };
        let source = "f('1a2b3', 'x');\n";
        let result = evaluate_file(
            &ast(
                json!({ "anchor": "capture.D", "code": "X", "message": "digit $D then $N",
                         "each": "D",
                         "fix": { "template": "#", "range": "capture.D" } }),
            ),
            "typescript",
            "a.ts",
            source,
        );
        let found = result["violations"].as_array().expect("findings");
        let mut rows = found
            .iter()
            .map(|violation| {
                let at = |span: &Value| {
                    let start = span["startByte"].as_u64().expect("start") as usize;
                    let end = span["endByte"].as_u64().expect("end") as usize;
                    source[start..end].to_owned()
                };
                (
                    violation["span"]["startByte"].as_u64().expect("start"),
                    at(&violation["span"]),
                    violation["message"].as_str().expect("message").to_owned(),
                    at(&violation["fix"]["span"]),
                )
            })
            .collect::<Vec<_>>();
        rows.sort();
        assert_eq!(
            rows,
            [
                (
                    3,
                    "1".to_owned(),
                    "digit 1 then a".to_owned(),
                    "1".to_owned()
                ),
                (
                    5,
                    "2".to_owned(),
                    "digit 2 then b".to_owned(),
                    "2".to_owned()
                ),
                (
                    7,
                    "3".to_owned(),
                    "digit 3 then ".to_owned(),
                    "3".to_owned()
                ),
            ]
        );
        assert_eq!(
            ids(&result)
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3
        );
        // Every match binds its own groups; a witness where no match binds the group reports
        // nothing (no-irregular-whitespace's walk over a program that finds no run).
        let plain = json!({ "anchor": "witness.span", "code": "X", "message": "m",
                            "each": "N" });
        let result = evaluate_file(&ast(plain), "typescript", "a.ts", "f('1', '2a3b');\n");
        let mut starts = result["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|violation| violation["captures"]["N"][0]["startByte"].as_u64())
            .collect::<Vec<_>>();
        starts.sort();
        assert_eq!(starts, [Some(9), Some(11)]);
        // A metavariable of the group's name is the binding `each` reads.
        let shadowed = compile(
            json!({ "kind": "call_expression", "regex": "(?<X>[0-9])", "has": { "stopBy": "end", "kind": "identifier", "pattern": "$X" } }),
            Some("X"),
            &defaults(),
        );
        assert_eq!(texts(&shadowed, "f(1, 2);\n"), ["f(1, 2) X=f"]);
        // A group under a relation describes another node: `each` cannot name it.
        assert_eq!(
            rejection(compile_with(
                json!({ "rule": { "kind": "call_expression", "has": { "kind": "number", "regex": "(?<G>1)" } } }),
                Some("G"),
                &defaults()
            )),
            "/clauses/0/violation/each: `$G` names no capture; the query binds no captures"
        );
    }

    #[test]
    fn each_is_capped_by_the_witness_budget() {
        let capabilities = capabilities_with(|capabilities| {
            capabilities["limits"]["maxWitnesses"] = json!(2);
        });
        let rule = json!({ "kind": "call_expression", "has": { "stopBy": "end", "kind": "identifier", "pattern": "$X" } });
        let result = evaluate_file(
            &compile(rule, Some("X"), &capabilities),
            "typescript",
            "a.ts",
            "f(a, b, c);\n",
        );
        assert_eq!(result["violations"], json!([]), "{result}");
        assert_eq!(result["failures"][0]["code"], "witness-limit", "{result}");
    }

    #[test]
    fn each_names_a_single_capture_of_an_ast_query() {
        let rule = json!({ "pattern": "f($X, $$$Y)" });
        for (each, expected) in [
            (
                "Z",
                "/clauses/0/violation/each: `$Z` names no capture; the query can bind: X, Y",
            ),
            (
                "Y",
                "/clauses/0/violation/each: `$$$Y` captures a node list; each enumerates single-node captures",
            ),
            (
                "$X",
                "/clauses/0/violation/each: `$X` is not a metavariable name (A-Z, 0-9, _, without `$`)",
            ),
        ] {
            assert_eq!(
                rejection(compile_with(
                    json!({ "rule": rule }),
                    Some(each),
                    &defaults()
                )),
                expected
            );
        }
        let regex = clause_document(json!({
            "id": "c",
            "forbid": { "regex": { "source": "file.content", "pattern": "(?<X>a)" } },
            "violation": { "anchor": "witness.span", "code": "X", "message": "x", "each": "X" },
        }));
        assert_eq!(
            rejection(compile_document_with(regex, &defaults())),
            "/clauses/0/violation/each: each enumerates the captures of an ast query"
        );
    }

    #[test]
    fn each_exclusion_probes_are_linear_in_distinct_bindings() {
        const BINDINGS: usize = 500;
        let source = format!(
            "f({});\n",
            (0..BINDINGS)
                .map(|index| format!("x{index}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let rule = json!({
            "kind": "call_expression",
            "has": {
                "kind": "arguments",
                "has": { "kind": "identifier", "pattern": "$X" }
            }
        });
        super::EXCLUSION_PROBES.set(0);
        let result = evaluate_file(
            &compile(rule, Some("X"), &defaults()),
            "typescript",
            "a.ts",
            &source,
        );
        assert_eq!(
            result["violations"].as_array().map(Vec::len),
            Some(BINDINGS)
        );
        // Each probe is one lookup in a hash set of the ranges reported so far. Round `k` passes
        // over the `k` bindings reported before it, probing each once, and probes the one it
        // reports: at most `k + 2` probes, for the `BINDINGS` rounds and the final one.
        let probes = super::EXCLUSION_PROBES.get();
        assert!(probes >= BINDINGS, "every enumeration round probes the set");
        assert!(
            probes <= BINDINGS * (BINDINGS + 1) / 2 + 2 * (BINDINGS + 1),
            "a round must probe each binding it passes over once: {probes} probes"
        );
    }

    /// A violation with `suppress` reads past bindings a comment suppresses, yet the enumeration
    /// still stops once the unsuppressed witnesses overflow the allowance: here none is
    /// suppressed, so it reads three of the 500 bindings, as it does without `suppress`.
    #[test]
    fn each_with_suppress_stops_enumerating_at_the_witness_overflow() {
        const BINDINGS: usize = 500;
        let source = format!(
            "f({});\n",
            (0..BINDINGS)
                .map(|index| format!("x{index}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let rule = json!({
            "kind": "call_expression",
            "has": {
                "kind": "arguments",
                "has": { "kind": "identifier", "pattern": "$X" }
            }
        });
        let capabilities = capabilities_with(|capabilities| {
            capabilities["limits"]["maxWitnesses"] = json!(2);
        });
        for suppress in [None, Some(json!({ "eslint": { "rules": ["no-undef"] } }))] {
            let mut violation = violation();
            violation["each"] = json!("X");
            if let Some(suppress) = &suppress {
                violation["suppress"] = suppress.clone();
            }
            let compiled = compile_document_with(
                clause_document(json!({ "id": "c", "forbid": { "ast": {
                    "language": "typescript", "rule": rule } }, "violation": violation })),
                &capabilities,
            )
            .unwrap_or_else(|error| panic!("{}", error.reason));
            super::EXCLUSION_PROBES.set(0);
            let result = evaluate_file(&compiled, "typescript", "a.ts", &source);
            assert_eq!(result["violations"], json!([]), "{suppress:?}: {result}");
            assert_eq!(
                result["failures"][0]["code"], "witness-limit",
                "{suppress:?}: {result}"
            );
            // Three rounds (the allowance and its overflow), each probing the bindings before it.
            let probes = super::EXCLUSION_PROBES.get();
            assert!(
                probes <= 3 * 4 / 2 + 2 * 4,
                "{suppress:?}: the enumeration read past the overflow: {probes} probes"
            );
        }
    }

    #[test]
    fn each_reuses_capture_text_shared_by_every_regex_group_result() {
        const ITEMS: usize = 500;
        const PAYLOAD_BYTES: usize = 16 * 1024;
        let source = format!(
            "// {}{}\n",
            "x".repeat(ITEMS),
            "z".repeat(PAYLOAD_BYTES - ITEMS)
        );
        let rule = json!({
            "kind": "comment",
            "all": [
                { "regex": "(?<WHOLE>[\\s\\S]*)" },
                { "regex": "(?<ITEM>x)" }
            ]
        });
        let compiled = compile(rule, Some("ITEM"), &defaults());
        let result = evaluate_file(&compiled, "typescript", "a.ts", &source);
        assert_eq!(result["violations"].as_array().map(Vec::len), Some(ITEMS));
        let root = GrammarDialect::TypeScript.ast_grep(&source);
        let comment = root
            .root()
            .dfs()
            .find(|node| node.kind() == "comment")
            .expect("comment");
        let matched = ast_grep_core::NodeMatch::new(comment, MetaVarEnv::new());
        let mut sets = Vec::new();
        violations::each_node_group_set(
            &compiled.inner.clauses[0].violation,
            GrammarDialect::TypeScript,
            &matched,
            || false,
            |groups| {
                sets.push(groups);
                true
            },
        );
        assert_eq!(sets.len(), ITEMS);
        let whole = |set: &[violations::NodeGroup]| {
            set.iter()
                .find(|(name, _, _)| name == "WHOLE")
                .map(|(_, text, _)| std::sync::Arc::clone(text))
                .expect("every set holds WHOLE")
        };
        let first = whole(&sets[0]);
        assert_eq!(first.len(), source.trim_end().len());
        assert!(
            sets.iter()
                .all(|set| std::sync::Arc::ptr_eq(&whole(set), &first)),
            "every each result shares the one WHOLE capture text"
        );
    }

    #[test]
    fn each_exports_only_the_groups_a_transform_reads() {
        const ITEMS: usize = 500;
        const PAYLOAD_BYTES: usize = 16 * 1024;
        let source = format!(
            "// {}{}\n",
            "x".repeat(ITEMS),
            "z".repeat(PAYLOAD_BYTES - ITEMS)
        );
        let ast = json!({
            "rule": {
                "kind": "comment",
                "all": [
                    { "regex": "(?<WHOLE>[\\s\\S]*)" },
                    { "regex": "(?<ITEM>x)" }
                ]
            },
            "transform": { "LOUD": { "convert": { "source": "$ITEM", "toCase": "upperCase" } } }
        });
        let mut violation = violation();
        violation["each"] = json!("ITEM");
        violation["message"] = json!("item $LOUD");
        let compiled = compile_document_with(
            clause_document(json!({ "id": "c", "forbid": { "ast": {
                "language": "typescript", "rule": ast["rule"], "transform": ast["transform"] } },
                "violation": violation })),
            &defaults(),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        crate::code_rule_dsl::matcher::exported::reset_exported_text_bytes();
        let result = evaluate_file(&compiled, "typescript", "a.ts", &source);
        let messages = result["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|violation| violation["message"].as_str().expect("message"))
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), ITEMS, "{result}");
        assert!(messages.iter().all(|message| *message == "item X"));
        assert!(
            crate::code_rule_dsl::matcher::exported::exported_text_bytes() <= ITEMS * 4,
            "the transform reads ITEM only: WHOLE must not be copied into each result's \
             environment ({} bytes exported)",
            crate::code_rule_dsl::matcher::exported::exported_text_bytes()
        );
    }

    #[test]
    fn each_runs_a_transform_of_a_shared_group_once_per_witness() {
        const ITEMS: usize = 500;
        const PAYLOAD_BYTES: usize = 16 * 1024;
        let source = format!(
            "// {}{}\n",
            "x".repeat(ITEMS),
            "z".repeat(PAYLOAD_BYTES - ITEMS)
        );
        let rule = json!({
            "kind": "comment",
            "all": [
                { "regex": "(?<WHOLE>[\\s\\S]*)" },
                { "regex": "(?<ITEM>x)" }
            ]
        });
        // TAIL reads WHOLE, which every each result shares; LOUD reads each result's own ITEM.
        let transform = json!({
            "TAIL": { "substring": { "source": "$WHOLE", "startChar": -3 } },
            "LOUD": { "convert": { "source": "$ITEM", "toCase": "upperCase" } }
        });
        let mut violation = violation();
        violation["each"] = json!("ITEM");
        violation["message"] = json!("item $LOUD $TAIL");
        let compiled = compile_document_with(
            clause_document(json!({ "id": "c", "forbid": { "ast": {
                "language": "typescript", "rule": rule, "transform": transform } },
                "violation": violation })),
            &defaults(),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        crate::code_rule_dsl::matcher::exported::reset_exported_text_bytes();
        let result = evaluate_file(&compiled, "typescript", "a.ts", &source);
        let messages = result["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|violation| violation["message"].as_str().expect("message"))
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), ITEMS, "{result}");
        assert!(
            messages.iter().all(|message| *message == "item X zzz"),
            "{messages:?}"
        );
        let exported = crate::code_rule_dsl::matcher::exported::exported_text_bytes();
        assert!(
            exported <= source.len() + ITEMS * 4,
            "WHOLE is the same in every each result: it is exported once per witness, not once \
             per result ({exported} bytes exported)"
        );
    }

    /// One comment reporting an each result per `x`, with a transform reading the WHOLE group
    /// every result shares and the ITEM group each result holds alone.
    fn shared_group_transform(items: usize) -> (CompiledCodeRuleDsl, String) {
        let source = format!("// {}zzz\n", "x".repeat(items));
        let rule = json!({
            "kind": "comment",
            "all": [
                { "regex": "(?<WHOLE>[\\s\\S]*)" },
                { "regex": "(?<ITEM>x)" }
            ]
        });
        let transform = json!({
            "TAIL": { "substring": { "source": "$WHOLE", "startChar": -3 } },
            "LOUD": { "convert": { "source": "$ITEM", "toCase": "upperCase" } }
        });
        let mut violation = violation();
        violation["each"] = json!("ITEM");
        violation["message"] = json!("item $LOUD $TAIL");
        let compiled = compile_document_with(
            clause_document(json!({ "id": "c", "forbid": { "ast": {
                "language": "typescript", "rule": rule, "transform": transform } },
                "violation": violation })),
            &defaults(),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        (compiled, source)
    }

    #[test]
    fn each_decides_which_groups_are_shared_in_one_pass_over_the_results() {
        const ITEMS: usize = 1000;
        let (compiled, source) = shared_group_transform(ITEMS);
        crate::code_rule_dsl::eval::reset_shared_group_lookups();
        let result = evaluate_file(&compiled, "typescript", "a.ts", &source);
        let lookups = crate::code_rule_dsl::eval::shared_group_lookups();
        let messages = result["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|violation| violation["message"].as_str().expect("message"))
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), ITEMS, "{result}");
        assert!(
            messages.iter().all(|message| *message == "item X zzz"),
            "{messages:?}"
        );
        // Two groups are read: WHOLE, the same in all ITEMS results, and ITEM, which differs from
        // the second result on. Deciding both once looks each up at most once per result.
        assert!(
            lookups <= 2 * ITEMS,
            "deciding which of 2 groups {ITEMS} each results share must look a group up at most \
             {} times, not once per result per result ({lookups} lookups)",
            2 * ITEMS
        );
    }

    #[test]
    fn each_stops_transforming_results_once_the_budget_is_spent() {
        const ITEMS: usize = 1000;
        let (compiled, source) = shared_group_transform(ITEMS);
        let violation = &compiled.inner.clauses[0].violation;
        let root = GrammarDialect::TypeScript.ast_grep(&source);
        let comment = root
            .root()
            .dfs()
            .find(|node| node.kind() == "comment")
            .expect("comment");
        let matched = ast_grep_core::NodeMatch::new(comment, MetaVarEnv::new());
        let polls = std::cell::Cell::new(0_usize);
        let poll = || {
            polls.set(polls.get() + 1);
            false
        };
        let mut sets = Vec::new();
        violations::each_node_group_set(
            violation,
            GrammarDialect::TypeScript,
            &matched,
            poll,
            |groups| {
                sets.push(groups);
                true
            },
        );
        assert_eq!(sets.len(), ITEMS);
        // The budget is spent just after every result is enumerated: the results' transforms are
        // all that remains, and they must notice it.
        let enumeration_polls = polls.replace(0);
        let file = crate::code_rule_dsl::model::FileInput::new(
            "a.ts".to_owned(),
            "typescript".to_owned(),
            std::sync::Arc::new(source.clone()),
            None,
            "file",
        )
        .expect("file input");
        let witnesses = crate::code_rule_dsl::eval::witnesses_from_match(
            &file,
            &matched,
            GrammarDialect::TypeScript,
            violation,
            ITEMS + 1,
            || {
                polls.set(polls.get() + 1);
                polls.get() > enumeration_polls
            },
            |start, end| crate::code_rule_dsl::violations::Span {
                start_byte: start,
                end_byte: end,
                ..Default::default()
            },
            |_| true,
        );
        let Ok(witnesses) = witnesses else {
            panic!("a spent budget cuts the results short; it is no witness error");
        };
        assert!(
            witnesses.is_empty(),
            "no result may be transformed after the budget is found spent ({} of {ITEMS} were)",
            witnesses.len()
        );
    }
}
