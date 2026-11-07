def make_handlers(names):
    handlers = []
    for name in names:
        handlers.append(lambda: print(name))
    return handlers


def make_adders(numbers):
    adders = []
    for n in numbers:
        def add(x):
            return x + n
        adders.append(add)
    return adders


def make_bound(names):
    return [lambda name=name: print(name) for name in names]
