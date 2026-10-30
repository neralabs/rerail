# @neralabs/rerail-pack-python

## 0.1.6

### Patch Changes

- 61cda45: Each pack's `covers`, the line the dashboard shows under the pack's name, says in one sentence what the pack's rules are for and where they come from. It no longer counts rules or says how many start on: the dashboard shows both beside it, and the counts went stale whenever a rule shipped.
- 1e7042b: README: the note that deriving pack test data is paused is gone, since derive runs again; no rule changes.
- c8352a7: README, `pack.json` descriptions and licence notes: the conformance check is the one gate a port passes to ship, the ECMAScript packs' tests carry rename variants that upstream labels (`holds-under-rename`), and new test data is published through a tool-pack's `publish/` directory; no rule changes.
- c8352a7: Licence notes: each `licenses/<project>/UPSTREAM.md` records its upstream repository per repo (source, the path patterns taken, what is not copied) instead of one row per copied test file; no rule changes.
- 3fb36af: The README describes only the rules the pack ships. What it said about the candidate ports moves to `candidates/README.md`, which is not published.
- c8352a7: README and licence notes: the ports are scored on labelled real-code samples instead of pinned corpora; no rule changes.
- f158c3b: Python pack: 51 more Ruff 0.16.9 rules ship, all off: Ruff's default selection leaves them off, and `gates.yaml` does not list the new ports yet (a follow-up adds the entries, and with them the 39 ports in Ruff's default selection that wait as candidates until they can start on). Four of the 24 rules #326 promoted go back to candidates, because this port's larger fixture set shows them disagreeing with Ruff: `ruff-b018-useless-expression` (a parenthesised t-string concatenation), `ruff-b035-static-key-dict-comprehension` (a comprehension split across lines by comments), `ruff-w605-invalid-escape-sequence` (an escape inside a string annotation in a stub) and `ruff-f722-forward-annotation-syntax-error` (`TypedDict` keyword fields). 207 rules, 75 of them off. 408 Ruff ports are candidates (48 before), every shipped Ruff rule has a `# noqa` case in the pack's own test data, and the README lists the 66 stable Ruff rules not ported, with reasons.
- e1b9a49: Python pack: ships 24 more rules, 160 in all, now that they pass every conformance check. Start on: `bandit-b103-set-bad-file-permissions`, `bandit-b201-flask-debug-true`, `bandit-b304-ciphers`, `bandit-b406-import-xml-sax`, `bandit-b508-snmp-insecure-version`, `bandit-b509-snmp-weak-cryptography`, `bandit-b609-linux-commands-wildcard-injection`, `bandit-b610-django-extra-used`, `bandit-b612-logging-config-insecure-listen`, `bandit-b614-pytorch-load`, `bandit-b702-use-of-mako-templates`, `ruff-b015-useless-comparison`, `ruff-b018-useless-expression`, `ruff-b035-static-key-dict-comprehension`, `ruff-f402-import-shadowed-by-loop-var`, `ruff-f704-yield-outside-function`, `ruff-sim102-collapsible-if`, `ruff-sim103-needless-bool`, `ruff-sim117-multiple-with-statements`, `ruff-sim118-in-dict-keys`, `ruff-sim222-expr-or-true`, `ruff-sim223-expr-and-false` and `ruff-w605-invalid-escape-sequence`. Starts off, as no approved upstream configuration turns it on: `ruff-f722-forward-annotation-syntax-error`.

## 0.1.5

### Patch Changes

- 7392aea: README and licence notes: the pack's test data now lives outside the package; no rule changes.
- fa6fe5e: Preserve comparison polarity in E711/F632 steers and add concise behavior-preserving guidance for f-string braces, zip truncation, exception ordering, ordered comparisons, deployment environment keys and async HTTP/process waits. Keep existing rule scopes and enforcement modes.
- d0dd309: README: internal ticket references removed; no rule changes.
- 211e8b5: The note on the port readiness recording in each `licenses/*/UPSTREAM.md` names the upstream licence file kept beside it (`LICENSE`) by its own name.

## 0.1.4

### Patch Changes

- e06db22: Python pack: messages whose rewrite only changed the voice are back to their original wording; in an eval with a coding agent, the original and rewritten messages steered it equally well. Messages where the original steered the agent better in that eval are back to their original wording too.
- e06db22: Python pack: every rule description now says in one plain sentence what the rule wants; the exact matching detail moved to the pack README.
- e06db22: Python pack: every violation message now says in one sentence what to change in the flagged code, without tool names or rule codes.

## 0.1.3

### Patch Changes

- d1e67d7: Python pack: the port readiness gates decide what ships. `ruff-b030-except-with-non-exception-classes` passes every gate and the pack's conformance check and ships, on; `bandit-b406-import-xml-sax`, `bandit-b610-django-extra-used` and `ruff-f722-forward-annotation-syntax-error` move to `candidates/` after failing the gates (B406 differs from Bandit on a Django file, B610 names words from Bandit's tests, F722 repeats too much structure); 136 rules ship.

## 0.1.2

### Patch Changes

- da9e12b: First release through the release workflow with npm trusted publishing. The rules are unchanged.

## 0.1.1

### Patch Changes

- a0200c4: The package names the repository and directory it is published from, `neralabs/rerail` at `packs/<id>`.
- 0179e73: Python pack: `ruff-b030-except-with-non-exception-classes` no longer reports every `except T as name` handler; an `as` binding is transparent, so like Ruff's B030 the rule reports only handler types that are not exception class expressions (such as `except 1 as e`).
- 48d8728: bandit-b604 (candidate) reads only a `shell=` keyword, as Bandit's has_shell does: a call's second positional argument is no longer taken as `shell` (pytest.mark.parametrize, getattr and re.sub stop being flagged), and `shell=<expression>` is now reported unless it is a falsy constant, `[]` or `{}`.
- b8cc317: Three Python rules (two on by default, B406 and B610; F722 is off by default) no longer steer agents to rewrite idiomatic code: `bandit-b406-import-xml-sax` skips imports of only `xml.sax.saxutils` (escaping and writing helpers, no parser), `bandit-b610-django-extra-used` reports `.extra(...)` only on a receiver that looks like a Django QuerySet or Manager, and `ruff-f722-forward-annotation-syntax-error` skips jaxtyping shape strings such as `Float[Array, "b n"]`.
- 5a9d5b6: Python pack: 138 Ruff 0.16.9 and Bandit 1.9.4 rules, each scored against the tool's recorded findings on its own tests, 25 of them off by default; 96 more ports, among them 5 rules the pack carried before, wait in candidates.
- 740cf08: One rule id style in every pack, before any pack is published: lowercase letters, digits and dashes, the same string as the rule's document file name and its port's id in `conformance.json`. JavaScript's `eslint:<rule>` and `unicorn:<rule>` ids become `eslint-<rule>` and `unicorn-<rule>`, with their documents renamed to match; TypeScript's `typescript-eslint:<rule>` become `typescript-eslint-<rule>`; Python's `python-conformance:<tool>-<rule>` become `<tool>-<rule>`; React's `react:no-array-index-key` becomes `react-no-array-index-key`; Next's probe documents are named for their ids; the complexity rules become `complexity-cyclomatic-<language>`. TypeScript's `no-explicit-any` candidate is converted from `tempo.policy/v1` to `rerail.policy/v1` and moves to `candidates/rules/`. The JavaScript and TypeScript packs say how many of their rules start on.
