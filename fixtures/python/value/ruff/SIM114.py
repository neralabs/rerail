# Consecutive if/elif/else arms with structurally equal bodies.
def f(a, b):
    if a:
        return 1
    elif b:
        return 1
    if a:
        x = (1, 2)
    else:
        x = 1, 2
    if a:
        x = 1
    elif b:
        x = 1.0
    if a:
        g(x, y=1)
    elif b:
        g(x, y=(1))
    else:
        g(x, y=1)
    if a:
        print("a" "b")
    elif b:
        print('ab')
    if a:
        pass
    elif b:
        return 2
    else:
        return 2
    if a:
        return x[1:]
    elif b:
        return x[:1]
    if a:
        return f"{x}"
    elif b:
        return f'{x}'
    if a:
        y = 1
        return y
    elif b:
        y = 1
    if a:
        return 0x10
    elif b:
        return 16
