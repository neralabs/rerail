import os
from os import path as osp

p = "x"
os.path.isdir(p)
os.path.isdir(s=p)
os.path.isdir(path=p)
os.path.isdir(p, p)
os.path.isdir(p,)
os.path.isdir(*[p])
os.path.isdir(**{"s": p})
os.path.isdir(x for x in [p])
(osp.isdir)(p)
os.path.isdir(
    # why
    p
)
