def f():
    out = []
    for i in range(3):
        out.append(i * 2)

def f():
    out = []
    for i in range(3):
        out.append(i * 2)  # noqa

def f():
    out = []
    for i in range(3):
        out.append(i * 2)  # noqa: PERF401

def f():
    out = []
    for i in range(3):
        out.append(i * 2)  # NOQA: PERF401

def f():
    out = []
    for i in range(3):
        out.append(i * 2)  # noqa: E501
