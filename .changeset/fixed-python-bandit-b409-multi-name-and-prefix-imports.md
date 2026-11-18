---
"@neralabs/rerail-pack-python": patch
---

`bandit-b409-import-xml-pulldom` now reports xml.dom.pulldom in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import sys, xml.dom.pulldom` or `from xml.dom import Node, pulldom`), through a relative import, and as any longer dotted name that starts with xml.dom.pulldom.
