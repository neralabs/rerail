def f(k):
    d = {}
    if k in d:
        del d[k]  

def f(k):
    d = {}
    if k in d:
        del d[k]    # noqa

def f(k):
    d = {}
    if k in d:
        del d[k]    # noqa: RUF051

def f(k):
    d = {}
    if k in d:
        del d[k]    # NOQA: RUF051

def f(k):
    d = {}
    if k in d:
        del d[k]    # noqa: E501
