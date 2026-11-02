---
"@neralabs/rerail-pack-python": patch
---

Python pack: `ruff-air302-airflow3-moved-to-provider`, `ruff-air311-airflow3-suggested-update` and `ruff-air312-airflow3-suggested-to-move-to-provider` carry their `airflow` import gate once, in the shared `seen-airflow` utility, now that the engine keeps a root-node gate through a utility; the inline copy every clause carried is gone. Findings and the work a file without an `airflow` import does are unchanged.
