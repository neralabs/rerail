---
"@neralabs/rerail-pack-python": patch
---

Python pack: `ruff-air302-airflow3-moved-to-provider`, `ruff-air311-airflow3-suggested-update` and `ruff-air312-airflow3-suggested-to-move-to-provider` now gate every clause on the file importing `airflow`, as Ruff's `seen_module(AIRFLOW)` check does, so a file that never imports Airflow does none of the rules' name matching. On 2918 Python files from seven libraries without Airflow, AIR302 went from 134 s to 1.8 s and AIR311 from 17.6 s to 1.3 s; findings on files that import Airflow are unchanged.
