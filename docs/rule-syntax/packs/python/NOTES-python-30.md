# NOTES python-30

Ground truth: Ruff on a fleet box (`ruff check --select <CODE> --no-cache`). The JSON original and Ruff agreed on
all baseline differences. Final comparison used `ptcheck` on examples, fixtures and samples-python.

## ruff-perf101-unnecessary-list-cast

Followed the original and Ruff. The translation now treats bindings in the loop body and a containing function's
`nonlocal` declaration as changes that can make the copied list necessary. Counterexample:
`x = [1]; for i in list(x): def x(): pass` is not PERF101. A rebinding to a known list, tuple or set remains
harmless and does not suppress the finding. Final `ptcheck`: same.

## ruff-perf102-incorrect-dict-iterator

Followed the original and Ruff. Parentheses around the `.items` callee do not hide the call:
`for k, v in (d.items)(): print(k)` is PERF102. An unread target is left alone when its immediately preceding
binding was read, as in `v = 1; print(v); for k, v in d.items(): print(k)`; an earlier unread loop binding does not
suppress a later finding. Final `ptcheck`: same.

## ruff-perf401-manual-list-comprehension

Followed the original and Ruff. List-valued receivers include `with list() as result`, a function name with a list
return annotation, and ordinary list bindings; redundant target parentheses are opened recursively. Counterexample:
`out = []; for ((i)) in items: out.append(i + 1)` is PERF401. A loop target declared `nonlocal` by a nested function
is excluded. Final `ptcheck`: same.

## ruff-perf402-manual-list-copy

Followed the original and Ruff. It shares PERF401's list-receiver and recursively parenthesized-target vocabulary.
Counterexample: `out = []; for ((i)) in items: out.append(i)` is PERF402. Final `ptcheck`: same.

## ruff-em101-raw-string-in-exception

Followed the original and Ruff. A backslash continuation immediately after `raise` no longer hides the raised call,
and bytes use the decoded `str` fact for their length because bytes deliberately have no scalar `value` fact.
Counterexamples: `raise \\` followed by `ValueError("bad")`, and `raise ValueError(b"bad")`, are EM101. Final
`ptcheck`: same.

## ruff-em102-f-string-in-exception

Followed the original and Ruff. It shares the continuation-aware raised-call relation. Counterexample: `raise \\`
followed by `ValueError(f"bad {x}")` is EM102. Final `ptcheck`: same.

## ruff-em103-dot-format-in-exception

Followed the original and Ruff. It shares the continuation-aware raised-call relation and opens parentheses around
the `.format` callee. Counterexample: `raise ValueError(("{}".format)(x))` is EM103. Final `ptcheck`: same.

## Library and engine notes

`lib/python/python-30.rerail` adds the concepts shared by at least two rules: recursively parenthesized plain-name
loop targets, list receivers bound by `with` or a function return annotation, and raised calls after a backslash
continuation. No engine change or new primitive is required.
