def f(y):
    a = [x for x in (Y1 := range(3))]
    b = {x: 1 for x in (Y2 := y)}
    c = [[z for z in (W := x)] for x in y]
    d = (x for x in y if (Y3 := x))
    e = [x for x in y for q in (Y4 := y)]
    return a, b, c, d, e
