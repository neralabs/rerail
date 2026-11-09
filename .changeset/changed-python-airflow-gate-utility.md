---
"@neralabs/rerail-pack-python": patch
---

Python pack: `ruff-air302-airflow3-moved-to-provider`, `ruff-air311-airflow3-suggested-update` and `ruff-air312-airflow3-suggested-to-move-to-provider` carry their `airflow` import gate once, in the shared `seen-airflow` utility, instead of a copy in every clause. Findings are unchanged, and a file without an `airflow` import costs what it did.
