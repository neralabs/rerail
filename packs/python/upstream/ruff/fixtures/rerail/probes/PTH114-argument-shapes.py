import os.path as osp

p = "x"
osp.islink(p)
osp.islink(path=p)
osp.islink(s=p)
osp.islink(p, p)
osp.islink(*[p])
osp.islink(**{"path": p})
(osp.islink)(p)
osp.islink(x for x in [p])
osp.islink(p,)
