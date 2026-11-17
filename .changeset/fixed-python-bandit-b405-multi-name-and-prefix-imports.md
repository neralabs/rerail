---
"@neralabs/rerail-pack-python": patch
---

`bandit-b405-import-xml-etree` now reports xml.etree.ElementTree and xml.etree.cElementTree in two more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, xml.etree.ElementTree` or `from xml.etree import ElementInclude, ElementTree`) and through a relative import.
