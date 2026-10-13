---
"@neralabs/rerail-pack-react": patch
---

React pack: the candidates honour Oxlint 1.88.0's suppression comments naming their rule (`// oxlint-disable-next-line rules-of-hooks` and the other `oxlint-disable` forms), as well as ESLint's. The 12 candidates Biome 2.5.15 has a rule the same as honour `biome-ignore` comments naming it (`// biome-ignore lint/correctness/useHookAtTopLevel: reason` and the other `biome-ignore` forms).
