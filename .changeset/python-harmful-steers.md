---
"@neralabs/rerail-pack-python": patch
---

Three Python rules (two on by default, B406 and B610; F722 is off by default) no longer steer agents to rewrite idiomatic code (NER-1827): `bandit-b406-import-xml-sax` skips imports of only `xml.sax.saxutils` (escaping and writing helpers, no parser), `bandit-b610-django-extra-used` reports `.extra(...)` only on a receiver that looks like a Django QuerySet or Manager, and `ruff-f722-forward-annotation-syntax-error` skips jaxtyping shape strings such as `Float[Array, "b n"]`.
