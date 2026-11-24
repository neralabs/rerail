---
"@neralabs/rerail-pack-django": patch
---

`django-practice-alterfield-rewrites-table` is a new rule; it starts on, in Log: Avoid column type changes that make PostgreSQL rewrite an existing large table, such as integer to bigint or a shorter length limit; the rewrite blocks reads and writes.
