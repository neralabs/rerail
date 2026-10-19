---
"@neralabs/rerail-pack-typescript": patch
"@neralabs/rerail-pack-next": patch
"@neralabs/rerail-pack-node": patch
"@neralabs/rerail-pack-react": patch
---

The match definition names its npm dependencies in the new form, `{ "dependencies": { "ecosystem": "npm", "names": [...] } }`, which core reads alongside PyPI and Composer dependencies. Which repositories the pack fits is unchanged.
