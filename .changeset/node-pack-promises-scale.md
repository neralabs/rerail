---
"@neralabs/rerail-pack-node": patch
---

Node.js pack: `n-prefer-promises-fs` and `n-prefer-promises-dns` finish on large files. Their aliasing branches read every occurrence of a callee's variable once per call, which was quadratic on large test suites (both rules reported nothing for a 14,000-line test file). A call is now read only in a file that names the module, and the aliasing branches skip imported and `const` bindings, which no program that runs can reassign. Their fixture and corpus scores are unchanged.
