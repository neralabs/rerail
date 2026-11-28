# 0727. No node crosses, not even a finding's place, and the cross part reads no base relation

Status: decided (2026-11-27), by the user: "no node should cross even as a finding's place. Oscar and Bend have the
solutions needed; the rule files need to be updated", and, of 0458's requirement that the cross part read other
files' base relations, "FUCKING RETIRE IT!". It retires 0458 (R1–R5) and amends 0677, whose output exception (a
finding's place made in the cross part) it removes.

## Intro

0677 kept node ids out of the cross part's relations, but let an output made there carry a node, the finding's
place, decoded off the corpus batch. 0458 built that route: the cross part read `node`, `child`, `field`, `text`,
`line`, `col` and `docof` of every file's nodes off their tapes, in place, and kept their statistics across runs.
Both put node ids in the cross part. This decision takes them out.

## Decision

1. **No node, line, match or token reaches the cross part, as a value of any relation, an output included.** The
   check (`node_crossings`, 2a7aa9b10a) refuses an output, or an output's located form, made in the cross part with a
   column of those types, and names the cross readers of a per-file relation that hands one on.
2. **The cross part reads no base relation.** What a cross rule needs of a node is read in the node's own file, by
   a per-file rule, and handed on as a symbol, a document key, a value or a node path (0722, `@node_path(n)`, the one
   node-made value that may cross).
3. **A finding is made in its node's own file.** The cross part derives a relation of names (no `cross_file`
   declaration needed: a derived relation of names a per-file rule reads is staged before the program, 0709); the
   per-file rule reads it and makes the finding: `used(q) :- imp_at(_, q), def_at(_, q).` cross, then
   `unused(f) :- def(f, q), !used(q).` per file.
4. **0458 is retired**: its requirements R1–R5 (the cross part reads every base relation of another file's nodes,
   its memo hits give those rows, an edit reads the changed file's old and new rows, a base relation's statistics are
   kept and patched, no SQL inside the waves for those reads) have no reader left. Their tests
   (`cross_views_tests.rs`) are removed, with the maintain test of a finding's places moved
   (`a_finding_s_places_moved_are_maintained`), whose program placed findings in the cross part.

## What changes in the rule files

A rule that joined one file's node with another file's facts is split at the node: the node's columns are read in
its own file (`call_col(x, k) :- called(c, x), node_start_column(c, k).`), the other side is projected to names
(`def_name(x) :- defined(_, x).`), and the cross rule joins those (`at(x, k) :- call_col(x, k), def_name(x).`).

## Built

- The check: 2a7aa9b10a.
- Per-file rows placed by `docof`, which answers every id, and nested string annotations found in the chunk:
  ad60941a15.
- Test programs rewritten: base, partial and maintain tests.
- Staging, so a finding is made in its node's file:
  - A relation of names derived from a request's answer is staged, and a guest of an unnumbered host node makes no
    row: d0bc0b6999.
  - A stage asks for the guests its relations read, and a load's answer is staged only through a folded constant:
    ff561d9661.
  - A layered read finds an admitted value's rows by the column's kind, and the cross part copies no view that reads
    a node: 53cf39c914.
  - A view over a world answer (`dir_entry`) is no view its readers cover: 6bfc3616fa.
  - A stage is prepared as the program it is cut from, so its joins are neither deferred nor factorized where the
    program's are not. A node path is a name, so a relation of names may hold one: 2b93934287.
  - A stage's probe keeps the `nearest` and `ast` literals that tie its nodes: d31c5eaa53.
- The Django pack's rules: each finding reads relations of names and node paths, and a path and line are the
  writer's `{w.file}:{w.line}`: get-handler-writes (2b93934287), select-for-update-outside-atomic,
  sync-orm-reached-from-async and dispatch-in-atomic-request (d31c5eaa53). The test
  `the_django_pack_is_split_with_no_node_crossing` passes.
