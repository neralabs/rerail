if a or True:
    pass
if a or 1:
    pass
if a or 0:
    pass
if a or [1]:
    pass
if a or []:
    pass
if a or list():
    pass
if a or list([1]):
    pass
if a or (1 + 1):
    pass
if a or -1:
    pass
if True or a:
    pass
z = a or True
z = True or a
z = 1 or a or b
if a or b or "x":
    pass
if a or "":
    pass
if a or ...:
    pass
if a or (lambda: 0):
    pass
if a or bool(0):
    pass
while a or True:
    pass
assert a or True
z = 1 if a or True else 2
if f() or True:
    pass
if a or (b, c):
    pass
if a or {1: 2}:
    pass
if a or b"x":
    pass
if a or 0.5:
    pass
if a and b or True:
    pass
if a or (b or True):
    pass
