# 0167: A computed value waits for the joins that narrow its arguments

Status: decided (2026-11-13)

## Intro

A rule's body can compute a value: `$s = @rr_node_text($n)`, `$n = @strlen($s)`, `@cat(…)`. The planner places
each filter or comparison as soon as its inputs are bound (`close`, and its twin `closure` in the subset search).
For a test that is right, since it only drops rows. A computed value is different: it makes a new value for each row,
and a text value is interned as an atom, which the store keeps.

So a value computed before a join that only narrows its rows is computed, and interned, for rows that join then
drops.

## What it cost

The rules that read `text(v, t)` of a node of any kind get twins reading long texts by `@rr_node_text(v)` (0109).
The twins add `rr_long_text_node(v)`, the nodes past the 256-byte inline bound, so the planner has a small domain to
seed from. `def_name`'s twin, over posthog/hogql:

```
seed def_name($f, $s): rr_long_text_node($n)      40,809 rows: every long node
  , $s = @rr_node_text($n)                        40,809 texts read and interned
  , field($f, "name", $n)                         0 rows kept: a name is never long
```

The long nodes are whole modules, classes, functions and blocks, nested in one another. One hogql run left 77 MB of
atoms in its store, 60 MB of it in strings over 100 bytes. Past `READ_WHOLE` (64 MB) a store is not read whole, so
the next run over it looked atoms up inside its waves (0160's open point).

## The decision

A comparison `v = <call>` with `v` unbound waits while a positive atom still to join reads one of the call's
variables and not `v`. That atom can only narrow the rows the call would run on. Once no such atom is left, the
value is placed as before. If an atom needs `v`, the value is not held back, so a join keyed by a computed value
keeps its key.

## Measured

posthog/hogql, profiling build, strict, fresh store:

| | before | after |
|---|---|---|
| texts over 300 KB interned | 17 | 0 |
| atoms in the store | 485,271, 77.1 MB | 451,031, 27.9 MB |
| findings | 1,629 | 1,629 |

`def_name`'s twin now reads `field($f, "name", $n)` before the text and computes none.

Also in this change: `literal_str` (python/names.rr) binds the kind of the subscript's value (`identifier` or
`attribute`, the only nodes spelling `Literal` or `x.Literal`). It read the text of any node, so the projection
demanded long text of every kind (`nodes with text: every node`). It now demands none.

## Prior work consulted

- 0038 (the join search and its greedy order), 0099 item 4 (an equality binds before the next join), 0108 §1 and
  0109 (long text read only for the nodes a rule binds), 0160 (the atoms past `READ_WHOLE`).
- ast-index-spike: its plans computed no interned values inside joins.
