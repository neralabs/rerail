zs = [[y for y in (fooA := x)] for x in [[1]]]
gs = {k: v for k, v in (fooB := {}).items()}
ss = {[z for z in (fooC := w)][0] for w in ["a"]}
ts = (a for a in [b for b in (fooD := [1])])
us = [x for x in [y for y in (fooE := [1])]]
vs = [x for x in [(fooF := y) for y in [1]]]
(fooN) += 1
((fooM)) -= 1
def f():
    global fooP
    (fooP) += 1
class K:
    (fooQ) *= 2
if x:
    (fooR) //= 2
(_fooS) += 1
(FooT) += 1
