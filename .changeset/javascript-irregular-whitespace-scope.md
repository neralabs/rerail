---
"@neralabs/rerail-pack-javascript": patch
---

JavaScript pack: `eslint-no-irregular-whitespace` checks JavaScript and TypeScript files only (`.js`, `.jsx`, `.mjs`, `.cjs`, `.ts`, `.tsx`, `.mts`, `.cts`), as ESLint lints them, instead of every file. Its regex clauses for whitespace before a file's first token used to read any text file in scope too: Markdown, JSON and YAML files then, and, since code rules with a regex read every text file, `Justfile`, `LICENSE`, lock files and the rest.
