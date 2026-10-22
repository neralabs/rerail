import os
from os.path import isfile as is_file

p = "x"
os.path.isfile(p)
os.path.isfile(path=p)
os.path.isfile(s=p)
os.path.isfile(p, p)
os.path.isfile(*[p])
os.path.isfile(**{"path": p})
is_file(p)
(os.path.isfile)(p)
os.path.isfile(x for x in [p])
