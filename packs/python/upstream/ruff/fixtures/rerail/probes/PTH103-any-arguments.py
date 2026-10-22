import os
from os import makedirs as mk

mk("a", exist_ok=True)
(os.makedirs)("b", 0o700, True)
os.makedirs(*["c"])
os.makedirs(**{"name": "d"})
os.makedirs()
os.makedirs("e", dir_fd=3)


def shadowed(os):
    os.makedirs("f")
