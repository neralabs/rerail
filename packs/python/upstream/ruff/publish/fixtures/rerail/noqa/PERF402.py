def f(x: list):
    out = []
    for i in x:
        out.append(i)

def f(x: list):
    out = []
    for i in x:
        out.append(i)  # noqa

def f(x: list):
    out = []
    for i in x:
        out.append(i)  # noqa: PERF402

def f(x: list):
    out = []
    for i in x:
        out.append(i)  # NOQA: PERF402

def f(x: list):
    out = []
    for i in x:
        out.append(i)  # noqa: E501
