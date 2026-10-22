a = 1, 2
for i in list(a):
    pass
def f1():
    x = [1]
    for i in list(x):
        (x.append(1))
def f2():
    x = [1]
    for i in list(x):
        x.append(1), 2
def f3():
    x = [1]
    for i in list(x):
        del (x[0], i)
def f4():
    x = [1]
    for i in list(x):
        del (x[0]), i
def f5():
    x = [1]
    for i in list(x):
        def x():
            pass
def f6():
    x = [1]
    for i in list(x):
        [x := 5 for _ in range(3)]
def f7():
    x = [1]
    for i in list(x):
        import x.y
def f8():
    x = [1]
    def inner():
        nonlocal x
        for i in list(x):
            pass
