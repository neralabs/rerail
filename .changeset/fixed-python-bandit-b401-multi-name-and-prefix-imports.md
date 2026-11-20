---
"@neralabs/rerail-pack-python": patch
---

`bandit-b401-import-telnetlib` now reports telnetlib in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, telnetlib`), through a relative import (`from . import telnetlib`), and as any module whose name starts with telnetlib (`import telnetlib3 as telnetlib`).
