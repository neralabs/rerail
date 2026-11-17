---
"@neralabs/rerail-pack-python": patch
---

`bandit-b413-import-pycrypto` now reports PyCrypto's modules in two more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, Crypto.Cipher.AES` or `from Crypto import Random, Hash`) and through a relative import.
