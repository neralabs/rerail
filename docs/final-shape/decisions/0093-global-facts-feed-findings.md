# 0093: A fact over every document feeds findings, not rules; lints for the forms that have a better rule

Status: accepted (2026-11-08). Decided, not built. The census (§1) is measured; the check (§2), the lints (§3) and
the engine fixes (§4) are not built.

## Intro

Decision 0092 sorts derived facts into two classes:

- **unit facts**, whose rows each belong to one definition, and which read other definitions only by a value they
  hold;
- **global facts**, where one edit anywhere can change a row: "does anything exist", counts over everything,
  recursion across files.

An edit costs what its delta reaches. A unit fact's delta is the edit's. A global fact's can be the whole relation,
and everything that reads it inherits that. One global flag read by a per-file rule makes that rule global too:

```text
// pure-lazy-runtime-purity.rr
plp_anycand() :- plp_cand(_).                    // "is there a candidate file anywhere?"
plp_docin(d)  :- plp_cand(d).
plp_docin(d)  :- doc(d, _, _), !plp_anycand().   // if not, analyse every file
plp_in_rec(f) :- pc_rec(f), docof(f, d), plp_docin(d).
```

The first candidate that appears in any file changes `plp_docin` for every other file, and with it the whole
analysis behind it.

Policy authors don't write rules with invalidation in mind, and they shouldn't have to. But some of these forms
have an equivalent rule that is per-unit, and the rest are reports: they read final facts and produce a finding or
a number that no other rule needs. This record says where a global fact may be read, and what the engine does
about the forms that have a better rule.

## Decision

### 1. The census: how the data-layer checks read other files

`RERAIL_SPLIT_DUMP` classes every cross-file rule of a run by 0092 §4's test, from the engine's own split. Over the
75 data-layer candidate checks on 133 files of the data layer, after the context fix of a30c360ee, 69 reached the
split. The class is the worst over the check's rules:

| Class | The check's own rules | With the library's and lowering's rules |
|---|---:|---:|
| file-local | 17 | 6 |
| same-unit | 25 | 11 |
| union of per-file rows | 1 | 19 |
| recursion within one file | 6 | 7 |
| config | 1 | — |
| lookup by a value | 10 | 14 |
| exists anywhere | 2 | 4 |
| aggregate by a cross-file key | 2 | 3 |
| total over everything | 2 | 2 |
| recursion across files | 3 | 3 |

By their own rules, 50 of the 69 checks are unit facts, and 10 more are lookups. The gap between the two columns
is the engine's, not the policies': §4.

The 24 checks with cross-file rules of their own use five forms:

| Form | Example | What it is |
|---|---|---|
| 1. a set of values from every file, then tested | `bc_layer(path) :- bc_path(_, _, path), match(…, path)` | a per-row test, written as a set |
| 2. a lookup or absence by a key | `LAO002(…) :- …, lao_target(f, t), !lao_lookup(0, t, _)` | real cross-file meaning, by value |
| 3. "anything anywhere", gating rules | `pos_any() :- POSTOP200(_, _, _)`, `plp_anycand()`, `cpl_clean()` | a global flag |
| 4. a summary of the whole repository | the noqa ratchet's `count : { hit(_, _) }`, duplicate keys across files, stale allowlist entries | a report |
| 5. recursion across files | `plp_shared` and `plp_colored` over `pc_edge`, `pha_helper`, re-export chains | resolution and reachability |

### 2. A global fact may be read only by findings

**The rule.** A relation of the global class, and every relation that reads one, may be read only by:

- an output relation: a finding, read by the findings layer (`FINDINGS-LAYER.md` §1, stages 2–7);
- another relation that is itself only read by output relations.

So everything downstream of a global fact is a **report**: rules that run once, after the unit facts reach their
fixpoint, over final facts. A report is derived again each run, or counted where it aggregates. Its cost is the
size of what it reads, not a multiplier on every unit fact.

**Where it is checked.** At lowering, on the program the engine runs, after libraries are combined, as the ban on
aggregation inside recursion is checked. The planner already classes every relation; the check is a walk from the
global ones to their readers. A refused program is told which relation is global, why, and which rule read it:

```text
plp_docin reads plp_anycand, a global fact (anything anywhere: plp_anycand() :- plp_cand(_)),
and plp_in_rec, which is not a finding, reads plp_docin.
A global fact may be read only by findings (decision 0093).
```

**Forms 3 and 4 under the rule.**

- `pos_any()` is read by one finding, `POSTOP000(@place(…)) :- !allow_status(…), !pos_any()`. It is allowed as it is.
- `cpl_clean()` gates `cpl_sign_msg`, a message relation of findings. Allowed.
- The noqa ratchet, duplicate keys, stale allowlist entries and the signing library's `allow_message` and
  `allow_status` are read only by findings. Allowed.
- `plp_anycand()` selects the files that the analysis runs over. That is a global fact feeding unit facts, and it
  is refused. The rewrite runs the analysis over every file it could select, and moves the condition to the output:

  ```text
  plp_in_rec(f) :- pc_rec(f).                                   // analyse every file
  PLP001(n)     :- plp_hit(n), docof(n, d), plp_cand(d).        // report where there are candidates
  PLP001(n)     :- plp_hit(n), !plp_anycand().                  // or everywhere, when there are none
  ```

  That is equivalent only when the analysis of a file doesn't change with which other files are analysed. Here it
  does: `plp_colored` follows `pc_edge` only between analysed functions. Such a check needs the scope as an input
  of its own, and §5 leaves that open.

**Why not a softer rule.** A global fact read by unit facts is correct, and DRed maintains it. The rule isn't about
correctness. It is a cost contract: no unit fact pays for the whole repository. Without it, the engine's
incremental cost would depend on rules nobody knows to be global.

### 3. Lints for forms 1 and 5, which have a better rule

These are warnings with the rewrite, not errors: the rules are correct, but one with the same meaning is cheaper.

**Form 1, a value set then tested.** The reader already holds the value:

```text
bc_layer(path) :- bc_path(_, _, path), match(RX, path).               // every path, from every file
bc_public(d)   :- bc_path(d, _, path), !bc_impl(d), bc_layer(path).   // d's own path, looked up in that set

// the same, with no set: the test on d's own value
bc_public(d)   :- bc_path(d, _, path), !bc_impl(d), match(RX, path).
```

The lint fires when a relation projects a document's rows to a set of values, and a reader binds the value from a
row of its own document. Where the projected relation and the reader's relation are the same, as here, the engine
also does the rewrite itself, since `bc_layer(path)` is then implied by the reader's own row and the test.

**Form 5, recursion across files.** Reachability over call edges and re-export chains belongs to the library's
phases (0083): resolution and calls, each derived once, with symbol keys. A policy that closes `pc_edge` itself
re-derives a closure per policy, keyed on nodes. The lint names the library relation to read. Until those phases
exist, the lint is informational.

### 4. Engine leaks: global facts nobody wrote

Three things in lowering and the split make unit facts look global. They are bugs, fixed in the engine, not
restrictions on policies.

1. **A context that loses its tie to the rule** (fixed in a30c360ee). A demand helper kept `ann_root(a)` and
   dropped `within(s, a)`, so "parse this string" became "parse every string, if any annotation exists anywhere".
   It made 51 of 69 checks global and was the cause of two checks not finishing in 900 s; they now take 17 s.
2. **Located rows projected to a set** (`x(c) :- x_at(_, c)`). Lowering drops the document column of a per-file
   relation, and its readers read the union. A reader that binds the document reads `x_at` with it; the projection
   only feeds findings.
3. **Joins between per-file pieces placed in the cross-file part.** When one rule has two per-file pieces joined by
   an equality between their carriers (`deco_word`'s `$e0 = $d`), the split runs that join across files. An equality
   between two columns that each name one document is one document: the split joins the pieces in the file.

The findings themselves (`rr_diagnostic` and `violation`) are assembled today by cross-file rules that drop the
document. They are form 4 by construction, and they are the findings layer's stages 2–3, not Datalog's.

### 5. What is not decided

- **A scope chosen by a global fact**, as `plp_anycand` chooses one. The candidates are a phase whose output is the
  scope of the next (0083 §1), keyed on its value. A flip then derives the next phase again whole, as DRed would. It
  is decided when a second check needs it.
- **Whether the restriction is an error at once.** It is an error for new checks. The checks it refuses today are
  listed in the open item, and each gets its rewrite before the error applies to them.

## What the prior work says

- **Decision 0092:** the classes and the cost bound of each. *This record's ground.*
- **FINDINGS-LAYER.md §1–§2:** findings are keys; stages 2–7 run after evaluation, once; an incomplete input makes
  places Unknown. *Agrees*: reports run there, and their inputs being final is what lets them run once.
- **Target architecture (03), §2:** only changed facts propagate. *Agrees*: the restriction keeps the changed set
  of a unit edit the edit's.
- **Decision 0083 §1, §3:** phases over separate fact spaces; exports by symbol. *Agrees*: form 5 is a phase's job,
  and §5's scope is a phase output.
- **Decision 0077:** DRed maintains global facts correctly. *Kept*: the restriction is about cost, not correctness.
- **Decision 0091 (final-shape-serverplan-quality):** library facts are canonical, and a policy's rules see only
  its scope. *Agrees*: a scope is a per-document guard, not a global fact.
- **The findings contract:** output is a set of finding keys, with no exact-shape parity. *Agrees*: a report may
  derive the same keys from final facts in any way.
- **Lessons ledger #14:** each file owns its rows. *Agrees*: §4's leaks are rows that lost their owner.

## Open

- O140: the engine leaks of §4, items 2 and 3, and the findings assembly.
- The classifier is the dev measurement `corpus::dump_split`; the check of §2 needs it in the planner, with a test
  per form.
- The 6 checks that did not reach the split are classed by hand only for the 2 still failing (O138: they read run
  artifacts).
