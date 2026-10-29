def f(d: dict):
    out = {}
    for k, v in d.items():
        out[k] = v

def f(d: dict):
    out = {}
    for k, v in d.items():
        out[k] = v  # noqa

def f(d: dict):
    out = {}
    for k, v in d.items():
        out[k] = v  # noqa: PERF403

def f(d: dict):
    out = {}
    for k, v in d.items():
        out[k] = v  # NOQA: PERF403

def f(d: dict):
    out = {}
    for k, v in d.items():
        out[k] = v  # noqa: E501
