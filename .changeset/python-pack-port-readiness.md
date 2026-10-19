---
"@neralabs/rerail-pack-python": patch
---

Python pack: the port readiness gates decide what ships. `ruff-b030-except-with-non-exception-classes` passes every gate and the pack's conformance check and ships, on; `bandit-b406-import-xml-sax`, `bandit-b610-django-extra-used` and `ruff-f722-forward-annotation-syntax-error` move to `candidates/` after failing the gates (B406 differs from Bandit on a Django file, B610 names words from Bandit's tests, F722 repeats too much structure); 136 rules ship.
