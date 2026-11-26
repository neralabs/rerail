# 0390. A label starts only where the rule that reads it can hold; a nested `inside` stays a range

Status: built (audited 2026-11-23). Accepted (2026-11-17)

## Intro

A matcher's rule is lowered into labels: node sets, each filled by a pass of walks from its starts. A label
used inside an `all` matters only at the nodes the `all` can match. Until now, though, it started from every
node its own rule allows. internal_dead_code's parameter rule has `not: {precedes: positional_separator,
stopBy: end}` beside `kind: identifier` and `inside: parameters`. Its sibling scan ran from all 877,007 nodes at
s1000, a module's statements included. That scan was the 288-step walk 0380 left open (Q182's leader).

0380 also left a second question: does a nested `inside: X, stopBy: end` repeat work? Each X marks its whole
subtree, so a node under k nested X's is marked k times. The user asked for both to be tried, measured on
internal_dead_code and public_api_triples, "so we have all the information we need to know how to move
forwards" (2026-11-17).

## Decision

1. **A label made for a conjunct starts where the conjunct is needed** (`Builder::demand`, `narrowed`).
   - While lowering an `all`, the builder carries its kinds: the kinds every conjunct allows
     (`MatchTree::kinds`), within the kinds carried in from outside. Each `any` alternative carries its own.
   - A pattern, constrained or roots label made there starts from those kinds alone. A forward count's starts
     are those kinds too.
   - These are made with no demand:
     - a walk's rule, which is tested at the node the walk reaches;
     - a constraint and a fact's slots, tested at their own nodes;
     - a recursive utility and a roots label's own program (a fresh builder).
   - Narrowing only drops starts where the label's value is never read. A label is interned by its domain, so a
     narrowed label is never shared with a use that reads it elsewhere.
2. **`inside: X, stopBy: end` stays a descendant range.** The rewrite was a closure from each X that stops at
   a nested X, which marks each node once, by its nearest X. It was built and measured, and it is not kept:
   - At s1000 it saved no step. The pack of two such labels that every node starts took 2,660,719 steps either
     way. A `desc` is a scan of 64-node windows (0380 item 3), and a nested X's marks fall in the same windows.
   - It moved those 877,009 walks from the short tier to the loop tier (loop-tier walks 301 K → 1.25 M).
     `label_micro_8` + `label_micro` went from 44.5 ms to 66–75 ms.
   - Every `inside … end` in the two programs is a plain kind (`class_definition`, `function_definition`),
     and these nest shallowly in Python.
3. **A child chain is a formula in both orders of its paths** (`compile_sets::chain`), and a fork's optional
   step is one too.
   - `bounded` emits `CHILD; FORK on; GOTO out; on: ψ; GOTO top; out:`, the path that stops first. `chain`
     read only the other order. A micro walk's `CLOSURE` keeps those words, and the formulas read the micro
     words. So no `stopBy: rule` loop was a formula: each ran in the host's interpreter (`targets[17]`).
   - `FORK a; steps; a:` with no `GOTO` (`inside` through a field to `end`: the field child or below it) is the
     union of the node and where the steps reach.
   - With both, every Targets label in the two programs is a formula. `RERAIL_SETS_CHECK` found none that
     differs from its walk.

## Measured

s1000, Metal pinned (`RERAIL_LANE_PIN=gpu`), `RERAIL_GPU_WALK_STEPS=1`, `RERAIL_GPU_KERNEL_TIMES=1`. Every output
relation's rows were compared, native against Metal and before against after. They are the same.

Item 1, the same binary with the narrowing switched off and on by a spike switch, now removed (load about 40):

| Program | Walk steps | Walks | Longest walk | `label_micro_8` | `label_micro` |
|---|---|---|---|---|---|
| internal_dead_code, off | 63.2 M | 12.9 M | 290 | 51.5 ms | 29.5 ms |
| internal_dead_code, on | 47.7 M | 12.3 M | 63 | 39.9 ms | 32.0 ms |
| public_api_triples, off | 26.2 M | 4.1 M | 276 | 32.4 ms | 28.9 ms |
| public_api_triples, on | 19.3 M | 2.7 M | 78 | 30.4 ms | 19.5 ms |

The longest walk at s1000 is now 63 and 78 steps, under Q56's bound of 100. The leader's 288 steps were the
cost of starting the scan at nodes nothing reads.

Item 3, natively, two alternating pairs per program at load 31–35. The column is the label passes' host CPU
(`cpu_ms.labels.fallback`):

| Program | Before | After |
|---|---|---|
| internal_dead_code | 1,139–1,303 ms | 789–839 ms |
| public_api_triples | 2,610–2,947 ms | 2,849–2,863 ms |

On Metal the walks are the device's, and their steps are unchanged to the step (47,738,556 and 19,278,995).

The whole run's instructions varied from 90 G to 146 G between runs of one binary. Step counts also moved
3× between unpinned runs, because the pricing put different waves on the device. So the comparisons above are
pinned and read the passes' own counters.

## What this tells us

- The walks' lengths followed their starts, not the rules' shape. Narrowing a label's starts to its reader's
  kinds is the lever. It is generic: every label built in a conjunct gets it.
- A nested `inside` costs nothing extra once a range is a windowed scan. A dedicated nearest-match scan
  primitive is not needed at this scale. If a corpus with deep nesting of one kind shows it, the closure
  rewrite above is the measured form. It wants the short tier to run closures first.

## Open

- Three of internal_dead_code's relations (`raw_base`, `raw_parameter`, `raw_decorator`) have 0 rows natively
  and on Metal. ast-grep finds 913, 14,717 and 2,272 over the policy's 339 files. An identical unread copy of a
  rule gives the right rows to both copies (913, 14,717, 2,271). Matching is right, and something about the
  relation as its readers bind it is not (work queue Q142).
- 0380's tables cite a findings md5 as the check. That was the md5 of a constant: these programs find nothing
  at s1000. The check that holds is each output relation's rows, as here.
