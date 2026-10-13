def f(xs):
    for x in xs:
        if x:
            continue
        else:
            print(x)

def f(xs):
    for x in xs:
        if x:
            continue
        else:  # noqa
            print(x)

def f(xs):
    for x in xs:
        if x:
            continue
        else:  # noqa: RET507
            print(x)

def f(xs):
    for x in xs:
        if x:
            continue
        else:  # NOQA: RET507
            print(x)

def f(xs):
    for x in xs:
        if x:
            continue
        else:  # noqa: E501
            print(x)
