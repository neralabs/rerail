import os

a = True or 1
b = 1 or 2.5
c = True & True
d = 1 if True else 2.0
e = (1 < 2) + 1
f = 1 // 0
g = (n := int("3"))
os.listdir(a)
os.listdir(b)
os.listdir(c)
os.listdir(d)
os.listdir(e)
os.listdir(f)
os.listdir(g)
with 3 as h, int(3) as (i, j):
    os.listdir(h)
    os.listdir(i)
os.listdir(*paths)


class C:
    def m(self):
        fd = 1

    def meth(self) -> int: ...


os.listdir(C.fd)
os.listdir(C.meth)
