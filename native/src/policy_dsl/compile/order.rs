//! The order of a rule object's terms by capture dependency: a term that reads a capture a later
//! sibling binds runs after that sibling, so a comparison inside one relation (or a `not`, an
//! `any`) sees what another relation of the same rule object bound.

use crate::policy_dsl::json::invalid;
use crate::policy_dsl::matcher::{AstMatcher, CaptureAnalysis};
use napi::Result;
use std::collections::BTreeSet;
/// An explicit `all` keeps its written order (ast-grep's, and the sequencing tool capture ordering
/// leaves to authors): an item that reads a capture which only a later item binds would read it
/// unbound, so it is refused, naming the reorder. `context` holds what the rule object's other terms
/// bind, which runs before the `all` (atomic terms first, siblings by capture ordering).
pub(super) fn all_order_conflicts(
    items: &[AstMatcher],
    context: &BTreeSet<String>,
    path: &str,
    captures: &mut CaptureAnalysis,
) -> Vec<AllOrderConflict> {
    let binds = items
        .iter()
        .map(|item| captures.binds(item))
        .collect::<Vec<_>>();
    let mut conflicts = Vec::new();
    let mut bound = context.clone();
    for (index, item) in items.iter().enumerate() {
        for name in captures.reads(item).difference(&bound) {
            let later = binds[index + 1..]
                .iter()
                .position(|later| later.contains(name));
            if let Some(later) = later {
                conflicts.push(AllOrderConflict {
                    path: path.to_owned(),
                    item: index,
                    later: index + 1 + later,
                    name: name.clone(),
                });
            }
        }
        bound.extend(binds[index].iter().cloned());
    }
    conflicts
}

/// An `all` item reading `name`, which a later item binds: refused unless something outside those
/// items (an enclosing rule, a caller) binds it too, which the query checks once it is compiled.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::policy_dsl) struct AllOrderConflict {
    /// The rule object holding the `all`.
    pub(in crate::policy_dsl) path: String,
    pub(in crate::policy_dsl) item: usize,
    pub(in crate::policy_dsl) later: usize,
    pub(in crate::policy_dsl) name: String,
}

impl AllOrderConflict {
    pub(in crate::policy_dsl) fn error(&self) -> napi::Error {
        let (path, item, later, name) = (&self.path, self.item, self.later, &self.name);
        invalid(format!(
            "{path}/all/{item} reads `${name}`, which only {path}/all/{later} binds, after it: \
             an `all` runs its items in the order written; move all/{later} before all/{item}"
        ))
    }
}

/// Orders `terms[from..]`, each with its path, so that a term reading a capture runs after every
/// sibling that binds it, unless a term before `from` binds it already. Terms keep their order
/// otherwise. A cycle (two terms each reading what the other binds) is a compile-time error.
pub(super) fn order_by_captures(
    terms: &mut Vec<(AstMatcher, String)>,
    from: usize,
    path: &str,
    captures: &mut CaptureAnalysis,
) -> Result<()> {
    let early = terms[..from]
        .iter()
        .flat_map(|(term, _)| captures.binds(term))
        .collect::<BTreeSet<_>>();
    let group = &terms[from..];
    let bound = group
        .iter()
        .map(|(term, _)| captures.binds(term))
        .collect::<Vec<_>>();
    // `reads` already leaves out what a term binds for itself (an `any` binds only in the branch
    // that matches, so its other branches still read).
    let needs = group
        .iter()
        .map(|(term, _)| {
            captures
                .reads(term)
                .into_iter()
                .filter(|name| !early.contains(name))
                .collect::<BTreeSet<_>>()
        })
        .collect::<Vec<_>>();
    // `after[i]`: the siblings that must run before term `i`, with a name each binds for it.
    let after = needs
        .iter()
        .enumerate()
        .map(|(index, need)| {
            bound
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .filter_map(|(other, binds)| {
                    need.iter()
                        .find(|name| binds.contains(*name))
                        .map(|name| (other, name.clone()))
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    if after.iter().all(Vec::is_empty) {
        return Ok(());
    }
    let mut placed = vec![false; group.len()];
    let mut order = Vec::with_capacity(group.len());
    while order.len() < group.len() {
        let Some(next) = (0..group.len()).find(|index| {
            !placed[*index] && after[*index].iter().all(|(before, _)| placed[*before])
        }) else {
            let stuck = (0..group.len())
                .find(|index| !placed[*index])
                .expect("an unplaced term");
            let (other, name) = after[stuck]
                .iter()
                .find(|(before, _)| !placed[*before])
                .expect("an unplaced dependency");
            let back = after[*other]
                .iter()
                .find(|(before, _)| !placed[*before])
                .map_or_else(String::new, |(_, name)| format!("`${name}`"));
            return Err(invalid(format!(
                "{path}: {} reads `${name}`, which {} binds, and {} reads {back}, which {} binds; \
                 no order binds both first. Bind one of them in an earlier term (a pattern, or an \
                 `all` whose first item binds it)",
                group[stuck].1, group[*other].1, group[*other].1, group[stuck].1
            )));
        };
        placed[next] = true;
        order.push(next);
    }
    let mut taken = terms
        .split_off(from)
        .into_iter()
        .map(Some)
        .collect::<Vec<_>>();
    for index in order {
        terms.push(taken[index].take().expect("each term once"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::policy_dsl::test_support::*;
    use serde_json::{Value, json};

    fn compile_python_rule(rule: Value) -> napi::Result<crate::policy_dsl::CompiledPolicyDsl> {
        compile_document_with(
            check_document(json!({ "id": "ast-match", "violation": finding(),
                                   "forbid": { "ast": { "language": "python", "rule": rule } } })),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
    }

    fn python_rule(rule: Value) -> crate::policy_dsl::CompiledPolicyDsl {
        compile_python_rule(rule).unwrap_or_else(|error| panic!("{}", error.reason))
    }

    fn texts(rule: Value, source: &str, words: usize) -> Vec<String> {
        let compiled =
            compile_typescript_rule(rule).unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        matched_texts(&result, source)
            .into_iter()
            .map(|text| text.split(' ').take(words).collect::<Vec<_>>().join(" "))
            .collect()
    }

    /// The ESLint suite's repro: a capture bound in `has` through a binding's value is read by a
    /// comparison inside a sibling `inside`, which runs first in ast-grep's order.
    #[test]
    fn a_relation_reading_a_sibling_relations_capture_runs_after_it() {
        let key = |name: &str| json!({ "stopBy": "neighbor", "kind": "computed_property_name", "has": { "stopBy": "neighbor", "kind": "identifier", "resolves": { "binding": { "value": { "pattern": format!("${name}") } } } } });
        let rule = json!({
            "kind": "method_definition",
            "has": key("GV"),
            "inside": { "kind": "class_body", "stopBy": "neighbor", "has": {
                "stopBy": "neighbor", "kind": "method_definition", "has": key("SV"),
                "compare": { "left": "GV", "op": "eq", "right": "SV", "text": "source" } } },
        });
        let source = "let p1='p1'; class M { set [p1](v){} get [p1](){ return ''; } }\n";
        assert_eq!(
            texts(rule.clone(), source, 2),
            ["set [p1](v){}", "get [p1](){"]
        );
        // Two names bound to different values do not compare equal.
        let source = "let p1='p1', p2='p2'; class M { set [p1](v){} get [p2](){ return ''; } }\n";
        let mut different = rule;
        different["inside"]["has"]["compare"]["op"] = json!("ne");
        assert_eq!(
            texts(different, source, 2),
            ["set [p1](v){}", "get [p2](){"]
        );
    }

    /// A negation or a disjunction reading what a sibling relation binds runs after it.
    #[test]
    fn composites_reading_a_sibling_relations_capture_run_after_it() {
        let earlier =
            json!({ "stopBy": "end", "kind": "shorthand_property_identifier", "pattern": "$PREV" });
        let base = |extra: Value| {
            let mut rule = json!({ "kind": "shorthand_property_identifier", "pattern": "$KEY", "follows": earlier });
            rule.as_object_mut()
                .expect("object")
                .extend(extra.as_object().expect("object").clone());
            rule
        };
        // Keys that sort before the key right before them.
        let unsorted =
            base(json!({ "not": { "compare": { "left": "PREV", "op": "le", "right": "KEY" } } }));
        let source = "const o = { b, a, c, a };\n";
        assert_eq!(
            texts(unsorted, source, 3),
            ["a KEY=a PREV=b", "a KEY=a PREV=c"]
        );
        let either = base(json!({ "any": [
            { "compare": { "left": "PREV", "op": "gt", "right": "KEY" } },
            { "regex": "^c$", "compare": { "left": "PREV", "op": "eq", "right": "KEY" } },
        ] }));
        let source = "const o = { b, a, c, c };\n";
        assert_eq!(
            texts(either, source, 3),
            ["a KEY=a PREV=b", "c KEY=c PREV=c"]
        );
    }

    /// The ESLint suite's M10 run 2 (no-invalid-void-type): an `any` one of whose branches reads a
    /// capture a sibling binds runs after that sibling, even where another branch binds the same
    /// name (a branch binds only when it is the one that matches), and whether the reading branch
    /// is plain or under `not`.
    #[test]
    fn an_any_reading_a_siblings_capture_runs_after_it_when_a_branch_also_binds_it() {
        let own_name = json!({ "has": { "kind": "member_expression", "has": {
            "kind": "property_identifier", "pattern": "$P",
            "compare": { "left": "N", "op": "eq", "right": "P", "text": "name" } } } });
        let rule = |reader: Value| {
            json!({
                "kind": "variable_declarator",
                "any": [reader, { "has": { "kind": "number", "pattern": "$N" } }],
                "has": { "kind": "identifier", "pattern": "$N" },
            })
        };
        let source = "var foo = o.foo; var bar = o.baz;\n";
        assert_eq!(texts(rule(own_name.clone()), source, 3), ["foo = o.foo"]);
        assert_eq!(
            texts(rule(json!({ "not": own_name })), source, 3),
            ["bar = o.baz"]
        );
    }

    /// M10b's binding terms order with the relations: a relation comparing against what `binds`'
    /// `value` or `resolves.binding`'s `occurrence` binds runs after it, and a cycle through `binds`
    /// is refused. (A pattern's metavariable binds; comparisons, `within` and negations read.)
    #[test]
    fn relations_reading_binds_and_occurrence_captures_run_after_them() {
        // Declarations whose literal value is also some call's argument.
        let binds = json!({
            "kind": "identifier",
            "binds": { "value": { "kind": "number", "pattern": "$V" } },
            "inside": { "kind": "program", "stopBy": "end", "has": { "stopBy": "end", "kind": "arguments",
                        "has": { "stopBy": "neighbor", "pattern": "$W",
                                 "compare": { "left": "W", "op": "eq", "right": "V", "text": "source" } } } },
        });
        let source = "const a = 1;\nconst b = 2;\nf(1);\n";
        assert_eq!(texts(binds, source, 3), ["a V=1 W=1"]);
        // References inside a call that also passes their declaration's literal: `occurrence`
        // binds `$V` at the declaration name, and the sibling `inside` reads it.
        let occurrence = json!({
            "kind": "identifier",
            "inside": { "kind": "call_expression", "stopBy": "end", "has": { "kind": "arguments",
                        "stopBy": "neighbor", "has": { "stopBy": "neighbor", "pattern": "$W",
                        "compare": { "left": "W", "op": "eq", "right": "V", "text": "source" } } } },
            "resolves": { "binding": { "occurrence": { "inside": {
                "kind": "variable_declarator", "stopBy": "neighbor",
                "has": { "stopBy": "neighbor", "kind": "number", "pattern": "$V" } } } } },
        });
        let source = "const a = 1;\nconst b = 2;\nf(a, 1);\nf(b, 1);\n";
        assert_eq!(texts(occurrence.clone(), source, 3), ["a V=1 W=1"]);
        // An `all` keeps its items' order: read before the binding term, `$V` would be unbound, so
        // the rule is refused with the reorder that makes it mean what it says.
        let unordered = json!({ "kind": "identifier", "all": [
            { "inside": occurrence["inside"].clone() }, { "resolves": occurrence["resolves"].clone() },
        ] });
        assert_eq!(
            rejection(compile_typescript_rule(unordered)),
            "/clauses/0/forbid/ast/rule/all/0 reads `$V`, which only /clauses/0/forbid/ast/rule/all/1 binds, after it: \
             an `all` runs its items in the order written; move all/1 before all/0"
        );
        let cycle = json!({
            "kind": "identifier",
            "binds": { "value": { "pattern": "$A", "compare": { "left": "A", "op": "eq", "right": "B" } } },
            "has": { "pattern": "$B", "compare": { "left": "B", "op": "eq", "right": "A" } },
        });
        assert_eq!(
            rejection(compile_typescript_rule(cycle)),
            "/clauses/0/forbid/ast/rule: /clauses/0/forbid/ast/rule/has reads `$A`, which /clauses/0/forbid/ast/rule/binds binds, and /clauses/0/forbid/ast/rule/binds reads `$B`, which /clauses/0/forbid/ast/rule/has binds; no order binds both first. Bind one of them in an earlier term (a pattern, or an `all` whose first item binds it)"
        );
    }

    /// A bare annotation's `binds` term orders like any other: a relation comparing against what
    /// its `annotation` sub-rule binds runs after it, whatever order the rule writes them in.
    #[test]
    fn relations_reading_a_bare_annotations_capture_run_after_it() {
        let compiled = python_rule(json!({
            "kind": "identifier",
            "inside": { "kind": "function_definition", "stopBy": "end", "has": {
                "kind": "type", "stopBy": "neighbor", "pattern": "$R",
                "compare": { "left": "R", "op": "eq", "right": "T", "text": "source" } } },
            "binds": { "form": "annotation", "annotation": { "pattern": "$T" } },
        }));
        let source = "def f() -> int:\n    x: int\n    y: str\n    z: int = 1\n    return 0\n";
        let result = evaluate_file(&compiled, "python", "a.py", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        // `z: int = 1` is `annotated`, not a bare annotation.
        assert_eq!(matched_texts(&result, source), ["x R=int T=int"]);
        let cycle = json!({
            "kind": "identifier",
            "binds": { "form": "annotation", "annotation": { "pattern": "$A", "compare": { "left": "A", "op": "eq", "right": "B" } } },
            "has": { "pattern": "$B", "compare": { "left": "B", "op": "eq", "right": "A" } },
        });
        assert!(
            rejection(compile_python_rule(cycle)).contains("no order binds both first"),
            "a cycle through a bare annotation's capture is refused"
        );
    }

    /// Two sibling relations each reading what the other binds cannot both run first.
    #[test]
    fn a_capture_cycle_between_siblings_is_rejected() {
        let rule = json!({
            "kind": "pair",
            "has": { "kind": "property_identifier", "pattern": "$A", "follows": { "stopBy": "end", "pattern": "$X", "compare": { "left": "X", "op": "eq", "right": "B" } } },
            "precedes": { "stopBy": "end", "kind": "pair", "has": { "kind": "property_identifier", "pattern": "$B", "compare": { "left": "B", "op": "eq", "right": "A" } } },
        });
        assert_eq!(
            rejection(compile_typescript_rule(rule)),
            "/clauses/0/forbid/ast/rule: /clauses/0/forbid/ast/rule/has reads `$B`, which /clauses/0/forbid/ast/rule/precedes binds, and /clauses/0/forbid/ast/rule/precedes reads `$A`, which /clauses/0/forbid/ast/rule/has binds; no order binds both first. Bind one of them in an earlier term (a pattern, or an `all` whose first item binds it)"
        );
    }

    /// An explicit `all` runs its items in the order written: an item reading a capture that only a
    /// later item binds would read it unbound (a comparison false, a negation testing any node), so
    /// the rule could never mean what it says. It is refused, naming the reorder.
    #[test]
    fn an_all_item_reading_what_only_a_later_item_binds_is_refused() {
        let binders = [
            json!({ "has": { "field": "function", "pattern": "$F" } }),
            json!({ "has": { "field": "arguments", "has": { "pattern": "$A", "kind": "identifier" } } }),
        ];
        let compare =
            json!({ "compare": { "left": "F", "op": "eq", "right": "A", "text": "tokens" } });
        assert_eq!(
            rejection(compile_typescript_rule(json!({
                "kind": "call_expression",
                "all": [compare, binders[0], binders[1]],
            }))),
            "/clauses/0/forbid/ast/rule/all/0 reads `$F`, which only /clauses/0/forbid/ast/rule/all/1 binds, after it: \
             an `all` runs its items in the order written; move all/1 before all/0"
        );
        // An earlier item binds it: a later item rebinding it is an equality test, no dependency.
        compile_typescript_rule(json!({
            "kind": "call_expression",
            "all": [binders[0], { "not": { "has": { "field": "arguments", "has": { "pattern": "$F" } } } }, { "has": { "stopBy": "end", "pattern": "$F", "kind": "identifier" } }],
        }))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_eq!(
            rejection(compile_typescript_rule(json!({
                "kind": "call_expression",
                "all": [{ "not": { "has": { "field": "arguments", "has": { "pattern": "$A" } } } }, binders[1]],
            }))),
            "/clauses/0/forbid/ast/rule/all/0 reads `$A`, which only /clauses/0/forbid/ast/rule/all/1 binds, after it: \
             an `all` runs its items in the order written; move all/1 before all/0"
        );
        // A sibling key binding it too runs before the `all`, which reads it before binding it
        // (capture ordering reads an `all` item by item), so the `all` tests what it bound.
        assert_eq!(
            texts(
                json!({
                    "kind": "call_expression",
                    "has": binders[1]["has"],
                    "all": [binders[0], compare, binders[1]],
                }),
                "f(f);\nf(g);\n",
                1
            ),
            ["f(f)"]
        );
        // Nested in a relation's rule, a utility's or a constraint's, the same.
        assert_eq!(
            rejection(compile_typescript_ast(json!({
                "rule": { "kind": "expression_statement", "has": { "matches": "call" } },
                "utils": { "call": { "kind": "call_expression", "all": [compare, binders[0], binders[1]] } },
            }))),
            "/clauses/0/forbid/ast/utils/call/all/0 reads `$F`, which only /clauses/0/forbid/ast/utils/call/all/1 binds, after it: \
             an `all` runs its items in the order written; move all/1 before all/0"
        );
        // Bound outside those items too (a caller, an enclosing rule, a parameter a caller passes),
        // the name may be bound when the `all` runs, and a later item rebinding it only tests it.
        let source = "f(f);\nf(g);\n";
        let called = |caller: Value, util: Value| {
            let compiled = compiled(json!({ "rule": caller, "utils": { "same": util } }));
            matched_texts(
                &evaluate_file(&compiled, "typescript", "a.ts", source),
                source,
            )
        };
        assert_eq!(
            called(
                json!({ "pattern": "$F($A)", "matches": "same" }),
                json!({ "all": [compare, binders[0], binders[1]] })
            ),
            ["f(f) A=f F=f"]
        );
        assert_eq!(
            called(
                json!({ "pattern": "$F($A)", "matches": { "same": { "P": "$A" } } }),
                json!({ "params": ["P"], "rule": { "all": [
                    { "has": { "field": "function", "pattern": "$G" } },
                    { "compare": { "left": "G", "op": "eq", "right": "P", "text": "tokens" } },
                    { "has": { "field": "arguments", "has": { "pattern": "$P", "kind": "identifier" } } },
                ] } })
            ),
            ["f(f) A=f F=f"]
        );
        // Written in order, or bound before the `all` (an atomic term, or a sibling key, which capture
        // ordering runs first), it compiles and matches.
        let source = "f(f);\nf(g);\n";
        for rule in [
            json!({ "kind": "call_expression", "all": [binders[0], binders[1], compare] }),
            json!({ "pattern": "$F($A)", "all": [compare, binders[0], binders[1]] }),
            json!({ "kind": "call_expression", "has": binders[1]["has"], "all": [binders[0], compare] }),
        ] {
            assert_eq!(texts(rule.clone(), source, 1), ["f(f)"], "{rule}");
        }
    }

    /// M11a's keys order with the relations: a relation comparing against what a sibling's
    /// `parse` exports runs after it; `formatArgs` runs after the relation binding its string and
    /// before one reading its `unused` binding; a cycle through `formatArgs` is refused.
    #[test]
    fn relations_order_with_parse_exports_and_format_args() {
        let python = |rule: Value, source: &str| {
            let result = evaluate_file(&python_rule(rule), "python", "a.py", source);
            assert_eq!(result["failures"], json!([]), "{result}");
            matched_texts(&result, source)
        };
        // A string statement equal to a mapping key of a `%` format before it: `has` reads `$K`,
        // which the later-listed `follows` exports from its `parse`.
        let keyed = json!({
            "kind": "expression_statement",
            "has": { "kind": "string", "pattern": "$Q",
                     "compare": { "left": "Q", "op": "eq", "right": "K", "text": "unquoted" } },
            "follows": { "kind": "expression_statement", "stopBy": "end", "has": { "kind": "string", "stopBy": "end",
                         "parse": { "grammar": "percentFormat",
                                    "has": { "kind": "mapping_key", "stopBy": "end", "capture": "K" } } } },
        });
        let source = "print('%(a)s' % d)\n'a'\n'b'\n";
        assert_eq!(python(keyed, source), ["'a' K=a Q='a'"]);
        // A `%` format whose unused key the next statement spells: `formatArgs` reads `$S` and
        // `$R` from `has` and binds `$U`, which `precedes` reads.
        let unused = json!({
            "kind": "expression_statement",
            "has": { "pattern": "$S % $R" },
            "formatArgs": { "style": "percent", "string": "S", "args": "R",
                            "unused": { "kind": "pair", "has": { "kind": "string", "pattern": "$U" } } },
            "precedes": { "kind": "expression_statement", "has": { "kind": "string", "pattern": "$Q",
                          "compare": { "left": "Q", "op": "eq", "right": "U", "text": "source" } } },
        });
        let source = "'%(a)s' % {'a': 1, 'b': 2}\n'b'\n'%(a)s' % {'a': 1, 'c': 2}\n'b'\n";
        assert_eq!(
            python(unused, source),
            ["'%(a)s' % {'a': 1, 'b': 2} Q='b' R={'a': 1, 'b': 2} S='%(a)s' U='b'"]
        );
        let cycle = json!({
            "kind": "expression_statement",
            "has": { "pattern": "$S % $R", "compare": { "left": "S", "op": "ne", "right": "U" } },
            "formatArgs": { "style": "percent", "string": "S", "args": "R", "unused": { "pattern": "$U" } },
        });
        assert!(
            rejection(compile_python_rule(cycle)).contains("no order binds both first"),
            "a cycle through formatArgs is refused"
        );
    }

    /// m10fix2's `any` ordering beside M11a's `parse`: an `any` one of whose branches reads what a
    /// sibling's `parse` exports runs after that sibling, even where another branch's own `parse`
    /// exports the same name, whether the reading branch is plain or under `not`.
    #[test]
    fn an_any_reading_a_siblings_parse_export_runs_after_it_when_a_branch_also_exports_it() {
        let python = |rule: Value, source: &str| {
            let result = evaluate_file(&python_rule(rule), "python", "a.py", source);
            assert_eq!(result["failures"], json!([]), "{result}");
            matched_texts(&result, source)
        };
        let exports = |stop: &str| {
            json!({ "kind": "string", "stopBy": stop, "parse": { "grammar": "percentFormat",
                    "has": { "kind": "mapping_key", "stopBy": "end", "capture": "K" } } })
        };
        let spells = json!({ "has": { "kind": "string", "pattern": "$Q",
            "compare": { "left": "Q", "op": "eq", "right": "K", "text": "unquoted" } } });
        let rule = |reader: Value| {
            json!({
                "kind": "expression_statement",
                "any": [reader, { "has": exports("neighbor") }],
                "follows": { "kind": "expression_statement", "stopBy": "end", "has": exports("end") },
            })
        };
        let source = "print('%(a)s' % d)\n'a'\n'b'\n";
        assert_eq!(python(rule(spells.clone()), source), ["'a' K=a Q='a'"]);
        assert_eq!(python(rule(json!({ "not": spells })), source), ["'b' K=a"]);
    }

    /// M12b's terms in the ordered group: `nthChild` with `ofRule` binds and reads like a relation,
    /// a relation narrowed by `field` is ordered like the relation, and a comparison of a transform
    /// variable reads the captures it is computed from.
    #[test]
    fn m12b_terms_order_by_the_captures_they_read_and_bind() {
        let ast = |rule: Value, transform: Value| {
            let mut ast = json!({ "rule": rule });
            if !transform.is_null() {
                ast["transform"] = transform;
            }
            let compiled =
                compile_typescript_ast(ast).unwrap_or_else(|error| panic!("{}", error.reason));
            let source = "const ABC = 1;\nf(f, abc);\ng(xyz, g);\n";
            let result = evaluate_file(&compiled, "typescript", "a.ts", source);
            assert_eq!(result["failures"], json!([]), "{result}");
            matched_texts(&result, source)
        };
        // `ofRule` counts the arguments other than the callee's name, which `inside` binds: the
        // first such argument, whatever `inside` is listed after.
        let first_other = json!({
            "kind": "identifier",
            "nthChild": { "position": 1, "ofRule": { "pattern": "$X",
                "not": { "compare": { "left": "X", "op": "eq", "right": "F", "text": "source" } } } },
            "inside": { "kind": "call_expression", "stopBy": "end", "pattern": "$F($$$)" },
        });
        assert_eq!(
            ast(first_other, Value::Null),
            ["abc F=f X=abc", "xyz F=g X=xyz"]
        );
        // A `has` narrowed to `arguments` binds `$A`; the `inside` listed before it compares an
        // upper-cased `$A` with a declared name, so it runs after the `has`.
        let transform = json!({ "T": { "convert": { "source": "$A", "toCase": "upperCase" } } });
        let declared = json!({
            "kind": "call_expression",
            "inside": { "kind": "program", "stopBy": "end", "has": { "stopBy": "end", "kind": "identifier",
                "pattern": "$K", "compare": { "left": "K", "op": "eq", "right": "T", "text": "source" } } },
            "has": { "field": "arguments", "has": { "kind": "identifier", "regex": "^[a-z]{3}$", "pattern": "$A" } },
        });
        assert_eq!(
            ast(declared, transform.clone())
                .into_iter()
                .map(|text| text.split(" A=").next().unwrap_or_default().to_owned())
                .collect::<Vec<_>>(),
            ["f(f, abc)"]
        );
        // A cycle through a transform variable is refused like one through its capture.
        let cycle = json!({
            "kind": "call_expression",
            "inside": { "kind": "program", "stopBy": "end", "has": { "stopBy": "end", "pattern": "$A",
                "compare": { "left": "A", "op": "ne", "right": "K", "text": "source" } } },
            "has": { "field": "arguments", "has": { "pattern": "$K",
                "compare": { "left": "K", "op": "ne", "right": "T", "text": "source" } } },
        });
        let reason = rejection(compile_typescript_ast(
            json!({ "rule": cycle, "transform": transform }),
        ));
        assert!(reason.contains("no order binds both first"), "{reason}");
    }

    /// The Ruff suite's PIE810 repro: a negation whose utility's pattern names `$R` and `$M`, beside
    /// a `matches` whose utility binds them, runs after it, so the names read as the bound ones
    /// ("no earlier call with the same receiver and method"), not as fresh ones ("no earlier
    /// call"). The same inside a utility, and a cycle through what two utilities bind is refused.
    #[test]
    fn a_negation_reading_what_a_utility_binds_runs_after_the_matches_term() {
        let python = |ast: Value, source: &str| {
            let compiled = compile_document_with(
                check_document(json!({ "id": "ast-match", "violation": finding(),
                                       "forbid": { "ast": ast } })),
                &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
            );
            let compiled = compiled.unwrap_or_else(|error| panic!("{}", error.reason));
            let result = evaluate_file(&compiled, "python", "a.py", source);
            assert_eq!(result["failures"], json!([]), "{result}");
            matched_texts(&result, source)
                .into_iter()
                .map(|text| text.split(' ').next().unwrap_or_default().to_owned())
                .collect::<Vec<_>>()
        };
        let call = json!({ "has": { "stopBy": "neighbor", "kind": "call", "pattern": "$R.$M()" } });
        let first = json!({
            "kind": "expression_statement",
            "all": [{ "not": { "follows": { "stopBy": "end", "matches": "call" } } }],
            "matches": "call",
        });
        let source = "a.f()\nb.g()\na.f()\nb.h()\n";
        let expected = ["a.f()", "b.g()", "b.h()"];
        assert_eq!(
            python(
                json!({ "language": "python", "rule": first, "utils": { "call": call } }),
                source
            ),
            expected
        );
        // Inside a utility, compiled before the one it calls.
        assert_eq!(
            python(
                json!({ "language": "python", "rule": { "matches": "a-first" },
                        "utils": { "a-first": first, "call": call } }),
                source
            ),
            expected
        );
        let cycle = json!({
            "kind": "expression_statement",
            "matches": "binds-b",
            "has": { "pattern": "$A", "not": { "has": { "stopBy": "end", "pattern": "$B" } } },
        });
        let reason = rejection(compile_document_with(
            check_document(
                json!({ "id": "ast-match", "violation": finding(), "forbid": { "ast": {
                "language": "python", "rule": cycle,
                "utils": { "binds-b": { "has": { "pattern": "$B", "not": { "has": { "stopBy": "end", "pattern": "$A" } } } } },
            } } }),
            ),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        ));
        assert!(reason.contains("no order binds both first"), "{reason}");
        // A utility that binds `$N` (through another) and tests it under its own negation reads
        // nothing outward, so two terms calling it are no cycle (the Ruff suite's PIE794): a
        // statement repeating an earlier statement's callee, without the callee as its argument.
        let call = json!({ "kind": "expression_statement", "has": { "stopBy": "neighbor", "kind": "call", "all": [
            { "matches": "callee" },
            { "not": { "has": { "stopBy": "end", "kind": "identifier", "pattern": "$N",
                                "inside": { "stopBy": "end", "kind": "argument_list" } } } },
        ] } });
        let repeated = json!({ "language": "python",
            "utils": { "call": call, "callee": { "pattern": "$N($$$)" } },
            "rule": { "matches": "call", "follows": { "stopBy": "end", "matches": "call" } } });
        assert_eq!(python(repeated, "f(1)\ng(f)\nf(2)\ng(g)\n"), ["f(2)"]);
    }

    /// A utility's comparison of a transform variable, called under `not` beside the relation
    /// that binds the variable's capture: it runs in its caller's environment, after that relation.
    #[test]
    fn a_utilitys_transform_comparison_reads_its_callers_capture() {
        let compiled = compile_typescript_ast(json!({
            "utils": { "declares": { "kind": "identifier",
                "compare": { "left": "K", "op": "eq", "right": "T", "text": "source" } } },
            "rule": { "kind": "call_expression",
                "any": [{ "has": { "stopBy": "end", "pattern": "$A", "regex": "^never$" } }, { "kind": "call_expression" }],
                "not": {
                    "inside": { "stopBy": "end", "kind": "program", "has": { "stopBy": "end", "pattern": "$K", "matches": "declares" } },
                    "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$A" } },
                } },
            "transform": { "T": { "convert": { "source": "$A", "toCase": "upperCase" } } },
        }))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "const ABC = 1;\nf(abc);\ng(xyz);\n";
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        assert_eq!(matched_texts(&result, source), ["g(xyz)"]);
    }
    #[test]
    fn each_any_branch_reads_utility_dependencies_in_its_own_environment() {
        let compiled = compile_typescript_ast(json!({
            "utils": { "same": {
                "compare": { "left": "A", "op": "eq", "right": "B", "text": "source" }
            } },
            "rule": {
                "kind": "shorthand_property_identifier",
                "pattern": "$B",
                "any": [
                    { "pattern": "$A", "regex": "^never$", "matches": "same" },
                    { "matches": "same" }
                ],
                "follows": {
                    "stopBy": "end",
                    "kind": "shorthand_property_identifier",
                    "pattern": "$A"
                }
            }
        }))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "const o = { x, x };\n";
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        assert_eq!(matched_texts(&result, source), ["x A=x B=x"]);
    }

    /// What a call binds and reads is its utility's, mapped at the call: each utility of a diamond
    /// chain is expanded a constant number of times per walk, not once per call path, and so is
    /// what `reads` and `mentions` need of what the terms below them bind.
    #[test]
    fn parameterized_utility_diamonds_order_in_linear_visits() {
        let analysis = crate::policy_dsl::matcher::CaptureAnalysis::default;
        let compiled = compiled_parameterized_utility_diamonds(DIAMOND_DEPTH);
        let matcher = &grammar_matchers(&compiled)[0].matcher;
        let bound = linear_walk(DIAMOND_DEPTH);
        reset_walks();
        let bound_names = analysis().binds(matcher);
        assert!(bound_names.contains("X"), "{bound_names:?}");
        assert!(
            walks(Walk::OrderBinds) <= bound,
            "binds expanded {} utilities (bound {bound})",
            walks(Walk::OrderBinds)
        );
        let expansions = || {
            [Walk::OrderBinds, Walk::OrderReads, Walk::OrderMentions]
                .map(walks)
                .into_iter()
                .sum::<usize>()
        };
        reset_walks();
        analysis().reads(matcher);
        assert!(
            expansions() <= 2 * bound,
            "reads expanded {} utilities (bound {})",
            expansions(),
            2 * bound
        );
        reset_walks();
        analysis().mentions(matcher);
        assert!(
            expansions() <= bound,
            "mentions expanded {} utilities (bound {bound})",
            expansions()
        );
    }

    /// A chain of utilities, each reached along one path, costs each of them a constant number of
    /// expansions: to compile (every rule object orders its terms by what they bind and read, for
    /// each grammar the query compiles for), and to read what the compiled rule binds, reads and
    /// mentions.
    #[test]
    fn utility_chains_order_in_linear_visits() {
        let analysis = crate::policy_dsl::matcher::CaptureAnalysis::default;
        let depth = 64;
        let bound = linear_walk(depth);
        reset_walks();
        let compiled = compiled(
            json!({ "rule": { "matches": format!("u{depth}") }, "utils": utility_chain(depth) }),
        );
        let compile_bound = grammar_matchers(&compiled).len() * bound;
        for walk in [Walk::OrderBinds, Walk::OrderReads] {
            assert!(
                walks(walk) <= compile_bound,
                "compiling expanded {} utilities for {walk:?} (bound {compile_bound})",
                walks(walk)
            );
        }
        let matcher = &grammar_matchers(&compiled)[0].matcher;
        let expansions = || {
            [Walk::OrderBinds, Walk::OrderReads, Walk::OrderMentions]
                .map(walks)
                .into_iter()
                .sum::<usize>()
        };
        reset_walks();
        assert!(analysis().binds(matcher).contains("A"));
        assert!(expansions() <= bound, "binds expanded {}", expansions());
        reset_walks();
        analysis().reads(matcher);
        assert!(expansions() <= 2 * bound, "reads expanded {}", expansions());
        reset_walks();
        assert!(analysis().mentions(matcher).contains("A"));
        assert!(expansions() <= bound, "mentions expanded {}", expansions());
    }

    /// A term reading what a call binds through a cycle of parameterized calls runs after that
    /// call, whichever `any` branch the walk meets first: the negated comparison then tests the
    /// bound `$X` and spares the statements whose previous statement has the same text.
    #[test]
    fn terms_reading_what_recursive_calls_bind_run_after_them_whatever_the_branch_order() {
        let v = json!({ "kind": "identifier", "matches": { "V": { "R": "$A" } } });
        let w = json!({ "matches": { "W": { "P": "$X" } } });
        let source = "a();\na();\nb();\n";
        for branches in [json!([v.clone(), w.clone()]), json!([w, v])] {
            let mut utils = mutually_recursive_calls();
            utils.insert("bindX".to_owned(), json!({ "pattern": "$X" }));
            let compiled = compiled(json!({
                "rule": {
                    "kind": "expression_statement",
                    "pattern": "$C",
                    "all": [{ "not": { "compare": { "left": "X", "op": "eq", "right": "C" } } }],
                    "any": branches,
                },
                "utils": utils,
            }));
            assert_eq!(
                matched_texts(
                    &evaluate_file(&compiled, "typescript", "a.ts", source),
                    source
                ),
                ["b(); C=b(); X=a();"],
                "{branches}"
            );
        }
    }

    /// The same for plain recursive utilities: `W` reads what `V` reads (`$Y`), so the `any`
    /// calling either runs after the `has` binding `$Y`, whichever branch comes first.
    #[test]
    fn terms_reading_through_recursive_utilities_run_after_the_binder_whatever_the_branch_order() {
        let utils = json!({
            "V": { "pattern": "$D", "all": [
                { "not": { "compare": { "left": "Y", "op": "eq", "right": "D" } } },
                { "any": [ { "kind": "call_expression" }, { "has": { "matches": "W" } } ] }
            ] },
            "W": { "has": { "matches": "V" } },
        });
        let v = json!({ "kind": "identifier", "pattern": "$Y", "matches": "V" });
        let w = json!({ "matches": "W" });
        let source = "a();\n";
        for branches in [json!([v.clone(), w.clone()]), json!([w, v])] {
            let compiled = compiled(json!({
                "rule": {
                    "kind": "expression_statement",
                    "any": branches,
                    "has": { "kind": "call_expression", "pattern": "$Y" },
                },
                "utils": utils.clone(),
            }));
            assert_eq!(
                matched_texts(
                    &evaluate_file(&compiled, "typescript", "a.ts", source),
                    source
                ),
                Vec::<String>::new(),
                "{branches}"
            );
        }
    }
}
