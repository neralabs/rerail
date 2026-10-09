x = 1
def f():
    print(x)
    x = 2

x = 1
def f():
    print(x)  # noqa
    x = 2

x = 1
def f():
    print(x)  # noqa: F823
    x = 2

x = 1
def f():
    print(x)  # NOQA: F823
    x = 2

x = 1
def f():
    print(x)  # noqa: E501
    x = 2
