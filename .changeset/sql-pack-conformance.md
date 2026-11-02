---
"@neralabs/rerail-pack-sql": patch
---

SQL pack: its 22 SQLFluff 4.4.0 ports run through the conformance check, and the 19 that pass it ship. On: AL04, AL08, AL09, AM01, AM05, AM09, CV02, CV04, CV05, CV08, RF04, RF05, RF06, ST01, ST02, ST08 and ST10. Off, as SQLFluff's default configuration leaves them (`force_enable = False`): PG01 and PG02. AL05, AM02 and RF01 remain candidates. `licenses/sqlfluff/UPSTREAM.md` lists the SQLFluff test cases the pack copies.
