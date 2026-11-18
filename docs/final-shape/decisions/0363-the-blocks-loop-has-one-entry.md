# 0363: The blocks' loop has one entry

Status: decided (the user, 2026-11-17: "just fix the shape of the generated code"). Follows
0349 and 0360.

## Intro

**The problem.** JSON's tier 1, written as Rust (`clif_rust`, the ahead-of-time form), had two
loops over a label switch: `loop { match d { 0 => …, 1 => … } }`. The translator writes that
form only for an irreducible graph, a loop entered at more than one place. It is what a `goto`
becomes: rustc compiles it to a jump table that every transition passes through. 0343 removed
the same form from the walk by hand. The lean lexer is nested loops.

The pass had three ways into its blocks' loop:

- `row → head`, the loop's head;
- `row → finish` for a file with no blocks, and `finish → tokens` (the token loop's last part)
  → `after` → `go`, back into the loop;
- `general → tokens` → `after` → `go → body_block`: the token part went back into the hot body
  directly, so the cycle hot body → token part → hot body was entered both at the hot body and
  at the token part.

## The decision

The blocks' loop has one entry, its head:

- the row jumps to `head`, which sends a file with no blocks to its end;
- the token part goes back to `head`, which takes the hot body or the general one, as it does
  for every block.

`head` then dominates every block in the cycle, so the graph is reducible. The head's two
compares now run once a token part (every 16 blocks) on the way back.

## Tests

- rr_parser `json_tier1_has_no_label_switch` (new): JSON's tier 1 as Rust has no loop over a
  label switch. Before this change it had 2.
- rr_parser: the JSON equality tests (789 files the same), `the_cpus_records_are_the_interpreters`,
  `rrs_records_are_rows_on_the_cpu` and the others of 0362's list.
- parser_tests: the JSON tests and the loop guards.
