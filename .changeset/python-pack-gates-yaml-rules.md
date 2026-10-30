---
"@neralabs/rerail-pack-python": patch
---

Python pack: `gates.yaml` lists every ported Ruff 0.16.9 rule (566, 157 before), each with Ruff's default selection as its `recommended` configuration, so a rule starts on when Ruff's default selection includes it. The 39 ports in that selection that waited as candidates ship on: C419, G010, G101, G201, G202, PYI001, PYI002, PYI003, PYI004, PYI005, PYI007, PYI008, PYI009, PYI010, PYI012, PYI015, PYI017, PYI020, PYI026, PYI029, PYI033, PYI035, PYI041, PYI044, PYI048, PYI050, PYI052, PYI066, RUF008, RUF009, RUF012, RUF013, UP007, UP024, UP028, UP029, UP030, UP032, UP043. 246 rules, 75 of them off; 369 Ruff ports are candidates.
