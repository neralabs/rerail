def f(*args, **kwargs):
    pass
f(a=1, *[2])

def f(*args, **kwargs):
    pass
f(a=1, *[2])  # noqa

def f(*args, **kwargs):
    pass
f(a=1, *[2])  # noqa: B026

def f(*args, **kwargs):
    pass
f(a=1, *[2])  # NOQA: B026

def f(*args, **kwargs):
    pass
f(a=1, *[2])  # noqa: E501
