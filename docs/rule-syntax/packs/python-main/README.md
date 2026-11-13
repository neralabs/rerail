# Main's Python rules, translated for the engine

## Intro

This directory holds main's 537 Python JSON rules (`rerail.policy/v1`), translated mechanically into the
engine's `.rr` rules. It exists to test the Datalog engine against main on every rule, not only on the ones that
translate easily. Because a program does the translation, nobody picks or tunes a rule to suit the engine.

Each `<id>.rr` file is generated. Do not edit one by hand: the next regeneration overwrites it. A rule that the
translator cannot handle gets a hand-written file in `overrides/<id>.rr`, and that file replaces the generated
one. Each override starts with a comment that says why it exists.

Every rule keeps main's logic and regexes. Every rule says `.guests off`, because main matches only files and
never the strings inside them. No rule includes a library beyond the core relations the translator emits.

## How to regenerate

```sh
PACK_RULES=<main's pack>/rules ENGINE=<rerail-engine> RERAIL_RR_LIB=<engine lib dir> \
    docs/rule-syntax/site/regen_main_pack.sh [ids file]
```

The script does three things:

1. It runs `docs/rule-syntax/site/translate_rules.py` over the JSON files. That is `translate.py` (the display
   translation) followed by `engine_dialect.py` (the engine's dialect).
2. It copies the overrides over the generated files.
3. It reader-checks every file with `rerail-engine check-rr`.

The verdicts go to `reader-check.txt`, one line per rule: its status and the first line of any error. Old
generated files are moved to a directory under `/tmp`, not deleted.

## How the tape changes the translation

A tape stores a file's named nodes. It leaves out the tokens its grammar implies, such as the `,` between
arguments, the `as` in an import alias, or the `return` keyword. Main's rules often test those tokens through an
anonymous child or sibling: `has: {regex: "^,$"}` or `follows: {regex: "^as$"}`. The translator answers such a
test from three places:

- a named node;
- a token the tape stores, read with `op` or the parent's `token` list (the relation says `.stored`, with a
  reason);
- a token the tape leaves out, read from the source between stored children (`structural_before` and
  `structural_after`), tested as whole tokens.

A `layout` gap to the neighbouring token reads the line of the code next to the node from those same gaps. It
climbs to the parent where the gap reaches the parent's edge, and steps past comments unless the rule says
`comments: true`.

## Overrides

| Rule | Why |
|---|---|
| `bandit-b110-try-except-pass`, `bandit-b112-try-except-continue` | `pattern: {context, selector}` has no `.rr` form. The bare `except:` it selects is written as an except clause with no child but its block. |
| `ruff-ret505` … `ruff-ret508` | Main matches the `else`/`elif` keyword token and captures it. The tape leaves the keyword out, so the finding is at the clause (same line), and BRANCH is read from the source before its first child. |
| `ruff-com818-trailing-comma-on-bare-tuple` | A capture regex over a node of unknown kind: the statement kinds are named. The pattern_list trailing comma is read from the gap or the stored token. |
| `ruff-pyi010-non-empty-stub-body` | A capture regex over a node of unknown kind: the statement kinds are named. |
| `ruff-n806-non-lowercase-variable-in-function` | Main passes `NAME` in from the caller, and Datalog cannot, so the reader refuses it as ungrounded. Here `NAME` ranges over the nodes of the model's file, and the caller's join keeps its own. A stray `unset(MODEL)` is dropped. |

## Rules that still fail, and the plan to run them all

The pinned source is `source/` (main's Python pack 0.1.8: `pack.json` and `rules/`). `reader-check.txt` holds each
rule's reader verdict, and `run-check.txt` (from `../../site/run_main_pack.sh`) each rule's verdict when run alone
on one file, under the caps (2 minutes, 2 GB).

### The 9 overrides: what makes each expressible

| Rule | What the translator cannot say | The fix that makes it generated |
|---|---|---|
| B110, B112 | `pattern: {context, selector}`: a snippet parsed in a context (`try: … except: pass`), matched at the node of the selector's kind. | The translator parses the context itself and writes out the selected node's shape (its kind and children's kinds and texts), as it already does for a plain pattern. |
| RET505–RET508 | A capture of an anonymous token (`else`, `elif`), which the tape does not store. | The translator already reads such tokens from the source gaps (`structural_before`) for tests; a capture of one becomes the gap's span, with the finding at the clause, which starts with the keyword. |
| COM818, PYI010 | A regex with captures over a node whose kind the rule does not name. | The translator names the kinds the node can have, from the grammar's node types under its parent, as the overrides do by hand. |
| N806 | A util that reads the caller's metavariable (`NAME`). | The translator's `demand()` pass already hands a caller's metavariable to a helper; it is extended to a util whose only binder is the caller's. |

### 14 refused by the reader

- **12 use the `percentFormat` or `braceFormat` grammar** (F501–F503, F506, F508, F509, F521, F523–F525, FAST003, UP031). These
  are main's grammars of the format mini-languages, the `%` printf formats and `str.format`'s `{}` templates. Main parses
  a string's value with Rust ports of Ruff's own parsers (`native/src/code_rule_dsl/literal/percent.rs`, `brace.rs`) and
  hands the rule the tree (`format`, `placeholder`, `mapping_key`, …, and `error` with Ruff's message). The engine's
  guests (decision 0055) parse only the registry's grammars (`languages/*.json`: a dialect or a fragment), so the reader
  refuses the name. **Plan:** make the two parsers guest grammars, giving a guest document with main's tree kinds and the
  error's message as its value. The rules then run as written. The final shape is a transducer program per grammar
  (rank 0 allows a lean parser until then).
- **UP030 and UP032 use `formatArgs` outside `ast(…)`**: the same plan, then `formatArgs` in Datalog (below).

### 13 refused at run time: 9 fixed, 4 left

- **Fixed: not stratifiable** (RUF018, FBT001, N805, PYI051). The engine narrows a negated or aggregated helper with a
  context relation built from the rule's other literals. In a recursive rule (a translator demand) those literals are
  the helper's own component, so the negation closed a cycle. Lowering now widens such a context: it drops the cycle's
  literals from it (`rr::lower::widen_cyclic_contexts`). A wider context gives the same rows where the rule reads it.
- **Fixed: JSON terms** (UP008, SIM113, B615, F402, F823). The translator cut the term out of its matcher (`ast(n,
  {"bindings": …})`), which is no matcher. It now writes the whole matcher (`{"language", "rule"}`), and writes
  `binds.bindings` (a count of the name's bindings) and `binds.redefines` (an earlier binding of the name in the scope)
  in Datalog, since the engine's matcher compiler has neither. `binds.site` is the statement that binds, not the name.
- **Left: `formatArgs`** (F504, F505, F507, F522): unsupported by the engine's matcher compiler. **Plan:** in Datalog,
  over the format string's guest tree (the grammars above) and the call's arguments.

### 8 too heavy, or aborting, at scale

Measured on 2026-11-13, each rule alone (`RERAIL_PROFILE`'s `heads.tsv` and `analysis.txt`).

| Rule | Measured | Cause | Fix |
|---|---|---|---|
| PIE810 | 1 file 221 MB; 10 files over 2 GB | A context's cover (`rr_cover41(c2)`) pairs every node `c2` under an ancestor with every other node `c7` under it, 1,988 rows into 107,176, and then rejects them all on `!rr_not104(c7)`. `c7` is only tested for existence. | Planner: an existence-only side of a join is checked once per ancestor (a semi-join), not paired. |
| C416 | 1 file 3.5 s; 10 files 80 s | The same shape: `dm_c416_alt_14_0` pairs a node under one sibling with nodes under a later sibling (23,263 rows into 1.9 M). | The same semi-join. |
| ANN401 | 1 file 254 MB; 10 files over 2 GB | The translator's demand `ann401_demand_alt_34(c5, LA)` carries `c5`, every node of the module, beside each annotation: 621 rows into 1.2 M. | Translator: a demand carries only the columns its helper is asked at. |
| UP028 | 1 file 1.4–2.1 GB, 33–70 s (the old engine too) | An inlined util joins names by text (`text(J, t), text(N, t)`) over every identifier of a scope inside a count, which is quadratic in a scope's names. Main resolves the name once a match. Not yet profiled to the relation: the run passes the cap first. | Translator: read same-name bindings through the library's name resolution (`binding`, `ref_site`). |
| B606, B323, B304 | Clean alone on 10 files | Strict mode aborted grouped 50-file runs: `$rr_sd0 >= 1073741824 has no native step` (a hiding bound with no native step). | Engine: the bound's comparison gets a native step. To be reproduced alone on 50 files. |
| AIR312 | Clean alone on 10 files (531 MB) | On 50 files, the writer's queue fills with the memo's entries (about 200 MB). | Engine: the memo's writes are bounded, as the other writes are. To be reproduced. |
| D418, TC006 | Abort alone on one file | Strict mode: `a stratum the native engine declined`. Grouped runs hid it. | Engine: the stratum's native lowering. |

