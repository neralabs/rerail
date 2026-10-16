//! `nthChild` (ast-grep-config 0.45.3 `rule/nth_child.rs`): the node's 1-based position among its
//! parent's named children, or among those `ofRule` matches, counted from the end with `reverse`.

use super::AstMatcher;
use super::CaptureAnalysis;
use super::spend_util_step;
use super::util::{THREE_VALUED, UNKNOWN};
use ast_grep_core::Matcher;
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;

/// CSS's `An+B`: a position `index` (1-based) matches where `index = step * n + offset` for some
/// `n >= 0`; a zero step matches `offset` alone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Position {
    pub(crate) step: i32,
    pub(crate) offset: i32,
}

impl Position {
    /// Upstream's `FunctionalPosition::is_matched`, integer division and remainder included.
    fn holds(self, index: usize) -> bool {
        let Some(index) = i64::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
        else {
            return false;
        };
        let step = i64::from(self.step);
        let offset = i64::from(self.offset);
        if step == 0 {
            index == offset
        } else {
            let Some(n) = index.checked_sub(offset) else {
                return false;
            };
            n / step >= 0 && n % step == 0
        }
    }
}

#[derive(Clone)]
pub(crate) struct NthChild {
    pub(crate) position: Position,
    pub(crate) of_rule: Option<AstMatcher>,
    pub(crate) reverse: bool,
    /// Whether `of_rule` reads and binds no capture (`CaptureAnalysis::mentions`), worked out on
    /// first use: its verdict at a sibling is then the same for every candidate.
    pub(crate) capture_free: std::sync::OnceLock<bool>,
}

impl NthChild {
    /// Whether `index` among `count` counted siblings holds, before or after reversal.
    pub(crate) fn index_holds(&self, index: usize, count: usize) -> bool {
        let index = if self.reverse {
            count - 1 - index
        } else {
            index
        };
        self.position.holds(index)
    }

    pub(super) fn match_node<'t, D: Doc>(
        &self,
        node: Node<'t, D>,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> Option<Node<'t, D>> {
        let index = self.index(&node, env)?;
        self.position.holds(index).then_some(node)
    }

    /// The node's 0-based index among the counted siblings. Each sibling is probed against a copy
    /// of the bindings, so one sibling's bindings never reject the next; the node's own bindings
    /// under `ofRule` are then committed. A sibling whose verdict is unknown leaves the index unknown.
    /// Where `ofRule` reads no capture, a parent's siblings are probed once per file (`memo`).
    fn index<'t, D: Doc>(
        &self,
        node: &Node<'t, D>,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> Option<usize> {
        let parent = node.parent()?;
        let named = || {
            parent.children().filter(|child| {
                let named = child.is_named();
                #[cfg(test)]
                if named {
                    NAMED_SIBLING_VISITS.set(NAMED_SIBLING_VISITS.get() + 1);
                }
                named
            })
        };
        let siblings = match &self.of_rule {
            None => super::memo::siblings(0, &parent, || {
                Some(Siblings {
                    counted: named().map(|child| child.node_id()).collect(),
                    unknown: false,
                })
            })?,
            Some(rule) => {
                let three = THREE_VALUED.get();
                let saved = three && UNKNOWN.replace(false);
                let free = *self
                    .capture_free
                    .get_or_init(|| CaptureAnalysis::default().mentions(rule).is_empty());
                let list = || {
                    let mut siblings = Siblings {
                        counted: Vec::new(),
                        unknown: false,
                    };
                    for child in named() {
                        if !spend_util_step() {
                            return None;
                        }
                        let mut probe = Cow::Borrowed(env.as_ref());
                        let id = child.node_id();
                        let matched = super::each::suspended(|| {
                            rule.match_node_with_env(child, &mut probe).is_some()
                        });
                        if matched {
                            siblings.counted.push(id);
                        } else if three && UNKNOWN.replace(false) {
                            siblings.unknown = true;
                            break;
                        }
                    }
                    Some(siblings)
                };
                let listed = if free {
                    super::memo::siblings(std::ptr::from_ref(self) as usize, &parent, list)
                } else {
                    list().map(std::sync::Arc::new)
                };
                // The budget ran out: no verdict.
                let Some(listed) = listed else {
                    if three {
                        UNKNOWN.set(saved || UNKNOWN.get());
                    }
                    return None;
                };
                if listed.unknown {
                    UNKNOWN.set(true);
                    return None;
                }
                if three {
                    UNKNOWN.set(saved);
                }
                listed
            }
        };
        let position = siblings.counted.binary_search(&node.node_id()).ok()?;
        let index = if self.reverse {
            siblings.counted.len() - 1 - position
        } else {
            position
        };
        if let Some(rule) = &self.of_rule {
            rule.match_node_with_env(node.clone(), env);
        }
        Some(index)
    }
}

/// A parent's named children counted by `nthChild` (all, or those its `ofRule` matches), by node
/// id in order; `unknown` where a probe's verdict was.
pub(super) struct Siblings {
    pub(super) counted: Vec<usize>,
    pub(super) unknown: bool,
}

/// Upstream's `parse_an_b`: white space is ignored; `n`/`N` with an optional signed step, then an
/// optional signed offset; or a bare signed number.
pub(crate) fn parse_position(input: &str) -> Result<Position, String> {
    enum State {
        Initial,
        N,
        Sign(bool),
        Num(bool),
    }
    let syntax = || "invalid syntax".to_owned();
    let (mut step, mut sign, mut num) = (0i32, 1i32, 0i32);
    let mut state = State::Initial;
    let digit = |c: char| c as i32 - '0' as i32;
    for c in input.chars().filter(|c| !c.is_whitespace()) {
        state = match (state, c) {
            (State::Initial, '+' | '-') => {
                sign = if c == '+' { 1 } else { -1 };
                State::Sign(false)
            }
            (State::Initial, '0'..='9') => {
                num = digit(c);
                State::Num(false)
            }
            (State::Initial, 'n' | 'N') => {
                step = sign;
                State::N
            }
            (State::Sign(_), '+' | '-') => return Err(syntax()),
            (State::Sign(has_n), '0'..='9') => {
                num = digit(c);
                State::Num(has_n)
            }
            (State::Sign(true), 'n' | 'N') => return Err(syntax()),
            (State::Sign(false), 'n' | 'N') => {
                step = sign;
                State::N
            }
            (State::Num(_), '+' | '-') => return Err(syntax()),
            (State::Num(has_n), '0'..='9') => {
                num = num
                    .checked_mul(10)
                    .and_then(|num| num.checked_add(digit(c)))
                    .ok_or_else(syntax)?;
                State::Num(has_n)
            }
            (State::Num(true), 'n' | 'N') => return Err(syntax()),
            (State::Num(false), 'n' | 'N') => {
                step = sign * num;
                num = 0;
                State::N
            }
            (State::N, '+' | '-') => {
                sign = if c == '+' { 1 } else { -1 };
                num = 0;
                State::Sign(true)
            }
            (State::N, '0'..='9' | 'n' | 'N') => return Err(syntax()),
            (_, other) => return Err(format!("illegal character `{other}`")),
        };
    }
    match state {
        State::Initial | State::Sign(_) => Err(syntax()),
        State::N | State::Num(_) => Ok(Position {
            step,
            offset: num * sign,
        }),
    }
}

#[cfg(test)]
thread_local! {
    static NAMED_SIBLING_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Named children `nthChild` listed on this thread (`reset_named_sibling_visits` restarts it).
#[cfg(test)]
pub(crate) fn named_sibling_visits() -> usize {
    NAMED_SIBLING_VISITS.get()
}

#[cfg(test)]
pub(crate) fn reset_named_sibling_visits() {
    NAMED_SIBLING_VISITS.set(0);
}

#[cfg(test)]
mod tests {
    use super::{Position, named_sibling_visits, parse_position, reset_named_sibling_visits};
    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};

    fn texts(rule: Value, source: &str) -> Vec<String> {
        let compiled =
            compile_typescript_rule(rule).unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        matched_texts(&result, source)
    }

    #[test]
    fn positions_parse_as_upstream_an_plus_b() {
        let parsed = |text: &str| parse_position(text).map(|p| (p.step, p.offset));
        assert_eq!(parsed("3"), Ok((0, 3)));
        assert_eq!(parsed("2n+1"), Ok((2, 1)));
        assert_eq!(parsed(" -n + 3 "), Ok((-1, 3)));
        assert_eq!(parsed("n"), Ok((1, 0)));
        assert_eq!(parsed("+5"), Ok((0, 5)));
        assert_eq!(parsed("-2n-1"), Ok((-2, -1)));
        assert_eq!(parsed("2x"), Err("illegal character `x`".to_owned()));
        for bad in ["", "+", "n+", "2n3", "nn", "++1", "1+"] {
            assert_eq!(parsed(bad), Err("invalid syntax".to_owned()), "{bad}");
        }
        let odd = Position { step: 2, offset: 1 };
        assert_eq!(
            (0..6).map(|index| odd.holds(index)).collect::<Vec<_>>(),
            [true, false, true, false, true, false]
        );
        let first_three = Position {
            step: -1,
            offset: 3,
        };
        assert_eq!(
            (0..5)
                .map(|index| first_three.holds(index))
                .collect::<Vec<_>>(),
            [true, true, true, false, false]
        );
    }

    #[test]
    fn nth_child_counts_named_siblings_from_one() {
        let source = "f(a, b, c);\n[x, , y];\n";
        assert_eq!(
            texts(json!({ "kind": "identifier", "nthChild": 2 }), source),
            ["b", "y"]
        );
        assert_eq!(
            texts(
                json!({ "kind": "identifier", "nthChild": "2n+1", "inside": { "kind": "arguments" } }),
                source
            ),
            ["a", "c"]
        );
        assert_eq!(
            texts(
                json!({ "kind": "identifier", "nthChild": { "position": 1, "reverse": true } }),
                source
            ),
            ["c", "y"]
        );
        // ofRule counts only the siblings it matches, and binds what it binds on the node itself.
        let source = "f(1, a, 2, b);\n";
        assert_eq!(
            texts(
                json!({ "kind": "identifier", "nthChild": { "position": 2, "ofRule": { "kind": "identifier", "pattern": "$X" } } }),
                source,
            ),
            ["b X=b"]
        );
        // A root has no parent, so no position.
        assert_eq!(
            texts(json!({ "kind": "program", "nthChild": 1 }), "a;\n"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn flat_named_siblings_are_visited_once_per_parent() {
        let source = format!(
            "f({});",
            (0..2000)
                .map(|index| format!("a{index}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        reset_named_sibling_visits();
        assert_eq!(
            texts(json!({ "kind": "identifier", "nthChild": "n" }), &source).len(),
            2001
        );
        assert!(
            named_sibling_visits() <= 4_000,
            "visited {} named siblings",
            named_sibling_visits()
        );
    }

    #[test]
    fn of_rule_positions_over_a_flat_list_stay_within_a_linear_budget() {
        const ARGUMENTS: usize = 500;
        let source = format!(
            "f({});\n",
            (0..ARGUMENTS)
                .map(|index| if index % 2 == 0 {
                    format!("a{index}")
                } else {
                    index.to_string()
                })
                .collect::<Vec<_>>()
                .join(", ")
        );
        // A quadratic probe of every sibling for every candidate (250,000 steps) exceeds this
        // budget; one pass over each parent's children stays far below it.
        let capabilities = capabilities_with(|capabilities| {
            capabilities["limits"]["maxVisitedNodes"] = json!(20_000);
        });
        let rule = json!({ "kind": "identifier", "inside": { "kind": "arguments" },
            "nthChild": { "position": "2n", "ofRule": { "kind": "identifier" } } });
        let compiled = compile_document_with(
            clause_document(json!({ "id": "c", "forbid": { "ast": {
                "language": "typescript", "rule": rule } }, "violation": violation() })),
            &capabilities,
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", &source);
        assert_eq!(result["failures"], json!([]), "{result}");
        let expected = (0..ARGUMENTS)
            .step_by(2)
            .skip(1)
            .step_by(2)
            .map(|index| format!("a{index}"))
            .collect::<Vec<_>>();
        assert_eq!(matched_texts(&result, &source), expected);
    }

    #[test]
    fn a_sibling_of_unknown_verdict_leaves_the_position_unknown() {
        let source = "f(a, q, b);\ng(c, d);\n";
        let of_rule =
            json!({ "position": 2, "ofRule": { "kind": "identifier", "testUnknown": "^q$" } });
        assert_eq!(
            texts(json!({ "kind": "identifier", "nthChild": of_rule }), source),
            ["d"]
        );
        // Unknown is neither true nor false: its negation holds nowhere in `f`'s arguments.
        assert_eq!(
            texts(
                json!({ "kind": "identifier", "inside": { "kind": "arguments" }, "not": { "nthChild": of_rule } }),
                source,
            ),
            ["c"]
        );
    }

    #[test]
    fn nth_child_shapes_are_checked() {
        for (value, expected) in [
            (
                json!("2x"),
                "/clauses/0/forbid/ast/rule/nthChild: illegal character `x`",
            ),
            (
                json!("n+"),
                "/clauses/0/forbid/ast/rule/nthChild: invalid syntax",
            ),
            (
                json!(-1),
                "/clauses/0/forbid/ast/rule/nthChild must be a non-negative integer, an An+B string, or { position, ofRule, reverse }",
            ),
            (
                json!({ "ofRule": { "kind": "identifier" } }),
                "/clauses/0/forbid/ast/rule/nthChild/position is required",
            ),
            (
                json!({ "position": 1, "of": {} }),
                "unknown field `of` at /clauses/0/forbid/ast/rule/nthChild",
            ),
            (
                json!({ "position": 1, "reverse": 1 }),
                "/clauses/0/forbid/ast/rule/nthChild/reverse must be a boolean",
            ),
            (
                json!({ "position": "x" }),
                "/clauses/0/forbid/ast/rule/nthChild/position: illegal character `x`",
            ),
            (
                json!({ "position": 1, "ofRule": { "kinds": "identifier" } }),
                "unknown field `kinds` at /clauses/0/forbid/ast/rule/nthChild/ofRule",
            ),
        ] {
            let rule = json!({ "kind": "identifier", "nthChild": value });
            assert_eq!(
                rejection(compile_typescript_rule(rule.clone())),
                expected,
                "{rule}"
            );
        }
    }

    #[test]
    fn extreme_offsets_evaluate_without_overflowing() {
        let source = "a;\nb;\n";
        assert_eq!(
            texts(
                json!({ "kind": "expression_statement", "nthChild": "n-2147483647" }),
                source
            ),
            ["a;", "b;"]
        );
        assert_eq!(
            texts(
                json!({ "kind": "expression_statement", "nthChild": "-n-2147483647" }),
                source
            ),
            Vec::<String>::new()
        );
    }
}
