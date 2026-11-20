---
"@neralabs/rerail-pack-javascript": patch
---

`eslint-curly` with the `multi` or `multi-or-nest` option no longer reports or removes the braces around a lone `using` or `await using` declaration, such as `if (foo) { using bar = f(); }`: the declaration cannot stand without them.
