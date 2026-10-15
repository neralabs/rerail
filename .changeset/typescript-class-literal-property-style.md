---
"@neralabs/rerail-pack-typescript": patch
---

`typescript-eslint-class-literal-property-style` now tells an agent to check for subclasses that override a getter as an accessor before turning it into a readonly field, since that change breaks the override with TS2611 (NER-1785).
