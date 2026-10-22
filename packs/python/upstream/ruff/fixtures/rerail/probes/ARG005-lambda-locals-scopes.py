def deco(*a):
    return lambda f: f


class A(deco(lambda x: locals())):
    pass


class B(metaclass=deco(lambda x: locals())):
    pass


@deco(lambda x: locals())
class C:
    pass


class D:
    @deco(lambda x: locals())
    def m(self, f=lambda y: locals()):
        pass

    def n(self):
        def inner():
            return lambda z: locals()

        return inner

    e = [lambda w: locals() for _ in range(2)]
    f = {k: (lambda v: locals()) for k in range(2)}
    g = lambda a: (lambda b: locals())
    h = lambda a: [(lambda b: locals()) for _ in range(2)]
    i = lambda a: (locals)()
    j = lambda a: (locals())
    k = lambda a: locals() if a else None
    l = lambda a: [y for y in (locals)()]


class E(D):
    class F:
        o = lambda a: locals()

    p = lambda a: locals()


def outer():
    class G:
        pass

    return lambda a: locals()


lam = lambda a: (lambda b: locals())
lam2 = lambda a: [(lambda b: locals()) for _ in range(2)]
lam3 = lambda a: (locals)()
lam4 = lambda a: locals() if a else None
lam5 = lambda a: [y for y in (locals)()]
lam6 = lambda a: [y for y in (locals)() if y]
lam7 = lambda a: [y for y in [z for z in range(1) if locals()]]
lam8 = lambda a: [y for y in range(3) if (z := locals())]
lam9 = lambda a: {**locals()}
lam10 = lambda a: [*locals()]
lam11 = lambda a: locals()[a]
lam12 = lambda a: locals().get("a")
lam13 = lambda a: print(a) or locals()
lam14 = lambda a: not locals()
lam15 = lambda a: [y for y in range(3)][locals()]
lam16 = lambda a: lambda: [y for y in locals()]
lam17 = lambda a=lambda b: locals(): a
lam18 = lambda a=(lambda b: [locals()]): a
lam19 = lambda a=lambda b: 0: locals()
lam20 = lambda a, c=a: c
