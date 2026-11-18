---
"@neralabs/rerail-pack-python": patch
---

`bandit-b415-import-pyghmi` now reports pyghmi in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, pyghmi`), through a relative import (`from . import pyghmi`), and as any module whose name starts with pyghmi.
