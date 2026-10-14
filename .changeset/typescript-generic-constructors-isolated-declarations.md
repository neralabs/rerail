---
"@neralabs/rerail-pack-typescript": patch
---

TypeScript pack: `typescript-eslint-consistent-generic-constructors` (candidate) tells agents to keep a generic type annotation when the project compiles with `isolatedDeclarations` and the declaration is exported, where moving the type arguments to the constructor fails tsc with TS9010.
