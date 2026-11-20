---
"@neralabs/rerail-pack-python": patch
---

`bandit-b405-import-xml-etree` now reports xml.etree.ElementTree and xml.etree.cElementTree in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, xml.etree.ElementTree` or `from xml.etree import ElementInclude, ElementTree`), through a relative import, and as any longer dotted name that starts with either module's.
