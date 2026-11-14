---
"@neralabs/rerail-pack-typescript": patch
---

Rules that test punctuation such as `,`, `=>` or `require` now name the token and report at it, so they run on the rule engine that leaves implied tokens off its syntax trees. What they report is unchanged.
