use super::AstMatcher;
#[cfg(test)]
use super::util::{THREE_VALUED, UNKNOWN};
#[cfg(test)]
use crate::policy_dsl::budget;
#[cfg(test)]
use ast_grep_core::matcher::{Matcher, MatcherExt};
#[cfg(test)]
use ast_grep_core::meta_var::MetaVarEnv;
#[cfg(test)]
use ast_grep_core::{Doc, Node};
#[cfg(test)]
use std::borrow::Cow;
#[cfg(test)]
use std::cell::Cell;
#[cfg(test)]
use std::ops::ControlFlow;

/// How far a relation walks, as ast-grep-config 0.45.3's `StopBy` (`rule/stop_by.rs`).
#[derive(Clone)]
pub(crate) enum StopBy {
    Neighbor,
    End,
    /// Walk until the first node this rule matches, inclusive: that node is still tested against the
    /// relation's rule, the walk stops after it. The stop rule matches in a fresh environment
    /// (`Node::matches`), so it neither reads nor binds the caller's metavariables.
    Rule(Box<AstMatcher>),
}

/// Whether a stop rule matches `node`; `None` when its verdict is unknown (a value it reads is).
/// `three` is `THREE_VALUED`, read once by the caller.
#[cfg(test)]
pub(super) fn stops_at<D: Doc>(stop: &AstMatcher, node: &Node<'_, D>, three: bool) -> Option<bool> {
    super::each::suspended(|| stops_at_inner(stop, node, three))
}

#[cfg(test)]
fn stops_at_inner<D: Doc>(stop: &AstMatcher, node: &Node<'_, D>, three: bool) -> Option<bool> {
    if !three {
        return Some(stop.match_node(node.clone()).is_some());
    }
    let saved = UNKNOWN.replace(false);
    let stopped = stop.match_node(node.clone()).is_some();
    let unknown = UNKNOWN.replace(saved);
    match (stopped, unknown) {
        (true, _) => Some(true),
        (false, false) => Some(false),
        (false, true) => None,
    }
}

/// Whether a rule can bind a capture: a pattern with a named metavariable, through the composites,
/// relations and utilities that share the caller's environment. `not`, `count` and stop rules bind
/// nothing the caller keeps.
pub(super) fn may_bind(matcher: &AstMatcher) -> bool {
    !super::captures::binds(matcher).is_empty()
}

/// A relation's search for its first matching node, three-valued. A candidate whose verdict is
/// unknown leaves the answer open unless a later candidate matches; when the relation's rule binds
/// captures, which node binds them first is then unknown too, so the search ends there (strict).
/// An unknown stop verdict is read as a stop that leaves the answer open.
#[cfg(test)]
struct Search {
    three: bool,
    /// Whether `finding.each` is excluding bindings (read once per search).
    each: bool,
    saved: bool,
    unknown: bool,
    strict: Option<bool>,
}

/// A candidate's test while `finding.each` excludes bindings: one that binds the enumerated
/// capture to a reported range is passed over, bindings and all. `None` where it does not match.
/// Kept out of `Search::test` so that the recursion's frames stay small when nothing is excluded.
#[inline(never)]
#[cfg(test)]
fn test_excluding<'t, D: Doc>(
    matcher: &AstMatcher,
    candidate: Node<'t, D>,
    env: &mut Cow<MetaVarEnv<'t, D>>,
) -> Option<ControlFlow<Option<Node<'t, D>>>> {
    let mut trial = Cow::Borrowed(env.as_ref());
    let found = matcher.match_node_with_env(candidate, &mut trial)?;
    if super::each::excluded(&trial) {
        return Some(ControlFlow::Continue(()));
    }
    if let Cow::Owned(bound) = trial {
        *env = Cow::Owned(bound);
    }
    Some(ControlFlow::Break(Some(found)))
}

#[cfg(test)]
impl Search {
    fn start() -> Self {
        let three = THREE_VALUED.get();
        Self {
            three,
            each: super::each::active(),
            saved: three && UNKNOWN.replace(false),
            unknown: false,
            strict: None,
        }
    }

    /// Tests one candidate: breaks with the match, or with `None` where a strict search must end.
    fn test<'t, D: Doc>(
        &mut self,
        matcher: &AstMatcher,
        candidate: Node<'t, D>,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> ControlFlow<Option<Node<'t, D>>> {
        if self.each {
            if let Some(flow) = test_excluding(matcher, candidate, env) {
                return flow;
            }
        } else if let Some(found) = matcher.match_node_with_env(candidate, env) {
            return ControlFlow::Break(Some(found));
        }
        if self.three && UNKNOWN.replace(false) {
            return self.open(matcher);
        }
        ControlFlow::Continue(())
    }

    /// Records an unknown verdict; breaks where the search must end (strict).
    fn open<'t, D: Doc>(&mut self, matcher: &AstMatcher) -> ControlFlow<Option<Node<'t, D>>> {
        self.unknown = true;
        if *self.strict.get_or_insert_with(|| may_bind(matcher)) {
            ControlFlow::Break(None)
        } else {
            ControlFlow::Continue(())
        }
    }

    fn finish<'t, D: Doc>(self, found: Option<Node<'t, D>>) -> Option<Node<'t, D>> {
        if !self.three {
            return found;
        }
        match found {
            Some(found) => {
                UNKNOWN.set(self.saved);
                Some(found)
            }
            None => {
                UNKNOWN.set(self.saved || self.unknown);
                None
            }
        }
    }
}

#[cfg(test)]
impl StopBy {
    /// Visits the nodes `find` would test, in order, until `visit` breaks. An unknown stop verdict
    /// ends the walk and is reported through `open`.
    pub(super) fn walk<'t, D: Doc, I: Iterator<Item = Node<'t, D>>>(
        &self,
        once: impl FnOnce() -> Option<Node<'t, D>>,
        all: impl FnOnce() -> I,
        mut visit: impl FnMut(Node<'t, D>) -> ControlFlow<()>,
        open: &Cell<bool>,
        three: bool,
    ) -> ControlFlow<()> {
        match self {
            Self::Neighbor => once().map_or(ControlFlow::Continue(()), visit),
            Self::End => all().try_for_each(visit),
            Self::Rule(stop) => {
                for node in all() {
                    let stopped = stops_at(stop, &node, three);
                    visit(node)?;
                    match stopped {
                        Some(false) => {}
                        Some(true) => break,
                        None => {
                            open.set(true);
                            break;
                        }
                    }
                }
                ControlFlow::Continue(())
            }
        }
    }

    /// ast-grep-config's `StopBy::find` for `inside`, `precedes` and `follows`: `once` is the
    /// neighbour, `all` the walk along the axis; with a rule, `all` is cut after the first stop node.
    pub(super) fn find<'t, D: Doc, I: Iterator<Item = Node<'t, D>>>(
        &self,
        once: impl FnOnce() -> Option<Node<'t, D>>,
        all: impl FnOnce() -> I,
        matcher: &AstMatcher,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> Option<Node<'t, D>> {
        let mut search = Search::start();
        let found = match self {
            Self::Neighbor => once()
                .and_then(|candidate| search.test(matcher, candidate, env).break_value().flatten()),
            Self::End => all()
                .try_for_each(|candidate| search.test(matcher, candidate, env))
                .break_value()
                .flatten(),
            Self::Rule(stop) => {
                let mut found = None;
                for candidate in all() {
                    let stopped = stops_at(stop, &candidate, search.three);
                    if let ControlFlow::Break(result) = search.test(matcher, candidate, env) {
                        found = result;
                        break;
                    }
                    match stopped {
                        Some(false) => {}
                        Some(true) => break,
                        None => {
                            let _ = search.open::<D>(matcher);
                            break;
                        }
                    }
                }
                found
            }
        };
        search.finish(found)
    }
}

/// Visits descendants in pre-order; a descendant `stop` matches is visited but not entered. One
/// whose stop verdict is unknown is not entered either, and sets `open`.
#[cfg(test)]
pub(super) fn descend_until<'tree, D: Doc>(
    node: &Node<'tree, D>,
    stop: &AstMatcher,
    mut visit: impl FnMut(Node<'tree, D>) -> ControlFlow<()>,
    open: &Cell<bool>,
    three: bool,
) -> ControlFlow<()> {
    let level = |node: &Node<'tree, D>| node.children().collect::<Vec<_>>().into_iter();
    let mut stack = vec![level(node)];
    while let Some(children) = stack.last_mut() {
        let Some(child) = children.next() else {
            stack.pop();
            continue;
        };
        visit(child.clone())?;
        match stops_at(stop, &child, three) {
            Some(false) => stack.push(level(&child)),
            Some(true) => {}
            None => open.set(true),
        }
    }
    ControlFlow::Continue(())
}

/// `has`: the first child (`neighbor`) or descendant in pre-order that matches; with a stop rule, a
/// descendant the stop rule matches is tested but its subtree is not entered.
#[cfg(test)]
pub(super) fn has<'tree, D: Doc>(
    node: &Node<'tree, D>,
    matcher: &AstMatcher,
    stop: &StopBy,
    env: &mut Cow<MetaVarEnv<'tree, D>>,
) -> Option<Node<'tree, D>> {
    let mut search = Search::start();
    let found = match stop {
        StopBy::Neighbor => node
            .children()
            .try_for_each(|child| search.test(matcher, child, env))
            .break_value()
            .flatten(),
        StopBy::End => node
            .dfs()
            .skip(1)
            .try_for_each(|child| search.test(matcher, child, env))
            .break_value()
            .flatten(),
        StopBy::Rule(stop) => {
            let mut found = None;
            let open = Cell::new(false);
            let three = search.three;
            let _ = descend_until(
                node,
                stop,
                |child| {
                    if budget::matcher_call_budget_exhausted() {
                        return ControlFlow::Break(());
                    }
                    match search.test(matcher, child, env) {
                        ControlFlow::Continue(()) => {}
                        ControlFlow::Break(result) => {
                            found = result;
                            return ControlFlow::Break(());
                        }
                    }
                    // A subtree left unentered on an unknown stop verdict leaves the answer open.
                    if open.take() && search.open::<D>(matcher).is_break() {
                        return ControlFlow::Break(());
                    }
                    ControlFlow::Continue(())
                },
                &open,
                three,
            );
            if open.get() {
                let _ = search.open::<D>(matcher);
            }
            found
        }
    };
    search.finish(found)
}

/// `inside` or `has` with a `field` (ast-grep-config 0.45.3 `Inside`/`Has`, `relational_rule.rs`).
#[derive(Clone)]
pub(crate) struct FieldRelation {
    /// `inside` when true, `has` otherwise.
    pub(crate) inside: bool,
    /// The grammar's field id.
    pub(crate) field: u16,
    pub(crate) rule: AstMatcher,
    pub(crate) stop: StopBy,
}

#[cfg(test)]
impl FieldRelation {
    pub(super) fn find<'t, D: Doc>(
        &self,
        node: &Node<'t, D>,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> Option<Node<'t, D>> {
        if self.inside {
            self.find_inside(node, env)
        } else {
            self.find_has(node, env)
        }
    }

    /// The ancestors walked as `stopBy` walks them; each is tested only where the node the walk
    /// came from is its child in the field, else skipped (the walk goes on).
    fn find_inside<'t, D: Doc>(
        &self,
        node: &Node<'t, D>,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> Option<Node<'t, D>> {
        let mut search = Search::start();
        let mut last = node.node_id();
        let mut entered = |candidate: &Node<'t, D>| {
            let expected = std::mem::replace(&mut last, candidate.node_id());
            candidate
                .child_by_field_id(self.field)
                .is_some_and(|child| child.node_id() == expected)
        };
        let found = match &self.stop {
            StopBy::Neighbor => node.parent().and_then(|candidate| {
                if entered(&candidate) {
                    search
                        .test(&self.rule, candidate, env)
                        .break_value()
                        .flatten()
                } else {
                    None
                }
            }),
            StopBy::End => node
                .ancestors()
                .try_for_each(|candidate| {
                    if entered(&candidate) {
                        search.test(&self.rule, candidate, env)
                    } else {
                        ControlFlow::Continue(())
                    }
                })
                .break_value()
                .flatten(),
            StopBy::Rule(stop) => {
                let mut found = None;
                for candidate in node.ancestors() {
                    let stopped = stops_at(stop, &candidate, search.three);
                    if entered(&candidate)
                        && let ControlFlow::Break(result) = search.test(&self.rule, candidate, env)
                    {
                        found = result;
                        break;
                    }
                    match stopped {
                        Some(false) => {}
                        Some(true) => break,
                        None => {
                            let _ = search.open::<D>(&self.rule);
                            break;
                        }
                    }
                }
                found
            }
        };
        search.finish(found)
    }

    /// The field child, then as `stopBy` says: nothing more (`neighbor`), its descendants in
    /// pre-order (`end`), or, where it neither matches nor stops, its children only (a stop rule;
    /// upstream tests one level below the field child and does not recurse).
    fn find_has<'t, D: Doc>(
        &self,
        node: &Node<'t, D>,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> Option<Node<'t, D>> {
        let child = node.child_by_field_id(self.field)?;
        let mut search = Search::start();
        let found = match &self.stop {
            StopBy::Neighbor => search.test(&self.rule, child, env).break_value().flatten(),
            StopBy::End => child
                .dfs()
                .try_for_each(|candidate| search.test(&self.rule, candidate, env))
                .break_value()
                .flatten(),
            StopBy::Rule(stop) => match search.test(&self.rule, child.clone(), env) {
                ControlFlow::Break(result) => result,
                ControlFlow::Continue(()) => match stops_at(stop, &child, search.three) {
                    Some(true) => None,
                    None => {
                        let _ = search.open::<D>(&self.rule);
                        None
                    }
                    Some(false) => child
                        .children()
                        .try_for_each(|candidate| search.test(&self.rule, candidate, env))
                        .break_value()
                        .flatten(),
                },
            },
        };
        search.finish(found)
    }
}

pub(crate) fn stop_rule(stop: &StopBy) -> Option<&AstMatcher> {
    match stop {
        StopBy::Rule(rule) => Some(rule),
        StopBy::Neighbor | StopBy::End => None,
    }
}

#[cfg(test)]
mod tests {
    use super::may_bind;
    use crate::policy_dsl::PolicyDslRuntime;
    use crate::policy_dsl::test_support::*;
    use serde_json::{Value, json};

    fn stop_rule_texts(rule: Value, source: &str) -> Vec<String> {
        let compiled =
            compile_typescript_rule(rule).unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        matched_texts(&result, source)
    }

    #[test]
    fn stop_rules_end_the_walk_after_the_first_node_they_match() {
        // `inside`: the stop node itself is still tested (ast-grep-config `inclusive_until`).
        let source = "class A { m() { f(); } }\nclass B { m() { return () => { g(); }; } }\n";
        let inside = |target: &str| {
            json!({
                "pattern": "$F()",
                "inside": { "kind": target, "stopBy": { "kind": "method_definition" } },
            })
        };
        assert_eq!(
            stop_rule_texts(inside("class_body"), source),
            Vec::<String>::new()
        );
        assert_eq!(
            stop_rule_texts(inside("method_definition"), source),
            ["f() F=f", "g() F=g"]
        );
        assert_eq!(
            stop_rule_texts(inside("arrow_function"), source),
            ["g() F=g"]
        );
        // `has`: a descendant the stop rule matches is tested but not entered.
        let source =
            "function f() { a(); function g() { b(); } }\nfunction h() { const k = () => c(); }\n";
        let has = |target: Value| {
            json!({
                "kind": "function_declaration",
                "has": { "all": [target], "stopBy": { "any": [
                    { "kind": "function_declaration" },
                    { "kind": "arrow_function" },
                ] } },
            })
        };
        assert_eq!(
            stop_rule_texts(has(json!({ "pattern": "b()" })), source),
            ["function g() { b(); }"]
        );
        assert_eq!(
            stop_rule_texts(has(json!({ "pattern": "c()" })), source),
            Vec::<String>::new()
        );
        assert_eq!(
            stop_rule_texts(has(json!({ "kind": "arrow_function" })), source),
            ["function h() { const k = () => c(); }"]
        );
        // `precedes` / `follows`: the stop sibling is tested, the siblings past it are not.
        let source = "a();\nbreak_here;\nb();\n";
        let sibling = |relation: &str, target: &str| {
            json!({
                "kind": "expression_statement",
                "pattern": target,
                relation: { "pattern": "$X();", "stopBy": { "pattern": "break_here;" } },
            })
        };
        assert_eq!(
            stop_rule_texts(sibling("precedes", "a();"), source),
            Vec::<String>::new()
        );
        assert_eq!(
            stop_rule_texts(sibling("follows", "b();"), source),
            Vec::<String>::new()
        );
        let reach_stop = json!({
            "pattern": "a();",
            "precedes": { "pattern": "break_here;", "stopBy": { "pattern": "break_here;" } },
        });
        assert_eq!(stop_rule_texts(reach_stop, source), ["a();"]);
    }

    #[test]
    fn stop_rules_neither_read_nor_bind_the_callers_metavariables() {
        // `$F` is bound to `f` when the stop rule runs; upstream matches the stop rule in a fresh
        // environment, so `$F()` stops at the first call of any name.
        let source = "f(g(h(f())));\n";
        let rule = json!({
            "pattern": "$F($$$)",
            "has": { "pattern": "$F()", "stopBy": { "pattern": "$F($$$)" } },
        });
        assert_eq!(stop_rule_texts(rule, source), Vec::<String>::new());
        // Names a stop rule would bind are not captures: a message or constraint using one is an error.
        let document = json!({
            "format": "rerail.policy/v1",
            "id": "stop-captures",
            "revision": 1,
            "checks": [{
                "id": "ast-match",
                "forbid": { "ast": { "language": "typescript", "rule": {
                    "pattern": "$F()",
                    "inside": { "kind": "function_declaration", "stopBy": { "pattern": "function $NAME() {}" } },
                } } },
                "finding": { "anchor": "witness.span", "code": "AST_MATCH", "message": "in $NAME", "severity": "error" },
            }],
        });
        let reason = rejection(PolicyDslRuntime::compile(
            document.to_string(),
            "json".to_owned(),
            CAPABILITIES.to_owned(),
        ));
        assert!(reason.contains("$NAME"), "{reason}");
        let reason = rejection(compile_typescript_ast(json!({
            "rule": { "pattern": "$F()", "inside": { "kind": "program", "stopBy": { "pattern": "$G($$$)" } } },
            "constraints": { "G": { "kind": "identifier" } },
        })));
        assert!(reason.contains("`$G` is not defined"), "{reason}");
    }

    #[test]
    fn stop_rules_compile_per_dialect_and_may_use_utils() {
        // Under the plain dialect the JSX stop rule never matches, so the walk runs to the end.
        let rule = json!({
            "kind": "identifier",
            "regex": "^go$",
            "inside": { "kind": "arrow_function", "stopBy": { "kind": "jsx_expression" } },
        });
        let jsx = compile_typescript_rule(rule).expect("whole in the tsx dialect");
        let source = "export const V = () => <b onClick={go} />;\n";
        assert_findings(&evaluate_file(&jsx, "tsx", "a.tsx", source), 0);
        assert_findings(
            &evaluate_file(&jsx, "typescript", "a.ts", "const V = () => go;\n"),
            1,
        );
        let rule = compiled(json!({
            "rule": { "pattern": "$F()", "inside": { "kind": "class_body", "stopBy": { "matches": "function" } } },
            "utils": { "function": { "any": [{ "kind": "function_declaration" }, { "kind": "method_definition" }] } },
        }));
        let source = "class A { x = f(); m() { g(); } }\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(matched_texts(&result, source), ["f() F=f"], "{result}");
    }

    #[test]
    fn inside_with_a_field_requires_the_path_to_enter_through_it() {
        // ast-grep-config 0.45.3 `Inside`: each ancestor tested must hold the previous node of the
        // walk in the field; an ancestor reached through another child is skipped, not stopped at.
        let source = "const a = b;\nif (a && x) { x; }\n";
        assert_eq!(
            stop_rule_texts(
                json!({ "kind": "identifier", "inside": { "kind": "variable_declarator", "field": "name" } }),
                source,
            ),
            ["a"]
        );
        let condition = json!({
            "kind": "identifier",
            "regex": "^x$",
            "inside": { "kind": "if_statement", "field": "condition", "stopBy": "end" },
        });
        assert_eq!(stop_rule_texts(condition, source), ["x"]);
        // With a stop rule the walk ends at the first stop node, entered through the field or not.
        let stopped = json!({
            "kind": "identifier",
            "regex": "^x$",
            "inside": { "kind": "if_statement", "field": "condition", "stopBy": { "kind": "binary_expression" } },
        });
        assert_eq!(stop_rule_texts(stopped, source), Vec::<String>::new());
        let reached = json!({
            "kind": "identifier",
            "regex": "^x$",
            "inside": { "kind": "if_statement", "field": "condition", "stopBy": { "kind": "if_statement" } },
        });
        assert_eq!(stop_rule_texts(reached, source), ["x"]);
    }

    #[test]
    fn has_with_a_field_starts_at_the_field_child() {
        let source = "function f() {}\nfunction g() { a(); }\nfunction h() { { b(); } }\n";
        let empty = json!({
            "kind": "function_declaration",
            "has": { "field": "body", "regex": "^\\{\\s*\\}$" },
        });
        assert_eq!(stop_rule_texts(empty, source), ["function f() {}"]);
        // `end` walks the field child and its descendants.
        let end = json!({
            "kind": "function_declaration",
            "has": { "field": "body", "stopBy": "end", "kind": "call_expression" },
        });
        assert_eq!(
            stop_rule_texts(end, source),
            ["function g() { a(); }", "function h() { { b(); } }"]
        );
        // The field child itself is a candidate under `end`, as upstream's `dfs` includes it.
        let own = json!({
            "kind": "call_expression",
            "has": { "field": "function", "stopBy": "end", "kind": "identifier" },
        });
        assert_eq!(stop_rule_texts(own, "a(b);\n"), ["a(b)"]);
        // With a stop rule upstream tests the field child, then only its children: one level.
        let quirk = json!({
            "kind": "function_declaration",
            "has": { "field": "body", "stopBy": { "kind": "return_statement" }, "kind": "expression_statement" },
        });
        assert_eq!(stop_rule_texts(quirk, source), ["function g() { a(); }"]);
        // A field child the stop rule matches is tested but not entered.
        let stop_at_body = json!({
            "kind": "function_declaration",
            "has": { "field": "body", "stopBy": { "kind": "statement_block" }, "kind": "expression_statement" },
        });
        assert_eq!(stop_rule_texts(stop_at_body, source), Vec::<String>::new());
        // A node without the field has nothing to test.
        let missing = json!({ "kind": "arrow_function", "has": { "field": "parameters", "kind": "formal_parameters" } });
        assert_eq!(
            stop_rule_texts(missing, "const k = x => x;\nconst l = (y) => y;\n"),
            ["(y) => y"]
        );
    }

    #[test]
    fn fields_are_checked_per_grammar_and_refused_elsewhere() {
        for (rule, expected) in [
            (
                json!({ "kind": "identifier", "precedes": { "kind": "identifier", "field": "name" } }),
                "/checks/0/forbid/ast/rule/precedes/field: field is only supported in has/inside",
            ),
            (
                json!({ "kind": "identifier", "follows": { "kind": "identifier", "field": "name" } }),
                "/checks/0/forbid/ast/rule/follows/field: field is only supported in has/inside",
            ),
            (
                json!({ "kind": "identifier", "count": { "has": { "kind": "identifier", "field": "name" }, "min": 1 } }),
                "/checks/0/forbid/ast/rule/count/has/field: field is not supported in count",
            ),
            (
                json!({ "kind": "identifier", "inside": { "kind": "identifier", "field": "" } }),
                "/checks/0/forbid/ast/rule/inside/field must be a non-empty string",
            ),
            (
                json!({ "kind": "identifier", "inside": { "kind": "identifier", "field": 3 } }),
                "/checks/0/forbid/ast/rule/inside/field must be a non-empty string",
            ),
            (
                json!({ "kind": "identifier", "field": "name" }),
                "unknown field `field` at /checks/0/forbid/ast/rule",
            ),
            (
                json!({ "kind": "identifier", "inside": { "kind": "call_expression", "field": "nme" } }),
                "/checks/0/forbid/ast/rule/inside/field: `nme` is not a field of this grammar (typescript, tsx, javascript grammars)",
            ),
        ] {
            assert_eq!(
                rejection(compile_typescript_rule(rule.clone())),
                expected,
                "{rule}"
            );
        }
        // Python's fields are Python's: a `yield` in a default is not in the function's body.
        let python = |field: &str| {
            compile_document_with(
                check_document(json!({
                    "id": "c",
                    "forbid": { "ast": { "language": "python", "rule": {
                        "kind": "yield",
                        "not": { "inside": { "kind": "function_definition", "field": field, "stopBy": "end" } },
                    } } },
                    "finding": finding(),
                })),
                &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
            )
        };
        let compiled = python("body").unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "def f(x=(yield)):\n    yield 1\nyield 2\n";
        let result = evaluate_file(&compiled, "python", "a.py", source);
        assert_eq!(matched_texts(&result, source), ["yield", "yield 2"]);
        assert!(rejection(python("declarator")).contains("`declarator` is not a field"));
    }
    #[test]
    fn strict_relation_search_recognizes_argument_bindings() {
        let compiled = compile_typescript_rule(json!({
            "kind": "program",
            "has": {
                "stopBy": "end",
                "kind": "call_expression",
                "arguments": { "signature": ["value"], "bind": { "ARG": "value" } }
            }
        }))
        .expect("rule compiles");
        let matcher = &grammar_matchers(&compiled)[0].matcher;
        assert!(may_bind(matcher));
    }
}
