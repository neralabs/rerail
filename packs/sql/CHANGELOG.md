# @neralabs/rerail-pack-sql

## 0.1.3

### Patch Changes

- e06db22: SQL pack: messages whose rewrite only changed the voice are back to their original wording; in an eval with a coding agent, the original and rewritten messages steered it equally well.
- e06db22: SQL pack: every rule description now says in one plain sentence what the rule wants; the exact matching detail moved to the pack README.
- e06db22: SQL pack: every violation message now says in one sentence what to change in the flagged code, without tool names or rule codes.

## 0.1.2

### Patch Changes

- da9e12b: First release through the release workflow with npm trusted publishing. The rules are unchanged.

## 0.1.1

### Patch Changes

- 2a303bf: Add the SQL pack. It ships no rules yet: its 22 SQLFluff 4.4.0 ports are candidates until they pass the V1 rule validation gates, each scored against SQLFluff's findings on its tests and on four pinned corpora with every disagreement declared, and the pack is never recommended while it has no rules.
