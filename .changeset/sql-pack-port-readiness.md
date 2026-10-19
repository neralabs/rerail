---
"@neralabs/rerail-pack-sql": patch
---

SQL pack: its 22 SQLFluff 4.4.0 candidates now run through the port readiness gates (SQLFluff configured as PostgreSQL, the dialect the engine reads every `.sql` file in). None passes every blocking gate yet, so the pack still ships no rules; `licenses/sqlfluff/UPSTREAM.md` lists the SQLFluff test cases the gates read.
