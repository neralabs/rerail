import os

p = "x"
os.readlink(p)
os.readlink(path=p)
os.readlink(p, dir_fd=None)
os.readlink(p, dir_fd=3)
os.readlink(dir_fd=None)
os.readlink(s=p)
os.readlink(*[p])
os.readlink(**{"path": p})
(os.readlink)(p)
os.readlink(x for x in [p])
