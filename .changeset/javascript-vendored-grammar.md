---
"@neralabs/rerail-pack-javascript": patch
"@neralabs/rerail-pack-typescript": patch
"@neralabs/rerail-pack-complexity": patch
---

READMEs: JavaScript is parsed with the vendored, patched tree-sitter-javascript 0.25.0, and the patched tree-sitter-typescript reads vscode's import types, so the JavaScript and TypeScript packs declare no divergences (the eleven were files the grammars could not parse).
