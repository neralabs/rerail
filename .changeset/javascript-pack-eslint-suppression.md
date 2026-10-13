---
"@neralabs/rerail-pack-javascript": patch
---

JavaScript pack: every rule honours ESLint's suppression comments naming it (`// eslint-disable-line no-console` and the other `eslint-disable` forms), as ESLint does; before, 12 of the 65 did. The 63 rules Oxlint 1.88.0 also implements honour `oxlint-disable` comments naming them too. The candidates do the same, except their regex clauses (`unicorn/filename-case`, and `no-irregular-whitespace` before the first token).
