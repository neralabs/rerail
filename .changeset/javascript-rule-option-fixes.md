---
"@neralabs/rerail-pack-javascript": patch
---

`eslint-no-setter-return` no longer reports `return /* comment */;` in a setter, now reports an arrow setter whose body follows a comment (`set: v => /* comment */ v`), and reports a parenthesized arrow body at the expression inside the parentheses, as ESLint does. Its message for a `return` now says to return without a value.
