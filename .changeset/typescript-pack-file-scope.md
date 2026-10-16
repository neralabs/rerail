---
"@neralabs/rerail-pack-typescript": patch
---

TypeScript pack: every typescript-eslint rule runs on TypeScript files only, so `no-require-imports` no longer reports CommonJS `require` in `.js` and `.cjs` files and no rule reports a JavaScript file its ESLint core twin already reports; the vscode corpus is scored on its TypeScript files.
