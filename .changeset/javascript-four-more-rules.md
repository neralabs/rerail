---
"@neralabs/rerail-pack-javascript": patch
---

`eslint-no-loss-of-precision` no longer reports a decimal literal that a JavaScript number stores exactly, such as `9223372036854775808` (2^63): it compares the literal with the number at the same precision, as ESLint does.
