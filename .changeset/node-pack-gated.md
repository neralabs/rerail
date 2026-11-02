---
"@neralabs/rerail-pack-node": patch
---

Node pack: the conformance check now gates its eslint-plugin-n ports, which have an upstream manifest (`upstream/eslint-plugin-n/gates.yaml`) like the other ESLint-family tool-packs. The pack still ships no rules: every port stays a candidate until it is promoted.
