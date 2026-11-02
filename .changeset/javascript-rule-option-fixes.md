---
"@neralabs/rerail-pack-javascript": patch
---

JavaScript pack: `eslint-dot-notation` honours any `allowPattern`, with or without `allowKeywords`, as ESLint does: a bracket key that matches the pattern is no longer reported. Before, only one built-in pattern took effect and every other was ignored. The pattern is read as the engine's (Rust) regular expression, so one in JavaScript-only syntax, such as lookbehind or a backreference, is refused when it is set instead of being ignored.

`eslint-no-duplicate-imports` with `allowSeparateTypeImports` treats `import type from 'm'`, a default import named `type`, as a value import, so it is reported beside another value import from the same module. `eslint-no-setter-return` no longer reports `return /* comment */;` in a setter, and again reports an arrow setter whose expression body follows a comment (`set: v => /* comment */ v`).

The messages of `eslint-dot-notation`, `eslint-no-duplicate-imports`, `eslint-no-extra-label` and `eslint-no-setter-return` now say what to write instead, such as "Write .name instead of ["name"]." and "Merge this import into the earlier import from 'm'.".
