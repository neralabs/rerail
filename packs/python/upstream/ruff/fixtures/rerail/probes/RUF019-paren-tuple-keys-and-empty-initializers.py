d = {}
x = y = 1
if ((x, y)) in d and d[x, y]:
    pass
if (*x,) in d and d[*x]:
    pass
if (list)() in d and d[(list)()]:
    pass
if list(  # empty
) in d and d[list()]:
    pass
z = bool(  # first argument
    x in d and d[x]
)
