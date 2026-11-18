---
"@neralabs/rerail-pack-python": patch
---

`bandit-b413-import-pycrypto` now reports PyCrypto's modules in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, Crypto.Cipher` or `from Crypto import Random, Hash`), through a relative import, and as a submodule or longer dotted name of one of them (`import Crypto.Cipher.AES`, `from Crypto.Util.number import bytes_to_long`).
