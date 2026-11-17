---
"@neralabs/rerail-pack-python": patch
---

`bandit-b408-import-xml-minidom` now reports xml.dom.minidom in two more kinds of import statement, as Bandit does: as one name of a multi-name import (`import sys, xml.dom.minidom` or `from xml.dom import Node, minidom`) and through a relative import.
