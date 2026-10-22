import os
c = input()
class A:
    fd: int = 1
    if c:
        fd = "x"
class B:
    fd = "x"
    if c:
        fd = 1
class C:
    fd = 1
    fd += 1
class D:
    fd, other = 1, "a"
class S:
    class T:
        fd: int = 1
    @property
    def gd(self) -> int: ...
os.stat(A.fd)
os.stat(B.fd)
os.stat(C.fd)
os.stat(D.fd)
os.stat(S.T.fd)
os.stat(S.gd)
n1 = 1 if c.strip() else 2
os.stat(n1)
n2 = True & False
os.stat(n2)
n3 = 1 @ 2
os.stat(n3)
n4 = 2 ** -1
os.stat(n4)
