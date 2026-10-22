import os
from os import mkdir

mkdir("a", dir_fd=(None))
mkdir("b", dir_fd=3)
(os.mkdir)("c", 0o700)
os.mkdir("d", dir_fd=None, **{"x": 1})
os.mkdir(*["e"])
os.mkdir()
os.mkdir("f", 0o700, dir_fd=fd)
