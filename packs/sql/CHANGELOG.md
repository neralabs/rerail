# @neralabs/rerail-pack-sql

## 0.1.4

### Patch Changes

- 7392aea: README and licence notes: the pack's test data now lives outside the package; no rule changes.
- 115b81b: SQL pack: its 22 SQLFluff 4.4.0 ports run through the port readiness gates (SQLFluff configured as PostgreSQL, the dialect the engine reads every `.sql` file in), and the 19 that pass every blocking gate ship. On: AL04, AL08, AL09, AM01, AM05, AM09, CV02, CV04, CV05, CV08, RF04, RF05, RF06, ST01, ST02, ST08 and ST10. Off, as SQLFluff's default configuration leaves them (`force_enable = False`): PG01 and PG02. AL05, AM02 and RF01 remain candidates. `licenses/sqlfluff/UPSTREAM.md` lists the SQLFluff test cases the gates read.

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
