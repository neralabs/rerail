---
"@neralabs/rerail-pack-typescript": patch
---

`typescript-eslint-no-unused-expressions` now tells agents to rewrite a short-circuit call such as `a && f()` as an `if` that keeps its condition, and names the condition. Agents given the old message sometimes dropped the condition. What the rule reports is unchanged.
