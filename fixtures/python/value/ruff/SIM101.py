# Duplicate isinstance calls: adjacent calls whose targets are structurally equal.
def f(a, b, c):
    if isinstance(a, int) or isinstance(a, float):
        pass
    if isinstance(a.b, int) or isinstance(a . b, float):
        pass
    if isinstance((a), int) or isinstance(a, float):
        pass
    if isinstance(a[0], int) or isinstance(a[0x0], float):
        pass
    if isinstance(a[0], int) or isinstance(a[0.0], float):
        pass
    if isinstance(a, int) or isinstance(b, float):
        pass
    if isinstance(a, int) or b or isinstance(a, float):
        pass
    if isinstance(a, int) or isinstance(b, int) or isinstance(b, str):
        pass
    if isinstance(f(a), int) or isinstance(f(a), str):
        pass
    if isinstance("a" "b", str) or isinstance("ab", str):
        pass
    if isinstance(a, int) and isinstance(a, float):
        pass
    if isinstance(-1, int) or isinstance(-(1), int):
        pass
    if isinstance(c, int) or isinstance(c, str) or isinstance(c, float):
        pass
