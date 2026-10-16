---
"@neralabs/rerail-pack-javascript": patch
---

JavaScript pack: the 21 ESLint core rules that typescript-eslint replaces or turns off for TypeScript (`no-redeclare`, `no-undef`, `no-unused-vars`, `no-dupe-keys` and others) run on JavaScript files only, so TypeScript overloads, declaration merges and type names are no longer reported, and they are scored on ESLint's JavaScript test cases only.

The `default-param-last` and `max-params` candidates read JavaScript's own parameter syntax, so on JavaScript files they report what ESLint does (they were written against TypeScript's parameter nodes and reported nothing there).
