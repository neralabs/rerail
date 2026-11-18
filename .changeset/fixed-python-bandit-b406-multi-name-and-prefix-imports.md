---
"@neralabs/rerail-pack-python": patch
---

`bandit-b406-import-xml-sax` now reports xml.sax in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, xml.sax` or `from xml import dom, sax`), through a relative import, and as any longer dotted name that starts with xml.sax (`import xml.sax.handler`, `import xml.sax.saxutils`). `from xml.sax import saxutils, handler` is now reported too; a from-import of only saxutils (`from xml.sax import saxutils`, `from xml.sax.saxutils import escape`) still is not.
