---
"@neralabs/rerail-pack-typescript": patch
---

typescript-eslint-no-namespace does not report `global { … }` inside `declare module 'x' { … }`, a global augmentation, as typescript-eslint does not; the engine now parses it, as an `internal_module` named `global`.
