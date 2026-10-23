---
"@neralabs/rerail-pack-javascript": patch
---

JavaScript pack: ships five more ESLint core rules now that they pass every port readiness gate. `eslint-no-self-assign` starts on, as `eslint:recommended` turns it on, and now follows ESLint's self-assignment check to any nesting depth (`[[[[a]]]] = [[[[a]]]]`, `[...[a]] = [...[a]]`), where it stopped at three levels. `eslint-dot-notation`, `eslint-no-duplicate-imports`, `eslint-no-extra-label` and `eslint-no-lonely-if` start off, since `eslint:recommended` does not turn them on. Their documents share repeated structure through utilities instead of writing it out per clause; what they find is unchanged.
