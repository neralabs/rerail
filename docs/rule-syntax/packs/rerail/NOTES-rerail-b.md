# rerail pack, batch B

Rules: `rerail-command-sigil`, `rerail-no-code-execution`, `rerail-session-paths`, `rerail-conformance-boundary`
(two files), `rerail-conformance-pinned-data`. Each follows its JSON clause for clause. The regexes are the JSON's, copied
exactly by a throwaway generator. A JSON `regex` on a node (an ast-grep search) becomes `match` over the node's text: wrapped in `(?s:.*)…(?s:.*)`, with the wrapper
dropped on the side a `^` or `$` anchors. `inside`/`has` without `stopBy` are the parent or a child, and with
`stopBy: end` any ancestor or descendant.

## Per rule

- **rerail-command-sigil**: the JSON's three ternary alternatives are one relation, `ternary_sigil(t, side, s)`:
  the condition names an agent and the two sides hold different sigils, or the condition names Codex and either side
  holds `$`. `(?i)sigil$` is `isuffix("sigil", …)`. The JSON's `follows: template_substitution` is `next_sibling`.
  Inside a template string every child but the backquotes is named, so the token right before a fragment is its named
  previous sibling.
- **rerail-no-code-execution**: the JSON's `file.path` filter on the first three clauses is `reads_data(d)`, a glob
  equal to the JSON's path regex. `^process\.execPath$` is a text equality. The updater exclusion is
  `p != "src/setup/updater.ts"`.
- **rerail-session-paths**: the `has: {field: operator}` on a binary expression is `op(b, "??") ; op(b, "||")`. A binary
  expression has no other anonymous token.
- **rerail-conformance-boundary**: the JSON's scope covers TypeScript and JavaScript files plus anything under
  `conformance/**/recorders/**`. That takes two policies with one `.policy` id:
  `rerail-conformance-boundary.rerail` (`.language typescript`, the eight TS/JS globs, all four clauses) and
  `rerail-conformance-boundary.text.rerail` (`.scope text`, the recorders glob minus the TS/JS globs, the recorder
  clause only). The original reports a `file.path` match at line 0, checked on the box for `.ts`, `.mjs`, `.py`, `.json`,
  `.sh`, `.txt` and `.md` files under a recorders folder, and the translation uses `@place(path, 0)`. The pack's caught
  example has `.split("\0")`. The fixture writes `.split(String.fromCharCode(0))` instead, because `site/check_rr.py`
  reads fixture lines as `.rerail` strings and calls `\0` an unknown escape. The finding is on line 1 either way.
- **rerail-conformance-pinned-data**: as the JSON. The module clause shares the library below with the boundary rule.

In both conformance rules, a string's `^["'](X)["']$` and a template's `` ^`(X)`$ `` are one test,
`["'`](X)["'`]`, over the literal that names the module. They agree on every well-formed literal, because a string
cannot open with a backquote and a template cannot open with a quote.

No rule is blocked, and no translation simplifies the original's matching.

## Library addition

`lib/tsx/rerail-module-loads.rerail` (new, mine): `module_literal(n, s)` is the literal `s` naming the module that `n`
loads (an import's strings, an export's source, the string or template argument of require, import, module.require or
process.getBuiltinModule), and `loads_types_only(n)` holds for `import type`, `export type` and a load inside
`typeof import(…)`. The names avoid tsx/values' `module_load`.

## Validation (nera-port-1, box-check.sh rpt-trb)

- test-rr: command-sigil 4/4, no-code-execution 4/4, session-paths 4/4, conformance-boundary 6/6 (4 + 2 text),
  conformance-pinned-data 3/3.
- Original against translation (keys are path, line and code):
  - caught examples: `same` for all five rules (1, 2, 1, 1 and 1 findings).
  - passes examples: `same`, with 0 findings for every rule.
  - repository at 7efcac9c67: `same` for all five rules. Only session-paths has findings (2 shared); the other four
    have 0.
  - every test's fixtures, run as a corpus: `same` on all 21 tests, 46 findings shared, 0 on either side only.
- `site/check_rr.py`: no problems in these files (14 problems remain, all in other files).
