---
"@neralabs/rerail-pack-python": patch
---

`bandit-b402-import-ftplib` now reports ftplib in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, ftplib`), through a relative import (`from . import ftplib`), and as any module whose name starts with ftplib.
