---
"@neralabs/rerail-pack-python": patch
---

`bandit-b406-import-xml-sax` now reports xml.sax in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, xml.sax`), through a relative import, and as a longer dotted name (`import xml.sax.handler`). `from xml.sax import saxutils, handler` is reported too; a from-import of only saxutils still is not.
