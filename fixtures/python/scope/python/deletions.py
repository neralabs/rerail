import os
import sys

a = 1
del a
print(a)

b = 1
if sys.argv:
    del b
print(b)

c = 1
while sys.argv:
    del c
    break
print(c)

d = 1
for _ in range(3):
    pass
else:
    del d
print(d)

e = 1
for _ in range(3):
    del e
    break
print(e)

f = 1
try:
    del f
except OSError:
    pass
print(f)

g = 1
try:
    pass
except OSError:
    del g
print(g)

h = 1
try:
    pass
except OSError:
    pass
else:
    del h
print(h)

i = 1
with open(os.devnull) as handle:
    del i
print(i)

j = 1
match sys.argv:
    case [_]:
        del j
    case _:
        pass
print(j)

k, m = 1, 2
del k, m
print(k, m)

n = 1
del (n)
n = 2
print(n)

del undefined_name


def reads_the_module_after_deletion():
    return a, b


def deletes_a_module_name():
    del b


def deletes_its_own():
    o = 1
    del o
    return o


def deletes_twice():
    p = 1
    del p
    del p


def deletes_a_global():
    global q
    del q


def outer():
    r = 1

    def inner():
        nonlocal r
        del r

    return inner, r


class Holder:
    s = 1
    del s
    t = s


try:
    pass
except OSError:

    def handler_function():
        u = 1
        del u
        return u


len = 1
del len
len(os.sep)

del os.environ["X"], sys.argv[0]
