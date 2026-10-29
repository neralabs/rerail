---
"@neralabs/rerail-pack-python": patch
---

Python pack: 99 more Ruff 0.16.9 rules ship, all off, as Ruff's default selection leaves them, and four of the 24 rules #326 promoted go back to candidates, because this port's larger fixture set shows them disagreeing with Ruff: `ruff-b018-useless-expression` (a parenthesised t-string concatenation), `ruff-b035-static-key-dict-comprehension` (a comprehension split across lines by comments), `ruff-w605-invalid-escape-sequence` (an escape inside a string annotation in a stub) and `ruff-f722-forward-annotation-syntax-error` (`TypedDict` keyword fields). 255 rules, 123 of them off. 360 Ruff ports are candidates (61 before), every shipped Ruff rule has a `# noqa` case in the pack's own test data, and the README lists the 66 stable Ruff rules not ported, with reasons.
