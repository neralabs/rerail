---
"@neralabs/rerail-pack-django": patch
---

`django-practice-runpython-with-schema-change` is a new rule; it starts off (Log when turned on): Give a data migration that writes rows its own migration, apart from schema changes to the same table; on PostgreSQL one shared transaction can fail or lock the table.
