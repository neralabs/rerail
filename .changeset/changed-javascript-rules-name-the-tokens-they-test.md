---
"@neralabs/rerail-pack-javascript": patch
---

Rules that test punctuation such as `,`, `=>` or `[` now name the token and report at it, so they run on the rule engine that leaves those tokens off its syntax trees. What they report is unchanged.
