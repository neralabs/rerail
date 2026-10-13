---
"@neralabs/rerail-pack-typescript": patch
---

TypeScript pack: every rule honours ESLint's suppression comments naming it (`// eslint-disable-next-line @typescript-eslint/consistent-type-definitions` and the other `eslint-disable` forms), as ESLint does, and Oxlint 1.88.0's (`// oxlint-disable-next-line consistent-type-definitions`); before, 6 of the 22 honoured ESLint's and none Oxlint's. The candidates do too.
