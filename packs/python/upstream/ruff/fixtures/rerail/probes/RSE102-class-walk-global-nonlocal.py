class A:
    class B:
        class C:
            def Foo(self):
                pass

            class bar(Exception):
                pass

        def Make(self):
            pass

    B2 = B


raise A.B.C.Foo()
raise A.B.C.bar()
raise (A.B).Make()
raise A.B2.Make()


class lowc(Exception):
    pass


def Upfn():
    pass


def uses_global():
    global lowc, Upfn
    raise lowc()
    raise Upfn()


def outer():
    def Fn():
        pass

    def inner():
        nonlocal Fn
        raise Fn()

    def shadowing():
        def Fn():
            pass

        raise Fn()

    return inner, shadowing
