import os
from os.path import isabs

p = "x"
os.path.isabs(p)
os.path.isabs(s=p)
os.path.isabs(path=p)
os.path.isabs(p, p)
os.path.isabs(*[p])
os.path.isabs(**{"s": p})
isabs(p)
(os.path.isabs)(p)
os.path.isabs(x for x in [p])
