def f(xs):
    for x in xs:
        if x:
            break
        else:
            print(x)

def f(xs):
    for x in xs:
        if x:
            break
        else:  # noqa
            print(x)

def f(xs):
    for x in xs:
        if x:
            break
        else:  # noqa: RET508
            print(x)

def f(xs):
    for x in xs:
        if x:
            break
        else:  # NOQA: RET508
            print(x)

def f(xs):
    for x in xs:
        if x:
            break
        else:  # noqa: E501
            print(x)
