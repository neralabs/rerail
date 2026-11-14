# 0182: Main's JSON rules need no hand-written overrides, and the Rust translator is the only one

Status: decided (2026-11-13, in the work to run main's rules directly: "do all these / retire the translator").

## Intro

The engine reads main's JSON rules (`rerail.policy/v1`) through its own translator (decision 0175). Until now, 9
of main's 537 Python rules could not be translated, so `packs/python-main` carried a hand-written `.rr` file for
each one. A JSON file loaded by the engine never got those overrides, so those 9 rules ran in their broken
generated form.

The translator was also kept twice: the Rust one in the engine, and the Python one in `docs/rule-syntax/site` it
was ported from, held equal byte for byte by a test.

The 9 rules needed three things the engine could not say:

```
// B110: main parses a snippet in a context, and matches the node of the selector's kind
pattern: {context: "try:\n    work()\nexcept:\n    pass", selector: except_clause}

// RET505: main captures the `else` keyword, which the tape leaves out
{regex: "^(else|elif)$", inside: {kind: if_statement}}

// COM818: a capture regex over a node whose kind the rule does not name
{regex: ".*(?P<COMMA>,)", inside: {kind: module}}
```

## Decision

1. **A pattern may have a selector**: ``n ~ `try: … except: pass` at "except_clause"``. The engine parses the
   context, and matches the context's node of the selector's kind at `n` (`language::pattern_key` joins the two
   for the matcher; `pattern_source` splits them). The tape stores the selector as field `selector`.
2. **A capture regex can be precomputed over the children of a kind** (`SpanIn::ChildOf(K)`), when the rule names
   only the parent's kind. The lowering takes K from a `kind(par, K)` atom or from the column's kind set.
3. **A token that opens its parent is read at the parent** (the translator's `leading_token`): its first stored
   token, or the source before its first stored child, in a `.stored` relation. A capture of it is a symbol column.
   A span finding has an `at: loc` column (a span in a node column gives no finding).
4. **A util that reads a transform's target gets the target's source from its caller**, so its column is bound.
5. **Backtick bodies keep real newlines.** An escaped `\n` is two characters of code to the reader, so a
   multi-line pattern never matched.
6. **The overrides are gone**: `packs/python-main/overrides/` and `generated-before-override/`. Every file in the
   pack is the engine's translation of the JSON, and the test `json_rules_pack_is_current` holds it so.
7. **The Python translator is retired**: `translate.py`, `translate_rules.py`, `engine_dialect.py`,
   `main_terms.py`, `simplify.py`, `fold.py` and `textglob.py` are deleted. The site build (`build.py`) gets its
   display text from `rerail-engine translate-json --display`. `souffle.py` stays, for `check_rr.py`.

## Consequences

- All 537 rules pass the reader with no override. On probes with known answers, each of the 9 generated rules gives
  the same findings as its override did (B110: lines 3, 8, 13; B112: lines 30, 34; RET505–508, COM818, PYI010 and
  N806 likewise).
- A translator change is made once, in Rust, and the pack is regenerated (`site/regen_main_pack.sh`).
- Ruff's fixtures (`site/upstream_check.py`) are unchanged: 11 of the 14 rules with a generated form are equal.
- Found on the N806 probe, not fixed here: the library's binding sites differ from main's. Main's `binds: {form}`
  is at the name, with forms `assignment`, `named`, `augmented` and so on. `lib/python/names.rr` puts a simple
  assignment's site on the statement, with forms `assign`, `walrus`, `augassign`. So `x = 1` gets no
  `site_form(x, "assignment")`, and N806 misses it, in its override as in its generated form. About 46 generated
  rules use `site_form`.
