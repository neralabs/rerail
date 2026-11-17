---
"@neralabs/rerail-pack-python": patch
---

`bandit-b407-import-xml-expat` now reports xml.dom.expatbuilder in two more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, xml.dom.expatbuilder` or `from xml.dom import Node, expatbuilder`) and through a relative import.
