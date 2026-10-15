---
"@neralabs/rerail-pack-python": minor
---

Python pack: `bandit-b406-import-xml-sax`, `bandit-b610-django-extra-used` and `ruff-f722-forward-annotation-syntax-error` move to `candidates/` after failing the rule gates (B406 differs from Bandit on a Django file, B610 names words from Bandit's tests, F722 repeats too much structure); 135 rules ship.
