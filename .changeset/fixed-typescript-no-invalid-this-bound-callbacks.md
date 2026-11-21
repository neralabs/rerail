---
"@neralabs/rerail-pack-typescript": patch
---

`typescript-eslint-no-invalid-this` no longer tells agents that `this` is undefined in a `function` passed as an argument, such as a Mocha test or a Cypress command, which bind it. There it says to keep the `function` and declare `this` as its first parameter. What the rule reports is unchanged.
