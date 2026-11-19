---
"@neralabs/rerail-pack-python": patch
---

`bandit-b411-import-xmlrpclib` now reports xmlrpc in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, xmlrpc.client`), through a relative import (`from . import xmlrpc`), and as any module whose name starts with xmlrpc, such as Python 2's `xmlrpclib`.
