# @neralabs/rerail-pack-python

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
- b8cc317: Three Python rules (two on by default, B406 and B610; F722 is off by default) no longer steer agents to rewrite idiomatic code (NER-1827): `bandit-b406-import-xml-sax` skips imports of only `xml.sax.saxutils` (escaping and writing helpers, no parser), `bandit-b610-django-extra-used` reports `.extra(...)` only on a receiver that looks like a Django QuerySet or Manager, and `ruff-f722-forward-annotation-syntax-error` skips jaxtyping shape strings such as `Float[Array, "b n"]`.
- 5a9d5b6: Python pack: 138 Ruff 0.16.9 and Bandit 1.9.4 rules, each scored against the tool's recorded findings on its own tests, 25 of them off by default; 96 more ports, among them 5 rules the pack carried before, wait in candidates.
- 740cf08: One rule id style in every pack, before any pack is published: lowercase letters, digits and dashes, the same string as the rule's document file name and its port's id in `conformance.json`. JavaScript's `eslint:<rule>` and `unicorn:<rule>` ids become `eslint-<rule>` and `unicorn-<rule>`, with their documents renamed to match; TypeScript's `typescript-eslint:<rule>` become `typescript-eslint-<rule>`; Python's `python-conformance:<tool>-<rule>` become `<tool>-<rule>`; React's `react:no-array-index-key` becomes `react-no-array-index-key`; Next's probe documents are named for their ids; the complexity rules become `complexity-cyclomatic-<language>`. TypeScript's `no-explicit-any` candidate is converted from `tempo.policy/v1` to `rerail.policy/v1` and moves to `candidates/rules/`. The JavaScript and TypeScript packs say how many of their rules start on.
