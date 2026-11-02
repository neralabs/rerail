---
"@neralabs/rerail-pack-javascript": patch
---

`eslint-no-irregular-whitespace` checks JavaScript and TypeScript files only, as ESLint lints them. It used to check every file it could read for whitespace before the first token: Markdown, JSON and YAML, and with this release also `Justfile`, `LICENSE` and lock files.
