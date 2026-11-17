---
"@neralabs/rerail-pack-python": patch
---

`bandit-b406-import-xml-sax` now reports xml.sax in two more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, xml.sax` or `from xml import dom, sax`) and through a relative import. `from xml.sax import saxutils, handler` is now reported too; importing only saxutils still is not.
