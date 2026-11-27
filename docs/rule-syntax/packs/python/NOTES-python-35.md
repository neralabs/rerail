# python-35

All rules follow Ruff on the fleet box (`ruff check --select CODE --no-cache FILE`) when Ruff and the JSON differ.

## ruff-com818-trailing-comma-on-bare-tuple

Followed Ruff and the JSON. The Python grammar does not wrap every bare one-element tuple in an
`expression_list`, and the comma is absent from the structural tape. The rule now recognizes an expression or
return statement whose source ends at a comma, allowing a trailing comment or explicit continuation, and reports
the captured comma. Counterexample fixed: `pytest.param("x"),` is COM818 even when its expression is a `call`.

Library/core changes: none.

## ruff-era001-commented-out-code

Followed Ruff. The batch's hand-written parse approximation was extended to three Python forms the JSON and Ruff
report: PEP 701 f-strings such as `# x = f"{d["k"]}"`, PEP 695 function type parameters such as
`# def f[T](x: T) -> T: return x`, and `# lazy import os`.

Ruff's final decision is whether the stripped comment parses as a Python module. The batch's bounded regex grammar
approximated that and accepted prose such as `# SIM103 (but not fixable)` and `# (See: http://example.test/revision)`
(22 fixture and 24 sample lines, original right). Fixed since (rule fixes on #1120's line): the rule asks Ruff's own
parser through the native `@py_parses`, and Ruff's trivia-skipping tokenizer through `@py_consecutive_names` for the
two-names-in-a-row prose test; no regex grammar or word scan is left. The prefix trim is Python whitespace only
(space, tab, form feed), and the issue-hash filter takes Unicode digits, as Ruff's. `translations:check`: fixtures
82 shared findings, none on one side only; samples-python 232 shared and 6 translation-only, the quoted-attribute
annotation comment `'month'. Eg: "Title must be unique for pub_date year"` in six Django samples. Ruff 0.17.6 over
samples-python reports exactly the translation's 238 findings, so the 6 are translation right.

Library changes: none.

## ruff-fast001-fast-api-redundant-response-model

Followed Ruff. Type comparison now ignores grouping parentheses and a trailing subscript comma while preserving
other tuple punctuation. This makes `response_model=List[(Item)]` equal `-> List[Item]`.

The JSON original is wrong for `response_model=List[Item,]` versus `-> List[Item]`: Ruff reports FAST001 and the
translation keeps that finding. Library/core changes: none.

## ruff-fix002-line-contains-todo

Followed Ruff and the JSON. The rule now reads the first task directive across `#`-separated segments, using Unicode
letters and numbers for the word boundary. Thus `# TODOé` and `# TODO²` are not findings, while `# TODO→ work` is;
`# FIXME # TODO` is not FIX002 because the first task directive wins.

Library/core changes: none. `python/misc-comments.rerail` still has the ASCII boundary used by the other FIX rules;
this rule keeps the exact Unicode form locally because existing library files are outside this batch.

## ruff-fly002-static-join-to-f-string

Followed Ruff for source trees. Ruff also runs expression rules inside parsed string annotations, so
`x: "''.join([a, b])"` is FLY002. In the batch the engine's matchers did not run over guest trees, so the rule saw
none of those calls (4 fixture keys, original right). Fixed since (rule fixes on #1120's line): the engine's AST
matchers read parsed Python guests, and `static_join` also takes calls in a type string (`annotation(c)`), not
Literal values or ordinary strings. `translations:check`: `same` (fixtures 19, samples-python 9 shared findings).

Library changes: none.

## ruff-icn001-unconventional-import-alias

Followed Ruff and the JSON. Python NFKC-normalises identifiers before Ruff sees them; the translation now applies
`@nfkc` to module components and aliases. Counterexample fixed: `import ｐandas` is an import of `pandas` and must be
aliased as `pd`.

Library/core changes: none.

## ruff-pgh005-invalid-mock-access

Followed Ruff and the JSON. The assert operand is now the first non-comment, non-line-continuation child, and a
parenthesized call target is unwrapped before reading its attribute. Counterexamples fixed:
`assert \\` followed by `m.not_called()`, and `assert (m.not_called)()`.

Library/core changes: none.

## ruff-pie794-duplicate-class-field-definition

Followed Ruff and the JSON. Tree-sitter represents `(name) = value` as a comma-less one-element `tuple_pattern`;
the rule now unwraps that target before comparing class field names. Counterexample fixed: consecutive
`(__metaclass__) = type` and `((__metaclass__)) = type` definitions make the latter PIE794.

Library/core changes: none.
