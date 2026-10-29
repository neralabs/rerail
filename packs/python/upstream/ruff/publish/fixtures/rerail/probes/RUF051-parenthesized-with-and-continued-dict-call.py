with (dict() as a):
    pass
if 1 in a:
    del a[1]
b = dict \
    ()
if 1 in b:
    del b[1]
c = {} \
    if b else {}
if 1 in c:
    del c[1]
with ({}, 1) as (d, e):
    pass
if 1 in d:
    del d[1]
if 1 in e:
    del e[1]
