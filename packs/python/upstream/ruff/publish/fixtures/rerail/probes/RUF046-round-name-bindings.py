inferred = 1
f_ = 1.0
a = b = 1
c, d = 1, 2
e = 1
e = 2
g: int
h: int = 1
i: float = 1
j = True
k = int("3")
l = int
(w := 1)


def fn(p: int, q: float, r, s: "int" = 1):
    int(round(inferred, 2))
    int(round(f_, 2))
    int(round(a, 2))
    int(round(b, 2))
    int(round(c, 2))
    int(round(d, 2))
    int(round(e, 2))
    int(round(g, 2))
    int(round(h, 2))
    int(round(i, 2))
    int(round(j, 2))
    int(round(k, 2))
    int(round(l, 2))
    int(round(w, 2))
    int(round(p, 2))
    int(round(q, 2))
    int(round(r, 2))
    int(round(s, 2))
    int(round((inferred), 2))
    int(round(inferred, -1))
    int(round(inferred, 1 + 1))
    int(round(inferred, h))
    int(round(1, h))
    int(round(1, 1 + 1))
    int(round(1, True))
    int(round(1, inferred))
    int(round(inferred))
    int(round(inferred, None))
    int(round(q))
    int(round(r))
    int(round(r, None))
    int(round(r, 2))
