---
"@neralabs/rerail-pack-python": patch
---

`bandit-b407-import-xml-expat` now reports xml.dom.expatbuilder in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, xml.dom.expatbuilder` or `from xml.dom import Node, expatbuilder`), through a relative import, and as any longer dotted name that starts with xml.dom.expatbuilder.
