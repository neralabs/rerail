# 0137. Join shape directives in the rule source, and no cross product priced from a default

Status: built (2026-11-12). Closes O-join-shape-directives.

## Intro

The planner orders a rule's joins from statistics. When a statistic is missing or wrong, it can pick an order
that does far more work than needed. At O125 step 07, `pv__ins` (a maintenance form of a `pv` rule, which no one
writes) crossed `pv` with `retparam`. The two share no variable, so 1,553 rows became 159,959. It also read the
15-row input delta `posarg__plus` last. The cause was a missing statistic: every input delta was priced at the
1,000-row default. That is fixed (6b04cb8d2). The next case like it should be fixable from the `.rr` file, without
rebuilding the engine.

This decision adds two things:

- a directive, written after a rule, that holds one join back;
- a planner guard that takes no cross product priced from a default size.

## The directive

```
// A call's positional argument to a parameter of the function called.
pv(c, k, v) :- pv0(a, k, v), posarg(c, i, a), cfn(c, _, f), retparam(f, i, _).
// Join retparam only once the call's function is known.
.shape retparam bound f
```

Two forms:

| Form | Meaning |
|---|---|
| `.shape B after A` | the literal of relation `B` is joined only once a literal of `A` is |
| `.shape B bound x, y` | `B` is joined only once the variables `x` and `y` are bound |

- **Where it applies:** the rules of the relation just above the directive, and every form made from them: their
  delta forms, their maintenance forms (`__ins`, `__del`, `__redo`, `__supN`) and the hiding copies' renamed rules.
  `front::shape_base` maps a form's relation back to the written one, so `posarg__plus` in `pv__ins` is `posarg`.
- **The driver:** the delta that drives a form is never held back.
- **It narrows the search; it does not fix an order.** A whole fixed order would be wrong for most derived forms.
  The planner still searches within the waits, as it does for unit gates (`gate_waits`), in both the DP and the
  greedy search. A kept order the directives do not admit is searched again. The directives are part of the plan
  key, so a plan kept without them is never reused.
- **Checked at lowering:** `.shape` after no rule, a relation no rule of that head reads, or a variable none of its
  rules has, is a syntax error with the line.
- **Merged with the program:** `combine` carries each program's directives, renamed as its relations are, and the
  plan-choices digest includes them.

## Explained and checked

- **EXPLAIN:** a shaped plan's search line ends `shaped by `.shape retparam bound f``
  (`profile::RulePlan::shaped`).
- **Profiled runs** also plan each shaped rule without its directives and record one of these decisions:
  - *join shape directives*: both estimates, the shaped plan's and the planner's own;
  - *a join shape directive no longer needed*: the planner's own order is the directive's. Remove it.

  The test is the order, not the estimate. A directive is written where the estimates are wrong, so a higher
  estimate for the shaped plan says nothing.
- **Unsatisfiable:** a directive that leaves no order (for example two directives waiting on each other) is
  planned without, and the profile says so.

## Each use opens an item

A directive hides its cause. Anyone who adds one also opens the item that fixes why the planner chose badly: a
missing statistic, a wrong containment, a cost the model lacks. O125's case would have hidden the missing delta
statistics. The next delta reader would have hit them unseen. This is a rule for authors. The engine cannot check
it.

## The cross-product guard

A step that shares no variable with what is joined is a cross product. While a connected step remains, the DP
does not take such a step where it, or a connected step it would go before, has no measured size. A relation
priced from `card`'s 1,000-row default has no size: it is not in `stats.card`, not lazy and not being solved.
The DP never had this rule. Greedy already avoided every cross product (`disconnected`).

## Tests

`plan::tests::a_join_shape_directive_holds_a_join_back_in_the_rule_and_its_forms`, O125's shape in small, checks:

- without the directive, the planner crosses `retparam` in early;
- `.shape retparam bound f` holds it back after `cfn`, in the rule and in its `out__ins` form, and EXPLAIN names it;
- `.shape retparam after cfn` does the same;
- profiled, `.shape pv after posarg` (already the planner's order) is reported as no longer needed;
- lowering errors;
- the guard: with `pv`'s delta driving and `posarg` unsized, `retparam` waits for `posarg`. With the guard off,
  the order is `pv, retparam, cfn, posarg`, O125's.
