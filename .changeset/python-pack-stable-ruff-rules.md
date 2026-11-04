---
"@neralabs/rerail-pack-python": patch
---

Python pack: 51 more Ruff 0.16.9 rules ship, all off, as Ruff's default selection leaves them. Four rules go back to candidates because a larger fixture set shows them disagreeing with Ruff: `ruff-b018-useless-expression`, `ruff-b035-static-key-dict-comprehension`, `ruff-w605-invalid-escape-sequence` and `ruff-f722-forward-annotation-syntax-error`. 207 rules, 75 of them off; every shipped Ruff rule has a `# noqa` case in the pack's test data.
