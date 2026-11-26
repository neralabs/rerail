# Batch misc-untranslated: python

## bandit-b105-hardcoded-password-string: different, the original right (an engine gap)

New translation, from Bandit 1.9.4's `hardcoded_password_string` (plugins/general_hardcoded_password.py): a str
Constant is reported by its parent in CPython's ast, through python/ast's view (Assign targets of a chain,
`unparen`, adjacent literals as one constant, not bytes or f/t-strings):

- the value of a plain Assign with a candidate target (a Name, or an Attribute's attr);
- a candidate key of a dict display (not a comprehension) whose value is any Constant (`-1` is not);
- a candidate key that is a subscript's whole slice, the subscript a target of an Assign whose value is a str;
- every str operand of a Compare whose left is a candidate Name/Attribute and whose first comparator is a str.

`candidate` is RE_CANDIDATES (`^W$|_W_|^W_|_W$`, ignoring case) as one full match,
`(?is)(?:.*_)?W(?:_.*|\n)?`, the `\n` for Python's `$` before a final newline. Where the JSON differs from Bandit
(parentheses: `password == ("x")`, `d["password"] = x = "s"`), this follows Bandit. On 25 ad-hoc cases (chains,
annotated and augmented assignment, tuples, bytes, f-strings, parenthesised values, concatenation, dict
comprehensions, tuple subscripts, trailing-comma subscripts, chained and reversed comparisons) its lines equal
Bandit's.

The tape keeps no `,` in a subscript, so check-rr refuses `op(sub, ",")`; `d["password",]` (a Tuple slice) is
told apart by the subscript's text instead.

Remaining difference, fixtures, 4 translation-only: `bandit/rerail/nosec/B105.py` lines 3, 5, 7, 9 carry `# nosec`,
`# nosec B105`, `# nosec: B105`, `# nosec hardcoded_password_string`; Bandit and the original honour them. The rule
declares `suppress nosec "B105"`, but `rerail-engine run-policies` returns its diagnostics with no suppression
applied or marked, so the suite counts them. Every Bandit translation shows this on its nosec fixture (B101: 4,
B113: 6). Engine gap: run-policies should apply (or mark) the policy's `suppress` comments as the product path does.

Library changes: none (the review fixes below too).

### Review fixes

1. **Parenthesised targets.** tree-sitter reads `(password) = "a"` as a `tuple_pattern` with no comma, not a
   `parenthesized_expression`, so `unparen` kept it and the target matched nothing. `bare_target` now takes the
   parentheses off a target: `unparen`, and through a comma-less one-element `tuple_pattern` (`paren_target`, stored
   as misc2-perf's and misc2-slots' are). `(x,) = …` stays a tuple.
2. **A comment inside a comparison.** `child(c, l, 0)` / `child(c, r, 1)` counted a comment as an operand.
   `cmp_operand(c, o, i)` numbers the operands with comments left out (as `body_stmt` does), and the comparison
   clauses use it. `cmp_pair` in `lib/python/ast.rerail` indexes the same way; this rule does not use it.

Bandit 1.9.4 (`-t B105`) against `run-rr`, 20 findings on both sides, identical:

| File | Bandit | Before | After |
|---|---|---|---|
| t1.py: the review's 5 targets, `((password))`, `(password,)`, a tuple, a comment in the parentheses, `x = (y) = (secret) = …` | 1, 2, 3, 4, 5, 6, 10, 11 | 2 | 1, 2, 3, 4, 5, 6, 10, 11 |
| t2.py: the review's comparison, two comments, a non-str first comparator, a str on the left | 3, 7, 8 | nothing | 3, 7, 8 |
| t3.py: plain, annotated, augmented, tuple, dict, subscript, chained, `is not` | 1, 5, 6, 8, 9, 9, 10, 11, 12 | the same | the same |

The negligible case (`"api_token"[0] = "x"`, a str literal as the subscripted value, which raises at run time) is
left as is, as the review suggests.

ptcheck after the fixes: unchanged, fixtures 16 both and the 4 nosec keys translation-only (the run-policies gap
above); samples-python 91 both.
