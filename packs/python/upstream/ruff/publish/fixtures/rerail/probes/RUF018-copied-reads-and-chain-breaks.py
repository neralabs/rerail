a = 0
print(a)
del a
assert (a := 1)
b = 0
print(b)
if c:
    del b
assert (b := 1)
try:
    pass
except E as e:
    print(e)
assert (e := 1)
def outer():
    x = 1
    def inner():
        print(x)
        assert (x := 2)
    assert (y := 1)
    def g():
        nonlocal y
assert (z := 1)
__all__ = ["z"]
