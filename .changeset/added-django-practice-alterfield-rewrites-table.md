---
"@neralabs/rerail-pack-django": patch
---

`django-practice-alterfield-rewrites-table` is a new rule; it starts off (Log when turned on): Avoid column type changes that make PostgreSQL rewrite an existing large table, such as integer to bigint or a shorter length limit; the rewrite blocks reads and writes.
