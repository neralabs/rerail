import os


class A:
    r = s = 7
    p, q = 1, "a"
    if c:
        t = 3
    u = 3
    for u in range(2):
        pass
    v = 3
    v += 1
    w = 3
    if c:
        w = "x"
    class B:
        y = 3
    def m(self) -> int:
        return 1


class O:
    class B:
        y = 3
    B = None


k1 = 1 + int("3")
k2 = 3 if c else True
k3 = 2 ** -1
k4 = 1 @ 2
k5 = True if c else False

with 3 as w1:
    pass

os.chmod(A.r, 1)
os.chmod(A.s, 1)
os.chmod(A.p, 1)
os.chmod(A.q, 1)
os.chmod(A.t, 1)
os.chmod(A.u, 1)
os.chmod(A.v, 1)
os.chmod(A.w, 1)
os.chmod((A.B).y, 1)
os.chmod(A.m, 1)
os.chmod(O.B.y, 1)
os.chmod(k1, 1)
os.chmod(k2, 1)
os.chmod(k3, 1)
os.chmod(k4, 1)
os.chmod(k5, 1)
os.chmod(w1, 1)
