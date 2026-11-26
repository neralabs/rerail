# python-35

All rules follow Ruff on the fleet box (`ruff check --select CODE --no-cache FILE`) when Ruff and the JSON differ.

## ruff-com818-trailing-comma-on-bare-tuple

Followed Ruff and the JSON. The Python grammar does not wrap every bare one-element tuple in an
`expression_list`, and the comma is absent from the structural tape. The rule now recognizes an expression or
return statement whose source ends at a comma, allowing a trailing comment or explicit continuation, and reports
the captured comma. Counterexample fixed: `pytest.param("x"),` is COM818 even when its expression is a `call`.

Library/core changes: none.

## ruff-era001-commented-out-code

Followed Ruff. The hand-written parse approximation now accepts three Python forms the JSON and Ruff report:
PEP 701 f-strings such as `# x = f"{d["k"]}"`, PEP 695 function type parameters such as
`# def f[T](x: T) -> T: return x`, and `# lazy import os`.

Ruff's final decision is whether the stripped comment parses as a Python module. That remains inexact without the
native parser relation already requested by the original batch:

```rerail
.decl py_parses(s: symbol) // s parses as a Python module under Ruff's parser
```

It should be demand-driven and cached once per distinct symbol. Then `era_parses` can replace its bounded regex
grammar with `py_parses(l)`. A representative false positive of the approximation is prose such as
`# (See: http://example.test/revision)`, which Ruff does not report.

Library changes: none. Engine gap: `py_parses` above.

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
`x: "''.join([a, b])"` is FLY002. The engine's file-tree scan does not expose those guest calls to this rule;
explicitly demanding `parsed_in(c, _)` and replacing the child aggregates with an index walk still produced no
guest findings.

Engine gap: expression rules need a generic relation that exposes guest expressions to ordinary tree relations,
for example:

```rerail
.decl guest_expr(e: node) // every expression in a parsed annotation guest tree, demand-driven
```

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
