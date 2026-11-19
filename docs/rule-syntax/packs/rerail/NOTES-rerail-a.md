# rerail pack, batch A

Seven rules, one file each, no library additions and no blocked rule. Every regex is the JSON's, copied exactly
(generated with json.dumps, as `fill.py` does). Where the JSON's `regex` is ast-grep's search over a node's text, the
translation is `match("(?s:.*)(?:RE)(?s:.*)", n)`, or RE without its `^`/`$` where the JSON anchors both ends.

- `rerail-pack-package-names`: the JSON's `any` of four literal kinds is `name_literal`; regex and `not regex` as
  written. No difference.
- `rerail-pack-rule-test-placement`: the five call shapes are `reads_pack_rule`. The fifth one's
  `not inside program (not has member_expression … inside binary_expression)` is read as "the file picks rules by
  `x.rule.id ===`" (`picks_rule_by_id(d)`), and the conversation-rule exception as `conversation_rule_test(d)`, both per
  document (`docof`) rather than per `program` ancestor: a TypeScript file has one program. No difference.
- `rerail-table-probes-in-storage`: two clauses, each over strings and templates (`string_literal`). No difference.
- `rerail-home-file-names`: the seven alternatives as seven rules; the shared "two bare names" regex of alternatives 2
  and 4 is `two_names`. No difference.
- `rerail-home-file-paths`, `rerail-fixed-value-lists`: `file.content` regex clauses as `match_all` over `source`,
  reported at the match. No difference.
- `rerail-host-sdk-imports`: the five loading forms (import, `export … from`, `import x = require`, `declare module`,
  require/import/createRequire call) are `loads(n, s)`; both clauses test `s` with their own regex. The JSON's `has`
  without `stopBy` is a direct child (`field`/`child`). No difference.

Validation (on a Linux build box): test-rr 23 passed, 0 failed (3+3+3+3+3+4+4). Every rule `same` on the
pack's caught examples (1/1/1/1/1/3/1 findings in both), passes examples (0 in both) and the repository at 7efcac9c67
(0 findings in both for all seven: the repository already conforms). As a stronger check, each test's fixtures were
also run as a corpus of their own against the JSON original: `same` on all 23 (50 findings in both, none on one side
only). `site/check_rr.py` reports nothing in these files (its 14 problems are pre-existing, elsewhere).
