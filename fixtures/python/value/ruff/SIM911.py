d1 = d2 = {}
zip(d1.keys(), d1.values())
zip(d2.keys(), d2.values())
x = [1]
zip(x.keys(), x.values())
zip(unbound.keys(), unbound.values())
annotated: dict = make()
zip(annotated.keys(), annotated.values())
zip(d1.keys(), d2.values())
zip(d1.keys(), d1.values(), strict=True)
zip(d1.keys(), d1.values(), other)


def function(mapping: dict, **kwargs):
    zip(mapping.keys(), mapping.values())
    zip(kwargs.keys(), kwargs.values())


def zip(*args):
    return args
