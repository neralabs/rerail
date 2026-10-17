---
"@neralabs/rerail-pack-typescript": patch
---

`typescript-eslint-no-unused-expressions` no longer steers agents into dropping the condition of a short-circuit call (NER-1945): for `a && f()`, `a || f()` and `a ?? f()` it reports the same finding with upstream's message plus which condition the call runs under and how to keep it in an `if` (`a && f()` becomes `if (a) { f(); }`). Set-up takes its `allowShortCircuit` and `allowTernary` from a `tslint.json` `no-unused-expression` (`allow-fast-null-checks`), as it already did from an ESLint config.
