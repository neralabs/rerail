---
"@neralabs/rerail-pack-next": patch
---

Next.js pack: every rule honours Oxlint 1.88.0's suppression comments naming it (`// oxlint-disable-next-line no-img-element` and the other `oxlint-disable` forms), as well as ESLint's. The 9 rules Biome 2.5.15 has a rule the same as honour `biome-ignore` comments naming it (`// biome-ignore lint/performance/noImgElement: reason` and the other `biome-ignore` forms), as do 2 candidates.
