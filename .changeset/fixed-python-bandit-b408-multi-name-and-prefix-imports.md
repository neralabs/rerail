---
"@neralabs/rerail-pack-python": patch
---

`bandit-b408-import-xml-minidom` now reports xml.dom.minidom in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import sys, xml.dom.minidom` or `from xml.dom import Node, minidom`), through a relative import, and as any longer dotted name that starts with xml.dom.minidom.
